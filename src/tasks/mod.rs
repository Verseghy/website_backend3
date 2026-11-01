//! Loco tasks, run as `backend3-cli task <name>` by operators and by the
//! Kubernetes `CronJob`s of ADR 0016 §4.
//!
//! `cargo loco generate task` appends its module here and registers it in
//! `App::register_tasks`. Every task is idempotent and does all of its
//! database work inside [`crate::models::task_lock::with_task_lock`], which
//! skips the run, successfully, while another run holds the task's lock
//! (ADR 0016 §4; recipe 6 of the backend3 skill). `tests/architecture.rs`
//! fails for a task whose `impl Task` does not call it under the name the
//! task registers.
//!
//! `import_mysql`, the legacy import, exists only with the default Cargo
//! feature `import-mysql` (data-migration.md §3.3).

#[cfg(feature = "import-mysql")]
pub mod import_mysql;
