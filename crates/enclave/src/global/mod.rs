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
) -> eyre::Result<(SecretKey, PublicKey)> {
    let mut mode = [0];
    stream.read_exact(&mut mode)?;
    match mode[0] {
        // Generate key from scratch
        0 => {
            let (secret, public) = derive_enclave_key(GLOBAL_SECRET_KEY_LABEL)?;

            // Write sealed key back to userspace
            let sealed_key = seal_key_data(secret)?;
            let len = (sealed_key.len() as u32).to_be_bytes();
            stream.write_all(&len)?;
            stream.write_all(&sealed_key)?;

            Ok((secret, public))
        }
        // Peer bootstrap
        1 => {
            // create a client key and generate a (client) attestation for it
            let (secret, public) = derive_enclave_key(LOCAL_SECRET_KEY_LABEL)?;
            let (quote, collateral) =
                crate::generate_attestation_for_key(stream, public, is_debug, false)?;

            // Read peers and fetch the secret from them
            let peers = read_bootstrap_peers(stream)?;
            let (secret, public) = fetch_global_secret(peers, secret, public, quote, collateral)?;

            // Write sealed key back to userspace
            let sealed_key = seal_key_data(secret)?;
            let len = (sealed_key.len() as u32).to_be_bytes();
            stream.write_all(&len)?;
            stream.write_all(&sealed_key)?;

            Ok((secret, public))
        }
        // Unseal from userspace
        2 => read_and_unseal_global_secret(stream),
        _ => bail!("Invalid enclave startup mode"),
    }
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
