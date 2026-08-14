#!/usr/bin/env bash
set -e

PORT=${1:-8080}

echo "================================================================================"
echo " OPENHEART SERVER RESTART SCRIPT :: PORT ${PORT}"
echo "================================================================================"

# Find and terminate any process bound to the target port
PIDS=$(lsof -t -i:${PORT} 2>/dev/null || true)
if [ -n "$PIDS" ]; then
    echo "[RESTART] Closing existing server process(es): ${PIDS}"
    kill -9 $PIDS 2>/dev/null || true
    sleep 1
else
    echo "[RESTART] No existing server process found running on port ${PORT}."
fi

echo "[RESTART] Launching OpenHeart Web Server on http://0.0.0.0:${PORT}..."
cargo run -- server ${PORT}
