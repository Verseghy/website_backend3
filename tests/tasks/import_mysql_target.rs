//! The import's side of its target (`tasks::import_mysql::target`,
//! data-migration.md §4.2, §11.1), on the test's own migrated database: the
//! one transaction that `dry-run` rolls back and `apply` commits, the check
//! of `roles` and the `user_permissions` CHECK against the code, and the
//! refusal of a target table that holds a row, with the lock that keeps the
//! tables empty until the transaction ends. The transaction is `READ
//! COMMITTED` under any default isolation, and every table of the migrated
//! schema is on one list of what the import does with it, so the guard
//! cannot skip an entity table that exists under another name.
//!
//! Rows are written in SQL or through the models, so each test controls
//! what the target holds; every account is invented.

use std::{
    future::Future,
    panic::{AssertUnwindSafe, resume_unwind},
    time::Duration,
};

use backend3::{
    domain::{
        activity::{Action, Actor},
        authz::Permission,
        import_mysql::{
            options::Mode,
            target::{IMPORT_TARGET_TABLES, READ_ONLY_TARGET_TABLES},
        },
        schema_snapshot::FRAMEWORK_TABLES,
    },
    models::activity_log::{self, EntityRef, Entry},
    tasks::import_mysql::target::{check_code_tables, in_import_transaction, refuse_non_empty},
};
use futures_util::FutureExt;
use loco_rs::{Error, app::AppContext, db};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use tokio::time;

use crate::support::with_app;

/// The single value in the first column of the first row of `sql`.
async fn scalar<T: sea_orm::TryGetable>(db: &impl ConnectionTrait, sql: &str) -> T {
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .expect("one row")
        .try_get_by_index(0)
        .expect("a value")
}

/// Inserts an invented account with `email` and returns its id.
async fn insert_account(db: &impl ConnectionTrait, email: &str) -> i64 {
    scalar(
        db,
        &format!(
            "INSERT INTO users (pid, name, email) VALUES (gen_random_uuid(), 'Próba', '{email}') \
             RETURNING id"
        ),
    )
    .await
}

/// The tables of `IMPORT_TARGET_TABLES` that the migrated schema has, in
/// the list's order.
async fn existing_target_tables(db: &DatabaseConnection) -> Vec<&'static str> {
    let mut existing = Vec::new();
    for table in IMPORT_TARGET_TABLES {
        if scalar::<bool>(db, &format!("SELECT to_regclass('{table}') IS NOT NULL")).await {
            existing.push(table);
        }
    }
    existing
}

/// Both checks, as the import runs them first, in a transaction that
/// rolls back.
async fn checks(db: &DatabaseConnection) -> Result<(), Error> {
    in_import_transaction(db, Mode::DryRun, async |txn| {
        check_code_tables(txn).await?;
        refuse_non_empty(txn).await
    })
    .await
}

#[tokio::test]
async fn a_freshly_migrated_target_passes_both_checks() {
    with_app(|_router, ctx| async move {
        checks(&ctx.db).await.expect("an empty, migrated target");
        // The tables that exist today; the entity tables join the list as
        // their migrations merge (the guard skips a table it does not find).
        let existing = existing_target_tables(&ctx.db).await;
        for table in [
            "users",
            "user_roles",
            "user_permissions",
            "revisions",
            "activity_log",
        ] {
            assert!(existing.contains(&table), "{table}: {existing:?}");
        }
    })
    .await;
}

/// The tables of the migrated schema that the import never writes, other
/// than `READ_ONLY_TARGET_TABLES` and the frameworks' tables, each with the
/// reason. None so far.
///
/// A migration that adds a table puts it on exactly one list: a table that
/// the import writes goes into `IMPORT_TARGET_TABLES`
/// (`src/domain/import_mysql/target.rs`) under its name in the insert order
/// of data-migration.md §4.3, and any other table (the release-1 tables of
/// ADR 0019 §5, for example) comes here.
const NEVER_WRITTEN: &[(&str, &str)] = &[];

/// Every table of the migrated schema is on one of the lists, so an entity
/// table created under another name than `IMPORT_TARGET_TABLES` gives it
/// fails here, rather than being skipped by the empty-target guard for
/// good.
#[tokio::test]
async fn every_table_of_the_migrated_schema_is_a_target_or_listed_as_never_written() {
    with_app(|_router, ctx| async move {
        let rows = ctx
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT c.relname::text FROM pg_class c WHERE c.relnamespace = \
                 'public'::regnamespace AND c.relkind IN ('r', 'p') ORDER BY 1",
            ))
            .await
            .expect("the tables of the schema");
        let tables: Vec<String> = rows
            .iter()
            .map(|row| row.try_get_by_index(0).expect("a name"))
            .collect();
        assert!(tables.iter().any(|table| table == "users"), "{tables:?}");
        for table in &tables {
            let lists = [
                IMPORT_TARGET_TABLES.contains(&table.as_str()),
                READ_ONLY_TARGET_TABLES.contains(&table.as_str()),
                FRAMEWORK_TABLES.contains(&table.as_str()),
                NEVER_WRITTEN.iter().any(|(name, _)| name == table),
            ];
            assert_eq!(
                lists.iter().filter(|on| **on).count(),
                1,
                "the migrated table `{table}` must be on exactly one list: a table the import \
                 writes on IMPORT_TARGET_TABLES (src/domain/import_mysql/target.rs) under its \
                 name in data-migration.md §4.3, any other on NEVER_WRITTEN of this test with \
                 the reason"
            );
        }
        for (table, reason) in NEVER_WRITTEN {
            assert!(!reason.is_empty(), "{table} needs a reason");
        }
    })
    .await;
}

