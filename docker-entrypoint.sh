#!/bin/sh
set -e

# (1) fix permissions
chown -R titan:titan /home/titan/data

# (2) drop privileges and exec Titan with env‑vars interpolated by the shell
exec gosu titan /usr/local/bin/titan \
  --commit-interval "${COMMIT_INTERVAL}" \
  --main-loop-interval "${MAIN_LOOP_INTERVAL}" \
  --bitcoin-rpc-url    "${BITCOIN_RPC_URL}" \
  --bitcoin-rpc-username "${BITCOIN_RPC_USERNAME}" \
  --bitcoin-rpc-password "${BITCOIN_RPC_PASSWORD}" \
  --chain               "${CHAIN}" \
  --http-listen         "${HTTP_LISTEN}" \
  --index-addresses \
  --index-bitcoin-transactions \
  --enable-tcp-subscriptions \
  --tcp-address         "${TCP_ADDRESS}"