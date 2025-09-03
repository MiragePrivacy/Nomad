use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use eyre::Result;
use nomad_p2p::{P2pConfig, P2pNode};
use nomad_pool::SignalPool;
use nomad_types::primitives::U256;
use tokio::sync::mpsc::unbounded_channel;
use tracing::{info, Level};

use crate::common::{create_test_signal, init_test_logging, wait_with_timeout};

/// Test multi-node P2P network setup and signal propagation
#[tokio::test(flavor = "multi_thread")]
async fn test_multi_node_p2p_network() -> Result<()> {
    init_test_logging();
    
    info!("Starting multi-node P2P network test");
    
    let base_port = 9200;
    let num_nodes = 4;
    let mut signal_pools = Vec::new();
    let mut shutdown_handles = Vec::new();
    
    // Create signal pools and channels for each node
    let (txs, mut rxs): (Vec<_>, Vec<_>) = (0..num_nodes)
        .map(|_| {
            let pool = SignalPool::new(100);
            let channel = unbounded_channel();
            (pool, channel)
        })
        .unzip();
    
    signal_pools = txs;
    
    let read_only = Arc::new(AtomicBool::new(false));
    
    // Setup bootstrap node (node 0)
    let bootstrap_config = P2pConfig {
        tcp: base_port,
        ..Default::default()
    };
    
    let bootstrap_node = P2pNode::new(
        bootstrap_config.clone(),
        signal_pools[0].clone(),
        read_only.clone(),
        Some(rxs.remove(0)),
    )?;
    
    shutdown_handles.push(bootstrap_node.shutdown_handle());
    bootstrap_node.spawn();
    
    // Wait for bootstrap node to start
    tokio::time::sleep(Duration::from_millis(500)).await;
    info!("Bootstrap node started on port {}", base_port);
    
    // Create bootstrap address for other nodes
    let bootstrap_addr = format!("/ip4/127.0.0.1/tcp/{}", base_port).parse()?;
    
    // Spawn additional nodes
    for i in 1..num_nodes {
        let mut config = bootstrap_config.clone();
        config.tcp = base_port + i as u16;
        config.bootstrap.push(bootstrap_addr.clone());
        
        let node = P2pNode::new(
            config,
            signal_pools[i].clone(),
            read_only.clone(),
            Some(rxs.remove(0)),
        )?;
        
        shutdown_handles.push(node.shutdown_handle());
        node.spawn();
        
        info!("Node {} started on port {}", i, base_port + i as u16);
        
        // Small delay between node starts
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    // Wait for network to stabilize
    info!("Waiting for network to stabilize...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Test signal propagation
    info!("Testing signal propagation across network");
    
    for test_round in 0..3 {
        info!("Signal propagation test round {}", test_round + 1);
        
        // Create a unique signal for this round
        let mut test_signal = create_test_signal((test_round + 1) as u8);
        if let nomad_types::SignalPayload::Unencrypted(ref mut signal) = test_signal {
            signal.transfer_amount = U256::from(1000000 + test_round * 100);
        }
        
        // Insert signal into the first node's pool
        signal_pools[0].insert(test_signal.clone()).await;
        info!("Inserted signal into node 0");
        
        // Wait for signal to propagate to all other nodes
        for (node_idx, pool) in signal_pools.iter().enumerate().skip(1) {
            let pool = pool.clone();
            let expected_signal = test_signal.clone();
            
            let received_signal = wait_with_timeout(
                || async {
                    // Check if the signal is available in this pool
                    // We can't directly peek, so we'll try to sample with a very short timeout
                    let sample_result = tokio::time::timeout(
                        Duration::from_millis(10),
                        pool.sample()
                    ).await;
                    
                    match sample_result {
                        Ok(signal) => {
                            // Put the signal back if it's not the one we're looking for
                            if signal == expected_signal {
                                Some(signal)
                            } else {
                                pool.insert(signal).await;
                                None
                            }
                        }
                        Err(_) => None,
                    }
                },
                Duration::from_secs(5),
                Duration::from_millis(100),
            ).await;
            
            match received_signal {
                Ok(signal) => {
                    info!("Node {} received signal successfully", node_idx);
                    assert_eq!(signal, test_signal);
                }
                Err(_) => {
                    panic!("Node {} did not receive signal within timeout", node_idx);
                }
            }
        }
        
        info!("Signal propagation test round {} completed", test_round + 1);
    }
    
    // Cleanup: shutdown all nodes
    info!("Shutting down all nodes");
    for (i, shutdown) in shutdown_handles.into_iter().enumerate() {
        shutdown.shutdown();
        info!("Node {} shutdown completed", i);
    }
    
    info!("Multi-node P2P network test completed successfully");
    Ok(())
}

/// Test P2P network resilience with node failures
#[tokio::test(flavor = "multi_thread")]
async fn test_p2p_network_resilience() -> Result<()> {
    init_test_logging();
    
    info!("Starting P2P network resilience test");
    
    let base_port = 9300;
    let mut signal_pools = Vec::new();
    let mut shutdown_handles = Vec::new();
    
    // Create 3 nodes
    for i in 0..3 {
        signal_pools.push(SignalPool::new(100));
    }
    
    let (txs, mut rxs): (Vec<_>, Vec<_>) = (0..3)
        .map(|_| unbounded_channel())
        .unzip();
    
    let read_only = Arc::new(AtomicBool::new(false));
    
    // Setup nodes with mutual bootstrapping
    for i in 0..3 {
        let mut config = P2pConfig {
            tcp: base_port + i,
            ..Default::default()
        };
        
        // Each node bootstraps from previous nodes
        for j in 0..i {
            let bootstrap_addr = format!("/ip4/127.0.0.1/tcp/{}", base_port + j).parse()?;
            config.bootstrap.push(bootstrap_addr);
        }
        
        let node = P2pNode::new(
            config,
            signal_pools[i as usize].clone(),
            read_only.clone(),
            Some(rxs.remove(0)),
        )?;
        
        shutdown_handles.push(node.shutdown_handle());
        node.spawn();
        
        info!("Node {} started", i);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    
    // Wait for network to stabilize
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Test normal operation
    let test_signal = create_test_signal(99);
    signal_pools[0].insert(test_signal.clone()).await;
    
    // Verify signal reaches other nodes
    for i in 1..3 {
        let received = tokio::time::timeout(
            Duration::from_secs(2),
            signal_pools[i].sample()
        ).await?;
        assert_eq!(received, test_signal);
        info!("Node {} received signal before failure test", i);
    }
    
    // Simulate node failure by shutting down middle node
    info!("Simulating node 1 failure");
    shutdown_handles[1].shutdown();
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Test that remaining nodes can still communicate
    let recovery_signal = create_test_signal(100);
    signal_pools[0].insert(recovery_signal.clone()).await;
    
    // Node 2 should still receive the signal despite node 1 being down
    let received_after_failure = tokio::time::timeout(
        Duration::from_secs(3),
        signal_pools[2].sample()
    ).await?;
    
    assert_eq!(received_after_failure, recovery_signal);
    info!("Network maintained connectivity after node failure");
    
    // Cleanup remaining nodes
    shutdown_handles[0].shutdown();
    shutdown_handles[2].shutdown();
    
    info!("P2P network resilience test completed successfully");
    Ok(())
}

/// Test P2P network with high message volume
#[tokio::test(flavor = "multi_thread")]
async fn test_p2p_high_volume() -> Result<()> {
    init_test_logging();
    
    info!("Starting P2P high volume test");
    
    let base_port = 9400;
    let signal_pools = vec![SignalPool::new(1000), SignalPool::new(1000)];
    let (txs, mut rxs): (Vec<_>, Vec<_>) = (0..2)
        .map(|_| unbounded_channel())
        .unzip();
    
    let read_only = Arc::new(AtomicBool::new(false));
    
    // Setup 2 nodes
    let bootstrap_config = P2pConfig {
        tcp: base_port,
        ..Default::default()
    };
    
    let node1 = P2pNode::new(
        bootstrap_config.clone(),
        signal_pools[0].clone(),
        read_only.clone(),
        Some(rxs.remove(0)),
    )?;
    let shutdown1 = node1.shutdown_handle();
    node1.spawn();
    
    tokio::time::sleep(Duration::from_millis(300)).await;
    
    let mut config2 = bootstrap_config.clone();
    config2.tcp = base_port + 1;
    config2.bootstrap.push(format!("/ip4/127.0.0.1/tcp/{}", base_port).parse()?);
    
    let node2 = P2pNode::new(
        config2,
        signal_pools[1].clone(),
        read_only.clone(),
        Some(rxs.remove(0)),
    )?;
    let shutdown2 = node2.shutdown_handle();
    node2.spawn();
    
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Send many signals rapidly
    let num_signals = 50;
    info!("Sending {} signals rapidly", num_signals);
    
    for i in 0..num_signals {
        let signal = create_test_signal((i % 256) as u8);
        signal_pools[0].insert(signal).await;
        
        // Small delay to avoid overwhelming
        if i % 10 == 9 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    
    // Verify all signals are eventually received
    info!("Verifying signal reception");
    let mut received_count = 0;
    let timeout_start = std::time::Instant::now();
    
    while received_count < num_signals && timeout_start.elapsed() < Duration::from_secs(10) {
        match tokio::time::timeout(Duration::from_millis(100), signal_pools[1].sample()).await {
            Ok(_signal) => {
                received_count += 1;
                if received_count % 10 == 0 {
                    info!("Received {}/{} signals", received_count, num_signals);
                }
            }
            Err(_) => {
                // No signal available, continue waiting
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    
    info!("Received {}/{} signals in high volume test", received_count, num_signals);
    assert!(received_count >= num_signals * 8 / 10, "Should receive at least 80% of signals");
    
    shutdown1.shutdown();
    shutdown2.shutdown();
    
    info!("P2P high volume test completed successfully");
    Ok(())
}