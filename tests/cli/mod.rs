//! The code-level guards of ADR 0024 §4 (layer 3), end to end through the
//! built `backend3-cli` binary.
//!
//! Each test points the database URI at a [`ConnectionProbe`] and checks
//! afterwards whether the process tried to connect. A positive control in
//! each module shows that the probe does see a connection when no guard
//! stops the command, so "no connection" really means "stopped before
//! connecting".
//!
//! [`ConnectionProbe`]: crate::support::ConnectionProbe

mod database_guard;
mod precheck;
