mod cli;

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::{k256::elliptic_curve::rand_core::le, local::PrivateKeySigner},
    sol,
};
use anyhow::Context as _;
use chrono::Utc;
use clap::Parser;
use futures::StreamExt;
use libp2p::{
    gossipsub, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, instrument, warn};
use tracing_subscriber::EnvFilter;

use crate::cli::Args;
use nomad_rpc::*;
use nomad_types::*;

#[derive(NetworkBehaviour)]
pub struct GossipBehavior {
    pub gossipsub: gossipsub::Behaviour,
}

#[tokio::main]
#[instrument]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    dotenvy::dotenv().ok();

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
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

    let p2p_addr = match args.p2p_port {
        Some(port) => format!("/ip4/0.0.0.0/tcp/{port}"),
        None => "/ip4/0.0.0.0/tcp/0".to_string(),
    };
    swarm.listen_on(p2p_addr.parse()?)?;

    if let Some(ref addr) = args.peer {
        let remote: Multiaddr = addr.parse()?;
        swarm.dial(remote)?;
        info!("Connected to peer: {}", addr);
    }

    let (signal_tx, mut signal_rx) = mpsc::unbounded_channel();
    let _ = spawn_rpc_server(signal_tx, args.rpc_port).await;


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
        info!("Running in read-only mode (no private keys provided)");
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
            warn!("Cannot use faucet without both pk1 and pk2");
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
                                        Ok(_) => info!("Published signal: {}", signal),
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
                    info!("Read-only mode: forwarding signal without processing");
                    match swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(signal_topic.clone(), serde_json::to_vec(&signal)?) {
                Ok(_) => info!("Published signal: {}", signal),
                Err(gossipsub::PublishError::Duplicate) => {
                        debug!(signal = %signal, "Signal already published (duplicate)");
                    }
                Err(e) => return Err(e.into()),
            }
                }
            }

            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } =>
                    info!("Listening on {}", address),

                SwarmEvent::Behaviour(GossipBehaviorEvent::Gossipsub(gossipsub::Event::Message { message, propagation_source, ..})) => {
                    match message.topic {
                        t if t == signal_topic.hash() => {
                            match serde_json::from_slice::<Signal>(&message.data) {
                                Ok(received_signal) => {
                                    info!("Received signal: {}", received_signal);
                                    info!("From peer: {:?}", propagation_source);
                                    if let Some((ref s1, ref s2)) = *signers {
                                        let process_result = process_signal(&received_signal, provider_with_wallet.clone().unwrap().clone(), s1, s2).await;
                                        if let Ok(processing_status) = process_result {
                                            match processing_status {
                                                ProcessSignalStatus::Broadcast => {
                                                    match swarm
                                                                .behaviour_mut()
                                                                .gossipsub
                                                                .publish(signal_topic.clone(), serde_json::to_vec(&received_signal)?) {
                                                            Ok(_) => info!("Published signal: {}", received_signal),
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
                                        info!("Read-only mode: forwarding received signal");
                                        match swarm
                                        .behaviour_mut()
                                        .gossipsub
                                        .publish(signal_topic.clone(), serde_json::to_vec(&received_signal)?) {
                                    Ok(_) => info!("Published signal: {}", received_signal),
                                    Err(gossipsub::PublishError::Duplicate) => {
                                            debug!(signal = %received_signal, "Signal already published (duplicate)");
                                        }
                                    Err(e) => return Err(e.into()),
                                }

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
    signal: &Signal,
    provider_with_wallet: Arc<P>,
    signer1: &PrivateKeySigner,
    signer2: &PrivateKeySigner,
) -> anyhow::Result<ProcessSignalStatus> {
    let addr_1 = signer1.address();
    let ether_balance_1 = provider_with_wallet.get_balance(addr_1).await?;
    let addr_2 = signer2.address();
    let ether_balance_2 = provider_with_wallet.get_balance(addr_2).await?;
    info!(
        "Address 1 ETH balance: {} ETH",
        ether_balance_1.to_string().parse::<f64>().unwrap_or(0.0) / 1e18
    );
    info!(
        "Address 2 ETH balance: {} ETH",
        ether_balance_2.to_string().parse::<f64>().unwrap_or(0.0) / 1e18
    );

    let token_contract = TokenContract::new(signal.token_contract, &provider_with_wallet);
    let usdt_balance_1 = token_contract.balanceOf(addr_1).call().await?;
    let usdt_balance_2 = token_contract.balanceOf(addr_2).call().await?;
    info!("Address 1 token balance: {}", usdt_balance_1);
    info!("Address 2 token balance: {}", usdt_balance_2);
    let bond_amount = signal
        .reward_amount
        .checked_mul(U256::from(52))
        .unwrap()
        .checked_div(U256::from(100))
        .unwrap();

    let min_eth_balance = U256::from(10_000_000_000_000_000u64); // 0.01 ETH for gas fees

    let escrow = Escrow::new(signal.escrow_contract, &provider_with_wallet);
    let is_already_bonded = escrow.is_bonded().call().await?;

    if is_already_bonded {
        info!("Escrow already bonded, broadcasting signal to network");
        return Ok(ProcessSignalStatus::Broadcast);
    }

    // question: do the failures here terminate the program? at most they should only be logged (at least that is the case for the contract interactions)

    let (transfer_signer, bond_signer) = if usdt_balance_2 > signal.transfer_amount
        && usdt_balance_1 > bond_amount
        && ether_balance_1 > min_eth_balance
        && ether_balance_2 > min_eth_balance
    {
        (signer2, signer1)
    } else if usdt_balance_1 > signal.transfer_amount
        && usdt_balance_2 > bond_amount
        && ether_balance_1 > min_eth_balance
        && ether_balance_2 > min_eth_balance
    {
        (signer1, signer2)
    } else {
        info!("Insufficient balance to process request, broadcasting to network");
        return Ok(ProcessSignalStatus::Broadcast);
    };

    let start_time = Utc::now().to_rfc3339();
    info!("Processing signal transactions...");

    info!("[1/4] Approving tokens for escrow...");
    let approval_tx = token_contract
        .approve(signal.escrow_contract, bond_amount)
        .from(bond_signer.address())
        .send()
        .await?
        .with_required_confirmations(1)
        .watch()
        .await?;

    info!("[2/4] Bonding to escrow with {} tokens..", bond_amount);
    let bond_tx = escrow
        .bond(bond_amount)
        .from(bond_signer.address())
        .send()
        .await?
        .with_required_confirmations(1)
        .watch()
        .await?;

    info!("[3/4] Transferring tokens to recipient...");
    let transfer_tx = token_contract
        .transfer(signal.recipient, signal.transfer_amount)
        .from(transfer_signer.address())
        .send()
        .await?
        .with_required_confirmations(1)
        .watch()
        .await?;

    info!("[4/4] Collecting from escrow...");
    let collect_tx = escrow
        .collect()
        .from(bond_signer.address())
        .send()
        .await?
        .with_required_confirmations(1)
        .watch()
        .await?;

    let end_time = Utc::now().to_rfc3339();

    let receipt = ReceiptFormat {
        start_time,
        end_time,
        approval_transaction_hash: format!("{:?}", approval_tx),
        bond_transaction_hash: format!("{:?}", bond_tx),
        transfer_transaction_hash: format!("{:?}", transfer_tx),
        collection_transaction_hash: format!("{:?}", collect_tx),
    };

    let client = reqwest::Client::new();
    if let Err(e) = client
        .post(&signal.acknowledgement_url)
        .json(&receipt)
        .send()
        .await
    {
        warn!(
            "Failed to send receipt to {}: {}",
            signal.acknowledgement_url, e
        );
    } else {
        info!(
            "Receipt sent successfully to {}",
            signal.acknowledgement_url
        );
    }

    info!(
        "Successfully processed payment of {} tokens to {}",
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
    info!("Minting tokens from faucet...");
    let token = TokenContract::new(token_addr, &provider_with_wallet);

    info!("Minting tokens for address 1: {}", signer1.address());
    let a = token.mint().from(signer1.address()).send().await?;
    info!("Mint successful for address 1");

    info!("Minting tokens for address 2: {}", signer2.address());
    let b = token.mint().from(signer2.address()).send().await?;
    info!("Mint successful for address 2");
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
            info!(
                "Using wallet addresses: {} and {}",
                s1.address(),
                s2.address()
            );
            Ok(Some((s1, s2)))
        }
        // neither present → run in read-only mode
        (None, None) => Ok(None),
        // one present, one missing → treat as a configuration error
        _ => anyhow::bail!("Supply *both* --pk1 and --pk2 or neither"),
    }
}
