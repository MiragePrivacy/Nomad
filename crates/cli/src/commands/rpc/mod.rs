use std::fmt::Display;

use alloy::signers::local::PrivateKeySigner;
use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use nomad_rpc::HttpClient;
use reqwest::Url;

use nomad_node::config::Config;

mod signal;

/// RPC Client for local and remote nodes
#[derive(Parser)]
pub struct RpcArgs {
    /// RPC URL for a nomad instance. Defaults to the local node's configured rpc server.
    #[arg(short, long, global = true)]
    pub url: Option<Url>,
    #[command(subcommand)]
    pub cmd: RpcCommand,
}

impl Display for RpcArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.cmd {
            RpcCommand::Signal(_) => f.write_str("rpc_signal"),
        }
    }
}

#[derive(Subcommand)]
pub enum RpcCommand {
    /// Submit signals to the node to gossip to the network
    Signal(signal::SignalArgs),
}

impl RpcArgs {
    pub async fn execute(self, config: Config, _signers: Vec<PrivateKeySigner>) -> Result<()> {
        let client = HttpClient::builder().build(
            self.url
                .map(|v| v.to_string())
                .unwrap_or(format!("http://localhost:{}", config.rpc.port)),
        )?;
        match self.cmd {
            RpcCommand::Signal(args) => args.execute(client).await,
        }
    }
}
