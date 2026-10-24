//! The `/admin` router: serves the built admin single-page app (ADR 0004 §3).
//!
//! Loco's `static` middleware is the wrong tool for an SPA with hashed assets:
//! it answers a missing file with the fallback file and status 200, and sets
//! one `Cache-Control` for every file. This router is built instead from
//! tower-http's `ServeDir` and `ServeFile` and follows the routing table of
//! ADR 0004 §3:
//!
//! | Request (GET or HEAD) | Response | `Cache-Control` |
//! |---|---|---|
//! | `/admin`, `/admin/` | `index.html`, 200 | `no-cache` |
//! | `/admin/assets/<file>` that exists | the file, 200 or 304 | `public, max-age=31536000, immutable` |
//! | `/admin/assets/<file>` that does not exist | 404, short plain text | `no-store` |
//! | `/admin/<name>.<ext>` that exists | the file, 200 | `no-cache` |
//! | `/admin/<name>.<ext>` that does not exist | 404 | `no-store` |
//! | `/admin/<path>` without a dot in its last segment | `index.html`, 200 | `no-cache` |
//! | any other method | 405 | none |
//!
//! The path decision is the pure function [`crate::domain::admin_path`]. This
//! module only does the file serving and sets the headers.
//!
//! `App::before_routes` mounts the router, so Loco wraps it in its whole
//! middleware stack: request id, logging, compression and the global secure
//! headers apply to `/admin` exactly as to the API (loco-rs 1.2.0
//! `boot::setup_routes`). The admin's own security headers (ADR 0023 §6) are
//! set here with `SetResponseHeaderLayer::overriding`; Loco's
//! `secure_headers` never overwrites a header that is already present, so
//! these values win on `/admin` while every other header of the preset is
//! still added.

use std::{
    convert::Infallible,
    io,
    path::{Path, PathBuf},
};

use axum::{
    Router,
    body::{Body, Bytes, HttpBody},
    extract::{Request, State},
    http::{
        HeaderName, HeaderValue, StatusCode, Uri,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use tower::ServiceExt;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

use crate::{
    domain::admin_path::{self, AdminRequest},
    settings::{ADMIN_UI_DIST_DIR_KEY, AdminUiSettings},
};

/// The path prefix of the admin UI (ADR 0014).
pub const ADMIN_PREFIX: &str = "/admin";

/// The file every client route and `/admin` itself answer with.
pub const INDEX_FILE: &str = "index.html";

/// The `Content-Security-Policy` of every `/admin` response (ADR 0023 §6).
///
/// - Scripts are never inline.
/// - `style-src 'unsafe-inline'` is a deliberate exception: the rich-text
///   editor and the UI kit set `style` attributes.
/// - `img-src` has no `data:`. ADR 0023 §6 kept it only if the UI kit itself
///   uses `data:` images; the built admin contains no `data:` URL and the E2E
///   suite, which fails on any CSP violation, passes without it, so the
///   bootstrap PR removed it as that section directs. `blob:` covers previews
///   of files chosen for upload.
/// - `frame-src` matches the editor's embed allow-list (ADR 0013).
pub const ADMIN_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' blob:; font-src 'self'; \
     connect-src 'self'; frame-src https://www.youtube-nocookie.com https://www.youtube.com \
     https://padlet.com; media-src 'self'; object-src 'none'; base-uri 'none'; \
     form-action 'self'; frame-ancestors 'none'";

/// The security headers that every `/admin` response carries, overriding the
/// global API values (ADR 0023 §6).
pub const ADMIN_SECURITY_HEADERS: &[(&str, &str)] = &[
    ("content-security-policy", ADMIN_CONTENT_SECURITY_POLICY),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "strict-origin-when-cross-origin"),
    (
        "permissions-policy",
        "camera=(), microphone=(), geolocation=(), payment=()",
    ),
    ("cross-origin-opener-policy", "same-origin"),
    ("x-robots-tag", "noindex, nofollow"),
];

/// Whether the router may be built when the admin UI directory is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistRequirement {
    /// The directory and its `index.html` must exist, or building the router
    /// fails. Production uses this, so a broken image never starts serving.
    Required,
    /// A missing directory is logged and every `/admin` request answers
    /// `404`. Development uses this, because there the Vite dev server serves
    /// the UI (ADR 0004 §4).
    Optional,
}

