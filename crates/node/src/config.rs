use std::path::PathBuf;

use eyre::{bail, Result};
use resolve_path::PathResolveExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use nomad_api::ApiConfig;
use nomad_ethereum::EthConfig;
use nomad_p2p::P2pConfig;

/// Top level config layout
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Config {
    pub p2p: P2pConfig,
    pub api: ApiConfig,
    pub vm: VmConfig,
    pub eth: EthConfig,
    pub otlp: OtlpConfig,
    pub enclave: crate::enclave::EnclaveConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct VmConfig {
    pub max_cycles: usize,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            max_cycles: 1024 * 1024,
        }
    }
}

/// Opentelemetry config, default with everything turned off
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
#[derive(Default)]
pub struct OtlpConfig {
    pub logs: bool,
    pub traces: bool,
    pub metrics: bool,
}

impl Config {
    /// Load the config, filling in missing values with defaults, and writing to disk after.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into().resolve().to_path_buf();
        debug!(config_path = ?path);

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
