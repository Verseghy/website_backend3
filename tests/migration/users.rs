//! The `users` migration (ADR 0006 §7, ADR 0009, ADR 0011): the column set,
//! the anonymisation CHECKs, the case-insensitive e-mail key over all rows and
//! the self-referencing audit columns. Rows are written in SQL, so each test
//! controls every column. Every address is invented.
//!
//! The rollback is covered by `search_support`'s test, which migrates every
//! migration down and up again.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

use crate::support::with_app;

async fn execute(db: &DatabaseConnection, sql: &str) -> Result<u64, DbErr> {
    db.execute_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .map(|result| result.rows_affected())
}

/// The text in the first column of each row that `sql` returns.
async fn texts(db: &DatabaseConnection, sql: &str) -> Vec<String> {
    db.query_all_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect(sql)
        .iter()
        .map(|row| row.try_get_by_index(0).expect("text"))
        .collect()
}

/// Runs `sql`, expects PostgreSQL to refuse it, and checks that the refusal
/// names `constraint`.
async fn assert_refused(db: &DatabaseConnection, sql: &str, constraint: &str) {
    let error = execute(db, sql)
        .await
        .expect_err(&format!("refused by {constraint}: {sql}"));
    assert!(
        error.to_string().contains(constraint),
        "expected {constraint}, got: {error}"
    );
}

/// `INSERT INTO users` with a fresh `pid`, a `name` and the given columns.
fn insert(columns: &str, values: &str) -> String {
    format!(
        "INSERT INTO users (pid, name, {columns}) VALUES (gen_random_uuid(), 'Próba', {values})"
    )
}

/// The instant these tests write to `deleted_at` and `anonymised_at`, as a
/// SQL literal. The CHECKs only ask whether a column is NULL, so any instant
/// does. A fixed one keeps the tests off the database's clock: the only
/// wall-clock reads in tests are the ones recipe 7 of the project skill
/// names.
const AT: &str = "TIMESTAMPTZ '2026-10-25 08:00:00+00'";

/// The columns that anonymisation clears or sets (ADR 0011 §6).
const ANONYMISED: &str =
    "email, password_hash, totp_secret_encrypted, active, deleted_at, anonymised_at";

/// The anonymised shape that both CHECKs accept, as values for [`ANONYMISED`].
fn anonymised_ok() -> String {
    format!("NULL, NULL, NULL, false, {AT}, {AT}")
}

