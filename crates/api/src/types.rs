use serde::{Deserialize, Serialize};

use nomad_types::{EncryptedSignal, Signal, SignalPayload};
use utoipa::ToSchema;

/// Encrypted or raw signal
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum SignalRequest {
    Unencrypted(Signal),
    Encrypted(EncryptedSignal),
}

impl SignalRequest {
    pub fn untraced(self) -> SignalPayload {
        match self {
            SignalRequest::Encrypted(s) => SignalPayload::Encrypted(s),
            SignalRequest::Unencrypted(s) => SignalPayload::Unencrypted(s),
        }
    }

    pub fn traced(self, id: Vec<u8>) -> SignalPayload {
        match self {
            SignalRequest::Encrypted(s) => {
                SignalPayload::TracedEncrypted(s, id.try_into().unwrap())
            }
            SignalRequest::Unencrypted(s) => {
                SignalPayload::TracedUnencrypted(s, id.try_into().unwrap())
            }
        }
    }
}

/// Node health report
#[derive(Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    #[schema(example = "healty")]
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
