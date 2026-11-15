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
//! - [`source_guard`]: the refusal of a source that is not local in the
//!   `test`, `development` and `e2e` environments (ADR 0024 §4, layer 3);
//! - [`source_tables`]: the source allow-list, the 17 legacy tables the
//!   import reads, and the checked `SELECT` that is the only statement
//!   form that names one; the newsletter's subscriber list is never read
//!   (NWS-01);
//! - [`source_session`]: the statements of the locked source session
//!   (session settings, `LOCK TABLES`, the fingerprint and `AUTO_INCREMENT`
//!   queries), the columns that hold the source's timestamps, and the
//!   refusal of a `snapshot_at` earlier than the latest of them (§3.3,
//!   §4.1);
//! - [`fingerprint`]: the comparison of the source's columns with the
//!   expected legacy DDL, naming every mismatch (§3.3);
//! - [`target`]: the checks on backend3's own database before the import:
//!   `roles` and the `user_permissions` CHECK agree with the code, and every
//!   table the import writes is empty (§4.2);
//! - [`resolutions`]: the defect resolution file of data-migration.md §8.2,
//!   `data/import/resolutions.toml`: its TOML schema, parsed and validated
//!   into [`resolutions::ResolutionFile`] with a message that names the
//!   entry and the field for every problem;
//! - [`defects`]: the generic defect detector (ADR 0017 §3, ADR 0047): every
//!   instance detected from its rule on every run and paired with the
//!   resolution file's entry, whose precondition must hold; `apply` stops at
//!   the first instance or entry it cannot resolve as signed, a dry run
//!   lists them all and fails at its end;
//! - [`n7_review`]: N7's review of the legacy bodies: a body from which the
//!   normaliser's sanitiser removed what no step planned is refused, naming
//!   the row and the rules, unless an `N7-accept` entry of the resolution
//!   file accepts it; a dry run collects every refusal, `apply` stops at the
//!   first (data-migration.md §7.1 N7, §8.2; ADR 0047). Its verdicts are
//!   N7's entry policy in [`defects`] (research: DECISIONS R38 (D161));
//! - [`report`]: the run report's `summary.json` and its Markdown skeleton
//!   (data-migration.md §11.3), from the instants the task read from the app
//!   clock, with the source session's and the defect detectors' parts;
//! - [`signoff`]: the editors' sign-off sheet, `signoff.md`, an extract of
//!   the Defects part with ids, public paths and proposed resolutions only
//!   (data-migration.md §8.2);
//! - [`not_migrated`]: the legacy tables that are not migrated, with their
//!   reasons (data-migration.md §9.1).
//!
//! These modules are compiled in every build, with or without the
//! `import-mysql` feature: they hold no MySQL code, and the typed settings
//! (`src/settings.rs`) read the source setting in every build. The PR that
//! removes the importer (data-migration.md §13 item 9) removes them with the
//! task.

pub mod defects;
pub mod fingerprint;
pub mod n7_review;
pub mod not_migrated;
pub mod options;
pub mod report;
pub mod resolutions;
pub mod signoff;
pub mod source;
pub mod source_guard;
pub mod source_session;
pub mod source_tables;
pub mod target;
