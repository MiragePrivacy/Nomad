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
  
  # Check for bc
  if ! command -v bc &> /dev/null; then
    echo "[deploy] ERROR: bc is not installed."
    ((missing_deps++))
  fi
  
  # Check for git
  if ! command -v git &> /dev/null; then
    echo "[deploy] ERROR: git is not installed."
    ((missing_deps++))
  fi
  
  # Check for npm (optional, used for some Uniswap dependencies)
  if ! command -v npm &> /dev/null; then
    debug_log "npm not found - some dependencies may not install"
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
  # Disable code size limit for large contracts like Uniswap Router
  anvil \
    --chain-id 31337 \
    --port 8545 \
    --host 0.0.0.0 \
    --accounts 10 \
    --balance 10000 \
    --gas-limit 30000000 \
    --gas-price 20000000000 \
    --code-size-limit 50000 &

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

# Distribute tokens to node keys
distribute_tokens_to_node_keys() {
  local token_contract=$1
  local deployer_key=$2

  debug_log "Distributing TUSDC tokens to node keys..."

  # Anvil's deterministic private keys (first 20 accounts for comprehensive coverage)
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
    "0xf214f2b2cd398c806f84e317254e0f0b801d0643303237d97a22a48e01628897"
    "0x701b615bbdfb9de65240bc28bd21bbc0d996645a3dd57e7b12bc2bdf6f192c82"
    "0xa267530f49f8280200edf313ee7af6b827f2a8bce2897751d06a843f644967b1"
    "0x47c99abed3324a2707c28affff1267e45918ec8c3f20b8aa892e8b065d2942dd"
    "0xc526ee95bf44d8fc405a158bb884d9d1238d99f0612e9f33d006bb0789009aaa"
    "0x8166f546bab6da521a8369cab06c5d2b9e46670292d85c875ee9ec20e84ffb61"
    "0xea6c44ac03bff858b476bba40716402b03e41b8e97e276d1baec7c37d42484a0"
    "0x689af8efa8c651a91ad287602527f3af2fe9f6501a7ac4b061667b5a93e037fd"
    "0xde9be858da4a475276426320d5e9262ecfc3ba460bfac56360bfa6c4c28b4ee0"
    "0xdf57089febbacf7ba0bc227dafbffa9fc08a93fdc68e1e42411a14efcf23656e"
  )

  # Corresponding addresses for logging
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
    "0xBcd4042DE499D14e55001CcbB24a551F3b954096"
    "0x71bE63f3384f5fb98995898A86B02Fb2426c5788"
    "0xFABB0ac9d68B0B445fB7357272Ff202C5651694a"
    "0x1CBd3b2770909D4e10f157cABC84C7264073C9Ec"
    "0xdF3e18d64BC6A983f673Ab319CCaE4f1a57C7097"
    "0xcd3B766CCDd6AE721141F452C550Ca635964ce71"
    "0x2546BcD3c84621e976D8185a91A922aE77ECEc30"
    "0xbDA5747bFD65F08deb54cb465eB87D40e51B197E"
    "0xdD2FD4581271e230360230F9337D5c0430Bf44C0"
    "0x8626f6940E2eb28930eFb4CeF49B2d1F2C9C1199"
  )

  debug_log "Minting and distributing tokens to ${#keys[@]} accounts..."

  # For each account, mint tokens multiple times to give them a substantial balance
  for i in "${!keys[@]}"; do
    local key="${keys[$i]}"
    local address="${addresses[$i]}"

    # Skip the deployer account (already has tokens from liquidity provision)
    if [ "$key" = "$deployer_key" ]; then
      debug_log "Skipping deployer account ($((i+1))/20): $address (already has tokens)"
      continue
    fi

    debug_log "Distributing to account ($((i+1))/20): $address"

    # Mint once (100,000 TUSDC per account)
    cast send \
      --private-key "$key" \
      --rpc-url "http://127.0.0.1:8545" \
      "$token_contract" \
      "mint()" >/dev/null 2>&1

    # Verify the balance
    local balance
    balance=$(cast call \
      --rpc-url "http://127.0.0.1:8545" \
      "$token_contract" \
      "balanceOf(address)(uint256)" \
      "$address" 2>/dev/null || echo "0")

    # Convert from 6 decimals to readable (handle large numbers)
    if [ "$balance" != "0" ] && [ -n "$balance" ]; then
      # Use awk instead of bc for better compatibility
      local balance_readable=$(echo "$balance" | awk '{printf "%.0f", $1/1000000}')
    else
      local balance_readable="0"
    fi
    debug_log "Account $address now has $balance_readable TUSDC"
  done

  debug_log "Token distribution complete - all accounts funded with 100,000 TUSDC each"
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
        uint256 amount = 100000 * 10**decimals; // Mint 100,000 TUSDC per call
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

  # Distribute tokens to node keys
  distribute_tokens_to_node_keys "$contract_address" "$deployer_key"
}

