//! HTTP handlers (ADR 0008): thin, they parse, call a model or a domain
//! function, and render a DTO.
//!
//! - [`admin`]: the admin API under `/api/admin`, every response
//!   `Cache-Control: no-store` (ADR 0023 §6).
//! - [`admin_ui`]: the `/admin` static router (ADR 0004 §3).
//! - [`metrics`]: `/metrics` on its own port, and the HTTP request metrics
//!   (ADR 0023 §9).
//! - [`not_found`]: the fallback for unknown paths, with the JSON `404` of
//!   `/api/admin`, `/api/v1` and backend1's retired REST routes.
//! - [`site`]: `/` and `/robots.txt` (ADR 0014).
//! - [`v1`]: the public read API under `/api/v1`: the one builder of its
//!   routes, with CORS (ADR 0014), and so far its OpenAPI document
//!   (ADR 0005 §4).
//!
//! The other admin endpoints and the public endpoints arrive with their
//! slices.

pub mod admin;
pub mod admin_ui;
pub mod metrics;
pub mod not_found;
pub mod site;
pub mod v1;
