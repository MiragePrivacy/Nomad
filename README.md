# Nomad

Nomad is the node software used by Mirage to ingest, gossip, validate, and execute signals.

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
│   ├── node/       Full node implementation
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

Use the `-c/--config` flag to specify a custom configuration file:

```bash
nomad --config /path/to/config.toml run
```

#### Automated Token Swapping

Nomad supports automated token-to-ETH swapping via Uniswap V2 to maintain minimum ETH balances. This feature is **optional** and disabled by default.

**Configuration:**

```toml
[eth]
rpc = "https://eth-sepolia.public.blastapi.io"
min_eth = 0.01  # Minimum ETH balance required for accounts

# Uniswap V2 configuration for automated token swapping
[eth.uniswap]
enabled = true                    # Enable/disable token swapping
router = "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"  # Uniswap V2 router
max_slippage_percent = 5          # Maximum slippage (5%)
swap_deadline = "20m"             # Transaction deadline
target_eth_amount = 0.005         # Target ETH amount per swap
check_interval = "5m"             # How often to check balances

# Token swap configuration - tokens available for swapping
# USDC is included by default but disabled for safety
[eth.token_swaps.USDC]
address = "0xA0b86a33E6d9A77F45Ac7Be05d83c1B40c8063c5"  # Mainnet USDC
min_balance = "1000000000"        # Keep 1000 USDC (6 decimals)
enabled = true                    # Set to true to enable swapping

[eth.token_swaps.DAI]
address = "0x6b175474e89094c44da98b954eedeac495271d0f"  # Token contract
min_balance = "1000000000000000000000"  # Keep 1000 DAI (18 decimals)
enabled = true
```

**How it works:**

1. **Background Monitoring**: Periodically checks account ETH balances (configurable interval)
2. **Swap Conditions**: If an account's ETH < `min_eth`, look for swappable tokens
3. **Token Selection**: Check configured tokens with `enabled = true`
4. **Minimum Retention**: Only swap tokens above `min_balance` threshold
5. **Target Amount**: Only swap if we can get at least `target_eth_amount` ETH
6. **Slippage Protection**: Swaps include configurable slippage limits

**Network Configuration:**

For different networks, only the router address needs to be updated (WETH and factory addresses are automatically retrieved from the router on startup):

```toml
# Mainnet
[eth.uniswap]
router = "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"

# Sepolia Testnet
[eth.uniswap]
router = "0xeE567Fe1712Faf6149d80dA1E6934E354124CfE3"
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
