use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    signers::local::PrivateKeySigner,
};
use anyhow::Context as _;
use clap::Parser;
use futures::StreamExt;
use jsonrpsee::{core::async_trait, proc_macros::rpc, server::Server};
use libp2p::{
    gossipsub, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, instrument, warn};
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

    #[arg(long, help = "Private key 1 to use ")]
    pk1: Option<String>,

    #[arg(long, help = "Private key 1 to use ")]
    pk2: Option<String>,
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
        info!(signal = %message, "Received signal");
        let _ = self.signal_tx.send(message.clone());
        format!("ack: {}", message)
    }
}

#[derive(NetworkBehaviour)]
pub struct GossipBehavior {
    pub gossipsub: gossipsub::Behaviour,
}

#[instrument(skip(signal_tx))]
async fn spawn_rpc_server(
    signal_tx: mpsc::UnboundedSender<Signal>,
    rpc_port: Option<u16>,
) -> anyhow::Result<()> {
    let addr = match rpc_port {
        Some(port) => format!("127.0.0.1:{port}"),
        None => "127.0.0.1:0".to_string(),
    };
    let server = Server::builder().build(addr).await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());

    println!("Running rpc on {server_addr}");

    tokio::spawn(rpc_server.stopped());
    Ok(())
}

#[tokio::main]
#[instrument]
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
        Some(port) => format!("/ip4/0.0.0.0/tcp/{port}"),
        None => "/ip4/0.0.0.0/tcp/0".to_string(),
    };
    swarm.listen_on(p2p_addr.parse()?)?;

    if let Some(ref addr) = args.peer {
        let remote: Multiaddr = addr.parse()?;
        swarm.dial(remote)?;
        info!(peer = %addr, "Dialed peer");
    }

    let (signal_tx, mut signal_rx) = mpsc::unbounded_channel();
    let _ = spawn_rpc_server(signal_tx, args.rpc_port).await;

    let mut highest_block: u64 = 0;

    let rpc_url = std::env::var("RPC").expect("RPC environment variable must be set");
    let ws = WsConnect::new(rpc_url);
    let provider = Arc::new(
        ProviderBuilder::new()
            .connect_ws(ws)
            .await
            .context("connect ws")?,
    );
    let signers = Arc::new(build_signers(&args)?);

    loop {
        tokio::select! {
            Some(signal) = signal_rx.recv() => {
                // publish with the network
                match swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(signal_topic.clone(), serde_json::to_vec(&signal)?) {
                    Ok(_) => info!(signal = %signal, "Published signal"),
                    Err(gossipsub::PublishError::Duplicate) => {
                        debug!(signal = %signal, "Signal already published (duplicate)");
                    }
                    Err(e) => return Err(e.into()),
                }

                if let Some((ref s1, ref s2)) = *signers {
                    if let Err(e) = process_signal(&signal, provider.clone(), s1, s2).await {
                        warn!(%e, "failed to process signal");
                    }
                } else {
                    debug!("Skipping signal; running in read-only mode");
                }
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
                                    info!(new_block = num, current_block = highest_block, "Heard gossip: advancing to new block");
                                    highest_block = num;    // accept progress
                                } // else silently ignore stale or duplicate heights
                            }
                        }
                        t if t == signal_topic.hash() => {
                            match serde_json::from_slice::<Signal>(&message.data) {
                                Ok(received_signal) => {
                                    info!(signal = %received_signal, "Received signal gossip");
                                    if let Some((ref s1, ref s2)) = *signers {
                                        if let Err(e) = process_signal(&received_signal, provider.clone(), s1, s2).await {
                                            warn!(%e, "failed to process signal");
                                        }
                                    } else {
                                        debug!("Skipping signal; running in read-only mode");
                                    }
                                }
                                Err(e) => {
                                    warn!(%e, signal_data = ?String::from_utf8_lossy(&message.data), "Failed to parse received signal");
                                }
                            }
                        }
                        _ => warn!("Received unrecognized message")
                    }
                }

                _ => {}
            }
        }
    }
}

pub async fn process_signal<P: Provider>(
    _signal: &Signal,
    provider: Arc<P>,
    signer1: &PrivateKeySigner,
    _signer2: &PrivateKeySigner,
) -> anyhow::Result<()> {
    let addr1 = signer1.address();
    let balance = provider.get_balance(addr1).await?;
    info!(%balance, "current balance");
    // …use signer2 and the signal…
    Ok(())
}

fn build_signers(args: &Args) -> anyhow::Result<Option<(PrivateKeySigner, PrivateKeySigner)>> {
    match (&args.pk1, &args.pk2) {
        // both present → parse, fail fast if either is malformed
        (Some(pk1), Some(pk2)) => {
            let s1: PrivateKeySigner = pk1.parse().context("parsing --pk1")?;
            let s2: PrivateKeySigner = pk2.parse().context("parsing --pk2")?;
            info!("Using addresses: {} and {}", s1.address(), s2.address());
            Ok(Some((s1, s2)))
        }
        // neither present → run in read-only mode
        (None, None) => Ok(None),
        // one present, one missing → treat as a configuration error
        _ => anyhow::bail!("Supply *both* --pk1 and --pk2 or neither"),
    }
}
