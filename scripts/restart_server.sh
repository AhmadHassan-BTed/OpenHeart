#!/usr/bin/env bash
set -euo pipefail

# Restarts the OpenHeart server on PORT (default: $OPENHEART_PORT or 8080).
# Only processes that are OpenHeart servers are stopped; anything else holding the port
# is reported and left alone.

PORT="${1:-${OPENHEART_PORT:-8080}}"
case "$PORT" in
    ''|*[!0-9]*)
        echo "[RESTART] Invalid port '${PORT}'. Usage: scripts/restart_server.sh [PORT]" >&2
        exit 2
        ;;
esac
HOST="${OPENHEART_HOST:-127.0.0.1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

echo "================================================================================"
echo " OPENHEART SERVER RESTART SCRIPT :: ${HOST}:${PORT}"
echo "================================================================================"

if command -v lsof >/dev/null 2>&1; then
    PIDS="$(lsof -t -iTCP:"${PORT}" -sTCP:LISTEN 2>/dev/null || true)"
else
    echo "[RESTART] lsof not found; cannot detect an existing server on port ${PORT}."
    PIDS=""
fi

for PID in $PIDS; do
    COMMAND="$(ps -o comm= -p "$PID" 2>/dev/null || true)"
    case "$COMMAND" in
        *openheart*)
            echo "[RESTART] Stopping OpenHeart server process ${PID}..."
            kill -TERM "$PID" 2>/dev/null || true
            for _ in 1 2 3 4 5 6 7 8 9 10; do
                kill -0 "$PID" 2>/dev/null || break
                sleep 0.5
            done
            if kill -0 "$PID" 2>/dev/null; then
                echo "[RESTART] Process ${PID} did not exit; sending SIGKILL."
                kill -KILL "$PID" 2>/dev/null || true
            fi
            ;;
        *)
            echo "[RESTART] Port ${PORT} is held by '${COMMAND:-unknown}' (PID ${PID}), not an" \
                 "OpenHeart server; refusing to stop it." >&2
            exit 1
            ;;
    esac
done

if [ -z "$PIDS" ]; then
    echo "[RESTART] No existing server process found listening on port ${PORT}."
fi

echo "[RESTART] Launching OpenHeart Web Server on http://${HOST}:${PORT}..."
exec cargo run -- server --host "${HOST}" --port "${PORT}"
