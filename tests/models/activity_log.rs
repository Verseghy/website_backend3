//! The `activity_log` model (ADR 0011 §5): `record` writes every part of an
//! entry as given, refuses an entry that breaks a rule before writing
//! anything, takes its instant from the writing transaction, and commits or
//! rolls back with the change it records; `with_permanent_delete_flag` opens
//! exactly the one `UPDATE` the anonymisation needs, until the outermost
//! transaction ends; `App::truncate` leaves the log alone; the table's
//! CHECKs allow exactly the actions and actor kinds of `src/domain/activity.rs`
//! and refuse an address or a name that skips the model.
//!
//! The SQL rules of the triggers themselves are tested in
//! `tests/migration/activity_log.rs`. Addresses come from the documentation
//! ranges of RFC 5737 and RFC 3849; every account is invented.

use std::net::IpAddr;

use backend3::{
    app::App,
    domain::activity::{
        ACTOR_KINDS, Action, ActionGroup, Actor, EntryError, PEOPLE_ENTITY_TYPES,
        changed_fields_details,
    },
    models::activity_log::{
        self, ActiveModel, EntityRef, Entry, Model, record, with_permanent_delete_flag,
    },
};
use loco_rs::{app::Hooks, model::ModelError};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
    IntoActiveModel, Statement, TransactionTrait, entity::prelude::IpNetwork,
};
use serde_json::json;

use crate::support::with_app;

/// An address from RFC 5737's documentation range.
const ADDRESS: &str = "192.0.2.20";

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

/// The text in the first column of each row that `sql` returns.
async fn texts(db: &DatabaseConnection, sql: &str) -> Vec<String> {
    db.query_all_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect(sql)
        .iter()
        .map(|row| row.try_get_by_index(0).expect("text"))
        .collect()
}

/// Inserts an invented account on `db` and returns its id.
async fn insert_user<C: ConnectionTrait>(db: &C, email: &str) -> i64 {
    value(
        db,
        &format!(
            "INSERT INTO users (pid, name, email) \
             VALUES (gen_random_uuid(), 'Próba', '{email}') RETURNING id"
        ),
    )
    .await
}

async fn entry_count(db: &DatabaseConnection) -> i64 {
    value(db, "SELECT count(*) FROM activity_log").await
}

async fn stored(db: &DatabaseConnection, id: i64) -> Model {
    activity_log::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("find the entry")
        .expect("the entry exists")
}

/// The [`EntryError`] inside a refusal of [`record`].
fn entry_error(error: &ModelError) -> &EntryError {
    match error {
        ModelError::Any(inner) => inner
            .downcast_ref::<EntryError>()
            .unwrap_or_else(|| panic!("not an EntryError: {inner}")),
        other => panic!("not ModelError::Any: {other:?}"),
    }
}

/// The quoted values of the CHECK `constraint` on `activity_log`, in order.
async fn check_values(db: &DatabaseConnection, constraint: &str) -> Vec<String> {
    texts(
        db,
        &format!(
            "SELECT matched[1] FROM pg_constraint, \
             regexp_matches(pg_get_constraintdef(pg_constraint.oid), '''([a-z_]+)''::text', 'g') \
             WITH ORDINALITY AS found(matched, n) \
             WHERE conrelid = 'activity_log'::regclass AND conname = '{constraint}' \
             ORDER BY n"
        ),
    )
    .await
}

