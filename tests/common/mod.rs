use std::{collections::HashMap, sync::Arc, time::Duration};

use alloy::signers::local::PrivateKeySigner;
use nomad_node::config::{Config, EthConfig, P2pConfig, RpcConfig, VmConfig, OtlpConfig};
use nomad_types::{primitives::U256, Signal, SignalPayload};
use tempfile::TempDir;
use tracing::Level;
use url::Url;

pub fn init_test_logging() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_test_writer()
        .try_init();
}

pub fn create_test_config(base_port: u16, temp_dir: &TempDir) -> Config {
    Config {
        p2p: P2pConfig {
            tcp: base_port,
            bootstrap: vec![],
            ..Default::default()
        },
        rpc: RpcConfig {
            port: base_port + 1000,
            ..Default::default()
        },
        eth: EthConfig {
            rpc_url: "http://localhost:8545".parse().unwrap(),
            chain_id: 1337,
            relayer_url: "http://localhost:3000".parse().unwrap(),
            ..Default::default()
        },
        vm: VmConfig {
            max_cycles: 1000,
        },
        otlp: OtlpConfig {
            url: None,
            headers: HashMap::new(),
            logs: false,
            traces: false,
            metrics: false,
        },
    }
}

pub fn create_test_signal(id: u8) -> SignalPayload {
    SignalPayload::Unencrypted(Signal {
        escrow_contract: [id; 20].into(),
        token_contract: [id; 20].into(),
        recipient: [id; 20].into(),
        transfer_amount: U256::from(1_000_000),
        reward_amount: U256::from(10_000),
        acknowledgement_url: format!("http://test-relayer.com/{}", id),
        selector_mapping: Default::default(),
    })
}

pub fn create_test_signers() -> Vec<PrivateKeySigner> {
    vec![
        "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .unwrap(),
        "0xfedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
            .parse()
            .unwrap(),
    ]
}

pub async fn wait_with_timeout<F, Fut, T>(
    mut condition: F,
    timeout: Duration,
    check_interval: Duration,
) -> Result<T, &'static str>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let start = std::time::Instant::now();
    loop {
        if let Some(result) = condition().await {
            return Ok(result);
        }
        if start.elapsed() > timeout {
            return Err("Timeout waiting for condition");
        }
        tokio::time::sleep(check_interval).await;
    }
}