//! The named fixtures of the `/api/v1` tests (`tests/support/fixtures.rs`,
//! public-api.md §17.1) and the soft-deleted twin convention of ADR 0011.
//!
//! The registry itself: every fixture is registered under a §17.1 name,
//! once, and all of them load together. The mechanism, against PostgreSQL:
//! no public table exists yet, so probe fixtures write `users` rows, the one
//! trashable table of the schema so far, to show that a twin copies every
//! column, sits in the twin range and is in the trash; that a failing
//! fixture leaves nothing behind; that a live row in the twin range is
//! refused; and that identity sequences move past the fixture ids. Every
//! account in them is invented.

use backend3::models::users;
use chrono::{DateTime, Utc};
use loco_rs::prelude::{ActiveModelTrait, ActiveValue, Uuid};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};
use serde_json::{Value, json};

use crate::support::{
    fixtures::{
        FIXTURES, Fixture, FixtureError, FixtureWriter, Load, LoadFuture, TWIN_DELETED_AT,
        TWIN_ID_OFFSET, Twin, fixture_names, load, row_ids_in, section_17_1_names, try_load,
        twin_deleted_at, twin_id, twin_ids_in,
    },
    repository_root, with_app,
};

/// The text of public-api.md.
fn public_api_md() -> String {
    std::fs::read_to_string(repository_root().join("docs/design/public-api.md"))
        .expect("read docs/design/public-api.md")
}

#[test]
fn every_fixture_is_registered_once_under_a_section_17_1_name() {
    let names = section_17_1_names(&public_api_md());
    let mut seen = Vec::new();
    for fixture in FIXTURES {
        assert!(
            names.iter().any(|name| name == fixture.name),
            "`{}` is not a fixture name of public-api.md §17.1: register it under the name of \
             its table row",
            fixture.name
        );
        assert!(!seen.contains(&fixture.name), "`{}` twice", fixture.name);
        seen.push(fixture.name);
    }
    assert_eq!(fixture_names(), seen);
}

#[test]
fn the_section_17_1_names_are_read_from_its_tables() {
    let names = section_17_1_names(&public_api_md());
    // The first and last row of the section, and one row of each table.
    for expected in [
        "Backdated post",
        "Trashed label",
        "New-year week",
        "Autumn overlap",
        "Hungarian order",
        "Menu edge cases",
        "Diáktoll issues",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected}: {names:?}"
        );
    }
    assert_eq!(names.first().map(String::as_str), Some("Backdated post"));
    assert_eq!(names.last().map(String::as_str), Some("Diáktoll issues"));
    // §17.2's first column is the legacy shape, not a fixture name.
    assert!(!names.iter().any(|name| name == "Entity-encoded body"));
    let sample = "## 17\n| Fixture | Shape |\n|---|---|\n| A | x |\n### 17.1 Seed\n\n\
                  | Fixture | Shape | Pins |\n|---|---|---|\n| One | a | b |\n\ntext | no\n\
                  | Two | c | d |\n### 17.2 Import\n| Three | e | f |\n";
    assert_eq!(section_17_1_names(sample), ["One", "Two"]);
    assert_eq!(section_17_1_names("no section"), Vec::<String>::new());
}

#[tokio::test]
async fn every_registered_fixture_loads_together() {
    with_app(|_router, ctx| async move {
        let names = fixture_names();
        let loaded = load(&ctx.db, &names).await;
        assert_eq!(loaded.fixtures, names);
    })
    .await;
}

#[test]
fn the_twin_constants_hold_together() {
    assert_eq!(twin_id(7), 1_000_000_007);
    // Every twin id is still exact as a JSON number (ADR 0005 §3).
    assert!(twin_id(TWIN_ID_OFFSET - 1) < 9_007_199_254_740_991);
    assert_eq!(
        twin_deleted_at(),
        "2026-09-01T06:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("RFC 3339")
    );
    assert_eq!(TWIN_DELETED_AT, "2026-09-01T06:00:00Z");
}

#[test]
fn twin_ids_are_found_anywhere_in_a_body() {
    let twin = twin_id(3);
    let body = json!({
        "id": 1,
        "labels": [{ "id": 2 }, { "id": twin }],
        "author": { "id": twin, "name": "x" },
        "items": [[{ "id": TWIN_ID_OFFSET }]],
        "a/b~c": { "id": twin }
    });
    let mut found = twin_ids_in(&body);
    found.sort();
    assert_eq!(
        found,
        [
            format!("/author/id: {twin}"),
            format!("/a~1b~0c/id: {twin}"),
            format!("/items/0/0/id: {TWIN_ID_OFFSET}"),
            format!("/labels/1/id: {twin}"),
        ]
    );
    // Ids below the range, ids that are not integers, and big numbers under
    // other names are not twins.
    let clean = json!({
        "id": TWIN_ID_OFFSET - 1,
        "items": [{ "id": "1000000007" }, { "id": 1.5e9 }],
        "count": twin
    });
    assert_eq!(twin_ids_in(&clean), Vec::<String>::new());
}

