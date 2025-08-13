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
use local_ip_address::local_ip;
use nomad_core::pool::SignalPool;
use nomad_types::Signal;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{info, warn};

#[derive(Deserialize, Serialize, Debug)]
pub struct P2pConfig {
    pub bootstrap: Vec<Multiaddr>,
    pub tcp: u16,
}

#[derive(NetworkBehaviour)]
pub struct GossipBehavior {
    pub gossipsub: gossipsub::Behaviour,
}

pub fn spawn_p2p(
    config: P2pConfig,
    mut rx: UnboundedReceiver<Signal>,
    signal_pool: SignalPool,
) -> anyhow::Result<()> {
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

    let block_topic = gossipsub::IdentTopic::new("eth-blocks");
    swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;

    let signal_topic = gossipsub::IdentTopic::new("mirage-signals");
    swarm.behaviour_mut().gossipsub.subscribe(&signal_topic)?;

    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", config.tcp).parse().unwrap())?;

    // Connect to bootstrap nodes
    for peer in config.bootstrap {
        swarm.dial(peer.clone())?;
        info!("Connected to peer: {peer}");
    }

    tokio::spawn(async move {
        let handle_swarm_event = |event| async {
            match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!("Listening on {}", address);

                    // Log local and global IP addresses for P2P server
                    if let Some(port) = address.iter().find_map(|protocol| match protocol {
                        libp2p::multiaddr::Protocol::Tcp(p) => Some(p),
                        _ => None,
                    }) {
                        if let Ok(local_ip) = local_ip() {
                            info!("P2P server local network access: {}:{}", local_ip, port);
                        }
                        if let Ok(res) = reqwest::get("https://ifconfig.me").await {
                            if let Ok(ip) = res.text().await {
                                info!("P2P Global Address: {ip}:{}", config.tcp);
                            }
                        }
                    }
                }

                SwarmEvent::Behaviour(GossipBehaviorEvent::Gossipsub(
                    gossipsub::Event::Message {
                        message,
                        propagation_source,
                        ..
                    },
                )) => match message.topic {
                    t if t == signal_topic.hash() => {
                        match serde_json::from_slice::<Signal>(&message.data) {
                            Ok(signal) => {
                                info!(
                                    peer = propagation_source.to_string(),
                                    "Received signal: {signal}"
                                );
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
