#!/bin/bash
# Portable dca-core Rust engine launcher (port 3002)
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

mkdir -p "$DIR/db"
BIN="$DIR/target/release/dca_server"
if [ ! -x "$BIN" ]; then
  echo "[dca-core] release binary missing, building..."
  cargo build --release --bin dca_server
fi
exec "$BIN"
