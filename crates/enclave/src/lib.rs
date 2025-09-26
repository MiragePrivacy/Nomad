use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::mpsc::SyncSender,
};

use ecies::PublicKey;
use nomad_types::{Signal, SignalPayload};

mod bootstrap;
mod global;
mod sealing;

pub fn main() -> eyre::Result<()> {
    let addr = std::env::args()
        .next()
        .expect("failed to read control socket arg");
    main_impl(TcpStream::connect(addr)?)
}

pub fn main_impl(mut stream: TcpStream) -> eyre::Result<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel(256);

    // fetch, generate, or unseal the global secret
    let (secret, public) = global::initialize_global_secret(&mut stream)?;
    println!(
        "Global Enclave Key (secp256k1): 0x{}",
        hex::encode(public.serialize_compressed())
    );

    // annouce sgx report over our public key to the stream
    let report = report_for_key(public);
    let len = (report.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(&report)?;

    // bootstrap and/or unseal node eoa accounts
    let _accounts = bootstrap::initialize_eoas(&mut stream)?;

    // spawn read thread for processing incoming signals
    let reader = stream.try_clone()?;
    std::thread::spawn(|| read_signals(reader, tx).expect("read thread failed"));

    // process incoming signals
    loop {
        let signal = rx.recv()?;
        let Ok(bytes) = ecies::decrypt(&secret.serialize(), &signal.0) else {
            continue;
        };
        let _signal: Signal = serde_json::from_slice(&bytes)?;

        todo!("execute signal");
    }
}

#[cfg(target_env = "sgx")]
fn report_for_key(publickey: PublicKey) -> Vec<u8> {
    // Generate an attestation report for the global public key
    let mut data = [u8; 64];
    data[0..33].copy_from_slice(publickey.serialize_compressed());
    let targetinfo = sgx_isa::Targetinfo::from(Report::for_self());
    sgx_isa::Report::for_target(&targetinfo, &data)
}

#[cfg(not(target_env = "sgx"))]
fn report_for_key(publickey: PublicKey) -> Vec<u8> {
    // If we're running the enclave without sgx, just send the raw public key
    publickey.serialize_compressed().to_vec()
}

/// Read signals from the stream and send them to the main thread for processing.
///
/// Encoding:
/// ```text
/// [ u32 len . bytes(json(signalpayload)) ]
/// ```
fn read_signals(mut stream: TcpStream, tx: SyncSender<SignalPayload>) -> eyre::Result<()> {
    loop {
        // Read u32 length prefixed signal payload from the stream
        let mut len = [0u8; 4];
        stream.read_exact(&mut len)?;

        let len = u32::from_be_bytes(len) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;

        let signal: SignalPayload = serde_json::from_slice(&payload)?;
        tx.send(signal)?;
    }
}
