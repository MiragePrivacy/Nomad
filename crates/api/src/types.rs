use nomad_types::{EncryptedSignal, Signal, SignalPayload};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub node_type: String,
    pub uptime_seconds: u64,
}
