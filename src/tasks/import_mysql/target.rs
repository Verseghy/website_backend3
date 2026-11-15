//! The import's side of its target, backend3's own PostgreSQL database
//! (docs/design/data-migration.md §4.2, §11.1): the one transaction that the
//! whole import runs in, and the checks that open it.
//!
//! [`in_import_transaction`] runs the import in one transaction and ends it
//! by the run's mode: `apply` commits, `dry-run` rolls back, and a failed
//! run always rolls back, so a refused or dry run leaves the target exactly
//! as it was (§11.1).
//!
//! Its first steps, before the source is read:
//!
//! - [`check_code_tables`]: `roles` holds exactly the four roles of the
//!   code, and the one CHECK on `user_permissions` admits exactly its
//!   sixteen permission names (no `edit newsletter`; ADR 0007 §1, §3), since
//!   the import maps the legacy roles and grants onto them by name (§6.13,
//!   §6.15);
//! - [`refuse_non_empty`]: every table the import writes is locked against
//!   other writers for the rest of the transaction and must be empty; the
//!   first that holds a row is named (§4.2). This is what makes the import
//!   impossible to run a second time after go-live, and what the pre-import
//!   invariant of `AGENTS.md` keeps true in production.
//!
//! The statements and the checks of their answers are pure, in
//! `domain::import_mysql::target`. The catalog query and the guard's
//! statements are raw SQL here because they serve only the import (ADR 0008
//! rule 5); the `roles` rows come from the model's finder.

use loco_rs::{Error, Result};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, IsolationLevel, Statement,
    TransactionTrait,
};

use crate::{
    domain::import_mysql::{
        options::Mode,
        target::{
            IMPORT_TARGET_TABLES, NotEmpty, check_permission_constraint, check_roles,
            has_rows_statement, lock_statement,
        },
    },
    models::roles,
};

/// Runs `work` in one transaction on `target_db`, and ends it by `mode`.
///
/// - `target_db`: backend3's database, the application pool.
/// - `mode`: `Apply` commits a run whose `work` succeeded; `DryRun` rolls
///   it back.
/// - `work`: the import, which receives the transaction.
///
/// A failed `work` is always rolled back, and its error is returned; a
/// rollback that fails as well is logged, because the error to fix is the
/// run's (PostgreSQL also ends the transaction when its connection closes).
///
/// The transaction is always `READ COMMITTED`, whatever
/// `default_transaction_isolation` the server, database or role sets. The
/// guard of [`refuse_non_empty`] locks the target tables after the first
/// query of the transaction ([`check_code_tables`] reads `roles` first):
/// under `REPEATABLE READ` or `SERIALIZABLE`, the transaction would keep the
/// snapshot of that first query, so a row that another writer committed
/// while the lock waited for it would stay invisible, and the guard would
/// pass a table that holds it. Under `READ COMMITTED` each statement sees
/// every row committed before it, and once the lock is held no other
/// transaction can write those tables until the import ends.
///
/// # Errors
///
/// The transaction cannot begin, `work` fails, or the commit or rollback
/// that ends a successful `work` fails.
pub async fn in_import_transaction<T>(
    target_db: &DatabaseConnection,
    mode: Mode,
    work: impl AsyncFnOnce(&DatabaseTransaction) -> Result<T>,
) -> Result<T> {
    // sea-orm issues `SET TRANSACTION ISOLATION LEVEL READ COMMITTED` right
    // after `BEGIN`, before any query, as PostgreSQL requires.
    let txn = target_db
        .begin_with_config(Some(IsolationLevel::ReadCommitted), None)
        .await?;
    match work(&txn).await {
        Ok(value) => {
            match mode {
                Mode::Apply => txn.commit().await?,
                Mode::DryRun => txn.rollback().await?,
            }
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback) = txn.rollback().await {
                tracing::error!(
                    %rollback,
                    "the failed import's transaction could not be rolled back"
                );
            }
            Err(error)
        }
    }
}

/// Checks that the target's `roles` rows and its `user_permissions` CHECK
/// agree with the code (`domain::authz`).
///
/// - `txn`: the import's transaction.
///
/// # Errors
///
/// A failed query, or the mismatch, which names every role or permission
/// that differs.
pub async fn check_code_tables(txn: &DatabaseTransaction) -> Result<()> {
    let found: Vec<(i64, String)> = roles::Model::list_in_id_order(txn)
        .await?
        .into_iter()
        .map(|role| (role.id, role.name))
        .collect();
    check_roles(&found).map_err(Error::msg)?;

    // `to_regclass` gives NULL for a target without the table, so the check
    // reports "no CHECK" rather than failing on a missing relation.
    let rows = txn
        .query_all_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT conname::text, pg_get_constraintdef(oid) FROM pg_constraint \
             WHERE conrelid = to_regclass('user_permissions') AND contype = 'c' \
             ORDER BY conname",
        ))
        .await?;
    let checks = rows
        .iter()
        .map(|row| Ok((row.try_get_by_index(0)?, row.try_get_by_index(1)?)))
        .collect::<std::result::Result<Vec<(String, String)>, sea_orm::DbErr>>()?;
    check_permission_constraint(&checks).map_err(Error::msg)
}

/// Locks every table of `IMPORT_TARGET_TABLES` that the target has against
/// other writers until the transaction ends, then refuses the import if any
/// of them holds a row, naming the first in the list's order.
///
/// - `txn`: the import's transaction.
///
/// A table of the list that the target's schema does not have yet holds no
/// row, so it is skipped, and a warning names every skipped table: until
/// every entity's migration has merged, a run skips their tables, and a
/// skip in a target that should have them shows up in the run's log.
/// Tests keep a misspelt or renamed table from being skipped for good:
/// `tests/domain/import_target_tables.rs` checks the list against the
/// insert order of data-migration.md §4.3, and
/// `tests/tasks/import_mysql_target.rs` that every table of the migrated
/// schema is on it or on a list of the tables the import never writes.
///
/// # Errors
///
/// A failed query, or [`NotEmpty`] for the first table that holds a row.
pub async fn refuse_non_empty(txn: &DatabaseTransaction) -> Result<()> {
    let mut existing = Vec::new();
    let mut skipped = Vec::new();
    for table in IMPORT_TARGET_TABLES {
        let exists: bool = txn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT to_regclass(quote_ident($1)) IS NOT NULL",
                [table.into()],
            ))
            .await?
            .ok_or_else(|| Error::string("the existence check of a target table returned no row"))?
            .try_get_by_index(0)?;
        if exists {
            existing.push(table);
        } else {
            skipped.push(table);
        }
    }
    if !skipped.is_empty() {
        tracing::warn!(
            skipped = %skipped.join(", "),
            "the empty-target guard skipped the import's target tables that the target's schema \
             does not have (data-migration.md §4.2)"
        );
    }
    if let Some(lock) = lock_statement(&existing) {
        txn.execute_unprepared(&lock).await?;
    }
    for table in existing {
        let has_rows: bool = txn
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                has_rows_statement(table),
            ))
            .await?
            .ok_or_else(|| Error::string("the emptiness check of a target table returned no row"))?
            .try_get_by_index(0)?;
        if has_rows {
            return Err(Error::msg(NotEmpty { table }));
        }
    }
    Ok(())
}
