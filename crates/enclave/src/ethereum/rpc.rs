use std::net::SocketAddr;

use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use serde::{Deserialize, Serialize};
use ureq::{tls::TlsConfig, Agent};

#[derive(Serialize)]
struct JsonRpcRequest<T> {
    jsonrpc: &'static str,
    method: &'static str,
    params: T,
    id: u64,
}

#[derive(Deserialize)]
struct JsonRpcResponse<T> {
    #[allow(dead_code)]
    jsonrpc: String,
    result: Option<T>,
    error: Option<JsonRpcError>,
    #[allow(dead_code)]
    id: u64,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub struct RpcClient {
    addr: SocketAddr,
    agent: Agent,
}

impl RpcClient {
    pub fn new(addr: SocketAddr, expected_cert: Option<String>) -> Self {
        let tls_config = TlsConfig::builder()
            // If we don't have an expected cert, disable tls verification (debug only)
            .disable_verification(expected_cert.is_none())
            // TODO: build root certificate store with expected cert
            .build();

        // Build the request agent
        let agent = Agent::config_builder()
            .tls_config(tls_config)
            .build()
            .new_agent();

        Self { addr, agent }
    }

    pub fn call<R: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: impl Serialize,
    ) -> Result<R> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        };

        let response = self
            .agent
            .post(format!("https://{}", self.addr))
            .send_json(&request)
            .context("Failed to send RPC request")?
            .body_mut()
            .read_json::<JsonRpcResponse<R>>()
            .context("Failed to parse RPC response")?;

        if let Some(error) = response.error {
            return Err(eyre!("RPC error {}: {}", error.code, error.message));
        }

        response.result.ok_or_else(|| eyre!("{:?}", response.error))
    }
}
