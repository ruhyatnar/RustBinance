//! Structured JSON logging initialization for `binance-botd`.

use anyhow::Result;
use tracing_subscriber::EnvFilter;

/// Initialize structured JSON tracing using `RUST_LOG` (defaulting to `info`).
///
/// # Errors
/// Returns an error if a global tracing subscriber has already been installed.
pub fn init_tracing() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init()
        .map_err(|err| anyhow::anyhow!("failed to initialize tracing subscriber: {err}"))
}
