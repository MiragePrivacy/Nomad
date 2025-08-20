#!/usr/bin/env bash
set -euo pipefail

# Script to run multiple Nomad nodes with the new CLI structure
# 
# Usage examples:
#   ./run_nodes.sh                              # Basic usage
#   ./run_nodes.sh -vv                          # With verbose logging
#   ./run_nodes.sh --config /path/to/config     # With custom config
#   ./run_nodes.sh faucet 0x1234...            # Run faucet on first node only
#
# The script now uses CLI argument overrides with the new structure:
#   nomad [global-args] run [run-args]
# Where:
#   global-args: --config, --pk, --verbose
#   run-args: --http-rpc, --rpc-port, --p2p-port, --peer, --bootstrap

# Error trapping
trap 'echo "[runner] ERROR: Script failed at line $LINENO. Exit code: $?" >&2' ERR

# Debug function with timestamp - only output if DEBUG=1
debug_log() {
  if [ "${DEBUG:-0}" = "1" ]; then
    echo "[runner] $(date '+%H:%M:%S') $*" >&2
  fi
}

# Check dependencies
check_dependencies() {
  debug_log "Checking dependencies..."
  
  local missing_deps=0
  
  # Check for jq
  if ! command -v jq &> /dev/null; then
    echo "[runner] ERROR: jq is not installed. Please install jq."
    ((missing_deps++))
  fi
  
  # Check for forge
  if ! command -v forge &> /dev/null; then
    echo "[runner] ERROR: forge (foundry) is not installed."
    ((missing_deps++))
  fi
  
  # Check for cast
  if ! command -v cast &> /dev/null; then
    echo "[runner] ERROR: cast (foundry) is not installed."
    ((missing_deps++))
  fi
  
  # Check for curl
  if ! command -v curl &> /dev/null; then
    echo "[runner] ERROR: curl is not installed."
    ((missing_deps++))
  fi
  
  if [ $missing_deps -gt 0 ]; then
    echo "[runner] ERROR: $missing_deps missing dependencies. Exiting."
    exit 1
  fi
}

# ------------------------------------------------------------
# 1. Parse arguments and get user input
# ------------------------------------------------------------
# All arguments are passed as extra args to nodes
EXTRA_ARGS="$@"

# Check dependencies first
check_dependencies

# Prompt user for node configuration
echo "Node Configuration:"
read -p "How many write nodes (with private keys)? " WRITE_NODES
read -p "How many read nodes (without private keys)? " READ_NODES
echo
read -p "Acknowledgement URL for receipts (leave empty for default): " ACK_URL
if [ -z "$ACK_URL" ]; then
  ACK_URL="https://httpbin.org/post"
  echo "[runner] Using default acknowledgement URL: $ACK_URL"
fi

# Validate input
if ! [[ "$WRITE_NODES" =~ ^[0-9]+$ ]] || [ "$WRITE_NODES" -lt 0 ]; then
  echo "Error: Write nodes must be a non-negative integer"
  exit 1
fi

if ! [[ "$READ_NODES" =~ ^[0-9]+$ ]] || [ "$READ_NODES" -lt 0 ]; then
  echo "Error: Read nodes must be a non-negative integer"
  exit 1
fi

NODE_COUNT=$((WRITE_NODES + READ_NODES))

if [ "$NODE_COUNT" -lt 1 ]; then
  echo "Error: Must have at least one node (write or read)"
  exit 1
fi

echo "Starting $WRITE_NODES write nodes and $READ_NODES read nodes (total: $NODE_COUNT)"

# ------------------------------------------------------------
# 2. Load private keys from .env file
# ------------------------------------------------------------
if [ -f .env ]; then
  source .env
fi

# Collect available keys for nodes
KEYS=()
for i in {1..20}; do  # Check up to 20 keys in case more are added
  key_name="KEY_$i"
  if [ -n "${!key_name:-}" ]; then
    KEYS+=("${!key_name}")
  fi
done

# Collect sender keys for escrow deployment
SENDER_KEYS=()
for i in {1..20}; do  # Check up to 20 sender keys
  sender_key_name="SENDER_KEY_$i"
  if [ -n "${!sender_key_name:-}" ]; then
    SENDER_KEYS+=("${!sender_key_name}")
  fi
