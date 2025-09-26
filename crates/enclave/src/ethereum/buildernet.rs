pub struct BuildernetClient {
    rpc_url: String,
    cert: String,
}

impl BuildernetClient {
    pub fn new(_atls_url: &str, rpc_url: String) -> eyre::Result<Self> {
        // TODO: connect to the atls endpoint and fetch the certificate

        Ok(Self {
            rpc_url,
            cert: "".to_string(),
        })
    }

    pub fn send_bundle(_txs: Vec<Vec<u8>>) -> eyre::Result<()> {
        todo!()
    }

    pub fn send_raw_transaction(_tx: Vec<u8>) {
        todo!()
    }
}
