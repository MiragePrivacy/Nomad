use std::sync::atomic::AtomicBool;

use alloy::signers::local::PrivateKeySigner;

use eyre::Result;
use opentelemetry::{global::meter_provider, metrics::Counter};
use tokio::sync::mpsc::unbounded_channel;
use tracing::{error, info, info_span, warn};

use nomad_ethereum::EthClient;
use nomad_p2p::P2pNode;
use nomad_pool::SignalPool;
use nomad_rpc::spawn_rpc_server;

use nomad_vm::{NomadVm, VmSocket};
pub mod config;
mod execute;

pub struct NomadNode {
    signal_pool: SignalPool,
    eth_client: EthClient,
    vm_socket: VmSocket,
    success: Counter<u64>,
    failure: Counter<u64>,
}

impl NomadNode {
    pub async fn init(config: config::Config, signers: Vec<PrivateKeySigner>) -> Result<Self> {
        // Spawn rpc server
        let (signal_tx, signal_rx) = unbounded_channel();
        let _ = spawn_rpc_server(config.rpc, signal_tx).await;

        // If we dont have two keys, don't process any signals
        let read_only = signers.is_empty();
        if read_only {
            warn!("No signers provided; running node in read-only mode!");
        }
        let read_only = AtomicBool::new(read_only).into();

        // Create shared signal pool and spawn p2p server
        let signal_pool = SignalPool::new(65535);
        P2pNode::new(config.p2p, signal_pool.clone(), read_only, Some(signal_rx))?.spawn();

        // Build eth client
        let eth_client = EthClient::new(config.eth, signers).await?;

        // Spawn a vm worker thread
        let vm_socket = NomadVm::new(config.vm.max_cycles).spawn();

        // Setup metrics
        let meter = meter_provider().meter("nomad");
        let up = meter.u64_gauge("up").with_description("Node is up").build();
        up.record(1, &[]);
        let success = meter
            .u64_counter("signal_success")
            .with_description("Number of successfully processed signals")
            .build();
        let failure = meter
            .u64_counter("signal_failure")
            .with_description("Number of failures when processing signals")
            .build();

        Ok(Self {
            signal_pool,
            eth_client,
            vm_socket,
            success,
            failure,
        })
    }

    pub async fn run(self) -> Result<()> {
        loop {
            self.next().await?;
        }
    }

    pub async fn next(&self) -> Result<()> {
        let signal = self.signal_pool.sample().await;

        let span = info_span!(
            "process_signal",
            token = ?signal.token_contract()
        );
        let _entered = span.enter();

        let res = execute::handle_signal(signal, &self.eth_client, &self.vm_socket).await;
        if let Err(e) = res {
            error!("Failed to process signal");
            error!(error = format!("{e:#}"));
            self.failure.add(1, &[]);
        } else {
            info!("Successfully processed signal");
            self.success.add(1, &[]);
        }
        Ok(())
    }
}