# Deploy WETH contract
deploy_weth() {
  debug_log "Downloading WETH9 contract..."

  # Create temporary directory for WETH contract
  local temp_dir=$(mktemp -d)

  # Download WETH9 bytecode from canonical source
  debug_log "Downloading WETH9 bytecode..."
  if ! curl -s -L "https://raw.githubusercontent.com/gnosis/canonical-weth/master/build/contracts/WETH9.json" -o "$temp_dir/weth9.json"; then
    echo "[deploy] ERROR: Failed to download WETH9 contract"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Extract bytecode
  local bytecode
  bytecode=$(cat "$temp_dir/weth9.json" | jq -r '.bytecode')

  if [ -z "$bytecode" ] || [ "$bytecode" = "null" ]; then
    echo "[deploy] ERROR: Could not extract WETH9 contract bytecode"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Deploy contract using first anvil account
  local deployer_key="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

  debug_log "Deploying WETH9 contract to anvil..."
  local deploy_result
  deploy_result=$(FOUNDRY_DISABLE_NIGHTLY_WARNING=1 cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    --create \
    --json \
    "$bytecode" 2>/dev/null)

  if [ $? -ne 0 ] || [ -z "$deploy_result" ]; then
    echo "[deploy] ERROR: Failed to deploy WETH9 contract"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Extract contract address
  local contract_address
  contract_address=$(echo "$deploy_result" | jq -r '.contractAddress // empty')

  if [ -z "$contract_address" ] || [ "$contract_address" = "null" ] || [ "$contract_address" = "empty" ]; then
    echo "[deploy] ERROR: Could not extract WETH9 contract address"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Cleanup temp directory
  rm -rf "$temp_dir"

  debug_log "WETH9 deployed at: $contract_address"
  export WETH_ADDRESS="$contract_address"
}

