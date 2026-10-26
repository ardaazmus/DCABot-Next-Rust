#!/bin/bash
# dca-core Rust engine launcher (port 3002)
set -e
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p /home/z/my-project/db
BIN=target/release/dca_server
if [ ! -x "$BIN" ]; then
  echo "[dca-core] release binary missing, building..."
  cargo build --release
fi
exec "$BIN"
