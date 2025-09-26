// use aes_gcm::{aead::AeadMutInPlace, KeyInit};
// use arrayref::array_ref;
// use chrono::Utc;
// use eyre::{bail, eyre, Context as _, Report, Result};
// use opentelemetry::{
//     global,
//     trace::{get_active_span, FutureExt, TraceContextExt, Tracer},
//     Context, KeyValue,
// };
// use otel_instrument::instrument;
// use reqwest::Url;
// use sha3::Digest;
// use tracing::{error, info, warn};
// use zeroize::Zeroizing;

// use nomad_ethereum::EthClient;
// use nomad_types::{ReceiptFormat, Signal, SignalPayload};
// use nomad_vm::VmSocket;

// use crate::_OTEL_TRACER_NAME;

// /// Wrapper around the implementation that optionally traces with the given trace id
// pub async fn execute_signal(
//     signal: SignalPayload,
//     eth_client: &EthClient,
//     vm_socket: &VmSocket,
// ) -> Result<()> {
//     // Initialize the span, optionally using the signal's trace id
//     let tracer = global::tracer(_OTEL_TRACER_NAME);
//     let mut builder = tracer.span_builder("execute_signal");
//     if let Some(trace_id) = signal.trace_id() {
//         builder = builder.with_trace_id(u128::from_be_bytes(trace_id).into());
//     }
//     let span = builder
//         .with_attributes([KeyValue::new("token", signal.token_contract().to_string())])
//         .start(&tracer);
//     async move {
//         execute_signal_impl(signal, eth_client, vm_socket)
//             .await
//             .inspect_err(|e: &Report| {
//                 // Mark span with errors if we have any
//                 get_active_span(|span| {
//                     span.set_status(opentelemetry::trace::Status::error(format!("{e:#}")));
//                     span.record_error(e.as_ref());
//                 })
//             })
//     }
//     .with_context(Context::current_with_span(span))
//     .await
// }

// /// Process signals sampled from the pool
// pub async fn execute_signal_impl(
//     signal: SignalPayload,
//     eth_client: &EthClient,
//     vm_socket: &VmSocket,
// ) -> Result<()> {
//     let start_time = Utc::now().to_rfc3339();
//     let signal = solve_and_decrypt_signal(vm_socket, signal).await?;

//     info!("Validating escrow contract");
//     eth_client.validate_contract(&signal).await?;

//     info!("Selecting active accounts");
//     let [eoa_1, eoa_2] = eth_client.select_accounts(signal.clone()).await?;

//     // Due to https://github.com/alloy-rs/alloy/issues/1318 continuing to poll in the
//     // background, the provider holds onto the span and prevents sending to telemetry.
//     // As a workaround, we only create a wallet provider while it's needed.
//     let provider = eth_client.wallet_provider().await?;

//     info!("Approving and bonding tokens to escrow");
//     let [approve, bond] = eth_client.bond(&provider, eoa_1, signal.clone()).await?;

//     info!("Transferring tokens to recipient");
//     let transfer = eth_client
//         .transfer(&provider, eoa_2, signal.clone())
//         .await?;

//     // Send receipt to client
//     acknowledgement(
//         signal.acknowledgement_url.clone(),
//         ReceiptFormat {
//             start_time,
//             end_time: Utc::now().to_rfc3339(),
//             approval_transaction_hash: approve.transaction_hash.to_string(),
//             bond_transaction_hash: bond.transaction_hash.to_string(),
//             transfer_transaction_hash: transfer.transaction_hash.to_string(),
//         },
//     )
//     .await?;

//     info!("Generating transfer proof");
//     let proof = eth_client.generate_proof(Some(&signal), &transfer).await?;

//     info!("Collecting rewards from escrow");
//     eth_client
//         .collect(
//             &provider,
//             eoa_1,
//             signal,
//             proof,
//             transfer.block_number.unwrap(),
//         )
//         .await
//         .inspect_err(|_| {
//             error!("Executed transfer but failed to collect rewards and transfer funds")
//         })?;

