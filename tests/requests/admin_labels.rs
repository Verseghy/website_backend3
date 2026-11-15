//! `GET /api/admin/labels` and `GET /api/admin/labels/{id}` (parity LBL-01,
//! SHR-02, SHR-03; ADR 0007 §4, ADR 0020 §4, ADR 0021 §4).
//!
//! Who may read: `edit labels`, or `edit posts` for the post form's picker;
//! nobody else (`403`), and nobody without a session (`401`). What the list
//! answers: live labels only, by name in the Hungarian alphabet then id
//! descending, 25 to a page, as `{data, total}`, with each refusal of the
//! list contract a `422` envelope naming its parameter. The record route
//! answers a live label, and `404` for anything else. Every answer is
//! `no-store`. The writes have their own tests (`admin_labels_write.rs`).
//! Every account and label is invented.

use axum::http::{StatusCode, header::COOKIE};
use axum_test::{TestResponse, TestServer};
use backend3::{
    domain::{
        authz::{Permission, Role},
        session::SessionScope,
        session_cookie::SESSION_COOKIE_NAME,
    },
    models::users::Model as User,
};
use loco_rs::app::AppContext;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

use crate::support::{
    admin_probe::{account_with, session_token},
    with_server,
};

/// The list route.
const LIST: &str = "/api/admin/labels";

/// `GET path` with a full session of `user`, or without a session.
async fn get(
    server: &TestServer,
    ctx: &AppContext,
    path: &str,
    user: Option<&User>,
) -> TestResponse {
    let request = server.get(path);
    match user {
        Some(user) => {
            let token = session_token(ctx, user, SessionScope::Admin);
            request
                .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
                .await
        }
        None => request.await,
    }
}

/// Inserts a live label with an explicit id.
async fn label(ctx: &AppContext, id: i64, name: &str, color: &str) {
    ctx.db
        .execute_unprepared(&format!(
            "INSERT INTO labels (id, name, color) VALUES ({id}, '{name}', '{color}')"
        ))
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"));
}

/// Moves the label `id` to the trash.
async fn trash(ctx: &AppContext, id: i64) {
    ctx.db
        .execute_unprepared(&format!(
            "UPDATE labels SET deleted_at = TIMESTAMPTZ '2026-11-15 08:00:00+00' WHERE id = {id}"
        ))
        .await
        .expect("trash");
}

/// A writer, who holds `edit labels` through the role (ADR 0007 §2).
async fn writer(ctx: &AppContext) -> User {
    account_with(ctx, "iro@example.test", &[Role::Writer], &[], |_| {}).await
}

/// The ids of a list answer, in order.
fn ids(body: &Value) -> Vec<i64> {
    body["data"]
        .as_array()
        .expect("data is a list")
        .iter()
        .map(|row| row["id"].as_i64().expect("an id"))
        .collect()
}

/// Asserts a `200` list answer of `ids` and `total`, `no-store`.
fn assert_page(response: &TestResponse, expected: &[i64], total: u64, case: &str) {
    assert_eq!(response.status_code(), StatusCode::OK, "{case}");
    assert_eq!(response.header("cache-control"), "no-store", "{case}");
    let body = response.json::<Value>();
    assert_eq!(ids(&body), expected, "{case}");
    assert_eq!(body["total"], json!(total), "{case}");
}

