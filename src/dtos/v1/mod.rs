//! Wire types of the public read API under `/api/v1` (ADR 0005,
//! public-api.md).
//!
//! Every type here derives `utoipa::ToSchema` (responses and bodies) or
//! `utoipa::IntoParams` (query and path parameters), and serde's names are
//! the only names (ADR 0005 §1-§2). The rules the utoipa spike found for
//! these types, such as `#[schema(required = true)]` on every `Option`
//! field, are in docs/design/openapi-spike.md §3.
//!
//! - [`error`]: the `Error` body of every `4xx` and `5xx` (public-api.md
//!   §6.15, §8).
//!
//! The response types arrive with the public endpoint slices (ADR 0025 §3,
//! class (c)).

pub mod error;
