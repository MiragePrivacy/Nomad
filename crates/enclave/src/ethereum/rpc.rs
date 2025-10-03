use std::net::SocketAddr;

use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use serde::{Deserialize, Serialize};
use ureq::{
    tls::TlsConfig,
    unversioned::{resolver::Resolver, transport::DefaultConnector},
    Agent,
};

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
    url: String,
    agent: Agent,
}

#[derive(Debug)]
pub struct FixedResolver {
    host: String,
    dest: SocketAddr,
}

impl Resolver for FixedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        if uri.host() == Some(&self.host) {
            let mut addrs = self.empty();
            addrs.push(self.dest);
            return Ok(addrs);
        }
        Err(ureq::Error::BadUri(uri.to_string()))
    }
}

impl RpcClient {
    pub fn new(host: String, dest: SocketAddr, _expected_cert: Option<String>) -> Self {
        let tls_config = TlsConfig::builder()
            // TODO: build root certificate store with expected cert
            .build();

        let url = format!("https://{host}:{}", dest.port());

        // Build the request agent
        let agent = Agent::with_parts(
            Agent::config_builder().tls_config(tls_config).build(),
            DefaultConnector::new(),
            FixedResolver { host, dest },
        );

        Self { url, agent }
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
            .post(&self.url)
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
