#!/usr/bin/env bash
set -e

# Change directory to the repository root where this script resides
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PORT="${1:-8080}"

echo "================================================================================"
echo "  OpenHeart — Precision Program Graph Studio"
echo "================================================================================"
echo ""

# Helper function to open browser across macOS, Linux, and WSL
open_browser() {
    local url="$1"
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$url" >/dev/null 2>&1 &
    elif command -v open >/dev/null 2>&1; then
        open "$url" >/dev/null 2>&1 &
    elif command -v cmd.exe >/dev/null 2>&1; then
        cmd.exe /c start "$url" >/dev/null 2>&1 &
    else
        echo "[OpenHeart] Open your browser and navigate to: $url"
    fi
}

# 1. Check if Cargo/Rust is installed
if command -v cargo >/dev/null 2>&1; then
    echo "[OpenHeart] Rust/Cargo detected. Launching native OpenHeart server on port ${PORT}..."
    echo "[OpenHeart] URL: http://localhost:${PORT}"
    (sleep 1 && open_browser "http://localhost:${PORT}") &
    exec cargo run -- server "${PORT}"
fi

# 2. Check if Python is installed
PYTHON_CMD=""
if command -v python3 >/dev/null 2>&1; then
    PYTHON_CMD="python3"
elif command -v python >/dev/null 2>&1; then
    PYTHON_CMD="python"
fi

if [ -n "$PYTHON_CMD" ]; then
    echo "[OpenHeart] ${PYTHON_CMD} detected. Launching OpenHeart Web Studio on port ${PORT}..."
    echo "[OpenHeart] URL: http://localhost:${PORT}"
    (sleep 1 && open_browser "http://localhost:${PORT}") &
    exec "$PYTHON_CMD" -m http.server "${PORT}" --directory web
fi

# 3. Direct browser fallback if neither Cargo nor Python is found
echo "[OpenHeart] Neither Cargo nor Python found. Opening OpenHeart Web Studio directly in browser..."
open_browser "$SCRIPT_DIR/web/index.html"
