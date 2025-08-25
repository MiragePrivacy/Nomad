use std::sync::atomic::AtomicBool;

use alloy::signers::local::PrivateKeySigner;
use chrono::Utc;
use eyre::{eyre, Result};
use opentelemetry::{global::meter_provider, metrics::Counter};
use tokio::sync::mpsc::unbounded_channel;
use tracing::{error, field::Empty, info, info_span, instrument, warn, Span};

use nomad_ethereum::{ClientError, EthClient};
use nomad_p2p::P2pNode;
use nomad_pool::SignalPool;
use nomad_rpc::spawn_rpc_server;
use nomad_types::{ReceiptFormat, Signal};
use nomad_vm::{NomadVm, Opcode, VmSocket};

pub mod config;

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
            token = ?signal.token_contract,
            otel.status_code = Empty,
            otel.status_message = Empty
        );
        let _entered = span.enter();

        let res = process_signal(&signal, &self.eth_client, &self.vm_socket).await;

        // Success and error tracking
        if let Err(e) = res {
            error!("Failed to process signal");
            error!(error = format!("{e:#}"));
            self.failure.add(1, &[]);
        } else {
            info!(
                "Successfully processed payment of {} tokens to {}",
                signal.transfer_amount, signal.recipient
            );
            span.record("otel.status_code", "OK");
            span.record("otel.status_message", format!("{signal:?}"));
            self.success.add(1, &[]);
        }
        Ok(())
    }
}

/// Process signals sampled from the pool
async fn process_signal(
    signal: &Signal,
    eth_client: &EthClient,
    vm_socket: &VmSocket,
) -> Result<()> {
    let start_time = Utc::now().to_rfc3339();

    // Due to https://github.com/alloy-rs/alloy/issues/1318 continuing to poll in the
    // background, the provider holds onto the span and prevents sending to telemetry.
    // As a workaround, we only create a wallet provider while it's needed.
    let provider = eth_client.wallet_provider().await?;

    info!("[1/9] Executing puzzle in vm");
    // TODO: Include the puzzle bytes in the signal payload.
    //       For now, we'll just halt which returns a key of [0; 32]
    let puzzle = vec![Opcode::Halt as u8];
    let _k2 = vm_socket
        .run((puzzle, Span::current()))
        .await
        .map_err(|_| eyre!("failed to execute puzzle"))?;

    info!("[2/9] TODO: Getting k1 from relayer");

    info!("[3/9] TODO: Decrypting signal payload");

    info!("[4/9] TODO: Validating escrow contract");
    // eth_client.validate_contract(signal, Vec::new());

    info!("[5/9] Selecting active accounts");
    let [eoa_1, eoa_2] = 'inner: loop {
        match eth_client.select_accounts(signal.clone()).await {
            Ok(accounts) => break 'inner accounts,
            // We don't have at least two accounts with enough balance, wait until they are funded
            Err(e @ ClientError::NotEnoughEth(_, _, _)) => {
                warn!("{e}");
                let ClientError::NotEnoughEth(_, accounts, need) = e else {
                    unreachable!()
                };
                eth_client.wait_for_eth(&accounts, need).await?;
                continue 'inner;
            }
            Err(e) => Err(e)?,
        };
    };

    info!("[6/9] Approving and bonding tokens to escrow");
    let [approve, bond] = eth_client.bond(&provider, eoa_1, signal.clone()).await?;

    info!("[7/9] Transferring tokens to recipient");
    let transfer = eth_client
        .transfer(&provider, eoa_2, signal.clone())
        .await?;

    info!("[8/9] Generating transfer proof");
    let proof = eth_client.generate_proof(signal, &transfer).await?;

    info!("[9/9] Collecting rewards from escrow");
    let collect = eth_client
        .collect(
            &provider,
            eoa_1,
            signal.clone(),
            proof,
            transfer.block_number.unwrap(),
        )
        .await?;

    // Send receipt to client
    acknowledgement(
        &signal.acknowledgement_url,
        ReceiptFormat {
            start_time,
            end_time: Utc::now().to_rfc3339(),
            approval_transaction_hash: approve.transaction_hash.to_string(),
            bond_transaction_hash: bond.transaction_hash.to_string(),
            transfer_transaction_hash: transfer.transaction_hash.to_string(),
            collection_transaction_hash: collect.transaction_hash.to_string(),
        },
    )
    .await;

    Ok(())
}

#[instrument(skip(receipt))]
async fn acknowledgement(url: &str, receipt: ReceiptFormat) {
    let res = reqwest::Client::new().post(url).json(&receipt).send().await;
    match res {
        Err(e) => warn!(?e, "Failed to send receipt"),
        Ok(_) => info!("Receipt sent successfully"),
    }
}
