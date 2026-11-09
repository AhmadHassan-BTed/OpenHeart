# ==============================================================================
# OpenHeart Makefile — local entry points for build, quality gates and release
# ==============================================================================
# Every gate CI is expected to enforce has a target here, so contributors run the
# exact same command locally before pushing. `make help` lists all targets; the
# list is generated from the `## ` comment directly above each target.

SHELL := bash
.DEFAULT_GOAL := all

CARGO ?= cargo
# Extra flags for every cargo build, check, test, lint and doc invocation. --locked
# is the default, locally exactly as in CI: a Cargo.lock that disagrees with
# Cargo.toml fails the command instead of being silently re-resolved. After editing
# [dependencies], run `make lock` once and commit the refreshed lockfile;
# `make <target> CARGO_FLAGS=` opts out for a single command.
CARGO_FLAGS ?= --locked
# Port used by the server and studio launchers.
PORT ?= 8080
# Port `make studio-smoke` starts its throwaway server on. It must be free: the
# target refuses to run when anything already answers there and never stops it.
SMOKE_PORT ?= 18080
# Test-name filter for the memory-safety jobs (Miri, AddressSanitizer).
UNSAFE_FILTER ?= psa::
# Miri needs host file access because the artifact serializers round-trip through
# temporary files.
MIRIFLAGS ?= -Zmiri-disable-isolation
# Optional line-coverage floor in percent for `make coverage`; unset means report only.
COVERAGE_MIN ?=
DOCKER_IMAGE ?= openheart:$(VERSION)
DIST_DIR ?= dist
# Honour a relocated cargo target directory.
TARGET_DIR := $(or $(CARGO_TARGET_DIR),target)

# Interpreters and linters for the Python harness and web-studio gates. Windows
# installs the interpreter as `python`; `python3` there is the Store launcher stub.
PYTHON ?= $(if $(filter Windows_NT,$(OS)),python,python3)
NODE ?= node
RUFF ?= ruff
# Rules added to ruff's defaults (which include F401, unused imports): blind
# `except Exception` (BLE001), subprocess with shell=True (S602) and hard-coded
# temporary paths such as /tmp (S108).
RUFF_EXTEND_SELECT ?= BLE001,S602,S108
# Every Python harness script, and every web-studio ES module (the generated
# diagram data under web/diagrams is excluded).
PY_SOURCES = $(sort $(wildcard *.py scripts/*.py scripts/*/*.py))
JS_SOURCES = $(sort $(wildcard web/*.js web/js/*.js web/js/*/*.js))

