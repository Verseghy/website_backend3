//! The `import_mysql` task: the legacy import of ADR 0017, specified by
//! docs/design/data-migration.md.
//!
//! ```sh
//! backend3-cli task import_mysql mode:dry-run snapshot_at:2026-10-23T17:01:02Z \
//!   resolutions:data/import/resolutions.toml storage_root:<dir> report:<empty dir>
//! ```
//!
//! This module is the only code of the import that touches `AppContext`
//! (ADR 0008 rules 2 and 3, data-migration.md §4.1). [`ImportMysql::run`]
//! takes the run apart in this order, and refuses with a message that names
//! what to fix at the first problem:
//!
//! 1. reads the start of the run from the app clock (ADR 0010 point 7);
//! 2. parses and validates the arguments into an [`ImportOptions`]
//!    (`domain::import_mysql::options`);
//! 3. reads the source setting, `settings.import.source_database_url`, and
//!    refuses an unset one (empty, or the placeholder `disabled`), one that
//!    is not a MySQL URL, and, in the `test`, `development` and `e2e`
//!    environments, one that is not local (the source guard of ADR 0024 §4,
//!    layer 3), naming the setting and never showing its value ([`source`]);
//! 4. checks the paths: the resolution file can be read, `storage_root` is a
//!    directory, and `report` is an empty directory outside `storage_root`
//!    ([`paths`]);
//! 5. then, under the task lock (ADR 0016 §4; a run that finds the lock
//!    held exits 0 and writes nothing), builds the source pool, calls
//!    [`run`] with the source, the target and the options, reads the end of
//!    the run from the app clock and writes the report, also for a run that
//!    failed ([`report`]).
//!
//! [`run`] is where the import steps go, one slice at a time (ADR 0025
//! class d). It runs in one transaction on the target, which `apply`
//! commits and `dry-run` rolls back ([`target`]), and so far:
//!
//! 1. checks that the target's `roles` and its `user_permissions` CHECK
//!    agree with the code, which the import maps the legacy roles and
//!    grants onto ([`target::check_code_tables`]);
//! 2. refuses a target in which any table the import writes holds a row,
//!    naming the first ([`target::refuse_non_empty`]);
//! 3. reads the source in one locked session: the schema fingerprint,
//!    every table's `AUTO_INCREMENT` and the latest timestamp, which
//!    `snapshot_at` must not precede ([`session`]).
//!
//! The resolution file, the defect detector and the table steps follow,
//! and until they arrive the run imports nothing, and the report says so
//! section by section. Every step reads the legacy tables through
//! [`source::select_rows`] on the session's connection, the source query
//! layer, which reads only the tables of the source allow-list
//! (`domain::import_mysql::source_tables`; the newsletter is never read,
//! NWS-01). `tests/architecture.rs` fails when a file of this module other
//! than `source.rs` and `session.rs` builds or sends a statement to the
//! source or opens a connection to it.
//!
//! The whole module, and the MySQL driver, exist only with the default Cargo
//! feature `import-mysql` (data-migration.md §3.3).

pub mod expected_schema;
mod paths;
pub mod report;
pub mod session;
pub mod source;
pub mod target;

use loco_rs::{
    Error, Result,
    app::AppContext,
    task::{Task, TaskInfo, Vars},
};
use sea_orm::DatabaseConnection;
use sqlx::MySqlPool;

use crate::{
    app,
    domain::import_mysql::{
        options::{ImportOptions, format_instant},
        report::{ReportParts, TASK_NAME},
    },
    models::task_lock::with_task_lock,
    settings::Settings,
};

/// The `import_mysql` task.
pub struct ImportMysql;

#[async_trait::async_trait]
impl Task for ImportMysql {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: TASK_NAME.to_owned(),
            detail: "Import the legacy MySQL data (data-migration.md): mode:dry-run (the \
                     default) rolls back, mode:apply commits; needs snapshot_at, storage_root \
                     and report"
                .to_owned(),
        }
    }

    /// Runs the import as the module documentation describes.
    ///
    /// # Errors
    ///
    /// A refused argument, an unset or unusable source setting, a path that
    /// fails its check, a failed step of the import or a report that cannot
    /// be written. Every message names what to fix and never shows the
    /// source URL. A run that finds the task lock held is not an error.
    async fn run(&self, ctx: &AppContext, vars: &Vars) -> Result<()> {
        let clock = app::clock(ctx)?;
        let started_at = clock.now();
        let options = ImportOptions::from_arguments(&vars.cli, started_at).map_err(Error::msg)?;
        let settings = Settings::from_section(ctx.config.settings.as_ref()).map_err(Error::msg)?;
        let source_options = source::connect_options(
            &settings.import.source_database_url,
            &ctx.environment.to_string(),
        )?;
        paths::check(&options)?;

        with_task_lock(&ctx.db, &self.task().name, || async {
            tracing::info!(
                task = TASK_NAME,
                mode = options.mode.as_str(),
                snapshot_at = %format_instant(options.snapshot_at),
                report = %options.report.display(),
                "the import run starts"
            );
            let source_db = source::pool(source_options);
            let mut parts = ReportParts::default();
            let result = run(&source_db, &ctx.db, &options, &mut parts).await;
            source_db.close().await;
            // The end is read after the source is closed, so the run time
            // covers the whole run; `finish` writes the report, also for a
            // failed run, and returns the run's own error first.
            report::finish(&options, started_at, clock.now(), parts, result)
        })
        .await?;
        Ok(())
    }
}

/// Imports the legacy data from `source_db` into `target_db`
/// (data-migration.md §4.3).
///
/// - `source_db`: the pool of the legacy MySQL source: one connection,
///   opened on first use, for the one locked session that reads the source
///   (data-migration.md §3.3).
/// - `target_db`: backend3's own database, the application pool.
/// - `options`: the run's validated arguments.
/// - `parts`: the report's parts, which each step fills as it ends, so the
///   report of a run that fails later still shows them.
///
/// Everything runs in one transaction on the target ([`target`]): the
/// checks of the code tables, the refusal of a target that holds data, and
/// the source session ([`session::read`]). `apply` commits it, `dry-run`
/// rolls it back, and a failed run always rolls back. The resolution file,
/// the defect detector and the table steps come in the slices that follow
/// (ADR 0025 class d), and each adds its part of the report.
///
/// # Errors
///
/// The refusals of data-migration.md §4.2 that the steps so far make: a
/// `roles` table or a permission CHECK that differs from the code, a
/// target table that holds a row, a source whose schema differs from the
/// expected DDL, and a `snapshot_at` earlier than the latest timestamp in
/// the source; and any failed exchange with either database.
pub async fn run(
    source_db: &MySqlPool,
    target_db: &DatabaseConnection,
    options: &ImportOptions,
    parts: &mut ReportParts,
) -> Result<()> {
    target::in_import_transaction(target_db, options.mode, async |txn| {
        target::check_code_tables(txn).await?;
        target::refuse_non_empty(txn).await?;
        parts.source = Some(session::read(source_db, options).await?);
        Ok(())
    })
    .await
}
