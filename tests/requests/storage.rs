//! The fallback `/storage` router (ADR 0015 §4; MEDIA-02, PLT-21).
//!
//! The router is checked where production would run it: in the application
//! router that `App`'s own hooks build (`before_routes`, `routes`,
//! `after_routes`, as loco-rs 1.2.0 `boot::setup_routes` calls them) with
//! `settings.media.serve_storage` on, behind the trailing-slash trim of
//! `App::serve`, on a real loopback listener. The media contract suite of
//! `tests/media_contract/` then runs against it exactly as it runs against
//! nginx, with the two differences the contract allows (no `ETag`, `416`
//! for a multi-range request).
//!
//! Beyond the suite, these tests pin what only the fallback must do:
//!
//! - its own answers where the contract leaves a choice (the `416`, the
//!   dot-segment guard on paths the suite does not send);
//! - that it runs outside Loco's middleware stack, so no request id,
//!   secure headers, `etag` or compression reach `/storage`;
//! - that it reads the volume on every request: a file written later is
//!   served, a removed one is `404`, and an unmounted root is `404` until
//!   the volume arrives;
//! - that special files never reach `ServeDir`, which would block opening
//!   a FIFO;
//! - that `If-Range` keeps a range only for the file it names, also once
//!   the file is replaced, beyond the suite's fixed dates;
//! - that a lookup error which says nothing about the path is `500`, under
//!   the sandbox and never cached (a test cannot cause `EMFILE` or `EIO`
//!   safely, so a root no `open` can name stands in for them);
//! - how a percent-encoded `/storage/` prefix is answered, the one known
//!   difference from nginx that the contract tables do not name;
//! - that the HTTP metrics count its requests under their route template;
//! - that its `TraceLayer` logs one line per request at `INFO`
//!   (ADR 0015 §1, Logs);
//! - that with the setting off, which every committed configuration but
//!   e2e has, `/storage` is an unknown path like any other.

use std::{
    collections::HashMap,
    fmt::{self, Write as _},
    fs,
    future::IntoFuture,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Router, ServiceExt,
    body::Body,
    extract::Request as AxumRequest,
    http::{
        Method, StatusCode,
        header::{ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_TYPE},
    },
};
use backend3::{
    app::{App, trim_trailing_slash},
    controllers::{
        metrics::Metrics,
        storage::{ROUTES, router_for},
    },
    domain::storage_rules::{RANGE_ERROR_BODY, RANGE_ERROR_TYPE, SANDBOX_CSP},
};
use loco_rs::app::{AppContext, Hooks};
use rustix::fs::{self as at, FileType, Mode, OFlags};
use serde_json::json;
use tokio::{net::TcpListener, sync::oneshot};
use tower::ServiceExt as _;
use tracing::{
    Event, Id, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Record},
};

use crate::{
    media_contract::{
        HyperTransport, LAST_MODIFIED, Origin, Request, Response, Server, Transport, run, seed,
    },
    support::{TempMedia, header_text, with_app, with_server},
};

/// A text fixture of the seeded tree: 300 bytes, `text/plain`, a download.
const TEXT: &str = "/storage/uploads/docs/jegyzet.txt";

/// The application router as `App`'s hooks build it, with
/// `settings.media.serve_storage` on and the media root at `root`: what
/// production serves with `MEDIA_SERVE_STORAGE=true`.
///
/// - `ctx`: the test's context; a copy of it gets the changed settings, and
///   both share one store, so the metrics of the copy are the test's.
/// - `root`: the directory to serve as `/storage`.
async fn fallback_router(ctx: &AppContext, root: &Path) -> Router {
    let mut ctx = ctx.clone();
    let settings = ctx
        .config
        .settings
        .as_mut()
        .expect("config/test.yaml has a settings section");
    settings["media"]["serve_storage"] = json!(true);
    settings["media"]["root"] = json!(root.to_str().expect("a UTF-8 scratch path"));
    let app = App::before_routes(&ctx).await.expect("before_routes");
    let router = App::routes(&ctx)
        .to_router::<App>(ctx.clone(), app)
        .expect("Loco's middleware stack");
    App::after_routes(router, &ctx).await.expect("after_routes")
}

