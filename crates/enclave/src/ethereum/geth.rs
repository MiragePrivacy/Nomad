//! # GETH RA-TLS Client
//!
//! Connects directly to a GETH Enclave and provides RPC client implementation for our EOAs
//! - Obfuscated contract write calls
//! - Balance checks (ETH, ERC20)
//! - Hidden transaction polling (buildernet wont publish until its in a block)
//! - Transaction Receipts

use eyre::Result;
use nomad_types::primitives::{Address, TxHash, U256};

pub struct GethClient {
    rpc_url: String,
    cert: String,
}

impl GethClient {
    pub fn new(rpc_url: String) -> eyre::Result<Self> {
        // TODO: connect to the rpc endpoint (with ra-tls) and cache the certificate

        Ok(Self {
            rpc_url,
            cert: "".to_string(),
        })
    }

    pub fn eth_balance_of(&self, _account: Address) -> Result<U256> {
        todo!()
    }

    pub fn erc20_balance_of(&self, _account: Address) -> Result<U256> {
        todo!()
    }

    pub fn get_block(&self) -> u64 {
        todo!()
    }

    pub fn get_transaction(&self, _hash: TxHash) -> Result<()> {
        todo!()
    }
}
