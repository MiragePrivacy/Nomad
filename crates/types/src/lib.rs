use std::hash::Hash;

use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use alloy_primitives as primitives;

mod api;
mod selectors;

pub use api::*;
pub use hex_schema::*;
pub use selectors::*;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug)]
pub struct ReportBody {
    /// Enclave global key (extracted from quote body's enclave report)
    #[schema(value_type = String)]
    pub public_key: FixedBytes<33>,
    /// Chain enclave is running on
    pub chain_id: u32,
    /// True if the enclave is running in debug mode
    pub is_debug: bool,
    /// True if the attestation is for the global key
    pub is_global: bool,
}

impl From<[u8; 64]> for ReportBody {
    fn from(value: [u8; 64]) -> Self {
        Self {
            public_key: value[0..33].try_into().unwrap(),
            chain_id: u32::from_be_bytes(value[33..33 + 4].try_into().unwrap()),
            is_debug: value[62] != 0,
            is_global: value[63] != 0,
        }
    }
}

impl From<ReportBody> for [u8; 64] {
    fn from(value: ReportBody) -> Self {
        let mut buf = [0; 64];
        buf[0..33].copy_from_slice(value.public_key.as_slice());
        buf[33..33 + 4].copy_from_slice(&value.chain_id.to_be_bytes());
        buf[62] = value.is_debug as u8;
        buf[63] = value.is_global as u8;
        buf
    }
}

/// Fully encrypted signal payload containing a json signal encrypted with ecies for
/// an enclave public key
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone, PartialEq, Eq, Hash)]
#[schema(value_type = String)]
#[schema(pattern = r"^0x[0-9a-fA-F]*$")]
pub struct SignalPayload(pub Bytes);

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
    /// Optional mappings for an obfuscated contract
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(default = "null")]
    pub selector_mapping: Option<SelectorMapping>,
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
