//! backend3: the backend of the Verseghy Ferenc Gimnázium website.
//!
//! One Loco application serves three things on one host (ADR 0014): the admin
//! API under `/api/admin`, the built admin UI under `/admin` (ADR 0004), and
//! the public read API under `/api/v1` (docs/design/public-api.md). The
//! design lives in `docs/design/`; the ADRs there are the specification.
//!
//! Layout (ADR 0008):
//!
//! - [`app`]: the Loco hooks.
//! - [`controllers`]: thin HTTP handlers; they touch `AppContext`.
//! - [`domain`]: pure logic only, with unit tests and proptests.
//! - [`dtos`]: wire types, the API contract (ADR 0005).
//! - [`models`]: Sea-ORM entities and Active Record model logic.
//! - [`settings`]: the typed `settings:` section of the configuration.
//! - [`tasks`], [`workers`]: Loco tasks and background workers (ADR 0016).

pub mod app;
pub mod controllers;
pub mod domain;
pub mod dtos;
pub mod models;
pub mod settings;
pub mod tasks;
pub mod workers;
