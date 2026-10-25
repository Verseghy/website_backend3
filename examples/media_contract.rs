//! Runs the media contract suite of `tests/media_contract/` from the command
//! line, against a `/storage` server that is already running (ADR 0015 §1).
//!
//! The Test job does not need this: it starts the pinned image itself
//! (`tests/media_contract/nginx.rs`). This is for a server elsewhere, such
//! as the media server pod of the co-mount test (ADR 0015 §3) behind
//! `oc port-forward`:
//!
//! ```text
//! cargo run --locked --example media_contract -- seed <empty directory>
//! cargo run --locked --example media_contract -- check <http://host:port> <nginx|fallback>
//! ```
//!
//! `seed` writes the fixture tree that `check` expects under `/storage`
//! (copy it to the root of the served tree last, keeping modification times
//! and links: `deploy/media/README.md`); `check` prints every failure and
//! exits with 1 if there is any, or with 2 on a usage error, including an
//! origin that it cannot reach, such as an `https` Route. It checks the
//! server itself, never a Route.

// `cargo test --all-targets`, which nextest runs, builds every example as a
// test too. Empty in that build, this crate does not compile the suite's own
// unit tests and container test a second time; the test binary of `tests/`
// runs them.
#![cfg(not(test))]

#[path = "../tests/media_contract/mod.rs"]
mod media_contract;

use std::{path::Path, process::ExitCode, time::Duration};

use media_contract::{HyperTransport, Origin, Server, Transport, run, seed};

const USAGE: &str = "usage:
  media_contract seed <empty directory>
  media_contract check <http://host:port> <nginx|fallback>";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match refs.as_slice() {
        ["seed", directory] => match seed(Path::new(directory)) {
            Ok(()) => {
                println!("wrote the media contract fixture tree to {directory}");
                ExitCode::SUCCESS
            }
            Err(error) => usage_error(&error),
        },
        ["check", origin, server] => {
            let origin = match origin.parse::<Origin>() {
                Ok(origin) => origin,
                Err(error) => return usage_error(&error),
            };
            let server = match parse_server(server) {
                Ok(server) => server,
                Err(error) => return usage_error(&error),
            };
            let transport = HyperTransport::new(Duration::from_secs(10));
            // An origin this transport cannot reach, such as an https Route,
            // is a usage error, not a contract failure.
            if let Err(error) = transport.reaches(&origin) {
                return usage_error(&error);
            }
            let report = run(&transport, &origin, server).await;
            print!("{report}");
            if report.passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        _ => usage_error("unknown command"),
    }
}

/// The server kind named on the command line.
///
/// - `name`: `nginx` or `fallback`.
fn parse_server(name: &str) -> Result<Server, String> {
    match name {
        "nginx" => Ok(Server::Nginx),
        "fallback" => Ok(Server::Fallback),
        other => Err(format!(
            "`{other}` is not a server kind; pass nginx (which must send ETag) or fallback \
             (which need not)"
        )),
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("error: {message}\n{USAGE}");
    ExitCode::from(2)
}
