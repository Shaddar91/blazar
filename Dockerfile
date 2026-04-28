# syntax=docker/dockerfile:1.7
# Blazar — multi-stage build producing a ~8 MB static binary on distroless.
#
# Stage 1: build against musl so the result has no glibc dependency.
# Stage 2: copy the single static binary into a nonroot distroless image.

FROM rust:1-alpine AS build

RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static

WORKDIR /src

# Prime the dependency cache before copying the real sources. This makes
# rebuilds that only touch src/ dramatically faster.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && \
    echo 'fn main() { println!("cache-primer"); }' > src/main.rs && \
    cargo build --release --target x86_64-unknown-linux-musl && \
    rm -rf src target/x86_64-unknown-linux-musl/release/blazar*

COPY src ./src
# COPY preserves the build-context mtime, which can be older than the
# cache-primer compile time and fool cargo's fingerprint check into reusing
# the primer binary. Stamp the real sources to "now" so cargo always rebuilds
# the bin crate from the freshly-copied src tree.
RUN find src -type f -exec touch {} + && \
    cargo build --release --target x86_64-unknown-linux-musl && \
    strip target/x86_64-unknown-linux-musl/release/blazar && \
    test "$(stat -c%s target/x86_64-unknown-linux-musl/release/blazar)" -gt 1000000 || \
        (echo "FATAL: blazar binary is suspiciously small — cache-primer leak" && exit 1)

# ---------------------------------------------------------------------------

FROM gcr.io/distroless/static-debian12:nonroot

COPY --from=build /src/target/x86_64-unknown-linux-musl/release/blazar /blazar

EXPOSE 3030
USER nonroot:nonroot
ENTRYPOINT ["/blazar"]
