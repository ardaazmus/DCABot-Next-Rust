#!/bin/sh
set -eu

# Default environment variables
export DCA_DB_PATH="${DCA_DB_PATH:-/app/data/dcabot.sqlite3}"
export DCA_BYBIT_TESTNET="${DCA_BYBIT_TESTNET:-1}"
export DCA_OKX_DEMO="${DCA_OKX_DEMO:-1}"
export DCA_ALLOW_MAINNET="${DCA_ALLOW_MAINNET:-0}"

DATA_DIR=$(dirname "$DCA_DB_PATH")
SECRET_DIR=${DCA_SECRET_DIR:-/app/secrets}
mkdir -p "$DATA_DIR"
mkdir -p "$SECRET_DIR"
chmod 700 "$SECRET_DIR"
umask 077

# *_FILE is supported for Docker/Kubernetes secret mounts. A configured file
# takes precedence over the corresponding environment variable and must be
# readable and non-empty; a typo must never silently generate a replacement.
load_secret_file() {
  case "$1" in
    DCA_API_KEY) secret_file=${DCA_API_KEY_FILE:-} ;;
    ENGINE_API_KEY) secret_file=${ENGINE_API_KEY_FILE:-} ;;
    DCA_VAULT_KEY) secret_file=${DCA_VAULT_KEY_FILE:-} ;;
    DCA_SESSION_SECRET) secret_file=${DCA_SESSION_SECRET_FILE:-} ;;
    DCA_UI_PASSWORD) secret_file=${DCA_UI_PASSWORD_FILE:-} ;;
    *) echo "[supervisor] Unsupported secret name: $1" >&2; return 1 ;;
  esac
  [ -n "$secret_file" ] || return 0
  if [ ! -f "$secret_file" ] || [ ! -r "$secret_file" ]; then
    echo "[supervisor] Configured secret file is missing or unreadable for $1." >&2
    return 1
  fi
  secret_value=$(cat "$secret_file") || return 1
  if [ -z "$secret_value" ]; then
    echo "[supervisor] Configured secret file is empty for $1." >&2
    return 1
  fi
  export "$1=$secret_value"
}

write_secret_file() {
  secret_path=$1
  secret_value=$2
  secret_tmp="${secret_path}.tmp.$$"
  if ! printf '%s\n' "$secret_value" > "$secret_tmp"; then
    rm -f "$secret_tmp"
    return 1
  fi
  chmod 600 "$secret_tmp"
  if ! mv -f "$secret_tmp" "$secret_path"; then
    rm -f "$secret_tmp"
    return 1
  fi
}

clear_legacy_secret() {
  legacy_path=$1
  if rm "$legacy_path" 2>/dev/null; then
    return 0
  fi
  # Some older bind/named volumes have a root-owned parent directory. The
  # dca user can still wipe its own 0600 key file even when it cannot unlink it.
  if printf '\n' > "$legacy_path"; then
    chmod 600 "$legacy_path"
    echo "[supervisor] Could not unlink a legacy key file; its contents were cleared."
    return 0
  fi
  echo "[supervisor] Could not clear a legacy key file: $legacy_path" >&2
  return 1
}

generate_secret() {
  secret_length=$1
  generated=$(head -c 48 /dev/urandom | base64 | tr -dc 'a-zA-Z0-9' | head -c "$secret_length")
  if [ "${#generated}" -ne "$secret_length" ]; then
    echo "[supervisor] Could not generate a secure secret." >&2
    return 1
  fi
  printf '%s' "$generated"
}

load_secret_file DCA_API_KEY
load_secret_file ENGINE_API_KEY
load_secret_file DCA_VAULT_KEY
load_secret_file DCA_SESSION_SECRET
load_secret_file DCA_UI_PASSWORD

# Persist generated keys outside the database volume. Existing installations
# migrate their old data-volume key files atomically before removing the old
# copies, preserving access to the encrypted vault across the volume change.
ensure_persisted_secret() {
  secret_name=$1
  secret_filename=$2
  legacy_filename=$3
  secret_length=$4
  case "$secret_name" in
    DCA_API_KEY) secret_value=${DCA_API_KEY:-} ;;
    DCA_VAULT_KEY) secret_value=${DCA_VAULT_KEY:-} ;;
    DCA_SESSION_SECRET) secret_value=${DCA_SESSION_SECRET:-} ;;
    DCA_UI_PASSWORD) secret_value=${DCA_UI_PASSWORD:-} ;;
  esac

  if [ -n "$secret_value" ]; then
    export "$secret_name=$secret_value"
    return 0
  fi

  secret_path="$SECRET_DIR/$secret_filename"
  legacy_path="$DATA_DIR/$legacy_filename"
  if [ -f "$secret_path" ]; then
    secret_value=$(cat "$secret_path") || return 1
    if [ -z "$secret_value" ]; then
      echo "[supervisor] Persisted secret is empty: $secret_path" >&2
      return 1
    fi
    if [ -f "$legacy_path" ] && [ "$(cat "$legacy_path")" = "$secret_value" ]; then
      clear_legacy_secret "$legacy_path"
      echo "[supervisor] Cleared a verified legacy key copy from the data volume."
    fi
  elif [ -f "$legacy_path" ]; then
    secret_value=$(cat "$legacy_path") || return 1
    if [ -z "$secret_value" ]; then
      echo "[supervisor] Legacy secret is empty: $legacy_path" >&2
      return 1
    fi
    write_secret_file "$secret_path" "$secret_value"
    if [ "$(cat "$secret_path")" != "$secret_value" ]; then
      echo "[supervisor] Legacy secret migration verification failed." >&2
      return 1
    fi
    clear_legacy_secret "$legacy_path"
    echo "[supervisor] Migrated a legacy key into the protected secrets volume."
  else
    secret_value=$(generate_secret "$secret_length")
    write_secret_file "$secret_path" "$secret_value"
  fi
  export "$secret_name=$secret_value"
}