done

echo "[runner] found ${#SENDER_KEYS[@]} sender keys for escrow deployment"

# ------------------------------------------------------------
# Faucet helper function using new CLI
# ------------------------------------------------------------
run_nomad_faucet() {
  local private_key=$1
  local token_contract=$2
  local description=$3
  
  echo "[runner] Running nomad faucet for $description using contract $token_contract"
  
  # Use the new faucet subcommand
  if $BIN --pk "$private_key" faucet "$token_contract" >/dev/null 2>&1; then
    echo "[runner] Nomad faucet successful"
    return 0
  else
    echo "[runner] ERROR: Nomad faucet failed"
    return 1
  fi
}

# ------------------------------------------------------------
# Balance validation functions
# ------------------------------------------------------------
call_token_faucet() {
  local private_key=$1
  local description=$2
  
  echo "[runner] Calling token faucet for $description..."
  
  # Call the mint function on the token contract (each call gives 1000 tokens)
  if cast send --private-key "$private_key" --rpc-url "$HTTP_RPC" \
    "$TOKEN_CONTRACT" "mint()" >/dev/null 2>&1; then
    echo "[runner] Faucet call successful - 1000 tokens minted"
    
    # Get updated balance to confirm
    local address=$(cast wallet address "$private_key" 2>/dev/null)
    local raw_balance=$(cast call "$TOKEN_CONTRACT" "balanceOf(address)(uint256)" "$address" --rpc-url "$HTTP_RPC" 2>/dev/null || echo "0")
    local new_balance=$(echo "$raw_balance" | awk '{print $1}')
    echo "[runner] New token balance: $new_balance tokens"
    
    # Check if we need more tokens (may need multiple faucet calls)
    local min_tokens="10000000000"  # 10000 tokens (assuming 6 decimals)
    if (( new_balance >= min_tokens )); then
      echo "[runner] Sufficient tokens after faucet call"
      return 0
    else
      echo "[runner] Still need more tokens, calling faucet again..."
      # Recursively call faucet until we have enough (with a limit)
      local calls_made=1
      while (( new_balance < min_tokens && calls_made < 15 )); do
        sleep 1  # Brief delay between calls
        if cast send --private-key "$private_key" --rpc-url "$HTTP_RPC" \
          "$TOKEN_CONTRACT" "mint()" >/dev/null 2>&1; then
          ((calls_made++))
          local raw_balance_loop=$(cast call "$TOKEN_CONTRACT" "balanceOf(address)(uint256)" "$address" --rpc-url "$HTTP_RPC" 2>/dev/null || echo "0")
          new_balance=$(echo "$raw_balance_loop" | awk '{print $1}')
          echo "[runner] Faucet call $calls_made: balance now $new_balance tokens"
        else
          echo "[runner] ERROR: Faucet call $calls_made failed"
          return 1
        fi
      done
      
      if (( new_balance >= min_tokens )); then
        echo "[runner] Sufficient tokens after $calls_made faucet calls"
        return 0
      else
        echo "[runner] ERROR: Could not get sufficient tokens after $calls_made faucet calls"
        return 1
      fi
    fi
  else
    echo "[runner] ERROR: Faucet call failed"
    return 1
  fi
}

