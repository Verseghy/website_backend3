//! The `activity_log` migration (ADR 0011 §5, ADR 0009): the columns, the
//! CHECKs and foreign key that fix the shape of an entry, the three indexes,
//! and the SQL tests that ADR 0011's "What must be true" asks for: `DELETE`
//! and `TRUNCATE` always fail; `UPDATE` fails outside a permanent-delete
//! transaction, even when it matches no row, and, inside one, unless it only
//! sets `ip` to NULL.
//!
//! Rows are written in SQL here, so each test controls every column; the
//! model's own path is `tests/models/activity_log.rs`. The flag is set
//! through `models::activity_log::with_permanent_delete_flag`, so these tests
//! also prove that the model and the trigger agree on its name. Addresses
//! come from the documentation ranges of RFC 5737 and RFC 3849.
//!
//! The rollback is covered by `search_support`'s test, which migrates every
//! migration down and up again.

use backend3::models::activity_log::{PERMANENT_DELETE_FLAG, with_permanent_delete_flag};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait};

use crate::support::with_app;

/// The start of every refusal by the append-only triggers.
const APPEND_ONLY: &str = "activity_log is append-only: ";

async fn execute<C: ConnectionTrait>(db: &C, sql: &str) -> Result<u64, DbErr> {
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

/// The one value that `sql` returns.
async fn value<T: sea_orm::TryGetable>(db: &DatabaseConnection, sql: &str) -> T {
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect(sql)
        .expect("one row")
        .try_get_by_index(0)
        .expect(sql)
}

/// Runs `sql`, expects PostgreSQL to refuse it, and checks that the refusal
/// contains `reason`: a constraint name or a trigger message.
async fn assert_refused(db: &DatabaseConnection, sql: &str, reason: &str) {
    let error = execute(db, sql)
        .await
        .expect_err(&format!("refused ({reason}): {sql}"));
    assert!(
        error.to_string().contains(reason),
        "{sql}: expected {reason:?}, got: {error}"
    );
}

/// Runs `sql` in a transaction that has set the permanent-delete flag, and
/// commits it if the statement succeeds.
async fn in_permanent_delete_transaction(db: &DatabaseConnection, sql: &str) -> Result<u64, DbErr> {
    let txn = db.begin().await.expect("begin");
    with_permanent_delete_flag(&txn)
        .await
        .expect("set the flag");
    let outcome = execute(&txn, sql).await;
    if outcome.is_ok() {
        txn.commit().await.expect("commit");
    }
    outcome
}

/// Like [`assert_refused`], inside a permanent-delete transaction.
async fn assert_refused_with_flag(db: &DatabaseConnection, sql: &str, reason: &str) {
    let error = in_permanent_delete_transaction(db, sql)
        .await
        .expect_err(&format!("refused with the flag ({reason}): {sql}"));
    assert!(
        error.to_string().contains(reason),
        "{sql}: expected {reason:?}, got: {error}"
    );
}

/// Inserts a sign-in entry of the system actor about account 7, with
/// details, a request id and an address (and so no label, which a sign-in
/// entry may not hold), and returns its id.
async fn insert_entry(db: &DatabaseConnection) -> i64 {
    value(
        db,
        "INSERT INTO activity_log (actor_kind, action, entity_type, entity_id, details, \
         request_id, ip) VALUES ('system', 'login', 'users', 7, '{\"count\": 1}', 'req-1', \
         '192.0.2.10') RETURNING id",
    )
    .await
}

/// Every column of entry `id` as one text, to compare before and after a
/// refused statement.
async fn row_text(db: &DatabaseConnection, id: i64) -> String {
    value(
        db,
        &format!("SELECT activity_log::text FROM activity_log WHERE id = {id}"),
    )
    .await
}

async fn entry_count(db: &DatabaseConnection) -> i64 {
    value(db, "SELECT count(*) FROM activity_log").await
}

#[tokio::test]
async fn the_table_has_exactly_the_columns_of_adr_0011() {
    with_app(|_router, ctx| async move {
        let columns = texts(
            &ctx.db,
            "SELECT column_name || ' ' || data_type || ' ' || is_nullable \
             FROM information_schema.columns WHERE table_name = 'activity_log' \
             ORDER BY ordinal_position",
        )
        .await;
        assert_eq!(
            columns,
            [
                "id bigint NO",
                "occurred_at timestamp with time zone NO",
                "actor_id bigint YES",
                "actor_kind text NO",
                "action text NO",
                "entity_type text YES",
                "entity_id bigint YES",
                "revision_id bigint YES",
                "label text YES",
                "details jsonb YES",
                "request_id text YES",
                "ip inet YES",
            ]
        );
        let indexes = texts(
            &ctx.db,
            "SELECT indexdef FROM pg_indexes WHERE tablename = 'activity_log' ORDER BY indexname",
        )
        .await;
        assert_eq!(
            indexes,
            [
                "CREATE INDEX activity_log_actor_id_occurred_at_idx ON public.activity_log \
                 USING btree (actor_id, occurred_at DESC)",
                "CREATE INDEX activity_log_entity_type_entity_id_occurred_at_idx ON \
                 public.activity_log USING btree (entity_type, entity_id, occurred_at DESC)",
                "CREATE INDEX activity_log_occurred_at_idx ON public.activity_log \
                 USING btree (occurred_at DESC)",
                "CREATE UNIQUE INDEX activity_log_pkey ON public.activity_log USING btree (id)",
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn the_checks_fix_the_shape_of_an_entry() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user: i64 = value(
            db,
            "INSERT INTO users (pid, name, email) \
             VALUES (gen_random_uuid(), 'Próba', 'proba@example.test') RETURNING id",
        )
        .await;
        let insert = |columns: &str, values: &str| {
            format!("INSERT INTO activity_log ({columns}) VALUES ({values})")
        };
        // The accepted shapes: each actor kind, a table without a record, a
        // record with its revision, and an object in details.
        for (columns, values) in [
            (
                "actor_kind, actor_id, action",
                format!("'user', {user}, 'update'"),
            ),
            ("actor_kind, action", "'system', 'purge'".to_owned()),
            (
                "actor_kind, action, entity_type",
                "'import', 'import', 'labels'".to_owned(),
            ),
            (
                "actor_kind, action, entity_type, entity_id, revision_id, details",
                "'system', 'update', 'posts', 7, 3, '{}'".to_owned(),
            ),
        ] {
            execute(db, &insert(columns, &values))
                .await
                .unwrap_or_else(|error| panic!("{values}: {error}"));
        }
        for (columns, values, constraint) in [
            (
                "actor_kind, action",
                "'robot', 'update'",
                "activity_log_actor_kind_known",
            ),
            (
                "actor_kind, action",
                "'user', 'update'",
                "activity_log_actor_shape",
            ),
            (
                "actor_kind, actor_id, action",
                "'system', 1, 'update'",
                "activity_log_actor_shape",
            ),
            (
                "actor_kind, actor_id, action",
                "'import', 1, 'import'",
                "activity_log_actor_shape",
            ),
            (
                "actor_kind, action",
                "'system', 'Update'",
                "activity_log_action_known",
            ),
            (
                "actor_kind, action",
                "'system', 'archive'",
                "activity_log_action_known",
            ),
            (
                "actor_kind, action, entity_id",
                "'system', 'update', 7",
                "activity_log_entity_shape",
            ),
            (
                "actor_kind, action, entity_type, revision_id",
                "'system', 'update', 'posts', 3",
                "activity_log_revision_shape",
            ),
            (
                "actor_kind, action, details",
                "'system', 'update', '[\"name\"]'",
                "activity_log_details_object",
            ),
            (
                "actor_kind, action, details",
                "'system', 'update', '\"name\"'",
                "activity_log_details_object",
            ),
            (
                "actor_kind, action, details",
                "'system', 'update', 'null'",
                "activity_log_details_object",
            ),
            (
                "actor_kind, actor_id, action",
                "'user', 999999, 'update'",
                "activity_log_actor_id_fkey",
            ),
        ] {
            assert_refused(db, &insert(columns, values), constraint).await;
        }
    })
    .await;
}

#[tokio::test]
async fn the_checks_keep_personal_data_out_of_an_entry() {
    // An entry can never be corrected and only its ip can be cleared, so an
    // address only on an account action and a label only about a content
    // record hold for every INSERT, not only for the model's (ADR 0011 §5).
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let insert = |columns: &str, values: &str| {
            format!("INSERT INTO activity_log ({columns}) VALUES ({values})")
        };
        for (columns, values) in [
            (
                "actor_kind, action, entity_type, entity_id, label",
                "'system', 'update', 'posts', 7, 'Szalagavató'",
            ),
            // A whole table names its entity type, so it may have a label.
            (
                "actor_kind, action, entity_type, label",
                "'system', 'reorder', 'menu_items', 'Főmenü'",
            ),
            (
                "actor_kind, action, entity_type, entity_id, ip",
                "'system', 'login_failed', 'users', 7, '192.0.2.30'",
            ),
            (
                "actor_kind, action, ip",
                "'system', 'logout', '2001:db8::31'",
            ),
        ] {
            execute(db, &insert(columns, values))
                .await
                .unwrap_or_else(|error| panic!("{values}: {error}"));
        }
        for (columns, values, constraint) in [
            (
                "actor_kind, action, entity_type, entity_id, ip",
                "'system', 'update', 'posts', 7, '192.0.2.30'",
                "activity_log_ip_account_only",
            ),
            (
                "actor_kind, action, ip",
                "'system', 'media_upload', '2001:db8::30'",
                "activity_log_ip_account_only",
            ),
            (
                "actor_kind, action, entity_type, entity_id, label",
                "'system', 'login', 'users', 7, 'Kitalált Személy'",
                "activity_log_label_content_only",
            ),
            (
                "actor_kind, action, entity_type, entity_id, label",
                "'system', 'roles_changed', 'posts', 7, 'Kitalált Személy'",
                "activity_log_label_content_only",
            ),
            (
                "actor_kind, action, label",
                "'system', 'publish', 'Kitalált Személy'",
                "activity_log_label_content_only",
            ),
            (
                "actor_kind, action, entity_type, entity_id, label",
                "'system', 'update', 'authors', 4, 'Kitalált Személy'",
                "activity_log_label_content_only",
            ),
            (
                "actor_kind, action, entity_type, entity_id, label",
                "'system', 'update', 'colleagues', 4, 'Kitalált Személy'",
                "activity_log_label_content_only",
            ),
        ] {
            assert_refused(db, &insert(columns, values), constraint).await;
        }
        assert_eq!(entry_count(db).await, 4);
    })
    .await;
}

#[tokio::test]
async fn an_account_that_has_entries_cannot_be_deleted() {
    // Accounts are anonymised, never deleted (ADR 0011 §6): the key refuses
    // at once rather than erasing who did what.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user: i64 = value(
            db,
            "INSERT INTO users (pid, name, email) \
             VALUES (gen_random_uuid(), 'Próba', 'proba@example.test') RETURNING id",
        )
        .await;
        execute(
            db,
            &format!(
                "INSERT INTO activity_log (actor_kind, actor_id, action) \
                 VALUES ('user', {user}, 'login')"
            ),
        )
        .await
        .expect("an entry of the account");
        assert_eq!(
            texts(
                db,
                "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
                 WHERE conname = 'activity_log_actor_id_fkey'",
            )
            .await,
            ["FOREIGN KEY (actor_id) REFERENCES users(id) ON DELETE RESTRICT"]
        );
        assert_refused(
            db,
            &format!("DELETE FROM users WHERE id = {user}"),
            "activity_log_actor_id_fkey",
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn occurred_at_is_the_start_of_the_writing_transaction() {
    // PostgreSQL's clock (research: DECISIONS R15, D90): every entry of one
    // transaction carries the transaction's start, as do the audit columns
    // of the change it records.
    with_app(|_router, ctx| async move {
        let txn = ctx.db.begin().await.expect("begin");
        let start: String = txn
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT now()::text",
            ))
            .await
            .expect("now")
            .expect("one row")
            .try_get_by_index(0)
            .expect("text");
        execute(
            &txn,
            "INSERT INTO activity_log (actor_kind, action) VALUES ('system', 'reorder'), \
             ('system', 'publish')",
        )
        .await
        .expect("two entries");
        let stamps: Vec<String> = txn
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT occurred_at::text FROM activity_log ORDER BY id",
            ))
            .await
            .expect("stamps")
            .iter()
            .map(|row| row.try_get_by_index(0).expect("text"))
            .collect();
        assert_eq!(stamps, [start.clone(), start]);
        txn.commit().await.expect("commit");
    })
    .await;
}

