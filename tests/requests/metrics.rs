//! `/metrics` and the HTTP request metrics (ADR 0023 §9, PLT-24).
//!
//! - On the public port, `/metrics` is an unknown path like any other
//!   (ADR 0023, "What must be true in code and tests").
//! - Requests through the whole middleware stack are recorded by route
//!   template, normalised method and status, and the exposed family names
//!   are pinned by a snapshot.
//! - `serve_until` serves both listeners from one task, and a shutdown still
//!   runs `on_shutdown` before both stop.

use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::http::{Method, StatusCode};
use backend3::{
    app::{bind_metrics_listener, serve_until},
    controllers::metrics::{Metrics, OPENMETRICS_CONTENT_TYPE},
    settings::MetricsSettings,
};
use loco_rs::app::AppContext;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

use crate::support::{with_app, with_server};

/// The application's metrics, from the shared store.
fn shared_metrics(ctx: &AppContext) -> Metrics {
    ctx.shared_store
        .get::<Metrics>()
        .expect("App::after_context stores the metrics")
}

/// The exposition of `ctx`'s registry.
fn scrape(ctx: &AppContext) -> String {
    shared_metrics(ctx).render().expect("render the registry")
}

/// The value of the `_count` series with exactly these labels, if any.
fn request_count(exposition: &str, labels: &str) -> Option<u64> {
    let prefix = format!("http_server_request_duration_seconds_count{{{labels}}} ");
    exposition
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.parse().expect("integer count"))
}

