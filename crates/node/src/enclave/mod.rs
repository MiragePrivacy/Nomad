use std::{
    net::{SocketAddrV4, ToSocketAddrs},
    path::PathBuf,
    time::Duration,
};

use alloy::primitives::Bytes;
use eyre::{bail, Context, Result};
use nomad_types::ReportBody;
use reqwest::Url;
use resolve_path::PathResolveExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tracing::info;

#[cfg(not(feature = "nosgx"))]
mod quote;

/// Configuration for the enclave
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct EnclaveConfig {
    /// Path to enclave sgxs file
    pub enclave_path: PathBuf,
    pub signature_path: PathBuf,
    /// Directory path to store sealed data (key.bin, eoa.bin).
    /// Defaults to nomad config directory.
    pub seal_path: PathBuf,

    pub geth_rpc: Url,
    pub builder_rpc: Url,
    pub builder_atls: Url,

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
            signature_path: config_path.join("mirage.sig"),
            seal_path: config_path,
            nodes: vec![],
            debug_keys: vec![],
            bootstrap_keys: vec![],
            num_accounts: 2,
            geth_rpc: "https://ethereum-sepolia-rpc.publicnode.com"
                .parse()
                .unwrap(),
            builder_rpc: "https://rpc.buildernet.org".parse().unwrap(),
            builder_atls: "https://rpc.buildernet.org:7936".parse().unwrap(),
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

/// Userspace runner for the nomad enclave
pub struct EnclaveRunner {
    config: EnclaveConfig,
    stream: TcpStream,
    #[cfg(not(feature = "nosgx"))]
    aesm_client: aesm_client::AesmClient,
}

impl EnclaveRunner {
    /// Create and spawn the enclave in a new process, and wait for the connection.
    /// For nosgx, the enclave is run as a blocking task in tokio.
    pub async fn create_enclave(config: EnclaveConfig) -> Result<Self> {
        // Bind listener for the enclave connection
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 8888)).await?;

        #[cfg(not(feature = "nosgx"))]
        let aesm_client = aesm_client::AesmClient::new();
        #[cfg(not(feature = "nosgx"))]
        {
            info!("Starting sgx enclave ...");
            let mut device = sgxs_loaders::isgx::Device::new()
                .unwrap()
                .einittoken_provider(aesm_client.clone())
                .build();

            // TODO: fetch enclave and signature if they don't exist.
            let mut enclave_builder = enclave_runner::EnclaveBuilder::new(&config.enclave_path);
            enclave_builder
                .signature(&config.signature_path)?
                .arg("127.0.0.1:8888");
            let enclave = enclave_builder
                .build(&mut device)
                .map_err(|e| eyre::eyre!("Failed to build enclave: {e}"))?;
            tokio::task::spawn_blocking(|| enclave.run().expect("uh oh, enclave crashed"));
        }

        #[cfg(feature = "nosgx")]
        {
            info!("Starting mocked enclave ...");
            tokio::task::spawn_blocking(|| {
                nomad_enclave::Enclave::init("localhost:8888")
                    .expect("failed to initialize non-sgx enclave")
                    .run()
                    .expect("uh oh, non-sgx enclave crashed")
            });
        }

        // Accept the connection from the enclave
        let Ok((stream, _)) = listener.accept().await else {
            bail!("Failed to get enclave connection");
        };
        info!("Enclave connection established");

        Ok(Self {
            config,
            stream,
            #[cfg(not(feature = "nosgx"))]
            aesm_client,
        })
    }

    /// Initialize the enclave, returning the attestation report for the global public key
    pub async fn initialize(&mut self) -> Result<(ReportBody, Option<(Bytes, serde_json::Value)>)> {
        self.init_eoa_keys()
            .await
            .context("failed to initialize EOA keys")?;

        // Send ethereum config
        let payload = serde_json::to_vec(&json!({
            "geth_rpc": self.config.geth_rpc.as_str().to_socket_addrs().unwrap().next(),
            "builder_rpc": self.config.builder_rpc.as_str().to_socket_addrs().unwrap().next(),
            "builder_atls": self.config.builder_atls.as_str().to_socket_addrs().unwrap().next(),
            "min_eth": 0.05
        }))
        .unwrap();
        self.stream.write_u32(payload.len() as u32).await?;
        self.stream.write_all(&payload).await?;

        let response = self
            .init_global_key()
            .await
            .context("failed to initialize global key")?;

        Ok(response)
    }

    /// Consume the runner and spawn a request/response handler
    pub fn spawn_handler(
        mut self,
        mut rx: UnboundedReceiver<EnclaveRequest>,
        tx: UnboundedSender<Vec<u8>>,
    ) {
        // Spawn tokio task to handle the enclave stream
        tokio::spawn(async move {
            loop {
                let request = rx.recv().await.expect("signal channel closed");
                self.stream.write_all(&request.to_vec()).await.unwrap();

                'inner: loop {
                    let len = self
                        .stream
                        .read_u32()
                        .await
                        .expect("failed to read response length delimiter");

                    // Check if enclave requested a timeout
                    if len == u32::MAX {
                        tokio::time::sleep(Duration::from_secs(4)).await;
                        self.stream
                            .write_u8(0)
                            .await
                            .expect("failed to send timeout release");
                        continue 'inner;
                    }

                    // Handle enclave response
                    let buf = if len != 0 {
                        let mut buf = vec![0; len as usize];
                        self.stream
                            .read_exact(&mut buf)
                            .await
                            .expect("failed to read reponse payload");
                        buf
                    } else {
                        Vec::new()
                    };
                    tx.send(buf).expect("failed to send response to node");
                    break 'inner;
                }
            }
        });
    }

    /// Initialize EOA accounts from a bootstrap account, sealed data, both, or debug only keys
    async fn init_eoa_keys(&mut self) -> Result<()> {
        // EOA account initialization
        let eoa_path = self.config.seal_path.join("eoa.bin");
        let eoa_path = eoa_path.resolve();
        match (
            !self.config.bootstrap_keys.is_empty(),
            std::fs::read(&eoa_path).ok(),
        ) {
            // init with debug keys
            (false, None) => {
                self.stream.write_u8(255).await?;
                self.stream
                    .write_u8(self.config.debug_keys.len() as u8)
                    .await?;
                for key in &self.config.debug_keys {
                    self.stream.write_all(key).await?;
                }
            }
            // Initialize new EOAs with funds from bootstrap accounts
            (true, None) => {
                self.stream.write_u8(0).await?;
                self.stream
                    .write_u8(self.config.bootstrap_keys.len() as u8)
                    .await?;
                for key in &self.config.bootstrap_keys {
                    self.stream.write_all(key).await?;
                }

                // Read sealed keys from enclave and save to disk
                let len = self.stream.read_u32().await? as usize;
                let mut payload = vec![0; len];
                self.stream.read_exact(&mut payload).await?;
                tokio::fs::write(eoa_path, &payload).await?;
            }
            // Init with sealed eoa keys
            (_, Some(data)) => {
                self.stream.write_u8(1).await?;
                self.stream.write_u32(data.len() as u32).await?;
                self.stream.write_all(&data).await?;
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

    /// Global key initialization routine
    async fn init_global_key(
        &mut self,
    ) -> Result<(ReportBody, Option<(Bytes, serde_json::Value)>)> {
        let key_path = self.config.seal_path.join("key.bin");
        let key_path = key_path.resolve();
        if let Ok(data) = std::fs::read(&key_path) {
            // Restore key from sealed data
            self.stream.write_u8(2).await?;
            self.stream.write_u32(data.len() as u32).await?;
            self.stream.write_all(&data).await?;
        } else {
            if !self.config.nodes.is_empty() {
                // Bootstrap from other node enclaves
                self.stream.write_u8(1).await?;

                // Enclave will report its client key and read the quote and collateral
                self.read_report_and_reply_with_quote()
                    .await
                    .context("failed to read report from enclave")?;

                // Send enclave addresses
                self.stream.write_u8(self.config.nodes.len() as u8).await?;
                for addr in &self.config.nodes {
                    self.stream.write_u32(addr.ip().to_bits()).await?;
                    self.stream.write_u16(addr.port()).await?;
                }
            } else {
                // First node, generate key
                self.stream.write_u8(0).await?;
            }

            // Read back sealed key and write to disk
            let len = self.stream.read_u32().await? as usize;
            let mut payload = vec![0; len];
            self.stream.read_exact(&mut payload).await?;
            tokio::fs::write(key_path, &payload).await?;
        }

        // Now that enclave has the global key, we wait for a report
        // and quote it, returning the data for adding to the api server.
        // The enclave will reuse this quote for bootstrapping new enclaves.
        self.read_report_and_reply_with_quote()
            .await
            .context("failed to read report from enclave")
    }

    async fn read_report_and_reply_with_quote(
        &mut self,
    ) -> Result<(ReportBody, Option<(Bytes, serde_json::Value)>)> {
        let len = self.stream.read_u32().await? as usize;
        let mut payload = vec![0; len];
        self.stream.read_exact(&mut payload).await?;

        #[cfg(feature = "nosgx")]
        {
            // read report data directly
            if payload.len() != 64 {
                bail!("unexpected test public key");
            }
            self.stream.write_u32(0).await?;
            self.stream.write_u32(0).await?;
            Ok((
                ReportBody::from(*arrayref::array_ref![payload, 0, 64]),
                None,
            ))
        }

        #[cfg(not(feature = "nosgx"))]
        {
            use eyre::ContextCompat;
            use serde_json::to_value;

            // Read report and parse public key
            let report = sgx_isa::Report::try_copy_from(&payload)
                .context("failed to decode enclave report")?;
            let data = ReportBody::from(report.reportdata);

            // Generate a quote for the report
            let (quote, collateral, _) = quote::get_quote_for_report(&self.aesm_client, &report)?;
            let collateral = to_value(collateral)?;
            self.stream.write_u32(quote.len() as u32).await?;
            self.stream.write_all(&quote).await?;
            let collateral_bytes = serde_json::to_vec(&collateral)?;
            self.stream.write_u32(collateral_bytes.len() as u32).await?;
            self.stream.write_all(&collateral_bytes).await?;

            Ok((data, Some((quote, collateral))))
        }
    }
}