ensure_persisted_secret DCA_API_KEY .api_key .api_key 32
ensure_persisted_secret DCA_VAULT_KEY .vault_key .vault_key 32
ensure_persisted_secret DCA_SESSION_SECRET .session_secret .session_secret 48
if [ "${#DCA_SESSION_SECRET}" -lt 32 ]; then
  echo "[supervisor] DCA_SESSION_SECRET must contain at least 32 bytes." >&2
  exit 1
fi
UI_PASSWORD_AUTO_GENERATED=0
if [ -z "${DCA_UI_PASSWORD:-}" ]; then
  UI_PASSWORD_AUTO_GENERATED=1
fi
ensure_persisted_secret DCA_UI_PASSWORD .ui_password .ui_password 24
export ENGINE_API_KEY="${ENGINE_API_KEY:-$DCA_API_KEY}"

echo "=================================================="
echo " Starting DCABOT Quantitative Production Runtime  "
echo " Database: $DCA_DB_PATH"
echo " Testnet Mode: $DCA_BYBIT_TESTNET"
echo " Mainnet Allowed: $DCA_ALLOW_MAINNET"
echo "=================================================="

# Portable POSIX child wrapper: Debian's /bin/sh (dash) does not implement
# `wait -n`, so children publish their exit status and forward shutdown signals.
ENGINE_STATUS="/tmp/dca-engine.$$.status"
UI_STATUS="/tmp/dca-ui.$$.status"
rm -f "$ENGINE_STATUS" "$UI_STATUS"
ENGINE_PID=""
UI_PID=""
cleanup() {
  echo "[supervisor] Stopping processes..."
  [ -n "$ENGINE_PID" ] && kill -TERM "$ENGINE_PID" 2>/dev/null || true
  [ -n "$UI_PID" ] && kill -TERM "$UI_PID" 2>/dev/null || true
  [ -n "$ENGINE_PID" ] && wait "$ENGINE_PID" 2>/dev/null || true
  [ -n "$UI_PID" ] && wait "$UI_PID" 2>/dev/null || true
  exit 0
}
trap cleanup TERM INT

run_child() {
  status_file=$1
  shift
  "$@" &
  child_pid=$!
  trap 'kill -TERM "$child_pid" 2>/dev/null || true; wait "$child_pid" 2>/dev/null || true; exit 143' TERM INT
  if wait "$child_pid"; then
    child_status=0
  else
    child_status=$?
  fi
  printf '%s\n' "$child_status" > "$status_file"
  return "$child_status"
}

# Start Rust Core Engine in background.
echo "[supervisor] Starting Rust Engine (port 3002)..."
run_child "$ENGINE_STATUS" /app/dca_server &
ENGINE_PID=$!

# Wait for engine readiness
for i in $(seq 1 30); do
  [ ! -f "$ENGINE_STATUS" ] || break
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
  wait "$ENGINE_PID" 2>/dev/null || true
  exit 1
fi

# UI auth — otomatik parola ayrı secret volume'da kalıcıdır; stdout'a yazılmaz.
if [ "$UI_PASSWORD_AUTO_GENERATED" -eq 1 ]; then
  echo "[supervisor] UI password is stored in the protected secrets volume. Retrieve it with: docker exec <container> cat $SECRET_DIR/.ui_password"
fi
export DCA_REQUIRE_AUTH="${DCA_REQUIRE_AUTH:-1}"

# Start Next.js standalone server under the same signal-forwarding wrapper.
echo "[supervisor] Starting Next.js UI (port 3000)..."
run_child "$UI_STATUS" node /app/server.js &
UI_PID=$!
while [ ! -f "$ENGINE_STATUS" ] && [ ! -f "$UI_STATUS" ]; do
  sleep 1 &
  monitor_pid=$!
  wait "$monitor_pid" 2>/dev/null || true
done
echo "[supervisor] Bir süreç sonlandı — container kapatılıyor."
cleanup
