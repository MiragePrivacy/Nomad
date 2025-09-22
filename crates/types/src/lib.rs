use std::hash::{Hash, Hasher};

use alloy_primitives::{Address, Bytes, TxHash, U256};
use serde::{Deserialize, Serialize};
use url::Url;

pub use alloy_primitives as primitives;

mod selectors;

pub use hex_schema::*;
pub use selectors::*;
use utoipa::ToSchema;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SignalPayload {
    Unencrypted(Signal),
    Encrypted(EncryptedSignal),
    TracedUnencrypted(Signal, [u8; 16]),
    TracedEncrypted(EncryptedSignal, [u8; 16]),
}

impl SignalPayload {
    pub fn token_contract(&self) -> Address {
        match self {
            SignalPayload::Encrypted(EncryptedSignal { token_contract, .. })
            | SignalPayload::Unencrypted(Signal { token_contract, .. })
            | SignalPayload::TracedEncrypted(EncryptedSignal { token_contract, .. }, _)
            | SignalPayload::TracedUnencrypted(Signal { token_contract, .. }, _) => *token_contract,
        }
    }

    pub fn trace_id(&self) -> Option<[u8; 16]> {
        match self {
            SignalPayload::TracedEncrypted(_, trace)
            | SignalPayload::TracedUnencrypted(_, trace) => Some(*trace),
            _ => None,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum EnclaveMessage {
    SignalFailure {
        hash: [u8; 32],
    },
    SignalCompleted {
        hash: [u8; 32],
        bond: TxHash,
        transfer: TxHash,
    },
    RewardsCollected {
        hash: [u8; 32],
        collect: TxHash,
    },
}

/// Fully encrypted signal containing the puzzle and relay address
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EncryptedSignal {
    /// Token contract to transfer
    #[schema(value_type = HexAddress)]
    pub token_contract: Address,
    /// Relay address for submitting puzzle solutions to
    #[schema(example = "http://your-server.com/relay")]
    pub relay: Url,
    /// Hex-encoded puzzle bytes
    #[schema(value_type = HexBytes)]
    pub puzzle: Bytes,
    /// Hex-encoded AES-GCM encrypted data containing a json [`Signal`]
    #[schema(value_type = HexBytes)]
    pub data: Bytes,
}

/// Decrypted signal payload containing all information required to execute
#[derive(Deserialize, Serialize, ToSchema, Clone, PartialEq, Eq)]
pub struct Signal {
    /// Escrow contract to bond to and collect rewards from
    #[schema(value_type = HexAddress)]
    pub escrow_contract: Address,
    /// Token contract to transfer
    #[schema(value_type = HexAddress)]
    pub token_contract: Address,
    /// Recipient for the transfer
    #[schema(value_type = HexAddress)]
    pub recipient: Address,
    /// Raw amount of tokens to transfer (including zeros)
    #[schema(value_type = U256String)]
    pub transfer_amount: U256,
    /// Reward amount for the node
    #[schema(value_type = U256String)]
    pub reward_amount: U256,
    /// Acknowledgement address for submitting the receipt to
    #[schema(example = "http://your-server.com/ack")]
    pub acknowledgement_url: Url,
    /// Optional mappings for an obfuscated contract
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(default = "null")]
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

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
pub struct ReceiptFormat {
    pub start_time: String,
    pub end_time: String,
    pub bond_transaction_hash: String,
    pub approval_transaction_hash: String,
    pub transfer_transaction_hash: String,
}

mod hex_schema {
    use utoipa::ToSchema;

    #[derive(ToSchema)]
    #[schema(pattern = r"^0x[0-9a-fA-F]*$")]
    #[schema(description = "Hex-encoded bytes as a string (e.g., '0x1234abcd')")]
    pub struct HexBytes(pub String);

    #[derive(ToSchema)]
    #[schema(pattern = r"^0x[0-9a-fA-F]{40}$")]
    #[schema(description = "Hex-encoded Ethereum address (20 bytes)")]
    #[schema(example = "0xBe41a9EC942d5b52bE07cC7F4D7E30E10e9B652A")]
    pub struct HexAddress(pub String);

    #[derive(ToSchema)]
    #[schema(pattern = r"^0x[0-9a-fA-F]{8}$")]
    #[schema(description = "Hex-encoded function selector (4 bytes)")]
    #[schema(example = "0x11223344")]
    pub struct HexSelector(pub String);

    #[derive(ToSchema)]
    #[schema(pattern = r"^[0-9]+$")]
    #[schema(description = "U256 value as a decimal string")]
    #[schema(example = "12345678")]
    pub struct U256String(pub String);
}
