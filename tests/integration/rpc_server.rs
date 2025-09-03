use std::time::Duration;

use eyre::Result;
use nomad_rpc::{HttpClient, MirageRpcClient, RpcConfig, SignalRequest};
use nomad_types::{primitives::U256, EncryptedSignal, Signal, SignalPayload};
use tokio::sync::mpsc::unbounded_channel;
use tracing::info;
use jsonrpsee::http_client::HttpClientBuilder;

use crate::common::{create_test_signal, init_test_logging};

/// Test RPC server startup and basic functionality
#[tokio::test]
async fn test_rpc_server_startup() -> Result<()> {
    init_test_logging();
    
    info!("Starting RPC server startup test");
    
    let config = RpcConfig { port: 8100 };
    let (signal_tx, mut signal_rx) = unbounded_channel();
    
    // Start RPC server
    nomad_rpc::spawn_rpc_server(config.clone(), signal_tx).await?;
    
    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    info!("RPC server started on port {}", config.port);
    
    // Test server is responsive by creating a client
    let client_url = format!("http://127.0.0.1:{}", config.port);
    let client = HttpClientBuilder::default()
        .build(&client_url)?;
    
    // Test sending an unencrypted signal
    let test_signal = create_test_signal(1);
    if let SignalPayload::Unencrypted(signal) = test_signal {
        let signal_request = SignalRequest::Unencrypted(signal.clone());
        
        info!("Sending signal via RPC");
        let response = client.signal(signal_request).await?;
        
        assert_eq!(response, "Signal acknowledged");
        info!("Received acknowledgment: {}", response);
        
        // Verify signal was received on the other end
        let received_signal = tokio::time::timeout(
            Duration::from_secs(1),
            signal_rx.recv()
        ).await?;
        
        if let Some(SignalPayload::Unencrypted(received)) = received_signal {
            assert_eq!(received.escrow_contract, signal.escrow_contract);
            assert_eq!(received.token_contract, signal.token_contract);
            assert_eq!(received.recipient, signal.recipient);
            info!("Signal successfully transmitted through RPC");
        } else {
            panic!("Did not receive expected signal through RPC channel");
        }
    }
    
    info!("RPC server startup test completed successfully");
    Ok(())
}

/// Test RPC server with encrypted signals
#[tokio::test]
async fn test_rpc_encrypted_signal() -> Result<()> {
    init_test_logging();
    
    info!("Starting RPC encrypted signal test");
    
    let config = RpcConfig { port: 8101 };
    let (signal_tx, mut signal_rx) = unbounded_channel();
    
    // Start RPC server
    nomad_rpc::spawn_rpc_server(config.clone(), signal_tx).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let client_url = format!("http://127.0.0.1:{}", config.port);
    let client = HttpClientBuilder::default()
        .build(&client_url)?;
    
    // Create a mock encrypted signal
    let encrypted_signal = EncryptedSignal {
        token_contract: [1; 20].into(),
        puzzle: vec![0x00, 0x00, 0x00, 0x00, 0x00, 42, 0x0A], // Set reg 0 to 42, halt
        relay: "http://localhost:3000".parse()?,
        data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16], // Mock encrypted data (12+ bytes)
    };
    
    let signal_request = SignalRequest::Encrypted(encrypted_signal.clone());
    
    info!("Sending encrypted signal via RPC");
    let response = client.signal(signal_request).await?;
    
    assert_eq!(response, "Signal acknowledged");
    info!("Received acknowledgment for encrypted signal");
    
    // Verify encrypted signal was received
    let received_signal = tokio::time::timeout(
        Duration::from_secs(1),
        signal_rx.recv()
    ).await?;
    
    if let Some(SignalPayload::Encrypted(received)) = received_signal {
        assert_eq!(received.token_contract, encrypted_signal.token_contract);
        assert_eq!(received.puzzle, encrypted_signal.puzzle);
        assert_eq!(received.relay, encrypted_signal.relay);
        assert_eq!(received.data, encrypted_signal.data);
        info!("Encrypted signal successfully transmitted through RPC");
    } else {
        panic!("Did not receive expected encrypted signal through RPC channel");
    }
    
    info!("RPC encrypted signal test completed successfully");
    Ok(())
}

