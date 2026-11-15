//! Tests of the import normaliser through its public API
//! (data-migration.md §7.1, §12).
//!
//! The unit tests and proptests of each step, P4 (idempotence) among them,
//! live next to the code in `src/domain/normalise/`. These tests run the
//! hand-written before-and-after fixtures of `tests/fixtures/normalise/`,
//! one or more per rule, which reproduce the production shapes the
//! normaliser meets. Reading them needs the file system, so the runner
//! cannot live in the pure `src/domain/` (ADR 0008).

mod cost;
mod fixtures;
