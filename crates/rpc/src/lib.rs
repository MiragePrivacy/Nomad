use jsonrpsee::{core::async_trait, proc_macros::rpc, server::Server, types::ErrorObjectOwned};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info, instrument};

use nomad_types::Signal;

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

#[rpc(server, client, namespace = "mirage")]
pub trait MirageRpc {
    #[method(name = "signal")]
    async fn signal(&self, message: Signal) -> Result<String, ErrorObjectOwned>;
}

struct MirageServer {
    signal_tx: UnboundedSender<Signal>,
}

#[async_trait]
impl MirageRpcServer for MirageServer {
    #[instrument(skip(self))]
    async fn signal(&self, message: Signal) -> Result<String, ErrorObjectOwned> {
        info!("Received RPC signal");
        if self.signal_tx.send(message.clone()).is_err() {
            return Err(ErrorObjectOwned::owned(
                500,
                "Failed to broadcast signal",
                None::<()>,
            ));
        }

        Ok(format!("Signal acknowledged: {message}"))
    }
}

pub async fn spawn_rpc_server(
    config: RpcConfig,
    signal_tx: UnboundedSender<Signal>,
) -> eyre::Result<()> {
    debug!(?config);
    let server = Server::builder().build(("0.0.0.0", config.port)).await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());
    info!("RPC server running on {}", server_addr);
    tokio::spawn(rpc_server.stopped());
    Ok(())
}
