use alloy::{primitives::Address, signers::local::PrivateKeySigner};
use clap::Parser;
use color_eyre::Result;

use nomad_ethereum::EthClient;
use nomad_node::config::Config;

#[derive(Parser)]
pub struct FaucetArgs {
    contract: Address,
}

impl FaucetArgs {
    /// Faucet tokens into each ethereum account
    pub async fn execute(self, config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        let eth_client = EthClient::new(config.eth, signers).await?;
        let provider = eth_client.wallet_provider().await?;
        eth_client.faucet(provider, self.contract).await?;
        Ok(())
    }
}
