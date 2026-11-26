#![forbid(unsafe_code)]
//! `binance-botctl` command-line interface binary.

use std::process::ExitCode;

use botctl::cli::Cli;
use botctl::{ReportFailure, run_cli};
use clap::Parser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run_cli(cli).await {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            if let Some(report) = err.downcast_ref::<ReportFailure>() {
                println!("{}", report.output);
            } else {
                eprintln!("Error: {err:#}");
            }
            ExitCode::FAILURE
        }
    }
}
