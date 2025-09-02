use jsonrpsee::{core::async_trait, proc_macros::rpc, server::Server, types::ErrorObjectOwned};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info, instrument};

use nomad_types::{EncryptedSignal, Signal, SignalPayload};

pub use jsonrpsee::http_client::HttpClient;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct RpcConfig {
    pub port: u16,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self { port: 8000 }
    }
}

/// Signal request allowing either an encrypted or unencrypted signal directly
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum SignalRequest {
    Encrypted(EncryptedSignal),
    Unencrypted(Signal),
}

impl From<SignalRequest> for SignalPayload {
    fn from(v: SignalRequest) -> Self {
        match v {
            SignalRequest::Encrypted(s) => Self::Encrypted(s),
            SignalRequest::Unencrypted(s) => Self::Unencrypted(s),
        }
    }
}

#[rpc(server, client, namespace = "mirage")]
pub trait MirageRpc {
    #[method(name = "signal")]
    async fn signal(&self, message: SignalRequest) -> Result<String, ErrorObjectOwned>;
}

struct MirageServer {
    signal_tx: UnboundedSender<SignalPayload>,
}

#[async_trait]
impl MirageRpcServer for MirageServer {
    #[instrument(skip(self), name = "rpc:signal")]
    async fn signal(&self, message: SignalRequest) -> Result<String, ErrorObjectOwned> {
        info!("Received");
        if self.signal_tx.send(message.into()).is_err() {
            return Err(ErrorObjectOwned::owned(
                500,
                "Failed to broadcast signal",
                None::<()>,
            ));
        }

        Ok("Signal acknowledged".into())
    }
}

pub async fn spawn_rpc_server(
    config: RpcConfig,
    signal_tx: UnboundedSender<SignalPayload>,
) -> eyre::Result<()> {
    debug!(?config);
    let server = Server::builder().build(("0.0.0.0", config.port)).await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());
    info!("RPC server running on {}", server_addr);
    tokio::spawn(rpc_server.stopped());
    Ok(())
}