/// Why the admin UI router could not be built.
#[derive(Debug, thiserror::Error)]
pub enum AdminUiError {
    /// The configured directory does not exist or is not a directory.
    #[error(
        "Refusing to start: the admin UI directory '{path}' (setting {key}) does not exist or is \
         not a directory: {source}. Build the admin UI (`npm run build` in admin/) into that \
         directory, or point {key} at the directory that holds it."
    )]
    MissingDirectory {
        /// The configured directory.
        path: PathBuf,
        /// The setting key, [`ADMIN_UI_DIST_DIR_KEY`].
        key: &'static str,
        /// The underlying file-system error.
        source: io::Error,
    },
    /// The directory exists but has no `index.html` file.
    #[error(
        "Refusing to start: the admin UI directory '{path}' (setting {key}) has no index.html. \
         Build the admin UI (`npm run build` in admin/) into that directory, or point {key} at \
         the directory that holds it."
    )]
    MissingIndex {
        /// The configured directory.
        path: PathBuf,
        /// The setting key, [`ADMIN_UI_DIST_DIR_KEY`].
        key: &'static str,
    },
}

/// Checks that the admin UI directory and its `index.html` exist.
///
/// - `dist_dir`: the configured directory of the built admin UI.
///
/// # Errors
///
/// Returns an [`AdminUiError`] that names the configured path and the
/// setting key.
pub fn check_dist_dir(dist_dir: &Path) -> Result<(), AdminUiError> {
    let missing_directory = |source| AdminUiError::MissingDirectory {
        path: dist_dir.to_path_buf(),
        key: ADMIN_UI_DIST_DIR_KEY,
        source,
    };
    let metadata = std::fs::metadata(dist_dir).map_err(missing_directory)?;
    if !metadata.is_dir() {
        return Err(missing_directory(io::Error::new(
            io::ErrorKind::NotADirectory,
            "it is a file",
        )));
    }
    if !dist_dir.join(INDEX_FILE).is_file() {
        return Err(AdminUiError::MissingIndex {
            path: dist_dir.to_path_buf(),
            key: ADMIN_UI_DIST_DIR_KEY,
        });
    }
    Ok(())
}

/// The file services of one admin UI directory.
#[derive(Clone)]
struct AdminUi {
    /// Serves file requests from the directory. Directories are not answered
    /// with their `index.html`: a directory is never a file request.
    files: ServeDir,
    /// Serves `index.html` for `/admin` and for client routes.
    index: ServeFile,
}

/// Builds the `/admin` router for one admin UI directory.
///
/// - `settings`: the admin UI settings; only `dist_dir` is used.
/// - `requirement`: whether a missing directory fails the build
///   ([`DistRequirement::Required`], production) or is only logged.
///
/// The returned router has no state of its own, so it merges into any
/// router, `Router<AppContext>` included.
///
/// # Errors
///
/// With [`DistRequirement::Required`], returns the [`AdminUiError`] of
/// [`check_dist_dir`].
pub fn router<S>(
    settings: &AdminUiSettings,
    requirement: DistRequirement,
) -> Result<Router<S>, AdminUiError>
where
    S: Clone + Send + Sync + 'static,
{
    let dist_dir = &settings.dist_dir;
    if let Err(error) = check_dist_dir(dist_dir) {
        match requirement {
            DistRequirement::Required => return Err(error),
            DistRequirement::Optional => {
                tracing::warn!(%error, "admin UI not available; /admin answers 404");
            }
        }
    }

    let ui = AdminUi {
        files: ServeDir::new(dist_dir).append_index_html_on_directories(false),
        index: ServeFile::new(dist_dir.join(INDEX_FILE)),
    };

    // `get` also answers HEAD; every other method gets axum's 405 with an
    // `Allow` header and no `Cache-Control`, as the routing table requires.
    let mut router = Router::new()
        .route(ADMIN_PREFIX, get(serve))
        .route(&format!("{ADMIN_PREFIX}/"), get(serve))
        .route(&format!("{ADMIN_PREFIX}/{{*path}}"), get(serve))
        .with_state(ui);
    for (name, value) in ADMIN_SECURITY_HEADERS {
        router = router.layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        ));
    }
    Ok(router)
}

