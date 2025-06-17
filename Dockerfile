# Stage 1: Builder – compile Titan
FROM rust:1.81.0-bookworm AS builder

RUN apt-get update && apt-get install -y \
    build-essential \
    libssl-dev \
    librocksdb-dev \
    pkg-config \
    libclang-dev \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /tmp
COPY . .
RUN cargo build --release

# Stage 2: Runner – minimal runtime
FROM debian:bookworm-slim AS runner

# Install runtime deps
RUN apt-get update && apt-get install -y \
    libssl3 \
    librocksdb7.8 \
    ca-certificates \
  && rm -rf /var/lib/apt/lists/*

# Create unprivileged titan user
RUN useradd -ms /bin/bash titan

# Create data directory
RUN mkdir -p /home/titan/data \
  && chown titan:titan /home/titan/data

# Copy the compiled binary
COPY --from=builder --chown=titan:titan /tmp/target/release/titan /usr/local/bin/titan
RUN chmod +x /usr/local/bin/titan

# Switch to non-root user
USER titan
WORKDIR /home/titan

# Default environment (overridable at runtime)
ENV COMMIT_INTERVAL=5
ENV BITCOIN_RPC_URL=127.0.0.1:18443
ENV BITCOIN_RPC_USERNAME=bitcoin
ENV BITCOIN_RPC_PASSWORD=bitcoinpass
ENV CHAIN=regtest
ENV HTTP_LISTEN=0.0.0.0:3030
ENV TCP_ADDRESS=0.0.0.0:8080

# Expose the mountpoint for the data dir
VOLUME ["/home/titan/data"]

CMD ["/bin/sh", "-c", "/usr/local/bin/titan --commit-interval ${COMMIT_INTERVAL} --bitcoin-rpc-url ${BITCOIN_RPC_URL} --bitcoin-rpc-username ${BITCOIN_RPC_USERNAME} --bitcoin-rpc-password ${BITCOIN_RPC_PASSWORD} --chain ${CHAIN} --http-listen ${HTTP_LISTEN} --index-addresses --index-bitcoin-transactions --enable-tcp-subscriptions --tcp-address ${TCP_ADDRESS}"]