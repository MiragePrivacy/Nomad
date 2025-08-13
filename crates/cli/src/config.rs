use std::path::PathBuf;

use resolve_path::PathResolveExt;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::cli::Args;
use nomad_ethereum::EthConfig;
use nomad_p2p::P2pConfig;
use nomad_rpc::RpcConfig;

/// Top level config layout
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    pub eth: EthConfig,
    pub rpc: RpcConfig,
    pub p2p: P2pConfig,
}

impl Config {
    const DEFAULT_PATH: &str = "~/.config/nomad/config.toml";

    /// Load the config, filling in missing values with defaults, and writing to disk after.
    pub fn load(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or(Self::DEFAULT_PATH.into());
        let path = path.resolve().to_path_buf();

        // Read config or get the default
        let config = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    warn!("Failed to create configuration directory {parent:?}: {e}");
                }
            }
        }

        // Write config (with potentially new items)
        if let Err(e) = std::fs::write(path, toml::to_string_pretty(&config).unwrap()) {
            warn!("Failed to write config to disk: {e}");
        }

        config
    }

    /// Override config items with cli args if provided
    pub fn merge_args(mut self, args: &Args) -> Self {
        if let Some(rpc) = args.http_rpc.clone() {
            self.eth.rpc = rpc;
        }
        if let Some(port) = args.rpc_port {
            self.rpc.port = port;
        }
        if let Some(port) = args.p2p_port {
            self.p2p.tcp = port;
        }
        if let Some(peer) = args.peer.clone() {
            self.p2p.bootstrap = vec![peer.parse().unwrap()];
        }
        self
    }
}
