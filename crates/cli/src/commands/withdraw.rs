use alloy::{
    primitives::{Address, U256},
    signers::local::PrivateKeySigner,
};
use clap::Parser;
use color_eyre::eyre::{bail, Result};

use nomad_ethereum::{contracts::IERC20, EthClient};
use nomad_node::config::Config;

#[derive(Parser)]
pub struct WithdrawArgs {
    /// Destination address to send tokens to
    #[arg(long)]
    pub to: Address,
    /// Token contract address
    #[arg(short = 't', long)]
    pub token_contract: Address,
    /// Amount of tokens to transfer (in wei/smallest unit)
    #[arg(short, long)]
    pub amount: u64,
}

impl WithdrawArgs {
    pub async fn execute(self, config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        // TODO: create a signal and broadcast it in the future for obfuscated withdrawl

        if signers.is_empty() {
            bail!("No signers provided. Use --pk to specify private keys.");
        }

        // Create ethereum client with all signers
        let eth_client = EthClient::new(config.eth, signers.clone()).await?;
        let provider = eth_client.wallet_provider().await?;

        // Create token contract instance
        let token = IERC20::new(self.token_contract, &provider);

        // Find a signer with sufficient balance
        let mut selected_signer = None;
        for signer in &signers {
            let from_address = signer.address();
            let balance = token.balanceOf(from_address).call().await?;

            if balance >= U256::from(self.amount) {
                selected_signer = Some(from_address);
                break;
            }
        }

        let from_address = selected_signer.ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "No signer has sufficient balance. Required: {}, checked {} addresses",
                self.amount,
                signers.len()
            )
        })?;

        // Execute the transfer
        println!(
            "Transferring {} tokens from {} to {} using contract {}",
            self.amount, from_address, self.to, self.token_contract
        );

        let tx = token
            .transfer(self.to, U256::from(self.amount))
            .from(from_address)
            .send()
            .await?;

        let receipt = tx.get_receipt().await?;

        println!("Transfer successful!");
        println!("Transaction hash: {}", receipt.transaction_hash);
        println!("Block number: {}", receipt.block_number.unwrap_or_default());
        println!("Gas used: {}", receipt.gas_used);

        Ok(())
    }
}
