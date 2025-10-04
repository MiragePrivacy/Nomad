use std::{
    io::{Read, Write},
    net::{SocketAddrV4, TcpStream},
};

use color_eyre::{
    eyre::{bail, Context},
    Result,
};
use ecies::{PublicKey, SecretKey};
use log::{error, trace};
use nomad_types::ReportBody;
use sgx_isa::Keypolicy;

use crate::sealing::derive_ecies_key;
use client::KeyshareClient;
pub use server::KeyshareServer;

mod client;
mod server;

/// Label used to derive keyshare client secret
const LOCAL_SECRET_KEY_LABEL: &str = "mirage_client_secret";
/// Label used to derive the global key in the first enclave
const GLOBAL_SECRET_KEY_LABEL: &str = "mirage_global_secret";
/// Label used to derive the sealing key for the global secret
const GLOBAL_SECRET_SEAL_LABEL: &str = "mirage_global_seal_key";

/// Initialize the global secret by reading the first byte;
///   - 0: Generate a new key as the first bootstrap peer
///   - 1: Fetching from given bootstrap peers
///   - 2: Unseal from previous enclave state
pub fn initialize_global_secret(
    stream: &mut TcpStream,
    is_debug: bool,
    chain_id: u64,
) -> Result<(SecretKey, PublicKey, Vec<u8>, Vec<u8>)> {
    trace!("Initializing global secret");
    let mut mode = [0];
    stream.read_exact(&mut mode)?;
    let (secret, public) = match mode[0] {
        // Generate key from scratch
        0 => {
            let (secret, public) = derive_ecies_key(GLOBAL_SECRET_KEY_LABEL)?;
            // Write sealed key back to userspace
            let sealed_key = crate::sealing::seal(
                Keypolicy::all(),
                GLOBAL_SECRET_SEAL_LABEL,
                &secret.serialize(),
            )?;
            let len = (sealed_key.len() as u32).to_be_bytes();
            stream.write_all(&len)?;
            stream.write_all(&sealed_key)?;
            trace!("Generated global secret");
            (secret, public)
        }
        // Peer bootstrap
        1 => {
            // create a client key and generate an attestation for it
            let (client_secret, client_public) = derive_ecies_key(LOCAL_SECRET_KEY_LABEL)?;
            let (client_quote, client_collateral) =
                generate_attestation_for_key(stream, client_public, is_debug, false, chain_id)?;

            // Read peers and fetch the secret from them
            let peers = read_bootstrap_peers(stream)?;
            let (secret, public) =
                fetch_global_secret(peers, client_secret, client_quote, client_collateral)?;

            // Write sealed key back to userspace
            let sealed_key = crate::sealing::seal(
                Keypolicy::all(),
                GLOBAL_SECRET_SEAL_LABEL,
                &secret.serialize(),
            )?;
            let len = (sealed_key.len() as u32).to_be_bytes();
            stream.write_all(&len)?;
            stream.write_all(&sealed_key)?;
            (secret, public)
        }
        // Unseal from userspace
        2 => read_and_unseal_global_secret(stream).context("Failed to unseal global secret")?,
        _ => bail!("Invalid enclave startup mode"),
    };

    let (quote, collateral) =
        generate_attestation_for_key(stream, public, is_debug, true, chain_id)?;
    Ok((secret, public, quote, collateral))
}

/// Get an attestation for a global or ephemeral public key.
///
/// Each report identifies the key attesting for as a client or global key.
/// This is to prevent using the exchange attestations to spoof the global key.
///
/// Reportdata:
/// ```text
/// [ 33 byte secp256k1 public key . zero padding . debug mode . global key ]
/// ```
fn generate_attestation_for_key(
    stream: &mut TcpStream,
    publickey: PublicKey,
    is_debug: bool,
    is_global: bool,
    chain_id: u64,
) -> Result<(Vec<u8>, Vec<u8>)> {
    // Create report data
    let data = ReportBody {
        public_key: publickey.serialize_compressed().into(),
        chain_id,
        is_debug,
        is_global,
    };

    // Generate an attestation report for the enclave public key and eoa debug mode
    #[cfg(target_env = "sgx")]
    let report = {
        // Read and parse target info
        let mut len = [0; 4];
        stream.read_exact(&mut len)?;
        let mut ti = vec![0; u32::from_be_bytes(len) as usize];
        stream.read_exact(&mut ti)?;
        let target_info = sgx_isa::Targetinfo::try_copy_from(&ti).context("Invalid target info")?;

        // Generate report
        let report = sgx_isa::Report::for_target(&target_info, &data.into());
        let report: &[u8] = report.as_ref();
        report.to_vec()
    };

    // If we're running the enclave without sgx, just send the report data directly
    #[cfg(not(target_env = "sgx"))]
    let report: [u8; 64] = data.into();

    trace!("Generated {} byte report", report.len());

    let len = (report.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(&report)?;

    // Read quote response
    let mut len = [0; 4];
    stream.read_exact(&mut len)?;
    let mut quote = vec![0; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut quote)?;

    // Read collateral response
    let mut len = [0; 4];
    stream.read_exact(&mut len)?;
    let mut collateral = vec![0; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut collateral)?;

    Ok((quote, collateral))
}

/// Read a list of peers from the stream.
///
///
/// Encoding:
/// ```text
/// [u8 num peers ] [ u32 ip . u16 port ... ]
/// ```
fn read_bootstrap_peers(stream: &mut TcpStream) -> Result<Vec<SocketAddrV4>> {
    // Read u8 num peers
    let mut num_peers = [0u8];
    stream.read_exact(&mut num_peers)?;
    // each peer socket addr is [u32, u16]
    let mut peers = vec![0u8; num_peers[0] as usize * 6];
    stream.read_exact(&mut peers)?;
    Ok(peers
        .chunks_exact(6)
        .map(|b| {
            let ip = u32::from_be_bytes(b[0..4].try_into().unwrap());
            let port = u16::from_be_bytes(b[4..6].try_into().unwrap());
            SocketAddrV4::new(ip.into(), port)
        })
        .collect())
}

/// Dial bootstrap peers and exchange the global secret key
fn fetch_global_secret(
    peers: Vec<SocketAddrV4>,
    secret: SecretKey,
    quote: Vec<u8>,
    collateral: Vec<u8>,
) -> Result<(SecretKey, PublicKey)> {
    let client = KeyshareClient::new(secret, quote, collateral)?;
    for addr in peers {
        match client.request_key(addr) {
            Err(e) => {
                error!("Failed to get key from remote enclave: {e}");
            }
            // TODO: consider getting keypair from multiple enclaves and using the most
            //       prevelant pair to avoid segmentation or abuse of the bootstrap process
            Ok(keypair) => return Ok(keypair),
        }
    }

    bail!("Failed to get global key from all remote enclaves");
}

/// Read encrypted secret data from the stream and decrypt it
fn read_and_unseal_global_secret(stream: &mut TcpStream) -> Result<(SecretKey, PublicKey)> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;

    let mut payload = vec![0; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut payload)?;

    let secret = crate::sealing::unseal(Keypolicy::all(), GLOBAL_SECRET_SEAL_LABEL, &payload)
        .context("failed to unseal global secret")?;
    let secret = SecretKey::parse_slice(&secret)?;
    let public = PublicKey::from_secret_key(&secret);
    Ok((secret, public))
}
