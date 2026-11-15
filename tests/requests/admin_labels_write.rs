//! `POST /api/admin/labels` and `PUT /api/admin/labels/{id}`: the label
//! writes through the shared write contract (parity LBL-02, SHR-09, SHR-10,
//! SHR-13, SHR-18; ADR 0005, ADR 0007 §4, ADR 0009, ADR 0011 §2-§7,
//! ADR 0021 §4).
//!
//! Who may write: `edit labels` only; `edit posts`, which reads labels for
//! the post form's picker, gets `403` like any other account without it,
//! and nobody without a session gets past `401`, before the body is read.
//! What a write answers: the saved label, trimmed (`201` for a create, `200`
//! for an edit), with its audit columns, its revision and its one activity
//! entry written in one transaction; `422 invalid_body` for a body that is
//! not exactly `{name, color}`; a `422` on the first field that cannot be
//! stored, with its `params`; `409 label.name_taken` on `name` for a name
//! that a live label has, whatever its case; and `404` for an edit of a
//! label that is missing or in the trash. A refused write writes nothing.
//! Every answer is `no-store`. Every account and label is invented.
//!
//! Every request carries the admin origin in `Origin`, as the admin UI's
//! does, so that the route table's guard lets a cookie-authenticated write
//! through to what these tests check (ADR 0006 §3, ADR 0044); the guard's
//! own refusal of a foreign origin is `tests/requests/admin_origin.rs`'s.

use axum::http::{
    Method, StatusCode,
    header::{COOKIE, ORIGIN},
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    controllers::admin::JSON_BODY_LIMIT,
    domain::{
        activity::{Action, Actor, CHANGED_FIELDS_KEY},
        authz::{Permission, Role},
        revision::Reason,
        session::SessionScope,
        session_cookie::SESSION_COOKIE_NAME,
    },
    models::{activity_log, labels, revisions, users::Model as User},
};
use loco_rs::app::AppContext;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, QueryFilter, QueryOrder, Statement,
};
use serde_json::{Value, json};

use crate::support::{
    ADMIN_ORIGIN,
    admin_probe::{account_with, session_token},
    with_server,
};

/// The list route, where a create posts.
const LIST: &str = "/api/admin/labels";

/// A request of `method` to `path` with a full session of `user`, or
/// without a session, from the admin origin.
fn request(
    server: &TestServer,
    ctx: &AppContext,
    method: Method,
    path: &str,
    user: Option<&User>,
) -> axum_test::TestRequest {
    let request = server.method(method, path).add_header(ORIGIN, ADMIN_ORIGIN);
    match user {
        Some(user) => {
            let token = session_token(ctx, user, SessionScope::Admin);
            request.add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
        }
        None => request,
    }
}

/// `method path` with the JSON `body`.
async fn send(
    server: &TestServer,
    ctx: &AppContext,
    method: Method,
    path: &str,
    user: Option<&User>,
    body: &Value,
) -> TestResponse {
    request(server, ctx, method, path, user).json(body).await
}

/// `method path` with `body` as it is, sent as `application/json`.
async fn send_raw(
    server: &TestServer,
    ctx: &AppContext,
    method: Method,
    path: &str,
    user: Option<&User>,
    body: String,
) -> TestResponse {
    request(server, ctx, method, path, user)
        .bytes(body.into())
        .content_type("application/json")
        .await
}

/// A writer, who holds `edit labels` through the role (ADR 0007 §2).
async fn writer(ctx: &AppContext) -> User {
    account_with(ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await
}

/// Creates a label through the API as `user` and returns its id.
async fn create(
    server: &TestServer,
    ctx: &AppContext,
    user: &User,
    name: &str,
    color: &str,
) -> i64 {
    let response = send(
        server,
        ctx,
        Method::POST,
        LIST,
        Some(user),
        &json!({"name": name, "color": color}),
    )
    .await;
    assert_eq!(response.status_code(), StatusCode::CREATED, "{name}");
    response.json::<Value>()["id"].as_i64().expect("an id")
}

/// Moves the label `id` to the trash, as the trash slice will.
async fn trash(ctx: &AppContext, id: i64) {
    ctx.db
        .execute_unprepared(&format!(
            "UPDATE labels SET deleted_at = CURRENT_TIMESTAMP WHERE id = {id}"
        ))
        .await
        .expect("trash");
}

/// The number of labels, revisions and activity entries.
async fn totals(ctx: &AppContext) -> (i64, i64, i64) {
    let mut counts = [0_i64; 3];
    for (count, table) in counts
        .iter_mut()
        .zip(["labels", "revisions", "activity_log"])
    {
        *count = ctx
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                format!("SELECT count(*) FROM {table}"),
            ))
            .await
            .expect("count")
            .expect("one row")
            .try_get_by_index(0)
            .expect("a count");
    }
    (counts[0], counts[1], counts[2])
}