#[test]
fn row_ids_are_the_ids_below_the_twin_range_each_once() {
    let body = json!({
        "id": 1,
        "labels": [{ "id": 2 }, { "id": twin_id(2) }, { "id": 2 }],
        "items": [{ "id": "3" }, { "id": 4.5 }, { "count": 5 }]
    });
    assert_eq!(row_ids_in(&body).into_iter().collect::<Vec<_>>(), [1, 2]);
    assert!(row_ids_in(&json!({ "items": [] })).is_empty());
}

/// The address of the probe account `id`; `example.invalid` cannot belong
/// to anyone (RFC 2606).
fn address(id: i64) -> String {
    format!("proba{id}@example.invalid")
}

/// Writes the live probe account `id` through the model, as a fixture does.
async fn write_account(writer: &FixtureWriter<'_>, id: i64) -> Result<(), DbErr> {
    users::ActiveModel {
        id: ActiveValue::Set(id),
        email: ActiveValue::Set(Some(address(id))),
        name: ActiveValue::Set(format!("Próba Fiók {id}")),
        ..Default::default()
    }
    .insert(writer.db())
    .await?;
    Ok(())
}

/// A probe fixture: account 7 and its twin, which needs its own `pid` and
/// address, because both are unique over trashed rows too.
fn account_with_twin<'w>(writer: &'w mut FixtureWriter<'_>) -> LoadFuture<'w> {
    Box::pin(async move {
        write_account(writer, 7).await?;
        writer
            .twin_with(
                "users",
                7,
                json!({ "pid": Uuid::new_v4().to_string(), "email": address(twin_id(7)) }),
            )
            .await?;
        Ok(())
    })
}

/// A probe fixture that writes account 8 and then fails.
fn failing<'w>(writer: &'w mut FixtureWriter<'_>) -> LoadFuture<'w> {
    Box::pin(async move {
        write_account(writer, 8).await?;
        Err(DbErr::Custom("the probe fails".to_owned()))
    })
}

/// A probe fixture with a live row in the twin range.
fn live_in_twin_range<'w>(writer: &'w mut FixtureWriter<'_>) -> LoadFuture<'w> {
    Box::pin(async move { write_account(writer, TWIN_ID_OFFSET + 5).await })
}

/// The probe registry.
const PROBES: &[Fixture] = &[
    Fixture {
        name: "Account with twin",
        load: account_with_twin,
    },
    Fixture {
        name: "Failing",
        load: failing,
    },
    Fixture {
        name: "Live in twin range",
        load: live_in_twin_range,
    },
];