/// Serves `router` as `App::serve` does (the trailing-slash trim and the
/// client address) on a free loopback port while `client` runs against its
/// origin, then shuts the server down and returns what `client` returned.
///
/// The server and the client run in this task with `tokio::join!`, never
/// `tokio::spawn`, which `clippy.toml` bans everywhere.
async fn while_serving<F, Fut, T>(router: Router, client: F) -> T
where
    F: FnOnce(Origin) -> Fut,
    Fut: Future<Output = T>,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let origin: Origin = format!("http://{}", listener.local_addr().expect("address"))
        .parse()
        .expect("a loopback origin");
    let (stop, stopped) = oneshot::channel::<()>();
    let service = ServiceExt::<AxumRequest>::into_make_service_with_connect_info::<SocketAddr>(
        trim_trailing_slash(router),
    );
    let server = axum::serve(listener, service).with_graceful_shutdown(async {
        let _ = stopped.await;
    });
    let client = async move {
        let value = client(origin).await;
        let _ = stop.send(());
        value
    };
    let (outcome, value) = tokio::join!(server.into_future(), client);
    outcome.expect("the server stops cleanly");
    value
}

/// Sends `request` to `origin`, failing the test on a transport error.
async fn send(transport: &HyperTransport, origin: &Origin, request: &Request) -> Response {
    transport
        .send(origin, request)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
}

/// A transport whose every exchange must finish within five seconds, so a
/// request that hangs fails the test instead of stalling it.
fn transport() -> HyperTransport {
    HyperTransport::new(Duration::from_secs(5))
}

/// Asserts the headers that every `/storage` answer that is not a file
/// carries: the CORS pair, `nosniff`, the sandbox, and no caching and no
/// download.
fn assert_refusal_headers(request: &Request, response: &Response) {
    assert_eq!(
        response.single("access-control-allow-origin"),
        Some("*"),
        "{request}"
    );
    assert_eq!(
        response.single("x-content-type-options"),
        Some("nosniff"),
        "{request}"
    );
    assert!(
        response.single("access-control-expose-headers").is_some(),
        "{request}"
    );
    assert_eq!(
        response.single("content-security-policy"),
        Some(SANDBOX_CSP),
        "{request}"
    );
    assert_eq!(
        response.values("cache-control"),
        Vec::<&str>::new(),
        "{request}"
    );
    assert_eq!(
        response.values("content-disposition"),
        Vec::<&str>::new(),
        "{request}"
    );
}

#[tokio::test]
async fn the_fallback_router_meets_the_media_contract() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        let root = &media.settings().root;
        seed(root).expect("seed the fixture tree");
        let router = fallback_router(&ctx, root).await;
        let report = while_serving(router, |origin| async move {
            run(
                &HyperTransport::new(Duration::from_secs(10)),
                &origin,
                Server::Fallback,
            )
            .await
        })
        .await;
        assert!(report.passed(), "{report}");
        // The suite ran in full: the file loop alone is 3 hosts × 24 files.
        assert!(report.requests > 72, "{report}");
    })
    .await;
}