/// The stored label `id`.
async fn stored(ctx: &AppContext, id: i64) -> labels::Model {
    labels::Entity::find_by_id(id)
        .one(&ctx.db)
        .await
        .expect("read the label")
        .expect("the label exists")
}

/// The activity entries about label `id`, oldest first.
async fn entries(ctx: &AppContext, id: i64) -> Vec<activity_log::Model> {
    activity_log::Entity::find()
        .filter(activity_log::Column::EntityType.eq("labels"))
        .filter(activity_log::Column::EntityId.eq(id))
        .order_by_asc(activity_log::Column::Id)
        .all(&ctx.db)
        .await
        .expect("read the entries")
}

/// The response's request id, which Loco made.
fn request_id(response: &TestResponse) -> String {
    response
        .header("x-request-id")
        .to_str()
        .expect("ASCII")
        .to_owned()
}

/// Asserts an admin error envelope of `code` on `field` with `params`,
/// `no-store`.
fn assert_envelope(
    response: &TestResponse,
    status: StatusCode,
    code: &str,
    field: &Value,
    params: &Value,
    case: &str,
) {
    assert_eq!(response.status_code(), status, "{case}");
    assert_eq!(response.header("cache-control"), "no-store", "{case}");
    let body = response.json::<Value>();
    assert_eq!(body["error"], json!(code), "{case}: {body}");
    assert_eq!(&body["field"], field, "{case}: {body}");
    assert_eq!(&body["params"], params, "{case}: {body}");
    assert!(
        body["description"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "{case}: {body}"
    );
}

#[tokio::test]
async fn every_account_with_edit_labels_may_create_and_edit_labels() {
    // ADR 0007 §2, §4: create and update need `edit labels`, which the
    // writer, supervisor and admin roles hold, and an extra grant gives.
    with_server(|server, ctx| async move {
        let author = writer(&ctx).await;
        let id = create(&server, &ctx, &author, "Hírek", "#123456").await;
        let allowed = [
            author,
            account_with(&ctx, "fo@example.test", &[Role::Supervisor], &[], |_| {}).await,
            account_with(&ctx, "admin@example.test", &[Role::Admin], &[], |_| {}).await,
            account_with(
                &ctx,
                "cimkek@example.test",
                &[],
                &[Permission::EditLabels],
                |_| {},
            )
            .await,
        ];
        for (index, user) in allowed.iter().enumerate() {
            let case = user.email.clone().unwrap_or_default();
            let name = format!("Címke {index}");
            let created = send(
                &server,
                &ctx,
                Method::POST,
                LIST,
                Some(user),
                &json!({"name": name, "color": "#123456"}),
            )
            .await;
            assert_eq!(created.status_code(), StatusCode::CREATED, "{case}");
            let edited = send(
                &server,
                &ctx,
                Method::PUT,
                &format!("{LIST}/{id}"),
                Some(user),
                &json!({"name": "Hírek", "color": format!("#00000{index}")}),
            )
            .await;
            assert_eq!(edited.status_code(), StatusCode::OK, "{case}");
        }
    })
    .await;
}

#[tokio::test]
async fn every_account_without_edit_labels_is_refused_before_the_body_is_read() {
    // ADR 0007 §4, §5: the picker's `edit posts` reads labels but never
    // writes one; the guard answers `403`, or `401` without a session, and
    // the handler never runs.
    with_server(|server, ctx| async move {
        let id = create(&server, &ctx, &writer(&ctx).await, "Hírek", "#123456").await;
        let before = totals(&ctx).await;
        let refused = [
            // The picker's permission alone, and as an extra grant.
            account_with(
                &ctx,
                "bejegyzesek@example.test",
                &[],
                &[Permission::EditPosts],
                |_| {},
            )
            .await,
            account_with(
                &ctx,
                "titkar-bejegyzes@example.test",
                &[Role::Secretary],
                &[Permission::EditPosts, Permission::PublishPosts],
                |_| {},
            )
            .await,
            account_with(&ctx, "titkar@example.test", &[Role::Secretary], &[], |_| {}).await,
            account_with(&ctx, "senki@example.test", &[], &[], |_| {}).await,
        ];
        let id_path = format!("{LIST}/{id}");
        for user in &refused {
            let case = user.email.clone().unwrap_or_default();
            for (method, path) in [(Method::POST, LIST), (Method::PUT, id_path.as_str())] {
                // A valid body and one that is not even JSON: the guard
                // answers before the body is read (ADR 0007 §5).
                for body in [
                    json!({"name": "Tilos", "color": "#123456"}).to_string(),
                    "not JSON".to_owned(),
                ] {
                    assert_envelope(
                        &send_raw(&server, &ctx, method.clone(), path, Some(user), body).await,
                        StatusCode::FORBIDDEN,
                        "forbidden",
                        &Value::Null,
                        &json!({}),
                        &format!("{case} {method} {path}"),
                    );
                }
            }
        }
        // No session: 401, also for an id that cannot name a label.
        for (method, path) in [
            (Method::POST, LIST.to_owned()),
            (Method::PUT, id_path.clone()),
            (Method::PUT, format!("{LIST}/abc")),
        ] {
            assert_envelope(
                &send(
                    &server,
                    &ctx,
                    method.clone(),
                    &path,
                    None,
                    &json!({"name": "Tilos", "color": "#123456"}),
                )
                .await,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                &Value::Null,
                &json!({}),
                &format!("{method} {path}"),
            );
        }
        assert_eq!(totals(&ctx).await, before, "a refused write writes nothing");
    })
    .await;
}

#[tokio::test]
async fn a_create_answers_201_with_the_trimmed_label_and_writes_its_safety_net() {
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let response = send(
            &server,
            &ctx,
            Method::POST,
            LIST,
            Some(&user),
            &json!({"name": "  Ünnepség a tornateremben \n", "color": " #1e88E5\t"}),
        )
        .await;
        assert_eq!(response.status_code(), StatusCode::CREATED);
        assert_eq!(response.header("cache-control"), "no-store");
        assert_eq!(response.header("content-type"), "application/json");
        let body = response.json::<Value>();
        let id = body["id"].as_i64().expect("an id");
        // SHR-09: stored trimmed, otherwise as written.
        assert_eq!(
            body,
            json!({"id": id, "name": "Ünnepség a tornateremben", "color": "#1e88E5"})
        );
        // The record route reads the same label.
        let read = request(
            &server,
            &ctx,
            Method::GET,
            &format!("{LIST}/{id}"),
            Some(&user),
        )
        .await;
        assert_eq!(read.json::<Value>(), body);

        // ADR 0011 §2: the actor in both audit columns, one instant.
        let label = stored(&ctx, id).await;
        assert_eq!(
            (label.created_by, label.updated_by),
            (Some(user.id), Some(user.id))
        );
        assert_eq!(label.created_at, label.updated_at);
        assert_eq!((label.deleted_at, label.deleted_by), (None, None));

        // ADR 0011 §4, §5: revision 1 and one `create` entry, in the same
        // transaction and the same request.
        let revisions = revisions::Model::for_record(&ctx.db, "labels", id)
            .await
            .expect("revisions");
        assert_eq!(revisions.len(), 1);
        let revision = &revisions[0];
        assert_eq!(revision.revision_no, 1);
        assert_eq!(revision.reason(), Ok(Reason::Create));
        assert_eq!(revision.schema_version, 1);
        assert_eq!(revision.created_by, Some(user.id));
        assert_eq!(
            revision.snapshot,
            json!({"name": "Ünnepség a tornateremben", "color": "#1e88E5"})
        );
        assert_eq!(revision.created_at, label.created_at);
        let entries = entries(&ctx, id).await;
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.action(), Ok(Action::Create));
        assert_eq!(entry.actor(), Ok(Actor::User(user.id)));
        assert_eq!(entry.revision_id, Some(revision.id));
        assert_eq!(entry.label.as_deref(), Some("Ünnepség a tornateremben"));
        assert_eq!(entry.occurred_at, revision.created_at);
        let request_id = request_id(&response);
        assert_eq!(entry.request_id.as_deref(), Some(request_id.as_str()));
        assert_eq!(revision.request_id.as_deref(), Some(request_id.as_str()));
        // A content entry never holds the client's address (ADR 0011 §5).
        assert_eq!(entry.client_address(), None);
    })
    .await;
}

