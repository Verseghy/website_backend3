//! The `roles_and_grants` migration (ADR 0007 §3, ADR 0009): the four
//! seeded roles and the permission CHECK equal the enums of
//! `src/domain/authz.rs`, and the join tables keep their links consistent
//! with `users` and `roles`.
//!
//! The first two tests are the guard that ADR 0007 §3 asks for: adding a
//! role or a permission is a migration and a code change in the same PR, and
//! these tests fail when only one of the two happened. The CHECK is compared
//! both by reading its definition from `pg_constraint` (the exact list) and
//! by inserting every name and some near misses (what it admits).
//!
//! Rows are written in SQL, so each test controls every column; the rollback
//! of every migration is covered by `search_support`'s test. Every account is
//! invented.

use backend3::domain::authz::{Permission, Role};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

use crate::support::with_app;

/// Runs `sql` and returns the number of rows it changed.
async fn execute(db: &DatabaseConnection, sql: &str) -> Result<u64, DbErr> {
    db.execute_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .map(|result| result.rows_affected())
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

/// The single integer in the first column of the first row of `sql`.
async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .expect("one row")
        .try_get_by_index(0)
        .expect("an integer")
}

/// Inserts an invented account with `email` and returns its id.
async fn account(db: &DatabaseConnection, email: &str) -> i64 {
    db.query_one_raw(Statement::from_string(
        DbBackend::Postgres,
        format!(
            "INSERT INTO users (pid, name, email) VALUES (gen_random_uuid(), 'Próba', '{email}') \
             RETURNING id"
        ),
    ))
    .await
    .expect("insert the account")
    .expect("the new row")
    .try_get_by_index(0)
    .expect("its id")
}

/// The string literals of a CHECK definition as `pg_get_constraintdef` writes
/// an `IN (...)` list: `CHECK ((permission = ANY (ARRAY['a'::text, ...])))`.
///
/// - `definition`: the deparsed constraint.
///
/// No permission name contains a quote, so each literal is the text between
/// a pair of single quotes.
fn quoted_literals(definition: &str) -> Vec<String> {
    definition
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[test]
fn quoted_literals_reads_the_deparsed_list() {
    assert_eq!(
        quoted_literals(
            "CHECK ((permission = ANY (ARRAY['edit posts'::text, 'view logs'::text])))"
        ),
        ["edit posts", "view logs"]
    );
    assert_eq!(quoted_literals("CHECK (true)"), Vec::<String>::new());
}

#[tokio::test]
async fn the_seeded_roles_are_the_role_enum() {
    with_app(|_router, ctx| async move {
        let rows = ctx
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT id, name FROM roles ORDER BY id",
            ))
            .await
            .expect("read the roles");
        let stored: Vec<(i64, String)> = rows
            .iter()
            .map(|row| {
                (
                    row.try_get_by_index(0).expect("id"),
                    row.try_get_by_index(1).expect("name"),
                )
            })
            .collect();
        let expected: Vec<(i64, String)> = Role::ALL
            .iter()
            .map(|role| (role.id(), role.name().to_owned()))
            .collect();
        assert_eq!(
            stored, expected,
            "the roles rows and Role::ALL of src/domain/authz.rs must list the same roles; add \
             a role with a migration and a code change in the same PR (ADR 0007 §3)"
        );
    })
    .await;
}

#[tokio::test]
async fn a_role_inserted_without_an_id_never_takes_a_seeded_one() {
    // The seeded rows carry explicit ids, which do not advance the identity
    // sequence; the migration sets it past them (ADR 0009, "Primary keys and
    // ids", rules 1 and 3). Nothing in backend3 inserts a role without an id,
    // so this only proves that such a row could not collide.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let highest = Role::ALL
            .iter()
            .map(|role| role.id())
            .max()
            .expect("at least one role");
        let next = count(
            db,
            "INSERT INTO roles (name) VALUES ('proba-szerepkor') RETURNING id",
        )
        .await;
        assert_eq!(next, highest + 1, "the next id follows the seeded ones");
    })
    .await;
}

#[tokio::test]
async fn the_permission_check_lists_exactly_the_permission_enum() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT conname::text, pg_get_constraintdef(oid) FROM pg_constraint \
                 WHERE conrelid = 'user_permissions'::regclass AND contype = 'c'",
            ))
            .await
            .expect("read the CHECKs");
        assert_eq!(rows.len(), 1, "one CHECK on user_permissions");
        let name: String = rows[0].try_get_by_index(0).expect("name");
        let definition: String = rows[0].try_get_by_index(1).expect("definition");
        assert_eq!(name, "user_permissions_permission_known");
        assert!(
            definition.starts_with("CHECK ((permission = ANY (ARRAY["),
            "{definition}"
        );
        assert_eq!(
            quoted_literals(&definition),
            Permission::ALL.map(|permission| permission.name().to_owned()),
            "the CHECK user_permissions_permission_known and Permission::ALL of \
             src/domain/authz.rs must list the same names; add a permission with a migration and \
             a code change in the same PR (ADR 0007 §3)"
        );

        // What it admits: every name, and nothing near one.
        let user = account(db, "jog@example.test").await;
        for permission in Permission::ALL {
            execute(
                db,
                &format!(
                    "INSERT INTO user_permissions (user_id, permission) VALUES ({user}, '{}')",
                    permission.name()
                ),
            )
            .await
            .unwrap_or_else(|error| panic!("{permission}: {error}"));
        }
        assert_eq!(
            count(db, "SELECT count(*) FROM user_permissions").await,
            16
        );
        for refused in [
            // Never created (ADR 0007, follow-up points 1 and 2).
            "edit newsletter",
            "manage backups",
            "Edit posts",
            "edit_posts",
            "edit posts ",
            "",
        ] {
            assert_refused(
                db,
                &format!(
                    "INSERT INTO user_permissions (user_id, permission) VALUES ({user}, '{refused}')"
                ),
                "user_permissions_permission_known",
            )
            .await;
        }
    })
    .await;
}

