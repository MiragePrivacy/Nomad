use std::{
    io::{Read, Write},
    net::{SocketAddrV4, TcpStream},
};

use ecies::{PublicKey, SecretKey};
use eyre::{bail, Context};
use sgx_isa::Keypolicy;

mod client;
mod server;

const GLOBAL_SECRET_SEAL_LABEL: &str = "mirage_global_seal_key";
const GLOBAL_SECRET_KEY_LABEL: &str = "mirage_global_secret";
const LOCAL_SECRET_KEY_LABEL: &str = "mirage_ephemeral_key";

/// Initialize the global secret by reading the first byte;
///   - 0: Generate a new key as the first bootstrap peer
///   - 1: Fetching from given bootstrap peers
///   - 2: Unseal from previous enclave state
pub fn initialize_global_secret(
    stream: &mut TcpStream,
    is_debug: bool,
) -> eyre::Result<(SecretKey, PublicKey, Vec<u8>, Vec<u8>)> {
    let mut mode = [0];
    stream.read_exact(&mut mode)?;
    let (secret, public) = match mode[0] {
        // Generate key from scratch
        0 => {
            let (secret, public) = derive_enclave_key(GLOBAL_SECRET_KEY_LABEL)?;
            // Write sealed key back to userspace
            let sealed_key = seal_key_data(secret)?;
            let len = (sealed_key.len() as u32).to_be_bytes();
            stream.write_all(&len)?;
            stream.write_all(&sealed_key)?;
            (secret, public)
        }
        // Peer bootstrap
        1 => {
            // create a client key and generate a (client) attestation for it
            let (client_secret, client_public) = derive_enclave_key(LOCAL_SECRET_KEY_LABEL)?;
            let (client_quote, client_collateral) =
                generate_attestation_for_key(stream, client_public, is_debug, false)?;

            // Read peers and fetch the secret from them
            let peers = read_bootstrap_peers(stream)?;
            let (secret, public) = fetch_global_secret(
                peers,
                client_secret,
                client_public,
                client_quote,
                client_collateral,
            )?;

            // Write sealed key back to userspace
            let sealed_key = seal_key_data(secret)?;
            let len = (sealed_key.len() as u32).to_be_bytes();
            stream.write_all(&len)?;
            stream.write_all(&sealed_key)?;
            (secret, public)
        }
        // Unseal from userspace
        2 => read_and_unseal_global_secret(stream)?,
        _ => bail!("Invalid enclave startup mode"),
    };

    let (quote, collateral) = generate_attestation_for_key(stream, public, is_debug, true)?;
    Ok((secret, public, quote, collateral))
}

/// Generate a new secret key
fn derive_enclave_key(label: &str) -> eyre::Result<(SecretKey, PublicKey)> {
    let data = crate::sealing::SealData::new_from_label(Keypolicy::all(), label);
    let key = crate::sealing::egetkey(&data)?;
    let secret = SecretKey::parse(&key)?;
    let public = PublicKey::from_secret_key(&secret);
    Ok((secret, public))
}

/// Seal a key for future inits
fn seal_key_data(secret: SecretKey) -> eyre::Result<Vec<u8>> {
    crate::sealing::seal(
        Keypolicy::all(),
        GLOBAL_SECRET_SEAL_LABEL,
        &secret.serialize(),
    )
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
) -> eyre::Result<(Vec<u8>, Vec<u8>)> {
    // Create report data
    let mut data = [0u8; 64];
    data[0..33].copy_from_slice(&publickey.serialize_compressed());
    data[62] = is_debug as u8;
    data[63] = is_global as u8;

    #[cfg(target_env = "sgx")]
    // Generate an attestation report for the enclave public key and eoa debug mode
    let report =
        sgx_isa::Report::for_target(&sgx_isa::Targetinfo::from(Report::for_self()), &data).to_vec();

    #[cfg(not(target_env = "sgx"))]
    // If we're running the enclave without sgx, just send the raw public key
    let report = data.to_vec();

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
fn read_bootstrap_peers(stream: &mut TcpStream) -> eyre::Result<Vec<SocketAddrV4>> {
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
    _peers: Vec<SocketAddrV4>,
    _secret: SecretKey,
    _public: PublicKey,
    _quote: Vec<u8>,
    _collateral: Vec<u8>,
) -> eyre::Result<(SecretKey, PublicKey)> {
    unimplemented!("client.rs")
}

/// Read encrypted secret data from the stream and decrypt it
fn read_and_unseal_global_secret(stream: &mut TcpStream) -> eyre::Result<(SecretKey, PublicKey)> {
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