#[tokio::test]
async fn an_edit_answers_200_and_writes_the_next_revision_with_the_changed_fields() {
    with_server(|server, ctx| async move {
        let author = writer(&ctx).await;
        let editor = account_with(&ctx, "fo@example.test", &[Role::Supervisor], &[], |_| {}).await;
        let id = create(&server, &ctx, &author, "Ünnepség", "#1E88E5").await;
        let created = stored(&ctx, id).await;
        let response = send(
            &server,
            &ctx,
            Method::PUT,
            &format!("{LIST}/{id}"),
            Some(&editor),
            &json!({"name": " Ünnepség ", "color": "#000000"}),
        )
        .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.header("cache-control"), "no-store");
        assert_eq!(
            response.json::<Value>(),
            json!({"id": id, "name": "Ünnepség", "color": "#000000"})
        );

        // ADR 0011 §2: the editor in `updated_by`; the creation untouched.
        let label = stored(&ctx, id).await;
        assert_eq!(
            (label.created_by, label.created_at),
            (Some(author.id), created.created_at)
        );
        assert_eq!(label.updated_by, Some(editor.id));
        assert!(label.updated_at >= created.updated_at);

        let revisions = revisions::Model::for_record(&ctx.db, "labels", id)
            .await
            .expect("revisions");
        assert_eq!(revisions.len(), 2);
        let revision = &revisions[1];
        assert_eq!(revision.revision_no, 2);
        assert_eq!(revision.reason(), Ok(Reason::Update));
        assert_eq!(revision.created_by, Some(editor.id));
        assert_eq!(
            revision.snapshot,
            json!({"name": "Ünnepség", "color": "#000000"})
        );
        // PostgreSQL's clock, the start of the edit's transaction (D90).
        assert_eq!(label.updated_at, revision.created_at);

        let entries = entries(&ctx, id).await;
        assert_eq!(entries.len(), 2);
        let entry = &entries[1];
        assert_eq!(entry.action(), Ok(Action::Update));
        assert_eq!(entry.actor(), Ok(Actor::User(editor.id)));
        assert_eq!(entry.revision_id, Some(revision.id));
        assert_eq!(entry.label.as_deref(), Some("Ünnepség"));
        // The names of the changed fields, never their values.
        assert_eq!(
            entry.details,
            Some(json!({ CHANGED_FIELDS_KEY: ["color"] }))
        );
        assert_eq!(
            entry.request_id.as_deref(),
            Some(request_id(&response).as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn every_field_rule_is_a_422_on_its_field_and_writes_nothing() {
    // SHR-13: every limit of the table is a field error, never a 500.
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let id = create(&server, &ctx, &user, "Hírek", "#123456").await;
        let too_long = "ő".repeat(65);
        let cases = [
            ("", "#123456", "field_required", "name", json!({})),
            (" \t\n", "#123456", "field_required", "name", json!({})),
            (
                too_long.as_str(),
                "#123456",
                "field_too_long",
                "name",
                json!({"max": 64}),
            ),
            (
                "Hír\u{0}ek",
                "#123456",
                "field_invalid_character",
                "name",
                json!({}),
            ),
            ("Sport", "", "field_required", "color", json!({})),
            ("Sport", "   ", "field_required", "color", json!({})),
            ("Sport", "#12345", "field_invalid_color", "color", json!({})),
            (
                "Sport",
                "#1234567",
                "field_invalid_color",
                "color",
                json!({}),
            ),
            ("Sport", "123456", "field_invalid_color", "color", json!({})),
            (
                "Sport",
                "#12345G",
                "field_invalid_color",
                "color",
                json!({}),
            ),
            ("Sport", "piros", "field_invalid_color", "color", json!({})),
            // The name is checked first: the envelope names one field.
            ("", "piros", "field_required", "name", json!({})),
        ];
        let before = totals(&ctx).await;
        let record = format!("{LIST}/{id}");
        for (name, color, code, field, params) in &cases {
            for (method, path) in [(Method::POST, LIST), (Method::PUT, record.as_str())] {
                let response = send(
                    &server,
                    &ctx,
                    method.clone(),
                    path,
                    Some(&user),
                    &json!({"name": name, "color": color}),
                )
                .await;
                assert_envelope(
                    &response,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    code,
                    &json!(field),
                    params,
                    &format!("{method} {name:?} {color:?}"),
                );
            }
        }
        // The refusal is checked before the label is looked up: a missing
        // label with a refused field is the field's 422.
        assert_envelope(
            &send(
                &server,
                &ctx,
                Method::PUT,
                &format!("{LIST}/999999"),
                Some(&user),
                &json!({"name": "", "color": "#123456"}),
            )
            .await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "field_required",
            &json!("name"),
            &json!({}),
            "a missing label",
        );
        assert_eq!(totals(&ctx).await, before, "a refused write writes nothing");
        let label = stored(&ctx, id).await;
        assert_eq!(
            (label.name.as_str(), label.color.as_str()),
            ("Hírek", "#123456")
        );
    })
    .await;
}

#[tokio::test]
async fn a_name_at_the_limit_is_accepted_counted_in_characters() {
    // LBL-02: up to 64 characters, as PostgreSQL's `length()` counts them:
    // 64 two-byte letters are 128 bytes and still fit.
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let longest = "ő".repeat(64);
        let response = send(
            &server,
            &ctx,
            Method::POST,
            LIST,
            Some(&user),
            &json!({"name": longest, "color": "#ABCDEF"}),
        )
        .await;
        assert_eq!(response.status_code(), StatusCode::CREATED);
        assert_eq!(response.json::<Value>()["name"], json!(longest));
    })
    .await;
}

