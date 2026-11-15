//! The label writes of the `labels` model (parity LBL-02, SHR-10, SHR-13;
//! ADR 0011 §2-§7): `Model::create` and `ActiveModel::edit` pass every
//! check of the safety-net helper (`tests/support/safety_net.rs`) for a user
//! and for the system; a name that a live label already has, whatever its
//! case, is refused as `EntityAlreadyExists` by the live-name index and
//! writes nothing, while a trashed label's name is free and a label may
//! change the case of its own name; a save that meets the same name in
//! another save's uncommitted row waits for it, and is that refusal once
//! the other commits and saved once it rolls back (the index decides, not a
//! lookup before the write); an edit of a missing or trashed label
//! is `EntityNotFound`; a save that changes nothing still writes its
//! revision; and only the named unique index counts as a conflict
//! (`models::unique_index`). Labels are written through the model, so the
//! identity hands out the ids; every name is invented.

use std::sync::atomic::{AtomicU32, Ordering};

use backend3::{
    domain::{
        activity::{Action, Actor, CHANGED_FIELDS_KEY},
        authz::Role,
        labels::{LabelInput, LabelRevision},
        revision::Reason,
    },
    models::{
        activity_log,
        labels::{ActiveModel, Column, EditLabelError, Entity, LIVE_NAME_INDEX, Model},
        revisions, unique_index,
    },
};
use loco_rs::model::{ModelError, ModelResult};
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Statement, TransactionTrait,
};
use serde_json::{Value, json};

use crate::support::{
    admin_probe::account_with,
    concurrent::{until_one_waits, with_second_pool},
    safety_net::{SafetyNetEntity, assert_safety_net},
    with_app,
};

/// The checked input of `name` and `color`; panics on a refusal.
fn input(name: &str, color: &str) -> LabelInput {
    LabelInput::check(name, color).unwrap_or_else(|error| panic!("{name} {color}: {error}"))
}

/// The number of rows of `table`.
async fn count(db: &DatabaseConnection, table: &str) -> i64 {
    db.query_one_raw(Statement::from_string(
        DbBackend::Postgres,
        format!("SELECT count(*) FROM {table}"),
    ))
    .await
    .expect("count")
    .expect("one row")
    .try_get_by_index(0)
    .expect("a count")
}

/// The number of labels, revisions and activity entries, to show that a
/// refused save wrote none.
async fn totals(db: &DatabaseConnection) -> (i64, i64, i64) {
    (
        count(db, "labels").await,
        count(db, "revisions").await,
        count(db, "activity_log").await,
    )
}

/// Creates and commits a label as `actor`, and returns the stored row.
async fn create(db: &DatabaseConnection, actor: Actor, name: &str, color: &str) -> Model {
    let txn = db.begin().await.expect("begin");
    let label = Model::create(&txn, actor, &input(name, color), None)
        .await
        .unwrap_or_else(|error| panic!("create {name}: {error}"));
    txn.commit().await.expect("commit");
    label
}

/// Runs `save` in a transaction of its own, rolls it back, and returns its
/// outcome: for the refusals, which leave the transaction aborted.
async fn refused_save<F>(db: &DatabaseConnection, save: F) -> ModelResult<Model>
where
    F: AsyncFnOnce(&DatabaseTransaction) -> ModelResult<Model>,
{
    let txn = db.begin().await.expect("begin");
    let outcome = save(&txn).await;
    txn.rollback().await.expect("roll back");
    outcome
}

/// Moves label `id` to the trash, as the trash slice will.
async fn trash(db: &DatabaseConnection, id: i64) {
    db.execute_unprepared(&format!(
        "UPDATE labels SET deleted_at = CURRENT_TIMESTAMP WHERE id = {id}"
    ))
    .await
    .expect("trash");
}

/// The labels under the safety-net helper: each create a new name (the
/// helper creates more than one label, and live names are unique), each
/// update a longer name and the other colour.
#[derive(Default)]
struct LabelSaves {
    /// How many labels this value has created, for the next name.
    created: AtomicU32,
}

impl SafetyNetEntity for LabelSaves {
    type Snapshot = LabelRevision;

