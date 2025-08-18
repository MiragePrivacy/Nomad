#!/usr/bin/env bash
# set -euo pipefail

# Error trapping
trap 'echo "[deploy] ERROR: Script failed at line $LINENO. Exit code: $?" >&2' ERR

# Debug function with timestamp
debug_log() {
  echo "[deploy] $(date '+%H:%M:%S') $*" >&2
}

# Check dependencies
check_dependencies() {
  debug_log "Checking dependencies..."
  
  local missing_deps=0
  
  # Check for anvil
  if ! command -v anvil &> /dev/null; then
    echo "[deploy] ERROR: anvil (foundry) is not installed."
    ((missing_deps++))
  fi
  
  # Check for forge
  if ! command -v forge &> /dev/null; then
    echo "[deploy] ERROR: forge (foundry) is not installed."
    ((missing_deps++))
  fi
  
  # Check for cast
  if ! command -v cast &> /dev/null; then
    echo "[deploy] ERROR: cast (foundry) is not installed."
    ((missing_deps++))
  fi
  
  # Check for jq
  if ! command -v jq &> /dev/null; then
    echo "[deploy] ERROR: jq is not installed."
    ((missing_deps++))
  fi
  
  if [ $missing_deps -gt 0 ]; then
    echo "[deploy] ERROR: $missing_deps missing dependencies. Exiting."
    exit 1
  fi
}

# Kill any existing anvil processes
cleanup_existing_anvil() {
  debug_log "Cleaning up existing anvil processes..."
  # pkill -f anvil || true
  sleep 1
}

# Start anvil in background
start_anvil() {
  debug_log "Starting anvil network..."
  
  # Start anvil with deterministic accounts and chain ID
  anvil \
    --chain-id 31337 \
    --port 8545 \
    --host 0.0.0.0 \
    --accounts 10 \
    --balance 10000 \
    --gas-limit 30000000 \
    --gas-price 20000000000 &
  
  ANVIL_PID=$!
  debug_log "Anvil started with PID: $ANVIL_PID"
  
  # Wait for anvil to be ready
  debug_log "Waiting for anvil to be ready..."
  local max_attempts=30
  local attempt=0
  
  while [ $attempt -lt $max_attempts ]; do
    debug_log "attempting"
    if cast chain-id --rpc-url "http://127.0.0.1:8545" > /dev/null 2>&1; then
      debug_log "Anvil is ready!"
      break
    fi
    
    sleep 1
    ((attempt++))
  done
  
  if [ $attempt -eq $max_attempts ]; then
    echo "[deploy] ERROR: Anvil failed to start within 30 seconds"
    kill $ANVIL_PID || true
    exit 1
  fi
}

# Deploy TUSDC contract
deploy_tusdc() {
  debug_log "Deploying TUSDC contract..."
  
  # TUSDC contract source code (ERC20-like with mint function)
  local tusdc_contract='// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract TUSDC {
    string public name = "Test USD Coin";
    string public symbol = "TUSDC";
    uint8 public decimals = 6;
    uint256 public totalSupply;
    
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    
    function mint() external {
        uint256 amount = 1000 * 10**decimals; // Mint 1000 TUSDC
        balanceOf[msg.sender] += amount;
        totalSupply += amount;
        emit Transfer(address(0), msg.sender, amount);
    }
    
    function transfer(address to, uint256 value) external returns (bool) {
        require(balanceOf[msg.sender] >= value, "Insufficient balance");
        balanceOf[msg.sender] -= value;
        balanceOf[to] += value;
        emit Transfer(msg.sender, to, value);
        return true;
    }
    
    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        emit Approval(msg.sender, spender, value);
        return true;
    }
    
    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        require(allowance[from][msg.sender] >= value, "Insufficient allowance");
        require(balanceOf[from] >= value, "Insufficient balance");
        
        allowance[from][msg.sender] -= value;
        balanceOf[from] -= value;
        balanceOf[to] += value;
        
        emit Transfer(from, to, value);
        return true;
    }
}'
  
  # Create temporary directory for contract
  local temp_dir=$(mktemp -d)
  echo "$tusdc_contract" > "$temp_dir/TUSDC.sol"
  
  # Create foundry.toml
  cat > "$temp_dir/foundry.toml" << 'EOF'
