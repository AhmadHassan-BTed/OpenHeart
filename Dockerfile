# ── Stage 1: Build Rust Compiler Engine ──
FROM rust:1.80-slim as builder

WORKDIR /usr/src/openheart

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests for caching
COPY Cargo.toml Cargo.lock ./

# Create dummy source to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release || true && rm -rf src

# Copy full source tree
COPY src ./src
COPY tests ./tests

# Build optimized release binary
RUN cargo build --release --bin openheart

# ── Stage 2: Runtime Image with Git & Web Studio ──
FROM debian:bookworm-slim

# Install git and ca-certificates for dynamic repository cloning
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy compiled binary from builder
COPY --from=builder /usr/src/openheart/target/release/openheart /usr/local/bin/openheart

# Copy web UI assets and sample diagrams
COPY web ./web

# Set environment defaults
ENV PORT=8080
ENV HOST=0.0.0.0

EXPOSE 8080

# Run OpenHeart server
CMD ["openheart", "serve", "--port", "8080"]
