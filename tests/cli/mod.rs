//! The code-level guards of ADR 0024 §4 (layer 3) and the settings check of
//! `App::load_config`, end to end through the built `backend3-cli` binary.
//!
//! Each test points the database URI at a [`ConnectionProbe`] and checks
//! afterwards whether the process tried to connect. A positive control in
//! each module shows that the probe does see a connection when no guard
//! stops the command, so "no connection" really means "stopped before
//! connecting".
//!
//! [`ConnectionProbe`]: crate::support::ConnectionProbe

mod database_guard;
// The legacy import exists only with its Cargo feature (data-migration.md §3.3).
#[cfg(feature = "import-mysql")]
mod import_mysql;
mod metrics_port;
mod png_effort;
mod precheck;
mod session_secret;
