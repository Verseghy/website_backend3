//! The pure parts of the legacy import, the `import_mysql` task
//! (ADR 0017, docs/design/data-migration.md).
//!
//! The task in `src/tasks/import_mysql/` is the only code that touches
//! `AppContext`, the environment, the clock, the file system and the two
//! databases. It turns what it reads into plain values and hands them to the
//! functions here (ADR 0008 rules 1-3, data-migration.md §4.1):
//!
//! - [`options`]: the task's arguments (`mode`, `snapshot_at`,
//!   `resolutions`, `storage_root`, `report`, `canteen_audit_local_before`),
//!   parsed and validated into [`options::ImportOptions`], with a message that
//!   names the argument for every refusal;
//! - [`source`]: the source setting, `settings.import.source_database_url`,
//!   whose empty and `disabled` values both mean unset, and the checks that
//!   never show its value, because it holds the source password;
//! - [`report`]: the run report's `summary.json` and its Markdown skeleton
//!   (data-migration.md §11.3), from the instants the task read from the app
//!   clock;
//! - [`not_migrated`]: the legacy tables that are not migrated, with their
//!   reasons (data-migration.md §9.1).
//!
//! These modules are compiled in every build, with or without the
//! `import-mysql` feature: they hold no MySQL code, and the typed settings
//! (`src/settings.rs`) read the source setting in every build. The PR that
//! removes the importer (data-migration.md §13 item 9) removes them with the
//! task.

pub mod not_migrated;
pub mod options;
pub mod report;
pub mod source;
