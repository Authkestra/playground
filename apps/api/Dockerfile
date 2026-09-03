# Multi-stage build for the playground API.
#
# Built from the repo root so the workspace manifest and lockfile are in scope:
#   docker build -f apps/api/Dockerfile .

FROM rust:1.95-bookworm AS builder

# aws-lc-rs (our TLS backend, see docs/decisions/0001) compiles C and assembly,
# so the build image needs a C toolchain, cmake and perl. This is the concrete
# reason the TLS choice and the base image are coupled — swapping to
# `rustls-no-provider` would let this drop to a much smaller builder.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake perl pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy manifests first so the dependency layer caches independently of source
# changes — a source-only edit then rebuilds in seconds rather than minutes.
COPY Cargo.toml Cargo.lock ./
COPY apps/api/Cargo.toml apps/api/Cargo.toml
RUN mkdir -p apps/api/src \
    && echo 'fn main() {}' > apps/api/src/main.rs \
    && cargo build --release --locked -p api \
    && rm -rf apps/api/src

COPY apps/api/src apps/api/src
# Cargo skips rebuilding if mtimes look unchanged; touch to force the real build.
RUN touch apps/api/src/main.rs && cargo build --release --locked -p api

FROM debian:bookworm-slim AS runtime

# ca-certificates is required: the engine makes outbound HTTPS calls to identity
# providers, and without a trust store every one of them fails at handshake.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 app

COPY --from=builder /build/target/release/api /usr/local/bin/api

USER app
EXPOSE 8000
ENV PORT=8000
CMD ["/usr/local/bin/api"]
