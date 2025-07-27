use std::{
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
};
use clap::Parser;
use futures::StreamExt;
use jsonrpsee::{core::async_trait, proc_macros::rpc, server::Server};
use libp2p::{
    gossipsub, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr,
};
use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc, time::sleep};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, help = "Port for the RPC server")]
    rpc_port: Option<u16>,
    
    #[arg(short, long, help = "Port for the P2P node")]
    p2p_port: Option<u16>,
    
    #[arg(help = "Multiaddr of a peer to connect to")]
    peer: Option<String>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Signal {
    escrow_contract: Address,
    token_contract: Address,
    recipient: Address,
    transfer_amount: U256,
    reward_amount: U256,
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "send {} tokens to {} and collect {} tokens from escrow {}",
            self.transfer_amount, self.recipient, self.reward_amount, self.escrow_contract
        )
    }
}

#[rpc(server, namespace = "mirage")]
pub trait MirageRpc {
    #[method(name = "signal")]
    async fn signal(&self, message: Signal) -> String;
}

struct MirageServer {
    signal_tx: mpsc::UnboundedSender<Signal>,
}

#[async_trait]
impl MirageRpcServer for MirageServer {
    async fn signal(&self, message: Signal) -> String {
        log_now!("Received signal: {}", message);
        let _ = self.signal_tx.send(message.clone());
        format!("ack: {}", message)
    }
}

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

async fn spawn_rpc_server(
    signal_tx: mpsc::UnboundedSender<Signal>,
    block_tx: mpsc::UnboundedSender<u64>,
    rpc_port: Option<u16>,
) -> anyhow::Result<()> {
    let addr = match rpc_port {
        Some(port) => format!("127.0.0.1:{}", port),
        None => "127.0.0.1:0".to_string(),
    };
    let server = Server::builder().build(addr).await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());

    println!("Running rpc on {server_addr}");

    tokio::spawn(rpc_server.stopped());

    tokio::spawn(async move {
        let rpc_url = std::env::var("RPC").expect("RPC environment variable must be set");
        let ws = WsConnect::new(rpc_url);
        let provider = ProviderBuilder::new().connect_ws(ws).await.unwrap();

        log_now!("⏳ Waiting 5 seconds for peers to connect...");
        sleep(Duration::from_secs(5)).await;
        log_now!("✅ Starting block publishing");

        let mut block_stream = provider.subscribe_blocks().await.unwrap().into_stream();
        log_now!("🔄 Monitoring for new blocks...");

        // Process each new block as it arrives
        while let Some(block) = block_stream.next().await {
            let _ = block_tx.send(block.number);
        }
    });
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
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

    let block_topic = gossipsub::IdentTopic::new("eth-blocks");
    swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;

    let signal_topic = gossipsub::IdentTopic::new("mirage-signals");
    swarm.behaviour_mut().gossipsub.subscribe(&signal_topic)?;

    let p2p_addr = match args.p2p_port {
        Some(port) => format!("/ip4/0.0.0.0/tcp/{}", port),
        None => "/ip4/0.0.0.0/tcp/0".to_string(),
    };
    swarm.listen_on(p2p_addr.parse()?)?;

    if let Some(addr) = args.peer {
        let remote: Multiaddr = addr.parse()?;
        swarm.dial(remote)?;
        log_now!("Dialed {}", addr);
    }

    let (block_tx, mut block_rx) = mpsc::unbounded_channel();
    let (signal_tx, mut signal_rx) = mpsc::unbounded_channel();
    let _ = spawn_rpc_server(signal_tx, block_tx, args.rpc_port).await;

    let mut highest_block: u64 = 0;

    loop {
        tokio::select! {
            Some(block) = block_rx.recv() => {
                if block > highest_block {
                    highest_block = block;
                    swarm
                        .behaviour_mut()
                        .gossipsub
                        .publish(block_topic.clone(), block.to_be_bytes().to_vec())?;
                    log_now!("Published block {}", block);
                } else {
                    log_now!("Ignored local trigger {} (already at {})", block, highest_block);
                }
            }

            Some(signal) = signal_rx.recv() => {
                swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(signal_topic.clone(), serde_json::to_vec(&signal)?)?;
                log_now!("Published signal {}", signal);
            }

            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } =>
                    println!("Listening on {address:?}"),

                SwarmEvent::Behaviour(GossipBehaviorEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    match message.topic {
                        t if t == block_topic.hash() => {
                            if message.data.len() == 8 {
                                // Turn the 8‑byte payload back into u64
                                let num = u64::from_be_bytes(message.data[..8].try_into().unwrap());
                                if num > highest_block {
                                    log_now!("Heard gossip: advance to block number {num} from the current height of {}", highest_block);
                                    highest_block = num;    // accept progress
                                } // else silently ignore stale or duplicate heights
                            }
                        }
                        t if t == signal_topic.hash() => {
                            log_now!("Just heard signal: {}", String::from_utf8(message.data)?)
                        }
                        _ => log_now!("UNRECOGNIZED MESSAGE")
                    }
                }

                _ => {}
            }
        }
    }
}
