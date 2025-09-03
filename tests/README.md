# Nomad Integration Tests

This directory contains comprehensive integration tests for the Nomad node software. These tests validate the entire system's functionality across all components.

## Test Structure

```
tests/
├── integration/
│   ├── signal_processing.rs    # End-to-end signal processing flow tests
│   ├── p2p_network.rs         # P2P networking and gossip protocol tests
│   ├── ethereum_integration.rs # Ethereum client and blockchain interaction tests
│   ├── rpc_server.rs          # RPC API server functionality tests
│   ├── vm_integration.rs      # Virtual machine puzzle execution tests
│   └── cli_config.rs          # Configuration and CLI argument tests
├── common/
│   └── mod.rs                 # Shared test utilities and helpers
├── Cargo.toml                 # Test dependencies and configuration
└── lib.rs                     # Test module declarations
```

## Test Categories

### 1. Signal Processing Integration (`signal_processing.rs`)
- **End-to-end signal flow**: Tests the complete lifecycle from signal ingestion to reward collection
- **Encrypted signal handling**: Validates cryptographic operations and decryption flow
- **Load testing**: Ensures system can handle multiple signals under pressure
- **Error scenarios**: Tests resilience with malformed or invalid signals

### 2. P2P Network Integration (`p2p_network.rs`)
- **Multi-node setup**: Tests network formation with bootstrap nodes
- **Signal propagation**: Validates gossip protocol across multiple peers
- **Network resilience**: Tests behavior when nodes fail or disconnect
- **High volume messaging**: Stress tests the P2P layer with many concurrent signals

### 3. Ethereum Integration (`ethereum_integration.rs`)
- **Client initialization**: Tests connection to Ethereum networks
- **Contract validation**: Validates escrow contract interaction logic
- **Transaction flow**: Tests the complete blockchain transaction sequence
- **Error handling**: Tests insufficient balance recovery and retry mechanisms
- **Proof generation**: Validates Merkle proof creation for transfer events

### 4. RPC Server Integration (`rpc_server.rs`)
- **Server startup**: Tests RPC server initialization and responsiveness
- **Signal submission**: Validates both encrypted and unencrypted signal endpoints
- **Concurrent requests**: Load testing with multiple simultaneous clients
- **Error handling**: Tests invalid request handling and error responses

### 5. VM Integration (`vm_integration.rs`)
- **Complex puzzles**: Tests VM with sophisticated computation programs
- **Memory operations**: Validates store/load operations across memory addresses
- **Conditional branching**: Tests jump instructions and program flow control
- **Cycle limits**: Ensures resource limits are properly enforced
- **Error handling**: Tests VM behavior with invalid programs
- **Concurrent execution**: Validates VM can handle multiple simultaneous requests

### 6. CLI and Configuration (`cli_config.rs`)
- **Config file parsing**: Tests TOML configuration loading and validation
- **Default values**: Ensures proper fallbacks when config is incomplete
- **CLI argument parsing**: Validates command-line interface structure
- **Environment integration**: Tests environment variable handling
- **Configuration serialization**: Validates config round-trip consistency

## Running Tests

### Prerequisites

Some integration tests require additional services to be running:

1. **Ethereum Node** (for ethereum_integration tests):
   ```bash
   # Start Anvil (local Ethereum testnet)
   anvil --chain-id 1337 --port 8545
   ```

2. **Mock Relayer** (for encrypted signal tests):
   ```bash
   # Start a mock HTTP server on port 3000
   # Tests will skip if not available
   ```

### Running All Integration Tests

```bash
# Run all integration tests
cargo test --package nomad-integration-tests

# Run tests with detailed output
cargo test --package nomad-integration-tests -- --nocapture

# Run tests with debug logging
RUST_LOG=debug cargo test --package nomad-integration-tests -- --nocapture
```

### Running Specific Test Categories

```bash
# Signal processing tests
cargo test --package nomad-integration-tests signal_processing

# P2P network tests
cargo test --package nomad-integration-tests p2p_network

# Ethereum integration tests
cargo test --package nomad-integration-tests ethereum_integration

# RPC server tests
cargo test --package nomad-integration-tests rpc_server

# VM integration tests
cargo test --package nomad-integration-tests vm_integration

# CLI and configuration tests
cargo test --package nomad-integration-tests cli_config
```

### Running Individual Tests

```bash
# Run a specific test
cargo test --package nomad-integration-tests test_end_to_end_signal_processing_mock

# Run with specific thread count for P2P tests
cargo test --package nomad-integration-tests test_multi_node_p2p_network -- --test-threads=1
```

## Test Design Philosophy

### Comprehensive Coverage
These tests aim to validate the entire Nomad system's functionality, from individual component behavior to cross-component integration scenarios.

### Real-world Scenarios
Tests simulate realistic usage patterns and edge cases that the system might encounter in production environments.

### Graceful Degradation
Tests validate that components fail gracefully and provide meaningful error messages when external dependencies (like Ethereum nodes) are unavailable.

### Performance Awareness
Load tests ensure the system can handle reasonable throughput and concurrent usage patterns.

### Isolation and Reliability
Tests use temporary directories, random ports, and isolated resources to avoid conflicts and ensure reproducible results.

## Test Utilities

The `common` module provides shared utilities:

- **`init_test_logging()`**: Sets up structured logging for test debugging
- **`create_test_config()`**: Generates valid test configurations
- **`create_test_signal()`**: Creates sample signals for testing
- **`create_test_signers()`**: Provides test Ethereum private keys
- **`wait_with_timeout()`**: Helper for async condition waiting

## Continuous Integration

These integration tests are designed to run in CI environments with the following considerations:

- **Service Dependencies**: Tests gracefully skip when required services aren't available
- **Timeout Management**: All tests include reasonable timeouts to prevent CI hangs
- **Resource Cleanup**: Tests properly clean up spawned processes and temporary files
- **Deterministic Behavior**: Tests avoid flaky timing issues with proper synchronization

## Troubleshooting

### Common Issues

1. **Port Conflicts**: If tests fail with "address already in use", ensure no other Nomad instances are running
2. **Timeout Failures**: Increase timeouts in CI environments or when running on slower systems
3. **Ethereum Tests Skipping**: Start a local Anvil node to enable full Ethereum integration tests
4. **P2P Test Flakiness**: P2P tests may occasionally fail due to network timing; retry if needed

### Debug Tips

```bash
# Enable trace-level logging
RUST_LOG=trace cargo test --package nomad-integration-tests -- --nocapture

# Run single-threaded to avoid test interference
cargo test --package nomad-integration-tests -- --test-threads=1

# Run specific failing test with extra output
cargo test --package nomad-integration-tests test_name -- --nocapture --exact
```

## Contributing

When adding new integration tests:

1. Place tests in the appropriate category module
2. Use the common utilities for consistency
3. Include both success and failure scenarios
4. Add proper documentation for complex test scenarios
5. Ensure tests clean up resources properly
6. Consider CI/CD environment constraints