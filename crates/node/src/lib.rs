use std::sync::{atomic::AtomicBool, Arc};

use alloy::signers::local::PrivateKeySigner;
use eyre::Result;
use nomad_types::SignalPayload;
use opentelemetry::{global::meter_provider, metrics::Counter};
use otel_instrument::tracer_name;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tracing::warn;

use nomad_api::spawn_api_server;
use nomad_ethereum::EthClient;
use nomad_p2p::P2pNode;
use nomad_pool::SignalPool;

pub mod config;
mod enclave;
mod execute;

tracer_name!("nomad");

pub struct NomadNode {
    tx: UnboundedSender<SignalPayload>,
    signal_pool: SignalPool,
    eth_client: EthClient,
    _success: Counter<u64>,
    _failure: Counter<u64>,
}

impl NomadNode {
    /// Initialize the node
    pub async fn init(config: config::Config) -> Result<Self> {
        let read_only = config.enclave.num_accounts < 2;

        // Spawn enclave
        let (tx, rx) = unbounded_channel();
        let (enclave_tx, _enclave_rx) = unbounded_channel();
        let (publickey, is_debug, attestation) =
            enclave::spawn_enclave(&config.enclave, rx, enclave_tx).await?;

        // Spawn api server
        // TODO: add attestation endpoint with report and enclave public key
        let (signal_tx, signal_rx) = unbounded_channel();
        let _ = spawn_api_server(
            config.api,
            config.p2p.bootstrap.is_empty(),
            read_only,
            attestation,
            publickey,
            is_debug,
            signal_tx,
        )
        .await;

        // Create shared signal pool and spawn p2p server
        let signal_pool = SignalPool::new(65535);
        let read_only = Arc::new(AtomicBool::new(read_only));
        P2pNode::new(config.p2p, signal_pool.clone(), read_only, Some(signal_rx))?.spawn();

        // Build eth client
        let accounts = config
            .enclave
            .debug_keys
            .iter()
            .map(|b| PrivateKeySigner::from_slice(b).expect("valid debug key"))
            .collect();
        let mut eth_client = EthClient::new(config.eth, accounts).await?;
        eth_client.enable_balance_metrics().await;

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
            tx,
            signal_pool,
            eth_client,
            _success: success,
            _failure: failure,
        })
    }

    /// Run the node
    pub async fn run(self) -> Result<()> {
        // Spawn background balance monitoring task if Uniswap is enabled
        if let Some(check_interval) = self.eth_client.swap_check_interval() {
            let eth_client_clone = self.eth_client.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(check_interval);
                loop {
                    interval.tick().await;
                    if let Err(e) = eth_client_clone.maintain_eth_balances().await {
                        warn!("Failed to maintain ETH balances: {}", e);
                    }
                }
            });
        }

        // Spawn background task for balance metrics reporting
        let eth_client_for_metrics = self.eth_client.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60)); // Report every minute
            loop {
                interval.tick().await;
                // Update all accounts periodically
                if let Err(e) = eth_client_for_metrics.report_balance_metrics().await {
                    warn!("Failed to report balance metrics: {}", e);
                }
            }
        });

        loop {
            self.next().await?;
        }
    }

    /// Handle the next signal from the pool (blocking until one is available)
    pub async fn next(&self) -> Result<()> {
        let signal = self.signal_pool.sample().await;
        self.tx.send(signal)?;
        Ok(())
    }
}