#[tokio::test]
async fn the_tables_have_exactly_the_planned_columns() {
    // No surrogate id on the join tables and no audit columns on any of the
    // three (ADR 0009, "Columns per table"); `roles.id` is an identity column
    // like every other `id` (ADR 0009, "Primary keys and ids", rule 1).
    with_app(|_router, ctx| async move {
        let rows = ctx
            .db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT table_name || '.' || column_name || ' ' || data_type || ' ' || \
                 is_nullable || ' ' || is_identity \
                 FROM information_schema.columns \
                 WHERE table_name IN ('roles', 'user_roles', 'user_permissions') ORDER BY 1",
            ))
            .await
            .expect("read the columns");
        let columns: Vec<String> = rows
            .iter()
            .map(|row| row.try_get_by_index(0).expect("text"))
            .collect();
        assert_eq!(
            columns,
            [
                // GENERATED BY DEFAULT AS IDENTITY, which still takes the
                // seeded rows' explicit ids.
                "roles.id bigint NO YES",
                "roles.name text NO NO",
                "user_permissions.permission text NO NO",
                "user_permissions.user_id bigint NO NO",
                "user_roles.role_id bigint NO NO",
                "user_roles.user_id bigint NO NO",
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn links_are_unique_and_point_at_existing_rows() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "kapcsolat@example.test").await;
        execute(
            db,
            &format!("INSERT INTO user_roles (user_id, role_id) VALUES ({user}, 1), ({user}, 4)"),
        )
        .await
        .expect("an account may hold several roles (ADR 0007 §9)");
        assert_refused(
            db,
            &format!("INSERT INTO user_roles (user_id, role_id) VALUES ({user}, 1)"),
            "user_roles_pkey",
        )
        .await;
        assert_refused(
            db,
            &format!("INSERT INTO user_roles (user_id, role_id) VALUES ({user}, 5)"),
            "user_roles_role_id_fkey",
        )
        .await;
        assert_refused(
            db,
            "INSERT INTO user_roles (user_id, role_id) VALUES (999999, 1)",
            "user_roles_user_id_fkey",
        )
        .await;
        execute(
            db,
            &format!(
                "INSERT INTO user_permissions (user_id, permission) VALUES ({user}, 'edit events')"
            ),
        )
        .await
        .expect("a grant");
        assert_refused(
            db,
            &format!(
                "INSERT INTO user_permissions (user_id, permission) VALUES ({user}, 'edit events')"
            ),
            "user_permissions_pkey",
        )
        .await;
        assert_refused(
            db,
            "INSERT INTO user_permissions (user_id, permission) VALUES (999999, 'edit events')",
            "user_permissions_user_id_fkey",
        )
        .await;
        // The role names are unique too.
        assert_refused(
            db,
            "INSERT INTO roles (id, name) VALUES (5, 'admin')",
            "roles_name_key",
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn a_held_role_cannot_go_and_an_account_takes_its_links_with_it() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "torolt@example.test").await;
        let other = account(db, "marad@example.test").await;
        // Two statements, so unprepared: a prepared statement holds one.
        db.execute_unprepared(&format!(
            "INSERT INTO user_roles (user_id, role_id) VALUES ({user}, 2), ({other}, 3); \
                 INSERT INTO user_permissions (user_id, permission) \
                 VALUES ({user}, 'view logs'), ({other}, 'edit menu')"
        ))
        .await
        .expect("links");

        // ON DELETE RESTRICT: a role that an account holds stays.
        assert_refused(
            db,
            "DELETE FROM roles WHERE id = 2",
            "user_roles_role_id_fkey",
        )
        .await;

        // ON DELETE CASCADE from users. backend3 anonymises accounts rather
        // than deleting them (ADR 0011 §6); the cascade keeps the database
        // consistent all the same.
        execute(db, &format!("DELETE FROM users WHERE id = {user}"))
            .await
            .expect("delete the account");
        assert_eq!(
            count(
                db,
                &format!("SELECT count(*) FROM user_roles WHERE user_id = {user}")
            )
            .await,
            0
        );
        assert_eq!(
            count(
                db,
                &format!("SELECT count(*) FROM user_permissions WHERE user_id = {user}")
            )
            .await,
            0
        );
        // The other account keeps its links, and the role is free to go.
        assert_eq!(count(db, "SELECT count(*) FROM user_roles").await, 1);
        assert_eq!(count(db, "SELECT count(*) FROM user_permissions").await, 1);
        execute(db, "DELETE FROM roles WHERE id = 2")
            .await
            .expect("a role nobody holds can be removed");
    })
    .await;
}
