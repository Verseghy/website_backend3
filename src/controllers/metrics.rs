//! `/metrics` on a port of its own, and the HTTP request metrics
//! (ADR 0023 §9, PLT-24).
//!
//! `App::after_context` creates one `prometheus_client` [`Registry`] per
//! application context, never a global one, registers every family on it and
//! stores the [`Metrics`] in `ctx.shared_store`, from where the hooks hand
//! the pieces down (ADR 0008, rules 2 and 3). Families are registered only
//! there, so the registry is then frozen in an `Arc` and a scrape takes no
//! lock against registration. Only [`router`] serves it, on the
//! `settings.metrics` listener; the application router has no `/metrics`
//! route, and no Route targets the metrics port (ADR 0014). Label values
//! come from sets the code bounds (`crate::domain::http_metrics`).
//!
//! [`with_http_metrics`] records every request in one histogram,
//! `http_server_request_duration_seconds{method, route, status}`; its
//! `_count` series is the request counter, so there is no separate counter.
//!
//! [`runtime`] adds the families that are read at scrape time instead: the
//! database pool, the job queue and the process. Each scrape refreshes them
//! first ([`Metrics::scrape`]).

pub mod runtime;

use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Router as AxumRouter,
    extract::{MatchedPath, Request, State},
    http::{Method, StatusCode, header::CONTENT_TYPE},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use prometheus_client::{
    encoding::{EncodeLabelSet, text::encode},
    metrics::{family::Family, histogram::Histogram},
    registry::{Registry, Unit},
};

use self::runtime::RuntimeMetrics;
use crate::domain::http_metrics::{
    CLIENT_CLOSED_STATUS, LATENCY_BUCKETS_SECONDS, method_label, route_label,
};

/// The path of the scrape endpoint on the metrics port.
pub const METRICS_PATH: &str = "/metrics";

/// The `Content-Type` of a scrape: the `OpenMetrics` 1.0 text format, which
/// is what `prometheus_client` writes and Prometheus reads.
pub const OPENMETRICS_CONTENT_TYPE: &str =
    "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// The name of the request-latency family as registered; the registry adds
/// the unit, so it is exposed as `http_server_request_duration_seconds`.
const HTTP_REQUEST_DURATION: &str = "http_server_request_duration";

/// Every metric family of backend3, with the registry that exposes them.
///
/// Cloning is cheap: the registry is shared, and the family handles are
/// shared references to the same series.
#[derive(Clone, Debug)]
pub struct Metrics {
    registry: Arc<Registry>,
    http: HttpMetrics,
    runtime: RuntimeMetrics,
}

impl Metrics {
    /// Freezes a registry together with the handles of the families
    /// registered on it.
    ///
    /// - `registry`: the registry every family below was registered on.
    /// - `http`: the HTTP request families, from [`HttpMetrics::register`].
    /// - `runtime`: the scrape-time families, from
    ///   [`RuntimeMetrics::register`].
    #[must_use]
    pub fn new(registry: Registry, http: HttpMetrics, runtime: RuntimeMetrics) -> Self {
        Self {
            registry: Arc::new(registry),
            http,
            runtime,
        }
    }

    /// The HTTP request families, for [`with_http_metrics`].
    #[must_use]
    pub fn http(&self) -> &HttpMetrics {
        &self.http
    }

    /// Writes every family in the `OpenMetrics` text format, ending with the
    /// `# EOF` line. Families that have no series yet are left out, and the
    /// runtime families show their last refresh ([`Metrics::scrape`]), or
    /// nothing before the first.
    ///
    /// # Errors
    ///
    /// Returns [`fmt::Error`] when a family cannot be written in the text
    /// format; writing into the `String` itself cannot fail.
    pub fn render(&self) -> Result<String, fmt::Error> {
        let mut body = String::new();
        encode(&mut body, &self.registry)?;
        Ok(body)
    }

    /// Refreshes the runtime families ([`RuntimeMetrics::refresh`]), then
    /// writes every family like [`Metrics::render`]. This is what
    /// `GET /metrics` serves.
    ///
    /// # Errors
    ///
    /// Returns [`fmt::Error`] as [`Metrics::render`] does. A runtime source
    /// that cannot be read is no error: its families are left out.
    pub async fn scrape(&self) -> Result<String, fmt::Error> {
        self.runtime.refresh().await;
        self.render()
    }
}

