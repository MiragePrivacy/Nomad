use std::hash::{Hash, Hasher};

use alloy_primitives::{Address, Bytes, U256};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

pub use alloy_primitives as primitives;

mod selectors;

pub use hex_schema::*;
pub use selectors::*;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SignalPayload {
    Unencrypted(Signal),
    Encrypted(EncryptedSignal),
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
#[derive(Deserialize, Serialize, JsonSchema, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EncryptedSignal {
    /// Token contract to transfer
    #[schemars(
        with = "HexAddress",
        example = "0xBe41a9EC942d5b52bE07cC7F4D7E30E10e9B652A"
    )]
    pub token_contract: Address,
    /// Relay address for submitting puzzle solutions to
    #[schemars(example = "http://your-server.com/relay")]
    pub relay: Url,
    /// Hex-encoded puzzle bytes
    #[schemars(with = "HexBytes", example = Bytes::from(vec![0; 500]))]
    pub puzzle: Bytes,
    /// Hex-encoded AES-GCM encrypted data containing a json [`Signal`]
    #[schemars(with = "HexBytes", example = Bytes::from(vec![0; 24]))]
    pub data: Bytes,
}

/// Decrypted signal payload containing all information required to execute
#[derive(Deserialize, Serialize, JsonSchema, Clone, PartialEq, Eq)]
pub struct Signal {
    /// Escrow contract to bond to and collect rewards from
    #[schemars(with = "HexAddress", example = "0x...")]
    pub escrow_contract: Address,
    /// Token contract to transfer
    #[schemars(
        with = "HexAddress",
        example = "0xBe41a9EC942d5b52bE07cC7F4D7E30E10e9B652A"
    )]
    pub token_contract: Address,
    /// Recipient for the transfer
    #[schemars(
        with = "HexAddress",
        example = "0x123453b4cE4B4bB18EAEc84C69eb745C83fC1b2F"
    )]
    pub recipient: Address,
    /// Raw amount of tokens to transfer (including zeros)
    #[schemars(with = "U256String", example = "25000000")]
    pub transfer_amount: U256,
    /// Reward amount for the node
    #[schemars(with = "U256String", example = "2000000")]
    pub reward_amount: U256,
    /// Acknowledgement address for submitting the receipt to
    #[schemars(example = "http://your-server.com/ack")]
    pub acknowledgement_url: String,
    /// Optional mappings for an obfuscated contract
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(example = None::<SelectorMapping>)]
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

mod hex_schema {
    use schemars::JsonSchema;

    #[derive(JsonSchema)]
    #[schemars(transparent)]
    pub struct HexBytes(
        #[schemars(regex(pattern = r"^0x[0-9a-fA-F]*$"))]
        #[schemars(description = "Hex-encoded bytes as a string (e.g., '0x1234abcd')")]
        pub String,
    );

    #[derive(JsonSchema)]
    #[schemars(transparent)]
    pub struct HexAddress(
        #[schemars(regex(pattern = r"^0x[0-9a-fA-F]{40}$"))]
        #[schemars(description = "Hex-encoded Ethereum address (20 bytes, e.g., '0x1234...abcd')")]
        pub String,
    );

    #[derive(JsonSchema)]
    #[schemars(transparent)]
    pub struct HexSelector(
        #[schemars(regex(pattern = r"^0x[0-9a-fA-F]{8}$"))]
        #[schemars(description = "Hex-encoded function selector (4 bytes, e.g., '0x12345678')")]
        pub String,
    );

    #[derive(JsonSchema)]
    #[schemars(transparent)]
    pub struct U256String(
        #[schemars(regex(pattern = r"^[0-9]+$"))]
        #[schemars(description = "U256 value as a decimal string (e.g., '1234567890')")]
        pub String,
    );
}
