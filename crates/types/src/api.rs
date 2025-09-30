use alloy_primitives::{self, Bytes};
use serde::{Deserialize, Serialize};

use utoipa::ToSchema;

/// Node health report
#[derive(Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    #[schema(example = "healthy")]
    pub status: String,
    /// Node version
    #[schema(example = "0.1.0")]
    pub version: String,
    /// Node implementation type
    #[schema(example = "nomad")]
    pub kind: String,
    /// Time since last startup
    #[schema(example = 420)]
    pub uptime_seconds: u64,
    /// Currently running in bootstrap mode
    pub is_bootstrap: bool,
    /// Currently only broadcasting and not processing signals
    pub read_only: bool,
    /// Ethereum network/chain ID
    #[schema(example = 1)]
    pub chain_id: u64,
}

/// Relay get response
#[derive(Serialize, Deserialize, Debug)]
pub struct RelayGetResponse {
    pub status: String,
    pub service: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AttestResponse {
    /// SGX Attestation containing a quote and collateral proving the key and debug mode.
    pub attestation: Option<Attestation>,
    /// Enclave global key (extracted from quote body's enclave report)
    #[schema(value_type = String)]
    pub global_key: alloy_primitives::FixedBytes<33>,
    /// True if the enclave is running in debug mode
    pub is_debug: bool,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Attestation {
    #[schema(value_type = String)]
    pub quote: Bytes,
    #[schema(value_type = Object)]
    pub collateral: serde_json::Value,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct KeyRequest {
    #[schema(value_type = String)]
    pub quote: Bytes,
}
