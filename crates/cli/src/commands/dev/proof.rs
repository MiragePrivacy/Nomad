use alloy::{primitives::TxHash, providers::Provider, signers::local::PrivateKeySigner};
use clap::Parser;
use color_eyre::eyre::Result;

use nomad_ethereum::EthClient;
use nomad_node::config::Config;
use reqwest::Url;

#[derive(Parser)]
pub struct ProofArgs {
    /// Transaction hash to generate proof for
    pub tx_hash: TxHash,
    /// Optional ethereum rpc url to override with
    #[arg(short('r'), long)]
    pub eth_rpc: Option<Url>,
}

impl ProofArgs {
    pub async fn execute(self, mut config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        // Create Ethereum client
        if let Some(url) = self.eth_rpc {
            config.eth.rpc = url;
        }
        let eth_client = EthClient::new(config.eth, signers).await?;

        // Fetch the transaction receipt
        let receipt = eth_client
            .read_provider
            .get_transaction_receipt(self.tx_hash)
            .await?
            .ok_or_else(|| color_eyre::eyre::eyre!("Transaction receipt not found"))?;

        println!("Found receipt for transaction: {}", self.tx_hash);
        println!("Block number: {:?}", receipt.block_number);
        println!("Block hash: {:?}", receipt.block_hash);
        println!("Gas used: {}", receipt.gas_used);
        println!("Token contract: {:?}", receipt.to);
        println!("Executor: {:?}", receipt.from);

        // Generate proof
        let proof = eth_client.generate_proof(None, &receipt).await?;

        println!("✅ Proof generated successfully!");
        println!("Header size: {} bytes", proof.header.len());
        println!("Receipt size: {} bytes", proof.receipt.len());
        println!("Proof size: {} bytes", proof.proof.len());
        println!("Path size: {} bytes", proof.path.len());
        println!("Log index: {}", proof.log);

        // Output the proof as JSON
        println!("\nProof JSON:");
        println!("{proof}");

        Ok(())
    }
}
