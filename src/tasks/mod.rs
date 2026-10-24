//! Loco tasks, run as `backend3-cli task <name>` by operators and by the
//! Kubernetes `CronJob`s of ADR 0016 §4.
//!
//! `cargo loco generate task` appends its module here and registers it in
//! `App::register_tasks`. Every task is idempotent and takes a Postgres
//! advisory lock keyed by its name (ADR 0016 §4).
