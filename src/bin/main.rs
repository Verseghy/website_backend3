//! `backend3-cli`: the Loco command-line entry point of backend3.
//!
//! The same binary serves `start`, `db`, `task` and `jobs`, and `cargo loco`
//! runs it too (the `loco` cargo alias is `run --`, and this is the package's
//! `default-run`). Before Loco parses anything, the CLI pre-check of
//! ADR 0024 §4 (layer 3) refuses `db reset`, `db truncate` and
//! `db seed --reset` in production, so those commands stop before any
//! configuration is loaded or any connection is made.

use std::process::ExitCode;

use backend3::{app::App, domain::cli_guard};
use loco_rs::{cli, environment::resolve_from_env};
use migration::Migrator;

/// Exit code of a command refused by the pre-check: the conventional code of
/// a usage error, distinct from the `1` of a failed run.
const EXIT_REFUSED: u8 = 2;

#[tokio::main]
async fn main() -> ExitCode {
    // `args_os` instead of `args`: a non-UTF-8 argument must not panic here.
    // clap reports it properly once Loco parses the arguments.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if let Err(refusal) = cli_guard::check(&args, &resolve_from_env()) {
        eprintln!("Error: {refusal}");
        return ExitCode::from(EXIT_REFUSED);
    }

    match cli::main::<App, Migrator>().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Display, not Debug: the guards' messages are written for
            // operators and must reach them verbatim.
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}