/// The labels of one HTTP request series. Field order is label order in the
/// exposition.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct HttpLabels {
    method: &'static str,
    route: String,
    status: u16,
}

/// The constructor of each series' histogram, with backend3's buckets.
fn latency_histogram() -> Histogram {
    Histogram::new(LATENCY_BUCKETS_SECONDS)
}

/// The HTTP request families.
#[derive(Clone, Debug)]
pub struct HttpMetrics {
    requests: Family<HttpLabels, Histogram, fn() -> Histogram>,
}

impl HttpMetrics {
    /// Registers the HTTP request families on `registry` and returns their
    /// handles.
    ///
    /// - `registry`: the application's registry, still being built in
    ///   `App::after_context`.
    #[must_use]
    pub fn register(registry: &mut Registry) -> Self {
        let requests = Family::new_with_constructor(latency_histogram as fn() -> Histogram);
        registry.register_with_unit(
            HTTP_REQUEST_DURATION,
            "Time from receiving an HTTP request until its response head was ready (status \
             499: the connection closed first), by method, matched route template and status \
             code",
            Unit::Seconds,
            requests.clone(),
        );
        Self { requests }
    }

    /// Records one request.
    ///
    /// - `method`: the request method; non-standard methods share one label.
    /// - `matched_template`: the route template the router matched, or
    ///   `None` when the fallback answered.
    /// - `status`: the response status code, or [`CLIENT_CLOSED_STATUS`].
    /// - `elapsed`: the time the request took.
    pub fn observe(
        &self,
        method: &Method,
        matched_template: Option<&str>,
        status: u16,
        elapsed: Duration,
    ) {
        let labels = HttpLabels {
            method: method_label(method.as_str()),
            route: route_label(matched_template).to_owned(),
            status,
        };
        self.requests
            .get_or_create(&labels)
            .observe(elapsed.as_secs_f64());
    }
}

/// Wraps `router` in the layer that records every request in `http`.
///
/// - `router`: the finished application router.
/// - `http`: the families to record into, from [`Metrics::http`].
///
/// `App::after_routes` calls it, after Loco's middleware stack has been
/// applied, so the layer is the outermost one around each route: it sees
/// the status that Loco's middlewares produce (a caught panic's `500`, for
/// example) and times them too. axum runs a router's layers only after
/// routing, which is what makes the matched template available here; the
/// fallback is wrapped as well and records [`route_label`]'s unmatched
/// value. The time ends when the response head is ready, so a streamed body
/// (a large file under `/admin`) is not included. A request whose
/// connection closes before that is still recorded, as
/// [`CLIENT_CLOSED_STATUS`] (see `InFlight`).
pub fn with_http_metrics(router: AxumRouter, http: HttpMetrics) -> AxumRouter {
    router.layer(middleware::from_fn_with_state(http, record_request))
}

/// One request in flight; dropping it records the request.
///
/// When the client, or the router in front of backend3 at its timeout,
/// closes the connection, hyper drops the request's future before it
/// finishes, so code after the `await` in [`record_request`] never runs.
/// The guard is dropped either way, and records a request that has no
/// `status` yet as [`CLIENT_CLOSED_STATUS`]. Without it, the slowest
/// requests would be missing from the latency histogram.
struct InFlight {
    http: HttpMetrics,
    method: Method,
    matched: Option<MatchedPath>,
    started: Instant,
    status: Option<StatusCode>,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.http.observe(
            &self.method,
            self.matched.as_ref().map(MatchedPath::as_str),
            self.status
                .map_or(CLIENT_CLOSED_STATUS, |status| status.as_u16()),
            self.started.elapsed(),
        );
    }
}