#[tokio::test]
async fn record_stores_every_part_of_an_entry() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = insert_user(db, "proba@example.test").await;

        let update = record(
            db,
            Actor::User(user),
            Entry {
                entity: Some(EntityRef::record("labels", 7)),
                revision_id: Some(3),
                label: Some("Hírek".to_owned()),
                details: Some(changed_fields_details(&[
                    "color".to_owned(),
                    "name".to_owned(),
                ])),
                request_id: Some("4bf92f35-request".to_owned()),
                ..Entry::new(Action::Update)
            },
        )
        .await
        .expect("record an update");
        assert_eq!(stored(db, update.id).await, update);
        assert_eq!(update.actor_kind, "user");
        assert_eq!(update.actor_id, Some(user));
        assert_eq!(update.actor(), Ok(Actor::User(user)));
        assert_eq!(update.action, "update");
        assert_eq!(update.action(), Ok(Action::Update));
        assert_eq!(update.entity_type.as_deref(), Some("labels"));
        assert_eq!(update.entity_id, Some(7));
        assert_eq!(update.revision_id, Some(3));
        assert_eq!(update.label.as_deref(), Some("Hírek"));
        assert_eq!(
            update.details,
            Some(json!({ "changed_fields": ["color", "name"] }))
        );
        assert_eq!(update.request_id.as_deref(), Some("4bf92f35-request"));
        assert_eq!(update.client_address(), None);

        // A sign-in entry: about a person, so no label, and with the address.
        for (address, host_bits) in [(ADDRESS, 32), ("2001:db8::20", 128)] {
            let address: IpAddr = address.parse().expect("address");
            let login = record(
                db,
                Actor::User(user),
                Entry {
                    entity: Some(EntityRef::record("users", user)),
                    ip: Some(address),
                    ..Entry::new(Action::Login)
                },
            )
            .await
            .expect("record a sign-in");
            let login = stored(db, login.id).await;
            assert_eq!(login.client_address(), Some(address));
            assert_eq!(login.label, None);
            assert_eq!(login.details, None);
            // Stored as a single host: the whole prefix length of its family.
            assert_eq!(
                value::<_, String>(
                    db,
                    &format!(
                        "SELECT host(ip) || '/' || masklen(ip) FROM activity_log WHERE id = {}",
                        login.id
                    )
                )
                .await,
                format!("{address}/{host_bits}")
            );
        }
    })
    .await;
}

#[tokio::test]
async fn each_actor_is_stored_with_its_kind() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = insert_user(db, "proba@example.test").await;
        for actor in [Actor::User(user), Actor::System, Actor::Import] {
            let entry = record(
                db,
                actor,
                Entry {
                    entity: Some(EntityRef::table("labels")),
                    ..Entry::new(Action::Import)
                },
            )
            .await
            .expect("record");
            let entry = stored(db, entry.id).await;
            assert_eq!(entry.actor(), Ok(actor));
            assert_eq!(entry.actor_kind, actor.kind());
            assert_eq!(entry.actor_id, actor.user_id());
            assert_eq!(entry.entity_id, None);
        }
    })
    .await;
}

#[tokio::test]
async fn the_checks_allow_exactly_the_actions_and_actor_kinds_of_the_domain() {
    // A new action is a variant of `Action` plus a migration that replaces
    // the CHECK; this fails until both are there (ADR 0009, "Column types").
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let names: Vec<&str> = Action::ALL.iter().map(|action| action.as_str()).collect();
        assert_eq!(
            check_values(db, "activity_log_action_known").await,
            names,
            "the activity_log_action_known CHECK and Action::ALL list different actions"
        );
        assert_eq!(
            check_values(db, "activity_log_actor_kind_known").await,
            ACTOR_KINDS,
            "the activity_log_actor_kind_known CHECK and ACTOR_KINDS differ"
        );
        // The two personal-data CHECKs list the account actions (and the
        // people entity types) of the domain, in the same order.
        let account: Vec<&str> = Action::ALL
            .iter()
            .filter(|action| action.group() == ActionGroup::Account)
            .map(|action| action.as_str())
            .collect();
        assert_eq!(
            check_values(db, "activity_log_ip_account_only").await,
            account,
            "the activity_log_ip_account_only CHECK and the account actions of Action differ"
        );
        assert_eq!(
            check_values(db, "activity_log_label_content_only").await,
            [PEOPLE_ENTITY_TYPES.as_slice(), &account].concat(),
            "the activity_log_label_content_only CHECK and PEOPLE_ENTITY_TYPES followed by the \
             account actions of Action differ"
        );
        // And the table takes every one of them through the model.
        for &action in Action::ALL {
            let entry = record(db, Actor::System, Entry::new(action))
                .await
                .unwrap_or_else(|error| panic!("{action}: {error}"));
            assert_eq!(stored(db, entry.id).await.action(), Ok(action));
        }
    })
    .await;
}

