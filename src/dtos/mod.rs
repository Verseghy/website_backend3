//! Wire types: every type that crosses the HTTP boundary (ADR 0005).
//!
//! DTOs are the API contract. Admin API types live in [`admin`] and derive
//! `ts_rs::TS`, which writes their TypeScript bindings into
//! `admin/src/bindings/` when the test suite runs; the CI `Contract` job fails
//! when the committed bindings differ. Public API types (`v1`) arrive with the
//! utoipa spike of ADR 0005 §5. Sea-ORM entities never serialize to a client;
//! handlers convert them into DTOs.

pub mod admin;
