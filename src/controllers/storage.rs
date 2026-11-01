//! `/storage` served by backend3 itself: the fallback router of ADR 0015 §4.
//!
//! The media variant in use is the separate nginx media server (ADR 0015
//! §1-§3; the co-mount test passed, research: DECISIONS R9, D43). This
//! router exists so that the other variant can deploy without a new image:
//! if the media server ever cannot share the uploads volume, backend3 runs
//! alone with the volume, `MEDIA_SERVE_STORAGE=true` turns
//! `settings.media.serve_storage` on, and the three `/storage` Routes point
//! at backend3 (ADR 0015 §4). With the setting off, which every committed
//! configuration but `config/e2e.yaml` has, [`mount`] adds nothing and
//! `/storage` is an unknown path like any other.
//!
//! Both servers must meet one media contract (ADR 0015 §1, with the two
//! header rows of ADR 0013 §7), and one suite checks both
//! (`tests/media_contract/`; this router's run is in
//! `tests/requests/storage.rs`). The router is tower-http's `ServeDir` on
//! `media.root`, never Loco's `static` middleware, which answers a missing
//! file with `200` (ADR 0015 §4), wrapped in what `ServeDir` lacks:
//!
//! - **No index files, no listings.** `append_index_html_on_directories(false)`,
//!   so a directory is `404`.
//! - **The dot-segment guard.** [`storage_rules::parse_request_path`] refuses
//!   a path with a segment that starts with `.` (dot-files, `..`), a NUL or
//!   invalid UTF-8, before anything touches the file system.
//! - **Regular files only, never through a link.** `ServeDir` follows
//!   symbolic links. Before it opens a file, [`look_up`] walks the path from
//!   the root, one segment at a time with `O_NOFOLLOW`, and serves it only
//!   if no segment is a link and the last is a regular file (the suite's
//!   reading: a path through a link is `403` or `404`, wherever it points).
//!   The root itself is the operator's mount and is opened as configured.
//!   `ServeDir` then opens the file by path again; in the fallback variant
//!   backend3 is the only writer of the volume and never creates a link
//!   (uploads refuse one, `models::media`), so both see the same tree.
//! - **The methods.** `GET` and `HEAD` are served, `OPTIONS` answers the
//!   CORS preflight for any path, every other method is `405` with `Allow`
//!   ([`storage_rules::method_answer`]).
//! - **The headers.** `SetResponseHeaderLayer::overriding` puts the three
//!   that every answer carries (`Access-Control-Allow-Origin: *`,
//!   `Access-Control-Expose-Headers` and `nosniff`) on everything the router
//!   sends; [`with_contract_headers`] adds the ones that depend on the
//!   answer, as [`storage_rules::response_headers`] decides them: the
//!   sandbox policy, `Content-Disposition: attachment` and `Cache-Control`
//!   per folder.
//! - **`If-Range`.** `ServeDir` ignores it and cuts the range out of
//!   whatever file it opened. [`storage_rules::range_answer_stands`] checks
//!   the request's `If-Range` against the `Last-Modified` of that answer,
//!   and when the condition fails the router asks `ServeDir` again without
//!   the `Range`, so the client gets the whole file with `200`, as nginx
//!   sends it (RFC 9110 §13.1.5).
//! - **Its own `TraceLayer`**, which logs one line per request at `INFO`
//!   (ADR 0015 §1, Logs): the router runs outside Loco's middleware stack,
//!   so Loco's logger and request id do not reach it.
//!
//! **A percent-encoded prefix.** Routing reads the raw path, so a request
//! whose `/storage/` is itself percent-encoded, such as
//! `/storage%2Fuploads/a.pdf` (or `/%73torage/...`), matches none of
//! [`ROUTES`] and gets the application's plain `404` for an unknown path,
//! without the contract's headers, where nginx decodes the path before it
//! matches its location and serves the file. This is the one known
//! difference the contract tables do not name. No link has that form:
//! every URL shape of ADR 0015 ("The URL contract") writes `/storage/`
//! literally, and the suite sends none. An encoded slash further down
//! separates segments as nginx reads it
//! ([`storage_rules::parse_request_path`]); `tests/requests/storage.rs`
//! pins both answers.
//!
//! **Outside Loco's middleware stack.** `App::after_routes` calls [`mount`]
//! on the finished router, after Loco has applied its middleware (loco-rs
//! 1.2.0 `boot::setup_routes`), so no body limit, request timeout, `etag`,
//! compression or secure-headers layer applies to `/storage` (ADR 0015 §4).
//! A request timeout would cut an 86 MB PDF on a slow line, and compression
//! would break byte ranges. The secure-headers preset would add the API's
//! deny-all `Content-Security-Policy` to every answer, a PDF's included,
//! which the contract exempts. The `etag` layer answers a matching
//! `If-None-Match` with a `304` of its own that keeps none of the contract's
//! headers (loco-rs 1.2.0 `controller/middleware/etag.rs`). The HTTP request
//! metrics, added after [`mount`], do count `/storage` requests.
//!
//! **Merged, not nested.** The routes are registered with their full
//! templates (`/storage/{*path}`) and merged into the router, rather than
//! `nest_service`d: axum gives a request below a nested service no
//! `MatchedPath`, so the request metrics would label every file request
//! `unmatched`, among the real unknown paths.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Router as AxumRouter,
    body::Body,
    extract::{Request, State},
    http::{
        HeaderName, HeaderValue, StatusCode, Uri,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
            ALLOW, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_SECURITY_POLICY, CONTENT_TYPE, IF_RANGE, LAST_MODIFIED, RANGE, USER_AGENT,
            X_CONTENT_TYPE_OPTIONS,
        },
        request::Parts,
    },
    response::{IntoResponse, Response},
    routing::any,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rustix::{
    fs::{self as at, FileType, Mode, OFlags},
    io::Errno,
};
use tower::ServiceExt as _;
use tower_http::{
    LatencyUnit,
    services::ServeDir,
    set_header::SetResponseHeaderLayer,
    trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{
    domain::storage_rules::{
        self, ALLOW_ORIGIN, ALLOWED_METHODS, ContentTypeRule, EXPOSE_HEADERS, MethodAnswer,
        NOSNIFF, PREFLIGHT_ALLOW_HEADERS, PREFLIGHT_MAX_AGE, RANGE_ERROR_BODY, RANGE_ERROR_TYPE,
        StoragePath,
    },
    settings::MediaSettings,
};

/// The path prefix of the served tree, on all three hosts (ADR 0015 §2).
pub const STORAGE_PREFIX: &str = "/storage";

/// The route templates the router registers, which are also the `route`
/// label of its requests in the HTTP metrics. `/storage/` never arrives
/// through `App::serve`, which trims the slash first, but is registered so
/// that the router answers it wherever it is served.
pub const ROUTES: [&str; 3] = ["/storage", "/storage/", "/storage/{*path}"];

/// Bytes that stay literal when [`serve_file`] hands the checked path to
/// `ServeDir`: RFC 3986 unreserved characters. Every other byte, `/` inside a
/// segment and every byte of a non-ASCII name included, is percent-encoded,
/// so `ServeDir`'s decoding gives back exactly the checked segments.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// What the routes share: the served tree and the file service on it.
#[derive(Clone)]
struct Storage {
    /// `media.root`, the tree served as `/storage/**`.
    root: Arc<PathBuf>,
    /// `ServeDir` on the root, without index files.
    files: ServeDir,
}

/// Adds the `/storage` routes to the finished application router when
/// `settings.serve_storage` is on, and returns the router unchanged when it
/// is off.
///
/// - `router`: the router Loco built, its middleware stack already applied;
///   the `/storage` routes stay outside that stack (module docs).
/// - `settings`: the media settings; `serve_storage` decides, `root` is the
///   served tree.
///
/// The root need not exist yet: production starts before its volume is
/// mounted, and until then every file request is `404` (ADR 0015 §3).
pub fn mount(router: AxumRouter, settings: &MediaSettings) -> AxumRouter {
    if !settings.serve_storage {
        return router;
    }
    tracing::info!(
        root = %settings.root.display(),
        "serving /storage from the media root (the fallback router of ADR 0015 §4)"
    );
    router.merge(router_for(&settings.root))
}

/// The `/storage` routes on one served tree, with the contract's header
/// layers and the router's own `TraceLayer`.
///
/// - `root`: the directory served as `/storage`, `media.root`.
///
/// Layers that `route_layer` adds run only for these routes, never for the
/// fallback of a router they are merged into. The last one added runs
/// first: the trace span opens before everything else and sees the final
/// answer, and the three constant headers are set on every answer,
/// refusals included.
pub fn router_for(root: &Path) -> AxumRouter {
    let storage = Storage {
        root: Arc::new(root.to_path_buf()),
        files: ServeDir::new(root).append_index_html_on_directories(false),
    };
    let mut router = AxumRouter::new();
    for template in ROUTES {
        router = router.route(template, any(serve));
    }
    router
        .with_state(storage)
        .route_layer(overriding(X_CONTENT_TYPE_OPTIONS, NOSNIFF))
        .route_layer(overriding(ACCESS_CONTROL_EXPOSE_HEADERS, EXPOSE_HEADERS))
        .route_layer(overriding(ACCESS_CONTROL_ALLOW_ORIGIN, ALLOW_ORIGIN))
        .route_layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_request(())
                // One line per request at INFO, the level production logs
                // (ADR 0015 §1, Logs); tower-http's default is DEBUG.
                .on_response(
                    DefaultOnResponse::new()
                        .level(Level::INFO)
                        .latency_unit(LatencyUnit::Micros),
                )
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
}

/// A layer that sets `name` to `value` on every answer, replacing any value
/// already there, so each is sent exactly once.
fn overriding(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
}

/// The span of one `/storage` request: the fields of Loco's own request
/// span (loco-rs 1.2.0 `controller/middleware/logger.rs`) except the request
/// id, which Loco's middleware would have set.
///
/// - `request`: the request.
fn request_span(request: &Request) -> tracing::Span {
    let user_agent = request
        .headers()
        .get(USER_AGENT)
        .map_or("", |value| value.to_str().unwrap_or(""));
    tracing::info_span!(
        "storage-request",
        "http.method" = %request.method(),
        "http.uri" = %request.uri(),
        "http.version" = ?request.version(),
        "http.user_agent" = %user_agent,
    )
}

/// Answers one request below `/storage`, whatever its method.
async fn serve(State(storage): State<Storage>, request: Request) -> Response {
    let below = request
        .uri()
        .path()
        .strip_prefix(STORAGE_PREFIX)
        .unwrap_or_default()
        .to_owned();
    let path = match storage_rules::parse_request_path(&below) {
        Ok(path) => path,
        Err(refusal) => {
            tracing::debug!(%refusal, "refused a /storage path");
            return with_contract_headers(not_found(), &StoragePath::default());
        }
    };
    let response = match storage_rules::method_answer(request.method().as_str()) {
        MethodAnswer::Serve => serve_file(&storage, &path, request).await,
        MethodAnswer::Preflight => preflight(),
        MethodAnswer::NotAllowed => method_not_allowed(),
    };
    with_contract_headers(response, &path)
}

/// Serves `path` with `ServeDir` once [`look_up`] has found a regular file
/// there, reached through no symbolic link, and answers again without the
/// `Range` when the request's `If-Range` no longer names that file.
///
/// - `storage`: the served tree and its file service.
/// - `path`: the checked path below the root.
/// - `request`: the `GET` or `HEAD`; its method and headers (`Range`,
///   `If-Modified-Since`) go to `ServeDir`, and its `If-Range` decides
///   whether `ServeDir`'s range answer stands.
async fn serve_file(storage: &Storage, path: &StoragePath, request: Request) -> Response {
    // The root is a directory and never served: no file-system call for it.
    if path.is_root() {
        return not_found();
    }
    match look_up(Arc::clone(&storage.root), path.clone()).await {
        Ok(Lookup::RegularFile) => {}
        Ok(Lookup::NotServed) => return not_found(),
        Err(error) => {
            tracing::error!(
                %error,
                path = %path.relative(),
                "cannot look up a /storage file below the media root"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    // ServeDir resolves the request path below its root, so it gets the
    // checked segments, each percent-encoded, and no query.
    let mut encoded = String::new();
    for segment in path.segments() {
        encoded.push('/');
        encoded.extend(utf8_percent_encode(segment, SEGMENT));
    }
    // ServeDir reads only the method, the target and the headers, never a
    // body (a GET or HEAD has none), so the parts are kept for a second ask.
    let (mut parts, _body) = request.into_parts();
    parts.uri = match Uri::builder().path_and_query(encoded.as_str()).build() {
        Ok(uri) => uri,
        // Every byte outside SEGMENT is encoded, so the target is always
        // valid; a refusal here is a 404 like any name that is not served.
        Err(_) => return not_found(),
    };
    let response = serve_dir(&storage.files, parts.clone()).await;
    let stands = {
        let validators: Vec<&[u8]> = parts
            .headers
            .get_all(IF_RANGE)
            .iter()
            .map(HeaderValue::as_bytes)
            .collect();
        let last_modified = response.headers().get(LAST_MODIFIED);
        storage_rules::range_answer_stands(
            response.status().as_u16(),
            &validators,
            last_modified.map(HeaderValue::as_bytes),
        )
    };
    if stands {
        return response;
    }
    // ServeDir ignores If-Range, and RFC 9110 §13.1.5 requires the whole
    // file when it fails. The check reads the Last-Modified of the file
    // ServeDir opened, not a stat of the lookup, so a range is cut only from
    // a file whose own date matches, even one replaced after the lookup.
    // Dropping the first answer closes its file unread; the second ask is
    // the same request without its Range.
    drop(response);
    parts.headers.remove(RANGE);
    serve_dir(&storage.files, parts).await
}

/// `ServeDir`'s answer to the request `parts`, with an empty body.
///
/// - `files`: the file service on the media root.
/// - `parts`: the request, its target already the checked, encoded path.
async fn serve_dir(files: &ServeDir, parts: Parts) -> Response {
    // ServeDir's error type is Infallible: it answers an I/O error with 500.
    let Ok(response) = files
        .clone()
        .oneshot(Request::from_parts(parts, Body::empty()))
        .await;
    response.map(Body::new)
}

/// What [`look_up`] found at a path below the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    /// A regular file, reached through directories only.
    RegularFile,
    /// Nothing the router serves: missing, a directory, a symbolic link or
    /// a path through one, a special file, or a name the file system
    /// refuses. The answer is `404`.
    NotServed,
}

/// Whether `path` is a regular file below `root`, reached without following
/// any symbolic link, on a blocking thread, since file-system calls block.
///
/// - `root`: the served tree, opened as configured (it may itself be a
///   link: the operator's mount path).
/// - `path`: the checked path below it.
///
/// # Errors
///
/// An I/O error that says nothing about the path, such as too many open
/// files: the router answers it `500`, not `404`, so it shows in the log.
async fn look_up(root: Arc<PathBuf>, path: StoragePath) -> io::Result<Lookup> {
    tokio::task::spawn_blocking(move || look_up_blocking(&root, path.segments()))
        .await
        .map_err(io::Error::other)?
}

/// The work of [`look_up`]: opens the root, then each folder segment with
/// `O_DIRECTORY | O_NOFOLLOW` relative to its parent, then the last segment
/// with `O_NOFOLLOW`, all with `O_PATH`, which reads no content, and checks
/// the type of what it found.
///
/// - `root`: the served tree.
/// - `segments`: the checked segments below it; none means the root, a
///   directory.
fn look_up_blocking(root: &Path, segments: &[String]) -> io::Result<Lookup> {
    let Some((last, folders)) = segments.split_last() else {
        return Ok(Lookup::NotServed);
    };
    let directory = OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut parent = match at::open(root, directory, Mode::empty()) {
        Ok(fd) => fd,
        Err(errno) => return not_served_or_error(errno),
    };
    for folder in folders {
        parent = match at::openat(
            &parent,
            folder.as_str(),
            directory | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(errno) => return not_served_or_error(errno),
        };
    }
    let file = match at::openat(
        &parent,
        last.as_str(),
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(errno) => return not_served_or_error(errno),
    };
    // With O_PATH | O_NOFOLLOW, a link is opened itself, so its type shows.
    let stat = at::fstat(&file)?;
    Ok(
        if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile {
            Lookup::RegularFile
        } else {
            Lookup::NotServed
        },
    )
}

/// The lookup result of an `open` that failed with `errno`: the errors that
/// mean "no such file to serve" are [`Lookup::NotServed`], the rest are
/// errors.
///
/// - `errno`: the failure. `ENOENT` (missing, or the volume not mounted),
///   `ENOTDIR` (a file used as a folder, or a link to a folder under
///   `O_DIRECTORY | O_NOFOLLOW`), `ELOOP` (a link under `O_NOFOLLOW`),
///   `EACCES` and `ENAMETOOLONG` all leave nothing to serve.
fn not_served_or_error(errno: Errno) -> io::Result<Lookup> {
    if matches!(
        errno,
        Errno::NOENT | Errno::NOTDIR | Errno::LOOP | Errno::ACCESS | Errno::NAMETOOLONG
    ) {
        Ok(Lookup::NotServed)
    } else {
        Err(errno.into())
    }
}

/// The `404` of every path the router does not serve: no body, so nothing
/// for a browser to render.
fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

/// The CORS preflight (ADR 0015 §1, CORS): `204` that allows `GET`, `HEAD`
/// and the `Range` header, cached by the browser for [`PREFLIGHT_MAX_AGE`]
/// seconds.
fn preflight() -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            (ALLOW, ALLOWED_METHODS),
            (ACCESS_CONTROL_ALLOW_METHODS, ALLOWED_METHODS),
            (ACCESS_CONTROL_ALLOW_HEADERS, PREFLIGHT_ALLOW_HEADERS),
            (ACCESS_CONTROL_MAX_AGE, PREFLIGHT_MAX_AGE),
        ],
    )
        .into_response()
}

