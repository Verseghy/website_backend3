//! Pure logic of backend3 (ADR 0008).
//!
//! Loco's doctrine puts logic on models. backend3 overrides it in one narrow
//! place: logic that needs neither the database nor any other external system
//! lives here, where it is tested directly, without a database, by unit tests
//! and proptest properties.
//!
//! The rules for everything under `src/domain/` (ADR 0008, rule 1):
//!
//! - No I/O, no `AppContext`, no `async`. Nothing here reads files, the
//!   network, the database, environment variables, the clock or a random
//!   number generator; "now", "today", random input and environment values
//!   are function arguments.
//! - Nothing here names another module of this crate. Callers outside
//!   (controllers, `app.rs`, `bin/main.rs`) translate framework types into the
//!   plain values these functions take.
//! - Every public function has unit tests, and every function with a
//!   non-trivial input space (strings, collections, integers, ...) also has at
//!   least one proptest property.
//!
//! `tests/architecture.rs` enforces the mechanical part of these rules.

pub mod admin_path;
pub mod api_path;
pub mod budapest;
pub mod cli_guard;
pub mod clock;
pub mod database_target;
pub mod email_address;
pub mod etag;
pub mod http_metrics;
pub mod image_optimise;
pub mod password_hash;
pub mod proc_stat;
pub mod rich_text;
pub mod route_coverage;
pub mod runtime_metrics;
pub mod schema_snapshot;
pub mod session_cookie;
pub mod task_lock_key;
pub mod totp;
pub mod totp_secret;
pub mod v1_params;
