//! Background workers on Loco's Postgres queue (ADR 0016 §3).
//!
//! `cargo loco generate worker` appends its module here and registers it in
//! `App::connect_workers`. Every worker is idempotent, takes small arguments
//! and records its outcome on the row it works on (ADR 0016 §3).
