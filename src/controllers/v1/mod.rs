//! The public read API under `/api/v1` (docs/design/public-api.md).
//!
//! - [`openapi`]: the OpenAPI document of the API and its route,
//!   `GET /api/v1/openapi.json` (ADR 0005 §4).
//!
//! The endpoints arrive with the public endpoint slices (ADR 0025 §3,
//! class (c)); each one adds its operation to the document.

pub mod openapi;
