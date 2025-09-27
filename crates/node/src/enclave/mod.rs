use std::{net::SocketAddrV4, path::PathBuf};

use aesm_client::AesmClient;
use alloy::primitives::Bytes;
use eyre::{bail, eyre, Context, Result};

use nomad_dcap_quote::SgxQlQveCollateral;
use resolve_path::PathResolveExt;
use serde::{Deserialize, Serialize};
use sgx_isa::Report;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tracing::info;

mod quote;

/// Configuration for the enclave
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct EnclaveConfig {
    /// Path to enclave sgxs file
    pub enclave_path: PathBuf,
    /// Directory path to store sealed data (key.bin, eoa.bin).
    /// Defaults to nomad config directory.
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
        let config_path: PathBuf = "~/.config/nomad/".into();
        Self {
            enclave_path: config_path.join("enclave.sgxs"),
            seal_path: config_path,
            nodes: vec![],
            debug_keys: vec![],
            bootstrap_keys: vec![],
            num_accounts: 2,
        }
    }
}

#[allow(unused)]
pub enum EnclaveRequest {
    Signal(Bytes),
    Keyshare(Bytes),
    Withdraw(),
}

impl EnclaveRequest {
    pub fn to_vec(&self) -> Vec<u8> {
        let mut buf = vec![match self {
            EnclaveRequest::Keyshare(_) => 0,
            EnclaveRequest::Signal(_) => 1,
            EnclaveRequest::Withdraw() => 2,
        }];
        match self {
            EnclaveRequest::Signal(bytes) | EnclaveRequest::Keyshare(bytes) => {
                buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                buf.extend_from_slice(bytes);
            }
            _ => {}
        }
        buf
    }
}

/// Spawn the enclave, returning the attestation report for the global public key
pub async fn spawn_enclave(
    config: &EnclaveConfig,
    mut rx: UnboundedReceiver<EnclaveRequest>,
    tx: UnboundedSender<Vec<u8>>,
) -> Result<([u8; 33], bool, Option<(Bytes, SgxQlQveCollateral)>)> {
    #[cfg(feature = "nosgx")]
    start_enclave()?;
    #[cfg(not(feature = "nosgx"))]
    let aesm_client = start_enclave(&config.enclave_path)?;

    // listen for the enclave connection
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 8888)).await?;
    let Ok((mut stream, _)) = listener.accept().await else {
        bail!("Failed to get enclave connection");
    };
    info!("Enclave connection established");

    init_eoa_keys(config, &mut stream)
        .await
        .context("failed to initialize EOA keys")?;

    #[cfg(feature = "nosgx")]
    let fut = init_global_key(config, &mut stream);
    #[cfg(not(feature = "nosgx"))]
    let fut = init_global_key(config, &mut stream, &aesm_client);
    let response = fut.await.context("failed to initialize global key")?;

    // Spawn tokio task to send requests into the enclave
    tokio::spawn(async move {
        loop {
            let request = rx.recv().await.expect("signal channel closed");
            stream.write_all(&request.to_vec()).await.unwrap();
            let len = stream
                .read_u32()
                .await
                .expect("failed to read response length delimiter") as usize;
            let mut buf = vec![0; len];
            stream
                .read_exact(&mut buf)
                .await
                .expect("failed to read reponse payload");
            tx.send(buf).expect("failed to send response to node");
        }
    });

    Ok(response)
}

/// Run the real sgx enclave
#[cfg(not(feature = "nosgx"))]
fn start_enclave(path: &std::path::Path) -> Result<AesmClient> {
    use enclave_runner::EnclaveBuilder;
    use sgxs_loaders::isgx;

    info!("Starting sgx enclave ...");

    let aesm_client = AesmClient::new();
    let mut device = isgx::Device::new()
        .unwrap()
        .einittoken_provider(aesm_client.clone())
        .build();

    // TODO: fetch enclave and signature if they don't exist.
    let mut enclave_builder = EnclaveBuilder::new(path);

    // TODO: load MRSIGNER signature
    enclave_builder.dummy_signature();

    let enclave = enclave_builder
        .build(&mut device)
        .map_err(|e| eyre!("Failed to build enclave: {e}"))?;
    tokio::task::spawn_blocking(|| enclave.run().expect("uh oh, enclave crashed"));
    Ok(aesm_client)
}

/// Run the enclave directly with all sgx operations mocked
#[cfg(feature = "nosgx")]
fn start_enclave() -> Result<()> {
    info!("Starting mocked enclave ...");
    tokio::task::spawn_blocking(|| {
        nomad_enclave::Enclave::new("localhost:8888")
            .run()
            .expect("uh oh, enclave crashed")
    });
}

