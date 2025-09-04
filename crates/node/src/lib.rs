use std::sync::{atomic::AtomicBool, Arc};

use alloy::signers::local::PrivateKeySigner;
use eyre::Result;
use opentelemetry::{global::meter_provider, metrics::Counter};
use otel_instrument::tracer_name;
use tokio::sync::mpsc::unbounded_channel;
use tracing::{error, info, warn};

use nomad_api::spawn_api_server;
use nomad_ethereum::{ClientError, EthClient};
use nomad_p2p::P2pNode;
use nomad_pool::SignalPool;
use nomad_vm::{NomadVm, VmSocket};

pub mod config;
mod execute;

tracer_name!("nomad");

pub struct NomadNode {
    signal_pool: SignalPool,
    eth_client: EthClient,
    vm_socket: VmSocket,
    success: Counter<u64>,
    failure: Counter<u64>,
}

impl NomadNode {
    /// Initialize the node with p2p, an eth client, and a vm worker thread
    pub async fn init(config: config::Config, signers: Vec<PrivateKeySigner>) -> Result<Self> {
        // If we dont have two keys, don't process any signals
        let read_only = signers.is_empty();
        if read_only {
            warn!("No signers provided; running node in read-only mode!");
        }

        // Spawn api server
        let (signal_tx, signal_rx) = unbounded_channel();
        let _ = spawn_api_server(
            config.api,
            config.p2p.bootstrap.is_empty(),
            read_only,
            signal_tx,
        )
        .await;

        // Create shared signal pool and spawn p2p server
        let signal_pool = SignalPool::new(65535);
        let read_only = Arc::new(AtomicBool::new(read_only));
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
            .with_description("Number of successfully executed signals")
            .build();
        let failure = meter
            .u64_counter("signal_failure")
            .with_description("Number of failures when executing signals")
            .build();

        Ok(Self {
            signal_pool,
            eth_client,
            vm_socket,
            success,
            failure,
        })
    }

    /// Run the node
    pub async fn run(self) -> Result<()> {
        loop {
            if let Err(e) = self.next().await {
                if let Ok(ClientError::NotEnoughEth(_, accounts, need)) = e.downcast() {
                    // wait for eth to be transferred
                    self.eth_client.wait_for_eth(&accounts, need).await?;
                }
            }
        }
    }

    /// Handle the next signal from the pool (blocking until one is available)
    pub async fn next(&self) -> Result<()> {
        let signal = self.signal_pool.sample().await;
        execute::execute_signal(signal, &self.eth_client, &self.vm_socket)
            .await
            .inspect(|_| {
                info!("Successfully executed signal");
                self.success.add(1, &[]);
            })
            .inspect_err(|e| {
                error!("Failed to execute signal: {e:#}");
                self.failure.add(1, &[]);
            })
    }
}