#[tokio::test]
async fn an_entry_that_breaks_a_rule_is_refused_before_anything_is_written() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let address: IpAddr = ADDRESS.parse().expect("address");
        for (entry, expected) in [
            (
                Entry {
                    entity: Some(EntityRef::record("posts", 7)),
                    ip: Some(address),
                    ..Entry::new(Action::Update)
                },
                EntryError::ClientAddressNotAllowed {
                    action: Action::Update,
                },
            ),
            (
                Entry {
                    entity: Some(EntityRef::record("authors", 4)),
                    label: Some("Próba Szerző".to_owned()),
                    ..Entry::new(Action::Delete)
                },
                EntryError::LabelOnPerson {
                    entity_type: "authors".to_owned(),
                },
            ),
            (
                Entry {
                    entity: Some(EntityRef::table("posts")),
                    revision_id: Some(1),
                    ..Entry::new(Action::Update)
                },
                EntryError::RevisionWithoutRecord {
                    action: Action::Update,
                },
            ),
            (
                Entry {
                    entity: Some(EntityRef::record("menu-items", 1)),
                    ..Entry::new(Action::Reorder)
                },
                EntryError::EntityTypeNotSnakeCase {
                    entity_type: "menu-items".to_owned(),
                },
            ),
            (
                Entry {
                    entity: Some(EntityRef::record("posts", 7)),
                    label: Some("  ".to_owned()),
                    ..Entry::new(Action::Publish)
                },
                EntryError::BlankLabel {
                    action: Action::Publish,
                },
            ),
            // A name can reach the label without naming a person's record:
            // on an account entry, or on an entry about no record at all.
            (
                Entry {
                    label: Some("Kitalált Személy".to_owned()),
                    ..Entry::new(Action::Login)
                },
                EntryError::LabelOnAccountEvent {
                    action: Action::Login,
                },
            ),
            (
                Entry {
                    label: Some("Kitalált Személy".to_owned()),
                    ..Entry::new(Action::Publish)
                },
                EntryError::LabelWithoutEntity {
                    action: Action::Publish,
                },
            ),
            (
                Entry {
                    request_id: Some(String::new()),
                    ..Entry::new(Action::Logout)
                },
                EntryError::RequestIdLength {
                    action: Action::Logout,
                    chars: 0,
                },
            ),
        ] {
            let error = record(db, Actor::System, entry).await.expect_err("refused");
            assert_eq!(entry_error(&error), &expected);
        }
        assert_eq!(entry_count(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn the_table_refuses_an_address_or_a_name_that_skips_the_model() {
    // Defence in depth (ADR 0011 §5): an INSERT that does not go through
    // `record` still cannot store an address or a name, which could never be
    // removed again (only the anonymisation's `ip` UPDATE is allowed).
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let row = |action: Action, entity_type: Option<&str>, label: Option<&str>, ip: bool| {
            ActiveModel {
                actor_kind: ActiveValue::Set("system".to_owned()),
                action: ActiveValue::Set(action.as_str().to_owned()),
                entity_type: ActiveValue::Set(entity_type.map(str::to_owned)),
                label: ActiveValue::Set(label.map(str::to_owned)),
                ip: ActiveValue::Set(
                    ip.then(|| IpNetwork::from(ADDRESS.parse::<IpAddr>().expect("address"))),
                ),
                ..Default::default()
            }
        };
        let name = Some("Kitalált Személy");
        for (model, constraint) in [
            (
                row(Action::Update, Some("posts"), None, true),
                "activity_log_ip_account_only",
            ),
            (
                row(Action::MediaUpload, None, None, true),
                "activity_log_ip_account_only",
            ),
            (
                row(Action::Login, Some("users"), name, false),
                "activity_log_label_content_only",
            ),
            (
                row(Action::RolesChanged, Some("posts"), name, false),
                "activity_log_label_content_only",
            ),
            (
                row(Action::Publish, None, name, false),
                "activity_log_label_content_only",
            ),
            (
                row(Action::Update, Some("authors"), name, false),
                "activity_log_label_content_only",
            ),
        ] {
            let error = model.insert(db).await.expect_err(constraint);
            assert!(
                error.to_string().contains(constraint),
                "{constraint}: {error}"
            );
        }
        assert_eq!(entry_count(db).await, 0);
        // What `record` itself writes passes them.
        row(Action::Login, Some("users"), None, true)
            .insert(db)
            .await
            .expect("an address on a sign-in");
        row(Action::Update, Some("posts"), Some("Szalagavató"), false)
            .insert(db)
            .await
            .expect("a label on a content record");
    })
    .await;
}

