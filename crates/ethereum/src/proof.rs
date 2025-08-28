use alloy::{
    consensus::{Receipt, ReceiptEnvelope, ReceiptWithBloom, TxType},
    eips::Encodable2718 as _,
    primitives::{Bytes, Log, U256},
    providers::Provider,
    rlp::{BufMut, Encodable},
    rpc::types::TransactionReceipt,
};
use alloy_trie::{proof::ProofRetainer, root::adjust_index_for_rlp, HashBuilder, Nibbles};
use tracing::{instrument, trace};

use nomad_types::Signal;

use crate::{ClientError, Escrow, EthClient, IERC20};

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("Transaction not found")]
    TransactionNotFound,
    #[error("Log not found in receipt")]
    LogNotFound,
    #[error("Log out of bounds")]
    LogIndexOutOfBounds,
    #[error("Log mismatched")]
    LogMismatch,
    #[error("Decoding error")]
    Decoding,
    #[error("Invalid root")]
    InvalidRoot,
}

impl EthClient {
    /// Creates a new `ProofInput` with the given block hash, transaction index, and optional log index.
    #[instrument(skip_all, fields(
        block_num = ?receipt.block_number.unwrap(),
        block_hash = ?receipt.block_hash.unwrap(),
        tx = ?receipt.transaction_hash
    ))]
    pub async fn generate_proof(
        &self,
        signal: Option<&Signal>,
        receipt: &TransactionReceipt,
    ) -> Result<Escrow::ReceiptProof, ClientError> {
        // Find the target transfer event and its LOCAL index within the transaction
        let mut log_idx = None;
        for (idx, log) in receipt.logs().iter().enumerate() {
            if let Ok(decoded) = log.log_decode::<IERC20::Transfer>() {
                let matches_signal = if let Some(signal) = signal {
                    decoded.address() == signal.token_contract
                        && decoded.data().to == signal.recipient
                        && decoded.data().value == signal.transfer_amount
                } else {
                    true // If no signal provided, accept any transfer event
                };
                
                if matches_signal {
                    log_idx = Some(idx);
                    break;
                }
            }
        }
        let Some(log_idx) = log_idx else {
            return Err(ProofError::LogNotFound.into());
        };

        trace!(?receipt, "Building proof for log index {log_idx}");

        // Get the block, build receipts trie
        let block_hash = receipt.block_hash.unwrap();
        let Some(block) = self.read_provider.get_block_by_hash(block_hash).await? else {
            return Err(ProofError::TransactionNotFound.into());
        };

        // RLP encode the block header
        let mut block_header_encoded = Vec::new();
        block.header.encode(&mut block_header_encoded);
        let Some(receipts) = self
            .read_provider
            .get_block_receipts(block_hash.into())
            .await?
        else {
            return Err(ProofError::TransactionNotFound.into());
        };
        let ordered_receipts = receipts
            .into_iter()
            .map(|r| {
                let rpc_receipt = r.inner.as_receipt_with_bloom().expect("Infallible");
                let consensus_receipt = ReceiptWithBloom::new(
                    Receipt {
                        status: rpc_receipt.receipt.status,
                        cumulative_gas_used: rpc_receipt.receipt.cumulative_gas_used,
                        logs: rpc_receipt
                            .receipt
                            .logs
                            .iter()
                            .map(|l| Log {
                                address: l.address(),
                                data: l.data().clone(),
                            })
                            .collect(),
                    },
                    rpc_receipt.logs_bloom,
                );
                match r.transaction_type() {
                    TxType::Legacy => ReceiptEnvelope::Legacy(consensus_receipt),
                    TxType::Eip2930 => ReceiptEnvelope::Eip2930(consensus_receipt),
                    TxType::Eip1559 => ReceiptEnvelope::Eip1559(consensus_receipt),
                    TxType::Eip4844 => ReceiptEnvelope::Eip4844(consensus_receipt),
                    TxType::Eip7702 => ReceiptEnvelope::Eip7702(consensus_receipt),
                }
            })
            .collect::<Vec<_>>();

        let target_tx_index = receipt.transaction_index.unwrap() as usize;
        let mut list = ordered_trie_with_encoder_for_target(
            ordered_receipts.as_ref(),
            |rlp: &ReceiptEnvelope, buf| rlp.encode_2718(buf),
            target_tx_index,
        );

        // Check receipts root is correct
        let root = list.root();
        if block.header.receipts_root != root {
            return Err(ProofError::InvalidRoot.into());
        }

        // Extract the proof nodes for the target receipt
        let proof_nodes = list.take_proof_nodes().clone();
        // Convert proof nodes to Bytes(u8)
        let proof_nodes_bytes = proof_nodes.iter().fold(Vec::new(), |mut acc, (_, node)| {
            acc.extend_from_slice(node);
            acc
        });

        // Get the target receipt that we're proving inclusion for
        let target_receipt = &ordered_receipts[target_tx_index];
        // Encode the target receipt for inclusion in proof
        let mut receipt_encoded = Vec::new();
        target_receipt.encode_2718(&mut receipt_encoded);

        // Encode receipt path using the ADJUSTED index (to match proof nodes)
        let mut path_buffer = Vec::new();
        let adjusted_index = adjust_index_for_rlp(target_tx_index, ordered_receipts.len());
        adjusted_index.encode(&mut path_buffer);

        let proof = Escrow::ReceiptProof {
            header: Bytes::from(block_header_encoded),
            receipt: Bytes::from(receipt_encoded),
            proof: Bytes::from(proof_nodes_bytes),
            path: Bytes::from(path_buffer),
            log: U256::from(log_idx),
        };

        trace!(
            "Generated {} byte proof for tx_index {} (adjusted to {})",
            proof.header.len() + proof.receipt.len() + proof.proof.len() + proof.path.len(),
            target_tx_index,
            adjusted_index
        );

        Ok(proof)
    }
}

/// FROM KONA: https://github.com/op-rs/kona/blob/HEAD/crates/proof/mpt/src/util.rs#L7-L51
/// Compute a trie root of the collection of items with a custom encoder.
/// Only retains proof for the specified target transaction.
pub fn ordered_trie_with_encoder_for_target<T, F>(
    items: &[T],
    mut encode: F,
    target_tx_index: usize,
) -> HashBuilder
where
    F: FnMut(&T, &mut dyn BufMut),
{
    let mut index_buffer = Vec::new();
    let mut value_buffer = Vec::new();
    let items_len = items.len();

    // Calculate the adjusted index for our target transaction
    let target_adjusted_index = adjust_index_for_rlp(target_tx_index, items_len);

    // Only retain proof for the target transaction's adjusted index
    let target_path = {
        index_buffer.clear();
        target_adjusted_index.encode(&mut index_buffer);
        Nibbles::unpack(&index_buffer)
    };

    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![target_path]));

    // Build the trie with all items, but only retain proof for target
    for i in 0..items_len {
        let index = adjust_index_for_rlp(i, items_len);
        index_buffer.clear();
        index.encode(&mut index_buffer);
        value_buffer.clear();
        encode(&items[index], &mut value_buffer);
        hb.add_leaf(Nibbles::unpack(&index_buffer), &value_buffer);
    }

    hb
}
