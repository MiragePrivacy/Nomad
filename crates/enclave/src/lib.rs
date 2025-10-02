use std::{
    io::{Read, Write},
    net::TcpStream,
};

use ecies::SecretKey;
use eyre::{bail, Context, ContextCompat};
use nomad_types::Signal;
use tracing::{error, info, instrument};

use crate::{
    ethereum::{EthClient, EthConfig},
    keyshare::KeyshareServer,
};

mod bootstrap;
mod ethereum;
mod keyshare;
mod sealing;

pub struct Enclave {
    keyshare: KeyshareServer,
    eth_client: EthClient,
    secret: SecretKey,
    stream: TcpStream,
}

impl Enclave {
    #[instrument(skip_all)]
    pub fn init(addr: &str) -> eyre::Result<Self> {
        // Setup crypto provider
        rustls_rustcrypto::provider()
            .install_default()
            .ok()
            .context("Failed to setup rustcrypto tls provider")?;

        // Connect to the runner
        let mut stream = TcpStream::connect(addr)?;

        // Bootstrap and/or unseal node eoa accounts
        let (keys, is_debug) = bootstrap::initialize_eoas(&mut stream)?;
        info!(
            "Loaded {}{} EOAs",
            keys.len(),
            if is_debug { " debug" } else { "" }
        );

        let config =
            EthConfig::read_from_stream(&mut stream).context("Failed to read enclave config")?;

        // Create eth client and prefetch attestated tls certificates
        let eth_client = ethereum::EthClient::new(keys, config)?;

        // Fetch, generate, or unseal the global secret
        let (secret, public, quote, collateral) =
            keyshare::initialize_global_secret(&mut stream, is_debug)?;

        info!(
            "Global Enclave Key: 0x{}",
            hex::encode(public.serialize_compressed())
        );

        Ok(Self {
            keyshare: KeyshareServer::new(quote, collateral),
            eth_client,
            secret,
            stream,
        })
    }

    /// Main thread loop
    pub fn run(mut self) -> eyre::Result<()> {
        loop {
            let mut kind = [0];
            self.stream.read_exact(&mut kind)?;
            match kind[0] {
                0 => self.handle_keyshare_request()?,
                1 => self.handle_signal_request()?,
                2 => todo!("handle withdraw request"),
                _ => bail!("Invalid request kind"),
            }
        }
    }

    fn handle_keyshare_request(&mut self) -> eyre::Result<()> {
        self.keyshare.handle_request(&mut self.stream, &self.secret)
    }

    #[instrument(name = "signal", skip_all)]
    fn handle_signal_request(&mut self) -> eyre::Result<()> {
        // Read u32 length prefixed signal payload from the stream
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;

        let len = u32::from_be_bytes(len) as usize;
        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload)?;

        // Decrypt signal
        let Ok(bytes) = ecies::decrypt(&self.secret.serialize(), &payload) else {
            self.stream.write_all(&0u32.to_be_bytes())?;
            return Ok(());
        };
        let Ok(signal) = serde_json::from_slice::<Signal>(&bytes) else {
            self.stream.write_all(&0u32.to_be_bytes())?;
            return Ok(());
        };

        // Execute signal
        if let Err(e) = self.execute_signal(signal) {
            error!("Failed to execute signal: {e:#}");
            self.stream.write_all(&0u32.to_be_bytes())?;
            return Ok(());
        }

        Ok(())
    }

    #[instrument(name = "execute", skip_all, fields(signal.token_contract))]
    fn execute_signal(&mut self, signal: Signal) -> eyre::Result<()> {
        let stream = &mut self.stream;
        self.eth_client.validate_signal(&signal)?;
        info!("Selecting accounts");
        let [eoa_1, eoa_2] = self.eth_client.select_accounts(&signal)?;
        info!("Bonding contract");
        let [_approve_tx, _bond_tx] = self.eth_client.bond(stream, eoa_1, &signal)?;
        info!("Transferring funds");
        let transfer_tx = self.eth_client.transfer(stream, eoa_2, &signal)?;
        info!("Collecting rewards");
        let _collect_tx = self
            .eth_client
            .collect(stream, eoa_1, &signal, transfer_tx)?;
        info!("Successfully executed signal");
        self.stream.write_all(&0u32.to_be_bytes())?;
        Ok(())
    }
}
