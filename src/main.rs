use std::{fmt::Display, hash::{DefaultHasher, Hash, Hasher}, time::Duration};

use futures::StreamExt;
use libp2p::{
    gossipsub, identity, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr,
};
use tokio::{sync::mpsc, time::{self, sleep}};
use tracing_subscriber::EnvFilter;

#[derive(NetworkBehaviour)]
pub struct GossipBehavior {
    pub gossipsub: gossipsub::Behaviour,
}

fn spawn_mock_block_source(tx: mpsc::UnboundedSender<u64>) {
    tokio::spawn(async move {
        let mut n = 0_u64;
        loop {
            sleep(tokio::time::Duration::from_secs(12)).await;
            n += 1;
            log(format!("publishing block {n}"));
            let _ = tx.send(n);
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::new(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|_| {
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
                gossipsub::MessageAuthenticity::Anonymous,
                gossipsub_config,
            )?;

            Ok(GossipBehavior { gossipsub })
        })?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))
        .build();

    let topic = gossipsub::IdentTopic::new("eth-blocks");
    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    if let Some(addr) = std::env::args().nth(1) {
        let remote: Multiaddr = addr.parse()?;
        swarm.dial(remote)?;
        println!("Dialed {addr}");
    }

    let (tx, mut rx) = mpsc::unbounded_channel();
    spawn_mock_block_source(tx);

    let mut highest_block: u64 = 0;

    loop {
        tokio::select! {
            // 1. external trigger: a new block height
            Some(block) = rx.recv() => {
                if block > highest_block {
                    highest_block = block;
                    swarm
                        .behaviour_mut()
                        .gossipsub
                        .publish(topic.clone(), block.to_be_bytes().to_vec())?;
                    log(format!("Published block {block}"));
                } else {
                    log(format!("Ignored local trigger {block} (already at {highest_block})"));
                }
            }

            // 2. libp2p swarm events
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } =>
                    println!("Listening on {address:?}"),

                SwarmEvent::Behaviour(GossipBehaviorEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    if message.data.len() == 8 {
                        // Turn the 8‑byte payload back into u64
                        let num = u64::from_be_bytes(message.data[..8].try_into().unwrap());
                        if num > highest_block {
                            highest_block = num;    // accept progress
                            log(format!("Advanced to block {num} from {:?}", message.source));
                        } // else silently ignore stale or duplicate heights
                    }
                }

                _ => {}
            }
        }
    }

    Ok(())
}

fn log<S: Display + AsRef<str>>(s: S) {
    println!("[{:?}] {s}", time::Instant::now());
}