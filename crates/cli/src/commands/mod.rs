use alloy::signers::local::PrivateKeySigner;
use clap::Subcommand;
use color_eyre::Result;

use crate::config::Config;

mod faucet;
pub mod run;

#[derive(Subcommand)]
pub enum Command {
    /// Run the node. If no keys are included, runs in read-only mode.
    Run(run::RunArgs),
    /// Use the faucet functionality on the given token contract. Requires keys.
    Faucet(faucet::FaucetArgs),
}

impl Command {
    /// Run the given command
    pub async fn execute(self, config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        match self {
            Command::Faucet(args) => args.execute(config, signers).await,
            Command::Run(args) => args.execute(config, signers).await,
        }
    }
}
