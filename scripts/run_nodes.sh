#!/usr/bin/env bash
set -euo pipefail

# ------------------------------------------------------------
# 1. Parse arguments and validate
# ------------------------------------------------------------
NODE_COUNT=${1:-2}
shift # Remove first argument (node count) so remaining args can be passed to binary

if ! [[ "$NODE_COUNT" =~ ^[0-9]+$ ]] || [ "$NODE_COUNT" -lt 1 ]; then
  echo "Usage: $0 [node_count] [additional_args...]"
  echo "  node_count: number of nodes to run (default: 2, minimum: 1)"
  echo "  additional_args: arbitrary flags and arguments passed to each node"
  exit 1
fi

# Capture remaining arguments to pass to each binary call
EXTRA_ARGS="$@"

# ------------------------------------------------------------
# 2. Load private keys from .env file
# ------------------------------------------------------------
if [ -f .env ]; then
  source .env
fi

# Collect available keys
KEYS=()
for i in {1..20}; do  # Check up to 20 keys in case more are added
  key_name="KEY_$i"
  if [ -n "${!key_name:-}" ]; then
    KEYS+=("${!key_name}")
  fi
done

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

# Calculate how many key pairs we have (each node needs 2 keys)
KEY_PAIRS=$((${#KEYS[@]} / 2))
echo "[runner] found ${#KEYS[@]} keys, supporting $KEY_PAIRS nodes with key pairs"

echo "[runner] starting $NODE_COUNT nodes"

# Starting ports for RPC and P2P
RPC_START_PORT=8000
P2P_START_PORT=9000

# ------------------------------------------------------------
# 2. Compile once – faster startups for all nodes
# ------------------------------------------------------------
cargo build --release
BIN=./target/release/nomad

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

NODE1_CMD="$BIN --rpc-port $RPC_PORT_1 --p2p-port $P2P_PORT_1"

# Calculate key indices for Node 1
key_idx1=0
key_idx2=1

if [ $key_idx2 -lt ${#KEYS[@]} ]; then
  NODE1_CMD="$NODE1_CMD --pk1 ${KEYS[$key_idx1]} --pk2 ${KEYS[$key_idx2]}"
  echo "[runner] Node 1: RPC port $RPC_PORT_1, P2P port $P2P_PORT_1 (with keys)"
else
  echo "[runner] Node 1: RPC port $RPC_PORT_1, P2P port $P2P_PORT_1 (no keys)"
fi

# Add extra arguments to Node 1 command (including faucet if specified)
if [ -n "$EXTRA_ARGS" ]; then
  NODE1_CMD="$NODE1_CMD $EXTRA_ARGS"
fi

setsid stdbuf -oL env RUST_LOG=nomad=debug $NODE1_CMD \
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
  
  # Build command with peer connection
  NODE_CMD="$BIN --rpc-port $RPC_PORT --p2p-port $P2P_PORT $PEER_ADDR"
  
  # Calculate key indices for this node
  key_idx1=$((2 * (i - 1)))
  key_idx2=$((2 * (i - 1) + 1))
  
  if [ $key_idx2 -lt ${#KEYS[@]} ]; then
    NODE_CMD="$NODE_CMD --pk1 ${KEYS[$key_idx1]} --pk2 ${KEYS[$key_idx2]}"
    echo "[runner] Node $i: RPC port $RPC_PORT, P2P port $P2P_PORT, connecting to Node $peer_idx (with keys)"
  else
    echo "[runner] Node $i: RPC port $RPC_PORT, P2P port $P2P_PORT, connecting to Node $peer_idx (no keys)"
  fi
  
  # Add extra arguments but exclude --faucet to prevent double execution
  if [ -n "$EXTRA_ARGS" ]; then
    # Remove --faucet from extra args for nodes after the first
    FILTERED_ARGS=$(echo "$EXTRA_ARGS" | sed 's/--faucet [^ ]*//')
    if [ -n "$FILTERED_ARGS" ]; then
      NODE_CMD="$NODE_CMD $FILTERED_ARGS"
    fi
  fi
  
  setsid stdbuf -oL env RUST_LOG=nomad=debug $NODE_CMD \
    | sed -u "s/^/\x1b[${colors[$color_idx]}mNode $i:\x1b[0m /" &
  PIDS[$i]=$!
  RPC_PORTS[$i]=$RPC_PORT
  
  # Brief pause to let node start before launching next
  sleep 0.5
done

# ------------------------------------------------------------
# 6. RPC call function with randomized parameters
# ------------------------------------------------------------
send_signal() {
  local port=$1
  
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
  
  local data="{\"escrow_contract\":\"0x742d35Cc6670C068c7a5FE1f1014A0C74b7F8E2f\",\"token_contract\":\"$TOKEN_CONTRACT\",\"recipient\":\"$recipient\",\"transfer_amount\":\"$transfer_amount\",\"reward_amount\":\"$reward_amount\"}"
  
  curl -s -X POST "http://127.0.0.1:$port" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"mirage_signal\",\"params\":[${data}],\"id\":1}" > /dev/null
}

# ------------------------------------------------------------
# 7. Wait for nodes to start, then send test signals
# ------------------------------------------------------------
echo "[runner] waiting 3 seconds for nodes to fully start..."
sleep 3

echo "[runner] sending test signals to all nodes..."
for ((i=1; i<=NODE_COUNT; i++)); do
  echo "[runner] sending signal to Node $i (port ${RPC_PORTS[$i]})..."
  send_signal "${RPC_PORTS[$i]}"
  sleep 1
done

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
  
  echo "[runner] cleanup complete"
  exit 0
}

trap cleanup INT TERM

wait