#!/bin/bash
# Portable keepalive for dca_server
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

while true; do
  CODE=$(curl -s -m 3 -o /dev/null -w "%{http_code}" http://127.0.0.1:3002/api/health 2>/dev/null || true)
  if [ "$CODE" != "200" ]; then
    pkill -f "dca_server" 2>/dev/null || true
    sleep 1
    echo "[$(date -Iseconds)] keepalive (re)starting dca_server" >> "$DIR/server.log"
    nohup bash run.sh >> "$DIR/server.log" 2>&1 &
    for i in $(seq 1 15); do
      C2=$(curl -s -m 2 -o /dev/null -w "%{http_code}" http://127.0.0.1:3002/api/health 2>/dev/null || true)
      [ "$C2" = "200" ] && break
      sleep 1
    done
  fi
  sleep 10
done
