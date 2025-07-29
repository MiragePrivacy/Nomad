use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    signers::local::PrivateKeySigner,
    sol,
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

    #[arg(long, help = "Private key 1 to use")]
    pk1: Option<String>,

    #[arg(long, help = "Private key 2 to use")]
    pk2: Option<String>,

    #[arg(
        long,
        help = "Use the faucet functionality on the given token contract. For testing mode."
    )]
    faucet: Option<String>,

    #[arg(long, help = "WebSocket RPC URL for reading blockchain data")]
    ws_rpc: String,

    #[arg(long, help = "HTTP RPC URL for sending transactions")]
    http_rpc: String,
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
async fn main() -> anyhow::Result<()> {
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

    let ws = WsConnect::new(&args.ws_rpc);
    let ws_provider = Arc::new(
        ProviderBuilder::new()
            .connect_ws(ws)
            .await
            .context("connect ws")?,
    );

    let signers = Arc::new(build_signers(&args)?);

    let provider_with_wallet = if let Some((ref s1, ref s2)) = *signers {
        let mut wallet = EthereumWallet::new(s1.clone());
        wallet.register_signer(s2.clone());

        let http_provider = ProviderBuilder::new()
            .connect(&args.http_rpc)
            .await
            .context("connect http")?;

        Some(Arc::new(
            ProviderBuilder::new()
                .wallet(wallet)
                .connect_provider(http_provider),
        ))
    } else {
        debug!("Skipping provider with wallet setup.");
        None
    };

    if let Some(token_contract) = args.faucet {
        if let Some((ref s1, ref s2)) = *signers {
            if let Some(provider_with_wallet_inner) = provider_with_wallet.clone() {
                call_faucet(
                    token_contract.parse()?,
                    provider_with_wallet_inner.clone(),
                    s1,
                    s2,
                )
                .await?;
            }
        } else {
            debug!("It's not possible to use the faucet without both pk1 and pk2")
        }
    }

    loop {
        tokio::select! {
            Some(signal) = signal_rx.recv() => {
                if let Some((ref s1, ref s2)) = *signers {
                    let process_result = process_signal(&signal, provider_with_wallet.clone().unwrap().clone(), s1, s2).await;
                    if let Ok(processing_status) = process_result {
                        match processing_status {
                            ProcessSignalStatus::Broadcast => {
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
                            }
                            _ => {}
                        }
                    } else if let Err(e) = process_result {
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
                        t if t == signal_topic.hash() => {
                            match serde_json::from_slice::<Signal>(&message.data) {
                                Ok(received_signal) => {
                                    info!(signal = %received_signal, "Received signal gossip");
                                    if let Some((ref s1, ref s2)) = *signers {
                                        let process_result = process_signal(&received_signal, provider_with_wallet.clone().unwrap().clone(), s1, s2).await;
                                        if let Ok(processing_status) = process_result {
                                            match processing_status {
                                                ProcessSignalStatus::Broadcast => {
                                                    match swarm
                                                                .behaviour_mut()
                                                                .gossipsub
                                                                .publish(signal_topic.clone(), serde_json::to_vec(&received_signal)?) {
                                                            Ok(_) => info!(signal = %received_signal, "Published signal"),
                                                            Err(gossipsub::PublishError::Duplicate) => {
                                                                    debug!(signal = %received_signal, "Signal already published (duplicate)");
                                                                }
                                                            Err(e) => return Err(e.into()),
                                                        }
                                                }
                                                _ => {}
                                            }
                                        } else if let Err(e) = process_result {
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

sol! {
    #[sol(rpc)]
    contract TokenContract {
        function balanceOf(address) public view returns (uint256);
        function mint() external;
        function transfer(address to, uint256 value) external returns (bool);
    }
}

pub enum ProcessSignalStatus {
    Processed,
    Broadcast,
}

pub async fn process_signal<P: Provider>(
    signal: &Signal,
    provider_with_wallet: Arc<P>,
    signer1: &PrivateKeySigner,
    signer2: &PrivateKeySigner,
) -> anyhow::Result<ProcessSignalStatus> {
    let addr_1 = signer1.address();
    let ether_balance_1 = provider_with_wallet.get_balance(addr_1).await?;
    let addr_2 = signer2.address();
    let ether_balance_2 = provider_with_wallet.get_balance(addr_2).await?;
    info!(%ether_balance_1, "current eth balance with address one");
    info!(%ether_balance_2, "current eth balance with address two");

    let token_contract = TokenContract::new(signal.token_contract, provider_with_wallet);
    let usdt_balance_1 = token_contract.balanceOf(addr_1).call().await?;
    let usdt_balance_2 = token_contract.balanceOf(addr_2).call().await?;
    info!(%usdt_balance_1, "current usdt balance with address one");
    info!(%usdt_balance_2, "current usdt balance with address two");

    if usdt_balance_1 > signal.transfer_amount {
        let _ = token_contract
            .transfer(signal.recipient, signal.transfer_amount)
            .from(signer1.address())
            .send()
            .await?;
    } else if usdt_balance_2 > signal.transfer_amount {
        let _ = token_contract
            .transfer(signal.recipient, signal.transfer_amount)
            .from(signer2.address())
            .send()
            .await?;
    } else {
        info!("Not enough balance to process the incoming request. Broadcasting...");
        return Ok(ProcessSignalStatus::Broadcast);
    }

    info!(
        "Processed the payment of {} tokens to {}",
        signal.transfer_amount, signal.recipient
    );

    Ok(ProcessSignalStatus::Processed)
}

pub async fn call_faucet<P: Provider>(
    token_addr: Address,
    provider_with_wallet: Arc<P>,
    signer1: &PrivateKeySigner,
    signer2: &PrivateKeySigner,
) -> anyhow::Result<()> {
    info!("minting tokens.");
    let token = TokenContract::new(token_addr, &provider_with_wallet);

    // First mint → signed by signer‑1 (default, but we set it explicitly for clarity)
    let a = token
        .mint()
        .from(signer1.address()) // chooses the key inside the wallet filler
        .send()
        .await?;
    info!("minted.");
    // Second mint → signed by signer‑2
    let b = token.mint().from(signer2.address()).send().await?;
    info!("minted.");
    let usdt_balance_1 = token.balanceOf(signer1.address()).call().await?;
    let usdt_balance_2 = token.balanceOf(signer2.address()).call().await?;

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