BIN_NAME := openheart
# Declared once in Cargo.toml and read back here, so the MSRV job and the release
# archive name can never drift from the manifest.
VERSION := $(shell sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
MSRV := $(shell sed -n 's/^rust-version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
# Evaluated lazily: only the targets that need the host triple invoke rustc.
HOST_TARGET = $(shell rustc -vV 2>/dev/null | sed -n 's/^host: //p')
EXE = $(if $(findstring windows,$(HOST_TARGET)),.exe,)

.PHONY: all build release lock check test nextest fmt fmt-check lint doc docs ci \
        ci-full ci-scripts py-check py-lint js-check msrv miri asan deny machete \
        coverage profile-build docker docker-smoke studio-smoke dist run server \
        serve clean help

## Default: compile every target, then run the full test suite
all: check test

## Build the debug binary and library
build:
	$(CARGO) build $(CARGO_FLAGS)

## Build the optimised release binary and library
release:
	$(CARGO) build --release $(CARGO_FLAGS)

## Re-resolve Cargo.lock after editing Cargo.toml (unrelated locked versions stay put)
lock:
	$(CARGO) update --workspace

## Type-check every target, then the library alone without default features (no server)
check:
	$(CARGO) check --all-targets $(CARGO_FLAGS)
	$(CARGO) check --lib --no-default-features $(CARGO_FLAGS)

## Run unit and integration tests, then doctests (`--all-targets` skips doctests)
test:
	$(CARGO) test --all-targets $(CARGO_FLAGS) -- --nocapture
	$(CARGO) test --doc $(CARGO_FLAGS)

## Run tests with cargo-nextest (per-test isolation and timeouts), then doctests
nextest:
	$(CARGO) nextest run --all-targets $(CARGO_FLAGS)
	$(CARGO) test --doc $(CARGO_FLAGS)

## Format the codebase with rustfmt
fmt:
	$(CARGO) fmt --all

## Fail if any file is not rustfmt-formatted
fmt-check:
	$(CARGO) fmt --all -- --check

## Run clippy on every target and treat each warning as an error
lint:
	$(CARGO) clippy --all-targets $(CARGO_FLAGS) -- -D warnings

## Build crate documentation and treat rustdoc warnings as errors
doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --no-deps $(CARGO_FLAGS)

## Build crate documentation and open it in a browser
docs:
	$(CARGO) doc --no-deps --open $(CARGO_FLAGS)

## Pull-request gates: formatting, compilation, clippy, tests and rustdoc
ci: fmt-check check lint test doc

## Pull-request gates plus MSRV, supply-chain, unused-dependency and script checks
ci-full: ci msrv deny machete ci-scripts

## Static gates for the Python harness and the web studio (no Rust toolchain needed)
ci-scripts: py-check py-lint js-check

## Compile every Python harness script; a syntax error fails (no .pyc files are written)
py-check:
	@if [ -z "$(strip $(PY_SOURCES))" ]; then \
	  echo "[py-check] no Python sources found; nothing to compile"; \
	  exit 0; \
	fi; \
	command -v $(PYTHON) >/dev/null 2>&1 || { echo "[py-check] '$(PYTHON)' not found; set PYTHON=<interpreter>" >&2; exit 1; }; \
	$(PYTHON) -c 'import pathlib, sys; [compile(pathlib.Path(p).read_bytes(), p, "exec", dont_inherit=True) for p in sys.argv[1:]]; print("[py-check] compiled", len(sys.argv) - 1, "file(s)")' $(PY_SOURCES)

## Lint the Python harness with ruff (unused imports, blind excepts, shell=True, /tmp paths)
py-lint:
	@if [ -z "$(strip $(PY_SOURCES))" ]; then \
	  echo "[py-lint] no Python sources found; nothing to lint"; \
	  exit 0; \
	fi; \
	command -v $(RUFF) >/dev/null 2>&1 || { echo "[py-lint] '$(RUFF)' not found; install it with 'pip install ruff'" >&2; exit 1; }; \
	$(RUFF) check --extend-select $(RUFF_EXTEND_SELECT) $(PY_SOURCES)

## Syntax-check every web-studio ES module with node (no npm packages needed)
js-check:
	@set -eu; \
	if [ -z "$(strip $(JS_SOURCES))" ]; then \
	  echo "[js-check] no web-studio modules found; nothing to check"; \
	  exit 0; \
	fi; \
	command -v $(NODE) >/dev/null 2>&1 || { echo "[js-check] '$(NODE)' not found; set NODE=<node binary>" >&2; exit 1; }; \
	failed=0; \
	for file in $(JS_SOURCES); do \
	  if ! $(NODE) --input-type=module --check < "$$file"; then \
	    echo "[js-check] syntax error in $$file" >&2; \
	    failed=1; \
	  fi; \
	done; \
	if [ "$$failed" -ne 0 ]; then exit 1; fi; \
	echo "[js-check] parsed $(words $(JS_SOURCES)) module(s)"

## Check the crate on the minimum supported Rust version declared in Cargo.toml
msrv:
	@if [ -z "$(MSRV)" ]; then \
	  echo "[msrv] Cargo.toml declares no rust-version; there is nothing to check against" >&2; \
	  exit 1; \
	fi
	rustup toolchain install $(MSRV) --profile minimal --no-self-update
	$(CARGO) +$(MSRV) check --all-targets --locked

## Run the unsafe-code tests under Miri (nightly) to detect undefined behaviour
miri:
	rustup toolchain install nightly --profile minimal --component miri,rust-src --no-self-update
	MIRIFLAGS="$(MIRIFLAGS)" $(CARGO) +nightly miri test --lib $(CARGO_FLAGS) $(UNSAFE_FILTER)

## Run the unsafe-code tests under AddressSanitizer (nightly)
asan:
	@if [ -z "$(HOST_TARGET)" ]; then \
	  echo "[asan] could not determine the host target triple from 'rustc -vV'" >&2; \
	  exit 1; \
	fi
	rustup toolchain install nightly --profile minimal --no-self-update
	RUSTFLAGS="-Zsanitizer=address" RUSTDOCFLAGS="-Zsanitizer=address" \
	  $(CARGO) +nightly test --lib --target $(HOST_TARGET) --target-dir $(TARGET_DIR)/asan $(CARGO_FLAGS) $(UNSAFE_FILTER)

## Audit dependencies with cargo-deny (advisories, bans, sources; licenses once deny.toml exists)
deny:
	$(CARGO) deny check $(if $(wildcard deny.toml),,advisories bans sources)

## Report dependencies declared in Cargo.toml but never used (cargo-machete)
machete:
	$(CARGO) machete

## Measure line coverage with cargo-llvm-cov, write lcov.info and print a summary
coverage:
	$(CARGO) llvm-cov clean --workspace
	$(CARGO) llvm-cov --all-targets $(CARGO_FLAGS) --no-report
	$(CARGO) llvm-cov report --lcov --output-path lcov.info
	$(CARGO) llvm-cov report --summary-only $(if $(COVERAGE_MIN),--fail-under-lines $(COVERAGE_MIN),)

## Build with release code generation plus debug symbols, for profilers
profile-build:
	$(CARGO) build --profile profiling $(CARGO_FLAGS)

## Build the Docker image (tag: DOCKER_IMAGE)
docker:
	docker build -t $(DOCKER_IMAGE) .

## Build the Docker image, start it with its default command and probe /api/health
docker-smoke: docker
	@set -eu; \
	cid=$$(docker run -d -p $(PORT):8080 $(DOCKER_IMAGE)); \
	trap 'docker rm -f "$$cid" >/dev/null 2>&1' EXIT; \
	for attempt in $$(seq 1 30); do \
	  if curl -fsS "http://127.0.0.1:$(PORT)/api/health" >/dev/null 2>&1; then \
	    echo "[docker-smoke] $(DOCKER_IMAGE) answered /api/health after $$attempt attempt(s)"; \
	    exit 0; \
	  fi; \
	  if [ "$$(docker inspect -f '{{.State.Running}}' "$$cid")" != "true" ]; then \
	    echo "[docker-smoke] container exited before serving /api/health; logs follow" >&2; \
	    docker logs "$$cid" >&2; \
	    exit 1; \
	  fi; \
	  sleep 1; \
	done; \
	echo "[docker-smoke] /api/health did not answer within 30 s; logs follow" >&2; \
	docker logs "$$cid" >&2; \
	exit 1

## Build the release server, start it on SMOKE_PORT and fetch /api/health, / and every studio module
studio-smoke:
	$(CARGO) build --release $(CARGO_FLAGS) --bin $(BIN_NAME)
	@set -eu; \
	base="http://127.0.0.1:$(SMOKE_PORT)"; \
	if curl -s -o /dev/null --max-time 2 "$$base/"; then \
	  echo "[studio-smoke] something already answers on port $(SMOKE_PORT); choose a free one with SMOKE_PORT=<port>" >&2; \
	  exit 1; \
	fi; \
	log="$$(mktemp)"; \
	env -u OPENHEART_API_TOKEN "$(TARGET_DIR)/release/$(BIN_NAME)$(EXE)" server \
	  --host 127.0.0.1 --port $(SMOKE_PORT) --web-root web --no-clone >"$$log" 2>&1 & \
	pid=$$!; \
	trap 'kill -TERM "$$pid" 2>/dev/null || true; wait "$$pid" 2>/dev/null || true; rm -f "$$log"' EXIT; \
	ready=0; \
	for attempt in $$(seq 1 60); do \
	  if curl -fsS -o /dev/null --max-time 2 "$$base/api/health" 2>/dev/null; then \
	    ready=1; \
	    break; \
	  fi; \
	  if ! kill -0 "$$pid" 2>/dev/null; then break; fi; \
	  sleep 0.5; \
	done; \
	if [ "$$ready" -ne 1 ]; then \
	  echo "[studio-smoke] the server did not answer /api/health within 30 s; its output follows" >&2; \
	  cat "$$log" >&2; \
	  exit 1; \
	fi; \
	failed=0; \
	for path in / $(patsubst web/%,/%,$(JS_SOURCES)); do \
	  if ! curl -fsS -o /dev/null --max-time 10 "$$base$$path"; then \
	    echo "[studio-smoke] GET $$path failed" >&2; \
	    failed=1; \
	  fi; \
	done; \
	if [ "$$failed" -ne 0 ]; then \
	  echo "[studio-smoke] server output follows" >&2; \
	  cat "$$log" >&2; \
	  exit 1; \
	fi; \
	echo "[studio-smoke] /api/health, / and $(words $(JS_SOURCES)) studio module(s) served on port $(SMOKE_PORT)"

## Package the release binary, web studio and licence into dist/ with a SHA-256 file
dist:
	@if [ -z "$(HOST_TARGET)" ] || [ -z "$(VERSION)" ]; then \
	  echo "[dist] cannot name the archive: host target='$(HOST_TARGET)' version='$(VERSION)'" >&2; \
	  exit 1; \
	fi
	$(CARGO) build --release --locked --bin $(BIN_NAME)
	@set -eu; \
	pkg="$(BIN_NAME)-$(VERSION)-$(HOST_TARGET)"; \
	stage="$(DIST_DIR)/$$pkg"; \
	rm -rf "$$stage" "$$stage.tar.gz" "$$stage.tar.gz.sha256"; \
	mkdir -p "$$stage"; \
	cp "$(TARGET_DIR)/release/$(BIN_NAME)$(EXE)" "$$stage/"; \
	cp README.md LICENSE CHANGELOG.md "$$stage/"; \
	cp -R web "$$stage/web"; \
	(cd "$(DIST_DIR)" && tar -czf "$$pkg.tar.gz" "$$pkg"); \
	rm -rf "$$stage"; \
	if command -v sha256sum >/dev/null 2>&1; then \
	  (cd "$(DIST_DIR)" && sha256sum "$$pkg.tar.gz" > "$$pkg.tar.gz.sha256"); \
	elif command -v shasum >/dev/null 2>&1; then \
	  (cd "$(DIST_DIR)" && shasum -a 256 "$$pkg.tar.gz" > "$$pkg.tar.gz.sha256"); \
	else \
	  echo "[dist] neither sha256sum nor shasum is available; removing the unchecksummed archive" >&2; \
	  rm -f "$$stage.tar.gz"; \
	  exit 1; \
	fi; \
	echo "[dist] $$stage.tar.gz"; \
	echo "[dist] $$stage.tar.gz.sha256"

## Launch OpenHeart Studio (auto-detects the engine and opens a browser) on PORT
run:
	./run.sh $(PORT)

## Restart the native OpenHeart web server on PORT (only an earlier OpenHeart server is stopped)
server:
ifeq ($(OS),Windows_NT)
	@echo "[server] scripts/restart_server.sh needs lsof and ps, which Windows lacks, so no earlier"
	@echo "[server] server is stopped; if port $(PORT) is taken the bind fails and says so."
	$(CARGO) run --release $(CARGO_FLAGS) --bin $(BIN_NAME) -- server --port $(PORT)
else
	./scripts/restart_server.sh $(PORT)
endif

## Alias of `server`
serve: server

## Remove cargo build output, release archives and coverage reports
clean:
	$(CARGO) clean
	rm -rf "$(DIST_DIR)" lcov.info

## List all targets with their descriptions
help:
	@echo "OpenHeart Makefile targets:"
	@awk '/^## / { desc = substr($$0, 4); next } /^[A-Za-z0-9_.-]+:/ && desc != "" { name = $$1; sub(/:.*/, "", name); printf "  make %-14s %s\n", name, desc } { desc = "" }' $(firstword $(MAKEFILE_LIST))
	@echo ""
	@echo "Variables: PORT=$(PORT) SMOKE_PORT=$(SMOKE_PORT) CARGO_FLAGS='$(CARGO_FLAGS)' UNSAFE_FILTER=$(UNSAFE_FILTER) COVERAGE_MIN='$(COVERAGE_MIN)' DOCKER_IMAGE=$(DOCKER_IMAGE) PYTHON=$(PYTHON)"
