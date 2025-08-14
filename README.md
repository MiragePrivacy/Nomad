# Nomad

Nomad is the node software used by Mirage to participate in the network, validate and execute signals.

## Signal Processing

Signals are sampled and processed one at a time from an local unordered pool by the node.
For each signal:

1. Execute the signal's puzzle program for k2
2. Contact relayer with keccak(k2) to get k1
3. Decrypt signal data with [k1, k2]
4. Validate the escrow contract bytecode against our own obfuscation
5. Validate the escrow contract is not bonded
6. Approve and bond the minimum tokens to the escrow contract
7. Execute the signal's transfer and collect the receipt
8. Build a merkle inclusion proof for the receipt's transfer log event
9. Submit proof to escrow contract and collect rewards

## Project Structure

```
Nomad/
├── crates/
│   ├── cli/        Command-line interface and main binary
│   ├── ethereum/   Ethereum integration and proof generation
│   ├── p2p/        Peer-to-peer networking
│   ├── pool/       Signal pool implementation
│   ├── rpc/        RPC server implementation
│   ├── types/      Shared type definitions
│   └── vm/         Virtual machine for puzzle execution
└── scripts/        Various scripts for testing and running nodes
```

## Installation

### Prerequisites

- Rust 1.89+ with Cargo
- OpenSSL development libraries
- pkg-config

### Building from Source

```bash
# Build in debug mode
cargo build

# Build optimized release binary
cargo build --release
```

Or install directly:

```bash
cargo install --path crates/cli
```

### Docker

Build and run using Docker:

```bash
# Build the Docker image
docker build -t nomad .

# Run the container
docker run nomad run --help
```

Images are also built and released on [ghcr.io](https://github.com/MiragePrivacy/Nomad/pkgs/container/nomad):

```bash
docker run --name nomad -i ghcr.io/mirageprivacy/nomad:latest
```

## Usage

Nomad supports two main commands:

### Run Node

Start a Nomad node to participate in the network:

```bash
# Read-only mode (no keys required)
nomad run

# Full node mode (requires 2+ Ethereum private keys)
nomad --pk <key1> --pk <key2> run
```

### Faucet

Use the Ethereum faucet functionality:

```bash
nomad --pk <key1> --pk <key2> faucet <CONTRACT>
```

### Configuration

Use the `--config` flag to specify a custom configuration file:

```bash
nomad --config /path/to/config.toml run
```

### Logging

Control logging verbosity with the `-v` flag or `RUST_LOG` environment variable:

```bash
# Info level (default)
nomad run

# Debug level
nomad -v run

# Trace level
nomad -vv run

# Custom logging via environment
RUST_LOG=nomad=debug nomad run
```
