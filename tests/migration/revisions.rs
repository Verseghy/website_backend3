//! The `revisions` migration (ADR 0011 §4, ADR 0009): the columns, the
//! CHECKs and foreign key that fix the shape of a revision, the unique index
//! on `(entity_type, entity_id, revision_no DESC)`, and the SQL tests that
//! ADR 0011's "What must be true" asks for: `UPDATE` and `TRUNCATE` always
//! fail; `DELETE` fails outside a permanent-delete transaction, even when it
//! matches no row, and succeeds inside one. One more test pins the limit
//! that the migration documents: when a statement nests the `DELETE` as a
//! `WITH` query, a flag it sets before that `DELETE` runs counts like
//! `SET LOCAL`.
//!
//! Rows are written in SQL here, so each test controls every column; the
//! model's own path is `tests/models/revisions.rs`. The flag is set through
//! `models::activity_log::with_permanent_delete_flag`, the one the
//! `activity_log` triggers read too, so these tests also prove that the
//! model and both tables agree on its name.
//!
//! The rollback is covered by `search_support`'s test, which migrates every
//! migration down and up again.

use backend3::models::activity_log::{PERMANENT_DELETE_FLAG, with_permanent_delete_flag};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait};

use crate::support::with_app;

/// The start of every refusal by the append-only triggers.
const APPEND_ONLY: &str = "revisions is append-only: ";

/// The refusal of a `DELETE` outside a permanent-delete transaction.
const DELETE_REFUSAL: &str = "revisions is append-only: DELETE is allowed only in a \
                              permanent-delete transaction, which sets backend3.permanent_delete \
                              with SET LOCAL (ADR 0011 §5)";

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
async fn value<C, T>(db: &C, sql: &str) -> T
where
    C: ConnectionTrait,
    T: sea_orm::TryGetable,
{
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

/// Inserts revision `revision_no` of post 7, with a request id, and returns
/// its id.
async fn insert_revision(db: &DatabaseConnection, revision_no: i32) -> i64 {
    value(
        db,
        &format!(
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot, request_id) VALUES ('posts', 7, {revision_no}, 'update', 1, \
             '{{\"title\": \"Szalagavató\"}}', 'req-1') RETURNING id"
        ),
    )
    .await
}

/// Every column of revision `id` as one text, to compare before and after a
/// refused statement.
async fn row_text(db: &DatabaseConnection, id: i64) -> String {
    value(
        db,
        &format!("SELECT revisions::text FROM revisions WHERE id = {id}"),
    )
    .await
}

async fn revision_count(db: &DatabaseConnection) -> i64 {
    value(db, "SELECT count(*) FROM revisions").await
}

/// Inserts an invented account and returns its id.
async fn insert_user(db: &DatabaseConnection) -> i64 {
    value(
        db,
        "INSERT INTO users (pid, name, email) \
         VALUES (gen_random_uuid(), 'Próba', 'proba@example.test') RETURNING id",
    )
    .await
}

