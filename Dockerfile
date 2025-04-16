# Stage 1: Builder – Compile the Titan binary
FROM rust:1.81.0-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    libssl-dev \
    librocksdb-dev \
    pkg-config \
    libclang-dev

WORKDIR /tmp
# Copy the entire source into the builder container
COPY . .
# Build the Titan binary in release mode
RUN cargo build --release

# Stage 2: Runner – Create a lightweight runtime image
FROM debian:bookworm-slim AS runner

# Install runtime dependencies (including CA certificates for TLS, if needed)
RUN apt-get update && apt-get install -y \
    libssl3 \
    librocksdb7.8 \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create the titan user and set up home directory
RUN useradd -ms /bin/bash titan

# Switch to the titan user
USER titan
WORKDIR /home/titan

# Copy the compiled Titan binary from the builder stage to /usr/local/bin
COPY --from=builder --chown=titan:titan /tmp/target/release/titan /usr/local/bin/titan

# Ensure the Titan binary is executable
RUN chmod +x /usr/local/bin/titan

# Define default environment variables (overridable at runtime)
# These can be adjusted through Kubernetes or your production configuration.
ENV COMMIT_INTERVAL=5
ENV BITCOIN_RPC_URL=127.0.0.1:18443
ENV BITCOIN_RPC_USERNAME=bitcoin
ENV BITCOIN_RPC_PASSWORD=bitcoinpass
ENV CHAIN=regtest
ENV HTTP_LISTEN=0.0.0.0:3030
ENV TCP_ADDRESS=0.0.0.0:8080

# Default command to run Titan using the above environment variables.
CMD ["/bin/sh", "-c", "/usr/local/bin/titan --commit-interval ${COMMIT_INTERVAL} --bitcoin-rpc-url ${BITCOIN_RPC_URL} --bitcoin-rpc-username ${BITCOIN_RPC_USERNAME} --bitcoin-rpc-password ${BITCOIN_RPC_PASSWORD} --chain ${CHAIN} --http-listen ${HTTP_LISTEN} --index-addresses --index-bitcoin-transactions --enable-tcp-subscriptions --tcp-address ${TCP_ADDRESS} --enable-file-logging"]