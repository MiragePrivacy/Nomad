use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic},
    identify, noise,
    swarm::SwarmEvent,
    tcp, yamux, Multiaddr, Swarm,
};
use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};
use tracing::{debug, info, info_span, warn, Instrument};

use nomad_pool::SignalPool;
use nomad_types::Signal;

use crate::{behaviour::MirageBehaviorEvent, shutdown::Shutdown};

pub mod behaviour;
pub mod shutdown;
#[cfg(test)]
mod tests;

const MIRAGE_DISCOVERY_ID: &str = "/mirage/discovery";
const MIRAGE_MESHSUB_ID: &str = "/mirage/meshsub";

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct P2pConfig {
    pub bootstrap: Vec<Multiaddr>,
    #[serde(with = "humantime_serde")]
    pub bootstrap_interval: Duration,
    pub tcp: u16,
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            bootstrap: Vec::new(),
            bootstrap_interval: Duration::from_secs(5 * 60),
            tcp: 9000,
        }
    }
}

/// Peer to peer node
pub struct P2pNode {
    pub swarm: Swarm<behaviour::MirageBehavior>,
    read_only: Arc<AtomicBool>,
    signal_pool: SignalPool,
    signal_topic: IdentTopic,
}

impl P2pNode {
    pub fn new(
        config: P2pConfig,
        signal_pool: SignalPool,
        read_only: Arc<AtomicBool>,
        rx: Option<UnboundedReceiver<Signal>>,
    ) -> eyre::Result<Self> {
        debug!(?config, ?read_only);

        if config.bootstrap.is_empty() {
            warn!("No bootstrap peers provided, running as a bootstrap node!");
        }

        // Setup the swarm
        let mut swarm = libp2p::SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::new(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|keypair| behaviour::MirageBehavior::new(keypair, &config))?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX))
            })
            .build();

        // Setup signal ingestion
        if let Some(rx) = rx {
            swarm.behaviour_mut().signal.connect_rx(rx);
        }

        // Subscribe to topics
        let block_topic = gossipsub::IdentTopic::new("eth-blocks");
        let signal_topic = gossipsub::IdentTopic::new("mirage-signals");
        swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;
        swarm.behaviour_mut().gossipsub.subscribe(&signal_topic)?;

        // Bind to p2p port
        swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", config.tcp).parse().unwrap())?;

        // Connect to bootstrap nodes
        for peer in &config.bootstrap {
            info!("Connecting to bootstrap peer: {peer}");
            swarm.dial(peer.clone())?;
        }

        Ok(Self {
            swarm,
            read_only,
            signal_pool,
            signal_topic,
        })
    }

    pub fn shutdown_handle(&self) -> Shutdown {
        self.swarm.behaviour().shutdown.clone()
    }

    pub fn spawn(mut self) -> JoinHandle<eyre::Result<()>> {
        tokio::spawn(async move {
            while let Some(event) = self.swarm.next().await {
                match event {
                    // We have a new address
                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Listening on {}", address);
                    }

                    // Shutdown signal
                    SwarmEvent::Behaviour(MirageBehaviorEvent::Shutdown(())) => {
                        info!("Shutting down p2p node");
                        break;
                    }

                    // Incoming signals
                    SwarmEvent::Behaviour(MirageBehaviorEvent::Signal(signal)) => {
                        // Encode data
                        let encoded = flexbuffers::to_vec(&signal).unwrap();

                        // Insert signal into our own signal pool
                        if !self.read_only.load(std::sync::atomic::Ordering::Relaxed) {
                            self.signal_pool.insert(signal).await;
                        }

                        // Publish signal to the network
                        if let Err(e) = self
                            .swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(self.signal_topic.clone(), encoded)
                        {
                            warn!(%e, "Failed to publish outgoing signal");
                        }
                    }

                    // Peer identified its protocols, connect them to the associated behaviours
                    SwarmEvent::Behaviour(MirageBehaviorEvent::Identify(
                        identify::Event::Received { peer_id, info, .. },
                    )) => {
                        debug!(?peer_id, "Peer identified");
                        for protocol in &info.protocols {
                            match protocol.as_ref() {
                                MIRAGE_DISCOVERY_ID => {
                                    let kad = &mut self.swarm.behaviour_mut().kad;
                                    for addr in info.listen_addrs.clone() {
                                        kad.add_address(&peer_id, addr);
                                    }
                                }
                                p if p.starts_with(MIRAGE_MESHSUB_ID) => {
                                    self.swarm
                                        .behaviour_mut()
                                        .gossipsub
                                        .add_explicit_peer(&peer_id);
                                }
                                _ => {}
                            }
                        }
                    }

                    // Process incoming gossip signals, but only if we are not in read-only mode
                    SwarmEvent::Behaviour(MirageBehaviorEvent::Gossipsub(
                        gossipsub::Event::Message {
                            message,
                            propagation_source,
                            ..
                        },
                    )) if !self.read_only.load(std::sync::atomic::Ordering::Relaxed) => {
                        if message.topic != self.signal_topic.hash() {
                            warn!(
                                peer = ?propagation_source,
                                "Received unrecognized message"
                            );
                            continue;
                        }

                        let Ok(signal) = flexbuffers::from_slice(&message.data) else {
                            warn!(signal_data = ?String::from_utf8_lossy(&message.data), "Failed to parse received signal");
                            continue;
                        };

                        // Insert signal to the pool
                        let duplicate = !self.signal_pool.insert(signal).await;
                        info!(
                            duplicate,
                            peer = ?propagation_source,
                            "Received signal"
                        );
                    }

                    _ => {}
                }
            }
            Ok(())
        }.instrument(info_span!("p2p")))
    }
}
