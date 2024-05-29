# Useful resources:
# - musl minimal image: https://github.com/bjornmolin/rust-minimal-docker/blob/master/Dockerfile
FROM --platform=$BUILDPLATFORM clux/muslrust:stable as builder

RUN groupadd -g 10001 -r dockergrp && useradd -r -g dockergrp -u 10001 dockeruser

# Download dependencies
COPY Cargo.lock .
COPY Cargo.toml .
RUN mkdir src && echo "fn main() {print!(\"Dummy main\");}" > src/main.rs
RUN cargo build --release
RUN rm target/aarch64-unknown-linux-musl/release/deps/container_retention_policy* && rm -r ./src

# Build binary
COPY src ./src
RUN cargo build --release
RUN mkdir /build-out && cp target/aarch64-unknown-linux-musl/release/container-retention-policy /build-out/container-retention-policy

# Strip binary?
#RUN #strip /build-out/container-retention-policy

# Move to minimal image
FROM scratch

LABEL org.opencontainers.image.source="https://github.com/snok/container-retention-policy"
LABEL org.opencontainers.image.description="Container image for deleting old Github packages"
LABEL org.opencontainers.image.licenses="MIT"

COPY --from=0 /etc/passwd /etc/passwd
USER dockeruser
COPY --from=builder /build-out/container-retention-policy /
CMD ["./container-retention-policy"]
