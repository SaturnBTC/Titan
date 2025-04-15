FROM rust:slim-bookworm AS builder
RUN apt-get update && apt-get install -y build-essential libssl-dev librocksdb-dev pkg-config libclang-dev
WORKDIR /tmp
COPY . .
RUN cargo build --release


FROM debian:bookworm-slim AS runner
RUN apt-get update && apt-get install -y libssl3 librocksdb7.8 ca-certificates
RUN useradd -ms /bin/bash titan
USER titan
WORKDIR /home/titan
COPY --from=builder --chown=titan:titan /tmp/target/release/titan /home/titan/titan
CMD ["/home/titan/titan"]
