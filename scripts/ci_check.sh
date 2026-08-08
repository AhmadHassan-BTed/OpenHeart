#!/usr/bin/env bash

set -euo pipefail

echo "=========================================="
echo "      OpenHeart Pre-Flight CI Check       "
echo "=========================================="

echo "[1/4] Checking Cargo Compilation..."
cargo check --all-targets

echo "[2/4] Running Cargo Formatting Check..."
cargo fmt --all -- --check

echo "[3/4] Running Clippy Linter..."
cargo clippy --all-targets -- -D warnings

echo "[4/4] Running Test Suite..."
cargo test --all-targets -- --nocapture

echo "=========================================="
echo "    SUCCESS: All local CI checks passed!  "
echo "=========================================="
