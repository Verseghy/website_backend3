//! Every row of the `/admin` routing table of ADR 0004 §3, against the
//! fixture in `tests/fixtures/admin-dist/`, plus HEAD, 405, compression and
//! path traversal.

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use tower::ServiceExt;

use crate::support::{FIXTURE_ASSET, fixture, with_app, with_server};

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

fn header(response: &axum_test::TestResponse, name: &str) -> Option<String> {
    response
        .maybe_header(name)
        .map(|value| value.to_str().expect("ASCII header").to_owned())
}

#[tokio::test]
async fn admin_root_serves_index_html() {
    with_server(|request, _ctx| async move {
        for path in ["/admin", "/admin/"] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{path}");
            assert_eq!(
                response.as_bytes().as_ref(),
                fixture("index.html"),
                "{path}"
            );
            assert_eq!(
                header(&response, "cache-control").as_deref(),
                Some("no-cache"),
                "{path}"
            );
            assert!(
                header(&response, "content-type")
                    .is_some_and(|value| value.starts_with("text/html")),
                "{path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn head_requests_get_headers_without_a_body() {
    with_server(|request, _ctx| async move {
        let asset = format!("/admin/{FIXTURE_ASSET}");
        for (path, cache) in [
            ("/admin", "no-cache"),
            ("/admin/posts/12/show", "no-cache"),
            (asset.as_str(), IMMUTABLE),
        ] {
            let response = request.method(Method::HEAD, path).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{path}");
            assert!(response.as_bytes().is_empty(), "{path}");
            assert_eq!(
                header(&response, "cache-control").as_deref(),
                Some(cache),
                "{path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn client_routes_serve_index_html() {
    with_server(|request, _ctx| async move {
        for path in [
            "/admin/posts",
            "/admin/posts/",
            "/admin/posts/12/show",
            "/admin/media?path=uploads/posts",
            "/admin/v1.2/posts",
        ] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::OK, "{path}");
            assert_eq!(
                response.as_bytes().as_ref(),
                fixture("index.html"),
                "{path}"
            );
            assert_eq!(
                header(&response, "cache-control").as_deref(),
                Some("no-cache"),
                "{path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn an_existing_asset_is_immutable() {
    with_server(|request, _ctx| async move {
        let path = format!("/admin/{FIXTURE_ASSET}");
        let response = request.get(&path).await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.as_bytes().as_ref(), fixture(FIXTURE_ASSET));
        assert_eq!(
            header(&response, "cache-control").as_deref(),
            Some(IMMUTABLE)
        );
        assert!(
            header(&response, "content-type").is_some_and(|value| value.contains("javascript")),
            "a JavaScript MIME type, never text/html"
        );

        // Revalidation answers 304 and keeps the immutable lifetime.
        let last_modified = header(&response, "last-modified").expect("Last-Modified");
        let revalidated = request
            .get(&path)
            .add_header("if-modified-since", last_modified.as_str())
            .await;
        assert_eq!(revalidated.status_code(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            header(&revalidated, "cache-control").as_deref(),
            Some(IMMUTABLE)
        );

        // A byte range is a success too.
        let ranged = request.get(&path).add_header("range", "bytes=0-9").await;
        assert_eq!(ranged.status_code(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(ranged.as_bytes().as_ref(), &fixture(FIXTURE_ASSET)[..10]);
        assert_eq!(header(&ranged, "cache-control").as_deref(), Some(IMMUTABLE));
    })
    .await;
}

#[tokio::test]
async fn a_missing_asset_is_a_plain_404_and_never_index_html() {
    with_server(|request, _ctx| async move {
        for path in [
            "/admin/assets/index-00000000.js",
            "/admin/assets/missing.css",
            "/admin/assets/no-extension",
            "/admin/assets/",
            "/admin/assets",
        ] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(response.text(), "Not Found\n", "{path}");
            assert_eq!(
                header(&response, "cache-control").as_deref(),
                Some("no-store"),
                "{path}"
            );
            assert_eq!(
                header(&response, "content-type").as_deref(),
                Some("text/plain; charset=utf-8"),
                "{path}"
            );
        }
        // A range on a missing asset is not a success either.
        let ranged = request
            .get("/admin/assets/missing.js")
            .add_header("range", "bytes=0-9")
            .await;
        assert_eq!(ranged.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(
            header(&ranged, "cache-control").as_deref(),
            Some("no-store")
        );
    })
    .await;
}

#[tokio::test]
async fn root_files_are_served_with_revalidation() {
    with_server(|request, _ctx| async move {
        let response = request.get("/admin/favicon.svg").await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.as_bytes().as_ref(), fixture("favicon.svg"));
        assert_eq!(
            header(&response, "cache-control").as_deref(),
            Some("no-cache")
        );
        assert_eq!(
            header(&response, "content-type").as_deref(),
            Some("image/svg+xml")
        );

        let index = request.get("/admin/index.html").await;
        assert_eq!(index.status_code(), StatusCode::OK);
        assert_eq!(header(&index, "cache-control").as_deref(), Some("no-cache"));
    })
    .await;
}

#[tokio::test]
async fn missing_root_files_are_a_404() {
    with_server(|request, _ctx| async move {
        for path in [
            "/admin/favicon.ico",
            "/admin/posts/12/show.json",
            "/admin/.env",
        ] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(response.text(), "Not Found\n", "{path}");
            assert_eq!(
                header(&response, "cache-control").as_deref(),
                Some("no-store"),
                "{path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn other_methods_get_405_without_cache_control() {
    with_server(|request, _ctx| async move {
        let asset = format!("/admin/{FIXTURE_ASSET}");
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            for path in ["/admin", "/admin/", "/admin/posts/1", asset.as_str()] {
                let response = request.method(method.clone(), path).await;
                assert_eq!(
                    response.status_code(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {path}"
                );
                assert_eq!(header(&response, "cache-control"), None, "{method} {path}");
                let allow = header(&response, "allow").expect("Allow header");
                assert!(allow.contains("GET") && allow.contains("HEAD"), "{allow}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn responses_are_compressed_by_the_loco_stack() {
    with_server(|request, _ctx| async move {
        let response = request
            .get("/admin/posts/1")
            .add_header("accept-encoding", "gzip")
            .await;
        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(
            header(&response, "content-encoding").as_deref(),
            Some("gzip")
        );
    })
    .await;
}

/// Raw request targets that try to leave `dist/`. They go through
/// `Router::oneshot` because HTTP clients normalise `..` (and `%2e%2e`)
/// before sending, which would hide the attempt.
#[tokio::test]
async fn path_traversal_never_leaves_the_dist_directory() {
    with_app(|router, _ctx| async move {
        let manifest =
            std::fs::read_to_string(crate::support::repository_root().join("Cargo.toml"))
                .expect("Cargo.toml");
        for target in [
            "/admin/../Cargo.toml",
            "/admin/assets/../../Cargo.toml",
            "/admin/assets/../../../Cargo.toml",
            "/admin/assets/%2e%2e/%2e%2e/Cargo.toml",
            "/admin/assets/%2E%2E%2F%2E%2E%2FCargo.toml",
            "/admin/..%2f..%2f..%2fCargo.toml",
            "/admin/assets/..%5c..%5cCargo.toml",
            "/admin/%2e%2e/%2e%2e/%2e%2e/Cargo.toml",
            "/admin/assets/%2e%2e/index.html",
        ] {
            let response = router
                .clone()
                .oneshot(Request::get(target).body(Body::empty()).expect("request"))
                .await
                .expect("infallible");
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{target}");
            assert_eq!(response.headers()["cache-control"], "no-store", "{target}");
            let body = to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body");
            assert_eq!(body, "Not Found\n", "{target}");
            assert!(
                !String::from_utf8_lossy(&body).contains(&manifest[..40]),
                "{target}"
            );
        }
    })
    .await;
}