#[tokio::test]
async fn the_table_has_exactly_the_columns_of_adr_0011() {
    with_app(|_router, ctx| async move {
        let columns = texts(
            &ctx.db,
            "SELECT column_name || ' ' || data_type || ' ' || is_nullable \
             FROM information_schema.columns WHERE table_name = 'revisions' \
             ORDER BY ordinal_position",
        )
        .await;
        assert_eq!(
            columns,
            [
                "id bigint NO",
                "entity_type text NO",
                "entity_id bigint NO",
                "revision_no integer NO",
                "reason text NO",
                "schema_version integer NO",
                "snapshot jsonb NO",
                "created_at timestamp with time zone NO",
                "created_by bigint YES",
                "request_id text YES",
            ]
        );
        let indexes = texts(
            &ctx.db,
            "SELECT indexdef FROM pg_indexes WHERE tablename = 'revisions' ORDER BY indexname",
        )
        .await;
        assert_eq!(
            indexes,
            [
                "CREATE INDEX revisions_created_by_idx ON public.revisions USING btree \
                 (created_by)",
                "CREATE UNIQUE INDEX revisions_entity_type_entity_id_revision_no_idx ON \
                 public.revisions USING btree (entity_type, entity_id, revision_no DESC)",
                "CREATE UNIQUE INDEX revisions_pkey ON public.revisions USING btree (id)",
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn the_checks_fix_the_shape_of_a_revision() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = insert_user(db).await;
        let insert = |values: &str| {
            format!(
                "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, \
                 schema_version, snapshot, created_by) VALUES ({values})"
            )
        };
        // The accepted shapes: every reason, a user's save and the system's,
        // the importer's two reasons without a user.
        for values in [
            format!("'posts', 7, 1, 'create', 1, '{{}}', {user}"),
            format!("'posts', 7, 2, 'update', 1, '{{\"title\": \"x\"}}', {user}"),
            "'posts', 7, 3, 'revert', 2, '{}', NULL".to_owned(),
            "'labels', 3, 1, 'import_original', 1, '{}', NULL".to_owned(),
            "'labels', 3, 2, 'import', 1, '{}', NULL".to_owned(),
            // The same number on another record, of the same or another type.
            "'posts', 8, 1, 'create', 1, '{}', NULL".to_owned(),
            "'pages', 7, 1, 'create', 1, '{}', NULL".to_owned(),
        ] {
            execute(db, &insert(&values))
                .await
                .unwrap_or_else(|error| panic!("{values}: {error}"));
        }
        for (values, constraint) in [
            (
                "'posts', 9, 1, 'delete', 1, '{}', NULL".to_owned(),
                "revisions_reason_known",
            ),
            (
                "'posts', 9, 1, 'Create', 1, '{}', NULL".to_owned(),
                "revisions_reason_known",
            ),
            (
                "'posts', 9, 0, 'create', 1, '{}', NULL".to_owned(),
                "revisions_revision_no_positive",
            ),
            (
                "'posts', 9, -1, 'create', 1, '{}', NULL".to_owned(),
                "revisions_revision_no_positive",
            ),
            (
                "'posts', 9, 1, 'create', 0, '{}', NULL".to_owned(),
                "revisions_schema_version_positive",
            ),
            (
                "'posts', 9, 1, 'create', 1, '[\"title\"]', NULL".to_owned(),
                "revisions_snapshot_object",
            ),
            (
                "'posts', 9, 1, 'create', 1, '\"title\"', NULL".to_owned(),
                "revisions_snapshot_object",
            ),
            (
                "'posts', 9, 1, 'create', 1, 'null', NULL".to_owned(),
                "revisions_snapshot_object",
            ),
            (
                format!("'labels', 4, 1, 'import', 1, '{{}}', {user}"),
                "revisions_import_without_user",
            ),
            (
                format!("'labels', 4, 1, 'import_original', 1, '{{}}', {user}"),
                "revisions_import_without_user",
            ),
            (
                "'posts', 9, 1, 'create', 1, '{}', 999999".to_owned(),
                "revisions_created_by_fkey",
            ),
            // One number once per record.
            (
                "'posts', 7, 2, 'update', 1, '{}', NULL".to_owned(),
                "revisions_entity_type_entity_id_revision_no_idx",
            ),
        ] {
            assert_refused(db, &insert(&values), constraint).await;
        }
        assert_refused(
            db,
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version) \
             VALUES ('posts', 9, 1, 'create', 1)",
            "snapshot",
        )
        .await;
        assert_eq!(revision_count(db).await, 7);
    })
    .await;
}

#[tokio::test]
async fn an_account_that_saved_revisions_cannot_be_deleted() {
    // Accounts are anonymised, never deleted (ADR 0011 §6).
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = insert_user(db).await;
        execute(
            db,
            &format!(
                "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, \
                 schema_version, snapshot, created_by) VALUES ('posts', 7, 1, 'create', 1, '{{}}', \
                 {user})"
            ),
        )
        .await
        .expect("a revision of the account");
        assert_eq!(
            texts(
                db,
                "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
                 WHERE conname = 'revisions_created_by_fkey'",
            )
            .await,
            ["FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE RESTRICT"]
        );
        assert_refused(
            db,
            &format!("DELETE FROM users WHERE id = {user}"),
            "revisions_created_by_fkey",
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn created_at_is_the_start_of_the_writing_transaction() {
    // PostgreSQL's clock (research: DECISIONS R15, D90): every revision of
    // one transaction carries the transaction's start.
    with_app(|_router, ctx| async move {
        let txn = ctx.db.begin().await.expect("begin");
        let start: String = value(&txn, "SELECT now()::text").await;
        execute(
            &txn,
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('posts', 7, 1, 'create', 1, '{}'), ('pages', 2, 1, 'create', 1, \
             '{}')",
        )
        .await
        .expect("two revisions");
        let stamps: Vec<String> = txn
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT created_at::text FROM revisions ORDER BY id",
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
async fn update_is_always_refused() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let id = insert_revision(db, 1).await;
        let before = row_text(db, id).await;
        let refusal = format!("{APPEND_ONLY}UPDATE is never allowed (ADR 0011 §5)");
        for sql in [
            format!("UPDATE revisions SET snapshot = '{{}}' WHERE id = {id}"),
            format!("UPDATE revisions SET request_id = NULL WHERE id = {id}"),
            format!("UPDATE revisions SET created_at = created_at WHERE id = {id}"),
            "UPDATE revisions SET reason = 'revert'".to_owned(),
            // An UPDATE spelled as an upsert or a MERGE.
            format!(
                "INSERT INTO revisions (id, entity_type, entity_id, revision_no, reason, \
                 schema_version, snapshot) VALUES ({id}, 'posts', 7, 1, 'update', 1, '{{}}') \
                 ON CONFLICT (id) DO UPDATE SET snapshot = '{{}}'"
            ),
            format!(
                "MERGE INTO revisions AS revision USING (SELECT {id} AS id) AS source \
                 ON revision.id = source.id WHEN MATCHED THEN UPDATE SET snapshot = '{{}}'"
            ),
            // Statement-level: refused although no row matches; so is an
            // upsert that conflicts with nothing, and a MERGE that matches
            // nothing.
            "UPDATE revisions SET snapshot = '{}' WHERE false".to_owned(),
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('posts', 8, 1, 'create', 1, '{}') ON CONFLICT (id) DO UPDATE \
             SET snapshot = '{}'"
                .to_owned(),
            "MERGE INTO revisions AS revision USING (SELECT 1 AS id) AS source ON false \
             WHEN MATCHED THEN UPDATE SET snapshot = '{}'"
                .to_owned(),
        ] {
            assert_refused(db, &sql, &refusal).await;
            // The permanent-delete flag does not open UPDATE: a revision is
            // removed whole or kept as written.
            assert_refused_with_flag(db, &sql, &refusal).await;
        }
        assert_eq!(
            revision_count(db).await,
            1,
            "the refused upsert inserted nothing"
        );
        assert_eq!(row_text(db, id).await, before);
        // The INSERT forms that fire no UPDATE trigger are not refused.
        for sql in [
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('posts', 7, 2, 'update', 1, '{}') ON CONFLICT DO NOTHING",
            "MERGE INTO revisions AS revision USING (SELECT -1 AS id) AS source \
             ON revision.id = source.id WHEN NOT MATCHED THEN INSERT (entity_type, entity_id, \
             revision_no, reason, schema_version, snapshot) VALUES ('posts', 7, 3, 'update', 1, \
             '{}')",
        ] {
            assert_eq!(execute(db, sql).await.expect(sql), 1, "{sql}");
        }
    })
    .await;
}

#[tokio::test]
async fn truncate_is_always_refused_also_through_users() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        insert_revision(db, 1).await;
        let refusal = format!("{APPEND_ONLY}TRUNCATE is never allowed (ADR 0011 §5)");
        for sql in ["TRUNCATE revisions", "TRUNCATE ONLY revisions RESTART IDENTITY"] {
            assert_refused(db, sql, &refusal).await;
            assert_refused_with_flag(db, sql, &refusal).await;
        }
        // Emptying the accounts would cascade to the revisions, so their
        // trigger refuses it too (why App::truncate never covers users).
        // The activity log's own trigger refuses the same cascade first, so
        // it is switched off for this one transaction, which then rolls
        // back with the switch.
        for flagged in [false, true] {
            let txn = db.begin().await.expect("begin");
            if flagged {
                with_permanent_delete_flag(&txn)
                    .await
                    .expect("set the flag");
            }
            execute(
                &txn,
                "ALTER TABLE activity_log DISABLE TRIGGER activity_log_refuse_truncate",
            )
            .await
            .expect("switch the activity log's trigger off");
            let error = execute(&txn, "TRUNCATE users CASCADE")
                .await
                .expect_err("the cascade reaches revisions");
            assert!(error.to_string().contains(&refusal), "{error}");
            txn.rollback().await.expect("rollback");
        }
        assert_eq!(
            texts(
                db,
                "SELECT tgenabled::text FROM pg_trigger WHERE tgname = 'activity_log_refuse_truncate'"
            )
            .await,
            ["O"],
            "the rollback switched the activity log's trigger back on"
        );
        assert_eq!(revision_count(db).await, 1);
    })
    .await;
}

