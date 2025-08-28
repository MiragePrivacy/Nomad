use std::fmt::Display;

use alloy::signers::local::PrivateKeySigner;
use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;

use nomad_node::config::Config;

mod faucet;
mod proof;

/// RPC Client for local and remote nodes
#[derive(Parser)]
pub struct DevArgs {
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
    /// Submit signals to the node to gossip to the network
    Faucet(faucet::FaucetArgs),
    /// Generate proof for a transaction
    Proof(proof::ProofArgs),
}

impl DevArgs {
    pub async fn execute(self, config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        match self.cmd {
            DevCommand::Faucet(args) => args.execute(config, signers).await,
            DevCommand::Proof(args) => args.execute(config, signers).await,
        }
    }
}
