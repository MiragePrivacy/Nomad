use std::fmt::Display;

use alloy::signers::local::PrivateKeySigner;
use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;

use nomad_ethereum::EthClient;
use nomad_node::config::Config;
use reqwest::Url;

mod faucet;
mod proof;

/// RPC Client for local and remote nodes
#[derive(Parser)]
pub struct DevArgs {
    /// Optional ethereum rpc url to override with
    #[arg(short('r'), long, global(true))]
    pub eth_rpc: Option<Url>,
    #[command(subcommand)]
    pub cmd: DevCommand,
}

impl Display for DevArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.cmd {
            DevCommand::Faucet(_) => f.write_str("dev_faucet"),
            DevCommand::Proof(_) => f.write_str("dev_proof"),
        }
    }
}

#[derive(Subcommand)]
pub enum DevCommand {
    /// Call faucet method on a token contract for each given account
    Faucet(faucet::FaucetArgs),
    /// Generate proof for a transaction
    Proof(proof::ProofArgs),
}

impl DevArgs {
    pub async fn execute(self, mut config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        if let Some(rpc) = self.eth_rpc {
            config.eth.rpc = rpc;
        }
        let client = EthClient::new(config.eth, signers).await?;
        match self.cmd {
            DevCommand::Faucet(args) => args.execute(client).await,
            DevCommand::Proof(args) => args.execute(client).await,
        }
    }
}
