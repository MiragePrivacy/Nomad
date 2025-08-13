use std::sync::Arc;

use alloy::{primitives::Address, providers::Provider, signers::local::PrivateKeySigner};
use anyhow::{anyhow, Context};
use chrono::Utc;
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{info, instrument, warn};
use tracing_subscriber::EnvFilter;

use nomad_core::*;
use nomad_ethereum::*;
use nomad_p2p::*;
use nomad_rpc::*;
use nomad_types::*;
use nomad_vm::*;

mod cli;
mod config;

#[tokio::main]
#[instrument]
async fn main() -> anyhow::Result<()> {
    // Parse cli arguments and app setup
    dotenvy::dotenv().ok();
    let args = cli::Args::parse();

    // Setup logging filters
    let env_filter = EnvFilter::builder().parse_lossy(match std::env::var("RUST_LOG") {
        // Environment override
        Ok(filter) => filter,
        // Default which is directed by the verbosity flag
        Err(_) => match args.verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
        .to_string(),
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .try_init();

    let config = config::Config::load(None).merge_args(&args);

    // Log local and remote ip addresses
    if let Ok(local_ip) = local_ip_address::local_ip() {
        info!("Local Address: {local_ip}");
    }
    if let Ok(res) = reqwest::get("https://ifconfig.me/ip").await {
        if let Ok(remote_ip) = res.text().await {
            info!("Remote Address: {remote_ip}");
        }
    }

    // Build eth clients
    let signers = build_signers(&args)?;
    let eth_client = EthClient::new(config.eth, signers).await?;

    // Setup background server tasks, shared signal pool
    let signal_pool = SignalPool::new();
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();
    let _ = spawn_rpc_server(config.rpc, signal_tx).await;
    let _ = spawn_p2p(config.p2p, signal_rx, signal_pool.clone());

    // Spawn a vm worker thread
    let vm_socket = NomadVm::new().spawn();

    loop {
        // Get a random signal from the pool and process it
        let signal = signal_pool.sample().await;
        if let Err(e) = handle_signal(signal, &eth_client, &vm_socket).await {
            warn!("failed to handle signal: {e}");
        }
    }
}

/// Process signals sampled from the pool
async fn handle_signal(
    signal: Signal,
    eth_client: &EthClient,
    vm_socket: &VmSocket,
) -> anyhow::Result<()> {
    let start_time = Utc::now().to_rfc3339();

    // TODO: Include the puzzle bytes in the signal payload
    info!("[0/3] Executing puzzle in vm");
    let puzzle = Vec::new();
    let _k2 = vm_socket
        .run(puzzle)
        .await
        .map_err(|_| anyhow!("failed to execute puzzle"))?;

    // TODO:
    //   - get k1 from relayer
    //   - decrypt signal
    //   - re-obfuscate contract for validation

    // validate contract
    // eth_client.validate_contract(signal, Vec::new());

    // select ideal accounts
    let [eoa_1, eoa_2] = eth_client.select_accounts(signal.clone()).await?;

    info!("[1/3] Approving and bonding tokens to escrow");
    let (approve, bond) = eth_client.bond(eoa_1, signal.clone()).await?;

    info!("[2/3] Transferring tokens to recipient");
    let (transfer, proof) = eth_client.transfer(eoa_2, signal.clone()).await?;

    info!("[3/3] Collecting rewards from escrow");
    let collect = eth_client.collect(eoa_1, signal.clone(), proof).await?;

    // Send receipt to client
    let client = reqwest::Client::new();
    let res = client
        .post(&signal.acknowledgement_url)
        .json(&ReceiptFormat {
            start_time,
            end_time: Utc::now().to_rfc3339(),
            approval_transaction_hash: approve.transaction_hash.to_string(),
            bond_transaction_hash: bond.transaction_hash.to_string(),
            transfer_transaction_hash: transfer.transaction_hash.to_string(),
            collection_transaction_hash: collect.transaction_hash.to_string(),
        })
        .send()
        .await;
    match res {
        Err(e) => {
            warn!(
                "Failed to send receipt to {}: {}",
                signal.acknowledgement_url, e
            );
        }
        Ok(_) => {
            info!(
                "Receipt sent successfully to {}",
                signal.acknowledgement_url
            );
        }
    }

    info!(
        "Successfully processed payment of {} tokens to {}",
        signal.transfer_amount, signal.recipient
    );
    Ok(())
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

    // Wait for transactions to be approved
    a.watch().await?;
    b.watch().await?;

    let usdt_balance_1 = token.balanceOf(signer1.address()).call().await?;
    let usdt_balance_2 = token.balanceOf(signer2.address()).call().await?;
    info!("Address 1: {usdt_balance_1}, Address 2: {usdt_balance_2}");

    Ok(())
}

fn build_signers(args: &cli::Args) -> anyhow::Result<Vec<PrivateKeySigner>> {
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
            Ok(vec![s1, s2])
        }
        // neither present → run in read-only mode
        (None, None) => Ok(vec![]),
        // one present, one missing → treat as a configuration error
        _ => anyhow::bail!("Supply *both* --pk1 and --pk2 or neither"),
    }
}
