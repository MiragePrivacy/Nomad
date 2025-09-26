use std::{net::SocketAddrV4, path::PathBuf};

use alloy::{hex, primitives::Bytes};
use eyre::{bail, Result};

use nomad_types::{EnclaveMessage, SignalPayload};
use serde::{Deserialize, Serialize};
use sgx_isa::Report;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::tcp::OwnedReadHalf,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tracing::info;

/// Configuration for the enclave
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct EnclaveConfig {
    /// Path to store sealed data in
    pub seal_path: PathBuf,
    /// List of bootstrap nodes to fetch enclave key from.
    /// If empty, assumes this is the first enclave and that
    /// we should create the key ourselves.
    pub nodes: Vec<SocketAddrV4>,
    /// List of bootstrap keys to seed EOAs with
    pub bootstrap_keys: Vec<Bytes>,
    /// List of debug keys to init the enclave with
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub debug_keys: Vec<Bytes>,
    /// Number of accounts we should provision when bootstrapping
    pub num_accounts: u8,
}

impl Default for EnclaveConfig {
    fn default() -> Self {
        Self {
            seal_path: "~/.config/nomad/".into(),
            nodes: vec![],
            debug_keys: vec![],
            bootstrap_keys: vec![],
            num_accounts: 2,
        }
    }
}

/// Spawn the enclave, returning the attestation report for the global public key
pub async fn spawn_enclave(
    config: &EnclaveConfig,
    mut rx: UnboundedReceiver<SignalPayload>,
    _tx: UnboundedSender<EnclaveMessage>,
) -> Result<Vec<u8>> {
    // startup enclave in a background thread
    start_enclave();

    // listen for the enclave connection
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 8888)).await?;
    let Ok((mut read, mut write)) = listener
        .accept()
        .await
        .map(|(stream, _)| stream.into_split())
    else {
        bail!("Failed to get enclave connection");
    };

    // global key initialization
    let key_path = config.seal_path.join("key.bin");
    if let Ok(data) = std::fs::read(&key_path) {
        // Restore key from sealed data
        write.write_u8(2).await?;
        write.write_u32(data.len() as u32).await?;
        write.write_all(&data).await?;
    } else {
        if !config.nodes.is_empty() {
            // Bootstrap from other node enclaves
            write.write_u8(1).await?;
            write.write_u8(config.nodes.len() as u8).await?;
            for addr in &config.nodes {
                write.write_u32(addr.ip().to_bits()).await?;
                write.write_u16(addr.port()).await?;
            }
        } else {
            // First node, generate key
            write.write_u8(0).await?;
        }

        // Read back sealed key and write to disk
        let len = read.read_u32().await? as usize;
        let mut payload = vec![0; len];
        read.read_exact(&mut payload).await?;
        tokio::fs::write(key_path, &payload).await?;
    }
    let (publickey, maybe_report) = read_global_key_report(&mut read).await?;
    info!("Enclave Global Key: 0x{}", hex::encode(publickey));

    // EOA account initialization
    let eoa_path = config.seal_path.join("eoa.bin");
    match (
        !config.bootstrap_keys.is_empty(),
        std::fs::read(&eoa_path).ok(),
    ) {
        // init with debug keys
        (false, None) => {
            write.write_u8(255).await?;
            write.write_u8(config.debug_keys.len() as u8).await?;
            for key in &config.debug_keys {
                write.write_all(key).await?;
            }
        }
        // Initialize new EOAs with funds from bootstrap accounts
        (true, None) => {
            write.write_u8(0).await?;
            write.write_u8(config.bootstrap_keys.len() as u8).await?;
            for key in &config.bootstrap_keys {
                write.write_all(key).await?;
            }

            // Read sealed keys from enclave and save to disk
            let len = read.read_u32().await? as usize;
            let mut payload = vec![0; len];
            read.read_exact(&mut payload).await?;
            tokio::fs::write(eoa_path, &payload).await?;
        }
        // Init with sealed eoa keys
        (_, Some(data)) => {
            write.write_u8(1).await?;
            write.write_u32(data.len() as u32).await?;
            write.write_all(&data).await?;
        }
    }

    // TODO: bootstrap more funds into existing eoas
    // (true, Some(data)) => {
    //     write.write_u8(2).await?;
    //     write.write_u8(config.bootstrap_keys.len() as u8);
    //     for key in &config.debug_keys {
    //         write.write_all(key).await?;
    //     }
    //     write.write_u32(data.len() as u32).await?;
    //     write.write_all(&data).await?;
    // }

    // Spawn tokio task to send signals into the enclave
    tokio::spawn(async move {
        loop {
            let signal = rx.recv().await.expect("signal channel closed");
            write.write_u32(signal.0.len() as u32).await.unwrap();
            write.write_all(&signal.0).await.unwrap();
        }
    });

    Ok(maybe_report
        .map(|report| report.as_ref().to_vec())
        .unwrap_or_else(|| {
            format!("nosgx=0x{}", hex::encode(publickey))
                .as_bytes()
                .to_vec()
        }))
}

fn start_enclave() {
    // Run the enclave directly with sgx operations mocked
    #[cfg(feature = "nosgx")]
    std::thread::spawn(nomad_enclave::main());

    #[cfg(not(feature = "nosgx"))]
    {
        // TODO: setup enclave runner
    }
}

async fn read_global_key_report(
    reader: &mut OwnedReadHalf,
) -> eyre::Result<([u8; 33], Option<Report>)> {
    let len = reader.read_u32().await? as usize;
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload).await?;

    #[cfg(feature = "nosgx")]
    {
        // read 33 byte public key directly
        if payload.len() != 33 {
            bail!("unexpected global public key payload");
        }
        Ok((array_ref![payload, 0, 33], None))
    }

    #[cfg(not(feature = "nosgx"))]
    {
        // read report and parse key from reportdata
        use eyre::ContextCompat;
        let report = Report::try_copy_from(&payload).context("failed to decode enclave report")?;
        let key = *arrayref::array_ref![report.reportdata, 0, 33];
        Ok((key, Some(report)))
    }
}