/// The `405` of every other method, with the `Allow` that RFC 9110
/// §15.5.6 requires (the suite's reading).
fn method_not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(ALLOW, ALLOWED_METHODS)]).into_response()
}

/// Sets the headers of `response` that depend on the answer, as
/// [`storage_rules::response_headers`] decides them, and turns a `416` into
/// the router's own short text.
///
/// - `response`: the answer, from `ServeDir` or the router.
/// - `path`: the requested path below the root; the root for a refused one,
///   which only `404` answers.
fn with_contract_headers(response: Response, path: &StoragePath) -> Response {
    let response_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    // The type ServeDir gives the file, for a 304, which sends none.
    let file_type = mime_guess::from_path(path.relative()).first_raw();
    let rules = storage_rules::response_headers(
        response.status().as_u16(),
        response_type.as_deref(),
        file_type,
        path,
    );
    let mut response = match rules.content_type {
        ContentTypeRule::Keep => response,
        ContentTypeRule::RangeError => range_error(&response),
    };
    let headers = response.headers_mut();
    for (name, value) in [
        (CONTENT_SECURITY_POLICY, rules.content_security_policy),
        (CONTENT_DISPOSITION, rules.content_disposition),
        (CACHE_CONTROL, rules.cache_control),
    ] {
        match value {
            Some(value) => {
                headers.insert(name, HeaderValue::from_static(value));
            }
            None => {
                headers.remove(name);
            }
        }
    }
    response
}

