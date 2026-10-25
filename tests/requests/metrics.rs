//! `/metrics` and the HTTP request metrics (ADR 0023 §9, PLT-24).
//!
//! - On the public port, `/metrics` is an unknown path like any other
//!   (ADR 0023, "What must be true in code and tests").
//! - Requests through the whole middleware stack are recorded by route
//!   template, normalised method and status, and the exposed family names
//!   are pinned by a snapshot.
//! - The runtime families (database pool, job queue, process) are read at
//!   each scrape; the queue age grows while no worker runs, and a queue that
//!   cannot be read leaves only its own families out. A locked queue table
//!   or a full pool delays a scrape by at most its time limit and leaves the
//!   pool's connections to the application.
//! - `serve_until` serves both listeners from one task, and a shutdown still
//!   runs `on_shutdown` before both stop.

use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use axum::http::{Method, StatusCode};
use backend3::{
    app::{bind_metrics_listener, serve_until},
    controllers::metrics::{
        Metrics, OPENMETRICS_CONTENT_TYPE,
        runtime::{QUEUE_QUERY_TIMEOUT, QUEUE_STATEMENT_TIMEOUT},
    },
    settings::MetricsSettings,
};
use loco_rs::app::AppContext;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

use sea_orm::{ConnectionTrait, TransactionTrait};

use crate::support::{QueueTableLock, create_queue_table, enqueue_job, with_app, with_server};

/// The application's metrics, from the shared store.
fn shared_metrics(ctx: &AppContext) -> Metrics {
    ctx.shared_store
        .get::<Metrics>()
        .expect("App::after_context stores the metrics")
}

/// The exposition of `ctx`'s registry, without refreshing the runtime
/// families.
fn scrape(ctx: &AppContext) -> String {
    shared_metrics(ctx).render().expect("render the registry")
}

/// The exposition that `GET /metrics` serves: the runtime families are read
/// first.
async fn scrape_now(ctx: &AppContext) -> String {
    shared_metrics(ctx)
        .scrape()
        .await
        .expect("render the registry")
}

