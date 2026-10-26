//! The schema snapshot of ADR 0009: the schema that the migrations create,
//! read from `pg_catalog` and compared with a committed insta snapshot, so
//! that any unplanned change to it fails, and every planned one shows up as a
//! reviewed diff of `snapshots/schema.snap` in the PR that makes it.
//!
//! [`catalog`] reads the database and lists what it covers; the text form,
//! and the reasons for it, are in `backend3::domain::schema_snapshot`.
//! [`guard`] fails the snapshot test when the database holds anything that
//! the reader does not show, so that the snapshot covers the whole schema
//! within the limits that `guard.rs` lists;
//! [`guard_cases`] tests it with one case per kind of object.

mod catalog;
mod guard;
mod guard_cases;
mod snapshot;
