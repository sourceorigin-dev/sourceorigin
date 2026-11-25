#!/usr/bin/env bash
# Restarts the stub API so every run starts from the same state. Stops only the
# process this script started, by the PID it saved.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p .run
if [ -f .run/api.pid ] && kill -0 "$(cat .run/api.pid)" 2>/dev/null; then
  kill "$(cat .run/api.pid)"
  while kill -0 "$(cat .run/api.pid)" 2>/dev/null; do sleep 0.05; done
fi
nohup api/target/release/stub-api fixture > .run/api.log 2>&1 &
echo $! > .run/api.pid
until curl -sf -o /dev/null http://127.0.0.1:4100/account; do sleep 0.05; done