#[tokio::test]
async fn delete_is_always_refused() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let id = insert_entry(db).await;
        let refusal = format!("{APPEND_ONLY}DELETE is never allowed (ADR 0011 §5)");
        for sql in [
            format!("DELETE FROM activity_log WHERE id = {id}"),
            "DELETE FROM activity_log".to_owned(),
            // Statement-level: refused although no row matches.
            "DELETE FROM activity_log WHERE false".to_owned(),
            // A DELETE spelled as MERGE fires the same trigger.
            format!(
                "MERGE INTO activity_log AS entry USING (SELECT {id} AS id) AS source \
                 ON entry.id = source.id WHEN MATCHED THEN DELETE"
            ),
        ] {
            assert_refused(db, &sql, &refusal).await;
            // The permanent-delete flag does not open DELETE.
            assert_refused_with_flag(db, &sql, &refusal).await;
        }
        assert_eq!(entry_count(db).await, 1);
    })
    .await;
}

#[tokio::test]
async fn truncate_is_always_refused_also_through_users() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        insert_entry(db).await;
        let refusal = format!("{APPEND_ONLY}TRUNCATE is never allowed (ADR 0011 §5)");
        for sql in [
            "TRUNCATE activity_log",
            "TRUNCATE ONLY activity_log RESTART IDENTITY",
            // Emptying the accounts would cascade to the log, so the same
            // trigger refuses it (why App::truncate never covers users).
            "TRUNCATE users CASCADE",
        ] {
            assert_refused(db, sql, &refusal).await;
            assert_refused_with_flag(db, sql, &refusal).await;
        }
        assert_eq!(entry_count(db).await, 1);
    })
    .await;
}