#[tokio::test]
async fn delete_is_refused_outside_a_permanent_delete_transaction() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let id = insert_revision(db, 1).await;
        let delete = format!("DELETE FROM revisions WHERE id = {id}");
        for sql in [
            delete.clone(),
            "DELETE FROM revisions".to_owned(),
            // Statement-level: refused although no row matches.
            "DELETE FROM revisions WHERE false".to_owned(),
            // A DELETE spelled as MERGE fires the same trigger.
            format!(
                "MERGE INTO revisions AS revision USING (SELECT {id} AS id) AS source \
                 ON revision.id = source.id WHEN MATCHED THEN DELETE"
            ),
            "MERGE INTO revisions AS revision USING (SELECT 1 AS id) AS source ON false \
             WHEN MATCHED THEN DELETE"
                .to_owned(),
            // A DELETE cannot open the check for itself: the trigger fires
            // before the DELETE's own WHERE clause, or a WITH query attached
            // to that DELETE, runs. (A DELETE nested in another statement
            // is a different case: see
            // `a_flag_set_earlier_in_a_statement_that_nests_the_delete_counts`.)
            format!(
                "DELETE FROM revisions WHERE id = {id} AND \
                 set_config('{PERMANENT_DELETE_FLAG}', 'on', true) = 'on'"
            ),
            format!(
                "WITH flag AS (SELECT set_config('{PERMANENT_DELETE_FLAG}', 'on', true) AS value) \
                 DELETE FROM revisions WHERE id = {id} AND EXISTS (SELECT FROM flag)"
            ),
        ] {
            assert_refused(db, &sql, DELETE_REFUSAL).await;
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
            let error = execute(&txn, &delete)
                .await
                .expect_err(&format!("flag {setting:?}"));
            assert!(
                error.to_string().contains(DELETE_REFUSAL),
                "{setting:?}: {error}"
            );
            txn.rollback().await.expect("rollback");
        }
        // The flag ends with its transaction, committed or rolled back: one
        // simple query keeps all of it on one connection, so the DELETE runs
        // in the same session right after the flagged transaction.
        for end in ["COMMIT", "ROLLBACK"] {
            let sql = format!("BEGIN; SET LOCAL {PERMANENT_DELETE_FLAG} = 'on'; {end}; {delete}");
            let error = db
                .execute_unprepared(&sql)
                .await
                .expect_err(&format!("after {end}"));
            assert!(
                error.to_string().contains(DELETE_REFUSAL),
                "after {end}: {error}"
            );
        }
        assert_eq!(revision_count(db).await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_flag_set_earlier_in_a_statement_that_nests_the_delete_counts() {
    // Pins the limit that the migration documents. A DELETE written as a
    // data-modifying WITH query runs only when the outer statement reads its
    // output, or after the rest of that statement, so its trigger fires
    // after a set_config(..., true) earlier in the statement. That call sets
    // the flag for the transaction, as SET LOCAL does, and the trigger reads
    // only the flag. Each case runs in a transaction that is rolled back, so
    // the revision stays; the statement's own deletion shows inside it.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let id = insert_revision(db, 1).await;
        for sql in [
            // The outer query never reads the DELETE: it runs at the end.
            format!(
                "WITH deleted AS (DELETE FROM revisions WHERE id = {id} RETURNING id) \
                 SELECT set_config('{PERMANENT_DELETE_FLAG}', 'on', true)"
            ),
            // The outer query reads the flag's WITH query before the DELETE.
            format!(
                "WITH flag AS (SELECT set_config('{PERMANENT_DELETE_FLAG}', 'on', true) AS value), \
                 deleted AS (DELETE FROM revisions WHERE id = {id} RETURNING id) \
                 SELECT (SELECT value FROM flag), (SELECT count(*) FROM deleted)"
            ),
        ] {
            let txn = db.begin().await.expect("begin");
            execute(&txn, &sql)
                .await
                .unwrap_or_else(|error| panic!("{sql}: {error}"));
            assert_eq!(
                value::<_, i64>(&txn, "SELECT count(*) FROM revisions").await,
                0,
                "{sql}"
            );
            txn.rollback().await.expect("rollback");
            assert_eq!(revision_count(db).await, 1, "{sql}");
        }
        // The same DELETE as the primary statement, with the set_config in
        // a WITH query attached to it, is refused (as in
        // `delete_is_refused_outside_a_permanent_delete_transaction`).
        assert_refused(
            db,
            &format!(
                "WITH flag AS (SELECT set_config('{PERMANENT_DELETE_FLAG}', 'on', true) AS value) \
                 DELETE FROM revisions WHERE id = {id} AND EXISTS (SELECT FROM flag)"
            ),
            DELETE_REFUSAL,
        )
        .await;
        assert_eq!(revision_count(db).await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_permanent_delete_transaction_may_delete_revisions() {
    // A purge deletes the record's revisions, and the anonymisation of a
    // user those of the account record (ADR 0011 §6).
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let first = insert_revision(db, 1).await;
        let second = insert_revision(db, 2).await;
        let other: i64 = value(
            db,
            "INSERT INTO revisions (entity_type, entity_id, revision_no, reason, schema_version, \
             snapshot) VALUES ('pages', 7, 1, 'create', 1, '{}') RETURNING id",
        )
        .await;
        // A statement that matches no row is allowed and changes nothing.
        assert_eq!(
            in_permanent_delete_transaction(db, "DELETE FROM revisions WHERE false")
                .await
                .expect("no row"),
            0
        );
        assert_eq!(
            in_permanent_delete_transaction(
                db,
                "DELETE FROM revisions WHERE entity_type = 'posts' AND entity_id = 7"
            )
            .await
            .expect("the post's revisions"),
            2
        );
        assert_eq!(
            texts(db, "SELECT id::text FROM revisions").await,
            [other.to_string()]
        );
        // The MERGE spelling too.
        assert_eq!(
            in_permanent_delete_transaction(
                db,
                &format!(
                    "MERGE INTO revisions AS revision USING (SELECT {other} AS id) AS source \
                     ON revision.id = source.id WHEN MATCHED THEN DELETE"
                )
            )
            .await
            .expect("merge delete"),
            1
        );
        assert_eq!(revision_count(db).await, 0);
        // A rolled-back permanent delete keeps them.
        let kept = insert_revision(db, 3).await;
        let txn = db.begin().await.expect("begin");
        with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        execute(&txn, &format!("DELETE FROM revisions WHERE id = {kept}"))
            .await
            .expect("delete");
        txn.rollback().await.expect("rollback");
        assert_eq!(revision_count(db).await, 1);
        assert!(first < second && second < other && other < kept);
    })
    .await;
}