/// Test RPC server under load with multiple concurrent requests
#[tokio::test]
async fn test_rpc_server_load() -> Result<()> {
    init_test_logging();
    
    info!("Starting RPC server load test");
    
    let config = RpcConfig { port: 8102 };
    let (signal_tx, mut signal_rx) = unbounded_channel();
    
    // Start RPC server
    nomad_rpc::spawn_rpc_server(config.clone(), signal_tx).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let client_url = format!("http://127.0.0.1:{}", config.port);
    let num_requests = 10;
    let mut handles = Vec::new();
    
    // Send multiple concurrent requests
    for i in 0..num_requests {
        let client_url = client_url.clone();
        let handle = tokio::spawn(async move {
            let client = HttpClientBuilder::default()
                .build(&client_url)
                .expect("Failed to create client");
            
            let signal = Signal {
                escrow_contract: [i as u8; 20].into(),
                token_contract: [i as u8; 20].into(),
                recipient: [(i + 100) as u8; 20].into(),
                transfer_amount: U256::from(1000000 + i),
                reward_amount: U256::from(10000 + i),
                acknowledgement_url: format!("http://test.com/{}", i),
                selector_mapping: Default::default(),
            };
            
            let signal_request = SignalRequest::Unencrypted(signal);
            client.signal(signal_request).await
        });
        
        handles.push(handle);
    }
    
    info!("Sent {} concurrent requests", num_requests);
    
    // Wait for all requests to complete
    let mut success_count = 0;
    for handle in handles {
        match handle.await? {
            Ok(response) => {
                assert_eq!(response, "Signal acknowledged");
                success_count += 1;
            }
            Err(e) => {
                panic!("Request failed: {}", e);
            }
        }
    }
    
    assert_eq!(success_count, num_requests);
    info!("All {} requests completed successfully", success_count);
    
    // Verify all signals were received
    let mut received_count = 0;
    while received_count < num_requests {
        match tokio::time::timeout(Duration::from_secs(1), signal_rx.recv()).await {
            Ok(Some(_signal)) => {
                received_count += 1;
            }
            Ok(None) => {
                panic!("Channel closed unexpectedly");
            }
            Err(_) => {
                panic!("Timeout waiting for signal {}", received_count + 1);
            }
        }
    }
    
    assert_eq!(received_count, num_requests);
    info!("All {} signals received through RPC channel", received_count);
    
    info!("RPC server load test completed successfully");
    Ok(())
}

/// Test RPC server error handling
#[tokio::test]
async fn test_rpc_server_error_handling() -> Result<()> {
    init_test_logging();
    
    info!("Starting RPC server error handling test");
    
    let config = RpcConfig { port: 8103 };
    let (signal_tx, _signal_rx) = unbounded_channel();
    
    // Start RPC server
    nomad_rpc::spawn_rpc_server(config.clone(), signal_tx.clone()).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let client_url = format!("http://127.0.0.1:{}", config.port);
    let client = HttpClientBuilder::default()
        .build(&client_url)?;
    
    // Test normal operation first
    let valid_signal = Signal {
        escrow_contract: [1; 20].into(),
        token_contract: [2; 20].into(),
        recipient: [3; 20].into(),
        transfer_amount: U256::from(1000000),
        reward_amount: U256::from(10000),
        acknowledgement_url: "http://test.com".to_string(),
        selector_mapping: Default::default(),
    };
    
    let valid_request = SignalRequest::Unencrypted(valid_signal);
    let response = client.signal(valid_request).await?;
    assert_eq!(response, "Signal acknowledged");
    info!("Normal operation confirmed");
    
    // Close the channel to simulate internal error
    drop(signal_tx);
    
    // Now try sending another signal - should fail
    let signal_after_close = Signal {
        escrow_contract: [4; 20].into(),
        token_contract: [5; 20].into(),
        recipient: [6; 20].into(),
        transfer_amount: U256::from(2000000),
        reward_amount: U256::from(20000),
        acknowledgement_url: "http://test2.com".to_string(),
        selector_mapping: Default::default(),
    };
    
    let request_after_close = SignalRequest::Unencrypted(signal_after_close);
    
    // This should fail because the channel is closed
    match client.signal(request_after_close).await {
        Ok(_) => {
            panic!("Expected RPC call to fail after channel close");
        }
        Err(e) => {
            info!("Expected error occurred: {}", e);
            // The error should be a JSON-RPC error about failing to broadcast
            let error_msg = e.to_string();
            assert!(error_msg.contains("500") || error_msg.contains("broadcast") || error_msg.contains("Internal"));
        }
    }
    
    info!("RPC server error handling test completed successfully");
    Ok(())
}

/// Test RPC server with invalid JSON requests
#[tokio::test]
async fn test_rpc_invalid_requests() -> Result<()> {
    init_test_logging();
    
    info!("Starting RPC invalid requests test");
    
    let config = RpcConfig { port: 8104 };
    let (signal_tx, _signal_rx) = unbounded_channel();
    
    // Start RPC server
    nomad_rpc::spawn_rpc_server(config.clone(), signal_tx).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let server_url = format!("http://127.0.0.1:{}", config.port);
    
    // Test with raw HTTP client to send invalid JSON
    let http_client = reqwest::Client::new();
    
    // Test 1: Invalid JSON
    info!("Testing invalid JSON");
    let invalid_json_response = http_client
        .post(&server_url)
        .header("Content-Type", "application/json")
        .body("{invalid json}")
        .send()
        .await?;
    
    // Should return error status
    assert!(!invalid_json_response.status().is_success());
    info!("Invalid JSON correctly rejected");
    
    // Test 2: Valid JSON but invalid RPC format
    info!("Testing invalid RPC format");
    let invalid_rpc_response = http_client
        .post(&server_url)
        .header("Content-Type", "application/json")
        .body(r#"{"not": "a valid rpc request"}"#)
        .send()
        .await?;
    
    // Should return error status
    assert!(!invalid_rpc_response.status().is_success());
    info!("Invalid RPC format correctly rejected");
    
    // Test 3: Valid RPC but invalid method
    info!("Testing invalid method");
    let invalid_method_response = http_client
        .post(&server_url)
        .header("Content-Type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"invalid_method","params":[]}"#)
        .send()
        .await?;
    
    // Should return JSON-RPC error response
    let response_text = invalid_method_response.text().await?;
    assert!(response_text.contains("error") || response_text.contains("Method not found"));
    info!("Invalid method correctly rejected");
    
    info!("RPC invalid requests test completed successfully");
    Ok(())
}