use std::hash::{Hash, Hasher};

use alloy_primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};
use url::Url;

pub use alloy_primitives as primitives;

mod selectors;
pub use selectors::*;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SignalPayload {
    Encrypted(EncryptedSignal),
    Unencrypted(Signal),
}

impl SignalPayload {
    pub fn token_contract(&self) -> Address {
        match self {
            SignalPayload::Encrypted(EncryptedSignal { token_contract, .. })
            | SignalPayload::Unencrypted(Signal { token_contract, .. }) => *token_contract,
        }
    }
}

/// Fully encrypted signal containing the puzzle and relay address
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EncryptedSignal {
    pub token_contract: Address,
    pub relay: Url,
    pub puzzle: Bytes,
    pub data: Bytes,
}

/// Decrypted signal
#[derive(Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct Signal {
    pub escrow_contract: Address,
    pub token_contract: Address,
    pub recipient: Address,
    pub transfer_amount: U256,
    pub reward_amount: U256,
    pub acknowledgement_url: String,
    pub selector_mapping: Option<SelectorMapping>,
}

impl Hash for Signal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.escrow_contract.hash(state);
        self.token_contract.hash(state);
        self.recipient.hash(state);
        self.transfer_amount.hash(state);
        self.reward_amount.hash(state);
        self.acknowledgement_url.hash(state);
        // deliberately exclude selector_mapping from hash
        // this way signals are deduplicated based on core content, not obfuscation
    }
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "send {} tokens to {} and collect {} tokens from escrow {}",
            self.transfer_amount, self.recipient, self.reward_amount, self.escrow_contract
        )
    }
}

impl std::fmt::Debug for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signal")
            .field("token", &self.token_contract)
            .field("escrow", &self.escrow_contract)
            .field("amount", &self.transfer_amount)
            .field("reward", &self.reward_amount)
            .finish()
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ReceiptFormat {
    pub start_time: String,
    pub end_time: String,
    pub bond_transaction_hash: String,
    pub approval_transaction_hash: String,
    pub transfer_transaction_hash: String,
    pub collection_transaction_hash: String,
}
