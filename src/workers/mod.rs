//! Background workers on Loco's Postgres queue (ADR 0016 §3).
//!
//! `cargo loco generate worker` appends its module here and registers it in
//! `App::connect_workers`. Every worker is idempotent and takes small
//! arguments. A worker that works on a row records its outcome there; a
//! worker whose work is files has no such row, so its outcome is the state
//! of the files, a failure leaves only orphans, and the recovery is
//! `jobs retry` (ADR 0016 §3, ADR 0011 §6; `AGENTS.md`, "Queue workers").
//!
//! - [`delete_owned_files`]: `DeleteOwnedFiles`, which removes the owned
//!   files that a purge left unreferenced (ADR 0011 §6).

pub mod delete_owned_files;
