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
//!    refuses an unset one (empty, or the placeholder `disabled`) or one that
//!    is not a MySQL URL, naming the setting and never showing its value
//!    ([`source`]);
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
//! class d): the source session and the target guard, the resolution file,
//! the defect detector and the table steps. Until they arrive it imports
//! nothing, and the report says so section by section.
//!
//! The whole module, and the MySQL driver, exist only with the default Cargo
//! feature `import-mysql` (data-migration.md §3.3).

mod paths;
pub mod report;
mod source;

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
        report::TASK_NAME,
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
        let source_options = source::connect_options(&settings.import.source_database_url)?;
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
            let result = run(&source_db, &ctx.db, &options).await;
            source_db.close().await;
            // The end is read after the source is closed, so the run time
            // covers the whole run; `finish` writes the report, also for a
            // failed run, and returns the run's own error first.
            report::finish(&options, started_at, clock.now(), result)
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
///
/// It has no step yet: the source session and the target guard, the
/// resolution file, the defect detector and each table's step come in the
/// slices that follow (ADR 0025 class d), and each adds its part of the
/// report. Until then it reads and writes nothing.
///
/// # Errors
///
/// None yet; each step adds the refusals of data-migration.md §4.2.
#[allow(
    clippy::unused_async,
    reason = "the import steps that follow await the source and the target"
)]
pub async fn run(
    source_db: &MySqlPool,
    target_db: &DatabaseConnection,
    options: &ImportOptions,
) -> Result<()> {
    let _ = (source_db, target_db, options);
    Ok(())
}
