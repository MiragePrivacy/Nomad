use aide::OperationOutput;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use nomad_types::{EncryptedSignal, Signal, SignalPayload};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Unified type for either an encrypted or unencrypted signal
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

#[derive(JsonSchema)]
pub enum SignalResponse {
    Success,
    Failure,
}

impl OperationOutput for SignalResponse {
    type Inner = String;
}

impl IntoResponse for SignalResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            SignalResponse::Success => Response::new("Signal recieved".into()),
            SignalResponse::Failure => {
                let mut response = Response::new("Failed to ingest signal".into());
                *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                response
            }
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
