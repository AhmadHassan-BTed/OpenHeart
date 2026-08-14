.PHONY: all build check test clean fmt docs ci serve help

# Default target
all: check test

## Build debug binary / library
build:
	cargo build

## Build release binary / library
release:
	cargo build --release

## Check compilation
check:
	cargo check --all-targets

## Run unit and integration tests
test:
	cargo test --all-targets -- --nocapture

## Format codebase according to rustfmt
fmt:
	cargo fmt --all

## Generate crate documentation
docs:
	cargo doc --no-deps --open

## Run local CI simulation script
ci:
	./scripts/ci_check.sh

## Launch Native OpenHeart Web Server (with auto-cleanup)
server:
	./restart_server.sh 8080

## Launch Web Portal Adapter Studio
serve:
	./restart_server.sh 8080

## Clean build target directory
clean:
	cargo clean

## Display help message
help:
	@echo "OpenHeart Makefile Targets:"
	@echo "  make build    - Build debug target"
	@echo "  make release  - Build release target"
	@echo "  make check    - Run cargo check"
	@echo "  make test     - Run cargo test"
	@echo "  make fmt      - Run cargo fmt"
	@echo "  make docs     - Build cargo documentation"
	@echo "  make ci       - Run local CI validation script"
	@echo "  make serve    - Launch Web Portal Adapter Studio on port 8080"
	@echo "  make clean    - Clean build target directory"
