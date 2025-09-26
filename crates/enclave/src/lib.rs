use std::{
    io::{Read, Write},
    net::TcpStream,
};

use ecies::PublicKey;
use nomad_types::{Signal, SignalPayload};

mod bootstrap;
mod global;
mod sealing;

pub fn main() -> eyre::Result<()> {
    main_impl(
        &std::env::args()
            .next()
            .expect("failed to read control socket arg"),
    )
}

pub fn main_impl(addr: &str) -> eyre::Result<()> {
    // Connect to the runner
    let mut stream = TcpStream::connect(addr)?;

    // Bootstrap and/or unseal node eoa accounts
    let (accounts, is_debug) = bootstrap::initialize_eoas(&mut stream)?;
    println!(
        "[init] Loaded {}{} EOAs",
        accounts.len(),
        if is_debug { " debug" } else { "" }
    );

    // Fetch, generate, or unseal the global secret
    let (secret, public) = global::initialize_global_secret(&mut stream, is_debug)?;
    let (_quote, _collateral) = generate_attestation_for_key(&mut stream, public, is_debug, true)?;
    println!(
        "[init] Global Enclave Key (secp256k1): 0x{}",
        hex::encode(public.serialize_compressed())
    );

    // TODO: Setup tcp server with global key and quote, and
    //       distribute the global key to other identical enclaves

    // process incoming signals
    loop {
        // Read u32 length prefixed signal payload from the stream
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;

        let len = u32::from_be_bytes(len) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;

        // Decrypt signal
        let signal: SignalPayload = serde_json::from_slice(&payload)?;
        let Ok(bytes) = ecies::decrypt(&secret.serialize(), &signal.0) else {
            continue;
        };
        let signal: Signal = serde_json::from_slice(&bytes)?;

        todo!("execute {signal}");
    }
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
