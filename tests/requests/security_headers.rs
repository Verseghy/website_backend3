//! Security headers (ADR 0023 §6). `/admin` responses carry the admin set,
//! and also every other header of the global `secure_headers` preset, which
//! proves that the `/admin` router runs inside Loco's middleware stack
//! (ADR 0004 §3). API responses carry the deny-all CSP.

use axum::http::{Method, StatusCode};
use backend3::controllers::admin_ui::ADMIN_SECURITY_HEADERS;

use crate::support::{FIXTURE_ASSET, with_server};

/// The global CSP of `config/<env>.yaml` for JSON responses.
const API_CSP: &str =
    "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// Headers of Loco's `github` preset that neither the admin set nor the
/// overrides replace.
const PRESET_HEADERS: &[(&str, &str)] = &[
    ("strict-transport-security", "max-age=631138519"),
    ("x-content-type-options", "nosniff"),
    ("x-download-options", "noopen"),
    ("x-permitted-cross-domain-policies", "none"),
];

#[tokio::test]
async fn admin_responses_carry_the_admin_headers() {
    with_server(|request, _ctx| async move {
        let asset = format!("/admin/{FIXTURE_ASSET}");
        for (method, path, status) in [
            (Method::GET, "/admin", StatusCode::OK),
            (Method::GET, "/admin/posts/12/show", StatusCode::OK),
            (Method::GET, asset.as_str(), StatusCode::OK),
            (
                Method::GET,
                "/admin/assets/missing.js",
                StatusCode::NOT_FOUND,
            ),
            (Method::POST, "/admin", StatusCode::METHOD_NOT_ALLOWED),
        ] {
            let response = request.method(method.clone(), path).await;
            assert_eq!(response.status_code(), status, "{method} {path}");
            for (name, value) in ADMIN_SECURITY_HEADERS.iter().chain(PRESET_HEADERS) {
                assert_eq!(
                    response
                        .maybe_header(*name)
                        .as_ref()
                        .map(|v| v.to_str().expect("ASCII")),
                    Some(*value),
                    "{method} {path}: {name}"
                );
            }
            assert!(
                response
                    .maybe_header("access-control-allow-origin")
                    .is_none(),
                "{method} {path}: /admin never sends CORS headers (ADR 0014)"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn api_responses_carry_the_global_headers() {
    with_server(|request, _ctx| async move {
        for path in [
            "/api/admin/nope",
            "/api/v1/nope",
            "/api/v1/openapi.json",
            "/_ping",
        ] {
            let response = request.get(path).await;
            assert_eq!(
                response.header("content-security-policy"),
                API_CSP,
                "{path}"
            );
            assert_eq!(
                response.header("referrer-policy"),
                "strict-origin-when-cross-origin",
                "{path}"
            );
            assert_eq!(response.header("x-frame-options"), "sameorigin", "{path}");
            for (name, value) in PRESET_HEADERS {
                assert_eq!(response.header(*name), *value, "{path}: {name}");
            }
        }
        let admin_api = request.get("/api/admin/nope").await;
        assert!(
            admin_api
                .maybe_header("access-control-allow-origin")
                .is_none()
        );
    })
    .await;
}
