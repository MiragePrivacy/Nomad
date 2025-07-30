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
        info!(signal = %message, "Received signal");
        let _ = self.signal_tx.send(message.clone());
        format!("ack: {}", message)
    }
}

#[instrument(skip(signal_tx))]
pub async fn spawn_rpc_server(
    signal_tx: mpsc::UnboundedSender<Signal>,
    rpc_port: Option<u16>,
) -> anyhow::Result<()> {
    let addr = match rpc_port {
        Some(port) => format!("127.0.0.1:{port}"),
        None => "127.0.0.1:0".to_string(),
    };
    let server = Server::builder().build(addr).await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());

    println!("Running rpc on {server_addr}");

    tokio::spawn(rpc_server.stopped());
    Ok(())
}