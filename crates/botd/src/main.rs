#![forbid(unsafe_code)]
//! Binance Rust trading daemon (headless engine).

mod logging;

use anyhow::Result;

fn main() -> Result<()> {
    logging::init_tracing()?;
    tracing::info!(
        mode = "paper",
        version = env!("CARGO_PKG_VERSION"),
        "binance-botd skeleton initialized"
    );
    Ok(())
}
