#!/bin/bash
# Auto-restart dca_server if the host OOM storm reaps it (sandbox constraint).
export PATH="$HOME/.cargo/bin:$PATH"
cd /home/z/my-project/mini-services/dca-core
while true; do
  CODE=$(curl -s -m 3 -o /dev/null -w "%{http_code}" http://127.0.0.1:3002/api/health 2>/dev/null)
  if [ "$CODE" != "200" ]; then
    pkill -f "target/release/dca_server" 2>/dev/null
    sleep 1
    echo "[$(date -Iseconds)] keepalive (re)starting dca_server" >> server.log
    setsid nohup bash run.sh >> server.log 2>&1 < /dev/null &
    for i in $(seq 1 15); do
      C2=$(curl -s -m 2 -o /dev/null -w "%{http_code}" http://127.0.0.1:3002/api/health 2>/dev/null)
      [ "$C2" = "200" ] && break
      sleep 1
    done
  fi
  sleep 10
done
