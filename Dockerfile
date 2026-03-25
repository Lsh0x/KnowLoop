# =============================================================================
# Stage 1: Build the backend (Rust)
# =============================================================================
# Trixie (Debian 13) provides glibc 2.40, required because ort-sys prebuilt
# ONNX Runtime binaries reference __isoc23_* symbols (glibc >= 2.38).
# Bookworm (glibc 2.36) fails with "undefined symbol: __isoc23_strtoll".
FROM rust:1.93-trixie AS builder

WORKDIR /app

# Install build dependencies (no libssl-dev — project uses rustls, not OpenSSL)
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests for dependency caching (Cargo.lock required for reproducible builds)
COPY Cargo.toml Cargo.lock ./

# Copy local crates (path dependencies referenced in Cargo.toml)
COPY crates ./crates/

# Create dummy source files to build dependencies.
# This must match the actual module layout so the dep-cache layer is valid.
# Module directories (16):
#   api, auth, chat, embeddings, events, graph, mcp, meilisearch,
#   neo4j, neurons, notes, orchestrator, parser, plan, resolver, skills
# Single files: setup_claude.rs, update.rs
# Binary: bin/mcp_server.rs
RUN mkdir -p src/bin \
             src/api src/auth src/chat src/embeddings src/events \
             src/graph src/mcp src/meilisearch src/neo4j src/neurons \
             src/notes src/orchestrator src/parser src/plan \
             src/resolver src/skills && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/cli.rs && \
    echo "fn main() {}" > src/bin/mcp_server.rs && \
    echo "pub mod api; pub mod auth; pub mod chat; pub mod embeddings; pub mod events; pub mod graph; pub mod mcp; pub mod meilisearch; pub mod neo4j; pub mod neurons; pub mod notes; pub mod orchestrator; pub mod parser; pub mod plan; pub mod resolver; pub mod setup_claude; pub mod skills; pub mod update;" > src/lib.rs && \
    echo "" > src/setup_claude.rs && \
    echo "" > src/update.rs && \
    for dir in api auth chat embeddings events graph mcp meilisearch neo4j neurons notes orchestrator parser plan resolver skills; do \
        echo "" > "src/$dir/mod.rs"; \
    done

# Build dependencies only
RUN cargo build --release 2>/dev/null || true

# Remove dummy source
RUN rm -rf src

# Copy actual source code
COPY src ./src

# Copy tree-sitter queries if they exist (optional — may not be present)
COPY querie[s] ./queries/

# Touch source files to trigger rebuild
RUN find src -name "*.rs" -exec touch {} \;

# Build the actual application
RUN cargo build --release

# =============================================================================
# Stage 2: Runtime image
# =============================================================================
# Must match builder glibc version (Trixie = glibc 2.40) so dynamically-linked
# symbols like __isoc23_strtoll resolve correctly at runtime.
FROM debian:trixie-slim AS runtime

# OCI labels
LABEL org.opencontainers.image.source="https://github.com/Lsh0x/KnowLoop"
LABEL org.opencontainers.image.description="AI agent orchestrator with Neo4j knowledge graph, Meilisearch, and Tree-sitter"
LABEL org.opencontainers.image.licenses="MIT"

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy all 3 binaries
COPY --from=builder /app/target/release/knowloop /app/knowloop
COPY --from=builder /app/target/release/kl /app/kl
COPY --from=builder /app/target/release/knowloop_mcp /app/knowloop_mcp

# Copy tree-sitter queries if they exist (optional)
COPY querie[s] ./queries/

# Provide a minimal config.yaml so setup_completed=true (env vars handle the rest).
# Without this file, the server enters setup-wizard mode because the Default trait
# sets setup_completed=false when no config file is found on disk.
COPY config.yaml.docker ./config.yaml

# Create data directory
RUN mkdir -p /data

ENV RUST_LOG=info
ENV WORKSPACE_PATH=/workspace

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["./knowloop", "serve"]
