use aes_gcm::{aead::Aead, KeyInit};
use arrayref::array_ref;
use chrono::Utc;
use eyre::{bail, eyre, Context, Result};
use nomad_types::{ReceiptFormat, Signal, SignalPayload};
use sha2::Digest;
use tracing::{info, instrument, warn, Span};
use zeroize::Zeroizing;

use nomad_ethereum::{ClientError, EthClient};
use nomad_vm::VmSocket;

/// Process signals sampled from the pool
pub async fn handle_signal(
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
