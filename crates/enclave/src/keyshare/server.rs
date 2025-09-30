use std::{
    io::{Read, Write},
    net::TcpStream,
    time::SystemTime,
};

use ecies::{PublicKey, SecretKey};
use eyre::{ensure, eyre, Context, Result};
use ra_verify::types::{quote::SgxQuote, report::MREnclave};

pub struct KeyshareServer {
    quote: Vec<u8>,
    collateral: Vec<u8>,
    mrenclave: MREnclave,
    is_debug: bool,
}

impl KeyshareServer {
    /// Create a new keyshare server
    pub fn new(quote: Vec<u8>, collateral: Vec<u8>) -> Self {
        #[cfg(target_env = "sgx")]
        let (mrenclave, is_debug) = {
            let report = SgxQuote::read(&mut quote.as_slice())
                .expect("our own quote to be valid")
                .quote_body
                .report_body;
            (report.mrenclave, report.sgx_report_data_bytes[62] != 0)
        };
        #[cfg(not(target_env = "sgx"))]
        let (mrenclave, is_debug) = ([42; 32], true);

        Self {
            quote,
            collateral,
            mrenclave,
            is_debug,
        }
    }

    pub fn handle_request(&self, stream: &mut TcpStream, secret: &SecretKey) -> Result<()> {
        // Send global public key attestation
        stream.write_all(&self.quote.len().to_be_bytes())?;

        // Read client attestation
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;
        let mut quote = vec![0; u32::from_be_bytes(len) as usize];
        stream.read_exact(&mut quote)?;
        let quote = SgxQuote::read(&mut quote.as_slice())
            .map_err(|e| eyre!("Failed to parse remote enclave quote: {e}"))?;

        // Verify client attestation
        let (_tcb, report) = ra_verify::verify_remote_attestation(
            SystemTime::now(),
            serde_json::from_slice(&self.collateral)?,
            quote,
            &self.mrenclave,
        )
        .map_err(|e| eyre!("Failed to verify remote attestation: {e}"))?;

        // Validate report data and extract client key
        let is_debug = report.sgx_report_data_bytes[62] != 0;
        ensure!(is_debug == self.is_debug, "Debug states must match");
        let is_global = report.sgx_report_data_bytes[63] != 0;
        ensure!(
            !is_global,
            "Client attestation must not be for a global key"
        );

        let public =
            PublicKey::parse_compressed(arrayref::array_ref![report.sgx_report_data_bytes, 0, 33])
                .context("Received invalid public key in client attestation")?;

        // Encrypt global secret for client key
        let encrypted = ecies::encrypt(&public.serialize(), &secret.serialize())
            .context("Failed to encrypt global secret")?;

        // Send encrypted secret response
        stream.write_all(&(encrypted.len() as u32).to_be_bytes())?;
        stream.write_all(&encrypted)?;
        Ok(())
    }
}
