#!/bin/bash
source /home/noderunner/Nomad/.env

# Compile in release mode first
cargo build --release

exec /home/noderunner/Nomad/target/release/nomad \
    --pk1 "$KEY_1" \
    --pk2 "$KEY_2" \
    --rpc-port 8000 \
    --p2p-port 9000 \
    # /ip4/127.0.0.1/tcp/9001/p2p/PEER_ID_HERE