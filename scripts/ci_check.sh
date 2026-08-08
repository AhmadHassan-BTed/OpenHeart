#!/usr/bin/env bash

set -euo pipefail

echo "=========================================="
echo "      OpenHeart Pre-Flight CI Check       "
echo "=========================================="

echo "[1/3] Checking Cargo Compilation..."
cargo check --all-targets

echo "[2/3] Running Cargo Formatting Check..."
cargo fmt --all -- --check

echo "[3/3] Running Test Suite..."
cargo test --all-targets -- --nocapture

echo "=========================================="
echo "    SUCCESS: All local CI checks passed!  "
echo "=========================================="
