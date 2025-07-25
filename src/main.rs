use std::{
    fmt::Display,
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use futures::StreamExt;
use libp2p::{
    gossipsub, identity, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr,
};
use tokio::{
    sync::mpsc,
    time::{self, sleep},
};
use tracing_subscriber::EnvFilter;

#[macro_export]
macro_rules! log_now {
    // --- format string only ----------------------------------------
    ($fmt:expr) => {{
        use chrono::Local;
        println!(
            "[{}] {}",
            Local::now().format("%d-%m-%Y %H:%M:%S"),
            $fmt
        );
    }};

    // --- format string + more expressions -------------------------
    ($fmt:expr, $($arg:expr),*) => {{
        use chrono::Local;
        println!(
            "[{}] {}",
            Local::now().format("%d-%m-%Y %H:%M:%S"),
            format!($fmt, $($arg),*)
        );
    }};
}

#[derive(NetworkBehaviour)]
pub struct GossipBehavior {
    pub gossipsub: gossipsub::Behaviour,
}

fn spawn_mock_block_source(tx: mpsc::UnboundedSender<u64>) {
    tokio::spawn(async move {
        let rpc_url = std::env::var("RPC").expect("RPC environment variable must be set");
        let ws = WsConnect::new(rpc_url);
        let provider = ProviderBuilder::new().connect_ws(ws).await.unwrap();

        log_now!("⏳ Waiting 20 seconds for peers to connect...");
        sleep(Duration::from_secs(20)).await;
        log_now!("✅ Starting block publishing");

        let mut block_stream = provider.subscribe_blocks().await.unwrap().into_stream();
        log_now!("🔄 Monitoring for new blocks...");

        // Process each new block as it arrives
        while let Some(block) = block_stream.next().await {
            log_now!("publishing block {}", block.number);
            let _ = tx.send(block.number);
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

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
        log_now!("Dialed {}", addr);
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
                    log_now!("Published block {}", block);
                } else {
                    log_now!("Ignored local trigger {} (already at {})", block, highest_block);
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
                            log_now!("Advanced to block {num} from {:?}", message.source);
                        } // else silently ignore stale or duplicate heights
                    }
                }

                _ => {}
            }
        }
    }

    Ok(())
}
