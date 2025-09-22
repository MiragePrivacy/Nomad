use eyre::{bail, Result};
use nomad_pool::SignalPool;
use nomad_types::SignalPayload;
use tokio::{net::TcpListener, sync::mpsc::Receiver};



pub struct Enclave {
    rx: Receiver<SignalPayload>
    tx: Sender<
}

impl Enclave {
    pub async fn run(self) -> Result<()> {
        // listen for a socket

        let listener = TcpListener::bind(("0.0.0.0", 8888)).await?;
        let Ok((read, write)) = listener.accept().await.map(|(s, _)| s.into_split()) else {
            bail!("Failed to get enclave connection");
        }

        // read signal status

        Ok(())
    }
}