/// The router's own `416`, which replaces `ServeDir`'s: [`RANGE_ERROR_BODY`]
/// as [`RANGE_ERROR_TYPE`], keeping only the `Content-Range: bytes */<size>`
/// that tells the client the file's length. `ServeDir` types its `416` as
/// the file it refused to cut, which would exempt a PDF's `416` from the
/// sandbox (`storage_rules`, module docs).
///
/// - `refused`: `ServeDir`'s `416`.
fn range_error(refused: &Response) -> Response {
    let mut response = (
        StatusCode::RANGE_NOT_SATISFIABLE,
        [(CONTENT_TYPE, RANGE_ERROR_TYPE)],
        RANGE_ERROR_BODY,
    )
        .into_response();
    if let Some(range) = refused.headers().get(CONTENT_RANGE) {
        response.headers_mut().insert(CONTENT_RANGE, range.clone());
    }
    // The body changed; hyper computes the length of the new one.
    response.headers_mut().remove(CONTENT_LENGTH);
    response
}

#[cfg(test)]
mod tests {
    //! The file-system lookup on a scratch tree; the HTTP behaviour is
    //! checked by the media contract suite and `tests/requests/storage.rs`.

    use std::{
        fs,
        os::unix::fs::symlink,
        sync::atomic::{AtomicU32, Ordering},
    };