#[tokio::test]
async fn occurred_at_is_the_transaction_start_on_the_database_clock() {
    // research: DECISIONS R15, D90. The test reads PostgreSQL's clock, never
    // its own (recipe 7 of the project skill).
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let txn = db.begin().await.expect("begin");
        let start: String = value(&txn, "SELECT now()::text").await;
        let first = record(&txn, Actor::System, Entry::new(Action::Reorder))
            .await
            .expect("first");
        let second = record(&txn, Actor::System, Entry::new(Action::Publish))
            .await
            .expect("second");
        txn.commit().await.expect("commit");
        for entry in [first, second] {
            let text: String = value(
                db,
                &format!(
                    "SELECT occurred_at::text FROM activity_log WHERE id = {}",
                    entry.id
                ),
            )
            .await;
            assert_eq!(text, start);
        }

        // On the pool, each entry is its own transaction: its instant lies
        // between two readings of the database clock around the call.
        let before: String = value(db, "SELECT clock_timestamp()::text").await;
        let entry = record(db, Actor::System, Entry::new(Action::Reorder))
            .await
            .expect("on the pool");
        let after: String = value(db, "SELECT clock_timestamp()::text").await;
        let within: bool = value(
            db,
            &format!(
                "SELECT occurred_at BETWEEN '{before}'::timestamptz AND '{after}'::timestamptz \
                 FROM activity_log WHERE id = {}",
                entry.id
            ),
        )
        .await;
        assert!(within, "{before} <= occurred_at <= {after}");
    })
    .await;
}

#[tokio::test]
async fn an_entry_commits_and_rolls_back_with_the_change_it_records() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;

        // Rolled back: neither the change nor its entry remains.
        let txn = db.begin().await.expect("begin");
        let user = insert_user(&txn, "visszavont@example.test").await;
        record(
            &txn,
            Actor::System,
            Entry {
                entity: Some(EntityRef::record("users", user)),
                ..Entry::new(Action::UserActivated)
            },
        )
        .await
        .expect("record");
        txn.rollback().await.expect("rollback");
        assert_eq!(entry_count(db).await, 0);
        assert_eq!(value::<_, i64>(db, "SELECT count(*) FROM users").await, 0);

        // A failed entry fails the transaction, so the change goes too
        // (ADR 0011, "A failure while inserting the activity entry rolls
        // back the record change"): here the actor names no account.
        let txn = db.begin().await.expect("begin");
        insert_user(&txn, "elveszett@example.test").await;
        let error = record(&txn, Actor::User(999_999), Entry::new(Action::Create))
            .await
            .expect_err("no such account");
        assert!(
            error.to_string().contains("activity_log_actor_id_fkey"),
            "{error}"
        );
        txn.rollback().await.expect("rollback");
        assert_eq!(value::<_, i64>(db, "SELECT count(*) FROM users").await, 0);

        // Committed: both remain.
        let txn = db.begin().await.expect("begin");
        let user = insert_user(&txn, "megmarad@example.test").await;
        let entry = record(&txn, Actor::User(user), Entry::new(Action::Create))
            .await
            .expect("record");
        txn.commit().await.expect("commit");
        assert_eq!(stored(db, entry.id).await, entry);
    })
    .await;
}

#[tokio::test]
async fn the_flag_lets_the_model_clear_only_an_address() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = insert_user(db, "proba@example.test").await;
        let login = record(
            db,
            Actor::User(user),
            Entry {
                entity: Some(EntityRef::record("users", user)),
                request_id: Some("req-2".to_owned()),
                ip: Some(ADDRESS.parse().expect("address")),
                ..Entry::new(Action::LoginFailed)
            },
        )
        .await
        .expect("record");

        // Without the flag the model's own UPDATE is refused.
        let mut clear: ActiveModel = login.clone().into_active_model();
        clear.ip = ActiveValue::Set(None);
        let error = clear.clone().update(db).await.expect_err("no flag");
        assert!(
            error
                .to_string()
                .contains("UPDATE is allowed only in a permanent-delete transaction"),
            "{error}"
        );

        // With it, an UPDATE that clears only the address goes through, as
        // the anonymisation of a user issues it (ADR 0011 §6).
        let txn = db.begin().await.expect("begin");
        with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        let cleared = clear.update(&txn).await.expect("clear the address");
        txn.commit().await.expect("commit");
        assert_eq!(cleared.client_address(), None);
        assert_eq!(
            cleared,
            Model {
                ip: None,
                ..login.clone()
            }
        );
        assert_eq!(stored(db, login.id).await, cleared);

        // Any other change is refused with the flag too, and so is a delete.
        let txn = db.begin().await.expect("begin");
        with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        let mut relabel: ActiveModel = cleared.clone().into_active_model();
        relabel.request_id = ActiveValue::Set(None);
        let error = relabel.update(&txn).await.expect_err("not only ip");
        assert!(
            error
                .to_string()
                .contains("a permanent-delete transaction may only set ip to NULL"),
            "{error}"
        );
        txn.rollback().await.expect("rollback");
        let txn = db.begin().await.expect("begin");
        with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        let error = activity_log::Entity::delete_by_id(login.id)
            .exec(&txn)
            .await
            .expect_err("never a delete");
        assert!(
            error.to_string().contains("DELETE is never allowed"),
            "{error}"
        );
        txn.rollback().await.expect("rollback");
        assert_eq!(stored(db, login.id).await, cleared);
    })
    .await;
}

