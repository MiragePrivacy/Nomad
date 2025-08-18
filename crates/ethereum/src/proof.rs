use alloy::{
    consensus::{Receipt, ReceiptEnvelope, ReceiptWithBloom, TxType},
    eips::Encodable2718 as _,
    primitives::{Bytes, Log},
    providers::Provider,
    rlp::{BufMut, Encodable},
    rpc::types::TransactionReceipt,
};
use alloy_trie::{proof::ProofRetainer, root::adjust_index_for_rlp, HashBuilder, Nibbles};
use nomad_types::Signal;
use serde::{Deserialize, Serialize};
use tracing::{instrument, trace};

use crate::{ClientError, EthClient, IERC20};

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("Transaction not found")]
    TransactionNotFound,
    #[error("Log out of bounds")]
    LogIndexOutOfBounds,
    #[error("Log mismatched")]
    LogMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofBlob {
    /// RLP-encoded block header
    pub block_header: Bytes,
    /// The transaction receipt
    pub receipt: Bytes,
    /// Minimal MPT proof nodes linking receipt to receiptsRoot
    pub proof_nodes: Bytes,
    // RLP-encoded transaction index
    pub receipt_path: Bytes,
    /// The specific log if log_index was provided
    pub target_log: Log,
}

impl EthClient {
    /// Creates a new `ProofInput` with the given block hash, transaction index, and optional log index.
    #[instrument(skip_all, fields(tx = ?receipt.transaction_hash))]
    pub(crate) async fn generate_proof(
        &self,
        signal: &Signal,
        receipt: &TransactionReceipt,
    ) -> Result<ProofBlob, ClientError> {
        // Locate signal transfer event in the receipt logs
        let mut target_log = None;
        for raw_log in receipt.logs() {
            let log = raw_log
                .log_decode::<IERC20::Transfer>()
                .map_err(|_| ProofError::LogMismatch)?;
            let data = log.data();
            if log.address() == signal.token_contract
                && data.to == signal.recipient
                && data.value == signal.transfer_amount
            {
                target_log = Some(raw_log.clone());
            }
        }
        let Some(target_log) = target_log else {
            return Err(ProofError::LogMismatch.into());
        };
        let Some(log_idx) = target_log.log_index else {
            return Err(ProofError::LogMismatch.into());
        };

        let block_hash = receipt.block_hash.unwrap();
        let Some(block) = self.provider.get_block_by_hash(block_hash).await? else {
            return Err(ProofError::TransactionNotFound.into());
        };

        let Some(receipts) = self.provider.get_block_receipts(block_hash.into()).await? else {
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
        let mut list =
            ordered_trie_with_encoder(ordered_receipts.as_ref(), |rlp: &ReceiptEnvelope, buf| {
                rlp.encode_2718(buf)
            });

        let tx_index = receipt.transaction_index.unwrap();

        //check receipts root is correct
        let root = list.root();
        assert_eq!(block.header.receipts_root, root, "Receipts root mismatch");
        // Extract the proof nodes for the target receipt
        let proof_nodes = list.take_proof_nodes().clone();
        // Get the target receipt that we're proving inclusion for
        let target_receipt = &ordered_receipts[tx_index as usize];
        // Encode the target receipt for inclusion in proof
        let mut receipt_encoded = Vec::new();
        target_receipt.encode_2718(&mut receipt_encoded);

        // Validate log index if provided and extract target log
        if log_idx >= target_receipt.logs().len() as u64 {
            return Err(ProofError::LogIndexOutOfBounds.into());
        }
        let proof_target_log = target_receipt.logs()[log_idx as usize].clone();

        // Ensure the target_log from RPC receipt matches the one from consensus receipt
        if target_log.address() != proof_target_log.address
            || target_log.data() != &proof_target_log.data
        {
            return Err(ProofError::LogMismatch.into());
        }

        // RLP encode the block header
        let mut block_header_encoded = Vec::new();
        block.header.encode(&mut block_header_encoded); // Convert proof nodes to Bytes(u8)
        let proof_nodes_bytes = proof_nodes.iter().fold(Vec::new(), |mut acc, (_, node)| {
            acc.extend_from_slice(node);
            acc
        });

        // Encode receipt path
        let mut path_buffer = Vec::new();
        let adjusted_index = adjust_index_for_rlp(tx_index as usize, ordered_receipts.len());
        adjusted_index.encode(&mut path_buffer);

        let proof = ProofBlob {
            block_header: Bytes::from(block_header_encoded),
            receipt: Bytes::from(receipt_encoded),
            proof_nodes: Bytes::from(proof_nodes_bytes),
            receipt_path: Bytes::from(path_buffer),
            target_log: proof_target_log,
        };

        let total = proof.block_header.len()
            + proof.receipt.len()
            + proof.proof_nodes.len()
            + proof.receipt_path.len();
        trace!("Generated {total} byte proof");

        Ok(proof)
    }
}

/// FROM KONA: https://github.com/op-rs/kona/blob/HEAD/crates/proof/mpt/src/util.rs#L7-L51
/// Compute a trie root of the collection of items with a custom encoder.
pub fn ordered_trie_with_encoder<T, F>(items: &[T], mut encode: F) -> HashBuilder
where
    F: FnMut(&T, &mut dyn BufMut),
{
    let mut index_buffer = Vec::new();
    let mut value_buffer = Vec::new();
    let items_len = items.len(); // Store preimages for all intermediates
    let path_nibbles = (0..items_len)
        .map(|i| {
            let index = adjust_index_for_rlp(i, items_len);
            index_buffer.clear();
            index.encode(&mut index_buffer);
            Nibbles::unpack(&index_buffer)
        })
        .collect::<Vec<_>>();
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(path_nibbles));
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
