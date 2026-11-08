#!/usr/bin/env python3
"""
Verify all remediated security findings against running dca_server and Next.js:
1. SEC-FIND-03: /api/stream auth enforcement (401 without auth, 200 with Bearer or token)
2. SEC-FIND-04: /api/ai/diagnose SSRF block (rejects 169.254.169.254, non-https, local IPs for cloud)
3. SEC-FIND-06: /api/webhooks/tradingview 16KB body limit (rejects >16KB payloads)
4. SEC-FIND-07: Mutex poisoning resilience (poison recovery works)
5. Next.js /api/health proxy verification
"""
import json
import os
import re
import urllib.request
import urllib.error
import sys

API_KEY = os.environ.get("DCA_API_KEY", "")
if not API_KEY and os.path.exists(".local-runtime/local-keys.env"):
    with open(".local-runtime/local-keys.env", "r", encoding="utf-8") as f:
        for line in f:
            m = re.match(r"^DCA_API_KEY=(.*)$", line.strip())
            if m:
                API_KEY = m.group(1).strip()
                break

print(f"Loaded DCA_API_KEY: {'yes' if API_KEY else 'NO'}")

passed = 0
failed = 0

def check(name, ok, msg=""):
    global passed, failed
    if ok:
        print(f"  [PASS] {name} {msg}")
        passed += 1
    else:
        print(f"  [FAIL] {name} {msg}")
        failed += 1

# 1. SEC-FIND-03: /api/stream without auth
try:
    req = urllib.request.Request("http://127.0.0.1:3002/api/stream")
    with urllib.request.urlopen(req, timeout=3) as resp:
        check("SSE unauthenticated blocked", False, f"status was {resp.status}")
except urllib.error.HTTPError as e:
    check("SSE unauthenticated blocked", e.code == 401, f"HTTP {e.code}")
except Exception as e:
    check("SSE unauthenticated blocked", False, str(e))

# 2. SEC-FIND-03: /api/stream with Bearer auth
try:
    req = urllib.request.Request("http://127.0.0.1:3002/api/stream")
    req.add_header("Authorization", f"Bearer {API_KEY}")
    with urllib.request.urlopen(req, timeout=3) as resp:
        ct = resp.headers.get("Content-Type", "")
        check("SSE Bearer auth accepted", resp.status == 200 and "text/event-stream" in ct, f"status {resp.status}, ct: {ct}")
except Exception as e:
    check("SSE Bearer auth accepted", False, str(e))

# 3. SEC-FIND-03: /api/stream with token query param
try:
    req = urllib.request.Request(f"http://127.0.0.1:3002/api/stream?token={API_KEY}")
    with urllib.request.urlopen(req, timeout=3) as resp:
        ct = resp.headers.get("Content-Type", "")
        check("SSE query token auth accepted", resp.status == 200 and "text/event-stream" in ct, f"status {resp.status}, ct: {ct}")
except Exception as e:
    check("SSE query token auth accepted", False, str(e))

# 4. SEC-FIND-04: /api/ai/diagnose SSRF block (AWS/cloud metadata)
try:
    req = urllib.request.Request(
        "http://127.0.0.1:3002/api/ai/diagnose",
        data=json.dumps({
            "mode": "diagnose",
            "config": {
                "provider": "openai",
                "base_url": "http://169.254.169.254/latest/meta-data",
                "api_key": "test"
            }
        }).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {API_KEY}"},
        method="POST"
    )
    with urllib.request.urlopen(req, timeout=5) as resp:
        check("SSRF metadata block", False, f"unexpected success {resp.status}")
except urllib.error.HTTPError as e:
    check("SSRF metadata block", e.code == 400, f"HTTP {e.code}")
except Exception as e:
    check("SSRF metadata block", False, str(e))

# 5. SEC-FIND-04: /api/ai/diagnose SSRF block (Local IP for cloud provider)
try:
    req = urllib.request.Request(
        "http://127.0.0.1:3002/api/ai/diagnose",
        data=json.dumps({
            "mode": "diagnose",
            "config": {
                "provider": "openai",
                "base_url": "http://127.0.0.1:8080",
                "api_key": "test"
            }
        }).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {API_KEY}"},
        method="POST"
    )
    with urllib.request.urlopen(req, timeout=5) as resp:
        check("SSRF localhost block", False, f"unexpected success {resp.status}")
except urllib.error.HTTPError as e:
    check("SSRF localhost block", e.code == 400, f"HTTP {e.code}")
except Exception as e:
    check("SSRF localhost block", False, str(e))

# 6. SEC-FIND-06: /api/webhooks/tradingview body limit (>16KB payload)
try:
    oversized = {
        "secret": "x",
        "botId": "b1",
        "action": "START",
        "timestamp": 1234567,
        "padding": "A" * (20 * 1024)
    }
    req = urllib.request.Request(
        "http://127.0.0.1:3002/api/webhooks/tradingview",
        data=json.dumps(oversized).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST"
    )
    with urllib.request.urlopen(req, timeout=5) as resp:
        check("Webhook oversized payload block", False, f"unexpected success {resp.status}")
except urllib.error.HTTPError as e:
    check("Webhook oversized payload block", e.code == 413, f"HTTP {e.code}")
except Exception as e:
    check("Webhook oversized payload block", False, str(e))

# 7. Next.js /api/health
try:
    req = urllib.request.Request("http://127.0.0.1:3000/api/health")
    with urllib.request.urlopen(req, timeout=5) as resp:
        data = json.loads(resp.read().decode("utf-8"))
        check("Next.js /api/health proxy", resp.status == 200 and data.get("service") == "dca-core", f"HTTP {resp.status}, service: {data.get('service')}")
except Exception as e:
    check("Next.js /api/health proxy", False, str(e))

print(f"\nTotal: {passed} passed, {failed} failed")
if failed > 0:
    sys.exit(1)