# Deploy Uniswap V2 contracts and create pool
deploy_uniswap() {
  debug_log "Downloading Uniswap V2 contracts..."

  # Create temporary directory for Uniswap contracts
  local temp_dir=$(mktemp -d)
  local original_dir=$(pwd)

  # Download and compile Uniswap V2 Core (Factory + Pair)
  debug_log "Downloading Uniswap V2 Core repository..."
  if ! git clone --depth 1 https://github.com/Uniswap/v2-core.git "$temp_dir/v2-core" >/dev/null 2>&1; then
    echo "[deploy] ERROR: Failed to download Uniswap V2 Core"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Compile Uniswap V2 Core
  debug_log "Compiling Uniswap V2 Core..."
  cd "$temp_dir/v2-core"
  if ! FOUNDRY_DISABLE_NIGHTLY_WARNING=1 forge build >/dev/null 2>&1; then
    echo "[deploy] ERROR: Failed to compile Uniswap V2 Core"
    cd "$original_dir"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Extract factory bytecode
  local factory_bytecode
  factory_bytecode=$(cat "$temp_dir/v2-core/out/UniswapV2Factory.sol/UniswapV2Factory.json" | jq -r '.bytecode.object')

  if [ -z "$factory_bytecode" ] || [ "$factory_bytecode" = "null" ]; then
    echo "[deploy] ERROR: Could not extract factory bytecode"
    cd "$original_dir"
    rm -rf "$temp_dir"
    exit 1
  fi

  local deployer_key="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

  # Deploy Factory (takes feeToSetter address as constructor parameter)
  debug_log "Deploying Test Uniswap V2 Factory..."
  local deployer_address="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
  local constructor_args
  constructor_args=$(cast abi-encode "constructor(address)" "$deployer_address")

  local factory_deploy_result
  factory_deploy_result=$(FOUNDRY_DISABLE_NIGHTLY_WARNING=1 cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    --create \
    --json \
    "${factory_bytecode}${constructor_args:2}" 2>/dev/null)

  local factory_address
  factory_address=$(echo "$factory_deploy_result" | jq -r '.contractAddress // empty')

  if [ -z "$factory_address" ] || [ "$factory_address" = "null" ]; then
    echo "[deploy] ERROR: Could not extract factory address"
    cd "$original_dir"
    rm -rf "$temp_dir"
    exit 1
  fi

  debug_log "Uniswap Factory deployed at: $factory_address"
  export FACTORY_ADDRESS="$factory_address"

  # Deploy Router with constructor args
  debug_log "Deploying Uniswap V2 Router..."
  # Download and compile Uniswap V2 Periphery (Router)
  debug_log "Downloading Uniswap V2 Periphery repository..."
  if ! git clone --depth 1 https://github.com/Uniswap/v2-periphery.git "$temp_dir/v2-periphery" >/dev/null 2>&1; then
    echo "[deploy] ERROR: Failed to download Uniswap V2 Periphery"
    cd "$original_dir"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Compile Uniswap V2 Periphery
  debug_log "Compiling Uniswap V2 Periphery..."
  cd "$temp_dir/v2-periphery"
  
  # Install dependencies first
  if [ -f "package.json" ]; then
    debug_log "Installing npm dependencies..."
    npm install >/dev/null 2>&1 || true
  fi
  
  # Try to compile, and if it fails, show the error for debugging
  if ! FOUNDRY_DISABLE_NIGHTLY_WARNING=1 forge build 2>"$temp_dir/build_error.log"; then
    debug_log "Build failed, trying with legacy compiler settings..."
    
    # Create a foundry.toml with older settings
    cat > foundry.toml << 'EOF'
[profile.default]
src = 'contracts'
out = 'out'
libs = ['node_modules', 'lib']
remappings = []
optimizer = true
optimizer_runs = 999999
via_ir = false
solc_version = "0.6.6"
EOF
    
    # Try again with explicit settings
    if ! FOUNDRY_DISABLE_NIGHTLY_WARNING=1 forge build 2>>"$temp_dir/build_error.log"; then
      echo "[deploy] ERROR: Failed to compile Uniswap V2 Periphery"
      debug_log "Build errors:"
      cat "$temp_dir/build_error.log" >&2
      cd "$original_dir"
      rm -rf "$temp_dir"
      exit 1
    fi
  fi

  # Extract router bytecode
  debug_log "Extracting router bytecode..."
  local router_bytecode
  router_bytecode=$(cat "$temp_dir/v2-periphery/out/UniswapV2Router02.sol/UniswapV2Router02.json" | jq -r '.bytecode.object')

  if [ -z "$router_bytecode" ] || [ "$router_bytecode" = "null" ] || [ "$router_bytecode" = "0x" ]; then
    echo "[deploy] ERROR: Could not extract router bytecode"
    cd "$original_dir"
    rm -rf "$temp_dir"
    exit 1
  fi

  # Encode constructor arguments (factory address, WETH address)
  local constructor_args
  constructor_args=$(cast abi-encode "constructor(address,address)" "$factory_address" "$WETH_ADDRESS")

  local router_deploy_result
  router_deploy_result=$(FOUNDRY_DISABLE_NIGHTLY_WARNING=1 cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    --create \
    --json \
    "${router_bytecode}${constructor_args:2}" 2>/dev/null)

  local router_address
  router_address=$(echo "$router_deploy_result" | jq -r '.contractAddress // empty')

  if [ -z "$router_address" ] || [ "$router_address" = "null" ]; then
    echo "[deploy] ERROR: Could not extract router address"
    rm -rf "$temp_dir"
    exit 1
  fi

  debug_log "Uniswap Router deployed at: $router_address"
  export ROUTER_ADDRESS="$router_address"

  # Create TUSDC/WETH pair
  debug_log "Creating TUSDC/WETH trading pair..."
  cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    "$factory_address" \
    "createPair(address,address)" \
    "$TUSDC_ADDRESS" \
    "$WETH_ADDRESS" >/dev/null 2>&1

  # Get the pair address
  local pair_address
  pair_address=$(cast call \
    --rpc-url "http://127.0.0.1:8545" \
    "$factory_address" \
    "getPair(address,address)(address)" \
    "$TUSDC_ADDRESS" \
    "$WETH_ADDRESS")

  debug_log "TUSDC/WETH pair created at: $pair_address"
  export PAIR_ADDRESS="$pair_address"

  # Add initial liquidity to the pair
  debug_log "Adding substantial liquidity to TUSDC/WETH pair..."

  # Mint TUSDC to deployer for liquidity (only need 1 mint call now)
  debug_log "Minting TUSDC tokens for liquidity..."
  cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    "$TUSDC_ADDRESS" \
    "mint()" >/dev/null 2>&1

  # Approve router to spend large amount of TUSDC (100,000 TUSDC)
  local tusdc_amount="100000000000"  # 100,000 TUSDC (6 decimals)
  debug_log "Approving router to spend $tusdc_amount TUSDC..."
  cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    "$TUSDC_ADDRESS" \
    "approve(address,uint256)" \
    "$router_address" \
    "$tusdc_amount" >/dev/null 2>&1

  # Add liquidity (100,000 TUSDC + 100 ETH = 1,000 TUSDC per ETH rate)
  local eth_amount="100000000000000000000"  # 100 ETH
  debug_log "Adding liquidity: 100,000 TUSDC + 100 ETH..."
  cast send \
    --private-key "$deployer_key" \
    --rpc-url "http://127.0.0.1:8545" \
    --value "$eth_amount" \
    "$router_address" \
    "addLiquidityETH(address,uint256,uint256,uint256,address,uint256)" \
    "$TUSDC_ADDRESS" \
    "$tusdc_amount" \
    "$((tusdc_amount * 95 / 100))" \
    "$((eth_amount * 95 / 100))" \
    "$deployer_address" \
    "$(($(date +%s) + 600))" >/dev/null 2>&1

  debug_log "Substantial liquidity added successfully (100K TUSDC + 100 ETH)"

  # Return to original directory and cleanup
  cd "$original_dir"
  rm -rf "$temp_dir"

  debug_log "Uniswap V2 deployment complete"
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

  # Ensure we can create .env file
  touch .env 2>/dev/null || {
    echo "[deploy] ERROR: Cannot create .env file in current directory"
    echo "[deploy] Current directory: $(pwd)"
    echo "[deploy] Permissions: $(ls -la . | head -2)"
    exit 1
  }
  
  cat > .env << EOF
# Anvil network configuration
HTTP_RPC=http://127.0.0.1:8545
TOKEN_CONTRACT=$TUSDC_ADDRESS
WETH_ADDRESS=$WETH_ADDRESS
UNISWAP_FACTORY=$FACTORY_ADDRESS
UNISWAP_ROUTER=$ROUTER_ADDRESS
TUSDC_WETH_PAIR=$PAIR_ADDRESS

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
  echo "WETH Address:  $WETH_ADDRESS"
  echo "Uniswap Factory: $FACTORY_ADDRESS"
  echo "Uniswap Router:  $ROUTER_ADDRESS"
  echo "TUSDC/WETH Pair: $PAIR_ADDRESS"
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
  deploy_weth
  deploy_uniswap
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
