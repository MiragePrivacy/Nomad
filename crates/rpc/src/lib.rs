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
    let addr = match rpc_port {
        Some(port) => format!("127.0.0.1:{port}"),
        None => "127.0.0.1:0".to_string(),
    };
    let server = Server::builder().build(addr).await?;
    let server_addr = server.local_addr()?;
    let rpc_server = server.start(MirageServer { signal_tx }.into_rpc());

    println!("RPC server running on {}", server_addr);

    // Log local and global IP addresses for RPC server
    let port = server_addr.port();
    if let Ok(local_ip) = std::process::Command::new("hostname")
        .arg("-I")
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                .unwrap_or("unknown")
                .to_string()
        })
    {
        println!("RPC server local network access: {local_ip}:{port}");
    }

    // TODO: replace with ureq instead of forking and depending on curl
    if let Ok(output) = std::process::Command::new("curl")
        .arg("-s")
        .arg("--max-time")
        .arg("3")
        .arg("http://ipv4.icanhazip.com")
        .output()
    {
        let global_ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !global_ip.is_empty() && global_ip != "unknown" {
            println!("RPC server global access: {global_ip}:{port}");
        }
    }

    tokio::spawn(rpc_server.stopped());
    Ok(())
}

