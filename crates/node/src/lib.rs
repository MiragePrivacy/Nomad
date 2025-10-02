use std::sync::{atomic::AtomicBool, Arc};

use alloy::signers::local::PrivateKeySigner;
use eyre::{ContextCompat, Result};
use opentelemetry::{global::meter_provider, metrics::Counter};
use otel_instrument::tracer_name;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::warn;

use nomad_api::spawn_api_server;
use nomad_ethereum::EthClient;
use nomad_p2p::P2pNode;
use nomad_pool::SignalPool;

use crate::enclave::EnclaveRequest;

pub mod config;
mod enclave;

tracer_name!("nomad");

pub struct NomadNode {
    // Channel to send requests to stream worker
    tx: UnboundedSender<EnclaveRequest>,
    // Channel to read responses from stream worker
    rx: UnboundedReceiver<Vec<u8>>,
    keyshare_rx: UnboundedReceiver<(Vec<u8>, UnboundedSender<Vec<u8>>)>,
    signal_pool: SignalPool,
    eth_client: EthClient,
    _success: Counter<u64>,
    _failure: Counter<u64>,
}

impl NomadNode {
    /// Initialize the node
    pub async fn init(config: config::Config) -> Result<Self> {
        let read_only = config.enclave.num_accounts < 2;

        // Build eth client
        let accounts = config
            .enclave
            .debug_keys
            .iter()
            .map(|b| PrivateKeySigner::from_slice(b).expect("valid debug key"))
            .collect();
        let mut eth_client = EthClient::new(config.eth, accounts).await?;
        eth_client.enable_balance_metrics().await;

        // Spawn and initialize enclave
        let mut runner = enclave::EnclaveRunner::create_enclave(config.enclave).await?;
        let (report, attestation) = runner.initialize().await?;

        // Spawn request-response handler
        let (req_tx, req_rx) = unbounded_channel();
        let (res_tx, res_rx) = unbounded_channel();
        runner.spawn_handler(req_rx, res_tx);

        // Spawn api server
        let (signal_tx, signal_rx) = unbounded_channel();
        let (keyshare_tx, keyshare_rx) = unbounded_channel();
        let _ = spawn_api_server(
            config.api,
            config.p2p.bootstrap.is_empty(),
            read_only,
            report,
            attestation,
            signal_tx,
            keyshare_tx,
        )
        .await;

        // Create shared signal pool and spawn p2p server
        let signal_pool = SignalPool::new(65535);
        let read_only = Arc::new(AtomicBool::new(read_only));
        P2pNode::new(config.p2p, signal_pool.clone(), read_only, Some(signal_rx))?.spawn();

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
            tx: req_tx,
            rx: res_rx,
            keyshare_rx,
            signal_pool,
            eth_client,
            _success: success,
            _failure: failure,
        })
    }

    /// Run the node
    pub async fn run(mut self) -> Result<()> {
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
    pub async fn next(&mut self) -> Result<()> {
        tokio::select! {
            signal = self.signal_pool.sample() => {
                self.tx.send(EnclaveRequest::Signal(signal.0))?;
                self.rx.recv().await;
            }
            keyshare = self.keyshare_rx.recv() => {
                let (request, tx) = keyshare.context("Keyshare channel dropped")?;
                self.tx.send(EnclaveRequest::Keyshare(request.into()))?;
                // read response payload from stream and send back to the caller
                if let Some(response) = self.rx.recv().await {
                    tx.send(response)?;
                }
            }
        }
        Ok(())
    }
}
