use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use nomad_types::{EncryptedSignal, Signal, SignalPayload};

/// Encrypted or raw signal
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
pub enum SignalRequest {
    Unencrypted(Signal),
    Encrypted(EncryptedSignal),
}

impl From<SignalRequest> for SignalPayload {
    fn from(v: SignalRequest) -> Self {
        match v {
            SignalRequest::Encrypted(s) => Self::Encrypted(s),
            SignalRequest::Unencrypted(s) => Self::Unencrypted(s),
        }
    }
}

/// Node health report
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct HealthResponse {
    /// 'healthy'
    pub status: String,
    /// Node version
    pub version: String,
    /// Node implementation type
    pub kind: String,
    /// Time since last startup
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
