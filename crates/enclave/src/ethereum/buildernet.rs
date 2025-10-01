use eyre::Result;
use nomad_types::primitives::{Bytes, TxHash};

pub struct BuildernetClient {
    rpc_url: String,
    cert: Option<String>,
}

impl BuildernetClient {
    pub fn new(_atls_url: &str, rpc_url: String) -> eyre::Result<Self> {
        // TODO: connect to the atls endpoint and fetch the certificate

        Ok(Self {
            rpc_url,
            cert: None,
        })
    }

    pub fn send_bundle(_txs: Vec<Vec<u8>>) -> eyre::Result<()> {
        todo!()
    }

    pub fn send_raw_transaction(&self, _signed_tx: Bytes) -> Result<TxHash> {
        todo!()
    }
}