check_balance() {
  local private_key=$1
  local description=$2
  
  # Get address from private key
  local address=$(cast wallet address "$private_key" 2>/dev/null || echo "ERROR")
  if [ "$address" = "ERROR" ]; then
    echo "[runner] ERROR: Invalid private key for $description"
    return 1
  fi
  
  # Get ETH balance
  local eth_balance=$(cast balance "$address" --rpc-url "$HTTP_RPC" 2>/dev/null || echo "0")
  local eth_balance_ether=$(cast to-unit "$eth_balance" ether 2>/dev/null || echo "0")
  
  # Get token balance if TOKEN_CONTRACT is set
  local token_balance="0"
  if [ -n "$TOKEN_CONTRACT" ]; then
    local raw_balance=$(cast call "$TOKEN_CONTRACT" "balanceOf(address)(uint256)" "$address" --rpc-url "$HTTP_RPC" 2>/dev/null || echo "0")
    # Extract just the numeric part before any space or bracket
    token_balance=$(echo "$raw_balance" | awk '{print $1}')
  fi
  
  echo "[runner] $description ($address): ${eth_balance_ether} ETH, ${token_balance} tokens"
  
  # Check minimum balances (0.01 ETH, 10000 tokens)
  local min_eth_wei="10000000000000000"  # 0.01 ETH in wei
  local min_tokens="10000000000"         # 10000 tokens (assuming 6 decimals)
  
  local has_sufficient_eth=false
  local has_sufficient_tokens=false
  
  if (( eth_balance >= min_eth_wei )); then
    has_sufficient_eth=true
  fi
  
  if (( token_balance >= min_tokens )); then
    has_sufficient_tokens=true
  fi
  
  if [ "$has_sufficient_eth" = false ] || [ "$has_sufficient_tokens" = false ]; then
    echo "[runner] WARNING: $description has insufficient balance"
    [ "$has_sufficient_eth" = false ] && echo "  - Need at least 0.01 ETH for gas fees"
    [ "$has_sufficient_tokens" = false ] && echo "  - Need at least 10000 tokens for escrow funding"
    
    # Try to use faucet for insufficient tokens
    if [ "$has_sufficient_tokens" = false ] && [ "$has_sufficient_eth" = true ]; then
      echo "[runner] Attempting to use token faucet for $description"
      call_token_faucet "$private_key" "$description"
      return $?
    fi
    
    return 1
  fi
  
  return 0
}

