# Multi-stage headless build for Binance Rust Trading Bot (binance-botd + binance-botctl + web-ui).
# Runs as non-root user `binancebot`. Default mode is `paper`; secrets are NEVER baked into the image.

FROM rust:1-bookworm AS rust-builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY protocol ./protocol
RUN cargo build --release --locked --bin binance-botd --bin binance-botctl

FROM node:22-bookworm-slim AS web-builder
WORKDIR /web
COPY web-ui/package.json web-ui/package-lock.json ./
RUN npm ci
COPY web-ui ./
RUN npm run build

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home-dir /var/lib/binance-bot --create-home --shell /usr/sbin/nologin binancebot \
    && install -d -m 0750 -o binancebot -g binancebot /var/lib/binance-bot /var/lib/binance-bot/backup /etc/binance-bot /run/binance-bot \
    && install -d -m 0755 /usr/share/binance-bot/web

COPY --from=rust-builder /src/target/release/binance-botd /usr/local/bin/binance-botd
COPY --from=rust-builder /src/target/release/binance-botctl /usr/local/bin/binance-botctl
COPY --from=web-builder /web/build /usr/share/binance-bot/web
COPY packaging/bot.toml.example /usr/share/binance-bot/bot.toml.example

USER binancebot
WORKDIR /var/lib/binance-bot
VOLUME ["/etc/binance-bot", "/var/lib/binance-bot"]

ENTRYPOINT ["/usr/local/bin/binance-botd", "--config", "/etc/binance-bot/bot.toml"]
