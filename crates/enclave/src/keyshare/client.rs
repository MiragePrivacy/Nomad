use std::{net::SocketAddrV4, time::SystemTime};

use ecies::{PublicKey, SecretKey};
use eyre::{bail, ensure, eyre, Context, Result};
use nomad_types::{AttestResponse, KeyRequest, ReportBody};
use ra_verify::types::{quote::SgxQuote, report::MREnclave};

pub struct KeyshareClient {
    secret: SecretKey,
    quote: Vec<u8>,
    collateral: Vec<u8>,
    mrenclave: MREnclave,
    report: ReportBody,
}

impl KeyshareClient {
    /// Create a new keyshare client
    pub fn new(secret: SecretKey, quote: Vec<u8>, collateral: Vec<u8>) -> Result<Self> {
        #[cfg(target_env = "sgx")]
        let (mrenclave, report) = {
            let sgx_quote =
                SgxQuote::read(&mut quote.as_slice()).expect("our own quote to be valid");
            (
                sgx_quote.quote_body.report_body.mrenclave,
                ReportBody::from(sgx_quote.quote_body.report_body.sgx_report_data_bytes),
            )
        };
        #[cfg(not(target_env = "sgx"))]
        let (mrenclave, report) = (
            [42; 32],
            ReportBody {
                public_key: [0; 33].into(),
                chain_id: 111333111,
                is_debug: true,
                is_global: false,
            },
        );

        Ok(Self {
            secret,
            quote,
            collateral,
            mrenclave,
            report,
        })
    }

    /// Request a key from an enclave at the remote address (using nomad api)
    pub fn request_key(&self, addr: SocketAddrV4) -> Result<(SecretKey, PublicKey)> {
        // Get global key attestation from remote enclave
        let AttestResponse {
            attestation: Some(attestation),
            ..
        } = ureq::get(format!("http://{addr}/attest"))
            .call()?
            .body_mut()
            .read_json::<AttestResponse>()?
        else {
            bail!("Peer is not running with sgx");
        };

        // Validate remote enclave attestation against our own collateral
        let quote = SgxQuote::read(&mut attestation.quote.as_ref())
            .map_err(|e| eyre!("Failed to parse remote enclave quote: {e}"))?;
        let (_tcb, report) = ra_verify::verify_remote_attestation(
            // TODO: double check its okay systemtime may be spoofed here
            SystemTime::now(),
            serde_json::from_slice(&self.collateral).context("invalid client collateral")?,
            quote,
            &self.mrenclave,
        )
        .map_err(|e| eyre!("Failed to verify remote attestation: {e}"))?;

        // Validate report data and parse public key
        let report = ReportBody::from(report.sgx_report_data_bytes);
        let public = PublicKey::parse_compressed(&report.public_key.into())
            .context("Invalid client public key")?;
        ensure!(report.is_global, "Attestation must be for a global key");
        ensure!(
            report.is_debug == self.report.is_debug,
            "Debug modes must match"
        );
        ensure!(
            report.chain_id == self.report.chain_id,
            "Chain ids must match"
        );

        // Send attestation for our client key
        let encrypted = ureq::post(format!("http://{addr}/key"))
            .send_json(KeyRequest {
                quote: self.quote.clone().into(),
            })?
            .into_body()
            .read_to_vec()?;

        // Decrypt global key
        let decrypted = ecies::decrypt(&self.secret.serialize(), &encrypted)
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