#[tokio::test]
async fn a_body_that_is_not_exactly_the_label_request_is_invalid_body() {
    // SHR-10: only the declared, writable fields are read; a body with any
    // other member is refused whole, before anything is checked or written.
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let id = create(&server, &ctx, &user, "Hírek", "#123456").await;
        let record = format!("{LIST}/{id}");
        let bodies = [
            // What ra-core's record holds besides the two fields.
            json!({"id": id, "name": "Sport", "color": "#123456"}).to_string(),
            json!({"name": "Sport", "color": "#123456", "created_by": 1}).to_string(),
            json!({"name": "Sport", "color": "#123456", "deleted_at": null}).to_string(),
            json!({"name": "Sport", "colour": "#123456"}).to_string(),
            json!({"name": "Sport"}).to_string(),
            json!({"color": "#123456"}).to_string(),
            json!({"name": null, "color": "#123456"}).to_string(),
            json!({"name": 5, "color": "#123456"}).to_string(),
            json!(["Sport", "#123456"]).to_string(),
            r##"{"name":"Sport","name":"Hírek","color":"#123456"}"##.to_owned(),
            "{}".to_owned(),
            "Sport".to_owned(),
            r#"{"name":"#.to_owned(),
            String::new(),
        ];
        let before = totals(&ctx).await;
        for body in &bodies {
            for (method, path) in [(Method::POST, LIST), (Method::PUT, record.as_str())] {
                assert_envelope(
                    &send_raw(
                        &server,
                        &ctx,
                        method.clone(),
                        path,
                        Some(&user),
                        body.clone(),
                    )
                    .await,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_body",
                    &Value::Null,
                    &json!({}),
                    &format!("{method} {body}"),
                );
            }
        }
        // Not sent as JSON.
        let response = request(&server, &ctx, Method::POST, LIST, Some(&user))
            .bytes(
                json!({"name": "Sport", "color": "#123456"})
                    .to_string()
                    .into(),
            )
            .content_type("text/plain")
            .await;
        assert_envelope(
            &response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_body",
            &Value::Null,
            &json!({}),
            "text/plain",
        );
        // Above the admin API's 4 MiB JSON limit (ADR 0023 §5).
        let head = r##"{"name":"Sport","color":"#123456""##;
        let oversized = format!("{head}{}}}", " ".repeat(JSON_BODY_LIMIT - head.len()));
        for (method, path) in [(Method::POST, LIST), (Method::PUT, record.as_str())] {
            assert_envelope(
                &send_raw(
                    &server,
                    &ctx,
                    method.clone(),
                    path,
                    Some(&user),
                    oversized.clone(),
                )
                .await,
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                &Value::Null,
                &json!({}),
                &format!("{method} oversized"),
            );
        }
        assert_eq!(totals(&ctx).await, before, "a refused body writes nothing");
    })
    .await;
}

