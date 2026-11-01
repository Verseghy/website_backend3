//! The OpenAPI document of `/api/v1` (ADR 0005 §4, public-api.md §7.18):
//! the committed snapshot (a snapshot change is an API change), the served
//! text and headers, the licence and source link (ADR 0012 §4), the public
//! `Error` schema (public-api.md §6.15) and route coverage in both
//! directions against Loco's route table.
//!
//! The CI `Contract` job runs every test whose path contains `openapi` with
//! `INSTA_UPDATE=no`, so a document that differs from the snapshot fails
//! there as well as in the `Test` job.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use backend3::{
    app::App,
    controllers::v1::openapi::{
        CACHE_CONTROL_HEADER, DOCUMENT_ROUTE, ETAG_HEADER, IF_NONE_MATCH_PARAMETER, LICENSE,
        NOT_MODIFIED_RESPONSE, SOURCE_REPOSITORY, document_json,
    },
    domain::{
        api_path::{ApiFamily, api_family},
        etag::weak_etag,
        route_coverage::{Operation, coverage_gaps, documented_operations},
    },
    dtos::v1::error::ErrorCode,
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
        // Through the /api/v1 layers (ADR 0014), so API explorers on other
        // origins can read it.
        assert_eq!(response.header("access-control-allow-origin"), "*");
        let document = response.text();
        assert_eq!(document, document_json().expect("the document serializes"));
        // public-api.md §9: the weak tag of the served text, so a client that
        // revalidates gets a 304 until the document changes.
        assert_eq!(
            response.header("etag").to_str().expect("ASCII"),
            weak_etag(document.as_bytes())
        );
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
async fn the_openapi_document_declares_the_public_error_body() {
    with_server(|request, _ctx| async move {
        let document: Value = request.get(DOCUMENT_ROUTE).await.json();
        let schemas = &document["components"]["schemas"];
        // public-api.md §6.15 and the codes of §8, in the order of the table.
        let codes: Vec<Value> = ErrorCode::ALL
            .iter()
            .map(|code| json!(code.as_str()))
            .collect();
        assert_eq!(schemas["ErrorCode"]["type"], "string");
        assert_eq!(schemas["ErrorCode"]["enum"], Value::Array(codes));
        let error = &schemas["Error"];
        assert_eq!(error["required"], json!(["error", "description"]));
        assert_eq!(
            error["properties"]["error"]["$ref"],
            "#/components/schemas/ErrorCode"
        );
        assert_eq!(error["properties"]["description"]["type"], "string");

        // A served error has exactly the declared fields and a declared code.
        let body: Value = request.get("/api/v1/nope").await.json();
        let declared: BTreeSet<&String> = error["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .collect();
        let served: BTreeSet<&String> = body.as_object().expect("an object").keys().collect();
        assert_eq!(served, declared);
        assert!(
            schemas["ErrorCode"]["enum"]
                .as_array()
                .expect("an enum")
                .contains(&body["error"]),
            "{body}"
        );
        assert!(body["description"].is_string(), "{body}");
    })
    .await;
}

#[tokio::test]
async fn the_openapi_document_declares_the_caching_headers() {
    with_server(|request, _ctx| async move {
        let response = request.get(DOCUMENT_ROUTE).await;
        let served_tag = response.header("etag").to_str().expect("ASCII").to_owned();
        let document: Value = response.json();
        let components = &document["components"];
        // public-api.md §9: the two headers of every 200, with the values
        // backend3 sends.
        let etag = &components["headers"][ETAG_HEADER];
        assert_eq!(etag["schema"]["type"], "string");
        let pattern = etag["schema"]["pattern"].as_str().expect("a pattern");
        assert_eq!(pattern, r#"^W/"[!#-~]+"$"#);
        assert_eq!(etag["schema"]["examples"], json!([weak_etag(b"[]")]));
        // The pattern accepts the tag the document itself was served with:
        // `W/"`, visible ASCII except `"`, and `"`.
        let opaque = served_tag
            .strip_prefix("W/\"")
            .and_then(|rest| rest.strip_suffix('"'))
            .expect("a weak tag");
        assert!(
            !opaque.is_empty()
                && opaque
                    .bytes()
                    .all(|b| b == b'!' || (b'#'..=b'~').contains(&b)),
            "{served_tag}"
        );
        assert_eq!(
            components["headers"][CACHE_CONTROL_HEADER]["schema"]["enum"],
            json!(["no-cache", "no-store"])
        );
        // The request header that revalidates.
        assert_eq!(
            components["parameters"][IF_NONE_MATCH_PARAMETER],
            json!({
                "name": "If-None-Match",
                "in": "header",
                "description": components["parameters"][IF_NONE_MATCH_PARAMETER]["description"],
                "required": false,
                "schema": { "type": "string" }
            })
        );
        // The 304, which has no content and refers to both headers.
        let not_modified = &components["responses"][NOT_MODIFIED_RESPONSE];
        assert!(not_modified.get("content").is_none(), "{not_modified}");
        assert_eq!(
            not_modified["headers"],
            json!({
                "Cache-Control": { "$ref": "#/components/headers/Cache-Control" },
                "ETag": { "$ref": "#/components/headers/ETag" }
            })
        );
        for description in [
            &etag["description"],
            &components["headers"][CACHE_CONTROL_HEADER]["description"],
            &components["parameters"][IF_NONE_MATCH_PARAMETER]["description"],
            &not_modified["description"],
        ] {
            assert!(description.as_str().is_some_and(|text| !text.is_empty()));
        }
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
