use std::{
    io::{Read, Write},
    net::TcpStream,
    time::SystemTime,
};

use color_eyre::{
    eyre::{ensure, eyre, Context},
    Result,
};
use ecies::{PublicKey, SecretKey};
use nomad_types::ReportBody;
use ra_verify::types::{quote::SgxQuote, report::MREnclave};

pub struct KeyshareServer {
    quote: Vec<u8>,
    collateral: Vec<u8>,
    mrenclave: MREnclave,
    report: ReportBody,
}

impl KeyshareServer {
    /// Create a new keyshare server
    pub fn new(quote: Vec<u8>, collateral: Vec<u8>) -> Self {
        #[cfg(target_env = "sgx")]
        let (mrenclave, report) = {
            let report = SgxQuote::read(&mut quote.as_slice())
                .expect("our own quote to be valid")
                .quote_body
                .report_body;
            (report.mrenclave, report.sgx_report_data_bytes.into())
        };
        #[cfg(not(target_env = "sgx"))]
        let (mrenclave, report) = (
            [42; 32],
            ReportBody {
                public_key: [128; 33].into(),
                chain_id: 11155111,
                is_global: true,
                is_debug: true,
            },
        );

        Self {
            quote,
            collateral,
            mrenclave,
            report,
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

        // Verify report data
        let report = ReportBody::from(report.sgx_report_data_bytes);
        let public = PublicKey::parse_compressed(&report.public_key.into())
            .context("Received invalid public key in client attestation")?;
        ensure!(!report.is_global, "Attestation must be for a client key");
        ensure!(
            report.is_debug == self.report.is_debug,
            "Debug states must match"
        );
        ensure!(
            report.chain_id == self.report.chain_id,
            "Chain ids must match"
        );

        // Encrypt global secret for client key
        let encrypted = ecies::encrypt(&public.serialize(), &secret.serialize())
            .context("Failed to encrypt global secret")?;

        // Send encrypted secret response
        stream.write_all(&(encrypted.len() as u32).to_be_bytes())?;
        stream.write_all(&encrypted)?;
        Ok(())
    }
}