#[tokio::test]
async fn a_live_name_is_taken_whatever_its_case_and_a_trashed_one_is_free() {
    // ADR 0009 "Constraints", ADR 0011 §3: the live-name index decides.
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let news = create(&server, &ctx, &user, "Hírek", "#123456").await;
        let sport = create(&server, &ctx, &user, "Sport", "#654321").await;
        let before = totals(&ctx).await;
        for (sent, shown) in [
            ("HÍREK", "HÍREK"),
            ("  hírek ", "hírek"),
            ("Hírek", "Hírek"),
            // A byte-order mark pasted with the name is trimmed as the admin
            // trims it, so the name is the one already taken.
            ("\u{feff}Hírek\u{feff}", "Hírek"),
        ] {
            assert_envelope(
                &send(
                    &server,
                    &ctx,
                    Method::POST,
                    LIST,
                    Some(&user),
                    &json!({"name": sent, "color": "#000000"}),
                )
                .await,
                StatusCode::CONFLICT,
                "label.name_taken",
                &json!("name"),
                &json!({"name": shown}),
                sent,
            );
        }
        // An edit may not take another live label's name...
        assert_envelope(
            &send(
                &server,
                &ctx,
                Method::PUT,
                &format!("{LIST}/{sport}"),
                Some(&user),
                &json!({"name": "hÍrEk", "color": "#654321"}),
            )
            .await,
            StatusCode::CONFLICT,
            "label.name_taken",
            &json!("name"),
            &json!({"name": "hÍrEk"}),
            "edit",
        );
        assert_eq!(totals(&ctx).await, before, "a refused write writes nothing");
        assert_eq!(stored(&ctx, sport).await.name, "Sport");
        // ...but may change the case of its own.
        let response = send(
            &server,
            &ctx,
            Method::PUT,
            &format!("{LIST}/{news}"),
            Some(&user),
            &json!({"name": "HÍREK", "color": "#123456"}),
        )
        .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.json::<Value>()["name"], json!("HÍREK"));
        // A trashed label's name is free again.
        trash(&ctx, sport).await;
        let again = create(&server, &ctx, &user, "sport", "#654321").await;
        assert_ne!(again, sport);
    })
    .await;
}