/// Runs `test` on a new pool whose sessions start with the database
/// default `default_transaction_isolation` set to `level`; `ctx.db`'s
/// pooled sessions may predate the default.
async fn with_default_isolation<F, Fut, T>(ctx: &AppContext, level: &str, test: F) -> T
where
    F: FnOnce(DatabaseConnection) -> Fut,
    Fut: Future<Output = T>,
{
    // `ALTER DATABASE` takes no bind parameters, so the server quotes them.
    let alter: String = scalar(
        &ctx.db,
        &format!(
            "SELECT format('ALTER DATABASE %I SET default_transaction_isolation = %L', \
             current_database(), '{level}')"
        ),
    )
    .await;
    ctx.db
        .execute_unprepared(&alter)
        .await
        .unwrap_or_else(|error| panic!("{alter}: {error}"));
    let pool = db::connect(&ctx.config.database)
        .await
        .expect("connect as Loco does");
    // The pool is closed also when `test` panics, so the test's database can
    // be dropped and the panic is the failure reported.
    let outcome = AssertUnwindSafe(test(pool.clone())).catch_unwind().await;
    pool.close_by_ref().await.expect("close the extra pool");
    outcome.unwrap_or_else(|panic| resume_unwind(panic))
}

/// The import's transaction is `READ COMMITTED` under any database default,
/// so the empty-target guard, which locks after the transaction's first
/// query, sees a row that a writer committed while the lock waited for it.
/// Under the `REPEATABLE READ` default it would keep the snapshot of that
/// first query and pass the table.
#[tokio::test]
async fn the_guard_sees_a_row_committed_while_its_lock_waited_under_any_default_isolation() {
    with_app(|_router, ctx| async move {
        for level in ["repeatable read", "serializable"] {
            let seen = with_default_isolation(&ctx, level, |pool| async move {
                let control = pool.begin().await.expect("a control transaction");
                let control_level: String = scalar(&control, "SHOW transaction_isolation").await;
                control.rollback().await.expect("roll the control back");
                let import_level: String =
                    in_import_transaction(&pool, Mode::DryRun, async |txn| {
                        Ok(scalar(txn, "SHOW transaction_isolation").await)
                    })
                    .await
                    .expect("the import's transaction");
                (control_level, import_level)
            })
            .await;
            assert_eq!(seen, (level.to_owned(), "read committed".to_owned()));
        }

        let observer = &ctx.db;
        let refusal = with_default_isolation(&ctx, "repeatable read", |pool| async move {
            // A writer holds an uncommitted account when the import starts.
            let writer = observer.begin().await.expect("the writer's transaction");
            insert_account(&writer, "kozben@example.test").await;
            let waiting = "SELECT count(*) FROM pg_locks WHERE relation = 'users'::regclass AND \
                           mode = 'ShareRowExclusiveLock' AND NOT granted";
            let commit_once_the_guard_waits = async {
                time::timeout(Duration::from_secs(30), async {
                    while scalar::<i64>(observer, waiting).await == 0 {
                        time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("the guard's lock never waited for the writer");
                writer.commit().await.expect("the writer commits");
            };
            let (outcome, ()) = tokio::join!(checks(&pool), commit_once_the_guard_waits);
            outcome.expect_err("the committed account").to_string()
        })
        .await;
        assert!(
            refusal.starts_with("refusing to import: the target table `users` already holds a row"),
            "{refusal}"
        );
    })
    .await;
}

#[tokio::test]
async fn dry_run_rolls_back_apply_commits_and_a_failed_run_always_rolls_back() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let count = || async { scalar::<i64>(db, "SELECT count(*) FROM users").await };

        in_import_transaction(db, Mode::DryRun, async |txn| {
            insert_account(txn, "szaraz@example.test").await;
            Ok(())
        })
        .await
        .expect("a dry run");
        assert_eq!(count().await, 0, "a dry run leaves the target as it was");

        let id = in_import_transaction(db, Mode::Apply, async |txn| {
            Ok(insert_account(txn, "eles@example.test").await)
        })
        .await
        .expect("an apply run");
        assert_eq!(count().await, 1, "apply commits");
        assert!(id > 0);

        let error = in_import_transaction(db, Mode::Apply, async |txn| {
            insert_account(txn, "hibas@example.test").await;
            Err::<(), _>(Error::string("V8 failed"))
        })
        .await
        .expect_err("the run's error");
        assert_eq!(error.to_string(), "V8 failed");
        assert_eq!(count().await, 1, "a failed run rolls back, also in apply");
    })
    .await;
}