/// Answers one GET or HEAD request below `/admin`.
async fn serve(State(ui): State<AdminUi>, request: Request) -> Response {
    let rest = request
        .uri()
        .path()
        .strip_prefix(ADMIN_PREFIX)
        .unwrap_or_default()
        .to_owned();
    let class = admin_path::classify(&rest);

    let response = match class {
        AdminRequest::Rejected => not_found(),
        AdminRequest::Index | AdminRequest::ClientRoute => {
            into_axum(ui.index.oneshot(request).await)
        }
        AdminRequest::Asset | AdminRequest::File => {
            // ServeDir resolves the request path below its directory, so hand
            // it the path without the `/admin` prefix. It percent-decodes the
            // path and refuses `..` itself as a second line of defence.
            let (mut parts, body) = request.into_parts();
            parts.uri = match Uri::builder().path_and_query(rest.as_str()).build() {
                Ok(uri) => uri,
                Err(_) => return with_cache_control(not_found(), class),
            };
            into_axum(ui.files.oneshot(Request::from_parts(parts, body)).await)
        }
    };

    // A missing file gets the short plain-text body of the routing table
    // instead of ServeDir's empty one.
    let response = if response.status() == StatusCode::NOT_FOUND {
        not_found()
    } else {
        response
    };
    with_cache_control(response, class)
}

/// Converts a tower-http file response into an axum response.
fn into_axum<B>(result: Result<axum::http::Response<B>, Infallible>) -> Response
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    // The file services never fail: their error type is `Infallible`.
    let Ok(response) = result;
    response.map(Body::new)
}

/// The `404` of the routing table: a short plain-text body, never
/// `index.html`.
fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Not Found\n",
    )
        .into_response()
}

/// Sets the `Cache-Control` that the routing table assigns to `class` and the
/// response's status.
fn with_cache_control(mut response: Response, class: AdminRequest) -> Response {
    let value = admin_path::cache_control(class, response.status().as_u16());
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(value));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_headers_are_valid_header_values() {
        for (name, value) in ADMIN_SECURITY_HEADERS {
            assert!(HeaderName::from_bytes(name.as_bytes()).is_ok(), "{name}");
            assert!(HeaderValue::from_str(value).is_ok(), "{name}");
            assert_eq!(
                *name,
                name.to_ascii_lowercase(),
                "header names must be lowercase"
            );
        }
    }

    #[test]
    fn the_csp_matches_adr_0023() {
        // ADR 0023 §6's value, with `data:` dropped from img-src as §6 directs
        // once the CSP-violation E2E passes without it.
        assert_eq!(
            ADMIN_CONTENT_SECURITY_POLICY,
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src \
             'self' blob:; font-src 'self'; connect-src 'self'; frame-src \
             https://www.youtube-nocookie.com https://www.youtube.com https://padlet.com; \
             media-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; \
             frame-ancestors 'none'"
        );
    }

    #[test]
    fn missing_directory_is_reported_with_path_and_key() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/no-such-dist");
        let error = check_dist_dir(&path).expect_err("missing");
        let message = error.to_string();
        assert!(matches!(error, AdminUiError::MissingDirectory { .. }));
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(message.contains(ADMIN_UI_DIST_DIR_KEY), "{message}");
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(matches!(
            check_dist_dir(&path),
            Err(AdminUiError::MissingDirectory { .. })
        ));
    }

    #[test]
    fn a_directory_without_index_is_reported() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/admin-dist/assets");
        let error = check_dist_dir(&path).expect_err("no index");
        assert!(matches!(error, AdminUiError::MissingIndex { .. }));
        assert!(error.to_string().contains("has no index.html"));
    }

    #[test]
    fn the_fixture_directory_is_complete() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/admin-dist");
        check_dist_dir(&path).expect("fixture has index.html");
    }

    #[test]
    fn required_fails_and_optional_builds_without_the_directory() {
        let settings = AdminUiSettings {
            dist_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/no-such-dist"),
        };
        assert!(router::<()>(&settings, DistRequirement::Required).is_err());
        assert!(router::<()>(&settings, DistRequirement::Optional).is_ok());
    }

    #[test]
    fn cache_control_is_set_from_the_class() {
        let response = with_cache_control(not_found(), AdminRequest::Asset);
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        let ok = with_cache_control(StatusCode::OK.into_response(), AdminRequest::Asset);
        assert_eq!(ok.headers()[CACHE_CONTROL], admin_path::CACHE_IMMUTABLE);
    }
}