#[tokio::test]
async fn metrics_is_not_served_on_the_public_port() {
    with_server(|request, _ctx| async move {
        for path in ["/metrics", "/metrics/", "/_metrics"] {
            let response = request.get(path).await;
            assert_eq!(response.status_code(), StatusCode::NOT_FOUND, "{path}");
            // The fallback's plain 404 for paths outside both APIs, the same
            // answer as for any other unknown path.
            assert_eq!(response.text(), "Not Found\n", "{path}");
            assert_ne!(
                response.header("content-type"),
                OPENMETRICS_CONTENT_TYPE,
                "{path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn requests_are_recorded_by_route_template_method_and_status() {
    with_server(|request, ctx| async move {
        let brew = Method::from_bytes(b"BREW").expect("extension method");
        request.get("/_ping").await.assert_status_ok();
        request.get("/_ping").await.assert_status_ok();
        request.get("/_ping/").await.assert_status_ok();
        request.post("/_ping").await;
        request.method(brew, "/_ping").await;
        request.get("/no/such/path").await.assert_status_not_found();
        request.get("/api/v1/does-not-exist").await;
        request.get("/admin/posts/12").await.assert_status_ok();

        let body = scrape(&ctx);
        let expected = [
            // The trailing slash is trimmed before routing, so it counts as
            // the same route.
            (r#"method="GET",route="/_ping",status="200""#, 3),
            (r#"method="POST",route="/_ping",status="405""#, 1),
            (r#"method="other",route="/_ping",status="405""#, 1),
            (r#"method="GET",route="unmatched",status="404""#, 2),
            (r#"method="GET",route="/admin/{*path}",status="200""#, 1),
        ];
        for (labels, count) in expected {
            assert_eq!(
                request_count(&body, labels),
                Some(count),
                "{labels}\n{body}"
            );
        }
        for raw in ["/no/such/path", "does-not-exist", "/admin/posts", "BREW"] {
            assert!(!body.contains(raw), "{raw} leaked into a label:\n{body}");
        }
    })
    .await;
}

/// The descriptor lines, and each sample's name with its label *keys*: the
/// shape a dashboard or alert depends on, without the values that vary.
fn shape(exposition: &str) -> String {
    let mut descriptors = Vec::new();
    let mut samples = BTreeSet::new();
    for line in exposition.lines() {
        if line.starts_with('#') {
            descriptors.push(line.to_owned());
            continue;
        }
        let (name, rest) = line.split_once('{').unwrap_or((line, ""));
        let keys: Vec<&str> = rest
            .split(',')
            .filter_map(|pair| pair.split_once('=').map(|(key, _)| key))
            .collect();
        samples.insert(format!("{name}{{{}}}", keys.join(",")));
    }
    descriptors.extend(samples);
    descriptors.join("\n")
}

/// The metric-name snapshot. A slice that adds a family adds its lines here
/// in the same PR, so every change to what the hub scrapes is reviewed.
#[tokio::test]
async fn the_exposed_metric_families_match_the_snapshot() {
    with_server(|request, ctx| async move {
        request.get("/_ping").await.assert_status_ok();
        assert_eq!(
            shape(&scrape(&ctx)),
            "# HELP http_server_request_duration_seconds Time from receiving an HTTP request \
             until its response head was ready (status 499: the connection closed first), by \
             method, matched route template and status code.\n\
             # TYPE http_server_request_duration_seconds histogram\n\
             # UNIT http_server_request_duration_seconds seconds\n\
             # EOF\n\
             http_server_request_duration_seconds_bucket{le,method,route,status}\n\
             http_server_request_duration_seconds_count{method,route,status}\n\
             http_server_request_duration_seconds_sum{method,route,status}"
        );
    })
    .await;
}

#[tokio::test]
async fn every_app_context_has_its_own_registry() {
    with_server(|request, ctx| async move {
        request.get("/_ping").await.assert_status_ok();
        assert!(scrape(&ctx).contains(r#"route="/_ping""#));
    })
    .await;
    with_app(|_router, ctx| async move {
        assert_eq!(scrape(&ctx), "# EOF\n", "a second app starts at zero");
    })
    .await;
}

/// One HTTP/1.1 GET over a fresh connection that the server closes after
/// the response, so a graceful shutdown never waits for it.
async fn http_get(address: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).await.expect("connect");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("send the request");
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .await
        .expect("read the response");
    let status = raw
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {raw:?}"));
    (status, raw)
}

#[tokio::test]
async fn serve_until_serves_both_ports_and_still_runs_on_shutdown() {
    with_app(|router, ctx| async move {
        let http = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let metrics_listener = bind_metrics_listener(&loopback(0))
            .await
            .expect("bind a free port");
        let http_address = http.local_addr().expect("address");
        let metrics_address = metrics_listener.local_addr().expect("address");
        let (stop, stopped) = oneshot::channel::<()>();
        let hook_ran = Arc::new(AtomicBool::new(false));
        let hook = {
            let hook_ran = Arc::clone(&hook_ran);
            async move { hook_ran.store(true, Ordering::SeqCst) }
        };

        let server = serve_until(
            http,
            router,
            metrics_listener,
            shared_metrics(&ctx),
            async move {
                let _ = stopped.await;
            },
            hook,
        );
        let client = async {
            let (status, _) = http_get(http_address, "/_ping").await;
            assert_eq!(status, 200, "the app on the HTTP port");
            let (status, _) = http_get(http_address, "/metrics").await;
            assert_eq!(status, 404, "no /metrics on the HTTP port");

            let (status, scrape) = http_get(metrics_address, "/metrics").await;
            assert_eq!(status, 200, "{scrape}");
            assert!(
                scrape.contains(&format!("content-type: {OPENMETRICS_CONTENT_TYPE}\r\n")),
                "{scrape}"
            );
            assert!(
                scrape.contains(r#"route="/_ping",status="200""#),
                "{scrape}"
            );
            assert!(
                scrape.contains(r#"route="unmatched",status="404""#),
                "{scrape}"
            );
            let (status, _) = http_get(metrics_address, "/_ping").await;
            assert_eq!(status, 404, "only /metrics on the metrics port");

            assert!(!hook_ran.load(Ordering::SeqCst), "not before the signal");
            stop.send(()).expect("the server is still waiting");
        };

        // A bound, so that a server that ignores the signal fails the test
        // instead of hanging the job.
        let (outcome, ()) = timeout(Duration::from_secs(10), async {
            tokio::join!(server, client)
        })
        .await
        .expect("serve_until did not return within 10 s of the shutdown signal");
        outcome.expect("both servers stop cleanly");
        assert!(
            hook_ran.load(Ordering::SeqCst),
            "on_shutdown ran before serve_until returned"
        );
    })
    .await;
}

/// Metrics settings on the IPv4 loopback address.
fn loopback(port: u16) -> MetricsSettings {
    MetricsSettings {
        binding: Ipv4Addr::LOCALHOST.into(),
        port,
    }
}

#[tokio::test]
async fn a_taken_metrics_port_is_refused_with_the_setting_keys() {
    let taken = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = taken.local_addr().expect("address").port();
    let message = bind_metrics_listener(&loopback(port))
        .await
        .expect_err("the port is taken")
        .to_string();
    let expected = format!(
        "Refusing to start: cannot listen for /metrics on 127.0.0.1:{port} \
         (settings.metrics.binding, settings.metrics.port; the port must be free): "
    );
    assert!(message.starts_with(&expected), "{message}");
}
