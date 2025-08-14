use std::path::PathBuf;

use alloy::signers::local::PrivateKeySigner;
use clap::{ArgAction, Parser};
use color_eyre::eyre::{bail, Context, Result};
use tracing::info;

use crate::config::Config;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Args {
    /// Path to config file
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// Increases the level of verbosity (the max level is -vvv).
    #[arg(short, global = true, action = ArgAction::Count)]
    pub verbose: u8,
    /// Ethereum private keys to use. Runs in read only mode if not provided.
    /// Requires at least 2 keys to run node
    #[arg(long, action(ArgAction::Append))]
    pub pk: Option<Vec<String>>,

    /// Use the faucet functionality on the given token contract. For testing mode.
    #[arg(long)]
    pub faucet: Option<String>,

    /* Config overrides */
    /// Port for the RPC server
    #[arg(short, long)]
    pub rpc_port: Option<u16>,
    /// Port for the p2p node
    #[arg(short, long)]
    pub p2p_port: Option<u16>,
    /// Multiaddr of a peer to connect to
    #[arg(long)]
    pub peer: Option<String>,
    /// HTTP RPC URL for sending transactions
    #[arg(long)]
    pub http_rpc: Option<String>,
}

impl Args {
    /// Load config and apply overrides from arguments
    pub fn load_config(&self) -> Result<Config> {
        let mut config = Config::load(self.config.as_ref())?;
        if let Some(rpc) = self.http_rpc.clone() {
            config.eth.rpc = rpc;
        }
        if let Some(port) = self.rpc_port {
            config.rpc.port = port;
        }
        if let Some(port) = self.p2p_port {
            config.p2p.tcp = port;
        }
        if let Some(peer) = self.peer.clone() {
            config.p2p.bootstrap = vec![peer.parse().unwrap()];
        }
        Ok(config)
    }

    /// Build list of signers from the cli arguments
    pub fn build_signers(&self) -> Result<Vec<PrivateKeySigner>> {
        let Some(accounts) = &self.pk else {
            return Ok(vec![]);
        };
        if accounts.len() < 2 {
            bail!("At least 2 ethereum keys are required");
        }
        accounts
            .iter()
            .map(|s| {
                s.parse::<PrivateKeySigner>()
                    .inspect(|v| {
                        info!("Using Ethereum Account: {}", v.address());
                    })
                    .with_context(|| format!("failed to parse key: {s}"))
            })
            .collect()
    }
}
