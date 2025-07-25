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

echo "[runner] starting $NODE_COUNT nodes"

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
stdbuf -oL "$BIN" \
  > >(tee "$LOG1" | sed -u "s/^/\x1b[${colors[0]}mNode 1:\x1b[0m /") 2>&1 &
PIDS[1]=$!

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
  stdbuf -oL "$BIN" "$IDENT" \
    | sed -u "s/^/\x1b[${colors[$color_idx]}mNode $i:\x1b[0m /" &
  PIDS[$i]=$!
done

# ------------------------------------------------------------
# 6. Clean shutdown on Ctrl‑C
# ------------------------------------------------------------
trap 'echo; echo "[runner] stopping…"; kill ${PIDS[@]} 2>/dev/null; wait; rm -f "$LOG1"' INT TERM

wait