#[tokio::test]
async fn update_is_refused_outside_a_permanent_delete_transaction() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let id = insert_entry(db).await;
        let before = row_text(db, id).await;
        let refusal = format!(
            "{APPEND_ONLY}UPDATE is allowed only in a permanent-delete transaction, which sets \
             backend3.permanent_delete with SET LOCAL (ADR 0011 §5)"
        );
        let clear_ip = format!("UPDATE activity_log SET ip = NULL WHERE id = {id}");
        for sql in [
            // Even the one change a permanent-delete transaction may make.
            clear_ip.clone(),
            format!("UPDATE activity_log SET label = 'Más' WHERE id = {id}"),
            format!("UPDATE activity_log SET occurred_at = occurred_at WHERE id = {id}"),
            // An UPDATE spelled as an upsert or a MERGE.
            format!(
                "INSERT INTO activity_log (id, actor_kind, action) VALUES ({id}, 'system', \
                 'import') ON CONFLICT (id) DO UPDATE SET label = 'Más'"
            ),
            format!(
                "MERGE INTO activity_log AS entry USING (SELECT {id} AS id) AS source \
                 ON entry.id = source.id WHEN MATCHED THEN UPDATE SET ip = NULL"
            ),
            // Statement-level: refused although no row matches, as DELETE
            // is; so is an upsert that conflicts with nothing, and a MERGE
            // that matches nothing.
            "UPDATE activity_log SET label = 'x' WHERE false".to_owned(),
            "UPDATE activity_log SET ip = NULL WHERE false".to_owned(),
            "INSERT INTO activity_log (actor_kind, action) VALUES ('system', 'import') \
             ON CONFLICT (id) DO UPDATE SET label = 'Más'"
                .to_owned(),
            "MERGE INTO activity_log AS entry USING (SELECT 1 AS id) AS source ON false \
             WHEN MATCHED THEN UPDATE SET ip = NULL"
                .to_owned(),
        ] {
            assert_refused(db, &sql, &refusal).await;
        }
        assert_eq!(
            entry_count(db).await,
            1,
            "the refused upsert inserted nothing"
        );
        // The INSERT forms that fire no UPDATE trigger are not refused.
        for sql in [
            "INSERT INTO activity_log (actor_kind, action) VALUES ('system', 'import') \
             ON CONFLICT (id) DO NOTHING",
            "MERGE INTO activity_log AS entry USING (SELECT -1 AS id) AS source \
             ON entry.id = source.id WHEN NOT MATCHED THEN \
             INSERT (actor_kind, action) VALUES ('system', 'import')",
        ] {
            assert_eq!(execute(db, sql).await.expect(sql), 1, "{sql}");
        }
        // Only `on` opens it: any other value of the flag does not.
        for setting in ["off", "true", "1", "ON", ""] {
            let txn = db.begin().await.expect("begin");
            execute(
                &txn,
                &format!("SET LOCAL {PERMANENT_DELETE_FLAG} = '{setting}'"),
            )
            .await
            .expect("set the flag");
            let error = execute(&txn, &clear_ip)
                .await
                .expect_err(&format!("flag {setting:?}"));
            assert!(error.to_string().contains(&refusal), "{setting:?}: {error}");
            txn.rollback().await.expect("rollback");
        }
        // The flag ends with its transaction, committed or rolled back: one
        // simple query keeps all of it on one connection, so the UPDATE runs
        // in the same session right after the flagged transaction.
        for end in ["COMMIT", "ROLLBACK"] {
            let sql = format!("BEGIN; SET LOCAL {PERMANENT_DELETE_FLAG} = 'on'; {end}; {clear_ip}");
            let error = ctx
                .db
                .execute_unprepared(&sql)
                .await
                .expect_err(&format!("after {end}"));
            assert!(error.to_string().contains(&refusal), "after {end}: {error}");
        }
        assert_eq!(row_text(db, id).await, before);
    })
    .await;
}

