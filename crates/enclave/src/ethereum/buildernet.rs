use eyre::{Context, Result};
use nomad_types::primitives::{Bytes, TxHash};
use serde::{Deserialize, Serialize};

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

pub struct BuildernetClient {
    rpc_url: String,
    _cert: Option<String>,
}

impl BuildernetClient {
    pub fn new(_atls_url: &str, rpc_url: String) -> eyre::Result<Self> {
        // TODO: connect to the atls endpoint and fetch the certificate

        Ok(Self {
            rpc_url,
            _cert: None,
        })
    }

    fn rpc_call<P: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: P,
    ) -> Result<R> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: 1,
        };

        let response: JsonRpcResponse<R> = ureq::post(&self.rpc_url)
            .send_json(&request)
            .context("Failed to send RPC request")?
            .body_mut()
            .read_json()
            .context("Failed to parse RPC response")?;

        if let Some(error) = response.error {
            return Err(eyre::eyre!("RPC error {}: {}", error.code, error.message));
        }

        response
            .result
            .ok_or_else(|| eyre::eyre!("rpc error: {:?}", response.error))
    }

    pub fn send_raw_transaction(&self, signed_tx: Bytes) -> Result<TxHash> {
        let tx_hash: String = self
            .rpc_call("eth_sendRawTransaction", vec![signed_tx.to_string()])
            .context("Failed to send raw transaction")?;

        Ok(TxHash::from_slice(&hex::decode(
            tx_hash.trim_start_matches("0x"),
        )?))
    }
}
