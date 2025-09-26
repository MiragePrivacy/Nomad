//! Enclave sealing and unsealing logic

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit};
use arrayref::array_ref;
use eyre::bail;
use sgx_isa::Keypolicy;
use sha2::{Digest, Sha256};

/// Seal some given content, outputting:
/// ```text
/// [ seal data . 12 byte nonce . payload ]
/// ```
pub fn seal(policy: Keypolicy, label: &str, data: &[u8]) -> eyre::Result<Vec<u8>> {
    let seal_data = SealData::new_from_label(policy, label);
    let key = egetkey(&seal_data)?;

    let mut nonce = [0u8; 12];
    rdrand::RdRand::new()?.try_fill_bytes(&mut nonce)?;

    // encrypt with aes-gcm
    let Ok(encrypted) = Aes256Gcm::new(&key.into()).encrypt(&nonce.into(), data) else {
        bail!("Failed to seal data");
    };

    // encode data to vec
    let mut buf = Vec::new();
    buf.extend_from_slice(&seal_data.to_vec());
    buf.extend_from_slice(&nonce);
    buf.extend_from_slice(&encrypted);
    Ok(buf)
}

/// Unseal a given sealed payload
pub fn unseal(policy: Keypolicy, label: &str, data: &[u8]) -> eyre::Result<Vec<u8>> {
    let seal_data = SealData::from_slice(policy, label, data)?;
    let key = egetkey(&seal_data)?;
    let nonce = *array_ref![data, SealData::SIZE, 12];
    let payload = &data[SealData::SIZE + 12..];
    let Ok(decrypted) = Aes256Gcm::new(&key.into()).decrypt(&nonce.into(), payload) else {
        bail!("Failed to unseal data");
    };
    Ok(decrypted)
}

pub struct SealData {
    policy: Keypolicy, // 1 byte
    keyid: [u8; 32],   // 32 bytes
    isvsvn: u16,       // 2 bytes
    cpusvn: [u8; 16],  // 16 bytes
}

impl SealData {
    const SIZE: usize = 51;

    pub fn new_from_label(policy: Keypolicy, label: &str) -> Self {
        let keyid = Sha256::digest(label).into();
        #[cfg(target_env = "sgx")]
        {
            let report = Report::for_self();
            SealData {
                policy,
                keyid,
                isvsvn: report.isvsvn,
                cpusvn: report.cpusvn,
            }
        }
        #[cfg(not(target_env = "sgx"))]
        SealData {
            policy,
            keyid,
            isvsvn: 0,
            cpusvn: [0; 16],
        }
    }

    fn to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::SIZE);
        buf.push(match self.policy {
            p if p == Keypolicy::all() => 0,
            Keypolicy::MRSIGNER => 1,
            _ => panic!("invalid policy"),
        });
        buf.extend_from_slice(&self.keyid);
        buf.extend_from_slice(&self.isvsvn.to_be_bytes());
        buf.extend_from_slice(&self.cpusvn);
        buf
    }

    fn from_slice(policy: Keypolicy, label: &str, slice: &[u8]) -> eyre::Result<Self> {
        if slice.len() < Self::SIZE {
            bail!("Invalid seal data length");
        }

        match slice[0] {
            0 if policy == Keypolicy::all() => {}
            1 if policy == Keypolicy::MRSIGNER => {}
            _ => bail!("invalid key policy"),
        };

        let keyid = *array_ref![slice, 1, 32];
        if keyid != Sha256::digest(label).as_slice() {
            bail!("Unexpected key id");
        }

        let isvsvn = u16::from_be_bytes(*array_ref![slice, 33, 2]);
        let cpusvn = *array_ref![slice, 35, 16];

        Ok(SealData {
            policy,
            keyid,
            isvsvn,
            cpusvn,
        })
    }
}

/// Derive a sealing key for the given `seal_data` configuration
#[cfg(target_env = "sgx")]
pub fn egetkey(seal_data: &SealData) -> eyre::Result<[u8; 32]> {
    use sgx_isa::{Attributes, Keyname, Keyrequest, Miscselect};
    let Ok(key) = Keyrequest {
        keyname: Keyname::Seal as _,
        keypolicy: seal_data.policy,
        isvsvn: seal_data.isvsvn,
        cpusvn: seal_data.cpusvn,
        keyid: seal_data.keyid,
        attributemask: [!0; 2],
        miscmask: !0,
        ..Default::default()
    }
    .egetkey() else {
        bail!("Failed to call egetkey");
    };
    Ok(key)
}

/// Derive a dummy sealing key on non sgx targets
pub fn egetkey(seal_data: &SealData) -> eyre::Result<[u8; 32]> {
    Ok(Sha256::digest(seal_data.keyid).into())
}
