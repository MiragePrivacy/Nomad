//! Enclave EOA bootstrapping

use std::{io::Read, net::TcpStream};

use eyre::bail;
use sgx_isa::Keypolicy;

const EOA_SEAL_KEY_LABEL: &str = "mirage_eoas";

/// Initialize EOA accounts by either:
///   - Accepting a list of signed KYC EOA accounts during network bootstrap period
///   - Accepting a bootstrap account and distributing funds into new EOAs
///   - Unsealing existing EOAs
///   - Unsealing and maybe bootstrapping additional funds
///   - DEBUG ONLY: Use raw keys provided
pub fn initialize_eoas(stream: &mut TcpStream) -> eyre::Result<(Vec<[u8; 32]>, bool)> {
    let mut mode = [0];
    stream.read_exact(&mut mode)?;
    match mode[0] {
        0 => handle_kyc_eoas(stream),
        1 => handle_bootstraping_new_eoas(stream),
        2 => handle_unsealing_eoas(stream),
        3 => handle_unsealing_and_maybe_bootstraping(stream),
        255 => handle_debug_eoas(stream),
        _ => bail!("Received invalid EOA mode from userspace"),
    }
}

/// Directly use EOA accounts that have been KYC'd and approved to run on the network.
/// These should only be used in the beginning stages of a network where new nodes need
/// to be able to bootstrap off of these. Eventually they should be replaced with new eoas.
fn handle_kyc_eoas(_stream: &mut TcpStream) -> eyre::Result<(Vec<[u8; 32]>, bool)> {
    // 1. Read private keys from stream
    // 2. Compute H(Public(key0) . Public(key1) ... )
    // 3. Read and verify signature from MRSIGNER
    unimplemented!()
}

/// Distribute funds to new eoas from a bootstrap account
fn handle_bootstraping_new_eoas(_stream: &mut TcpStream) -> eyre::Result<(Vec<[u8; 32]>, bool)> {
    println!("[init] Bootstrapping new EOAs");
    // 1. Read bootstrap account private key
    // 2. OFAC compliance check
    // 3. Generate and seal n EOAs
    // 4. Create random distribution of funds to EOAs
    // 5. Send network signals to userspace for broadcast
    // 6. Poll account balances until transfers are completed
    unimplemented!()
}

/// Unseal EOA accounts from previous enclave state
fn handle_unsealing_eoas(stream: &mut TcpStream) -> eyre::Result<(Vec<[u8; 32]>, bool)> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    let mut payload = vec![0; len];
    stream.read_exact(&mut payload)?;
    let decrypted = crate::sealing::unseal(Keypolicy::MRSIGNER, EOA_SEAL_KEY_LABEL, &payload)?;
    if !decrypted.len().is_multiple_of(32) {
        bail!("invalid decrypted eoa payload");
    }
    Ok((
        decrypted
            .chunks_exact(32)
            .map(|k| k.try_into().unwrap())
            .collect(),
        false,
    ))
}

/// Unseal from enclave state, and also attempt to provision additional funds
/// to existing EOAs from a bootstrap account
fn handle_unsealing_and_maybe_bootstraping(
    _stream: &mut TcpStream,
) -> eyre::Result<(Vec<[u8; 32]>, bool)> {
    unimplemented!()
}

/// DEBUG ONLY: Use raw keys passed directly on the stream
fn handle_debug_eoas(stream: &mut TcpStream) -> eyre::Result<(Vec<[u8; 32]>, bool)> {
    println!("[init] loading raw keys in debug mode");
    let mut num = [0];
    stream.read_exact(&mut num)?;
    let mut keys = vec![0; num[0] as usize * 32];
    stream.read_exact(&mut keys)?;
    Ok((
        keys.chunks_exact(32)
            .map(|k| k.try_into().unwrap())
            .collect(),
        true,
    ))
}