/// The middleware behind [`with_http_metrics`].
async fn record_request(
    State(http): State<HttpMetrics>,
    matched: Option<MatchedPath>,
    request: Request,
    next: Next,
) -> Response {
    let mut in_flight = InFlight {
        http,
        method: request.method().clone(),
        matched,
        // Not "now" from the app clock: this monotonic reading only measures
        // the request's duration and never becomes an instant, which is why
        // `tests/architecture.rs` allows `Instant::now` outside `src/domain/`.
        started: Instant::now(),
        status: None,
    };
    let response = next.run(request).await;
    in_flight.status = Some(response.status());
    response
}

/// The router of the metrics port: `GET /metrics` and nothing else, without
/// Loco's middleware stack. Its requests are not counted in the HTTP
/// families, so scrapes do not show up in the application's traffic.
///
/// - `metrics`: the application's metrics, from the shared store.
pub fn router(metrics: Metrics) -> AxumRouter {
    AxumRouter::new()
        .route(METRICS_PATH, get(scrape))
        .with_state(metrics)
}

/// `GET /metrics` on the metrics port.
async fn scrape(State(metrics): State<Metrics>) -> Response {
    match metrics.scrape().await {
        Ok(body) => ([(CONTENT_TYPE, OPENMETRICS_CONTENT_TYPE)], body).into_response(),
        Err(error) => {
            tracing::error!(%error, "cannot encode the metrics registry");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;

    use axum::body::Body;
    use futures_util::FutureExt as _;
    use prometheus_client::{
        encoding::{EncodeMetric, MetricEncoder},
        metrics::MetricType,
    };
    use tower::ServiceExt as _;

    use super::*;

    /// A registry with the HTTP families on it and whatever `extra` adds.
    fn metrics_with(extra: impl FnOnce(&mut Registry)) -> Metrics {
        let mut registry = Registry::default();
        let http = HttpMetrics::register(&mut registry);
        // A disconnected database: the runtime families read nothing from
        // it, and these tests need no PostgreSQL.
        let runtime =
            RuntimeMetrics::register(&mut registry, sea_orm::DatabaseConnection::default());
        extra(&mut registry);
        Metrics::new(registry, http, runtime)
    }

    #[test]
    fn a_request_whose_connection_closes_is_recorded_as_client_closed() {
        let metrics = metrics_with(|_| {});
        let slow = AxumRouter::new().route("/slow", get(pending::<()>));
        let request = Request::get("/slow").body(Body::empty()).expect("request");
        // One poll reaches the handler, which never finishes; dropping the
        // future then is what hyper does when the connection closes.
        let abandoned = with_http_metrics(slow, metrics.http().clone()).oneshot(request);
        assert!(abandoned.now_or_never().is_none());
        let body = metrics.render().expect("render");
        let series = r#"_count{method="GET",route="/slow",status="499"} 1"#;
        assert!(body.contains(series), "{body}");
    }

    /// A metric that cannot be encoded, like a collector whose source fails.
    #[derive(Debug)]
    struct Unencodable;

    impl EncodeMetric for Unencodable {
        fn encode(&self, _encoder: MetricEncoder) -> Result<(), fmt::Error> {
            Err(fmt::Error)
        }

        fn metric_type(&self) -> MetricType {
            MetricType::Gauge
        }
    }

    // A runtime, not `now_or_never`: the scrape refreshes the runtime
    // families first, and the queue query's timeout needs tokio's timer.
    #[tokio::test]
    async fn a_registry_that_cannot_be_encoded_is_a_500() {
        let metrics = metrics_with(|registry| registry.register("broken", "Never", Unencodable));
        let request = Request::get(METRICS_PATH)
            .body(Body::empty())
            .expect("request");
        let response = router(metrics).oneshot(request).await;
        assert_eq!(
            response.expect("infallible").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn a_scrape_without_a_database_still_serves_the_other_families() {
        let metrics = metrics_with(|_| {});
        let request = Request::get(METRICS_PATH)
            .body(Body::empty())
            .expect("request");
        let response = router(metrics).oneshot(request).await.expect("infallible");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body = String::from_utf8(body.to_vec()).expect("UTF-8");
        assert!(body.contains("\nprocess_cpu_seconds_total "), "{body}");
        assert!(!body.contains("db_client_connections"), "{body}");
        assert!(!body.contains("loco_queue"), "{body}");
    }
}
