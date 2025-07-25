#!/usr/bin/env bash
set -euo pipefail

# ------------------------------------------------------------
# 1. Compile once – faster startups for both nodes
# ------------------------------------------------------------
cargo build --release                            # adjust bin path if needed
BIN=./target/release/nomad                       # binary file

# ------------------------------------------------------------
# 2. Launch Node 1 and tee its stdout
# ------------------------------------------------------------
LOG1=$(mktemp)
stdbuf -oL "$BIN" \
  > >(tee "$LOG1" | sed -u 's/^/\x1b[32mNode 1:\x1b[0m /') 2>&1 &
PID1=$!

echo "[runner] waiting for Node 1 to announce its address …"

# ------------------------------------------------------------
# 3. Parse the first “Listening on /ip4/…/tcp/…” line
# ------------------------------------------------------------
while true; do
  if IDENT=$(grep -m1 -oE '/ip4/[^ ]+' "$LOG1"); then
    break
  fi
  sleep 0.1
done

echo "[runner] captured IDENTIFIER = $IDENT"

# ------------------------------------------------------------
# 4. Launch Node 2 with that IDENTIFIER
# ------------------------------------------------------------
stdbuf -oL "$BIN" "$IDENT" \
  | sed -u 's/^/\x1b[34mNode 2:\x1b[0m /' &
PID2=$!

# ------------------------------------------------------------
# 5. Clean shutdown on Ctrl‑C
# ------------------------------------------------------------
trap 'echo; echo "[runner] stopping…"; kill $PID1 $PID2 2>/dev/null; wait; rm -f "$LOG1"' INT TERM

wait                   # wait for both children unless interrupted
