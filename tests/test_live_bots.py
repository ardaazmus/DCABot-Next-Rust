#!/usr/bin/env python3
"""
Test bot creation, lifecycle, and market feeds for:
1. BYBIT (both Spot and Linear Futures capabilities, Demo Trading account with active Vault key)
2. BINANCE (Spot capability, Demo Trading account with active Vault key)
"""
import json
import os
import re
import urllib.request
import urllib.error
import time

BASE_URL = "http://127.0.0.1:3002"
_LOCAL_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})

API_KEY = os.environ.get("DCA_API_KEY", "")
if not API_KEY and os.path.exists(".local-runtime/local-keys.env"):
    with open(".local-runtime/local-keys.env", "r", encoding="utf-8") as f:
        for line in f:
            m = re.match(r"^DCA_API_KEY=(.*)$", line.strip())
            if m:
                API_KEY = m.group(1).strip()
                break


def _assert_local_api(url: str, path: str) -> str:
    """Yalniz localhost motora, / ile baslayan ve ../ icermeyen yollara istek (SSRF guard)."""
    from urllib.parse import urlparse

    if not path.startswith("/") or ".." in path:
        raise SystemExit(f"gecersiz API yolu reddedildi: {path}")
    p = urlparse(url)
    if p.scheme != "http" or (p.hostname or "") not in _LOCAL_HOSTS:
        raise SystemExit(f"test yalniz localhost motora baglanir; reddedildi: {url}")
    return url


def api_call(path, method="GET", payload=None):
    url = _assert_local_api(f"{BASE_URL}{path}", path)
    data = json.dumps(payload).encode("utf-8") if payload else None
    headers = {"Content-Type": "application/json"} if payload else {}
    if API_KEY:
        headers["Authorization"] = f"Bearer {API_KEY}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            body = resp.read().decode("utf-8")
            return resp.status, json.loads(body) if body else {}
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8")
        try:
            return e.code, json.loads(body)
        except Exception:
            return e.code, {"raw": body}
    except Exception as e:
        return 0, {"error": str(e)}

def bot_config(symbol="BTCUSDT"):
    return {
        "base_order_size": "25",
        "combined_target_profit": "50",
        "cooldown_bars": 0,
        "deviation_pct": "0.012",
        "entry_signal": None,
        "entry_trigger": "Instant",
        "grid_levels": None,
        "grid_lower": None,
        "grid_mode": "arithmetic",
        "grid_trail": False,
        "grid_upper": None,
        "hedge_drawdown_trigger_pct": "6",
        "hedge_ratio": "0.5",
        "kind": "DCA_LONG",
        "leverage": "1",
        "max_safety_orders": 2,
        "pine_inputs": {},
        "pine_script": None,
        "risk_policy": {
            "active_safety_orders": 1,
            "daily_dd_breaker_pct": "2",
            "fill_model": "REALISTIC",
            "gap_consolidation": "ENFORCE",
            "grid_fee_breaker": "PERMISSIVE_WITH_WARNING",
            "m20_sl_ladder_guard": "PERMISSIVE_WITH_WARNING",
            "macro_blackout": "ENFORCE",
            "opening_salvo_buffer": "ENFORCE",
            "regime_filter": "PERMISSIVE_WITH_WARNING",
            "session_guard": "ENFORCE",
            "session_type": "CRYPTO_24_7"
        },
        "safety_order_size": "25",
        "sizing_mode": "QUOTE_NOTIONAL",
        "step_scale": "1.5",
        "stop_loss_pct": None,
        "symbol": symbol,
        "target_profit_pct": "0.015",
        "trailing_deviation_pct": "0.005",
        "trailing_enabled": False,
        "trailing_trigger_pct": "0.01",
        "use_net_tp": True,
        "volume_scale": "1"
    }

