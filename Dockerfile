# Stage 1: Build the binary
FROM --platform=$BUILDPLATFORM clux/muslrust:stable AS builder

ARG TARGETPLATFORM

# Set a default value for the target environment variable
ENV TARGET_ENV="x86_64-unknown-linux-musl"

# Conditionally set the environment variable based on the platform
RUN --mount=type=cache,target=/root/.cache \
    if [ "$TARGETPLATFORM" = "linux/arm64" ]; then \
        export TARGET_ENV="aarch64-unknown-linux-musl"; \
    fi && echo "TARGET_ENV=$TARGET_ENV"

# Create a non-root user
RUN groupadd -g 10001 -r dockergrp && useradd -r -g dockergrp -u 10001 dockeruser

# Download dependencies ala cargo chef
WORKDIR /app
COPY Cargo.lock Cargo.toml ./
RUN mkdir src && echo "fn main() { print!(\"Dummy main\"); }" > src/main.rs
RUN cargo build --release
RUN rm target/$TARGET_ENV/release/deps/container_retention_policy* && rm -r src

# Build the actual binary
COPY src ./src
RUN cargo build --release
RUN strip target/$TARGET_ENV/release/container-retention-policy

RUN mkdir /build-out && cp target/$TARGET_ENV/release/container-retention-policy /build-out/container-retention-policy

# Stage 2: Create a minimal image
FROM scratch

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /build-out/container-retention-policy /container-retention-policy

USER dockeruser
ENTRYPOINT ["/container-retention-policy"]
