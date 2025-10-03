#!/usr/bin/env bash
set -e

ROOT=$(cargo metadata --format-version 1 | jq .workspace_root -r)
cd $ROOT

# We compute the number of threads dynamically from spawn calls in the code.
# The build script is included for counting the main thread.
THREADS=1
STACK_SIZE=0x200000 # 2 MiB (rust default)
HEAP_SIZE=0x20000000 # 512 MiB

echo "[Stage 1] Building fortanix enclave"
cargo build -p nomad-enclave --locked --release --target x86_64-fortanix-unknown-sgx
BUILD_OUTPUT="$ROOT/target/x86_64-fortanix-unknown-sgx/release/nomad-enclave"

echo "[Stage 2] Converting to SGXS (threads=$THREADS, stack=$STACK_SIZE, heap=$HEAP_SIZE)"
ftxsgx-elf2sgxs --threads $THREADS --stack-size $STACK_SIZE --heap-size $HEAP_SIZE $BUILD_OUTPUT
SGXS_OUTPUT="$BUILD_OUTPUT.sgxs"

VERSION=$(cargo metadata --format-version 1 | jq '.packages[] | select(.name == "nomad-enclave") | .version' -r)
FINAL_OUTPUT="./nomad-enclave-$VERSION.sgxs"
mv $SGXS_OUTPUT $FINAL_OUTPUT

echo "[Stage 3] Signing enclave"
SIGNER=mirage.pem
if [ ! -f "$SIGNER" ]; then
  openssl genrsa -3 3072 > $SIGNER
fi
SIG_OUTPUT="./nomad-enclave-$VERSION.sig"
sgxs-sign --key $SIGNER $FINAL_OUTPUT $SIG_OUTPUT -d --xfrm 7/0 --isvprodid 0 --isvsvn 0
