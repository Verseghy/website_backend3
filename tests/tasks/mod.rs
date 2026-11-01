//! Task tests; `cargo loco generate task` adds a module per task here.

// The legacy import exists only with its Cargo feature (data-migration.md §3.3).
#[cfg(feature = "import-mysql")]
mod import_mysql;