    async fn create(&self, txn: &DatabaseTransaction, actor: Actor) -> ModelResult<i64> {
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        let name = format!("Próba címke {number}");
        Model::create(
            txn,
            actor,
            &input(&name, "#336699"),
            Some("labels-safety-net".to_owned()),
        )
        .await
        .map(|label| label.id)
    }

    async fn update(&self, txn: &DatabaseTransaction, actor: Actor, id: i64) -> ModelResult<()> {
        let current = Model::find_live_by_id(txn, id).await?;
        let color = if current.color == "#336699" {
            "#993366"
        } else {
            "#336699"
        };
        let next = input(&format!("{} +", current.name), color);
        current
            .into_active_model()
            .edit(txn, actor, &next, None)
            .await
            .map(|_| ())
    }

    fn changed_fields(&self) -> Vec<String> {
        vec!["color".to_owned(), "name".to_owned()]
    }

    async fn snapshot(&self, db: &DatabaseConnection, id: i64) -> LabelRevision {
        Entity::find_by_id(id)
            .one(db)
            .await
            .expect("read the label")
            .expect("the label exists")
            .revision_snapshot()
    }

    fn label(&self, snapshot: &LabelRevision) -> Option<String> {
        // A label is content, so its entries carry its name (ADR 0011 §5).
        Some(snapshot.name.clone())
    }
}

#[tokio::test]
async fn the_label_writes_meet_every_safety_net_check_for_a_user_and_for_the_system() {
    with_app(|_router, ctx| async move {
        let editor = account_with(&ctx, "cimkek@example.test", &[Role::Writer], &[], |_| {}).await;
        let saves = LabelSaves::default();
        for actor in [Actor::User(editor.id), Actor::System] {
            let id = assert_safety_net(&ctx.db, &saves, actor).await;
            // Create and two updates: revisions 1 to 3, each of version 1.
            let stored = revisions::Model::for_record(&ctx.db, "labels", id)
                .await
                .expect("revisions");
            assert_eq!(
                stored
                    .iter()
                    .map(|revision| (revision.revision_no, revision.schema_version))
                    .collect::<Vec<_>>(),
                [(1, 1), (2, 1), (3, 1)],
                "{actor:?}"
            );
            assert_eq!(stored[0].reason(), Ok(Reason::Create), "revision 1 of {id}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_create_stores_the_checked_input_and_its_snapshot() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let editor = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let label = create(db, Actor::User(editor.id), "  Ünnepség ", " #a1B2c3").await;
        assert_eq!(
            (label.name.as_str(), label.color.as_str()),
            ("Ünnepség", "#a1B2c3")
        );
        assert_eq!(
            (
                label.created_by,
                label.updated_by,
                label.deleted_at,
                label.deleted_by
            ),
            (Some(editor.id), Some(editor.id), None, None)
        );
        let revision = revisions::Model::for_record(db, "labels", label.id)
            .await
            .expect("revisions")
            .pop()
            .expect("revision 1");
        assert_eq!(
            revision.snapshot,
            json!({"name": "Ünnepség", "color": "#a1B2c3"})
        );
        let entry = activity_log::Entity::find()
            .filter(activity_log::Column::EntityType.eq("labels"))
            .filter(activity_log::Column::EntityId.eq(label.id))
            .one(db)
            .await
            .expect("read the entry")
            .expect("the create entry");
        assert_eq!(entry.action(), Ok(Action::Create));
        assert_eq!(entry.label.as_deref(), Some("Ünnepség"));
        assert_eq!(entry.revision_id, Some(revision.id));
        // A content entry holds no client address (ADR 0011 §5).
        assert_eq!(entry.client_address(), None);
    })
    .await;
}

#[tokio::test]
async fn a_live_name_is_taken_whatever_its_case_and_nothing_is_written() {
    // ADR 0009 "Constraints", ADR 0011 §3: unique on lower(name) among live
    // labels, decided by the index.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let editor = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let actor = Actor::User(editor.id);
        create(db, actor, "Ünnepség", "#123456").await;
        let before = totals(db).await;
        for taken in ["Ünnepség", "ünnepség", "ÜNNEPSÉG", "  ÜnnepSÉG  "] {
            let outcome = refused_save(db, async |txn| {
                Model::create(txn, actor, &input(taken, "#654321"), None).await
            })
            .await;
            assert!(
                matches!(outcome, Err(ModelError::EntityAlreadyExists)),
                "{taken}: {outcome:?}"
            );
        }
        assert_eq!(totals(db).await, before, "a refused create writes nothing");
        // Another name is free, and so is the name of a trashed label.
        let sport = create(db, actor, "Sport", "#123456").await;
        trash(db, sport.id).await;
        let again = create(db, actor, "SPORT", "#123456").await;
        assert_ne!(again.id, sport.id);
    })
    .await;
}

