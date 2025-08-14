#!/bin/bash
source .env

# Compile in release mode first
cargo build --release

exec env /home/noderunner/Nomad/target/release/nomad \
    run \
    --http-rpc "$HTTP_RPC" \
    --pk "$KEY_1" \
    --pk "$KEY_2" \
    --rpc-port 8000 \
    --p2p-port 9000 \
    # /ip4/127.0.0.1/tcp/9001/p2p/PEER_ID_HERE
