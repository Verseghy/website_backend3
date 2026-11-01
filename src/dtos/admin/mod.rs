//! Wire types of the admin API under `/api/admin` (ADR 0005, ADR 0021 §4).
//!
//! Every type here derives `ts_rs::TS` with
//! `#[ts(export, export_to = "../admin/src/bindings/")]`, so the admin UI gets
//! a compile-checked TypeScript copy. 64-bit integer fields carry
//! `#[ts(type = "number")]` (ADR 0005 §3).

pub mod auth;
pub mod content;
pub mod error;
