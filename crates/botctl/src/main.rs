#![forbid(unsafe_code)]
//! `binance-botctl` command-line interface binary.

use anyhow::Result;
use botctl::cli::Cli;
use botctl::run_cli;
use clap::Parser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let output = run_cli(cli).await?;
    println!("{output}");
    Ok(())
}