/// How the first of two concurrent saves of one name ends, in
/// [`a_save_that_meets_an_uncommitted_name_waits_for_its_outcome`].
#[derive(Debug, Clone, Copy)]
enum FirstSave {
    /// The first save commits: the name is taken.
    Commits,
    /// The first save rolls back: the name is free again.
    RollsBack,
}

#[tokio::test]
async fn a_save_that_meets_an_uncommitted_name_waits_for_its_outcome() {
    // The live-name index decides, not a lookup before the write: a second
    // save of a name that a first, uncommitted save wrote waits for the
    // first. Once the first commits, the second is the conflict, never a
    // database error, which a lookup that could not see the uncommitted
    // row would have let through; once it rolls back, the name is free.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let actor = Actor::System;
        let sport = create(db, actor, "Sport", "#123456").await.id;
        let before = totals(db).await;
        // The first save is a create; the second, a create or an edit of
        // `sport`, sends the same name in capitals.
        let cases = [
            (FirstSave::Commits, false, "Közös"),
            (FirstSave::Commits, true, "Közös 2"),
            (FirstSave::RollsBack, false, "Közös 3"),
            (FirstSave::RollsBack, true, "Közös 4"),
        ];
        // Two sessions for the two saves; the poll runs on the app's pool.
        with_second_pool(ctx.config.database.uri.as_str(), 2, |pool| async move {
            for (first_save, second_edits, name) in cases {
                let case = format!("{first_save:?}, the second edits: {second_edits}");
                let first = pool.begin().await.expect("begin the first save");
                Model::create(&first, actor, &input(name, "#000000"), None)
                    .await
                    .unwrap_or_else(|error| panic!("{case}: the first save: {error}"));
                let capitals = name.to_uppercase();
                let second = async {
                    let txn = pool.begin().await.expect("begin the second save");
                    let next = input(&capitals, "#654321");
                    let outcome = if second_edits {
                        Model::find_live_by_id(&txn, sport)
                            .await
                            .expect("live")
                            .into_active_model()
                            .edit(&txn, actor, &next, None)
                            .await
                    } else {
                        Model::create(&txn, actor, &next, None).await
                    };
                    if outcome.is_ok() {
                        txn.commit().await.expect("commit the second save");
                    } else {
                        txn.rollback().await.expect("roll the second save back");
                    }
                    outcome
                };
                // The first save ends only once the database shows the
                // second waiting for it, so the second really met it.
                let end_first = async {
                    until_one_waits(db).await;
                    let ended = match first_save {
                        FirstSave::Commits => first.commit().await,
                        FirstSave::RollsBack => first.rollback().await,
                    };
                    ended.expect("end the first save");
                };
                let (outcome, ()) = tokio::join!(second, end_first);
                match first_save {
                    FirstSave::Commits => assert!(
                        matches!(outcome, Err(ModelError::EntityAlreadyExists)),
                        "{case}: {outcome:?}"
                    ),
                    FirstSave::RollsBack => {
                        let saved = outcome.unwrap_or_else(|error| panic!("{case}: {error}"));
                        assert_eq!(saved.name, capitals, "{case}");
                    }
                }
            }
        })
        .await;
        // The two committed first saves and the created second one are a
        // label each, and the one accepted edit renamed `sport`; each of the
        // four saves wrote its revision and entry, the refused ones nothing.
        let names: Vec<String> = Entity::find()
            .order_by_asc(Column::Id)
            .all(db)
            .await
            .expect("read the labels")
            .into_iter()
            .map(|label| label.name)
            .collect();
        assert_eq!(names, ["KÖZÖS 4", "Közös", "Közös 2", "KÖZÖS 3"]);
        assert_eq!(totals(db).await, (before.0 + 3, before.1 + 4, before.2 + 4));
        let revisions = revisions::Model::for_record(db, "labels", sport)
            .await
            .expect("revisions");
        assert_eq!(revisions.len(), 2, "the create and the one accepted edit");
    })
    .await;
}