/// The admin error envelope of `code` with `field` and the default
/// description's place taken by any text.
fn assert_envelope(response: &TestResponse, status: StatusCode, code: &str, field: &Value) {
    assert_eq!(response.status_code(), status, "{code}");
    assert_eq!(response.header("cache-control"), "no-store", "{code}");
    let body = response.json::<Value>();
    assert_eq!(body["error"], json!(code), "{body}");
    assert_eq!(&body["field"], field, "{body}");
    assert_eq!(body["params"], json!({}), "{body}");
    assert!(
        body["description"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "{body}"
    );
}

#[tokio::test]
async fn edit_labels_or_edit_posts_may_read_labels_and_nobody_else() {
    // ADR 0007 §4: labels are read with `edit labels`, or with `edit posts`
    // for the post form's picker.
    with_server(|server, ctx| async move {
        label(&ctx, 1, "Hírek", "#123456").await;
        let allowed = [
            writer(&ctx).await,
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
            // A secretary with `edit posts` as an extra grant: the picker.
            account_with(
                &ctx,
                "bejegyzesek@example.test",
                &[Role::Secretary],
                &[Permission::EditPosts],
                |_| {},
            )
            .await,
        ];
        for user in &allowed {
            let case = user.email.clone().unwrap_or_default();
            assert_page(&get(&server, &ctx, LIST, Some(user)).await, &[1], 1, &case);
            let one = get(&server, &ctx, &format!("{LIST}/1"), Some(user)).await;
            assert_eq!(one.status_code(), StatusCode::OK, "{case}");
        }
        let refused = [
            account_with(&ctx, "titkar@example.test", &[Role::Secretary], &[], |_| {}).await,
            account_with(&ctx, "senki@example.test", &[], &[], |_| {}).await,
            account_with(
                &ctx,
                "szerzok@example.test",
                &[],
                &[Permission::EditAuthors, Permission::PublishPosts],
                |_| {},
            )
            .await,
        ];
        for user in &refused {
            for path in [LIST.to_owned(), format!("{LIST}/1")] {
                // A list the account may not see is a 403, never `[]`
                // (ADR 0007 §5), and a record it may not see is no 404.
                assert_envelope(
                    &get(&server, &ctx, &path, Some(user)).await,
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    &Value::Null,
                );
            }
        }
        // No session at all: 401, also for a bad query or id, which the
        // handler never reads.
        for path in [
            LIST.to_owned(),
            format!("{LIST}/1"),
            format!("{LIST}?perPage=7"),
            format!("{LIST}/abc"),
        ] {
            assert_envelope(
                &get(&server, &ctx, &path, None).await,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                &Value::Null,
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_access_map_offers_the_label_screens_to_edit_labels_only() {
    // ADR 0007 §7: `edit posts` reads labels through the picker, which is
    // no screen, so the sidebar shows „Címkék” only to `edit labels`.
    with_server(|server, ctx| async move {
        let me = "/api/admin/auth/me";
        let writer = writer(&ctx).await;
        let body = get(&server, &ctx, me, Some(&writer)).await.json::<Value>();
        assert_eq!(
            body["access"]["labels"],
            json!(["list", "show", "create", "edit"])
        );
        let picker_only = account_with(
            &ctx,
            "bejegyzesek@example.test",
            &[],
            &[Permission::EditPosts],
            |_| {},
        )
        .await;
        let body = get(&server, &ctx, me, Some(&picker_only))
            .await
            .json::<Value>();
        assert_eq!(body["access"], json!({}));
    })
    .await;
}

#[tokio::test]
async fn the_list_answers_live_labels_in_hungarian_order_with_their_fields() {
    // ADR 0020, the probe's ten words, inserted out of order; a trashed
    // label between them is left out (ADR 0011 §3).
    with_server(|server, ctx| async move {
        let words = [
            "zsák", "alma", "óra", "csiga", "őz", "cukor", "zúgó", "olló", "ábra", "öröm",
        ];
        for (index, word) in words.iter().enumerate() {
            label(
                &ctx,
                i64::try_from(index).expect("small") + 1,
                word,
                "#1E88E5",
            )
            .await;
        }
        label(&ctx, 11, "cs-lomtár", "#000000").await;
        trash(&ctx, 11).await;
        let user = writer(&ctx).await;
        let response = get(&server, &ctx, LIST, Some(&user)).await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.header("content-type"), "application/json");
        let body = response.json::<Value>();
        let names: Vec<&str> = body["data"]
            .as_array()
            .expect("data")
            .iter()
            .map(|row| row["name"].as_str().expect("a name"))
            .collect();
        assert_eq!(
            names,
            [
                "ábra", "alma", "cukor", "csiga", "olló", "óra", "öröm", "őz", "zúgó", "zsák"
            ]
        );
        assert_eq!(body["total"], json!(10));
        // Each row is exactly the label DTO.
        assert_eq!(
            body["data"][0],
            json!({"id": 9, "name": "ábra", "color": "#1E88E5"})
        );
    })
    .await;
}

#[tokio::test]
async fn the_list_pages_sorts_and_filters_as_the_contract_says() {
    with_server(|server, ctx| async move {
        for id in 1..=30 {
            label(
                &ctx,
                id,
                &format!("Címke {id:02}"),
                if id % 2 == 0 { "#BBBBBB" } else { "#AAAAAA" },
            )
            .await;
        }
        let user = writer(&ctx).await;
        let all: Vec<i64> = (1..=30).collect();
        let list = |query: &str| {
            let path = format!("{LIST}?{query}");
            let server = &server;
            let ctx = &ctx;
            let user = &user;
            async move { get(server, ctx, &path, Some(user)).await }
        };
        // SHR-02: 25 rows by default, then 10, 25, 50 and 100.
        assert_page(&list("").await, &all[..25], 30, "default");
        assert_page(&list("page=2").await, &all[25..], 30, "page 2");
        assert_page(
            &list("perPage=10&page=3").await,
            &all[20..],
            30,
            "page 3 of 10",
        );
        assert_page(&list("perPage=50").await, &all, 30, "50");
        assert_page(&list("perPage=100").await, &all, 30, "100");
        assert_page(&list("page=9").await, &[], 30, "past the end");
        // SHR-03: a header sort, then id descending.
        let odd_desc: Vec<i64> = (1..=30).rev().filter(|id| id % 2 == 1).collect();
        let even_desc: Vec<i64> = (1..=30).rev().filter(|id| id % 2 == 0).collect();
        let by_color: Vec<i64> = odd_desc.iter().chain(&even_desc).copied().collect();
        assert_page(
            &list("sort=color&order=ASC&perPage=50").await,
            &by_color,
            30,
            "color",
        );
        let reversed: Vec<i64> = all.iter().rev().copied().collect();
        assert_page(
            &list("sort=id&order=DESC&perPage=50").await,
            &reversed,
            30,
            "id desc",
        );
        assert_page(
            &list("sort=name&order=DESC&perPage=50").await,
            &reversed,
            30,
            "name desc",
        );
        // The picker's getMany: the named live labels, in the list's order.
        trash(&ctx, 7).await;
        assert_page(
            &list("id=7&id=12&id=3&id=12&id=999").await,
            &[3, 12],
            2,
            "ids",
        );
    })
    .await;
}

/// The queries that the admin's generic handlers write
/// (`admin/src/data-provider/list-handlers.ts`), byte for byte as
/// `URLSearchParams` writes them: the label list's first page, a page of
/// another size, a header sort, and the `getMany` of the post form's
/// picker, 100 to a page with the ids repeated and no sort.
#[tokio::test]
async fn the_queries_the_admin_writes_are_answered_as_the_screen_expects() {
    with_server(|server, ctx| async move {
        for id in 1..=30 {
            label(
                &ctx,
                id,
                &format!("Címke {id:02}"),
                if id % 3 == 0 { "#AAAAAA" } else { "#BBBBBB" },
            )
            .await;
        }
        let user = writer(&ctx).await;
        let all: Vec<i64> = (1..=30).collect();
        let list = |query: &str| {
            let path = format!("{LIST}?{query}");
            let server = &server;
            let ctx = &ctx;
            let user = &user;
            async move { get(server, ctx, &path, Some(user)).await }
        };
        assert_page(
            &list("page=1&perPage=25&sort=name&order=ASC").await,
            &all[..25],
            30,
            "the screen's first page",
        );
        assert_page(
            &list("page=2&perPage=10&sort=name&order=ASC").await,
            &all[10..20],
            30,
            "page 2 of 10",
        );
        // Ties on the colour go to the higher id, in both directions.
        let threes_desc: Vec<i64> = (1..=30).rev().filter(|id| id % 3 == 0).collect();
        let others_desc: Vec<i64> = (1..=30).rev().filter(|id| id % 3 != 0).collect();
        let ascending: Vec<i64> = threes_desc.iter().chain(&others_desc).copied().collect();
        let descending: Vec<i64> = others_desc.iter().chain(&threes_desc).copied().collect();
        assert_page(
            &list("page=1&perPage=100&sort=color&order=ASC").await,
            &ascending,
            30,
            "colour ascending",
        );
        assert_page(
            &list("page=1&perPage=100&sort=color&order=DESC").await,
            &descending,
            30,
            "colour descending",
        );
        // getMany: the named labels, each once, in the list's default order.
        assert_page(
            &list("perPage=100&id=12&id=3&id=29").await,
            &[3, 12, 29],
            3,
            "getMany",
        );
    })
    .await;
}

#[tokio::test]
async fn every_refused_list_query_is_a_422_that_names_its_parameter() {
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let too_many: Vec<String> = (1..=101).map(|id| format!("id={id}")).collect();
        let too_many = too_many.join("&");
        for (query, code, field) in [
            ("page=0", "invalid_page", "page"),
            ("page=x", "invalid_page", "page"),
            ("page=1&page=2", "invalid_page", "page"),
            ("perPage=20", "invalid_per_page", "perPage"),
            ("perPage=-1", "invalid_per_page", "perPage"),
            ("perPage=all", "invalid_per_page", "perPage"),
            ("sort=created_at", "invalid_sort", "sort"),
            ("sort=Name", "invalid_sort", "sort"),
            ("order=asc", "invalid_order", "order"),
            ("q=alma", "unknown_filter", "q"),
            ("filter=%7B%7D", "unknown_filter", "filter"),
            ("id=abc", "invalid_filter", "id"),
            ("id=0", "invalid_filter", "id"),
            (too_many.as_str(), "invalid_filter", "id"),
            // The first failing check wins, whatever the order.
            ("q=x&sort=x&page=0", "invalid_page", "page"),
        ] {
            let response = get(&server, &ctx, &format!("{LIST}?{query}"), Some(&user)).await;
            assert_envelope(
                &response,
                StatusCode::UNPROCESSABLE_ENTITY,
                code,
                &json!(field),
            );
        }
        // The description says what the parameter takes.
        let response = get(&server, &ctx, &format!("{LIST}?perPage=20"), Some(&user)).await;
        assert_eq!(
            response.json::<Value>()["description"],
            json!("perPage must appear at most once, as 10, 25, 50 or 100")
        );
    })
    .await;
}

#[tokio::test]
async fn the_record_route_answers_a_live_label_and_404_for_anything_else() {
    with_server(|server, ctx| async move {
        label(&ctx, 5, "Ünnepség", "#a1b2c3").await;
        label(&ctx, 6, "Lomtár", "#000000").await;
        trash(&ctx, 6).await;
        let user = writer(&ctx).await;
        let response = get(&server, &ctx, &format!("{LIST}/5"), Some(&user)).await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.header("cache-control"), "no-store");
        assert_eq!(
            response.json::<Value>(),
            json!({"id": 5, "name": "Ünnepség", "color": "#a1b2c3"})
        );
        // Leading zeros name the same id, as in the public API.
        assert_eq!(
            get(&server, &ctx, &format!("{LIST}/005"), Some(&user))
                .await
                .json::<Value>()["id"],
            json!(5)
        );
        for id in [
            "6",
            "7",
            "0",
            "-5",
            "abc",
            "5.0",
            "9007199254740992",
            "99999999999999999999999",
            "%FF",
            "%2B5",
        ] {
            assert_envelope(
                &get(&server, &ctx, &format!("{LIST}/{id}"), Some(&user)).await,
                StatusCode::NOT_FOUND,
                "not_found",
                &Value::Null,
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_label_routes_take_only_their_methods() {
    // GET and POST on the list, GET and PUT on the record; delete comes
    // with the trash. axum answers 405 to every other method, with the
    // admin's `no-store`.
    with_server(|server, ctx| async move {
        let user = writer(&ctx).await;
        let token = session_token(&ctx, &user, SessionScope::Admin);
        for (path, methods) in [
            (
                LIST.to_owned(),
                [
                    axum::http::Method::PUT,
                    axum::http::Method::PATCH,
                    axum::http::Method::DELETE,
                ],
            ),
            (
                format!("{LIST}/1"),
                [
                    axum::http::Method::POST,
                    axum::http::Method::PATCH,
                    axum::http::Method::DELETE,
                ],
            ),
        ] {
            for method in methods {
                let response = server
                    .method(method.clone(), &path)
                    .add_header(COOKIE, format!("{SESSION_COOKIE_NAME}={token}"))
                    .await;
                assert_eq!(
                    response.status_code(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {path}"
                );
                assert_eq!(response.header("cache-control"), "no-store");
            }
        }
    })
    .await;
}