#[tokio::test]
async fn the_flag_is_set_only_in_its_transaction() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let flag = "SELECT coalesce(current_setting('backend3.permanent_delete', true), '')";
        let txn = db.begin().await.expect("begin");
        assert_eq!(value::<_, String>(&txn, flag).await, "");
        with_permanent_delete_flag(&txn)
            .await
            .expect("set the flag");
        assert_eq!(value::<_, String>(&txn, flag).await, "on");
        // A savepoint (Sea-ORM's nested transaction) sees it as well.
        let nested = txn.begin().await.expect("savepoint");
        assert_eq!(value::<_, String>(&nested, flag).await, "on");
        nested.commit().await.expect("release");
        txn.commit().await.expect("commit");
        // `SET LOCAL` ends with the transaction; every pooled connection is
        // back to the empty value or has never seen the setting.
        for _ in 0..3 {
            assert_eq!(value::<_, String>(db, flag).await, "");
        }
    })
    .await;
}

#[tokio::test]
async fn a_flag_set_in_a_savepoint_lasts_until_the_outer_transaction_ends() {
    // PostgreSQL ties `SET LOCAL` to the transaction, not to a savepoint, so
    // `with_permanent_delete_flag` is set on the outermost transaction of a
    // permanent delete (its docs; recipe 8 step 3 of the project skill).
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let flag = "SELECT coalesce(current_setting('backend3.permanent_delete', true), '')";
        let login = record(
            db,
            Actor::System,
            Entry {
                entity: Some(EntityRef::record("users", 1)),
                ip: Some(ADDRESS.parse().expect("address")),
                ..Entry::new(Action::LoginFailed)
            },
        )
        .await
        .expect("record");
        let clear_address = || {
            let mut clear: ActiveModel = login.clone().into_active_model();
            clear.ip = ActiveValue::Set(None);
            clear
        };

        // Committed (RELEASE SAVEPOINT): the flag outlives the nested
        // transaction, and the rest of the outer one may still clear an
        // address.
        let txn = db.begin().await.expect("begin");
        let nested = txn.begin().await.expect("savepoint");
        with_permanent_delete_flag(&nested)
            .await
            .expect("set the flag");
        nested.commit().await.expect("release");
        assert_eq!(value::<_, String>(&txn, flag).await, "on");
        clear_address()
            .update(&txn)
            .await
            .expect("still flagged after the savepoint");
        txn.rollback().await.expect("rollback");
        assert_eq!(stored(db, login.id).await, login);

        // Rolled back (ROLLBACK TO SAVEPOINT): the flag goes with it.
        let txn = db.begin().await.expect("begin");
        let nested = txn.begin().await.expect("savepoint");
        with_permanent_delete_flag(&nested)
            .await
            .expect("set the flag");
        nested.rollback().await.expect("roll back the savepoint");
        assert_eq!(value::<_, String>(&txn, flag).await, "");
        let error = clear_address()
            .update(&txn)
            .await
            .expect_err("no flag after the savepoint rolled back");
        assert!(
            error
                .to_string()
                .contains("UPDATE is allowed only in a permanent-delete transaction"),
            "{error}"
        );
        txn.rollback().await.expect("rollback");
        assert_eq!(stored(db, login.id).await, login);
    })
    .await;
}

#[tokio::test]
async fn app_truncate_leaves_the_activity_log_alone() {
    with_app(|_router, ctx| async move {
        let entry = record(&ctx.db, Actor::System, Entry::new(Action::Purge))
            .await
            .expect("record");
        App::truncate(&ctx).await.expect("truncate");
        assert_eq!(stored(&ctx.db, entry.id).await, entry);
    })
    .await;
}
