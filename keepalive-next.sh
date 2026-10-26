#!/bin/bash
# Auto-restart Next dev server if it dies (sandbox reaps detached processes)
export PATH="/home/z/.bun/bin:$PATH"
cd /home/z/my-project
while true; do
  CODE=$(curl -s -m 3 -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/ 2>/dev/null)
  if [ "$CODE" != "200" ]; then
    pkill -f "next dev" 2>/dev/null
    pkill -f "next-server" 2>/dev/null
    sleep 1
    # fresh log marker
    echo "[$(date -Iseconds)] keepalive starting dev server" >> /home/z/my-project/dev.log
    setsid nohup bun run dev >> /home/z/my-project/dev.log 2>&1 < /dev/null &
    sleep 5
    # wait for readiness up to 90s
    for i in $(seq 1 45); do
      C2=$(curl -s -m 3 -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/ 2>/dev/null)
      [ "$C2" = "200" ] && break
      sleep 2
    done
  fi
  sleep 3
done