def main():
    print("=== DCABOT VENUE & BOT INTEGRATION TEST ===")
    
    # 1. Check health
    status, health = api_call("/api/health")
    print(f"\n[1] Health Check: HTTP {status} -> {health.get('status')}")
    
    # 2. Check Vault Keys
    status, keys = api_call("/api/vault/keys")
    print(f"\n[2] Vault Keys: HTTP {status}")
    for k in keys.get("keys", []):
        print(f"  - [{k.get('venue')}] {k.get('label')}: maskedKey={k.get('maskedKey')}, perms={k.get('permissions')}")

    # 3. Test Market Data for BYBIT Spot & Linear
    print("\n[3] Testing Bybit Market Data:")
    status, spot_markets = api_call("/api/markets?venue=BYBIT&category=spot&symbol=BTCUSDT")
    insts = spot_markets.get("instruments", [])
    print(f"  - Bybit Spot BTCUSDT: HTTP {status}, found {len(insts)} instruments")

    status, linear_markets = api_call("/api/markets?venue=BYBIT&category=linear&symbol=BTCUSDT")
    insts_lin = linear_markets.get("instruments", [])
    print(f"  - Bybit Linear BTCUSDT: HTTP {status}, found {len(insts_lin)} instruments")

    # 4. Test Market Data for BINANCE Spot
    print("\n[4] Testing Binance Market Data:")
    status, bin_markets = api_call("/api/markets?venue=BINANCE&category=spot&symbol=BTCUSDT")
    insts_bin = bin_markets.get("instruments", [])
    print(f"  - Binance Spot BTCUSDT: HTTP {status}, found {len(insts_bin)} instruments")

    # 5. Create BYBIT Bot (LIVE mode)
    print("\n[5] Creating Bybit Bot (LIVE Mode):")
    bybit_bot_req = {
        "name": "Bybit Live Demo Bot",
        "venue": "BYBIT",
        "mode": "LIVE",
        "kind": "DCA_LONG",
        "symbol": "BTCUSDT",
        "interval": "1m",
        "startCash": "1000",
        "config": bot_config("BTCUSDT")
    }
    status, bybit_bot_resp = api_call("/api/bots", method="POST", payload=bybit_bot_req)
    print(f"  - Bybit LIVE bot creation: HTTP {status}")
    print(f"    Response: {json.dumps(bybit_bot_resp, indent=2)}")
    bybit_bot_id = bybit_bot_resp.get("id")

    # 6. Create BINANCE Bot (PAPER mode with Live Binance Pricing)
    print("\n[6] Creating Binance Bot (PAPER Mode):")
    binance_bot_req = {
        "name": "Binance Spot Bot",
        "venue": "BINANCE",
        "mode": "PAPER",
        "kind": "DCA_LONG",
        "symbol": "BTCUSDT",
        "interval": "1m",
        "startCash": "1000",
        "config": bot_config("BTCUSDT")
    }
    status, binance_bot_resp = api_call("/api/bots", method="POST", payload=binance_bot_req)
    print(f"  - Binance bot creation: HTTP {status}")
    print(f"    Response: {json.dumps(binance_bot_resp, indent=2)}")
    binance_bot_id = binance_bot_resp.get("id")

    # 7. Check Bots Status
    print("\n[7] Inspecting Created Bots:")
    status, bots_list = api_call("/api/bots")
    for b in bots_list.get("bots", []):
        if b.get("id") in [bybit_bot_id, binance_bot_id]:
            print(f"  - Bot [{b.get('id')}] '{b.get('name')}': venue={b.get('venue')}, mode={b.get('mode')}, status={b.get('status')}, phase={b.get('stats', {}).get('phase')}")

    # 8. Test Bot Actions (Pause / Start / Panic Sell / Clean)
    if bybit_bot_id:
        print(f"\n[8] Testing Bot Actions on Bybit bot [{bybit_bot_id}]:")
        status, act_resp = api_call(f"/api/bots/{bybit_bot_id}/actions", method="POST", payload={"action": "PAUSE"})
        print(f"  - Action PAUSE: HTTP {status} -> {act_resp}")
        status, act_resp = api_call(f"/api/bots/{bybit_bot_id}/actions", method="POST", payload={"action": "START"})
        print(f"  - Action START: HTTP {status} -> {act_resp}")

    if binance_bot_id:
        print(f"\n[9] Testing Bot Actions on Binance bot [{binance_bot_id}]:")
        status, act_resp = api_call(f"/api/bots/{binance_bot_id}/actions", method="POST", payload={"action": "PAUSE"})
        print(f"  - Action PAUSE: HTTP {status} -> {act_resp}")
        status, act_resp = api_call(f"/api/bots/{binance_bot_id}/actions", method="POST", payload={"action": "START"})
        print(f"  - Action START: HTTP {status} -> {act_resp}")

    # 10. Summary
    status, summ = api_call("/api/summary")
    print(f"\n[10] Engine Summary: HTTP {status}")
    print(f"  Total bots: {summ.get('totalBots')}, Active bots: {summ.get('activeBots')}")

if __name__ == "__main__":
    main()