# Validate node key balances
validate_node_balances() {
  if [ ${#KEYS[@]} -eq 0 ]; then
    echo "[runner] WARNING: No node keys found."
    return 1
  fi
  
  echo "[runner] Checking node key balances..."
  local insufficient_count=0
  local faucet_attempts=0
  
  for i in "${!KEYS[@]}"; do
    local key="${KEYS[$i]}"
    if ! check_balance "$key" "Node key $((i+1))"; then
      ((insufficient_count++))
      ((faucet_attempts++))
    fi
  done
  
  if [ $insufficient_count -gt 0 ]; then
    echo
    if [ $faucet_attempts -gt 0 ]; then
      echo "[runner] $faucet_attempts node key(s) attempted faucet calls"
      echo "[runner] Re-checking balances after faucet attempts..."
      
      # Re-check balances after faucet attempts
      insufficient_count=0
      for i in "${!KEYS[@]}"; do
        local key="${KEYS[$i]}"
        local address=$(cast wallet address "$key" 2>/dev/null)
        local raw_token_balance=$(cast call "$TOKEN_CONTRACT" "balanceOf(address)(uint256)" "$address" --rpc-url "$HTTP_RPC" 2>/dev/null || echo "0")
        local token_balance=$(echo "$raw_token_balance" | awk '{print $1}')
        local min_tokens="10000000000"
        
        if (( token_balance < min_tokens )); then
          ((insufficient_count++))
          echo "[runner] Node key $((i+1)) still has insufficient tokens: $token_balance"
        fi
      done
    fi
    
    if [ $insufficient_count -gt 0 ]; then
      echo "[runner] WARNING: $insufficient_count node key(s) still have insufficient balance after faucet attempts"
      read -p "Continue anyway? (y/N): " continue_choice
      if [[ ! "$continue_choice" =~ ^[Yy]$ ]]; then
        echo "[runner] Exiting. Please fund the node keys manually and try again."
        exit 1
      fi
    else
      echo "[runner] All node keys now have sufficient balance after faucet calls"
    fi
  fi
  
  return 0
}

# Validate sender key balances
validate_sender_balances() {
  if [ ${#SENDER_KEYS[@]} -eq 0 ]; then
    echo "[runner] WARNING: No sender keys found. Cannot deploy escrows."
    echo "Add SENDER_KEY_1, SENDER_KEY_2, etc. to your .env file"
    return 1
  fi
  
  echo "[runner] Checking sender key balances..."
  local insufficient_count=0
  local faucet_attempts=0
  
  for i in "${!SENDER_KEYS[@]}"; do
    local key="${SENDER_KEYS[$i]}"
    if ! check_balance "$key" "Sender key $((i+1))"; then
      ((insufficient_count++))
      ((faucet_attempts++))
    fi
  done
  
  if [ $insufficient_count -gt 0 ]; then
    echo
    if [ $faucet_attempts -gt 0 ]; then
      echo "[runner] $faucet_attempts sender key(s) attempted faucet calls"
      echo "[runner] Re-checking balances after faucet attempts..."
      
      # Re-check balances after faucet attempts
      insufficient_count=0
      for i in "${!SENDER_KEYS[@]}"; do
        local key="${SENDER_KEYS[$i]}"
        local address=$(cast wallet address "$key" 2>/dev/null)
        local raw_token_balance=$(cast call "$TOKEN_CONTRACT" "balanceOf(address)(uint256)" "$address" --rpc-url "$HTTP_RPC" 2>/dev/null || echo "0")
        local token_balance=$(echo "$raw_token_balance" | awk '{print $1}')
        local min_tokens="10000000000"
        
        if (( token_balance < min_tokens )); then
          ((insufficient_count++))
          echo "[runner] Sender key $((i+1)) still has insufficient tokens: $token_balance"
        fi
      done
    fi
    
    if [ $insufficient_count -gt 0 ]; then
      echo "[runner] WARNING: $insufficient_count sender key(s) still have insufficient balance after faucet attempts"
      read -p "Continue anyway? (y/N): " continue_choice
      if [[ ! "$continue_choice" =~ ^[Yy]$ ]]; then
        echo "[runner] Exiting. Please fund the sender keys manually and try again."
        exit 1
      fi
    else
      echo "[runner] All sender keys now have sufficient balance after faucet calls"
    fi
  fi
  
  return 0
}

# Shuffle the keys array to randomize key assignment
shuffle_array() {
  local -n arr=$1
  local i tmp size=${#arr[@]}
  for ((i=size-1; i>0; i--)); do
    local j=$((RANDOM % (i+1)))
    tmp=${arr[i]}
    arr[i]=${arr[j]}
    arr[j]=$tmp
  done
}

shuffle_array KEYS

# Calculate how many key pairs we have (each write node needs 2 keys)
KEY_PAIRS=$((${#KEYS[@]} / 2))
echo "[runner] found ${#KEYS[@]} keys, supporting $KEY_PAIRS write nodes with key pairs"

# Validate we have enough keys for write nodes
if [ "$WRITE_NODES" -gt "$KEY_PAIRS" ]; then
  echo "Error: Requested $WRITE_NODES write nodes but only have $KEY_PAIRS key pairs available"
  exit 1
fi

# Starting ports for RPC and P2P
RPC_START_PORT=8000
P2P_START_PORT=9000

# ------------------------------------------------------------
# 2. Validate balances and compile
# ------------------------------------------------------------
# Validate balances if TOKEN_CONTRACT and RPC are set
if [ -n "$TOKEN_CONTRACT" ] && [ -n "$HTTP_RPC" ]; then
  validate_node_balances
  validate_sender_balances
elif [ ${#SENDER_KEYS[@]} -gt 0 ]; then
  echo "[runner] WARNING: Found sender keys but missing TOKEN_CONTRACT or RPC in .env"
  echo "Cannot validate balances. Set TOKEN_CONTRACT and RPC to enable balance checking."
fi

# Download Escrow contract bytecode
setup_escrow_contract() {
  local contract_dir="/tmp/escrow_$$"
  mkdir -p "$contract_dir"
  
  echo "[runner] Setting up Escrow contract..."
  
  # Download the precompiled bytecode
  echo "[runner] Downloading precompiled bytecode..."
  if ! curl -s -L "https://raw.githubusercontent.com/MiragePrivacy/escrow/master/artifacts/bytecode.hex" -o "$contract_dir/bytecode.hex"; then
    echo "[runner] ERROR: Failed to download bytecode"
    return 1
  fi
  
  # Verify the bytecode file is not empty and starts with 0x
  local bytecode=$(cat "$contract_dir/bytecode.hex" 2>/dev/null)
  if [ -z "$bytecode" ] || [[ ! "$bytecode" =~ ^0x[0-9a-fA-F]+$ ]]; then
    echo "[runner] ERROR: Invalid bytecode format"
    return 1
  fi
  
  # Export path for later use
  export ESCROW_CONTRACT_DIR="$contract_dir"
  echo "[runner] Escrow contract bytecode ready"
}

# Setup escrow contract if we have sender keys
if [ ${#SENDER_KEYS[@]} -gt 0 ]; then
  setup_escrow_contract
fi

cargo build --release
BIN="./target/release/nomad"

# ------------------------------------------------------------
# 3. Launch nodes sequentially with random peer connections
# ------------------------------------------------------------
colors=(32 34 33 35 36 31 37)  # green, blue, yellow, magenta, cyan, red, white
declare -a NODE_ADDRS

# Launch Node 1 first (no peer to connect to)
color_idx=0
RPC_PORT_1=$((RPC_START_PORT))
P2P_PORT_1=$((P2P_START_PORT))
LOG1=$(mktemp)

# Build Node 1 command with CLI overrides
NODE1_CMD="$BIN"

# Node 1 gets keys if it's a write node (node number <= WRITE_NODES)
if [ 1 -le "$WRITE_NODES" ]; then
  key_idx1=0
  key_idx2=1
  NODE1_CMD="$NODE1_CMD --pk ${KEYS[$key_idx1]} --pk ${KEYS[$key_idx2]}"
  echo "[runner] Node 1: RPC port $RPC_PORT_1, P2P port $P2P_PORT_1 (write node with keys)"
else
  echo "[runner] Node 1: RPC port $RPC_PORT_1, P2P port $P2P_PORT_1 (read node, no keys)"
fi

# Add extra arguments (these will be passed to the run subcommand)
if [ -n "$EXTRA_ARGS" ]; then
  NODE1_CMD="$NODE1_CMD $EXTRA_ARGS"
fi

# Add run subcommand with CLI overrides
NODE1_CMD="$NODE1_CMD run --http-rpc $HTTP_RPC --rpc-port $RPC_PORT_1 --p2p-port $P2P_PORT_1"

setsid stdbuf -oL env $NODE1_CMD \
  > >(tee "$LOG1" | sed -u "s/^/\x1b[${colors[0]}mNode 1:\x1b[0m /") 2>&1 &
PIDS[1]=$!
RPC_PORTS[1]=$RPC_PORT_1

echo "[runner] waiting for Node 1 to announce its address..."

# Wait for Node 1 address
while true; do
  if ADDR=$(grep -m1 -oE '/ip4/[^ ]+' "$LOG1"); then
    NODE_ADDRS[1]=$ADDR
    echo "[runner] Node 1 address: $ADDR"
    break
  fi
  sleep 0.1
done

# Launch remaining nodes, each connecting to a random previous node
for ((i=2; i<=NODE_COUNT; i++)); do
  color_idx=$(((i-1) % ${#colors[@]}))
  RPC_PORT=$((RPC_START_PORT + i - 1))
  P2P_PORT=$((P2P_START_PORT + i - 1))
  
  # Pick a random peer from previously started nodes (1 to i-1)
  peer_idx=$((1 + RANDOM % (i - 1)))
  PEER_ADDR="${NODE_ADDRS[$peer_idx]}"
  
  # Build command with CLI args
  NODE_CMD="$BIN"
  
  # This node gets keys if it's a write node (node number <= WRITE_NODES)
  if [ $i -le "$WRITE_NODES" ]; then
    key_idx1=$((2 * (i - 1)))
    key_idx2=$((2 * (i - 1) + 1))
    NODE_CMD="$NODE_CMD --pk ${KEYS[$key_idx1]} --pk ${KEYS[$key_idx2]}"
    echo "[runner] Node $i: RPC port $RPC_PORT, P2P port $P2P_PORT, connecting to Node $peer_idx (write node with keys)"
  else
    echo "[runner] Node $i: RPC port $RPC_PORT, P2P port $P2P_PORT, connecting to Node $peer_idx (read node, no keys)"
  fi
  
  # Add extra arguments (filtered to remove faucet commands for subsequent nodes)
  if [ -n "$EXTRA_ARGS" ]; then
    # Remove any faucet subcommands from extra args for nodes after the first
    FILTERED_ARGS=$(echo "$EXTRA_ARGS" | sed 's/faucet [^ ]*//' | sed 's/--verbose/-v/g')
    if [ -n "$FILTERED_ARGS" ]; then
      NODE_CMD="$NODE_CMD $FILTERED_ARGS"
    fi
  fi
  
  # Add run subcommand with CLI overrides
  NODE_CMD="$NODE_CMD run --http-rpc $HTTP_RPC --rpc-port $RPC_PORT --p2p-port $P2P_PORT --peer $PEER_ADDR"
  
  # Create log file for this node to capture its address
  LOG_FILE=$(mktemp)
  
  setsid stdbuf -oL env $NODE_CMD \
    > >(tee "$LOG_FILE" | sed -u "s/^/\x1b[${colors[$color_idx]}mNode $i:\x1b[0m /") 2>&1 &
  PIDS[$i]=$!
  RPC_PORTS[$i]=$RPC_PORT
  
  # Wait for this node's address before launching the next node
  echo "[runner] waiting for Node $i address..."
  while true; do
    # Look for the specific port this node is listening on
    if ADDR=$(grep -m1 "Listening on /ip4/127.0.0.1/tcp/$P2P_PORT" "$LOG_FILE" | grep -oE '/ip4/[^ ]+'); then
      NODE_ADDRS[$i]=$ADDR
      echo "[runner] Node $i address: $ADDR"
      rm -f "$LOG_FILE"
      break
    fi
    sleep 0.1
  done
done

# ------------------------------------------------------------
# 5. Escrow deployment functions
# ------------------------------------------------------------
deploy_escrow() {
  local sender_key=$1
  local token_contract=$2
  local recipient=$3
  local expected_amount=$4
  
  # Validation checks
  if [ -z "$ESCROW_CONTRACT_DIR" ] || [ ! -f "$ESCROW_CONTRACT_DIR/bytecode.hex" ]; then
    echo "[runner] ERROR: Escrow contract bytecode not available" >&2
    return 1
  fi
  
  # Read and validate bytecode
  local bytecode=$(cat "$ESCROW_CONTRACT_DIR/bytecode.hex" 2>/dev/null)
  if [ -z "$bytecode" ] || [[ ! "$bytecode" =~ ^0x[0-9a-fA-F]+$ ]]; then
    echo "[runner] ERROR: Invalid bytecode format" >&2
    return 1
  fi
  
  # Encode constructor arguments
  local constructor_args
  constructor_args=$(env FOUNDRY_DISABLE_NIGHTLY_WARNING=1 cast abi-encode 'constructor(address,address,uint256)' "$token_contract" "$recipient" "$expected_amount" 2>/dev/null)
  local encode_status=$?
  
  if [ $encode_status -ne 0 ] || [ -z "$constructor_args" ]; then
    echo "[runner] ERROR: Could not encode constructor arguments" >&2
    return 1
  fi
  
  # Combine bytecode with constructor args
  local full_bytecode="${bytecode}${constructor_args#0x}"
  
  # Deploy the contract with timeout
  local deploy_result
  deploy_result=$(timeout 30s env FOUNDRY_DISABLE_NIGHTLY_WARNING=1 cast send \
    --private-key "$sender_key" \
    --rpc-url "$HTTP_RPC" \
    --create \
    --json \
    "$full_bytecode" 2>/dev/null)
  local deploy_status=$?
  
  if [ $deploy_status -eq 124 ]; then
    echo "[runner] ERROR: Contract deployment timed out after 30 seconds" >&2
    return 1
  fi
  
  if [ $deploy_status -ne 0 ] || [ -z "$deploy_result" ]; then
    echo "[runner] ERROR: Failed to deploy Escrow contract" >&2
    return 1
  fi
  
  # Try to extract contract address
  local contract_address
  
  # First try parsing as JSON (cast --json output)
  contract_address=$(echo "$deploy_result" | jq -r '.contractAddress // empty' 2>/dev/null)
  
  # If that fails, try parsing as plain transaction hash
  if [ -z "$contract_address" ] || [ "$contract_address" = "null" ] || [ "$contract_address" = "empty" ]; then
    local tx_hash=$(echo "$deploy_result" | grep -oE "0x[a-fA-F0-9]{64}" | head -1)
    
    if [ -n "$tx_hash" ]; then
      sleep 3  # Wait for transaction to be mined
      
      local receipt
      receipt=$(timeout 10s cast receipt "$tx_hash" --rpc-url "$HTTP_RPC" --json 2>/dev/null)
      if [ $? -eq 0 ] && [ -n "$receipt" ]; then
        contract_address=$(echo "$receipt" | jq -r '.contractAddress // empty')
      fi
    fi
  fi
  
  # Final validation
  if [ -z "$contract_address" ] || [ "$contract_address" = "null" ] || [ "$contract_address" = "empty" ]; then
    echo "[runner] ERROR: Could not extract contract address from deployment" >&2
    return 1
  fi
  
  # Validate contract address format
  if [[ ! "$contract_address" =~ ^0x[a-fA-F0-9]{40}$ ]]; then
    echo "[runner] ERROR: Invalid contract address format: $contract_address" >&2
    return 1
  fi
  
  # Output only the contract address (no debug messages)
  echo "$contract_address"
}

fund_escrow() {
  local escrow_address=$1
  local sender_key=$2
  local reward_amount=$3
  local payment_amount=$4
  
  echo "[runner] Funding escrow $escrow_address with reward: $reward_amount, payment: $payment_amount"
  
  # First approve the escrow to spend tokens
  local total_amount=$((reward_amount + payment_amount))
  echo "[runner] Approving escrow to spend $total_amount tokens..."
  
  local approve_result=$(cast send --private-key "$sender_key" --rpc-url "$HTTP_RPC" \
    "$TOKEN_CONTRACT" "approve(address,uint256)" "$escrow_address" "$total_amount" 2>&1)
  
  if [ $? -ne 0 ]; then
    echo "[runner] ERROR: Failed to approve escrow for token spending"
    echo "[runner] Approve error: $approve_result"
    return 1
  fi
  
  echo "[runner] Approval successful, now funding escrow..."
  
  # Fund the escrow
  local fund_result=$(cast send --private-key "$sender_key" --rpc-url "$HTTP_RPC" \
    "$escrow_address" "fund(uint256,uint256)" "$reward_amount" "$payment_amount" 2>&1)
  
  if [ $? -ne 0 ]; then
    echo "[runner] ERROR: Failed to fund escrow"
    echo "[runner] Fund error: $fund_result"
    return 1
  fi
  
  echo "[runner] Escrow funded successfully"
}

# ------------------------------------------------------------
# 6. RPC call function with randomized parameters
# ------------------------------------------------------------
deploy_escrow_and_send_signal() {
  local port=$1
  
  # Check if we have sender keys and necessary env vars
  if [ ${#SENDER_KEYS[@]} -eq 0 ] || [ -z "$TOKEN_CONTRACT" ] || [ -z "$HTTP_RPC" ]; then
    echo "[runner] Skipping escrow deployment (missing sender keys, TOKEN_CONTRACT, or RPC)"
    return 0
  fi
  
  # Select random sender key
  local sender_idx=$((RANDOM % ${#SENDER_KEYS[@]}))
  local sender_key="${SENDER_KEYS[$sender_idx]}"
  
  # Generate random transfer amount between 200-2000 USDT (whole numbers only, in micro units)
  local min_usdt=200
  local max_usdt=2000
  local usdt_amount=$((min_usdt + RANDOM % (max_usdt - min_usdt + 1)))
  local transfer_amount=$((usdt_amount * 1000000))
  
  # Array of different recipient addresses
  local recipients=(
    "0x742d35Cc6670C068c7a5FE1f1014A0C74b7F8E2f"
    "0x1234567890123456789012345678901234567890"
    "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
    "0x9876543210987654321098765432109876543210"
    "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
    "0xcafebabecafebabecafebabecafebabecafebabe"
    "0x1111111111111111111111111111111111111111"
    "0x2222222222222222222222222222222222222222"
  )
  
  # Select random recipient
  local recipient_idx=$((RANDOM % ${#recipients[@]}))
  local recipient="${recipients[$recipient_idx]}"
  
  # Generate random reward amount (5-25% of transfer amount)
  local reward_percentage=$((5 + RANDOM % 21))
  local reward_amount=$((transfer_amount * reward_percentage / 100))
  
  echo "[runner] Creating escrow for signal to port $port (sender key $((sender_idx + 1)))"
  echo "[runner]   Transfer: $usdt_amount USDT, Reward: $((reward_amount / 1000000)) USDT, Recipient: $recipient"
  
  # Deploy escrow contract
  local escrow_address
  set +e  # Temporarily disable exit on error
  escrow_address=$(deploy_escrow "$sender_key" "$TOKEN_CONTRACT" "$recipient" "$transfer_amount" 2>/dev/null)
  local deploy_status=$?
  set -e  # Re-enable exit on error
  
  if [ $deploy_status -ne 0 ] || [ -z "$escrow_address" ]; then
    echo "[runner] ERROR: Failed to deploy escrow, skipping signal"
    return 1
  fi
  
  echo "[runner] Deployed escrow at: $escrow_address"
  
  # Fund the escrow
  if ! fund_escrow "$escrow_address" "$sender_key" "$reward_amount" "$transfer_amount"; then
    echo "[runner] ERROR: Failed to fund escrow, skipping signal"
    return 1
  fi
  
  # Send the signal with real escrow address
  local data="{\"escrow_contract\":\"$escrow_address\",\"token_contract\":\"$TOKEN_CONTRACT\",\"recipient\":\"$recipient\",\"transfer_amount\":\"$transfer_amount\",\"reward_amount\":\"$reward_amount\",\"acknowledgement_url\":\"$ACK_URL\"}"
  
  echo "[runner] Sending signal to port $port with escrow $escrow_address"
  
  # Construct the JSON-RPC payload
  local rpc_payload="{\"jsonrpc\":\"2.0\",\"method\":\"mirage_signal\",\"params\":[$data],\"id\":1}"
  
  local curl_result
  curl_result=$(curl -s -X POST "http://127.0.0.1:$port" \
    -H "Content-Type: application/json" \
    -d "$rpc_payload" 2>&1)
  local curl_status=$?
  
  if [ $curl_status -eq 0 ]; then
    echo "[runner] Signal sent successfully"
  else
    echo "[runner] ERROR: Failed to send signal"
    echo "[runner] curl error: $curl_result"
    return 1
  fi
}

# ------------------------------------------------------------
# 7. Wait for nodes to start, then send test signals
# ------------------------------------------------------------
echo "[runner] waiting 3 seconds for nodes to fully start..."
sleep 3

echo "[runner] sending test signals to all nodes..."
for ((i=1; i<=NODE_COUNT; i++)); do
  echo "[runner] sending signal to Node $i (port ${RPC_PORTS[$i]})..."
  deploy_escrow_and_send_signal "${RPC_PORTS[$i]}"
  sleep 2  # Increased delay for contract deployment
done

echo "[runner] All signals sent. Nodes will continue running..."
echo "[runner] Press Ctrl+C to stop all nodes"

# ------------------------------------------------------------
# 8. Clean shutdown on Ctrl‑C
# ------------------------------------------------------------
cleanup() {
  echo
  echo "[runner] stopping nodes..."
  
  # Kill all child processes and their descendants
  for pid in "${PIDS[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then
      echo "[runner] terminating process tree for PID $pid"
      # Kill the process group to ensure all child processes are terminated
      kill -- -"$pid" 2>/dev/null || true
      # Give processes time to terminate gracefully
      sleep 0.5
      # Force kill if still running
      kill -9 -- -"$pid" 2>/dev/null || true
    fi
  done
  
  # Also kill any remaining nomad processes that might have been missed
  pkill -f "nomad.*--rpc-port" 2>/dev/null || true
  
  # Clean up log file
  rm -f "$LOG1" 2>/dev/null || true
  
  # Clean up contract directory
  if [ -n "$ESCROW_CONTRACT_DIR" ] && [ -d "$ESCROW_CONTRACT_DIR" ]; then
    rm -rf "$ESCROW_CONTRACT_DIR" 2>/dev/null || true
  fi
  
  echo "[runner] cleanup complete"
  exit 0
}

trap cleanup INT TERM

# Keep the script running to monitor nodes
wait
