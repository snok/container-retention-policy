# Stage 1: Build the binary
FROM rust:1-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN groupadd -g 10001 -r dockergrp && useradd -r -g dockergrp -u 10001 dockeruser

# Download dependencies ala cargo chef
WORKDIR /app
COPY Cargo.lock Cargo.toml ./
RUN mkdir src && echo "fn main() { print!(\"Dummy main\"); }" > src/main.rs
RUN cargo build --release
RUN rm -rf target/release/deps/container_retention_policy* src

# Build the actual binary
COPY src ./src
RUN cargo build --release
RUN strip target/release/container-retention-policy

RUN mkdir /build-out && cp target/release/container-retention-policy /build-out/container-retention-policy

# Stage 2: Playwright runtime
FROM mcr.microsoft.com/playwright:v1.60.0-noble

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /build-out/container-retention-policy /container-retention-policy

USER dockeruser
ENTRYPOINT ["/container-retention-policy"]
