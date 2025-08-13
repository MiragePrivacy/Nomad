use std::sync::Arc;

use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use anyhow::{anyhow, Context};
use chrono::Utc;
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{info, instrument, warn};
use tracing_subscriber::EnvFilter;

use nomad_ethereum::*;
use nomad_p2p::*;
use nomad_rpc::*;
use nomad_types::*;
use nomad_vm::*;

mod cli;

#[tokio::main]
#[instrument]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .try_init();

    let args = cli::Args::parse();

    let signers = build_signers(&args)?;
    let eth_client = EthClient::new(
        ProviderBuilder::new()
            .connect(&args.http_rpc)
            .await
            .context("connect http")?,
        signers,
    );

    let signal_pool = nomad_core::pool::SignalPool::default();
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();

    log_addresses().await;

    let _ = spawn_rpc_server(signal_tx, args.rpc_port).await;
    let _ = spawn_p2p(
        P2pConfig {
            bootstrap: Vec::new(),
            tcp: args.p2p_port.unwrap_or(0),
        },
        signal_rx,
        signal_pool.clone(),
    );
    let vm_socket = NomadVm::new().spawn();

    loop {
        let signal = signal_pool.sample().await;
        if let Err(e) = handle_signal(signal, &eth_client, &vm_socket).await {
            warn!("failed to handle signal: {e}");
        }
    }
}

/// Main logic for processing a signal sampled from the pool
async fn handle_signal<P: Provider + Clone>(
    signal: Signal,
    eth_client: &EthClient<P>,
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

/// Print local and remote ip addresses
async fn log_addresses() {
    if let Ok(local_ip) = local_ip_address::local_ip() {
        info!("Local Address: {local_ip}");
    }
    if let Ok(res) = reqwest::get("https://ifconfig.me").await {
        if let Ok(remote_ip) = res.text().await {
            info!("Remote Address: {remote_ip}");
        }
    }
}