#[tokio::test]
async fn a_permanent_delete_transaction_may_only_set_ip_to_null() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let id = insert_entry(db).await;
        let before = row_text(db, id).await;
        let refusal = format!(
            "{APPEND_ONLY}a permanent-delete transaction may only set ip to NULL, and this \
             UPDATE changes entry {id} otherwise (ADR 0011 §5)"
        );
        for sql in [
            format!("UPDATE activity_log SET ip = '192.0.2.11' WHERE id = {id}"),
            format!("UPDATE activity_log SET ip = '2001:db8::1' WHERE id = {id}"),
            format!("UPDATE activity_log SET ip = NULL, label = 'Más' WHERE id = {id}"),
            format!("UPDATE activity_log SET details = NULL WHERE id = {id}"),
            format!("UPDATE activity_log SET request_id = NULL WHERE id = {id}"),
            format!(
                "UPDATE activity_log SET ip = NULL, occurred_at = occurred_at + \
                 interval '1 microsecond' WHERE id = {id}"
            ),
            format!("UPDATE activity_log SET ip = NULL, id = id + 1000 WHERE id = {id}"),
            // Equal under `=`, but stored differently: the binary row
            // comparison refuses it too.
            format!(
                "UPDATE activity_log SET ip = NULL, details = '{{\"count\": 1.0}}' WHERE id = {id}"
            ),
        ] {
            assert_refused_with_flag(db, &sql, &refusal).await;
        }
        assert_eq!(row_text(db, id).await, before);

        // The anonymisation's UPDATE (ADR 0011 §6): only ip changes.
        let clear_ip = format!("UPDATE activity_log SET ip = NULL WHERE id = {id}");
        assert_eq!(
            in_permanent_delete_transaction(db, &clear_ip)
                .await
                .expect("clear ip"),
            1
        );
        assert_eq!(row_text(db, id).await, before.replace(",192.0.2.10)", ",)"));
        // Clearing it again changes nothing and is allowed.
        assert_eq!(
            in_permanent_delete_transaction(db, &clear_ip)
                .await
                .expect("again"),
            1
        );
        // So is an UPDATE that sets every column to its own value besides.
        in_permanent_delete_transaction(
            db,
            &format!(
                "UPDATE activity_log SET ip = NULL, label = label, details = details \
                 WHERE id = {id}"
            ),
        )
        .await
        .expect("no other change");
        // And one that matches no row: it changes nothing.
        assert_eq!(
            in_permanent_delete_transaction(db, "UPDATE activity_log SET ip = NULL WHERE false")
                .await
                .expect("no row"),
            0
        );
        assert_eq!(
            value::<Option<String>>(
                db,
                &format!("SELECT host(ip) FROM activity_log WHERE id = {id}")
            )
            .await,
            None
        );
    })
    .await;
}

#[tokio::test]
async fn a_large_entry_can_have_its_address_cleared() {
    // A `details` large enough to be compressed and stored out of line
    // still compares equal to itself byte for byte.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let id: i64 = value(
            db,
            "INSERT INTO activity_log (actor_kind, action, details, ip) VALUES ('system', \
             'login', jsonb_build_object('paths', repeat('uploads/posts/7/', 20000)), \
             '192.0.2.12') RETURNING id",
        )
        .await;
        assert!(
            value::<i32>(
                db,
                &format!("SELECT pg_column_size(details) FROM activity_log WHERE id = {id}")
            )
            .await
                < 20000,
            "details is compressed"
        );
        in_permanent_delete_transaction(
            db,
            &format!("UPDATE activity_log SET ip = NULL WHERE id = {id}"),
        )
        .await
        .expect("clear ip");
    })
    .await;
}
