//! The `/api/v1` router (ADR 0014, public-api.md §2.1 and §8): every public
//! response carries `Access-Control-Allow-Origin: *` and never allows
//! credentials, errors included; no other path sends a CORS header; the
//! public routes answer `GET` and `HEAD`, a CORS preflight on `OPTIONS`
//! (public-api.md §2.1, open item O-10, settled), and `405` with `Allow` for
//! every other method. Also Loco's health routes, which parity PUB-13 keeps.
//!
//! The route tests take every `/api/v1` route from Loco's route table
//! (`AppRoutes::collect()`, what `cargo loco routes` prints), and every other
//! route of the table for the absence of CORS headers, so a route that a
//! later slice adds is checked without changing this file.

use std::collections::BTreeSet;

use axum::http::{
    HeaderName, Method, StatusCode,
    header::{ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, ALLOW, ORIGIN, VARY},
};
use backend3::{
    app::App,
    controllers::v1::openapi::DOCUMENT_ROUTE,
    domain::api_path::{ApiFamily, api_family},
};
use loco_rs::app::{AppContext, Hooks};
use serde_json::{Value, json};

use crate::support::{
    FIXTURE_ASSET, SITE_ORIGIN, assert_no_cors, assert_public_cors, assert_public_not_found,
    cors_headers, header_text, public_get, public_paths, sample_path, with_server,
};

/// Every path of Loco's route table outside `/api/v1` (the admin API, the
/// health routes and the site routes), with each `{parameter}` segment
/// replaced by `1`.
///
/// These routes are added beside `controllers::v1::routes()` in
/// `App::routes`, so they show that its CORS layer stays on the public
/// routes: the fallback paths of the admin API alone would not, because the
/// fallback runs outside every route group.
fn other_paths(ctx: &AppContext) -> Vec<String> {
    let paths: BTreeSet<String> = App::routes(ctx)
        .collect()
        .into_iter()
        .filter(|route| api_family(&route.uri) != Some(ApiFamily::V1))
        .map(|route| sample_path(&route.uri))
        .collect();
    // Without this the admin route group would go unchecked.
    assert!(
        paths
            .iter()
            .any(|path| api_family(path) == Some(ApiFamily::Admin)),
        "no /api/admin route in {paths:?}"
    );
    paths.into_iter().collect()
}

/// The methods an `Allow` header lists.
fn allowed(response: &axum_test::TestResponse) -> BTreeSet<String> {
    header_text(response, &ALLOW)
        .expect("an Allow header")
        .split(',')
        .map(|method| method.trim().to_owned())
        .collect()
}

#[test]
fn path_parameters_get_a_sample_value() {
    assert_eq!(sample_path("/api/v1/posts/{id}"), "/api/v1/posts/1");
    assert_eq!(
        sample_path("/api/v1/canteen/{isoYear}/{isoWeek}"),
        "/api/v1/canteen/1/1"
    );
    assert_eq!(sample_path(DOCUMENT_ROUTE), DOCUMENT_ROUTE);
}

