use std::fmt::Display;

use alloy::signers::local::PrivateKeySigner;
use clap::Subcommand;
use color_eyre::Result;

use crate::config::Config;

mod faucet;
mod rpc;
pub mod run;

#[derive(Subcommand)]
pub enum Command {
    /// Run the node. If no keys are included, runs in read-only mode.
    Run(Box<run::RunArgs>),
    /// Call RPC methods on a local or remote node.
    Rpc(Box<rpc::RpcArgs>),
    /// Use the faucet functionality on the given token contract. Requires keys.
    Faucet(Box<faucet::FaucetArgs>),
}

impl Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::Run(_) => f.write_str("run"),
            Command::Rpc(args) => {
                f.write_str("rpc_")?;
                match args.cmd {
                    rpc::RpcCommand::Signal(_) => f.write_str("signal")?,
                }
                Ok(())
            }
            Command::Faucet(_) => f.write_str("faucet"),
        }
    }
}

impl Command {
    /// Run the given command
    pub async fn execute(self, config: Config, signers: Vec<PrivateKeySigner>) -> Result<()> {
        match self {
            Command::Run(args) => args.execute(config, signers).await,
            Command::Rpc(args) => args.execute(config, signers).await,
            Command::Faucet(args) => args.execute(config, signers).await,
        }
    }
}
