//! The OpenAPI document of `/api/v1` (ADR 0005 §4, public-api.md §7.18):
//! the committed snapshot (a snapshot change is an API change), the served
//! text and headers, the licence and source link (ADR 0012 §4), and route
//! coverage in both directions against Loco's route table.
//!
//! The CI `Contract` job runs every test whose path contains `openapi` with
//! `INSTA_UPDATE=no`, so a document that differs from the snapshot fails
//! there as well as in the `Test` job.

use axum::http::{Method, StatusCode};
use backend3::{
    app::App,
    controllers::v1::openapi::{DOCUMENT_ROUTE, LICENSE, SOURCE_REPOSITORY, document_json},
    domain::{
        api_path::{ApiFamily, api_family},
        route_coverage::{Operation, coverage_gaps, documented_operations},
    },
};
use loco_rs::app::Hooks;
use serde_json::{Value, json};

use crate::support::with_server;

/// Asserts that `document`, the document text from the builder or from a
/// response, equals the committed `snapshots/openapi_document.snap`.
///
/// Both snapshot tests call this, so they share one file whose header does
/// not depend on which test wrote it. insta prefixes a snapshot with the
/// module path by default, which here starts with the keyword crate name of
/// `tests/mod.rs` (`r#mod__...`); the setting keeps the plain name.
fn assert_document_snapshot(document: &str) {
    insta::with_settings!({ prepend_module_to_snapshot => false }, {
        insta::assert_snapshot!("openapi_document", document);
    });
}

#[test]
fn the_openapi_document_matches_the_snapshot() {
    assert_document_snapshot(&document_json().expect("the document serializes"));
}

#[tokio::test]
async fn the_served_openapi_document_equals_the_snapshot() {
    with_server(|request, _ctx| async move {
        // The published URL (public-api.md §7.18), written out rather than
        // read from `DOCUMENT_ROUTE`, so that moving the document fails here.
        let response = request.get("/api/v1/openapi.json").await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.header("content-type"), "application/json");
        assert_eq!(response.header("cache-control"), "no-cache");
        let document = response.text();
        assert_eq!(document, document_json().expect("the document serializes"));
        assert_document_snapshot(&document);
    })
    .await;
}

#[tokio::test]
async fn the_openapi_route_answers_get_and_head_only() {
    with_server(|request, _ctx| async move {
        // public-api.md DEC-10: the trailing-slash form is served the same.
        let slash = request.get(&format!("{DOCUMENT_ROUTE}/")).await;
        assert_eq!(slash.status_code(), StatusCode::OK);
        assert_eq!(slash.text(), document_json().expect("serializes"));
        // public-api.md §2.1: HEAD is answered as axum answers it for GET.
        let head = request.method(Method::HEAD, DOCUMENT_ROUTE).await;
        assert_eq!(head.status_code(), StatusCode::OK);
        assert_eq!(head.header("content-type"), "application/json");
        assert!(head.as_bytes().is_empty());
        // public-api.md §8: other methods get axum's 405, whose body clients
        // must not rely on.
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            let response = request.method(method.clone(), DOCUMENT_ROUTE).await;
            assert_eq!(response.status_code(), StatusCode::METHOD_NOT_ALLOWED);
            let allow = response.header("allow");
            let allow = allow.to_str().expect("ASCII");
            assert!(allow.split(',').any(|m| m.trim() == "GET"), "{method}");
        }
    })
    .await;
}

#[tokio::test]
async fn the_openapi_document_names_the_licence_and_links_to_the_source() {
    with_server(|request, _ctx| async move {
        let document: Value = request.get(DOCUMENT_ROUTE).await.json();
        assert_eq!(document["openapi"], "3.1.0");
        assert_eq!(LICENSE, "AGPL-3.0-only");
        // OpenAPI 3.1 allows `identifier` or `url`, not both.
        assert_eq!(
            document["info"]["license"],
            json!({ "name": LICENSE, "identifier": LICENSE })
        );
        // AGPL-3.0 §13: the source offer for network users.
        assert_eq!(
            SOURCE_REPOSITORY,
            "https://github.com/Verseghy/website_backend3"
        );
        let description = document["info"]["description"].as_str().expect("text");
        assert!(description.contains(SOURCE_REPOSITORY), "{description}");
        // The paths carry /api/v1, so no `servers` entry differs per host.
        assert!(document.get("servers").is_none());
    })
    .await;
}

#[tokio::test]
async fn every_public_route_is_documented_and_every_operation_is_routed() {
    with_server(|request, ctx| async move {
        // The table `cargo loco routes` prints (ADR 0005 §4).
        let table = App::routes(&ctx).collect();
        // `Routes::add` reads a route's methods from the `Debug` text of its
        // axum `MethodRouter` and finds none for `any(...)` or a shape it
        // does not parse. Such a route would escape the comparison below.
        let unread: Vec<&str> = table
            .iter()
            .filter(|route| api_family(&route.uri) == Some(ApiFamily::V1))
            .filter(|route| route.actions.is_empty())
            .map(|route| route.uri.as_str())
            .collect();
        assert!(
            unread.is_empty(),
            "Loco records no method for {unread:?}: register each with a verb \
             method of `Routes`, such as `Routes::get`."
        );
        let routes: Vec<Operation> = table
            .iter()
            .flat_map(|route| {
                let path = &route.uri;
                route
                    .actions
                    .iter()
                    .map(|method| Operation::new(method.as_str(), path))
            })
            .collect();
        let document_route = Operation::new("GET", DOCUMENT_ROUTE);
        // Without this the test would pass on an empty route table.
        assert!(routes.contains(&document_route), "{routes:?}");

        let document: Value = request.get(DOCUMENT_ROUTE).await.json();
        let documented = documented_operations(&document).expect("an OpenAPI document");
        let gaps = coverage_gaps(&routes, &documented, &[document_route]);
        let list = |operations: &[Operation]| {
            operations
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        assert!(
            gaps.is_empty(),
            "routes without an operation: [{}]; operations without a route: [{}]. Put \
             #[utoipa::path] on the handler and list it in `PublicApi` \
             (src/controllers/v1/openapi.rs), or remove the stale operation.",
            list(&gaps.undocumented_routes),
            list(&gaps.unrouted_operations)
        );
    })
    .await;
}