#[tokio::test]
async fn a_multi_range_request_gets_the_routers_own_416_under_the_sandbox() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        seed(&media.settings().root).expect("seed");
        let router = fallback_router(&ctx, &media.settings().root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            for target in ["/storage/uploads/diaktoll/ujsag_final_2021osz.pdf", TEXT] {
                let request = Request::new(Method::GET, target).header("range", "bytes=0-9,20-29");
                let response = send(&transport, &origin, &request).await;
                assert_eq!(
                    response.status,
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    "{request}"
                );
                // Not the file's type: a PDF's 416 is not the PDF, so it is
                // sandboxed like every other refusal.
                assert_eq!(
                    response.single("content-type"),
                    Some(RANGE_ERROR_TYPE),
                    "{request}"
                );
                assert_eq!(
                    response.body.as_ref(),
                    RANGE_ERROR_BODY.as_bytes(),
                    "{request}"
                );
                let size = if target == TEXT { 300 } else { 4096 };
                assert_eq!(
                    response.single("content-range"),
                    Some(format!("bytes */{size}").as_str()),
                    "{request}"
                );
                assert_refusal_headers(&request, &response);
            }
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn dot_segments_nul_and_invalid_utf8_are_404_for_every_method() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        seed(&media.settings().root).expect("seed");
        let router = fallback_router(&ctx, &media.settings().root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            // Paths the suite does not send: an encoded dot, a `.` segment
            // that nginx would resolve, a NUL and a byte that is not UTF-8.
            for target in [
                "/storage/uploads/%2ehtaccess",
                "/storage/uploads/%2Ehidden/titkos.pdf",
                "/storage/uploads/./docs/jegyzet.txt",
                "/storage/uploads/docs/jegyzet.txt%00",
                "/storage/uploads/docs/%FF.txt",
            ] {
                for method in [Method::GET, Method::HEAD, Method::OPTIONS, Method::POST] {
                    let request = Request::new(method, target);
                    let response = send(&transport, &origin, &request).await;
                    // The guard comes before the method: as nginx's dot-file
                    // location, a refused path is 404 for every method.
                    assert_eq!(response.status, StatusCode::NOT_FOUND, "{request}");
                    assert!(response.body.is_empty(), "{request}");
                    assert_refusal_headers(&request, &response);
                }
            }
            // A HEAD never follows a link either.
            let request = Request::new(Method::HEAD, "/storage/uploads/docs/hivatkozas.pdf");
            let response = send(&transport, &origin, &request).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{request}");
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn an_encoded_slash_separates_segments_as_nginx_reads_it() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        seed(&media.settings().root).expect("seed");
        let router = fallback_router(&ctx, &media.settings().root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            let request = Request::new(Method::GET, "/storage/uploads%2Fdocs%2fjegyzet.txt");
            let response = send(&transport, &origin, &request).await;
            assert_eq!(response.status, StatusCode::OK, "{request}");
            assert_eq!(response.single("content-length"), Some("300"));
            assert!(response.body.starts_with(b"uploads/docs/jegyzet.txt\n"));

            // An encoded prefix never reaches the router: routing reads the
            // raw path, so these get the application's plain 404 for an
            // unknown path, where nginx decodes first and serves the file.
            // No URL shape of ADR 0015 has this form (module docs of
            // src/controllers/storage.rs).
            for target in [
                "/storage%2Fuploads/docs/jegyzet.txt",
                "/%73torage/uploads/docs/jegyzet.txt",
            ] {
                let request = Request::new(Method::GET, target);
                let response = send(&transport, &origin, &request).await;
                assert_eq!(response.status, StatusCode::NOT_FOUND, "{request}");
                assert_eq!(
                    response.single("content-type"),
                    Some("text/plain; charset=utf-8"),
                    "{request}"
                );
                assert_eq!(
                    response.values("access-control-allow-origin"),
                    Vec::<&str>::new(),
                    "{request}"
                );
            }
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn storage_runs_outside_locos_middleware_stack() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        seed(&media.settings().root).expect("seed");
        let router = fallback_router(&ctx, &media.settings().root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            // The control: a route inside the stack gets Loco's request id
            // and the secure headers of the github preset.
            let ping = send(&transport, &origin, &Request::new(Method::GET, "/_ping")).await;
            assert_eq!(ping.status, StatusCode::OK);
            assert!(ping.single("x-request-id").is_some(), "{:?}", ping.headers);
            assert!(
                ping.single("x-frame-options").is_some(),
                "{:?}",
                ping.headers
            );

            let request = Request::new(Method::GET, TEXT).header("accept-encoding", "gzip, br");
            let file = send(&transport, &origin, &request).await;
            assert_eq!(file.status, StatusCode::OK);
            for header in [
                "x-request-id",
                "x-frame-options",
                "strict-transport-security",
                "etag",
                "content-encoding",
                "server",
            ] {
                assert_eq!(file.values(header), Vec::<&str>::new(), "{header}");
            }
            // Uncompressed, so byte ranges stay byte ranges.
            assert_eq!(file.single("content-length"), Some("300"));
            assert_eq!(file.body.len(), 300);
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn every_request_reads_the_volume_again() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        let root = media.settings().root.clone();
        let router = fallback_router(&ctx, &root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            let request = Request::new(Method::GET, "/storage/uploads/docs/uj.pdf");
            let missing = send(&transport, &origin, &request).await;
            assert_eq!(missing.status, StatusCode::NOT_FOUND);
            assert_refusal_headers(&request, &missing);

            fs::create_dir_all(root.join("uploads/docs")).expect("create the folder");
            fs::write(root.join("uploads/docs/uj.pdf"), b"%PDF-1.4\n").expect("write");
            let written = send(&transport, &origin, &request).await;
            assert_eq!(written.status, StatusCode::OK);
            assert_eq!(written.body.as_ref(), b"%PDF-1.4\n");

            fs::remove_file(root.join("uploads/docs/uj.pdf")).expect("remove");
            let removed = send(&transport, &origin, &request).await;
            assert_eq!(removed.status, StatusCode::NOT_FOUND);
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn a_root_that_is_not_mounted_yet_is_404_until_the_volume_arrives() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        let root = media.base().join("not-mounted-yet");
        // The router builds without the root, as production starts before
        // its volume is mounted (ADR 0015 §3).
        let router = fallback_router(&ctx, &root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            let request = Request::new(Method::GET, "/storage/uploads/a.pdf");
            let before = send(&transport, &origin, &request).await;
            assert_eq!(before.status, StatusCode::NOT_FOUND);
            assert_refusal_headers(&request, &before);

            fs::create_dir_all(root.join("uploads")).expect("mount");
            fs::write(root.join("uploads/a.pdf"), b"%PDF").expect("write");
            let after = send(&transport, &origin, &request).await;
            assert_eq!(after.status, StatusCode::OK);
            // The PDF's own answer goes without the sandbox (ADR 0013 §7).
            assert_eq!(after.values("content-security-policy"), Vec::<&str>::new());
        })
        .await;
    })
    .await;
}

/// Opens the write end of a FIFO, without blocking, when dropped, so that a
/// reader stuck in `open(2)` on it returns.
///
/// If the router ever opened the FIFO, its test would otherwise hang until
/// CI's job timeout instead of failing: the stuck open runs on tokio's
/// blocking pool, and the runtime waits for that thread when it shuts down,
/// after the transport's timeout has already failed the test. The guard
/// lives in the test's future, which is dropped before the runtime, panic
/// or not. In the passing case no reader waits, and the open fails with
/// `ENXIO`, which is ignored.
struct UnblockFifo(PathBuf);

impl Drop for UnblockFifo {
    fn drop(&mut self) {
        // The descriptor closes at once, so the released reader sees the
        // end of the file.
        let _ = at::open(
            &self.0,
            OFlags::WRONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        );
    }
}

#[tokio::test]
async fn a_fifo_is_404_at_once_and_never_opened() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        let root = media.settings().root.clone();
        fs::create_dir_all(root.join("uploads")).expect("folder");
        // Opening a FIFO for reading blocks until a writer comes; ServeDir
        // would hang on it. The lookup refuses it from its type alone.
        let fifo = root.join("uploads/cso.pdf");
        at::mknodat(
            at::CWD,
            &fifo,
            FileType::Fifo,
            Mode::from_raw_mode(0o644),
            0,
        )
        .expect("mkfifo");
        let _unblock = UnblockFifo(fifo);
        let router = fallback_router(&ctx, &root).await;
        while_serving(router, |origin| async move {
            for method in [Method::GET, Method::HEAD] {
                let request = Request::new(method, "/storage/uploads/cso.pdf");
                let response = send(&transport(), &origin, &request).await;
                assert_eq!(response.status, StatusCode::NOT_FOUND, "{request}");
            }
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn if_range_keeps_a_range_only_for_the_file_it_names() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        let root = media.settings().root.clone();
        seed(&root).expect("seed");
        let router = fallback_router(&ctx, &root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            let stale = "Mon, 31 Aug 2026 08:00:00 GMT";
            let ranged = |method: Method, range: &str, validator: &str| {
                Request::new(method, TEXT)
                    .header("range", range)
                    .header("if-range", validator)
            };

            // A HEAD is decided the same way, without a body.
            let request = ranged(Method::HEAD, "bytes=0-9", stale);
            let response = send(&transport, &origin, &request).await;
            assert_eq!(response.status, StatusCode::OK, "{request}");
            assert_eq!(response.single("content-length"), Some("300"));
            assert_eq!(response.values("content-range"), Vec::<&str>::new());
            assert!(response.body.is_empty(), "{request}");

            // An unsatisfiable range: 416 while the validator names the
            // file, the whole file once it does not.
            let request = ranged(Method::GET, "bytes=300-", LAST_MODIFIED);
            let response = send(&transport, &origin, &request).await;
            assert_eq!(response.status, StatusCode::RANGE_NOT_SATISFIABLE);
            assert_eq!(response.body.as_ref(), RANGE_ERROR_BODY.as_bytes());
            assert_refusal_headers(&request, &response);
            let request = ranged(Method::GET, "bytes=300-", stale);
            let response = send(&transport, &origin, &request).await;
            assert_eq!(response.status, StatusCode::OK, "{request}");
            assert_eq!(response.body.len(), 300, "{request}");

            // Two If-Range values are no single validator, even if both
            // name the file.
            let request =
                ranged(Method::GET, "bytes=0-9", LAST_MODIFIED).header("if-range", LAST_MODIFIED);
            let response = send(&transport, &origin, &request).await;
            assert_eq!(response.status, StatusCode::OK, "{request}");

            // The file is replaced under its name: the validator the client
            // kept now names the old file, so the new one comes whole, never
            // a slice of it to splice into the old copy.
            let request = ranged(Method::GET, "bytes=0-9", LAST_MODIFIED);
            let before = send(&transport, &origin, &request).await;
            assert_eq!(before.status, StatusCode::PARTIAL_CONTENT);
            assert_eq!(before.body.as_ref(), b"uploads/do");
            let replaced = b"a new file under the old name\n";
            fs::write(root.join("uploads/docs/jegyzet.txt"), replaced).expect("replace");
            let after = send(&transport, &origin, &request).await;
            assert_eq!(after.status, StatusCode::OK, "{request}");
            assert_eq!(after.body.as_ref(), replaced);
            let current = after
                .single("last-modified")
                .expect("ServeDir sends Last-Modified")
                .to_owned();
            assert_ne!(current, LAST_MODIFIED);
            // The new file's own date names it again.
            let request = ranged(Method::GET, "bytes=0-9", &current);
            let response = send(&transport, &origin, &request).await;
            assert_eq!(response.status, StatusCode::PARTIAL_CONTENT, "{request}");
            assert_eq!(response.body.as_ref(), &replaced[..10]);
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn a_lookup_error_is_500_under_the_sandbox_and_never_cached() {
    // A NUL makes every open of the root fail with EINVAL, an error that says
    // nothing about the requested path, as EMFILE or EIO would; neither of
    // those can be caused safely in a test process. The router is built
    // directly, on that root alone.
    let router = router_for(Path::new("/storage-root\0unopenable"));
    while_serving(router, |origin| async move {
        let transport = transport();
        for method in [Method::GET, Method::HEAD] {
            let request = Request::new(method, "/storage/uploads/docs/a.pdf");
            let response = send(&transport, &origin, &request).await;
            assert_eq!(
                response.status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "{request}"
            );
            assert!(response.body.is_empty(), "{request}");
            // Not the PDF: sandboxed like every refusal, and not cached, so
            // the file shows once the error is gone.
            assert_refusal_headers(&request, &response);
        }
    })
    .await;
}

/// The value of the request count with exactly `labels`, if any.
fn request_count(ctx: &AppContext, labels: &str) -> Option<u64> {
    let exposition = ctx
        .shared_store
        .get::<Metrics>()
        .expect("App::after_context stores the metrics")
        .render()
        .expect("render the registry");
    let prefix = format!("http_server_request_duration_seconds_count{{{labels}}} ");
    exposition
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.parse().expect("an integer count"))
}

#[tokio::test]
async fn storage_requests_are_counted_under_their_route_template() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        seed(&media.settings().root).expect("seed");
        let router = fallback_router(&ctx, &media.settings().root).await;
        while_serving(router, |origin| async move {
            let transport = transport();
            for target in [TEXT, TEXT, "/storage/uploads/nincs.pdf", "/storage/"] {
                send(&transport, &origin, &Request::new(Method::GET, target)).await;
            }
        })
        .await;
        let route = ROUTES[2];
        assert_eq!(
            request_count(
                &ctx,
                &format!(r#"method="GET",route="{route}",status="200""#)
            ),
            Some(2)
        );
        assert_eq!(
            request_count(
                &ctx,
                &format!(r#"method="GET",route="{route}",status="404""#)
            ),
            Some(1)
        );
        // `/storage/` arrives trimmed, at the prefix's own template.
        assert_eq!(
            request_count(&ctx, r#"method="GET",route="/storage",status="404""#),
            Some(1)
        );
        assert_eq!(
            request_count(&ctx, r#"method="GET",route="unmatched",status="404""#),
            None
        );
    })
    .await;
}

#[tokio::test]
async fn each_request_is_logged_once_at_info_with_its_status() {
    with_app(|_router, ctx| async move {
        let media = TempMedia::new();
        seed(&media.settings().root).expect("seed");
        let router = trim_trailing_slash(fallback_router(&ctx, &media.settings().root).await);
        let recorder = Arc::new(Recorder::default());
        // The test runs on one thread (tokio::test's runtime), so the
        // thread's default subscriber sees the router's events.
        let _guard = tracing::subscriber::set_default(Shared(Arc::clone(&recorder)));
        for (target, status) in [(TEXT, 200), ("/storage/uploads/.htaccess", 404)] {
            let request = AxumRequest::get(target)
                .body(Body::empty())
                .expect("request");
            let response = router.clone().oneshot(request).await.expect("infallible");
            assert_eq!(response.status().as_u16(), status, "{target}");
        }
        let lines = recorder.lines();
        let finished: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("finished processing request"))
            .collect();
        assert_eq!(finished.len(), 2, "{lines:#?}");
        for (line, (target, status)) in finished
            .iter()
            .zip([(TEXT, 200), ("/storage/uploads/.htaccess", 404)])
        {
            assert!(line.starts_with("storage-request "), "{line}");
            assert!(line.contains(&format!("http.uri={target}")), "{line}");
            assert!(line.contains("http.method=GET"), "{line}");
            assert!(line.contains(" | INFO "), "{line}");
            let status = format!("status={status}");
            assert!(
                line.split_whitespace().any(|token| token == status),
                "{line}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn storage_is_an_unknown_path_while_the_fallback_is_off() {
    // config/test.yaml keeps `serve_storage` off, as production does.
    with_server(|server, _ctx| async move {
        for path in [
            "/storage/uploads/docs/a.pdf",
            "/storage",
            "/storage/posts_images/a.jpg",
        ] {
            let response = server.get(path).await;
            assert_eq!(response.status_code(), StatusCode::NOT_FOUND, "{path}");
            // The plain-text 404 of every unknown path, not the contract's.
            assert_eq!(
                header_text(&response, &CONTENT_TYPE).as_deref(),
                Some("text/plain; charset=utf-8"),
                "{path}"
            );
            assert_eq!(
                header_text(&response, &ACCESS_CONTROL_ALLOW_ORIGIN),
                None,
                "{path}"
            );
        }
    })
    .await;
}

/// What a [`Shared`] subscriber saw: one line per event, `span fields |
/// LEVEL fields`, with the innermost entered span.
#[derive(Default)]
struct Recorder {
    next: AtomicU64,
    spans: Mutex<HashMap<u64, String>>,
    entered: Mutex<Vec<u64>>,
    events: Mutex<Vec<String>>,
}

impl Recorder {
    fn lines(&self) -> Vec<String> {
        self.events.lock().expect("events").clone()
    }
}

/// A minimal subscriber that records every span's name and fields and every
/// event, so a test can read what the router logged without a subscriber
/// crate of its own.
struct Shared(Arc<Recorder>);

/// Appends ` name=value` for each field it visits.
struct Fields<'a>(&'a mut String);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let _ = write!(self.0, " {}={value:?}", field.name());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = write!(self.0, " {}={value}", field.name());
    }
}

impl Subscriber for Shared {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let id = self.0.next.fetch_add(1, Ordering::Relaxed) + 1;
        let mut text = span.metadata().name().to_owned();
        span.record(&mut Fields(&mut text));
        self.0.spans.lock().expect("spans").insert(id, text);
        Id::from_u64(id)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let span = self
            .0
            .entered
            .lock()
            .expect("entered")
            .last()
            .and_then(|id| self.0.spans.lock().expect("spans").get(id).cloned())
            .unwrap_or_default();
        let mut text = format!("{span} | {}", event.metadata().level());
        event.record(&mut Fields(&mut text));
        self.0.events.lock().expect("events").push(text);
    }

    fn enter(&self, span: &Id) {
        self.0
            .entered
            .lock()
            .expect("entered")
            .push(span.into_u64());
    }

    fn exit(&self, _span: &Id) {
        self.0.entered.lock().expect("entered").pop();
    }
}
