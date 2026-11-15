//! The `labels` migration (ADR 0009, ADR 0011; parity LBL-01): the column
//! set, the colour CHECK, the case-insensitive name key over live rows only,
//! and the audit columns' references to `users`. Rows are written in SQL,
//! so each test controls every column. Every name is invented.
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

/// `INSERT INTO labels (name, color)` of one row; `name` and `color` are
/// SQL literals.
fn insert(name: &str, color: &str) -> String {
    format!("INSERT INTO labels (name, color) VALUES ({name}, {color})")
}

/// The instant these tests write to `deleted_at`, as a SQL literal. Only
/// whether it is NULL matters, so a fixed instant keeps the tests off the
/// database's clock.
const AT: &str = "TIMESTAMPTZ '2026-11-15 08:00:00+00'";

#[tokio::test]
async fn the_table_has_exactly_the_planned_columns() {
    with_app(|_router, ctx| async move {
        let columns = texts(
            &ctx.db,
            "SELECT column_name || ' ' || data_type || ' ' || is_nullable \
             FROM information_schema.columns WHERE table_name = 'labels' ORDER BY 1",
        )
        .await;
        assert_eq!(
            columns,
            [
                "color text NO",
                "created_at timestamp with time zone NO",
                "created_by bigint YES",
                "deleted_at timestamp with time zone YES",
                "deleted_by bigint YES",
                "id bigint NO",
                "name text NO",
                "updated_at timestamp with time zone NO",
                "updated_by bigint YES",
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn the_colour_is_hash_and_six_hex_digits() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        for (index, color) in ["'#1e88e5'", "'#1E88E5'", "'#000000'", "'#aBcDeF'"]
            .into_iter()
            .enumerate()
        {
            execute(db, &insert(&format!("'Címke {index}'"), color))
                .await
                .unwrap_or_else(|error| panic!("{color}: {error}"));
        }
        for color in [
            "'1E88E5'",
            "'#1E88E'",
            "'#1E88E55'",
            "'#GGGGGG'",
            "'#1e88e5 '",
            "' #1e88e5'",
            // `$` ends the text in PostgreSQL's regular expressions, so a
            // trailing line break does not pass.
            "E'#1e88e5\\n'",
            "'#１２３４５６'",
            "''",
            "'red'",
        ] {
            assert_refused(db, &insert("'Hibás'", color), "labels_color_format").await;
        }
        // The colour is required.
        assert_refused(db, &insert("'Hiányzó'", "NULL"), "color").await;
    })
    .await;
}

#[tokio::test]
async fn a_name_is_unique_among_live_labels_whatever_its_case() {
    // ADR 0009 "Constraints" and ADR 0011 §3: unique on lower(name) among
    // the rows not in the trash.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        execute(db, &insert("'Ünnepség'", "'#123456'"))
            .await
            .expect("first label");
        for duplicate in ["'Ünnepség'", "'ünnepség'", "'ÜNNEPSÉG'"] {
            assert_refused(
                db,
                &insert(duplicate, "'#654321'"),
                "labels_live_lower_name_key",
            )
            .await;
        }
        // Another word is another name.
        execute(db, &insert("'Ünnepségek'", "'#654321'"))
            .await
            .expect("a different name");

        // A trashed label keeps its name without blocking a live one ...
        execute(
            db,
            &format!("UPDATE labels SET deleted_at = {AT} WHERE name = 'Ünnepség'"),
        )
        .await
        .expect("trash it");
        execute(db, &insert("'ünnepség'", "'#654321'"))
            .await
            .expect("the name is free among live labels");
        // ... and two trashed labels may share one.
        execute(
            db,
            &format!("UPDATE labels SET deleted_at = {AT} WHERE name = 'ünnepség'"),
        )
        .await
        .expect("trash the second");
        // Restoring one while a live label holds the name is refused.
        execute(db, &insert("'ÜNNEPSÉG'", "'#654321'"))
            .await
            .expect("a third, live");
        assert_refused(
            db,
            "UPDATE labels SET deleted_at = NULL WHERE name = 'Ünnepség'",
            "labels_live_lower_name_key",
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn the_audit_columns_reference_accounts_that_cannot_be_deleted() {
    // ADR 0011 §2, §6: ON DELETE RESTRICT, as `users`' own audit columns.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        execute(
            db,
            "INSERT INTO users (pid, name, email) \
             VALUES (gen_random_uuid(), 'Próba', 'cimke@example.test')",
        )
        .await
        .expect("an account");
        for column in ["created_by", "updated_by", "deleted_by"] {
            execute(db, "DELETE FROM labels").await.expect("empty");
            execute(
                db,
                &format!(
                    "INSERT INTO labels (name, color, {column}) \
                     SELECT 'Hírek', '#123456', id FROM users"
                ),
            )
            .await
            .unwrap_or_else(|error| panic!("{column}: {error}"));
            assert_refused(db, "DELETE FROM users", &format!("labels_{column}_fkey")).await;
            // An id that names no account is refused too.
            assert_refused(
                db,
                &format!(
                    "INSERT INTO labels (name, color, {column}) VALUES ('Más', '#123456', -1)"
                ),
                &format!("labels_{column}_fkey"),
            )
            .await;
        }
        // Every foreign key has its index (ADR 0009, "Constraints").
        let indexes = texts(
            db,
            "SELECT indexname FROM pg_indexes WHERE tablename = 'labels' ORDER BY 1",
        )
        .await;
        assert_eq!(
            indexes,
            [
                "labels_created_by_idx",
                "labels_deleted_by_idx",
                "labels_live_lower_name_key",
                "labels_pkey",
                "labels_updated_by_idx",
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn ids_are_an_identity_that_takes_explicit_legacy_ids() {
    // ADR 0009, primary-key rules 1 and 2: the importer inserts legacy ids.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        execute(
            db,
            "INSERT INTO labels (id, name, color) VALUES (42, 'Régi', '#123456')",
        )
        .await
        .expect("an explicit id");
        assert_eq!(texts(db, "SELECT id::text FROM labels").await, ["42"]);
        assert_eq!(
            texts(
                db,
                "SELECT data_type || ' ' || is_identity || ' ' || identity_generation \
                 FROM information_schema.columns WHERE table_name = 'labels' AND column_name = 'id'"
            )
            .await,
            ["bigint YES BY DEFAULT"]
        );
    })
    .await;
}
