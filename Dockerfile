# Stage 1: Build the binary
FROM --platform=$BUILDPLATFORM clux/muslrust:stable as builder

# Create a non-root user
RUN groupadd -g 10001 -r dockergrp && useradd -r -g dockergrp -u 10001 dockeruser

# Download dependencies
WORKDIR /app
COPY Cargo.lock Cargo.toml ./
RUN mkdir src && echo "fn main() { print!(\"Dummy main\"); }" > src/main.rs
RUN cargo build --release
RUN rm target/aarch64-unknown-linux-musl/release/deps/container_retention_policy* && rm -r src

# Build the actual binary
COPY src ./src
RUN cargo build --release
RUN mkdir /build-out && cp target/aarch64-unknown-linux-musl/release/container-retention-policy /build-out/container-retention-policy

# Stage 2: Create a minimal image
FROM scratch

LABEL org.opencontainers.image.source="https://github.com/snok/container-retention-policy"
LABEL org.opencontainers.image.description="Container image for deleting old Github packages"
LABEL org.opencontainers.image.licenses="MIT"

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /build-out/container-retention-policy /container-retention-policy

USER dockeruser
ENTRYPOINT ["/container-retention-policy"]
