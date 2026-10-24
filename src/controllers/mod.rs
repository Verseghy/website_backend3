//! HTTP handlers (ADR 0008): thin, they parse, call a model or a domain
//! function, and render a DTO.
//!
//! - [`admin_ui`]: the `/admin` static router (ADR 0004 §3).
//! - [`not_found`]: the fallback for unknown paths, with the JSON `404` of
//!   `/api/admin` and `/api/v1`.
//! - [`site`]: `/` and `/robots.txt` (ADR 0014).
//!
//! The admin API (`admin/`) and the public API (`v1/`) arrive with the
//! vertical slices.

pub mod admin_ui;
pub mod not_found;
pub mod site;
