//! Wire types: every type that crosses the HTTP boundary (ADR 0005).
//!
//! DTOs are the API contract. Admin API types live in [`admin`] and derive
//! `ts_rs::TS`, which writes their TypeScript bindings into
//! `admin/src/bindings/` when the test suite runs; the CI `Contract` job fails
//! when the committed bindings differ. Public API types live in [`v1`]; they
//! derive `utoipa::ToSchema` or `utoipa::IntoParams`, and the OpenAPI document
//! of `crate::controllers::v1::openapi` describes them (ADR 0005 §2, and the
//! rules the spike found in `docs/design/openapi-spike.md`). Each API has its
//! own error body: [`admin::error`] and [`v1::error`]. Sea-ORM entities never
//! serialize to a client; handlers convert them into DTOs.

pub mod admin;
pub mod v1;
