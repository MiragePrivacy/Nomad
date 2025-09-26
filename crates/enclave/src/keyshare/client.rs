//! Keyshare Client
//!
//! 1. Connect to server and send attestation containing our client key and debug mode
//! 2. Receive and validate remote attestation containing the global public key
//! 3. Receive ecies payload containing global secret
//! 4. Decrypt with client ecies and validate `secret.publickey == Quote.reportdata.publickey`
//! 5. Seal global key and send to userspace

use std::{
    io::{Read, Write},
    net::{SocketAddrV4, TcpStream},
    time::SystemTime,
};

use arrayref::array_ref;
use ecies::{PublicKey, SecretKey};
use eyre::{ensure, eyre, Context, Result};
use ra_verify::types::{quote::SgxQuote, report::MREnclave};

pub struct KeyshareClient {
    secret: SecretKey,
    quote: Vec<u8>,
    collateral: Vec<u8>,
    mrenclave: MREnclave,
    is_debug: bool,
}

impl KeyshareClient {
    /// Create a new keyshare client
    pub fn new(secret: SecretKey, quote: Vec<u8>, collateral: Vec<u8>) -> Result<Self> {
        let sgx_quote = SgxQuote::read(&mut quote.as_slice()).expect("our own quote to be valid");
        let mrenclave = sgx_quote.quote_body.report_body.mrenclave;
        let is_debug = sgx_quote.quote_body.report_body.sgx_report_data_bytes[62] != 0;
        Ok(Self {
            secret,
            quote,
            collateral,
            mrenclave,
            is_debug,
        })
    }

    /// Request a key from an enclave at the remote address
    pub fn request_key(&self, addr: SocketAddrV4) -> Result<(SecretKey, PublicKey)> {
        // establish tcp connection to addr
        let mut stream = TcpStream::connect(addr).context("failed to dial remote enclave")?;

        // Read remote attestation
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;
        let mut quote = vec![0; u32::from_be_bytes(len) as usize];
        stream.read_exact(&mut quote)?;
        let quote = SgxQuote::read(&mut quote.as_slice())
            .map_err(|e| eyre!("Failed to parse remote enclave quote: {e}"))?;

        // Validate remote enclave attestation
        let (_tcb, report) = ra_verify::verify_remote_attestation(
            // TODO: double check its okay systemtime may be spoofed here
            SystemTime::now(),
            serde_json::from_slice(&self.collateral)?,
            quote,
            &self.mrenclave,
        )
        .map_err(|e| eyre!("Failed to verify remote attestation: {e}"))?;

        // Validate report data and parse public key
        let report_data = report.sgx_report_data_bytes;
        let public = PublicKey::parse_compressed(array_ref![report_data, 0, 33])?;
        let is_debug = report_data[62] != 0;
        ensure!(is_debug == self.is_debug, "Debug modes must match");

        // Send attestation for our client key
        stream.write_all(&(self.quote.len() as u32).to_be_bytes())?;
        stream.write_all(&self.quote)?;

        // Read and decrypt global key
        let mut payload = Vec::new();
        stream.read_to_end(&mut payload)?;
        let decrypted = ecies::decrypt(&self.secret.serialize(), &payload)
            .context("failed to decrypt global key payload")?;
        let secret =
            SecretKey::parse_slice(&decrypted).context("received invalid global secret key")?;
        ensure!(
            public == PublicKey::from_secret_key(&secret),
            "Secret material must match expected global public key"
        );

        Ok((secret, public))
    }
}
