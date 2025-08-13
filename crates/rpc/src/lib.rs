use jsonrpsee::{core::async_trait, proc_macros::rpc, server::Server};
use nomad_types::Signal;
use tokio::sync::mpsc;
use tracing::{info, instrument};

#[rpc(server, namespace = "mirage")]
pub trait MirageRpc {
    #[method(name = "signal")]
    async fn signal(&self, message: Signal) -> String;
}

struct MirageServer {
    signal_tx: mpsc::UnboundedSender<Signal>,
}

#[async_trait]
impl MirageRpcServer for MirageServer {
    async fn signal(&self, message: Signal) -> String {
        info!("Received RPC signal: {}", message);
        let _ = self.signal_tx.send(message.clone());
        format!("Signal acknowledged: {}", message)
    }
}

#[instrument(skip(signal_tx))]
pub async fn spawn_rpc_server(
    signal_tx: mpsc::UnboundedSender<Signal>,
    rpc_port: Option<u16>,
) -> anyhow::Result<()> {
    let server = Server::builder()
        .build(("0.0.0.0", rpc_port.unwrap_or_default()))
        .await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());
    println!("RPC server running on {}", server_addr);
    tokio::spawn(rpc_server.stopped());
    Ok(())
}
