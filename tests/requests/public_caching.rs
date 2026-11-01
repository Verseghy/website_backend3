//! The caching rules of `/api/v1` (public-api.md §8, §9, §16.3) through the
//! whole middleware stack: every `200` carries `Cache-Control: no-cache` and
//! the weak `ETag` of its body; `If-None-Match` with that tag gets `304 Not
//! Modified` with an empty body, the same validator headers and the CORS
//! header; any other validator gets the full answer; every error carries
//! `no-store` and no `ETag`, also the ones the framework builds.
//!
//! The route tests take every `/api/v1` route from Loco's route table, so a
//! route that a later slice adds is checked without changing this file. On
//! the empty test database only the routes that answer `200` without data
//! (so far the OpenAPI document) exercise the `200` rules; the per-route
//! fixtures of later slices reach the others.

use axum::http::{
    HeaderName, Method, StatusCode,
    header::{
        ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE,
        IF_NONE_MATCH, LAST_MODIFIED, ORIGIN, VARY,
    },
};
use axum_test::{TestResponse, TestServer};
use backend3::{
    app::App,
    controllers::v1::{caching::NotModifiedCors, openapi::DOCUMENT_ROUTE},
    domain::etag::weak_etag,
};
use loco_rs::boot::list_middlewares;

use crate::support::{
    FIXTURE_ASSET, SITE_ORIGIN, assert_no_cors, assert_public_cors, header_text, public_get,
    public_paths, with_app, with_server,
};

/// Every value of header `name` in `response`, in order.
fn all_values(response: &TestResponse, name: &HeaderName) -> Vec<String> {
    response
        .iter_headers_by_name(name.clone())
        .map(|value| value.to_str().expect("an ASCII header").to_owned())
        .collect()
}

/// Asserts the headers of a cacheable `200` (public-api.md §9) and returns
/// its `ETag`: JSON, one `Cache-Control: no-cache`, one weak `ETag` that is
/// the tag of exactly the body received, and one `Vary: accept-encoding`.
///
/// - `response`: an uncompressed `200` of an `/api/v1` route.
/// - `context`: what was requested, for the failure message.
fn assert_cacheable(response: &TestResponse, context: &str) -> String {
    assert_eq!(response.status_code(), StatusCode::OK, "{context}");
    assert_eq!(
        header_text(response, &CONTENT_TYPE).as_deref(),
        Some("application/json"),
        "{context}"
    );
    assert_eq!(
        header_text(response, &CACHE_CONTROL).as_deref(),
        Some("no-cache"),
        "{context}"
    );
    assert_eq!(
        all_values(response, &VARY),
        ["accept-encoding"],
        "{context}"
    );
    let etag = header_text(response, &ETAG).expect("a 200 carries an ETag");
    assert_eq!(etag, weak_etag(response.as_bytes()), "{context}");
    etag
}

/// Asserts a `304` for a revalidation with `etag` (public-api.md §9,
/// §16.3): an empty body, the same `ETag`, `no-cache`, the `Vary` of the
/// `200`, the CORS header of every `/api/v1` answer, and no representation
/// header.
///
/// - `response`: the answer to a request with `If-None-Match: etag`.
/// - `etag`: the tag the `200` carried.
/// - `context`: what was requested, for the failure message.
fn assert_not_modified(response: &TestResponse, etag: &str, context: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::NOT_MODIFIED,
        "{context}"
    );
    assert_eq!(response.as_bytes().as_ref(), b"", "{context}");
    assert_eq!(
        header_text(response, &ETAG).as_deref(),
        Some(etag),
        "{context}"
    );
    assert_eq!(
        header_text(response, &CACHE_CONTROL).as_deref(),
        Some("no-cache"),
        "{context}"
    );
    assert_eq!(
        all_values(response, &VARY),
        ["accept-encoding"],
        "{context}"
    );
    assert_public_cors(response, context);
    assert_eq!(header_text(response, &CONTENT_TYPE), None, "{context}");
    assert_eq!(header_text(response, &CONTENT_ENCODING), None, "{context}");
    // The secure headers sit outside the `etag` middleware, so the `304`
    // gets them too (ADR 0023 §6).
    assert_eq!(
        header_text(response, &HeaderName::from_static("x-content-type-options")).as_deref(),
        Some("nosniff"),
        "{context}"
    );
}

/// Sends `method path` with `If-None-Match: validator` and an `Origin`, as a
/// browser revalidating a stored answer does.
async fn revalidate(
    server: &TestServer,
    method: Method,
    path: &str,
    validator: &str,
) -> TestResponse {
    server
        .method(method, path)
        .add_header(ORIGIN, SITE_ORIGIN)
        .add_header(IF_NONE_MATCH, validator)
        .await
}

