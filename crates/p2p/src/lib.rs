use std::{
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

use futures::StreamExt;
use libp2p::{
    gossipsub, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{debug, info, warn};

use nomad_pool::SignalPool;
use nomad_types::Signal;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct P2pConfig {
    pub bootstrap: Vec<Multiaddr>,
    pub tcp: u16,
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            bootstrap: Vec::new(),
            tcp: 9000,
        }
    }
}

#[derive(NetworkBehaviour)]
struct GossipBehavior {
    pub gossipsub: gossipsub::Behaviour,
}

pub fn spawn_p2p(
    config: P2pConfig,
    read_only: bool,
    mut rx: UnboundedReceiver<Signal>,
    signal_pool: SignalPool,
) -> eyre::Result<()> {
    debug!(?config, ?read_only);

    // Setup the swarm
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::new(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|_| -> Result<_, _> {
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(12))
                .validation_mode(gossipsub::ValidationMode::None)
                .message_id_fn(|message: &gossipsub::Message| {
                    let mut h = DefaultHasher::new();
                    message.data.hash(&mut h);
                    gossipsub::MessageId::from(h.finish().to_string())
                })
                .build()
                .expect("Failed to make the gossipsub conf");

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Author(libp2p::PeerId::random()),
                gossipsub_config,
            )?;
            Ok(GossipBehavior { gossipsub })
        })?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))
        .build();

    // Subscribe to topics
    let block_topic = gossipsub::IdentTopic::new("eth-blocks");
    let signal_topic = gossipsub::IdentTopic::new("mirage-signals");
    swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;
    swarm.behaviour_mut().gossipsub.subscribe(&signal_topic)?;

    // Bind to p2p port
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", config.tcp).parse().unwrap())?;

    // Connect to bootstrap nodes
    for peer in config.bootstrap {
        swarm.dial(peer.clone())?;
        info!("Connected to peer: {peer}");
    }

    // Spawn the main event loop
    tokio::spawn(async move {
        let handle_swarm_event = |event| async {
            match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!("Listening on {}", address);
                }
                SwarmEvent::Behaviour(GossipBehaviorEvent::Gossipsub(
                    gossipsub::Event::Message {
                        message,
                        propagation_source,
                        ..
                    },
                )) if !read_only => match message.topic {
                    t if t == signal_topic.hash() => {
                        match serde_json::from_slice::<Signal>(&message.data) {
                            Ok(signal) => {
                                info!(
                                    peer = propagation_source.to_string(),
                                    "Received signal: {signal}"
                                );
                                // Insert signal to the pool
                                if !signal_pool.insert(signal).await {
                                    warn!(
                                        peer = propagation_source.to_string(),
                                        "Received duplicate signal"
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(%e, signal_data = ?String::from_utf8_lossy(&message.data), "Failed to parse received signal");
                            }
                        };
                    }
                    _ => warn!(
                        peer = propagation_source.to_string(),
                        "Received unrecognized message"
                    ),
                },
                _ => {}
            }
        };

        loop {
            tokio::select! {
                // Handle libp2p events
                biased;
                event = swarm.select_next_some() => handle_swarm_event(event).await,

                // Handle incoming signals
                Some(signal) = rx.recv() => {
                    let encoded = serde_json::to_vec(&signal).unwrap();

                    // insert into our own signal pool
                    signal_pool.insert(signal).await;

                    // publish signal to the network
                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(signal_topic.clone(), encoded) {
                        warn!(%e, "Failed to publish outgoing signal");
                    }
                },
            }
        }
    });

    Ok(())
}