#[tokio::test]
async fn a_target_table_that_holds_a_row_is_refused_naming_the_first() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        // An activity entry alone, as a stray write before the cut-over
        // would leave one.
        activity_log::record(
            db,
            Actor::Import,
            Entry {
                entity: Some(EntityRef::table("labels")),
                ..Entry::new(Action::Import)
            },
        )
        .await
        .expect("an activity entry");
        let message = checks(db).await.expect_err("activity_log").to_string();
        assert_eq!(
            message,
            "refusing to import: the target table `activity_log` already holds a row. The import \
             never merges into existing data, so it runs only into a freshly migrated, empty \
             database (data-migration.md §4.2, §11.2): recreate the target database, migrate it \
             with `backend3-cli db migrate`, and run the import again before anything else \
             writes to it"
        );

        // An account as well: `users` comes first in the list, so it is the
        // one named.
        insert_account(db, "elso@example.test").await;
        let message = checks(db).await.expect_err("users").to_string();
        assert!(
            message.starts_with("refusing to import: the target table `users` already holds a row"),
            "{message}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_guard_locks_every_target_table_it_found_until_the_transaction_ends() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let existing = existing_target_tables(db).await;
        let lock_query = "SELECT coalesce(string_agg(relation::regclass::text, ',' ORDER BY \
                          relation::regclass::text), '') FROM pg_locks WHERE locktype = \
                          'relation' AND database = (SELECT oid FROM pg_database WHERE datname = \
                          current_database()) AND mode = 'ShareRowExclusiveLock'";

        let held: String = in_import_transaction(db, Mode::DryRun, async |txn| {
            refuse_non_empty(txn).await?;
            Ok(scalar::<String>(txn, lock_query).await)
        })
        .await
        .expect("the guard");
        let mut expected: Vec<&str> = existing.clone();
        expected.sort_unstable();
        assert_eq!(held, expected.join(","));
        for table in READ_ONLY_TARGET_TABLES {
            assert!(!held.split(',').any(|locked| locked == table), "{table}");
        }

        // The transaction ended, and its locks with it.
        assert_eq!(scalar::<String>(db, lock_query).await, "");
    })
    .await;
}

#[tokio::test]
async fn roles_that_differ_from_the_code_are_refused_naming_the_difference() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        db.execute_unprepared("UPDATE roles SET name = 'author' WHERE id = 1")
            .await
            .expect("rename a role");
        let message = checks(db).await.expect_err("roles differ").to_string();
        assert!(
            message.starts_with(
                "refusing to import: the target's `roles` table differs from the roles of the \
                 code (src/domain/authz.rs, ADR 0007 §3), to which the import maps the legacy \
                 role names (data-migration.md §6.13): missing 1 `writer`; unexpected 1 `author`."
            ),
            "{message}"
        );
    })
    .await;
}

/// A CHECK of `user_permissions` that admits `names`, in place of the
/// migration's.
async fn replace_permission_check(db: &DatabaseConnection, names: &[&str]) {
    let list = names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    db.execute_unprepared(&format!(
        "ALTER TABLE user_permissions DROP CONSTRAINT user_permissions_permission_known, \
         ADD CONSTRAINT user_permissions_permission_known CHECK (permission IN ({list}))"
    ))
    .await
    .expect("replace the CHECK");
}

#[tokio::test]
async fn a_permission_check_that_differs_from_the_code_is_refused_naming_each_name() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        // The code's sixteen names in another order are the same CHECK.
        let mut names: Vec<&str> = Permission::ALL
            .iter()
            .map(|permission| permission.name())
            .collect();
        names.reverse();
        replace_permission_check(db, &names).await;
        checks(db).await.expect("the same names in another order");

        // `edit newsletter` admitted and `view logs` not.
        let names: Vec<&str> = names
            .into_iter()
            .filter(|name| *name != "view logs")
            .chain(["edit newsletter"])
            .collect();
        replace_permission_check(db, &names).await;
        let message = checks(db).await.expect_err("differs").to_string();
        assert!(
            message.contains(
                ": not admitted `view logs`; admitted but unknown to the code `edit newsletter`. "
            ),
            "{message}"
        );

        // No CHECK at all.
        db.execute_unprepared(
            "ALTER TABLE user_permissions DROP CONSTRAINT user_permissions_permission_known",
        )
        .await
        .expect("drop the CHECK");
        let message = checks(db).await.expect_err("no CHECK").to_string();
        assert!(
            message.starts_with("refusing to import: `user_permissions` has 0 CHECK constraints"),
            "{message}"
        );
    })
    .await;
}