/// The value of the sample `series` (name and label set exactly as
/// exposed), if the exposition has it.
fn sample(exposition: &str, series: &str) -> Option<f64> {
    exposition
        .lines()
        .find_map(|line| line.strip_prefix(series)?.strip_prefix(' '))
        .map(|value| value.parse().expect("a number"))
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
        // The name ends at the label set or, for a sample without labels,
        // at the space before the value.
        let name_end = line.find(['{', ' ']).unwrap_or(line.len());
        let (name, rest) = line.split_at(name_end);
        let rest = rest.strip_prefix('{').unwrap_or("");
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
/// in the same PR, so every change to what the hub scrapes is reviewed. The
/// maintainer confirmed these families as built on 2026-10-25 (research:
/// DECISIONS R15, D85): `http_server_request_duration_seconds`,
/// `db_client_connections`, `loco_queue_jobs`,
/// `loco_queue_oldest_queued_job_age_seconds`, `process_cpu_seconds_total`
/// and `process_resident_memory_bytes`. That answer does not cover a family
/// a later slice adds.
#[tokio::test]
async fn the_exposed_metric_families_match_the_snapshot() {
    with_server(|request, ctx| async move {
        // As in production, where the Postgres queue creates its table.
        create_queue_table(&ctx).await;
        request.get("/_ping").await.assert_status_ok();
        assert_eq!(
            shape(&scrape_now(&ctx).await),
            "# HELP http_server_request_duration_seconds Time from receiving an HTTP request \
             until its response head was ready (status 499: the connection closed first), by \
             method, matched route template and status code.\n\
             # TYPE http_server_request_duration_seconds histogram\n\
             # UNIT http_server_request_duration_seconds seconds\n\
             # HELP db_client_connections Connections of the application's database pool, by \
             state: used (checked out) or idle.\n\
             # TYPE db_client_connections gauge\n\
             # HELP loco_queue_jobs Jobs in the Postgres job queue table pg_loco_queue, by \
             state.\n\
             # TYPE loco_queue_jobs gauge\n\
             # HELP loco_queue_oldest_queued_job_age_seconds Time since the oldest queued job \
             became due, 0 when none is waiting; it grows while no worker takes jobs.\n\
             # TYPE loco_queue_oldest_queued_job_age_seconds gauge\n\
             # UNIT loco_queue_oldest_queued_job_age_seconds seconds\n\
             # HELP process_cpu_seconds Total user and system CPU time spent in seconds.\n\
             # TYPE process_cpu_seconds counter\n\
             # UNIT process_cpu_seconds seconds\n\
             # HELP process_resident_memory_bytes Resident memory size in bytes.\n\
             # TYPE process_resident_memory_bytes gauge\n\
             # UNIT process_resident_memory_bytes bytes\n\
             # EOF\n\
             db_client_connections{state}\n\
             http_server_request_duration_seconds_bucket{le,method,route,status}\n\
             http_server_request_duration_seconds_count{method,route,status}\n\
             http_server_request_duration_seconds_sum{method,route,status}\n\
             loco_queue_jobs{state}\n\
             loco_queue_oldest_queued_job_age_seconds{}\n\
             process_cpu_seconds_total{}\n\
             process_resident_memory_bytes{}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_stopped_worker_shows_a_growing_oldest_age() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        // No worker runs in the tests, like a process started without
        // `--server-and-worker` (ADR 0016).
        enqueue_job(&ctx, -90).await;
        enqueue_job(&ctx, -5).await;
        // Due in an hour: queued, but not waiting yet.
        enqueue_job(&ctx, 3600).await;

        let age = "loco_queue_oldest_queued_job_age_seconds";
        let first = scrape_now(&ctx).await;
        assert_eq!(
            sample(&first, r#"loco_queue_jobs{state="queued"}"#),
            Some(3.0),
            "{first}"
        );
        for state in ["processing", "completed", "failed", "cancelled"] {
            let series = format!(r#"loco_queue_jobs{{state="{state}"}}"#);
            assert_eq!(sample(&first, &series), Some(0.0), "{state}\n{first}");
        }
        let first_age = sample(&first, age).expect("an age sample");
        assert!((90.0..150.0).contains(&first_age), "{first}");

        tokio::time::sleep(Duration::from_millis(100)).await;
        let second = scrape_now(&ctx).await;
        let second_age = sample(&second, age).expect("an age sample");
        assert!(
            second_age >= first_age + 0.09,
            "the age grows with the wait: {first_age} then {second_age}"
        );
    })
    .await;
}

#[tokio::test]
async fn an_empty_queue_has_zero_jobs_and_zero_age() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        let body = scrape_now(&ctx).await;
        assert_eq!(
            sample(&body, "loco_queue_oldest_queued_job_age_seconds"),
            Some(0.0),
            "{body}"
        );
        assert_eq!(
            sample(&body, r#"loco_queue_jobs{state="queued"}"#),
            Some(0.0),
            "{body}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_queue_that_cannot_be_read_leaves_only_its_own_families_out() {
    // config/test.yaml has no queue, so this database has no queue table.
    with_app(|_router, ctx| async move {
        let body = scrape_now(&ctx).await;
        assert!(!body.contains("loco_queue"), "{body}");
        assert!(
            sample(&body, r#"db_client_connections{state="idle"}"#).is_some(),
            "{body}"
        );
        assert!(
            sample(&body, "process_cpu_seconds_total").is_some_and(|seconds| seconds > 0.0),
            "{body}"
        );
        assert!(
            sample(&body, "process_resident_memory_bytes").is_some_and(|bytes| bytes > 0.0),
            "{body}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_checked_out_connection_counts_as_used() {
    with_app(|_router, ctx| async move {
        // A transaction holds one connection of the pool until it ends.
        let transaction = ctx.db.begin().await.expect("begin");
        let body = scrape_now(&ctx).await;
        let used = sample(&body, r#"db_client_connections{state="used"}"#).expect("used");
        let idle = sample(&body, r#"db_client_connections{state="idle"}"#).expect("idle");
        assert!(used >= 1.0, "{body}");
        // config/test.yaml caps the pool at 2 connections.
        assert!(used + idle <= 2.0, "{body}");
        transaction.rollback().await.expect("rollback");

        // sqlx returns a connection to the pool from a background task, after
        // a ping, so it counts as idle again shortly after the rollback.
        timeout(Duration::from_secs(5), async {
            loop {
                let body = scrape_now(&ctx).await;
                if sample(&body, r#"db_client_connections{state="used"}"#) == Some(0.0) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the connection is idle again within 5 s");
    })
    .await;
}

/// A scrape like `GET /metrics`, and how long it took.
async fn timed_scrape(ctx: &AppContext) -> (String, Duration) {
    let started = Instant::now();
    let body = scrape_now(ctx).await;
    (body, started.elapsed())
}

#[tokio::test]
async fn scrapes_of_a_locked_queue_table_keep_the_pool_free() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        let lock = QueueTableLock::take(&ctx).await;

        // More scrapes at once than the pool has connections (2 in
        // config/test.yaml). One reads the queue; the others find that read
        // running and leave the queue out without waiting.
        let (a, b, c) = tokio::join!(timed_scrape(&ctx), timed_scrape(&ctx), timed_scrape(&ctx));
        let mut took = Vec::new();
        for (body, elapsed) in [a, b, c] {
            assert!(!body.contains("loco_queue"), "{body}");
            assert!(
                sample(&body, "process_cpu_seconds_total").is_some(),
                "{body}"
            );
            took.push(elapsed);
        }
        took.sort();
        assert!(took[1] < QUEUE_STATEMENT_TIMEOUT / 2, "{took:?}");
        // PostgreSQL cancelled the read before the scrape would have given
        // up on it, so the read's connection did not stay checked out.
        assert!(
            (QUEUE_STATEMENT_TIMEOUT..QUEUE_QUERY_TIMEOUT).contains(&took[2]),
            "{took:?}"
        );
        let (body, elapsed) = timed_scrape(&ctx).await;
        assert!(!body.contains("loco_queue"), "{body}");
        assert!(elapsed < QUEUE_QUERY_TIMEOUT, "{elapsed:?}");

        // The application still gets a connection at once.
        timeout(
            Duration::from_secs(1),
            ctx.db.execute_unprepared("SELECT 1"),
        )
        .await
        .expect("a connection within 1 s")
        .expect("SELECT 1");
        lock.release().await;
        let body = scrape_now(&ctx).await;
        assert_eq!(
            sample(&body, r#"loco_queue_jobs{state="queued"}"#),
            Some(0.0),
            "{body}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_scrape_while_every_connection_is_busy_answers_in_time() {
    with_app(|_router, ctx| async move {
        create_queue_table(&ctx).await;
        // config/test.yaml caps the pool at 2 connections; both are taken.
        let first = ctx.db.begin().await.expect("begin");
        let second = ctx.db.begin().await.expect("begin");

        let (body, took) = timed_scrape(&ctx).await;
        // The scrape's own limit, not the pool's longer acquire timeout.
        assert!(
            (QUEUE_QUERY_TIMEOUT..QUEUE_QUERY_TIMEOUT + Duration::from_secs(1)).contains(&took),
            "{took:?}"
        );
        assert!(!body.contains("loco_queue"), "{body}");
        assert_eq!(
            sample(&body, r#"db_client_connections{state="used"}"#),
            Some(2.0),
            "{body}"
        );
        assert_eq!(
            sample(&body, r#"db_client_connections{state="idle"}"#),
            Some(0.0),
            "{body}"
        );
        first.rollback().await.expect("rollback");
        second.rollback().await.expect("rollback");
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
