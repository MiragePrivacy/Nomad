use std::{
    iter::repeat_with,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use nomad_pool::SignalPool;
use nomad_types::primitives::U256;
use tokio::sync::mpsc::unbounded_channel;
use tracing::{info, Level};

use crate::{P2pConfig, P2pNode};

macro_rules! port {
    () => {
        line!() as u16 + 9000
    };
}

#[tokio::test]
async fn start_and_stop() -> eyre::Result<()> {
    let signal_pool = SignalPool::new(100);
    let config = P2pConfig {
        tcp: port!(),
        ..Default::default()
    };
    let node = P2pNode::new(config, signal_pool, AtomicBool::new(true).into(), None)?;
    let shutdown = node.shutdown_handle();
    let handle = node.spawn();

    // do stuff

    shutdown.shutdown();
    handle.await?
}

#[tokio::test(flavor = "multi_thread")]
async fn bootstrap_and_propagate_signal() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    let signal_pools = repeat_with(|| SignalPool::new(100))
        .take(3)
        .collect::<Vec<_>>();
    let (txs, mut rxs) = repeat_with(unbounded_channel)
        .take(3)
        .unzip::<_, _, Vec<_>, Vec<_>>();

    let read_only = Arc::new(AtomicBool::new(false));

    // Setup base config and bootstrap node
    let mut base_config = P2pConfig {
        tcp: port!(),
        ..Default::default()
    };
    let node0 = P2pNode::new(
        base_config.clone(),
        signal_pools[0].clone(),
        read_only.clone(),
        Some(rxs.remove(0)),
    )?;
    let mut shutdowns = vec![node0.shutdown_handle()];

    // Add bootstrap node to base config
    base_config.bootstrap.push(
        format!("/ip4/127.0.0.1/tcp/{}", base_config.tcp)
            .parse()
            .unwrap(),
    );

    // Spawn bootstrap node and wait for it to start
    node0.spawn();
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("spawned bootstrap node");

    // Spawn 2 more nodes with bootstrap configured
    for i in 1..=2 {
        let mut config = base_config.clone();
        config.tcp += i;
        let node = P2pNode::new(
            config,
            signal_pools[i as usize].clone(),
            read_only.clone(),
            Some(rxs.remove(0)),
        )?;
        shutdowns.push(node.shutdown_handle());
        node.spawn();
        info!("spawned node {i}");
    }

    // Wait for them to all connect
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Test sending a signal to each node
    for i in 0..=2 {
        let signal = nomad_types::Signal {
            escrow_contract: [i; 20].into(),
            token_contract: [i; 20].into(),
            recipient: [i; 20].into(),
            transfer_amount: U256::from(12345678),
            reward_amount: U256::from(1234),
            acknowledgement_url: String::new(),
            selector_mapping: Default::default(),
        };

        // Send signal to p2p node to broadcast and index
        txs[i as usize].send(signal.clone()).unwrap();

        info!("Sent signal to node {i}");

        // All signal pools should have the signal eventually
        for pool in &signal_pools {
            assert_eq!(signal, pool.sample().await);
            info!("Recieved signal from node {i}");
        }
    }

    // Stop all nodes
    for shutdown in shutdowns {
        shutdown.shutdown();
        info!("Shutdown node");
    }
    Ok(())
}