#[tokio::test]
async fn an_edit_may_change_the_case_of_its_own_name_but_not_take_another_s() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let editor = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let actor = Actor::User(editor.id);
        let news = create(db, actor, "Hírek", "#123456").await;
        let sport = create(db, actor, "Sport", "#123456").await;

        let txn = db.begin().await.expect("begin");
        let renamed = news
            .clone()
            .into_active_model()
            .edit(&txn, actor, &input("HÍREK", "#123456"), None)
            .await
            .expect("its own name in another case");
        txn.commit().await.expect("commit");
        assert_eq!(renamed.name, "HÍREK");

        let before = totals(db).await;
        let outcome = refused_save(db, async |txn| {
            sport
                .clone()
                .into_active_model()
                .edit(txn, actor, &input("hírek", "#123456"), None)
                .await
        })
        .await;
        assert!(
            matches!(outcome, Err(ModelError::EntityAlreadyExists)),
            "{outcome:?}"
        );
        assert_eq!(totals(db).await, before, "a refused edit writes nothing");
        assert_eq!(
            Model::find_live_by_id(db, sport.id)
                .await
                .expect("live")
                .name,
            "Sport"
        );
    })
    .await;
}

#[tokio::test]
async fn an_edit_of_a_missing_or_trashed_label_is_not_found() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let editor = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let actor = Actor::User(editor.id);
        let label = create(db, actor, "Lomtár", "#123456").await;
        trash(db, label.id).await;
        let before = totals(db).await;
        for id in [label.id, label.id + 1000] {
            let outcome = refused_save(db, async |txn| {
                ActiveModel {
                    id: ActiveValue::Unchanged(id),
                    ..Default::default()
                }
                .edit(txn, actor, &input("Új név", "#654321"), None)
                .await
            })
            .await;
            assert!(
                matches!(outcome, Err(ModelError::EntityNotFound)),
                "{id}: {outcome:?}"
            );
        }
        assert_eq!(totals(db).await, before);
        // The trashed row kept its name.
        let stored = Entity::find_by_id(label.id)
            .one(db)
            .await
            .expect("read")
            .expect("the trashed row");
        assert_eq!(stored.name, "Lomtár");

        // Without an id there is no row to edit: refused before any query.
        let outcome = refused_save(db, async |txn| {
            ActiveModel::default()
                .edit(txn, actor, &input("Új név", "#654321"), None)
                .await
        })
        .await;
        let Err(error) = outcome else {
            panic!("an edit without an id was accepted");
        };
        assert_eq!(error.to_string(), EditLabelError::NoId.to_string());
    })
    .await;
}

#[tokio::test]
async fn an_edit_that_changes_nothing_still_writes_a_revision_listing_no_field() {
    // ADR 0011 §4: every update writes one revision.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let editor = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let actor = Actor::User(editor.id);
        let label = create(db, actor, "Hírek", "#123456").await;
        let txn = db.begin().await.expect("begin");
        label
            .clone()
            .into_active_model()
            .edit(&txn, actor, &input(" Hírek ", "#123456"), None)
            .await
            .expect("the same values");
        txn.commit().await.expect("commit");
        let stored = revisions::Model::for_record(db, "labels", label.id)
            .await
            .expect("revisions");
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[1].reason(), Ok(Reason::Update));
        let entry = activity_log::Entity::find()
            .filter(activity_log::Column::RevisionId.eq(stored[1].id))
            .one(db)
            .await
            .expect("read the entry")
            .expect("the update entry");
        assert_eq!(entry.action(), Ok(Action::Update));
        assert_eq!(
            entry
                .details
                .as_ref()
                .and_then(|details| details.get(CHANGED_FIELDS_KEY)),
            Some(&json!([]))
        );
    })
    .await;
}