/// The paths of [`public_paths`] whose `GET` answers `200` on the empty
/// database, with their `200`. The document route always does.
async fn public_successes(server: &TestServer, paths: &[String]) -> Vec<(String, TestResponse)> {
    let mut successes = Vec::new();
    for path in paths {
        let response = public_get(server, path).await;
        if response.status_code() == StatusCode::OK {
            successes.push((path.clone(), response));
        }
    }
    assert!(
        successes.iter().any(|(path, _)| path == DOCUMENT_ROUTE),
        "the OpenAPI document answers 200"
    );
    successes
}

#[tokio::test]
async fn every_public_200_is_no_cache_with_the_weak_etag_of_its_body() {
    with_server(|request, ctx| async move {
        for path in public_paths(&ctx) {
            let response = public_get(&request, &path).await;
            let context = format!("GET {path}");
            if response.status_code() == StatusCode::OK {
                assert_cacheable(&response, &context);
                // The same answer gets the same tag: it depends on the body
                // only (public-api.md §9).
                let again = request.get(&path).await;
                assert_eq!(header_text(&again, &ETAG), header_text(&response, &ETAG));
                // HEAD gets the headers of GET, the tag included.
                let head = request.method(Method::HEAD, &path).await;
                assert_eq!(head.status_code(), StatusCode::OK, "HEAD {path}");
                assert_eq!(header_text(&head, &ETAG), header_text(&response, &ETAG));
            } else {
                // Every other answer of an empty database is an error.
                assert!(response.status_code().is_client_error(), "{context}");
                assert_eq!(
                    header_text(&response, &CACHE_CONTROL).as_deref(),
                    Some("no-store"),
                    "{context}"
                );
                assert_eq!(header_text(&response, &ETAG), None, "{context}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn if_none_match_with_the_returned_etag_gets_304_with_an_empty_body() {
    with_server(|request, ctx| async move {
        for (path, response) in public_successes(&request, &public_paths(&ctx)).await {
            let etag = assert_cacheable(&response, &format!("GET {path}"));
            for method in [Method::GET, Method::HEAD] {
                let context = format!("{method} {path} with If-None-Match");
                let revalidated = revalidate(&request, method, &path, &etag).await;
                assert_not_modified(&revalidated, &etag, &context);
            }
            // The SSR pod sends no Origin; it gets the CORS header as well.
            let without_origin = request
                .get(&path)
                .add_header(IF_NONE_MATCH, etag.as_str())
                .await;
            assert_not_modified(
                &without_origin,
                &etag,
                &format!("GET {path} without Origin"),
            );
            // public-api.md DEC-10: the trailing-slash form revalidates alike.
            let slash = revalidate(&request, Method::GET, &format!("{path}/"), &etag).await;
            assert_not_modified(&slash, &etag, &format!("GET {path}/"));
        }
    })
    .await;
}

#[tokio::test]
async fn any_other_validator_gets_the_full_answer() {
    with_server(|request, ctx| async move {
        for (path, response) in public_successes(&request, &public_paths(&ctx)).await {
            let etag = assert_cacheable(&response, &format!("GET {path}"));
            let strong = etag.trim_start_matches("W/").to_owned();
            let list = format!("{}, {etag}", weak_etag(b"another body"));
            // Loco compares the header byte for byte, so a stale tag, the
            // tag without `W/`, a list and `*` all get the full 200, which is
            // never wrong (src/controllers/v1/caching.rs). The published
            // document says only that these *may* get the full answer, so
            // this is a tripwire for a Loco bump, not the contract: if a
            // later Loco answers 304 to the list, to `*` or to the tag
            // without `W/`, update this test and public-api.md §9, and the
            // API stays v1. A stale tag and an empty value always get the
            // full answer.
            for validator in [
                weak_etag(b"another body").as_str(),
                strong.as_str(),
                list.as_str(),
                "*",
                "",
            ] {
                let context = format!("GET {path} with If-None-Match: {validator:?}");
                let full = revalidate(&request, Method::GET, &path, validator).await;
                assert_eq!(assert_cacheable(&full, &context), etag, "{context}");
                assert_eq!(full.as_bytes(), response.as_bytes(), "{context}");
                assert_public_cors(&full, &context);
            }
        }
    })
    .await;
}

#[tokio::test]
async fn compression_keeps_the_etag_and_the_304() {
    with_server(|request, ctx| async move {
        for (path, response) in public_successes(&request, &public_paths(&ctx)).await {
            let etag = assert_cacheable(&response, &format!("GET {path}"));
            if response.as_bytes().len() < 32 {
                // Below the compression middleware's threshold.
                continue;
            }
            // The weak tag is the same for the gzip-compressed body
            // (public-api.md §9, RFC 9110 §8.8.1).
            let gzip = request.get(&path).add_header(ACCEPT_ENCODING, "gzip").await;
            assert_eq!(gzip.status_code(), StatusCode::OK, "{path}");
            assert_eq!(
                header_text(&gzip, &CONTENT_ENCODING).as_deref(),
                Some("gzip"),
                "{path}"
            );
            assert_eq!(header_text(&gzip, &ETAG).as_deref(), Some(etag.as_str()));
            assert_eq!(all_values(&gzip, &VARY), ["accept-encoding"], "{path}");
            let revalidated = request
                .get(&path)
                .add_header(ACCEPT_ENCODING, "gzip")
                .add_header(ORIGIN, SITE_ORIGIN)
                .add_header(IF_NONE_MATCH, etag.as_str())
                .await;
            assert_not_modified(&revalidated, &etag, &format!("GET {path} with gzip"));
        }
    })
    .await;
}

#[tokio::test]
async fn public_errors_carry_no_store_and_no_etag() {
    with_server(|request, ctx| async move {
        let some_tag = weak_etag(b"[]");
        for path in public_paths(&ctx) {
            // axum's 405, which sets no Cache-Control itself, and never a
            // 304, whatever the client sends (public-api.md §8, §9).
            for method in [Method::POST, Method::PUT, Method::DELETE] {
                let context = format!("{method} {path}");
                let response = revalidate(&request, method, &path, &some_tag).await;
                assert_eq!(
                    response.status_code(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{context}"
                );
                assert_eq!(
                    header_text(&response, &CACHE_CONTROL).as_deref(),
                    Some("no-store"),
                    "{context}"
                );
                assert_eq!(header_text(&response, &ETAG), None, "{context}");
                assert_public_cors(&response, &context);
            }
        }
        // The fallback's 404 for an unknown path.
        let response = revalidate(&request, Method::GET, "/api/v1/nope", "*").await;
        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(
            header_text(&response, &CACHE_CONTROL).as_deref(),
            Some("no-store")
        );
        assert_eq!(header_text(&response, &ETAG), None);
        assert_public_cors(&response, "GET /api/v1/nope");
    })
    .await;
}

#[tokio::test]
async fn a_304_outside_the_public_api_gets_no_cors_header() {
    // The admin UI's own revalidation (ADR 0004 §3): the middleware that adds
    // the CORS header to a public 304 leaves every other path alone
    // (ADR 0014).
    with_server(|request, _ctx| async move {
        let asset = format!("/admin/{FIXTURE_ASSET}");
        let response = request.get(&asset).await;
        let last_modified = header_text(&response, &LAST_MODIFIED).expect("Last-Modified");
        let revalidated = request
            .get(&asset)
            .add_header(ORIGIN, SITE_ORIGIN)
            .add_header(IF_MODIFIED_SINCE, last_modified.as_str())
            .await;
        assert_eq!(revalidated.status_code(), StatusCode::NOT_MODIFIED);
        assert_no_cors(&revalidated, &format!("GET {asset} with If-Modified-Since"));
    })
    .await;
}

#[tokio::test]
async fn the_cors_middleware_sits_right_outside_etag() {
    with_app(|_router, ctx| async move {
        // `cargo loco middleware` lists the stack in the order Loco applies
        // it, innermost first.
        let stack = list_middlewares::<App>(&ctx);
        let names: Vec<&str> = stack.iter().map(|layer| layer.id.as_str()).collect();
        let etag = names
            .iter()
            .position(|name| *name == "etag")
            .expect("Loco's etag middleware");
        assert_eq!(
            names.get(etag + 1),
            Some(&NotModifiedCors::NAME),
            "{names:?}"
        );
        // Explicitly on in config/test.yaml, as in every environment.
        assert!(stack[etag].enabled, "{names:?}");
        assert!(stack[etag + 1].enabled, "{names:?}");
        // The compression and secure-headers middlewares stay outside both.
        for outer in ["compression", "secure_headers"] {
            let place = names.iter().position(|name| *name == outer).expect(outer);
            assert!(place > etag + 1, "{outer}: {names:?}");
        }
    })
    .await;
}
