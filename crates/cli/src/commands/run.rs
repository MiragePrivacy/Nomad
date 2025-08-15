use alloy::signers::local::PrivateKeySigner;
use chrono::Utc;
use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use nomad_ethereum::EthClient;
use nomad_p2p::spawn_p2p;
use nomad_pool::SignalPool;
use nomad_rpc::spawn_rpc_server;
use nomad_types::{ReceiptFormat, Signal};
use nomad_vm::{NomadVm, VmSocket};
use reqwest::Url;
use tokio::sync::mpsc::unbounded_channel;
use tracing::{info, warn};

use crate::config::Config;

#[derive(Parser)]
pub struct RunArgs {
    /// Port for the RPC server
    #[arg(short, long)]
    pub rpc_port: Option<u16>,
    /// Port for the p2p node
    #[arg(short, long)]
    pub p2p_port: Option<u16>,
    /// Multiaddr of a peer to connect to
    #[arg(long)]
    pub peer: Option<String>,
    /// HTTP RPC URL for sending transactions
    #[arg(long, env("HTTP_RPC"))]
    pub http_rpc: Option<Url>,
}

impl RunArgs {
    /// Apply argument overrides to configuration
    fn override_config(&self, config: &mut Config) {
        if let Some(rpc) = self.http_rpc.clone() {
            config.eth.rpc = rpc;
        }
        if let Some(port) = self.rpc_port {
            config.rpc.port = port;
        }
        if let Some(port) = self.p2p_port {
            config.p2p.tcp = port;
        }
        if let Some(peer) = self.peer.clone() {
            config.p2p.bootstrap = vec![peer.parse().unwrap()];
        }
    }

    /// Run the node!
    pub async fn execute(self, mut config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        self.override_config(&mut config);

        // Log local and remote ip addresses
        if let Ok(local_ip) = local_ip_address::local_ip() {
            info!("Local Address: {local_ip}");
        }
        if let Ok(res) = reqwest::get("https://ifconfig.me/ip").await {
            if let Ok(remote_ip) = res.text().await {
                info!("Remote Address: {remote_ip}");
            }
        }

        // Setup background server tasks, shared signal pool
        let signal_pool = SignalPool::new(65535);
        let (signal_tx, signal_rx) = unbounded_channel();
        let _ = spawn_rpc_server(config.rpc, signal_tx).await;

        // If we dont have two keys, don't process any signals
        let read_only = signers.is_empty();
        if read_only {
            warn!("No signers provided; running node in read-only mode!");
        }
        let _ = spawn_p2p(config.p2p, read_only, signal_rx, signal_pool.clone());

        // Build eth clients
        let eth_client = EthClient::new(config.eth, signers).await?;

        // Spawn a vm worker thread
        let vm_socket = NomadVm::new().spawn();

        // Main event loop
        loop {
            let signal = signal_pool.sample().await;
            if let Err(e) = handle_signal(signal, &eth_client, &vm_socket).await {
                warn!("failed to handle signal: {e}");
            }
        }
    }
}

/// Process signals sampled from the pool
async fn handle_signal(signal: Signal, eth_client: &EthClient, vm_socket: &VmSocket) -> Result<()> {
    let start_time = Utc::now().to_rfc3339();

    // TODO: Include the puzzle bytes in the signal payload
    info!("[0/3] Executing puzzle in vm");
    let puzzle = Vec::new();
    let _k2 = vm_socket
        .run(puzzle)
        .await
        .map_err(|_| eyre!("failed to execute puzzle"))?;

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
