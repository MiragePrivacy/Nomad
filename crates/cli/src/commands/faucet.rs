use alloy::{primitives::Address, signers::local::PrivateKeySigner};
use clap::Parser;
use color_eyre::Result;

use crate::config::Config;
use nomad_ethereum::EthClient;

#[derive(Parser)]
pub struct FaucetArgs {
    contract: Address,
}

impl FaucetArgs {
    /// Faucet tokens into each ethereum account
    pub async fn execute(self, config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        let eth_client = EthClient::new(config.eth, signers).await?;
        eth_client.faucet(self.contract).await?;
        Ok(())
    }
}
