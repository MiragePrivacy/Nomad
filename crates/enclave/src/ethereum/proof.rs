use alloy_consensus::{Receipt, ReceiptEnvelope, ReceiptWithBloom};
use alloy_network::eip2718::Encodable2718;
use alloy_rlp::{BufMut, Encodable};
use alloy_trie::{proof::ProofRetainer, root::adjust_index_for_rlp, HashBuilder, Nibbles};
use color_eyre::{
    eyre::{bail, eyre},
    Result,
};
use nomad_types::primitives::{Address, Bloom, Bytes, Log, LogData, TxHash, B256, U256};

use super::contracts::Escrow;

impl super::EthClient {
    /// Generate a Merkle proof for a transfer transaction receipt
    pub fn generate_proof(
        &self,
        transfer_tx: TxHash,
        recipient: Address,
        amount: U256,
    ) -> Result<Escrow::ReceiptProof> {
        // Get transaction receipt
        let receipt = self
            .geth
            .get_transaction_receipt(transfer_tx)
            .ok_or_else(|| eyre!("Transaction receipt not found"))?;

        // Find the target transfer event log index
        let mut log_idx = None;
        for (idx, log) in receipt.inner.logs().iter().enumerate() {
            let topics = log.topics();
            // Check if this is a Transfer event (signature matches)
            if topics.len() >= 3 {
                let event_sig = &topics[0];
                // Transfer event signature: keccak256("Transfer(address,address,uint256)")
                let transfer_sig = B256::from_slice(
                    &hex::decode(
                        "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    )
                    .unwrap(),
                );
                if event_sig == &transfer_sig {
                    // Parse the log data to check recipient and amount
                    // topics[2] is the recipient (indexed)
                    let log_recipient = Address::from_slice(&topics[2].as_slice()[12..]); // Last 20 bytes

                    // data contains the amount (not indexed)
                    let log_amount = U256::from_be_slice(log.data().data.as_ref());

                    if log_recipient == recipient && log_amount == amount {
                        log_idx = Some(idx);
                        break;
                    }
                }
            }
        }

        let Some(log_idx) = log_idx else {
            bail!("Transfer log not found in receipt");
        };

        // Get the block
        let block_hash = receipt
            .block_hash
            .ok_or_else(|| eyre!("Block hash not found"))?;
        let block = self.geth.get_block_by_hash(block_hash)?;

        // Get all receipts in the block
        let receipts = self.geth.get_block_receipts(block_hash)?;

        // Convert RPC receipts to consensus receipts
        let ordered_receipts: Vec<ReceiptEnvelope> = receipts
            .iter()
            .map(|r| {
                let status = r.inner.status();
                let cumulative_gas_used = r.inner.cumulative_gas_used();

                let logs: Vec<Log> = r
                    .inner
                    .logs()
                    .iter()
                    .map(|l| {
                        let address = Address::from_slice(l.address().as_slice());
                        let topics: Vec<B256> = l
                            .topics()
                            .iter()
                            .map(|t| B256::from_slice(t.as_slice()))
                            .collect();
                        Log {
                            address,
                            data: LogData::new_unchecked(topics, l.data().data.clone()),
                        }
                    })
                    .collect();

                let logs_bloom = Bloom::from_slice(r.inner.logs_bloom().as_slice());

                let consensus_receipt = ReceiptWithBloom::new(
                    Receipt {
                        status: status.into(),
                        cumulative_gas_used,
                        logs,
                    },
                    logs_bloom,
                );

                match r.transaction_type() as u8 {
                    0 => ReceiptEnvelope::Legacy(consensus_receipt),
                    1 => ReceiptEnvelope::Eip2930(consensus_receipt),
                    2 => ReceiptEnvelope::Eip1559(consensus_receipt),
                    3 => ReceiptEnvelope::Eip4844(consensus_receipt),
                    4 => ReceiptEnvelope::Eip7702(consensus_receipt),
                    _ => ReceiptEnvelope::Legacy(consensus_receipt),
                }
            })
            .collect();

        let target_tx_index = receipt
            .transaction_index
            .ok_or_else(|| eyre!("Transaction index not found"))?
            as usize;

        let mut list = ordered_trie_with_encoder_for_target(
            ordered_receipts.as_ref(),
            |rlp: &ReceiptEnvelope, buf| {
                let temp = rlp.encoded_2718();
                buf.put_slice(&temp);
            },
            target_tx_index,
        );

        // Get root and verify
        let root = list.root();
        let expected_root = block.header.receipts_root;

        if root.as_slice() != expected_root.as_slice() {
            bail!("Receipts root mismatch");
        }

        // Sort proof nodes by path
        let mut proof_nodes_vec: Vec<_> = list
            .take_proof_nodes()
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();

        proof_nodes_vec
            .sort_by(|(ka, _), (kb, _)| ka.len().cmp(&kb.len()).then_with(|| ka.cmp(kb)));

        let proof_nodes_array: Vec<Bytes> = proof_nodes_vec
            .into_iter()
            .map(|(_, node)| Bytes::from(node.to_vec()))
            .collect();

        // RLP encode proof nodes
        let mut proof_nodes_encoded = Vec::new();
        proof_nodes_array.encode(&mut proof_nodes_encoded);

        // Encode block header
        let mut block_header_encoded = Vec::new();
        block.header.encode(&mut block_header_encoded);

        // Encode receipt
        let trie_receipt = &ordered_receipts[target_tx_index];
        let receipt_encoded = trie_receipt.encoded_2718();

        // Encode path
        let mut path_buffer = Vec::new();
        target_tx_index.encode(&mut path_buffer);

        Ok(Escrow::ReceiptProof {
            header: Bytes::from(block_header_encoded),
            receipt: Bytes::from(receipt_encoded),
            proof: Bytes::from(proof_nodes_encoded),
            path: Bytes::from(path_buffer),
            log: U256::from(log_idx),
        })
    }
}

/// Compute a trie root of the collection of items with a custom encoder.
/// Only retains proof for the specified target transaction.
fn ordered_trie_with_encoder_for_target<T, F>(
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

    // Use raw transaction index for proof retention path
    let target_path = {
        index_buffer.clear();
        target_tx_index.encode(&mut index_buffer);
        Nibbles::unpack(&index_buffer)
    };

    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![target_path]));

    // Build the trie with all items and retain proof for target
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
