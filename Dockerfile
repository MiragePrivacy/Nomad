FROM lukemathwalker/cargo-chef:latest-rust-1.89-slim AS chef
WORKDIR /app
RUN echo 'deb [arch=amd64] https://download.01.org/intel-sgx/sgx_repo/ubuntu focal main' | sudo tee /etc/apt/sources.list.d/intel-sgx.list && \
    wget -qO - https://download.01.org/intel-sgx/sgx_repo/ubuntu/intel-sgx-deb.key | sudo apt-key add && \
    apt-get update && \
    apt-get install -y \
    libsgx-dcap-default-qpl \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# Build dependencies - this is the caching Docker layer!
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
# Build real application
COPY . .
RUN cargo build --release --bin nomad

FROM debian:bookworm-slim AS runtime
RUN echo 'deb [arch=amd64] https://download.01.org/intel-sgx/sgx_repo/ubuntu focal main' | sudo tee /etc/apt/sources.list.d/intel-sgx.list && \
    wget -qO - https://download.01.org/intel-sgx/sgx_repo/ubuntu/intel-sgx-deb.key | sudo apt-key add && \
    apt-get update && \
    libsgx-dcap-default-qpl \
    ca-certificates \
    openssl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/nomad /usr/local/bin/nomad
RUN chmod +x /usr/local/bin/nomad
ENTRYPOINT ["/usr/local/bin/nomad"]