#[tokio::test]
async fn an_edit_of_a_label_that_is_missing_trashed_or_unnamable_is_404() {
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let trashed = create(&server, &ctx, &user, "Lomtár", "#000000").await;
        trash(&ctx, trashed).await;
        let before = totals(&ctx).await;
        let valid = json!({"name": "Új név", "color": "#123456"});
        for id in [
            trashed.to_string(),
            "999999".to_owned(),
            "0".to_owned(),
            "-1".to_owned(),
            "abc".to_owned(),
            "9007199254740992".to_owned(),
        ] {
            assert_envelope(
                &send(
                    &server,
                    &ctx,
                    Method::PUT,
                    &format!("{LIST}/{id}"),
                    Some(&user),
                    &valid,
                )
                .await,
                StatusCode::NOT_FOUND,
                "not_found",
                &Value::Null,
                &json!({}),
                &id,
            );
        }
        // An id that cannot name a label is read before the body.
        assert_envelope(
            &send_raw(
                &server,
                &ctx,
                Method::PUT,
                &format!("{LIST}/abc"),
                Some(&user),
                "not JSON".to_owned(),
            )
            .await,
            StatusCode::NOT_FOUND,
            "not_found",
            &Value::Null,
            &json!({}),
            "abc with a bad body",
        );
        assert_eq!(totals(&ctx).await, before, "a refused edit writes nothing");
        assert_eq!(stored(&ctx, trashed).await.name, "Lomtár");
    })
    .await;
}
