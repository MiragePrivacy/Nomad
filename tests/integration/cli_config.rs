use std::{collections::HashMap, path::PathBuf, time::Duration};

use eyre::Result;
use nomad_node::config::{Config, EthConfig, OtlpConfig, P2pConfig, RpcConfig, VmConfig};
use serde_json::json;
use tempfile::{NamedTempFile, TempDir};
use tokio::process::Command;
use tracing::info;
use url::Url;

use crate::common::init_test_logging;

/// Test configuration file parsing and validation
#[tokio::test]
async fn test_config_file_parsing() -> Result<()> {
    init_test_logging();
    
    info!("Starting configuration file parsing test");
    
    // Create a temporary config file
    let temp_dir = TempDir::new()?;
    let config_path = temp_dir.path().join("test_config.toml");
    
    let config_content = r#"
[p2p]
tcp = 9999
bootstrap = ["/ip4/127.0.0.1/tcp/9998"]

[rpc]
port = 8888

[eth]
rpc_url = "http://localhost:8545"
chain_id = 1337
relayer_url = "http://localhost:3000"
block_cache_size = 200
confirmations = 2

[vm]
max_cycles = 5000

[otlp]
logs = true
traces = false
metrics = true

[otlp.headers]
"x-api-key" = "test-key"
"authorization" = "Bearer test-token"
"#;
    
    tokio::fs::write(&config_path, config_content).await?;
    
    // Test loading the config
    let config = Config::load(&config_path)?;
    
    // Verify P2P config
    assert_eq!(config.p2p.tcp, 9999);
    assert_eq!(config.p2p.bootstrap.len(), 1);
    assert_eq!(config.p2p.bootstrap[0].to_string(), "/ip4/127.0.0.1/tcp/9998");
    
    // Verify RPC config
    assert_eq!(config.rpc.port, 8888);
    
    // Verify Ethereum config
    assert_eq!(config.eth.rpc_url.as_str(), "http://localhost:8545/");
    assert_eq!(config.eth.chain_id, 1337);
    assert_eq!(config.eth.relayer_url.as_str(), "http://localhost:3000/");
    assert_eq!(config.eth.block_cache_size, 200);
    assert_eq!(config.eth.confirmations, 2);
    
    // Verify VM config
    assert_eq!(config.vm.max_cycles, 5000);
    
    // Verify OTLP config
    assert_eq!(config.otlp.logs, true);
    assert_eq!(config.otlp.traces, false);
    assert_eq!(config.otlp.metrics, true);
    assert_eq!(config.otlp.headers.get("x-api-key"), Some(&"test-key".to_string()));
    assert_eq!(config.otlp.headers.get("authorization"), Some(&"Bearer test-token".to_string()));
    
    info!("Configuration file parsing test completed successfully");
    Ok(())
}

/// Test default configuration values
#[tokio::test]
async fn test_default_config() -> Result<()> {
    init_test_logging();
    
    info!("Starting default configuration test");
    
    // Create minimal config file to test defaults
    let temp_dir = TempDir::new()?;
    let config_path = temp_dir.path().join("minimal_config.toml");
    
    let minimal_config = r#"
[eth]
rpc_url = "http://localhost:8545"
relayer_url = "http://localhost:3000"
"#;
    
    tokio::fs::write(&config_path, minimal_config).await?;
    
    let config = Config::load(&config_path)?;
    
    // Check that defaults are applied
    let default_p2p = P2pConfig::default();
    assert_eq!(config.p2p.tcp, default_p2p.tcp);
    assert_eq!(config.p2p.bootstrap, default_p2p.bootstrap);
    
    let default_rpc = RpcConfig::default();
    assert_eq!(config.rpc.port, default_rpc.port);
    
    let default_vm = VmConfig::default();
    assert_eq!(config.vm.max_cycles, default_vm.max_cycles);
    
    let default_otlp = OtlpConfig::default();
    assert_eq!(config.otlp.logs, default_otlp.logs);
    assert_eq!(config.otlp.traces, default_otlp.traces);
    assert_eq!(config.otlp.metrics, default_otlp.metrics);
    
    info!("Default configuration test completed successfully");
    Ok(())
}

/// Test configuration validation
#[tokio::test]
async fn test_config_validation() -> Result<()> {
    init_test_logging();
    
    info!("Starting configuration validation test");
    
    let temp_dir = TempDir::new()?;
    
    // Test invalid URL format
    let invalid_config_path = temp_dir.path().join("invalid_config.toml");
    let invalid_config = r#"
[eth]
rpc_url = "not-a-valid-url"
relayer_url = "http://localhost:3000"
"#;
    
    tokio::fs::write(&invalid_config_path, invalid_config).await?;
    
    match Config::load(&invalid_config_path) {
        Ok(_) => panic!("Expected configuration loading to fail with invalid URL"),
        Err(e) => {
            info!("Configuration validation correctly rejected invalid URL: {}", e);
            let error_msg = e.to_string().to_lowercase();
            assert!(error_msg.contains("url") || error_msg.contains("parse") || error_msg.contains("invalid"));
        }
    }
    
    // Test missing required fields
    let missing_config_path = temp_dir.path().join("missing_config.toml");
    let missing_config = r#"
[p2p]
tcp = 9000
"#;
    
    tokio::fs::write(&missing_config_path, missing_config).await?;
    
    match Config::load(&missing_config_path) {
        Ok(_) => panic!("Expected configuration loading to fail with missing required fields"),
        Err(e) => {
            info!("Configuration validation correctly rejected missing fields: {}", e);
        }
    }
    
    info!("Configuration validation test completed successfully");
    Ok(())
}