//     // Update balance metrics for both accounts used in the transaction
//     if let Err(e) = eth_client
//         .update_account_balance_metrics(&[eoa_1, eoa_2])
//         .await
//     {
//         warn!(
//             "Failed to update balance metrics after signal execution: {}",
//             e
//         );
//     }

//     info!("Successfully executed signal");
//     Ok(())
// }

// /// Decrypt signal payloads into an executable request
// #[instrument(skip_all)]
// async fn solve_and_decrypt_signal(vm_socket: &VmSocket, signal: SignalPayload) -> Result<Signal> {
//     match signal {
//         SignalPayload::Unencrypted(signal) | SignalPayload::TracedUnencrypted(signal, _) => {
//             Ok(signal)
//         }
//         SignalPayload::Encrypted(mut signal) | SignalPayload::TracedEncrypted(mut signal, _) => {
//             if signal.data.len() < 12 {
//                 bail!("Encrypted data does not contain enough bytes for a nonce prefix");
//             }
//             if signal.data.len() < 24 {
//                 // TODO: calculate minimum encrypted signal size
//                 bail!("Encrypted data does not contain enough bytes for a signal");
//             }

//             info!("Executing puzzle in vm");
//             let k2 = vm_socket
//                 .run((signal.puzzle.to_vec(), Context::current()))
//                 .await
//                 .map_err(|e| eyre!("failed to receive puzzle response: {e}"))?
//                 .context("failed to execute puzzle")?;

//             info!("Posting digest to relay");
//             let digest = sha3::Sha3_256::digest(k2);
//             let k1 = reqwest::Client::new()
//                 .post(signal.relay)
//                 .body(digest.to_vec())
//                 .send()
//                 .await
//                 .context("failed to request k1 from relay")?
//                 .bytes()
//                 .await
//                 .context("failed to ready k1 from relay")?;
//             if k1.len() != 32 {
//                 bail!(
//                     "Invalid relay response, expected 32 bytes, got {}",
//                     k1.len()
//                 );
//             }

//             info!("Decrypting data");
//             // The first 12 bytes in data contain the nonce
//             let nonce_bytes = signal.data.split_to(12);
//             // The rest of the payload is our ciphertext
//             let mut data = signal.data.split_to(signal.data.len()).to_vec();
//             // sort k1 and k2 to determine hashing order
//             let mut sorted_shares = [*array_ref![k1, 0, 32], k2];
//             sorted_shares.sort();
//             // Compute sha356(k1 . k2) for 256 bit encryption key
//             let key = Zeroizing::new(sha3::Sha3_256::digest(sorted_shares.as_flattened()));
//             // Decrypt signal with aes-gcm
//             aes_gcm::Aes256Gcm::new(&key)
//                 .decrypt_in_place(array_ref![nonce_bytes, 0, 12].into(), &[], &mut data)
//                 .map_err(|e| eyre!("Failed to decrypt data: {e}"))?;

//             info!("Parsing raw signal");
//             // TODO: consider supporting more encodings
//             let raw_signal: Signal =
//                 serde_json::from_slice(&data).context("Failed to decode signal")?;
//             if raw_signal.token_contract != signal.token_contract {
//                 warn!(
//                     inner_token = ?raw_signal.token_contract,
//                     "decrypted signal doesn't match encrypted signal's token contract",
//                 );
//             }
//             Ok(raw_signal)
//         }
//     }
// }

// /// Send acknowledgement receipt to the signal producer
// #[instrument(skip(receipt))]
// async fn acknowledgement(url: Url, receipt: ReceiptFormat) -> Result<()> {
//     let res = reqwest::Client::new().post(url).json(&receipt).send().await;
//     match res {
//         Err(error) => warn!(?error, "Failed to send receipt"),
//         Ok(_) => info!("Receipt sent successfully"),
//     }
//     Ok(())
// }