/// Global key initialization routine
async fn init_global_key(
    config: &EnclaveConfig,
    stream: &mut TcpStream,
    #[cfg(not(feature = "nosgx"))] aesm_client: &AesmClient,
) -> Result<([u8; 33], bool, Option<(Bytes, SgxQlQveCollateral)>)> {
    let key_path = config.seal_path.join("key.bin");
    let key_path = key_path.resolve();
    if let Ok(data) = std::fs::read(&key_path) {
        // Restore key from sealed data
        stream.write_u8(2).await?;
        stream.write_u32(data.len() as u32).await?;
        stream.write_all(&data).await?;
    } else {
        if !config.nodes.is_empty() {
            // Bootstrap from other node enclaves
            stream.write_u8(1).await?;

            // Enclave will report its client key and read the quote and collateral
            #[cfg(feature = "nosgx")]
            let fut = read_report_and_reply_with_quote(stream);
            #[cfg(not(feature = "nosgx"))]
            let fut = read_report_and_reply_with_quote(stream, aesm_client);
            fut.await.context("failed to read report from enclave")?;

            // Send enclave addresses
            stream.write_u8(config.nodes.len() as u8).await?;
            for addr in &config.nodes {
                stream.write_u32(addr.ip().to_bits()).await?;
                stream.write_u16(addr.port()).await?;
            }
        } else {
            // First node, generate key
            stream.write_u8(0).await?;
        }

        // Read back sealed key and write to disk
        let len = stream.read_u32().await? as usize;
        let mut payload = vec![0; len];
        stream.read_exact(&mut payload).await?;
        tokio::fs::write(key_path, &payload).await?;
    }

    // Now that enclave has the global key, we wait for a report
    // and quote it, returning the data for adding to the api server.
    // The enclave will reuse this quote for bootstrapping new enclaves.
    #[cfg(feature = "nosgx")]
    let fut = read_report_and_reply_with_quote(stream);
    #[cfg(not(feature = "nosgx"))]
    let fut = read_report_and_reply_with_quote(stream, aesm_client);
    fut.await.context("failed to read report from enclave")
}

async fn read_report_and_reply_with_quote(
    stream: &mut TcpStream,
    #[cfg(not(feature = "nosgx"))] aesm_client: &AesmClient,
) -> Result<([u8; 33], bool, Option<(Bytes, SgxQlQveCollateral)>)> {
    let len = stream.read_u32().await? as usize;
    let mut payload = vec![0; len];
    stream.read_exact(&mut payload).await?;

    #[cfg(feature = "nosgx")]
    {
        // read report data directly
        if payload.len() != 64 {
            bail!("unexpected test public key");
        }
        stream.write_u32(0).await?;
        stream.write_u32(0).await?;

        Ok((
            *arrayref::array_ref![payload, 0, 33],
            payload[62] != 0,
            None,
        ))
    }

    #[cfg(not(feature = "nosgx"))]
    {
        use eyre::ContextCompat;

        // Read report and parse public key
        let report = Report::try_copy_from(&payload).context("failed to decode enclave report")?;
        let key = *arrayref::array_ref![report.reportdata, 0, 33];

        // Generate a quote for the report
        let (quote, collateral, _) = quote::get_quote_for_report(aesm_client, &report)?;
        stream.write_u32(quote.len() as u32).await?;
        stream.write_all(&quote).await?;
        let collateral_bytes = serde_json::to_vec(&collateral)?;
        stream.write_u32(collateral_bytes.len() as u32).await?;
        stream.write_all(&collateral_bytes).await?;

        Ok((key, report.reportdata[62] != 0, Some((quote, collateral))))
    }
}

async fn init_eoa_keys(config: &EnclaveConfig, stream: &mut TcpStream) -> Result<()> {
    // EOA account initialization
    let eoa_path = config.seal_path.join("eoa.bin");
    let eoa_path = eoa_path.resolve();
    match (
        !config.bootstrap_keys.is_empty(),
        std::fs::read(&eoa_path).ok(),
    ) {
        // init with debug keys
        (false, None) => {
            stream.write_u8(255).await?;
            stream.write_u8(config.debug_keys.len() as u8).await?;
            for key in &config.debug_keys {
                stream.write_all(key).await?;
            }
        }
        // Initialize new EOAs with funds from bootstrap accounts
        (true, None) => {
            stream.write_u8(0).await?;
            stream.write_u8(config.bootstrap_keys.len() as u8).await?;
            for key in &config.bootstrap_keys {
                stream.write_all(key).await?;
            }

            // Read sealed keys from enclave and save to disk
            let len = stream.read_u32().await? as usize;
            let mut payload = vec![0; len];
            stream.read_exact(&mut payload).await?;
            tokio::fs::write(eoa_path, &payload).await?;
        }
        // Init with sealed eoa keys
        (_, Some(data)) => {
            stream.write_u8(1).await?;
            stream.write_u32(data.len() as u32).await?;
            stream.write_all(&data).await?;
        }
    }

    // TODO: poll and bootstrap additional funds into existing eoas
    // (true, Some(data)) => {
    //     write.write_u8(2).await?;
    //     write.write_u8(config.bootstrap_keys.len() as u8);
    //     for key in &config.debug_keys {
    //         write.write_all(key).await?;
    //     }
    //     write.write_u32(data.len() as u32).await?;
    //     write.write_all(&data).await?;
    // }

    Ok(())
}
