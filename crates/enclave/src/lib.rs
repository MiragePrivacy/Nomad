use std::{io::Read, net::TcpStream};

use nomad_types::{Signal, SignalPayload};

mod bootstrap;
mod ethereum;
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
    let (keys, is_debug) = bootstrap::initialize_eoas(&mut stream)?;
    println!(
        "[init] Loaded {}{} EOAs",
        keys.len(),
        if is_debug { " debug" } else { "" }
    );

    let eth_client = ethereum::EthClient::new(keys, "todo", "todo".into(), "todo".into())?;

    // Fetch, generate, or unseal the global secret
    let (secret, public, _quote, _collateral) =
        global::initialize_global_secret(&mut stream, is_debug)?;

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

        // Execute signal
        let [eoa_1, eoa_2] = eth_client.select_accounts(&signal)?;
        let [_approve_tx, _bond_tx] = eth_client.bond(eoa_1, &signal)?;
        let transfer_tx = eth_client.transfer(eoa_2, &signal)?;
        let _collect_tx = eth_client.collect(eoa_1, &signal, transfer_tx)?;

        // TODO: Sign and send acknowledgements
    }
}
