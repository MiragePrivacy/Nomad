#![allow(unused)]

use eyre::Result;
use nomad_types::{
    primitives::{Address, TxHash, U256},
    Signal,
};

mod buildernet;
mod geth;

/// High level attested ethereum client
pub struct EthClient {
    keys: Vec<[u8; 32]>,
    accounts: Vec<Address>,
    bn: buildernet::BuildernetClient,
    geth: geth::GethClient,
}

impl EthClient {
    pub fn new(
        keys: Vec<[u8; 32]>,
        bn_atls_url: &str,
        bn_rpc_url: String,
        geth_url: String,
    ) -> Result<Self> {
        Ok(Self {
            keys,
            // TODO: derive pks
            accounts: vec![],
            bn: buildernet::BuildernetClient::new(bn_atls_url, bn_rpc_url)?,
            geth: geth::GethClient::new(geth_url)?,
        })
    }

    /// Select a pair of accounts to execute a signal with
    pub fn select_accounts(&self, signal: &Signal) -> Result<[usize; 2]> {
        todo!()
    }

    /// Bond to a signal with a given eoa
    pub fn bond(&self, eoa_1: usize, signal: &Signal) -> Result<[TxHash; 2]> {
        todo!()
    }

    /// Execute the transfer for a signal using a given eoa
    pub fn transfer(&self, eoa_2: usize, signal: &Signal) -> Result<TxHash> {
        todo!()
    }

    /// Collect a reward for a signal with a given eoa
    pub fn collect(&self, eoa_1: usize, signal: &Signal, transfer_tx: TxHash) -> Result<TxHash> {
        todo!()
    }

    /// Try to swap for some eth, ensuring we dont spend more than a minimum amount of tokens
    pub fn try_swap(
        &self,
        eoa: usize,
        token: Address,
        target_eth: U256,
        min_tokens: U256,
    ) -> Result<()> {
        todo!()
    }

    /// Try to find a suitable redistribution and emit a new signal for transfer
    pub fn redistribute(&self, _target: usize, _token: Address, _min_tokens: U256) -> Result<()> {
        todo!()
    }
}
