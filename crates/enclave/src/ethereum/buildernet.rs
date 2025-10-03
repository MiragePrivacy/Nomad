use std::net::SocketAddr;

use color_eyre::{eyre::Context, Result};
use nomad_types::primitives::{Bytes, TxHash};

use crate::ethereum::rpc::RpcClient;

pub struct BuildernetClient {
    rpc: RpcClient,
}

impl BuildernetClient {
    pub fn new(_atls_url: SocketAddr, addr: SocketAddr) -> Result<Self> {
        // TODO: connect to the atls endpoint and fetch the certificate
        Ok(Self {
            rpc: RpcClient::new(addr, None),
        })
    }

    pub fn send_raw_transaction(&self, signed_tx: Bytes) -> Result<TxHash> {
        let tx_hash: String = self
            .rpc
            .call("eth_sendRawTransaction", vec![signed_tx.to_string()])
            .context("Failed to send raw transaction")?;

        Ok(TxHash::from_slice(&hex::decode(
            tx_hash.trim_start_matches("0x"),
        )?))
    }
}
