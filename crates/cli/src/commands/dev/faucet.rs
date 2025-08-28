use alloy::primitives::Address;
use clap::Parser;
use color_eyre::Result;

use nomad_ethereum::EthClient;

#[derive(Parser)]
pub struct FaucetArgs {
    contract: Address,
}

impl FaucetArgs {
    /// Faucet tokens into each ethereum account
    pub async fn execute(self, eth_client: EthClient) -> Result<()> {
        let provider = eth_client.wallet_provider().await?;
        eth_client.faucet(provider, self.contract).await?;
        Ok(())
    }
}
