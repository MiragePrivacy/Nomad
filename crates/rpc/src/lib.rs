use jsonrpsee::{core::async_trait, proc_macros::rpc, server::Server};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, instrument};

use nomad_types::Signal;

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

#[rpc(server, namespace = "mirage")]
pub trait MirageRpc {
    #[method(name = "signal")]
    async fn signal(&self, message: Signal) -> String;
}

struct MirageServer {
    signal_tx: UnboundedSender<Signal>,
}

#[async_trait]
impl MirageRpcServer for MirageServer {
    async fn signal(&self, message: Signal) -> String {
        info!("Received RPC signal: {message}");
        self.signal_tx
            .send(message.clone())
            .expect("failed to send signal to gossip");
        format!("Signal acknowledged: {message}")
    }
}

#[instrument(skip(signal_tx))]
pub async fn spawn_rpc_server(
    config: RpcConfig,
    signal_tx: UnboundedSender<Signal>,
) -> anyhow::Result<()> {
    let server = Server::builder().build(("0.0.0.0", config.port)).await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());
    println!("RPC server running on {}", server_addr);
    tokio::spawn(rpc_server.stopped());
    Ok(())
}
