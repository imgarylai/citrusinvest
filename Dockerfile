# Native backtest server for the Cloudflare Container. Multi-stage: build the
# release binary with the full Rust toolchain, then ship only the binary on a
# slim runtime. Built for linux/amd64 (Cloudflare Containers requirement) — the
# --platform flag is passed by Alchemy at deploy time.
#
# Build context = this directory (the cargo workspace). `-p yuzu-server` compiles
# only the server's dep graph; the wasm crates are skipped.
FROM rust:1-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p yuzu-server

FROM debian:bookworm-slim
# ca-certificates: TLS roots for the HTTPS GETs to R2's S3 endpoint.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/yuzu-server /usr/local/bin/yuzu-server
ENV PORT=8080
EXPOSE 8080
CMD ["/usr/local/bin/yuzu-server"]
