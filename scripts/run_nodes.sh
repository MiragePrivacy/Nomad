#!/usr/bin/env bash
set -euo pipefail

# ------------------------------------------------------------
# 1. Parse arguments and validate
# ------------------------------------------------------------
NODE_COUNT=${1:-2}
if ! [[ "$NODE_COUNT" =~ ^[0-9]+$ ]] || [ "$NODE_COUNT" -lt 1 ]; then
  echo "Usage: $0 [node_count]"
  echo "  node_count: number of nodes to run (default: 2, minimum: 1)"
  exit 1
fi

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
# 3. Launch Node 1 and capture its address
# ------------------------------------------------------------
LOG1=$(mktemp)
colors=(32 34 33 35 36 31 37)  # green, blue, yellow, magenta, cyan, red, white
RPC_PORT_1=$((RPC_START_PORT))
P2P_PORT_1=$((P2P_START_PORT))

# Build command with keys if available
NODE1_CMD="$BIN --rpc-port $RPC_PORT_1 --p2p-port $P2P_PORT_1"
if [ ${#KEYS[@]} -ge 2 ]; then
  NODE1_CMD="$NODE1_CMD --pk1 ${KEYS[0]} --pk2 ${KEYS[1]}"
  echo "[runner] Node 1: RPC port $RPC_PORT_1, P2P port $P2P_PORT_1 (with keys)"
else
  echo "[runner] Node 1: RPC port $RPC_PORT_1, P2P port $P2P_PORT_1 (no keys)"
fi

setsid stdbuf -oL env RUST_LOG=nomad=debug $NODE1_CMD \
  > >(tee "$LOG1" | sed -u "s/^/\x1b[${colors[0]}mNode 1:\x1b[0m /") 2>&1 &
PIDS[1]=$!
RPC_PORTS[1]=$RPC_PORT_1

echo "[runner] waiting for Node 1 to announce its address …"

# ------------------------------------------------------------
# 4. Parse the first "Listening on /ip4/…/tcp/…" line
# ------------------------------------------------------------
while true; do
  if IDENT=$(grep -m1 -oE '/ip4/[^ ]+' "$LOG1"); then
    break
  fi
  sleep 0.1
done

echo "[runner] captured IDENTIFIER = $IDENT"

# ------------------------------------------------------------
# 5. Launch remaining nodes with that IDENTIFIER
# ------------------------------------------------------------
for ((i=2; i<=NODE_COUNT; i++)); do
  color_idx=$(((i-1) % ${#colors[@]}))
  RPC_PORT=$((RPC_START_PORT + i - 1))
  P2P_PORT=$((P2P_START_PORT + i - 1))
  
  # Build command with keys if available for this node
  NODE_CMD="$BIN --rpc-port $RPC_PORT --p2p-port $P2P_PORT $IDENT"
  
  # Calculate key indices for this node (node i uses keys at positions 2*(i-1) and 2*(i-1)+1)
  key_idx1=$((2 * (i - 1)))
  key_idx2=$((2 * (i - 1) + 1))
  
  if [ $key_idx2 -lt ${#KEYS[@]} ]; then
    NODE_CMD="$NODE_CMD --pk1 ${KEYS[$key_idx1]} --pk2 ${KEYS[$key_idx2]}"
    echo "[runner] Node $i: RPC port $RPC_PORT, P2P port $P2P_PORT (with keys)"
  else
    echo "[runner] Node $i: RPC port $RPC_PORT, P2P port $P2P_PORT (no keys)"
  fi
  
  setsid stdbuf -oL env RUST_LOG=nomad=debug $NODE_CMD \
    | sed -u "s/^/\x1b[${colors[$color_idx]}mNode $i:\x1b[0m /" &
  PIDS[$i]=$!
  RPC_PORTS[$i]=$RPC_PORT
done

# ------------------------------------------------------------
# 6. RPC call function
# ------------------------------------------------------------
send_signal() {
  local port=$1
  local data='{"escrow_contract":"0x742d35Cc6670C068c7a5FE1f1014A0C74b7F8E2f","token_contract":"0x1234567890123456789012345678901234567890","recipient":"0xabcdefabcdefabcdefabcdefabcdefabcdefabcd","transfer_amount":"1000","reward_amount":"50"}'
  
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
  
  rm -f "$LOG1"
  echo "[runner] cleanup complete"
  exit 0
}

trap cleanup INT TERM

wait