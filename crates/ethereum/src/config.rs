use std::collections::HashMap;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EthConfig {
    /// Url for rpc commands
    pub rpc: Url,
    /// Minimum eth required for an account to be usable
    pub min_eth: f64,
    /// Uniswap V2 configuration
    pub uniswap: UniswapV2Config,
    /// Token swap configuration - table keyed by name
    pub token: HashMap<String, TokenConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct UniswapV2Config {
    pub enabled: bool,
    pub router: Address,
    pub max_slippage_percent: u8,
    #[serde(with = "humantime_serde")]
    pub swap_deadline: Duration,
    pub target_eth_amount: f64,
    #[serde(with = "humantime_serde")]
    pub check_interval: Duration,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TokenConfig {
    pub address: Address,
    pub min_balance: U256,
    pub swap: bool,
}

impl std::fmt::Debug for EthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hide potentially sensitive query parameters
        f.debug_struct("EthConfig")
            .field("rpc", &self.rpc.host_str().unwrap_or("missing rpc host"))
            .finish()
    }
}

impl Default for EthConfig {
    fn default() -> Self {
        // Add default USDC configuration (mainnet)
        let mut token = HashMap::new();
        token.insert(
            "USDC".to_string(),
            TokenConfig {
                address: "0xA0b86a33E6d9A77F45Ac7Be05d83c1B40c8063c5"
                    .parse()
                    .unwrap(), // Mainnet USDC
                min_balance: U256::from(1_000_000_000u64), // 1000 USDC (6 decimals)
                swap: false,                               // Disabled by default for safety
            },
        );

        Self {
            rpc: "https://ethereum-rpc.publicnode.com".parse().unwrap(),
            min_eth: 0.01,
            uniswap: UniswapV2Config::default(),
            token,
        }
    }
}

impl Default for UniswapV2Config {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default for safety
            router: "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"
                .parse()
                .unwrap(), // Mainnet router
            max_slippage_percent: 5,
            swap_deadline: Duration::from_secs(20 * 60), // 20 minutes
            target_eth_amount: 0.005, // Default to swapping for 0.005 ETH at a time
            check_interval: Duration::from_secs(5 * 60), // Check every 5 minutes
        }
    }
}