#[tokio::test]
async fn every_public_route_sends_the_cors_header_on_get_and_head() {
    with_server(|request, ctx| async move {
        for path in public_paths(&ctx) {
            let get = public_get(&request, &path).await;
            assert_ne!(get.status_code(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
            assert_public_cors(&get, &format!("GET {path}"));

            // The SSR pod and curl send no Origin header; they get the same
            // header (ADR 0014).
            let without_origin = request.get(&path).await;
            assert_eq!(without_origin.status_code(), get.status_code(), "{path}");
            assert_public_cors(&without_origin, &format!("GET {path} without Origin"));

            let head = request
                .method(Method::HEAD, &path)
                .add_header(ORIGIN, SITE_ORIGIN)
                .await;
            assert_eq!(head.status_code(), get.status_code(), "HEAD {path}");
            assert_eq!(head.as_bytes().as_ref(), b"", "HEAD {path}");
            assert_public_cors(&head, &format!("HEAD {path}"));
        }
    })
    .await;
}

#[tokio::test]
async fn other_methods_get_405_with_allow_and_cors() {
    with_server(|request, ctx| async move {
        for path in public_paths(&ctx) {
            for method in [
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::PATCH,
                Method::TRACE,
            ] {
                let context = format!("{method} {path}");
                let response = request
                    .method(method, &path)
                    .add_header(ORIGIN, SITE_ORIGIN)
                    .await;
                assert_eq!(
                    response.status_code(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{context}"
                );
                assert_eq!(
                    allowed(&response),
                    BTreeSet::from(["GET".to_owned(), "HEAD".to_owned()]),
                    "{context}"
                );
                // public-api.md §8: axum's empty body, which clients must
                // not rely on.
                assert_eq!(response.as_bytes().as_ref(), b"", "{context}");
                assert_public_cors(&response, &context);
            }
        }
    })
    .await;
}

#[tokio::test]
async fn options_answers_a_preflight_that_grants_only_get_and_head() {
    // public-api.md §2.1 and open item O-10, settled on 2026-10-25: OPTIONS
    // gets the CorsLayer's preflight answer of ADR 0014, never the 405 of the
    // other methods.
    let private_network = HeaderName::from_static("access-control-request-private-network");
    with_server(|request, ctx| async move {
        for path in public_paths(&ctx) {
            for (case, headers) in [
                (
                    "a preflight for GET",
                    vec![
                        (ORIGIN, SITE_ORIGIN),
                        (ACCESS_CONTROL_REQUEST_METHOD, "GET"),
                    ],
                ),
                (
                    "a preflight for DELETE with Authorization",
                    vec![
                        (ORIGIN, SITE_ORIGIN),
                        (ACCESS_CONTROL_REQUEST_METHOD, "DELETE"),
                        (ACCESS_CONTROL_REQUEST_HEADERS, "authorization"),
                    ],
                ),
                (
                    "a private-network preflight",
                    vec![
                        (ORIGIN, SITE_ORIGIN),
                        (ACCESS_CONTROL_REQUEST_METHOD, "GET"),
                        (private_network.clone(), "true"),
                    ],
                ),
                // Not a preflight (`curl -X OPTIONS`, no Origin): the
                // CorsLayer answers every OPTIONS request the same way, so the
                // settled answer holds for it too.
                ("a plain OPTIONS", vec![]),
            ] {
                let context = format!("{case} on {path}");
                let mut options = request.method(Method::OPTIONS, &path);
                for (name, value) in headers {
                    options = options.add_header(name, value);
                }
                let response = options.await;
                assert_eq!(response.status_code(), StatusCode::OK, "{context}");
                assert_eq!(response.as_bytes().as_ref(), b"", "{context}");
                // Exactly these, each once: no allowed request header, no
                // credentials, no private-network access and no max-age, so a
                // browser refuses the DELETE and the Authorization header.
                let mut headers = cors_headers(&response);
                headers.sort();
                assert_eq!(
                    headers,
                    [
                        (
                            "access-control-allow-methods".to_owned(),
                            "GET,HEAD".to_owned()
                        ),
                        ("access-control-allow-origin".to_owned(), "*".to_owned()),
                    ],
                    "{context}"
                );
                assert_eq!(header_text(&response, &VARY), None, "{context}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn public_errors_carry_the_cors_header() {
    with_server(|request, _ctx| async move {
        for path in [
            "/api/v1",
            "/api/v1/nope",
            "/api/v1/openapi.json/nope",
            "/api/v1/posts/1",
        ] {
            let response = public_get(&request, path).await;
            assert_public_not_found(&response, path);
            assert_public_cors(&response, path);
        }
        // An unknown path is a 404 for every method, never a 405.
        let response = request
            .method(Method::POST, "/api/v1/nope")
            .add_header(ORIGIN, SITE_ORIGIN)
            .await;
        assert_public_not_found(&response, "POST /api/v1/nope");
        assert_public_cors(&response, "POST /api/v1/nope");

        // OPTIONS included (public-api.md §2.1, open item O-10): the CorsLayer
        // is on the routes, not on the router fallback, so a preflight to an
        // unknown path gets the JSON 404 with only the allow-origin header,
        // never the preflight answer of a route.
        for path in ["/api/v1", "/api/v1/nope"] {
            let context = format!("a preflight for GET on {path}");
            let response = request
                .method(Method::OPTIONS, path)
                .add_header(ORIGIN, SITE_ORIGIN)
                .add_header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .await;
            assert_public_not_found(&response, &context);
            assert_public_cors(&response, &context);
        }
    })
    .await;
}

#[tokio::test]
async fn routes_outside_the_public_api_never_send_cors_headers() {
    with_server(|request, ctx| async move {
        // Every method a route group answers or refuses: the route's own
        // answer, axum's 405 and a preflight.
        for path in other_paths(&ctx) {
            for method in [Method::GET, Method::POST, Method::OPTIONS] {
                let response = request
                    .method(method.clone(), &path)
                    .add_header(ORIGIN, SITE_ORIGIN)
                    .add_header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .await;
                assert_no_cors(&response, &format!("{method} {path}"));
            }
        }
    })
    .await;
}

#[tokio::test]
async fn paths_outside_the_public_api_never_send_cors_headers() {
    with_server(|request, _ctx| async move {
        let asset = format!("/admin/{FIXTURE_ASSET}");
        for (method, path) in [
            (Method::GET, "/admin"),
            (Method::GET, "/admin/posts/12/show"),
            (Method::GET, asset.as_str()),
            (Method::GET, "/admin/assets/missing.js"),
            (Method::POST, "/admin"),
            (Method::OPTIONS, "/admin"),
            (Method::GET, "/api/admin"),
            (Method::GET, "/api/admin/posts"),
            (Method::POST, "/api/admin/posts"),
            (Method::OPTIONS, "/api/admin/posts"),
            (Method::GET, "/"),
            (Method::GET, "/robots.txt"),
            (Method::GET, "/nope"),
        ] {
            let response = request
                .method(method.clone(), path)
                .add_header(ORIGIN, SITE_ORIGIN)
                .add_header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .await;
            assert_no_cors(&response, &format!("{method} {path}"));
        }
    })
    .await;
}

#[tokio::test]
async fn health_routes_answer_get_and_head_without_cors() {
    with_server(|request, _ctx| async move {
        // Loco's health routes (ADR 0014, ADR 0023), which parity PUB-13
        // keeps beside the OpenAPI document. `/_readiness` also pings the
        // database.
        for path in ["/_ping", "/_health", "/_readiness"] {
            let get = public_get(&request, path).await;
            assert_eq!(get.status_code(), StatusCode::OK, "GET {path}");
            assert_eq!(get.json::<Value>(), json!({ "ok": true }), "GET {path}");
            assert_no_cors(&get, &format!("GET {path}"));

            let head = request
                .method(Method::HEAD, path)
                .add_header(ORIGIN, SITE_ORIGIN)
                .await;
            assert_eq!(head.status_code(), StatusCode::OK, "HEAD {path}");
            assert_eq!(head.as_bytes().as_ref(), b"", "HEAD {path}");
            assert_no_cors(&head, &format!("HEAD {path}"));
        }
    })
    .await;
}
