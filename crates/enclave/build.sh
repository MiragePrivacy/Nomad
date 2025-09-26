#!/usr/bin/env bash

ROOT=$(cargo metadata --format-version 1 | jq .workspace_root -r)
cd $ROOT/crates/enclave

# We compute the number of threads dynamically from spawn calls in the code.
# The build script is included for counting the main thread.
THREADS=$(find . -type f -exec grep -o "std::thread::spawn" {} + | wc -l)
STACK_SIZE=0x200000 # 2 MiB (rust default)
HEAP_SIZE=0x20000000 # 512 MiB

echo "[Stage 1] Building fortanix enclave"
cargo +nightly build --locked --release --target x86_64-fortanix-unknown-sgx
BUILD_OUTPUT="$ROOT/target/x86_64-fortanix-unknown-sgx/release/nomad-enclave"

echo "[Stage 2] Converting to SGXS (threads=$THREADS, stack=$STACK_SIZE, heap=$HEAP_SIZE)"
ftxsgx-elf2sgxs --threads $THREADS --stack-size $STACK_SIZE --heap-size $HEAP_SIZE $BUILD_OUTPUT
SGXS_OUTPUT="$BUILD_OUTPUT.sgxs"

VERSION=$(cargo metadata --format-version 1 | jq '.packages[] | select(.name == "nomad-enclave") | .version' -r)
DEST="${1:-./nomad-enclave-$VERSION.sgxs}"
cd -
mv $SGXS_OUTPUT $DEST -v

# TODO: sign the enclave too
