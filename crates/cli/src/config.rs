use std::path::PathBuf;

use color_eyre::{eyre::bail, Result};
use resolve_path::PathResolveExt;
use serde::{Deserialize, Serialize};

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
    pub fn load(path: Option<impl Into<PathBuf>>) -> Result<Self> {
        let path = path.map(|v| v.into()).unwrap_or(Self::DEFAULT_PATH.into());
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
                    bail!("Failed to create configuration directory {parent:?}: {e}");
                }
            }
        }

        // Write config (with potentially new items)
        if let Err(e) = std::fs::write(&path, toml::to_string_pretty(&config)?) {
            bail!("Failed to write configuration to {path:?}: {e}");
        }

        Ok(config)
    }
}
