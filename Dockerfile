# ── Stage 1: Build Rust Compiler Engine ──
# The toolchain matches Cargo.toml's rust-version and the runtime image's Debian release,
# so the binary links against the glibc it runs with.
FROM rust:1.88-slim-bookworm AS builder

WORKDIR /usr/src/openheart

# The tree-sitter grammars are C sources compiled by build scripts.
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Cache the dependency graph by building it once against placeholder sources.
# A failure here is a real failure: nothing is masked with `|| true`.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && touch src/lib.rs \
    && cargo build --release --bin openheart \
    && rm -rf src

# Build the real binary. Touching the crate roots makes them newer than the placeholder
# build, so cargo cannot mistake the cached placeholder for an up-to-date binary.
COPY src ./src
RUN touch src/main.rs src/lib.rs \
    && cargo build --release --bin openheart

# ── Stage 2: Runtime Image with Git & Web Studio ──
FROM debian:bookworm-slim

# git and CA certificates serve server-side cloning only. An image that analyzes mounted
# sources can drop them and run with OPENHEART_ALLOW_CLONE=false.
RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 openheart \
    && useradd --system --uid 10001 --gid openheart \
        --home-dir /var/lib/openheart --create-home --shell /usr/sbin/nologin openheart

WORKDIR /app

COPY --from=builder /usr/src/openheart/target/release/openheart /usr/local/bin/openheart
COPY web ./web
COPY examples/test_patterns_codebase ./examples/test_patterns_codebase

# The container listens on all interfaces so the published port works. Because that is a
# non-loopback bind, the server requires an access token: pass OPENHEART_API_TOKEN, or
# read the generated login URL from the container log. The image runs unprivileged and
# keeps all mutable state under /var/lib/openheart, so it also runs with --read-only
# (plus --tmpfs /tmp and a volume on /var/lib/openheart).
ENV OPENHEART_HOST=0.0.0.0 \
    OPENHEART_PORT=8080 \
    OPENHEART_WORKSPACE=/var/lib/openheart/workspace \
    OPENHEART_ALLOW_CLONE=true \
    OPENHEART_LOG=info

USER openheart:openheart

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["openheart", "health"]

# Run OpenHeart server
CMD ["openheart", "server"]
