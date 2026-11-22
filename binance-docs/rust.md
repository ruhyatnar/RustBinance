# Rust

The official Rust connector for Binance APIs, designed for backend applications and services written
in Rust.

The connector is published as a single crate (`binance-sdk`). You enable the Binance products you
need using Cargo features (for example `spot`). It supports REST APIs, WebSocket APIs and WebSocket
Streams, and provides built-in request signing, connection management, and strongly typed request
and response models.

For source code, issues, and release notes, see the
[binance-connector-rust](https://github.com/binance/binance-connector-rust) repository on GitHub.

## Supported environments

- Rust 1.86 or newer (stable toolchain)
- Backend Rust applications and services

> This connector is intended for server-side usage only.

## Package structure

The Rust connector is published as a single crate with product modules behind feature flags.

Enable the products you want in `Cargo.toml` (for example `spot`,
`derivatives_trading_usds_futures`, `wallet` etc.), then import the corresponding module in your
code. This keeps dependencies minimal and avoids pulling in APIs you don’t use.

## Typical use cases

- Low-latency trading and execution services
- High-throughput market data ingestion
- Backend services managing accounts, balances, and orders
- Applications consuming WebSocket APIs or WebSocket Streams for real-time updates

## Key features

- Support for Binance REST APIs, WebSocket APIs and WebSocket Streams
- Built-in request signing for authenticated endpoints
- WebSocket connection handling with automatic reconnection and session renewal
- Strongly typed request and response models
- Support for both production and testnet environments
- Memory-safe and performant Rust API design

## Getting started

Enable the product feature you want under `binance-sdk`:

```toml
[dependencies]
binance-sdk = { version = "<VERSION>", features = ["spot"] }
```

Create a client and send a request:

```rust
use anyhow::{Context, Result};
use std::env;

use binance_sdk::config::{ConfigurationRestApi, PrivateKey};
use binance_sdk::spot::{SpotRestApi, rest_api::GetAccountParams};

#[tokio::main]
async fn main() -> Result<()> {
    let api_key = env::var("BINANCE_API_KEY").context("BINANCE_API_KEY must be set")?;
    let private_key = env::var("BINANCE_PRIVATE_KEY").context("BINANCE_PRIVATE_KEY must be set")?;
    let configuration = ConfigurationRestApi::builder()
        .api_key(api_key)
        .private_key(PrivateKey::File(private_key))
        .build()?;

    let spot_client = SpotRestApi::production(configuration);

    let params = GetAccountParams::default();
    let account_info = spot_client
        .get_account(params)
        .await
        .context("get_account request failed")?;

    Ok(())
}
```

The example above uses asymmetric key authentication. HMAC-based authentication using an API secret
is also supported.

Refer to the crate documentation for product-specific REST, WebSocket API and WebSocket Streams
examples.

## Notes and best practices

- Store API keys securely using environment variables or a secrets manager
- Prefer WebSocket APIs or Streams for real-time data instead of polling REST endpoints
- Monitor rate limits and endpoint weights when building high-throughput services
- Use testnet environments when developing and validating integrations