#[tokio::test]
async fn the_table_has_exactly_the_planned_columns() {
    // No api_key, verification, mailed-reset or magic-link column of Loco's
    // starter (ADR 0006 §4).
    with_app(|_router, ctx| async move {
        let columns = texts(
            &ctx.db,
            "SELECT column_name || ' ' || data_type || ' ' || is_nullable \
             FROM information_schema.columns WHERE table_name = 'users' ORDER BY 1",
        )
        .await;
        assert_eq!(
            columns,
            [
                "active boolean NO",
                "anonymised_at timestamp with time zone YES",
                "created_at timestamp with time zone NO",
                "created_by bigint YES",
                "deleted_at timestamp with time zone YES",
                "deleted_by bigint YES",
                "email text YES",
                "id bigint NO",
                "last_login_at timestamp with time zone YES",
                "must_change_password boolean NO",
                "name text NO",
                "password_hash text YES",
                "pid uuid NO",
                "temp_password_expires_at timestamp with time zone YES",
                "token_version integer NO",
                "totp_enabled_at timestamp with time zone YES",
                "totp_last_step bigint YES",
                "totp_secret_encrypted bytea YES",
                "updated_at timestamp with time zone NO",
                "updated_by bigint YES",
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn the_anonymisation_checks_fix_the_shape_of_a_row() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        execute(db, &insert(ANONYMISED, &anonymised_ok()))
            .await
            .expect("the anonymised shape of ADR 0006 §7");

        // Without an address a row must be anonymised; trashed is not enough.
        for (columns, values) in [
            ("email", "NULL".to_owned()),
            ("email, active, deleted_at", format!("NULL, false, {AT}")),
        ] {
            assert_refused(db, &insert(columns, &values), "users_email_present").await;
        }

        // The accepted shape with one column wrong.
        for (case, values) in [
            (
                "keeps an address",
                format!("'x@example.test', NULL, NULL, false, {AT}, {AT}"),
            ),
            (
                "keeps a hash",
                format!("NULL, 'x', NULL, false, {AT}, {AT}"),
            ),
            (
                "keeps a TOTP secret",
                format!("NULL, NULL, '\\x00', false, {AT}, {AT}"),
            ),
            ("is active", format!("NULL, NULL, NULL, true, {AT}, {AT}")),
            (
                "is not trashed",
                format!("NULL, NULL, NULL, false, NULL, {AT}"),
            ),
        ] {
            let error = execute(db, &insert(ANONYMISED, &values))
                .await
                .expect_err(case);
            assert!(
                error.to_string().contains("users_anonymised_shape"),
                "{case}: {error}"
            );
        }

        // An existing account cannot be half anonymised either.
        execute(
            db,
            &insert("email, deleted_at", &format!("'fel@example.test', {AT}")),
        )
        .await
        .expect("a trashed account");
        assert_refused(
            db,
            &format!("UPDATE users SET anonymised_at = {AT} WHERE email = 'fel@example.test'"),
            "users_anonymised_shape",
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn emails_and_pids_are_unique_across_trashed_rows() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        execute(db, &insert("email", "'Egyedi@Example.test'"))
            .await
            .expect("first account");
        let duplicate = insert("email", "'egyedi@example.TEST'");
        assert_refused(db, &duplicate, "users_lower_email_key").await;

        // A trashed account keeps its address until it is anonymised
        // (ADR 0011 §3).
        execute(db, &format!("UPDATE users SET deleted_at = {AT}"))
            .await
            .expect("trash it");
        assert_refused(db, &duplicate, "users_lower_email_key").await;

        // Anonymised rows hold no address, so any number of them coexist.
        for _ in 0..2 {
            execute(db, &insert(ANONYMISED, &anonymised_ok()))
                .await
                .expect("anonymised rows share the NULL address");
        }

        // The public id is unique too: it names one account in the JWT.
        let pid = "'3c2b1a09-8f7e-4d6c-9b5a-0f1e2d3c4b5a'";
        let with_pid = |email: &str| {
            format!("INSERT INTO users (pid, name, email) VALUES ({pid}, 'Próba', '{email}')")
        };
        execute(db, &with_pid("a@example.test"))
            .await
            .expect("first pid");
        assert_refused(db, &with_pid("b@example.test"), "users_pid_key").await;
    })
    .await;
}

#[tokio::test]
async fn every_audit_reference_is_indexed_and_restricts_deletes() {
    // ADR 0009 wants an index on every foreign key. The audit references
    // refuse the delete at once: not SET NULL, which would erase authorship,
    // nor NO ACTION, which checks only at the end of the statement.
    with_app(|_router, ctx| async move {
        let foreign_keys = texts(
            &ctx.db,
            "SELECT conname || ': ' || pg_get_constraintdef(oid) FROM pg_constraint \
             WHERE conrelid = 'users'::regclass AND contype = 'f' ORDER BY 1",
        )
        .await;
        assert_eq!(
            foreign_keys,
            ["created_by", "deleted_by", "updated_by"].map(|column| format!(
                "users_{column}_fkey: FOREIGN KEY ({column}) REFERENCES users(id) ON DELETE RESTRICT"
            ))
        );
        let indexes = texts(
            &ctx.db,
            "SELECT indexdef FROM pg_indexes WHERE tablename = 'users' ORDER BY indexname",
        )
        .await;
        assert_eq!(
            indexes,
            [
                "CREATE INDEX users_created_by_idx ON public.users USING btree (created_by)",
                "CREATE INDEX users_deleted_by_idx ON public.users USING btree (deleted_by)",
                "CREATE UNIQUE INDEX users_lower_email_key ON public.users USING btree (lower(email))",
                "CREATE UNIQUE INDEX users_pid_key ON public.users USING btree (pid)",
                "CREATE UNIQUE INDEX users_pkey ON public.users USING btree (id)",
                "CREATE INDEX users_updated_by_idx ON public.users USING btree (updated_by)",
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn an_account_that_authored_a_change_cannot_be_deleted() {
    // Accounts are anonymised, never deleted, so every audit reference stays
    // valid (ADR 0011 §6); the catalog test above pins the other two columns.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        execute(db, &insert("id, email", "1001, 'szerzo@example.test'"))
            .await
            .expect("author");
        execute(db, &insert("email, created_by", "'uj@example.test', 1001"))
            .await
            .expect("a row the author created");
        assert_refused(
            db,
            "DELETE FROM users WHERE id = 1001",
            "users_created_by_fkey",
        )
        .await;
        assert_refused(
            db,
            &insert("email, updated_by", "'arva@example.test', 999999"),
            "users_updated_by_fkey",
        )
        .await;
    })
    .await;
}
