# Useful resources:
# - musl minimal image: https://github.com/bjornmolin/rust-minimal-docker/blob/master/Dockerfile

FROM clux/muslrust:stable as builder
RUN groupadd -g 10001 -r dockergrp && useradd -r -g dockergrp -u 10001 dockeruser

# Download dependencies
COPY Cargo.lock .
COPY Cargo.toml .
RUN mkdir src && echo "fn main() {print!(\"Dummy main\");}" > src/main.rs
RUN set -x && cargo build --target aarch64-unknown-linux-musl --release
RUN ["/bin/bash", "-c", "set -x && rm target/aarch64-unknown-linux-musl/release/deps/container-retention-policy*"]

# Build binary
COPY src ./src
RUN set -x && cargo build --release
RUN mkdir -p /build-out
RUN set -x && cp target/aarch64-unknown-linux-musl/release/container-retention-policy /build-out/

# Strip binary?
#RUN strip /build-out/container-retention-policy

# Move to minimal image
FROM scratch
COPY --from=0 /etc/passwd /etc/passwd
USER dockeruser
COPY --from=builder /build-out/container-retention-policy /
CMD ["/container-retention-policy"]
