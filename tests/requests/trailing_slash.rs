//! A trailing slash is trimmed before routing, so both spellings of a path
//! are served the same, without a redirect (public-api.md DEC-10, open item
//! O-2), and the admin UI is served exactly as before.
//!
//! `with_server` wraps the router the way `App::serve` does. Without the
//! trim, the slash spellings of Loco's routes (`/robots.txt/`, `/_ping/`)
//! would miss them and fall through to the fallback; the admin cases pin
//! that the trim changes nothing under `/admin`.

use axum::http::StatusCode;
use serde_json::Value;

use crate::support::{fixture, with_server};

#[tokio::test]
async fn a_route_with_a_trailing_slash_is_the_route() {
    with_server(|request, _ctx| async move {
        for path in ["/robots.txt/", "/robots.txt//"] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{path}");
            assert_eq!(response.text(), "User-agent: *\nDisallow: /\n", "{path}");
        }
        let ping = request.get("/_ping/").await;
        assert_eq!(ping.status_code(), StatusCode::OK);
        assert_eq!(ping.json::<Value>(), serde_json::json!({ "ok": true }));
    })
    .await;
}

#[tokio::test]
async fn the_public_api_answers_both_spellings_alike() {
    with_server(|request, _ctx| async move {
        // No /api/v1 route exists yet, so both spellings reach the JSON 404;
        // the slices' `/api/v1/posts` will answer `/api/v1/posts/` the same.
        let plain = request.get("/api/v1/posts").await;
        let slashed = request.get("/api/v1/posts/").await;
        assert_eq!(slashed.status_code(), plain.status_code());
        assert_eq!(slashed.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(slashed.json::<Value>(), plain.json::<Value>());
        assert_eq!(slashed.header("access-control-allow-origin"), "*");
        assert!(slashed.maybe_header("location").is_none(), "no redirect");
    })
    .await;
}

#[tokio::test]
async fn the_admin_is_served_as_before() {
    with_server(|request, _ctx| async move {
        for path in ["/admin", "/admin/", "/admin/posts/12/show/"] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{path}");
            assert_eq!(
                response.as_bytes().as_ref(),
                fixture("index.html"),
                "{path}"
            );
            assert_eq!(response.header("cache-control"), "no-cache", "{path}");
            assert!(response.maybe_header("location").is_none(), "{path}");
        }
        // A file spelt with a trailing slash stays the file, never index.html
        // (the admin classifier ignores empty segments, so this holds with or
        // without the trim).
        let favicon = request.get("/admin/favicon.svg/").await;
        assert_eq!(favicon.status_code(), StatusCode::OK);
        assert_eq!(favicon.as_bytes().as_ref(), fixture("favicon.svg"));
        assert_eq!(favicon.header("content-type"), "image/svg+xml");
    })
    .await;
}