[profile.default]
src = "."
out = "out"
libs = ["lib"]
optimizer = true
optimizer_runs = 200
EOF
  
  # Compile contract
  debug_log "Compiling TUSDC contract..."
  if ! FOUNDRY_DISABLE_NIGHTLY_WARNING=1 forge build --root "$temp_dir" --contracts "$temp_dir" --out "$temp_dir/out" >/dev/null 2>&1; then
    echo "[deploy] ERROR: Failed to compile TUSDC contract"
    rm -rf "$temp_dir"
    exit 1
  fi
  
  # Extract bytecode
  local bytecode
  bytecode=$(cat "$temp_dir/out/TUSDC.sol/TUSDC.json" | jq -r '.bytecode.object')
  
  if [ -z "$bytecode" ] || [ "$bytecode" = "null" ]; then
    echo "[deploy] ERROR: Could not extract contract bytecode"
    rm -rf "$temp_dir"
    exit 1
  fi
  
  # Deploy contract using first anvil account
  local deployer_key="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
  
  debug_log "Deploying contract to anvil..."
  local deploy_result
  deploy_result=$(FOUNDRY_DISABLE_NIGHTLY_WARNING=1 cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    --create \
    --json \
    "$bytecode" 2>/dev/null)
  
  if [ $? -ne 0 ] || [ -z "$deploy_result" ]; then
    echo "[deploy] ERROR: Failed to deploy TUSDC contract"
    rm -rf "$temp_dir"
    exit 1
  fi
  
  # Extract contract address
  local contract_address
  contract_address=$(echo "$deploy_result" | jq -r '.contractAddress // empty')
  
  if [ -z "$contract_address" ] || [ "$contract_address" = "null" ] || [ "$contract_address" = "empty" ]; then
    echo "[deploy] ERROR: Could not extract contract address"
    rm -rf "$temp_dir"
    exit 1
  fi
  
  # Cleanup temp directory
  rm -rf "$temp_dir"
  
  debug_log "TUSDC deployed at: $contract_address"
  export TUSDC_ADDRESS="$contract_address"
}

# Generate .env file with deployment info
generate_env_file() {
  debug_log "Generating .env file..."
  
  # Anvil's deterministic private keys (first 10 accounts)
  local keys=(
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
    "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6"
    "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a"
    "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba"
    "0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e"
    "0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356"
    "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97"
    "0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6"
  )
  
  # Anvil's deterministic addresses (first 10 accounts)  
  local addresses=(
    "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
    "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
    "0x90F79bf6EB2c4f870365E785982E1f101E93b906"
    "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65"
    "0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc"
    "0x976EA74026E726554dB657fA54763abd0C3a0aa9"
    "0x14dC79964da2C08b23698B3D3cc7Ca32193d9955"
    "0x23618e81E3f5cdF7f54C3d65f7FBc0aBf5B21E8f"
    "0xa0Ee7A142d267C1f36714E4a8F75612F20a79720"
  )
  
  cat > .env << EOF
# Anvil network configuration
HTTP_RPC=http://127.0.0.1:8545
TOKEN_CONTRACT=$TUSDC_ADDRESS

# Node private keys (for nomad nodes)
EOF

  for i in {1..8}; do
    echo "KEY_$i=${keys[$((i-1))]}" >> .env
  done
  
  cat >> .env << EOF

# Sender private keys (for escrow deployment)
EOF

  for i in {1..5}; do
    echo "SENDER_KEY_$i=${keys[$((i+2))]}" >> .env
  done
  
  debug_log ".env file created with network and contract information"
}

# Display deployment summary
show_summary() {
  echo
  echo "════════════════════════════════════════════════════════════════════════════════"
  echo "                                DEPLOYMENT SUMMARY"
  echo "════════════════════════════════════════════════════════════════════════════════"
  echo
  echo "Network:       Anvil (localhost:8545)"
  echo "Chain ID:      31337"  
  echo "TUSDC Address: $TUSDC_ADDRESS"
  echo
  echo "Available Accounts (with 10,000 ETH each):"
  
  # Show first few accounts
  local addresses=(
    "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
    "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
    "0x90F79bf6EB2c4f870365E785982E1f101E93b906"
    "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65"
  )
  
  for i in {0..4}; do
    echo "  [$((i+1))] ${addresses[$i]}"
  done
  echo "  ... and 5 more accounts"
  echo
  echo "Configuration saved to: .env"
  echo 
  echo "To use with nomad:"
  echo "  ./scripts/run_nodes.sh"
  echo
  echo "To mint TUSDC tokens:"
  echo "  cast send --private-key <private_key> --rpc-url http://127.0.0.1:8545 \\"
  echo "    $TUSDC_ADDRESS \"mint()\""
  echo
  echo "To stop anvil:"
  echo "  kill $ANVIL_PID"
  echo
  echo "════════════════════════════════════════════════════════════════════════════════"
}

# Cleanup function for script exit
cleanup_on_exit() {
  if [ -n "${ANVIL_PID:-}" ]; then
    debug_log "Cleaning up anvil process..."
    kill $ANVIL_PID 2>/dev/null || true
  fi
}

# Main execution
main() {
  echo "[deploy] Starting anvil deployment script..."
  
  check_dependencies
  cleanup_existing_anvil
  start_anvil
  deploy_tusdc
  generate_env_file
  show_summary
  
  # Keep anvil running
  debug_log "Anvil is running in the background (PID: $ANVIL_PID)"
  debug_log "Press Ctrl+C to stop anvil and exit"
  
  # Set up cleanup trap
  trap cleanup_on_exit INT TERM EXIT
  
  # Wait for anvil process
  wait $ANVIL_PID
}

main "$@"
