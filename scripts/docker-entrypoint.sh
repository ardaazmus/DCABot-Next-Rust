#!/bin/sh
set -e

# Default environment variables
export DCA_DB_PATH="${DCA_DB_PATH:-/app/data/dcabot.sqlite3}"
export DCA_BYBIT_TESTNET="${DCA_BYBIT_TESTNET:-1}"
export DCA_OKX_DEMO="${DCA_OKX_DEMO:-1}"
export DCA_ALLOW_MAINNET="${DCA_ALLOW_MAINNET:-0}"

DATA_DIR=$(dirname "$DCA_DB_PATH")
mkdir -p "$DATA_DIR"

# Persist generated API and Vault keys across container restarts
if [ -z "$DCA_API_KEY" ]; then
  if [ -f "$DATA_DIR/.api_key" ]; then
    export DCA_API_KEY=$(cat "$DATA_DIR/.api_key")
  else
    export DCA_API_KEY=$(head -c 32 /dev/urandom | base64 | tr -dc 'a-zA-Z0-9' | head -c 32)
    echo "$DCA_API_KEY" > "$DATA_DIR/.api_key"
    chmod 600 "$DATA_DIR/.api_key" 2>/dev/null || true
  fi
fi
export ENGINE_API_KEY="${ENGINE_API_KEY:-$DCA_API_KEY}"

if [ -z "$DCA_VAULT_KEY" ]; then
  if [ -f "$DATA_DIR/.vault_key" ]; then
    export DCA_VAULT_KEY=$(cat "$DATA_DIR/.vault_key")
  else
    export DCA_VAULT_KEY=$(head -c 32 /dev/urandom | base64 | tr -dc 'a-zA-Z0-9' | head -c 32)
    echo "$DCA_VAULT_KEY" > "$DATA_DIR/.vault_key"
    chmod 600 "$DATA_DIR/.vault_key" 2>/dev/null || true
  fi
fi

echo "=================================================="
echo " Starting DCABOT Quantitative Production Runtime  "
echo " Database: $DCA_DB_PATH"
echo " Testnet Mode: $DCA_BYBIT_TESTNET"
echo " Mainnet Allowed: $DCA_ALLOW_MAINNET"
echo "=================================================="

# Start Rust Core Engine in background
echo "[supervisor] Starting Rust Engine (port 3002)..."
/app/dca_server &
ENGINE_PID=$!

# Wait for engine readiness
for i in $(seq 1 30); do
  if curl -s -m 2 http://127.0.0.1:3002/api/health >/dev/null 2>&1; then
    echo "[supervisor] Rust Engine is UP and HEALTHY."
    break
  fi
  sleep 1
done

# Faz 28 (P1-6): readiness fail-closed — motor hazır değilse UI'ı
# başlatmadan container başarısız olsun (restart policy yeniden dener).
if ! curl -s -m 2 http://127.0.0.1:3002/api/health >/dev/null 2>&1; then
  echo "[supervisor] FATAL: Rust engine 30s içinde hazır olmadı — container başarısız oluyor (restart policy yeniden deneyecek)." >&2
  kill -TERM "$ENGINE_PID" 2>/dev/null || true
  exit 1
fi

# Faz 28 (P0-1): UI auth — DCA_UI_PASSWORD boşsa rastgele üret + log yaz.
if [ -z "$DCA_UI_PASSWORD" ]; then
  DCA_UI_PASSWORD=$(head -c 24 /dev/urandom | base64 | tr -dc 'a-zA-Z0-9' | head -c 24)
  export DCA_UI_PASSWORD
  echo "[supervisor] DCA_UI_PASSWORD otomatik üretildi: $DCA_UI_PASSWORD  <-- kaydedin"
fi
export DCA_REQUIRE_AUTH="${DCA_REQUIRE_AUTH:-1}"

# Handle graceful shutdown — TERM/INT sinyali HER İKİ child'a gider.
UI_PID=""
cleanup() {
  echo "[supervisor] Stopping processes..."
  kill -TERM "$ENGINE_PID" 2>/dev/null || true
  [ -n "$UI_PID" ] && kill -TERM "$UI_PID" 2>/dev/null || true
  wait "$ENGINE_PID" 2>/dev/null || true
  [ -n "$UI_PID" ] && wait "$UI_PID" 2>/dev/null || true
  exit 0
}
trap cleanup TERM INT

# Start Next.js standalone server — exec DEĞİL: trap'in her iki süreci
# kapatabilmesi için node arka planda, shell çocukları bekler.
echo "[supervisor] Starting Next.js UI (port 3000)..."
node /app/server.js &
UI_PID=$!
wait -n "$ENGINE_PID" "$UI_PID"
echo "[supervisor] Bir süreç sonlandı — container kapatılıyor."
cleanup