#[tokio::test]
async fn an_edit_lists_exactly_the_fields_it_changed() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let editor = account_with(&ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await;
        let actor = Actor::User(editor.id);
        let label = create(db, actor, "Hírek", "#123456").await;
        for (name, color, changed) in [
            ("Hírek", "#abcdef", json!(["color"])),
            ("Friss hírek", "#abcdef", json!(["name"])),
            // A colour in another case is another value: stored as written.
            ("Friss hírek", "#ABCDEF", json!(["color"])),
            ("Hírek", "#123456", json!(["color", "name"])),
            ("Hírek", "#123456", json!([])),
        ] {
            let txn = db.begin().await.expect("begin");
            Model::find_live_by_id(&txn, label.id)
                .await
                .expect("live")
                .into_active_model()
                .edit(&txn, actor, &input(name, color), None)
                .await
                .expect("saved");
            txn.commit().await.expect("commit");
            let entry = activity_log::Entity::find()
                .filter(activity_log::Column::EntityType.eq("labels"))
                .filter(activity_log::Column::EntityId.eq(label.id))
                .order_by_desc(activity_log::Column::Id)
                .one(db)
                .await
                .expect("read")
                .expect("the latest entry");
            assert_eq!(
                entry
                    .details
                    .as_ref()
                    .and_then(|details| details.get(CHANGED_FIELDS_KEY)),
                Some(&changed),
                "{name} {color}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn only_the_named_unique_index_is_a_conflict() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let label = create(db, Actor::System, "Hírek", "#123456").await;
        // An `execute` fails as `DbErr::Exec`, the model's `RETURNING`
        // statements as `DbErr::Query`; both are read.
        let duplicate_name = db
            .execute_unprepared("INSERT INTO labels (name, color) VALUES ('HÍREK', '#000000')")
            .await
            .expect_err("the live name is taken");
        assert!(unique_index::refused_by(&duplicate_name, LIVE_NAME_INDEX));
        assert!(!unique_index::refused_by(&duplicate_name, "labels_pkey"));
        assert!(matches!(
            unique_index::conflict_or_error(duplicate_name, LIVE_NAME_INDEX),
            ModelError::EntityAlreadyExists
        ));

        let duplicate_id = db
            .execute_unprepared(&format!(
                "INSERT INTO labels (id, name, color) VALUES ({}, 'Más', '#000000')",
                label.id
            ))
            .await
            .expect_err("the id is taken");
        assert!(unique_index::refused_by(&duplicate_id, "labels_pkey"));
        assert!(!unique_index::refused_by(&duplicate_id, LIVE_NAME_INDEX));
        assert!(matches!(
            unique_index::conflict_or_error(duplicate_id, LIVE_NAME_INDEX),
            ModelError::DbErr(_)
        ));

        // A CHECK violation is no unique violation, whatever it names.
        let bad_color = db
            .execute_unprepared("INSERT INTO labels (name, color) VALUES ('Piros', 'red')")
            .await
            .expect_err("the colour CHECK refuses it");
        assert!(!unique_index::refused_by(&bad_color, LIVE_NAME_INDEX));
        assert!(!unique_index::refused_by(&bad_color, "labels_color_format"));

        // The model's own insert, a query with `RETURNING`.
        let txn = db.begin().await.expect("begin");
        let outcome = Model::create(&txn, Actor::System, &input("hírek", "#000000"), None).await;
        txn.rollback().await.expect("roll back");
        assert!(
            matches!(outcome, Err(ModelError::EntityAlreadyExists)),
            "{outcome:?}"
        );
        // Nothing of the refused writes is left.
        assert_eq!(count(db, "labels").await, 1);
        let snapshot: Value = revisions::Model::for_record(db, "labels", label.id)
            .await
            .expect("revisions")[0]
            .snapshot
            .clone();
        assert_eq!(snapshot, json!({"name": "Hírek", "color": "#123456"}));
    })
    .await;
}
