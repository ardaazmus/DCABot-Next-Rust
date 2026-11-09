#!/bin/bash
# Portable Auto-restart for Next.js dev server
ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"
while true; do
  CODE=$(curl -s -m 3 -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/ 2>/dev/null || true)
  if [ "$CODE" != "200" ]; then
    pkill -f "next dev" 2>/dev/null || true
    sleep 1
    echo "[$(date -Iseconds)] keepalive starting next server" >> "$ROOT_DIR/dev.log"
    nohup npm run dev >> "$ROOT_DIR/dev.log" 2>&1 &
    sleep 5
    for i in $(seq 1 45); do
      C2=$(curl -s -m 3 -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/ 2>/dev/null || true)
      [ "$C2" = "200" ] && break
      sleep 2
    done
  fi
  sleep 10
done
