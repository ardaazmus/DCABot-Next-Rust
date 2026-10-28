#!/usr/bin/env python3
"""
Ingest credentials from .env safely into dca-core Vault.
Strictly adheres to Zero-Leakage: secrets are never printed to stdout/stderr or logs.
"""
import os
import sys
import json
import urllib.request
import urllib.error

CORE_URL = "http://127.0.0.1:3002/api/vault/keys"
_LOCAL_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})


def _assert_local_target(url: str) -> str:
    """Bu arac yalniz localhost motor kasa'sina baglanir (SSRF guard)."""
    from urllib.parse import urlparse

    p = urlparse(url)
    if p.scheme != "http" or (p.hostname or "") not in _LOCAL_HOSTS:
        raise SystemExit(f"ingest yalniz localhost motora baglanir; reddedildi: {url}")
    return url

def parse_env(file_path=".env"):
    env_vars = {}
    if not os.path.exists(file_path):
        print(f"Error: {file_path} not found.")
        sys.exit(1)
    with open(file_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if "=" in line:
                k, v = line.split("=", 1)
                k = k.strip()
                v = v.strip().strip("'").strip('"')
                env_vars[k] = v
    return env_vars

def post_vault_key(venue, label, api_key, api_secret):
    payload = {
        "venue": venue,
        "label": label,
        "api_key": api_key,
        "api_secret": api_secret
    }
    data = json.dumps(payload).encode("utf-8")
    _assert_local_target(CORE_URL)
    req = urllib.request.Request(
        CORE_URL,
        data=data,
        headers={"Content-Type": "application/json"}
    )
    try:
        with urllib.request.urlopen(req) as resp:
            resp_body = resp.read().decode("utf-8")
            return resp.status, json.loads(resp_body)
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8")
        try:
            return e.code, json.loads(err_body)
        except Exception:
            return e.code, {"raw_error": err_body}
    except Exception as e:
        return 0, {"error": str(e)}

def main():
    env = parse_env()
    
    # 1. Bybit Key Ingestion (Spot & Futures)
    bybit_key = env.get("BYBIT_API_KEY")
    bybit_secret = env.get("BYBIT_API_SECRET")
    
    if bybit_key and bybit_secret:
        print("[INGEST] Registering BYBIT credentials (Spot & Futures)...")
        status, res = post_vault_key("BYBIT", "Bybit Production (Spot & Futures)", bybit_key, bybit_secret)
        print(f"  Status: HTTP {status}")
        print(f"  Result: {json.dumps(res, indent=2)}")
    else:
        print("[SKIP] BYBIT keys missing in .env")

    # 2. Binance Key Ingestion (Spot only)
    binance_key = env.get("BINANCE_API_KEY")
    binance_secret = env.get("BINANCE_API_SECRET")
    
    if binance_key and binance_secret:
        print("[INGEST] Registering BINANCE credentials (Spot)...")
        status, res = post_vault_key("BINANCE", "Binance Production (Spot)", binance_key, binance_secret)
        print(f"  Status: HTTP {status}")
        print(f"  Result: {json.dumps(res, indent=2)}")
    else:
        print("[SKIP] BINANCE keys missing in .env")

if __name__ == "__main__":
    main()
