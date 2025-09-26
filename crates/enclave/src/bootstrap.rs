//! Enclave EOA bootstrapping

use std::{io::Read, net::TcpStream};

use eyre::bail;
use sgx_isa::Keypolicy;

const EOA_SEAL_KEY_LABEL: &str = "mirage_eoas";

/// Initialize EOA accounts by either:
///   - Accepting a bootstrap account and distribution configuration
///   - Unsealing existing EOAs
///   - DEBUG ONLY: Use raw keys provided
pub fn initialize_eoas(stream: &mut TcpStream) -> eyre::Result<Vec<[u8; 32]>> {
    let mut mode = [0];
    stream.read_exact(&mut mode)?;
    match mode[0] {
        0 => {
            // Distribution from bootstrap account
            // 1. Read bootstrap account private key
            // 2. OFAC compliance check
            // 3. Send network signals to userspace
            // 4. Monitor account balances until balances are filled
            unimplemented!()
        }
        1 => unseal_eoas(stream),
        2 => {
            // unseal from enclave state, and also provision additional funds
            // to existing EOAs with a bootstrap account
            unimplemented!()
        }
        255 => {
            // DEBUG ONLY: Use raw keys passed directly to the stream
            let mut num = [0];
            stream.read_exact(&mut num)?;
            let mut keys = vec![0; num[0] as usize * 32];
            stream.read_exact(&mut keys)?;
            Ok(keys
                .chunks_exact(32)
                .map(|k| k.try_into().unwrap())
                .collect())
        }
        _ => bail!("Received invalid EOA mode from userspace"),
    }
}

// Unseal EOA accounts from previous enclave state
fn unseal_eoas(stream: &mut TcpStream) -> eyre::Result<Vec<[u8; 32]>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    let mut payload = vec![0; len];
    stream.read_exact(&mut payload)?;
    let decrypted = crate::sealing::unseal(Keypolicy::MRSIGNER, EOA_SEAL_KEY_LABEL, &payload)?;
    if !decrypted.len().is_multiple_of(32) {
        bail!("invalid decrypted eoa payload");
    }
    Ok(decrypted
        .chunks_exact(32)
        .map(|k| k.try_into().unwrap())
        .collect())
}