    use super::*;

    /// A fresh directory below the git-ignored `target/tmp`, removed on
    /// drop. The process id and a counter keep parallel tests and test
    /// processes apart.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/tmp")
                .join(format!(
                    "storage-lookup-{name}-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("uploads/docs")).expect("create the tree");
            fs::write(path.join("uploads/docs/a.pdf"), b"%PDF").expect("write a file");
            Self(path)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn segments(path: &str) -> Vec<String> {
        path.split('/').map(str::to_owned).collect()
    }

    #[test]
    fn a_regular_file_is_found_and_everything_else_is_not_served() {
        let tree = Tree::new("kinds");
        let found = |path: &str| look_up_blocking(&tree.0, &segments(path)).expect("lookup");
        assert_eq!(found("uploads/docs/a.pdf"), Lookup::RegularFile);
        for path in [
            "uploads/docs",
            "uploads/docs/missing.pdf",
            "uploads/docs/a.pdf/inner",
            "missing/a.pdf",
        ] {
            assert_eq!(found(path), Lookup::NotServed, "{path}");
        }
        assert_eq!(
            look_up_blocking(&tree.0, &[]).expect("lookup"),
            Lookup::NotServed
        );
        let long = "x".repeat(300);
        assert_eq!(found(&format!("uploads/{long}")), Lookup::NotServed);
    }

    #[test]
    fn no_symbolic_link_is_followed_wherever_it_points() {
        let tree = Tree::new("links");
        symlink("a.pdf", tree.0.join("uploads/docs/link.pdf")).expect("file link");
        symlink("docs", tree.0.join("uploads/folder")).expect("folder link");
        symlink("/etc/os-release", tree.0.join("uploads/outside.pdf")).expect("outside link");
        symlink("loop", tree.0.join("uploads/loop")).expect("loop");
        for path in [
            "uploads/docs/link.pdf",
            "uploads/folder/a.pdf",
            "uploads/outside.pdf",
            "uploads/loop",
            "uploads/loop/a.pdf",
        ] {
            assert_eq!(
                look_up_blocking(&tree.0, &segments(path)).expect("lookup"),
                Lookup::NotServed,
                "{path}"
            );
        }
    }

    #[test]
    fn a_root_reached_through_a_link_is_served_and_a_missing_root_is_not() {
        let tree = Tree::new("root");
        let mount = tree.0.with_extension("mount");
        symlink(&tree.0, &mount).expect("root link");
        let found = look_up_blocking(&mount, &segments("uploads/docs/a.pdf"));
        fs::remove_file(&mount).expect("remove the root link");
        assert_eq!(found.expect("lookup"), Lookup::RegularFile);
        let missing = tree.0.join("not-mounted");
        assert_eq!(
            look_up_blocking(&missing, &segments("uploads/docs/a.pdf")).expect("lookup"),
            Lookup::NotServed
        );
    }

    #[test]
    fn a_fifo_is_not_served() {
        let tree = Tree::new("fifo");
        let fifo = tree.0.join("uploads/docs/pipe.pdf");
        at::mknodat(
            at::CWD,
            &fifo,
            FileType::Fifo,
            Mode::from_raw_mode(0o644),
            0,
        )
        .expect("mkfifo");
        assert_eq!(
            look_up_blocking(&tree.0, &segments("uploads/docs/pipe.pdf")).expect("lookup"),
            Lookup::NotServed
        );
    }

    #[test]
    fn only_errors_about_the_path_mean_not_served() {
        for errno in [
            Errno::NOENT,
            Errno::NOTDIR,
            Errno::LOOP,
            Errno::ACCESS,
            Errno::NAMETOOLONG,
        ] {
            assert_eq!(
                not_served_or_error(errno).expect("not served"),
                Lookup::NotServed,
                "{errno}"
            );
        }
        for errno in [Errno::MFILE, Errno::IO, Errno::NOMEM] {
            assert!(not_served_or_error(errno).is_err(), "{errno}");
        }
    }

    #[test]
    fn the_segment_encoding_round_trips_every_name() {
        for name in ["Felvételi (1) [PR].pdf", "a%2Fb", "100%.pdf", "kép #1?.gif"] {
            let encoded = utf8_percent_encode(name, SEGMENT).to_string();
            assert!(
                encoded.bytes().all(|byte| byte.is_ascii_graphic()),
                "{encoded}"
            );
            assert!(!encoded.contains('/'), "{encoded}");
            let decoded = percent_encoding::percent_decode_str(&encoded)
                .decode_utf8()
                .expect("UTF-8");
            assert_eq!(decoded, name);
        }
    }

    #[test]
    fn the_route_templates_cover_the_prefix() {
        for template in ROUTES {
            assert!(template.starts_with(STORAGE_PREFIX), "{template}");
        }
    }
}
