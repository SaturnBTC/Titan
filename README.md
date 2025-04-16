# Titan - High-Performance Bitcoin Indexer

Titan is a next-gen Bitcoin indexer by Saturn, designed for real-time blockchain access, optimized queries, and seamless Runes protocol integration. It delivers superior performance with minimal resource consumption.

## Key Features

- Optimized Performance – Fast, efficient queries tailored for dApps and validator nodes.
- Real-Time Blockchain Access – Instant data retrieval for responsive applications.
- Scalable & Lightweight – High throughput with minimal computational overhead.
- Advanced Runes Integration – Native support for Runes tokens, instant token data, and secure transaction validation.
- Mempool-Level Indexing for Runes – Differentiates pending vs confirmed runes in outputs from transactions in the mempool. 

## Requirements

- Bitcoin Node 27.0
- Rust v1.56+ (using the 2021 edition)
- Cargo v1.85.1

## How to run it

```bash
cargo run -p titan -- --bitcoin-rpc-url http://localhost:<PORT> --bitcoin-rpc-username <USERNAME> --bitcoin-rpc-password <PASSWORD> --chain regtest --index-addresses --index-bitcoin-transactions --enable-tcp-subscriptions --data-dir ~/titan-indexer
```

## How to build it

```bash
cargo build --release
```

## Client SDKs

Titan provides official client SDKs for easy integration:

- Rust SDK – [Titan Client for Rust](./client/README.md)

- TypeScript SDK – [Titan Client for TypeScript](./ts-sdk/README.md)

Each SDK has its own README with detailed installation and usage instructions.

# Running Titan with Bitcoind Using Docker

This guide explains how to build and run the Titan indexer together with a Bitcoin Core node (Bitcoind) using Docker and docker-compose. Both the Titan Dockerfile and the docker-compose file are configured for production-style deployment and can be easily adapted.

## Prerequisites

- **Docker & Docker Compose:** Make sure you have Docker and Docker Compose installed.
- **Git:** Required to build the Titan image from source.
- **.env Configuration:** Before starting, copy the provided example environment file and update the necessary values.

## How to run

The docker-compose file orchestrates two services on a custom network (titan-network):

- bitcoind:
  Runs the official Bitcoin Core container in regtest mode with -txindex enabled and RPC authentication via rpcauth.

- titan:
  Builds the Titan image from the local Dockerfile. It depends on Bitcoind (using a health check) and passes its configuration via environment variables.

Before running containers run the following command to enable necessary permissions to the titan user:

```bash
sudo chown -R 1000:1000 ./titan_data
```

After that run the following command in order to run services:

```bash
docker-compose up --build
```
