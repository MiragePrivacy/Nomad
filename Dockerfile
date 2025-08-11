# Multi-stage Rust build
FROM rust:1.89-slim as builder

# Install required system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy the entire workspace
COPY . .

# Build the CLI binary
RUN cargo build --release --bin nomad

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    openssl \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder stage
COPY --from=builder /app/target/release/nomad /usr/local/bin/nomad

# Make binary executable
RUN chmod +x /usr/local/bin/nomad

# Set the binary as entrypoint
ENTRYPOINT ["/usr/local/bin/nomad"]