/// Every `users` row as `(id, deleted_at, the row as JSON)`, by id.
async fn accounts(db: &DatabaseConnection) -> Vec<(i64, Option<DateTime<Utc>>, Value)> {
    let sql = "SELECT id, deleted_at, to_jsonb(u) FROM users AS u ORDER BY id";
    db.query_all_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect(sql)
        .iter()
        .map(|row| {
            (
                row.try_get_by_index(0).expect("id"),
                row.try_get_by_index(1).expect("deleted_at"),
                row.try_get_by_index(2).expect("row"),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_twin_is_the_row_again_in_the_trash_with_a_twin_id() {
    with_app(|_router, ctx| async move {
        let loaded = try_load(PROBES, &ctx.db, &["Account with twin"])
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(loaded.fixtures, ["Account with twin"]);
        assert_eq!(
            loaded.twins,
            [Twin {
                fixture: "Account with twin",
                table: "users".to_owned(),
                live_id: 7,
                id: twin_id(7),
            }]
        );
        assert_eq!(loaded.twin_ids("users"), [twin_id(7)]);
        assert_eq!(loaded.twin_ids("posts"), [] as [i64; 0]);

        let rows = accounts(&ctx.db).await;
        let [(live_id, live_deleted, live), (copy_id, copy_deleted, copy)] = rows.as_slice() else {
            panic!("two accounts: {rows:?}");
        };
        assert_eq!((*live_id, *live_deleted), (7, None));
        assert_eq!(
            (*copy_id, *copy_deleted),
            (twin_id(7), Some(twin_deleted_at()))
        );
        // Every other column is copied, apart from the overrides.
        for (column, value) in live.as_object().expect("a row") {
            match column.as_str() {
                "id" | "deleted_at" | "pid" | "email" => {}
                _ => assert_eq!(&copy[column], value, "{column}"),
            }
        }
        assert_eq!(copy["email"], json!(address(twin_id(7))));
        assert_ne!(copy["pid"], live["pid"]);
    })
    .await;
}

#[tokio::test]
async fn identity_sequences_move_past_the_fixture_ids() {
    with_app(|_router, ctx| async move {
        try_load(PROBES, &ctx.db, &["Account with twin"])
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        // A row a test creates afterwards gets the next id below the twin
        // range, not one a fixture holds.
        let created = users::ActiveModel {
            email: ActiveValue::Set(Some(address(0))),
            name: ActiveValue::Set("Próba Fiók".to_owned()),
            ..Default::default()
        }
        .insert(&ctx.db)
        .await
        .expect("insert without an id");
        assert_eq!(created.id, 8);
    })
    .await;
}

#[tokio::test]
async fn a_failing_fixture_leaves_nothing_behind() {
    with_app(|_router, ctx| async move {
        let error = try_load(PROBES, &ctx.db, &["Account with twin", "Failing"])
            .await
            .expect_err("the probe fails");
        assert!(
            matches!(
                &error,
                FixtureError::Load {
                    fixture: "Failing",
                    ..
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains("the probe fails"), "{error}");
        assert_eq!(accounts(&ctx.db).await, []);
    })
    .await;
}

#[tokio::test]
async fn a_live_row_in_the_twin_range_is_refused() {
    with_app(|_router, ctx| async move {
        let error = try_load(PROBES, &ctx.db, &["Live in twin range"])
            .await
            .expect_err("the range is for twins");
        assert!(
            matches!(&error, FixtureError::LiveRowInTwinRange { table, ids }
                if table == "users" && *ids == [TWIN_ID_OFFSET + 5]),
            "{error}"
        );
        assert!(
            error.to_string().contains("Give fixture rows smaller ids"),
            "{error}"
        );
        assert_eq!(accounts(&ctx.db).await, []);
    })
    .await;
}

#[tokio::test]
async fn unknown_and_repeated_names_are_refused_before_anything_runs() {
    with_app(|_router, ctx| async move {
        let error = try_load(PROBES, &ctx.db, &["Account with twin", "Nope"])
            .await
            .expect_err("unknown");
        assert!(
            matches!(&error, FixtureError::Unknown { name, known }
                if name == "Nope" && known.len() == PROBES.len()),
            "{error}"
        );
        assert!(
            error.to_string().contains("register it in FIXTURES"),
            "{error}"
        );
        let error = try_load(PROBES, &ctx.db, &["Failing", "Failing"])
            .await
            .expect_err("twice");
        assert!(matches!(&error, FixtureError::RequestedTwice(name) if name == "Failing"));
        let doubled = [PROBES[0], PROBES[0]];
        let error = try_load(&doubled, &ctx.db, &[])
            .await
            .expect_err("registered twice");
        assert!(matches!(
            error,
            FixtureError::DuplicateRegistration("Account with twin")
        ));
        assert_eq!(accounts(&ctx.db).await, []);
    })
    .await;
}

/// A probe fixture that writes account 9 and then runs `twin` on it.
macro_rules! twin_probe {
    ($name:ident, $table:expr, $id:expr, $overrides:expr) => {
        fn $name<'w>(writer: &'w mut FixtureWriter<'_>) -> LoadFuture<'w> {
            Box::pin(async move {
                write_account(writer, 9).await?;
                writer.twin_with($table, $id, $overrides).await?;
                Ok(())
            })
        }
    };
}

/// A probe fixture that copies account 9 without overrides, so the twin
/// has the same `pid` and address.
fn same_unique_values<'w>(writer: &'w mut FixtureWriter<'_>) -> LoadFuture<'w> {
    Box::pin(async move {
        write_account(writer, 9).await?;
        writer.twin("users", 9).await?;
        Ok(())
    })
}

twin_probe!(no_trash_column, "seaql_migrations", 9, json!({}));
twin_probe!(no_such_table, "nope", 9, json!({}));
twin_probe!(no_such_column, "users", 9, json!({ "nickname": "x" }));
twin_probe!(sets_the_id, "users", 9, json!({ "id": 5 }));
twin_probe!(not_an_object, "users", 9, json!(["email"]));
twin_probe!(missing_row, "users", 10, json!({}));
twin_probe!(row_in_twin_range, "users", TWIN_ID_OFFSET, json!({}));

#[tokio::test]
async fn a_twin_that_cannot_be_written_fails_its_fixture() {
    with_app(|_router, ctx| async move {
        for (load, finding) in [
            (
                same_unique_values as Load,
                // `pid` and the address are unique over trashed rows too.
                "duplicate key value",
            ),
            (no_trash_column, "has no deleted_at column"),
            (no_such_table, "there is no table nope"),
            (no_such_column, "users has no column nickname"),
            (sets_the_id, "the overrides may not set id"),
            (not_an_object, "the overrides are not a JSON object"),
            (missing_row, "the row does not exist"),
            (row_in_twin_range, "a fixture row's id is between 1 and"),
        ] {
            let registry = [Fixture {
                name: "Probe",
                load,
            }];
            let error = try_load(&registry, &ctx.db, &["Probe"])
                .await
                .expect_err(finding);
            assert!(error.to_string().contains(finding), "{finding}: {error}");
            assert_eq!(accounts(&ctx.db).await, [], "{finding}");
        }
    })
    .await;
}
