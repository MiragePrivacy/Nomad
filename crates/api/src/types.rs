use nomad_dcap_quote::SgxQlQveCollateral;
use nomad_types::primitives::{self, Bytes};
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
}

/// Relay get response
#[derive(Serialize, Deserialize, Debug)]
pub struct RelayGetResponse {
    pub status: String,
    pub service: String,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AttestResponse {
    /// SGX Attestation containing a quote and collateral proving the key and debug mode.
    pub attestation: Option<Attestation>,
    /// Enclave global key (extracted from quote body's enclave report)
    #[schema(value_type = String)]
    pub global_key: primitives::FixedBytes<33>,
    /// True if the enclave is running in debug mode
    pub is_debug: bool,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Attestation {
    #[schema(value_type = String)]
    pub quote: Bytes,
    pub collateral: SgxQlQveCollateral,
}