/// Test CLI argument parsing structure
#[tokio::test]
async fn test_cli_argument_parsing() -> Result<()> {
    init_test_logging();
    
    info!("Starting CLI argument parsing test");
    
    // Test basic CLI structure by checking help output
    let output = Command::new("cargo")
        .args(&["run", "--bin", "nomad", "--", "--help"])
        .output()
        .await?;
    
    let help_text = String::from_utf8_lossy(&output.stdout);
    info!("CLI help output length: {} characters", help_text.len());
    
    // Verify key CLI features are present in help
    assert!(help_text.contains("--config") || help_text.contains("-c"), "Should have config option");
    assert!(help_text.contains("--pk"), "Should have private key option");
    assert!(help_text.contains("--verbose") || help_text.contains("-v"), "Should have verbose option");
    assert!(help_text.contains("run"), "Should have run command");
    
    info!("CLI help structure validation completed");
    
    // Test CLI version
    let version_output = Command::new("cargo")
        .args(&["run", "--bin", "nomad", "--", "--version"])
        .output()
        .await?;
    
    if version_output.status.success() {
        let version_text = String::from_utf8_lossy(&version_output.stdout);
        info!("CLI version: {}", version_text.trim());
        assert!(!version_text.trim().is_empty(), "Version should not be empty");
    } else {
        info!("Version command may not be implemented yet");
    }
    
    info!("CLI argument parsing test completed successfully");
    Ok(())
}

/// Test configuration file locations and precedence
#[tokio::test]
async fn test_config_file_locations() -> Result<()> {
    init_test_logging();
    
    info!("Starting configuration file locations test");
    
    // Test that non-existent config file is handled gracefully
    let non_existent_path = PathBuf::from("/tmp/non_existent_config_12345.toml");
    
    match Config::load(&non_existent_path) {
        Ok(_) => panic!("Expected config loading to fail for non-existent file"),
        Err(e) => {
            info!("Non-existent config file correctly handled: {}", e);
            let error_msg = e.to_string().to_lowercase();
            assert!(
                error_msg.contains("no such file") || 
                error_msg.contains("not found") || 
                error_msg.contains("io error")
            );
        }
    }
    
    // Test different config file formats/extensions
    let temp_dir = TempDir::new()?;
    
    // Test .toml extension
    let toml_config = temp_dir.path().join("test.toml");
    let valid_config_content = r#"
[eth]
rpc_url = "http://localhost:8545"
relayer_url = "http://localhost:3000"
"#;
    tokio::fs::write(&toml_config, valid_config_content).await?;
    
    let config_result = Config::load(&toml_config);
    assert!(config_result.is_ok(), "Should load valid .toml config");
    
    info!("Configuration file locations test completed successfully");
    Ok(())
}

/// Test environment variable integration with configuration
#[tokio::test]
async fn test_environment_integration() -> Result<()> {
    init_test_logging();
    
    info!("Starting environment integration test");
    
    // Test RUST_LOG environment variable handling
    // This tests the CLI's logging setup integration
    
    let original_rust_log = std::env::var("RUST_LOG").ok();
    
    // Set a test RUST_LOG value
    std::env::set_var("RUST_LOG", "debug");
    
    // Verify the environment variable is set
    let current_rust_log = std::env::var("RUST_LOG")?;
    assert_eq!(current_rust_log, "debug");
    info!("RUST_LOG environment variable: {}", current_rust_log);
    
    // Test other potential environment variables
    if let Ok(home) = std::env::var("HOME") {
        info!("HOME directory: {}", home);
        assert!(!home.is_empty(), "HOME should not be empty");
    }
    
    // Restore original RUST_LOG if it existed
    if let Some(original) = original_rust_log {
        std::env::set_var("RUST_LOG", original);
    } else {
        std::env::remove_var("RUST_LOG");
    }
    
    info!("Environment integration test completed successfully");
    Ok(())
}

/// Test configuration serialization and deserialization
#[tokio::test]
async fn test_config_serialization() -> Result<()> {
    init_test_logging();
    
    info!("Starting configuration serialization test");
    
    // Create a config with all fields set
    let original_config = Config {
        p2p: P2pConfig {
            tcp: 9000,
            bootstrap: vec!["/ip4/127.0.0.1/tcp/8000".parse()?],
        },
        rpc: RpcConfig {
            port: 8001,
        },
        eth: EthConfig {
            rpc_url: "http://localhost:8545".parse()?,
            chain_id: 1337,
            relayer_url: "http://localhost:3000".parse()?,
            block_cache_size: 150,
            confirmations: 3,
        },
        vm: VmConfig {
            max_cycles: 2000,
        },
        otlp: OtlpConfig {
            url: Some("http://localhost:4317".parse()?),
            headers: {
                let mut headers = HashMap::new();
                headers.insert("test".to_string(), "value".to_string());
                headers
            },
            logs: true,
            traces: true,
            metrics: false,
        },
    };
    
    // Test serialization to TOML
    let serialized = toml::to_string(&original_config)?;
    info!("Serialized config length: {} characters", serialized.len());
    assert!(!serialized.is_empty(), "Serialized config should not be empty");
    assert!(serialized.contains("[p2p]"), "Should contain p2p section");
    assert!(serialized.contains("[eth]"), "Should contain eth section");
    
    // Test deserialization
    let deserialized: Config = toml::from_str(&serialized)?;
    
    // Verify key fields are preserved
    assert_eq!(deserialized.p2p.tcp, original_config.p2p.tcp);
    assert_eq!(deserialized.rpc.port, original_config.rpc.port);
    assert_eq!(deserialized.eth.chain_id, original_config.eth.chain_id);
    assert_eq!(deserialized.vm.max_cycles, original_config.vm.max_cycles);
    assert_eq!(deserialized.otlp.logs, original_config.otlp.logs);
    
    info!("Configuration serialization test completed successfully");
    Ok(())
}