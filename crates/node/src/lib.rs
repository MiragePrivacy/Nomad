use std::sync::atomic::AtomicBool;

use aes_gcm::{aead::Aead, KeyInit};
use alloy::signers::local::PrivateKeySigner;
use arrayref::array_ref;
use chrono::Utc;
use eyre::{bail, eyre, Context, Result};
use opentelemetry::{global::meter_provider, metrics::Counter};
use sha2::Digest;
use tokio::sync::mpsc::unbounded_channel;
use tracing::{error, info, info_span, instrument, warn, Span};

use nomad_ethereum::{ClientError, EthClient};
use nomad_p2p::P2pNode;
use nomad_pool::SignalPool;
use nomad_rpc::spawn_rpc_server;
use nomad_types::{ReceiptFormat, Signal, SignalPayload};
use nomad_vm::{NomadVm, VmSocket};
use zeroize::Zeroizing;

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
            token = ?signal.token_contract()
        );
        let _entered = span.enter();

        let res = process_signal(signal, &self.eth_client, &self.vm_socket).await;
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

/// Process signals sampled from the pool
async fn process_signal(
    signal: SignalPayload,
    eth_client: &EthClient,
    vm_socket: &VmSocket,
) -> Result<()> {
    let start_time = Utc::now().to_rfc3339();
    let signal = solve_and_decrypt_signal(vm_socket, signal).await?;

    info!("TODO: Validating escrow contract");
    // eth_client.validate_contract(signal, Vec::new());

    info!("Selecting active accounts");
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

    // Due to https://github.com/alloy-rs/alloy/issues/1318 continuing to poll in the
    // background, the provider holds onto the span and prevents sending to telemetry.
    // As a workaround, we only create a wallet provider while it's needed.
    let provider = eth_client.wallet_provider().await?;

    info!("Approving and bonding tokens to escrow");
    let [approve, bond] = eth_client.bond(&provider, eoa_1, signal.clone()).await?;

    info!("Transferring tokens to recipient");
    let transfer = eth_client
        .transfer(&provider, eoa_2, signal.clone())
        .await?;

    info!("Generating transfer proof");
    let proof = eth_client.generate_proof(Some(&signal), &transfer).await?;

    info!("Collecting rewards from escrow");
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

#[instrument(skip_all)]
async fn solve_and_decrypt_signal(vm_socket: &VmSocket, signal: SignalPayload) -> Result<Signal> {
    match signal {
        SignalPayload::Unencrypted(signal) => Ok(signal),
        SignalPayload::Encrypted(signal) => {
            if signal.data.len() < 12 {
                bail!("Data does not contain nonce prefix");
            }

            info!("Executing puzzle in vm");
            let k2 = vm_socket
                .run((signal.puzzle.to_vec(), Span::current()))
                .await
                .map_err(|e| eyre!("failed to receive puzzle response: {e}"))?
                .context("failed to execute puzzle")?;

            info!("Posting digest to relay");
            // send post request to relay address with sha256(k2)
            let digest = sha2::Sha256::digest(k2);
            let k1 = reqwest::Client::new()
                .post(signal.relay)
                .body(digest.to_vec())
                .send()
                .await
                .context("failed to request k1 from relay")?
                .bytes()
                .await
                .context("failed to ready k1 from relay")?;
            if k1.len() != 32 {
                bail!(
                    "Invalid relay response, expected 32 bytes, got {}",
                    k1.len()
                );
            }

            info!("Decrypting data");
            // sort k1 and k2 to determine order
            let mut sorted_shares = [*array_ref![k1, 0, 32], k2];
            sorted_shares.sort();
            // Compute sha256(k1 . k2) for 256 bit encryption key
            let key = Zeroizing::new(sha2::Sha256::digest(sorted_shares.as_flattened()));
            // The first 12 bytes in data contain the nonce
            let nonce = array_ref![signal.data, 0, 12].into();
            // Decrypt signal with aes-gcm
            let decrypted = aes_gcm::Aes256Gcm::new(&key)
                .decrypt(nonce, &signal.data[12..])
                .map_err(|e| eyre!("Failed to decrypt data: {e}"))?;

            // Decode unencrypted data as json
            // TODO: consider supporting more encodings
            let raw_signal: Signal =
                serde_json::from_slice(&decrypted).context("Failed to decode signal")?;
            if raw_signal.token_contract != signal.token_contract {
                warn!(
                    "decrypted signal doesn't match encrypted signal's token contract: {}",
                    raw_signal.token_contract
                );
            }
            Ok(raw_signal)
        }
    }
}

#[instrument(skip(receipt))]
async fn acknowledgement(url: &str, receipt: ReceiptFormat) {
    let res = reqwest::Client::new().post(url).json(&receipt).send().await;
    match res {
        Err(error) => warn!(?error, "Failed to send receipt"),
        Ok(_) => info!("Receipt sent successfully"),
    }
}
