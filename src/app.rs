//! The Loco hooks of backend3: how the application loads its configuration,
//! builds its router and registers workers and tasks.
//!
//! Seven hooks carry project decisions:
//!
//! - `after_context` registers the app clock, the one source of "now"
//!   (ADR 0010 point 7). Handlers, workers and tasks read it with [`clock`]
//!   and pass the instant down; tests replace it with [`set_clock`]. It also
//!   creates the metrics registry, registers every metric family on it and
//!   puts the [`Metrics`] into the shared store, from where the hooks below
//!   hand it down (ADR 0023 §9, ADR 0008 rule 3).
//! - `load_config` runs the database-target guard of ADR 0024 §4 (layer 3)
//!   and validates the typed settings, including that `settings.metrics.port`
//!   is not `server.port`, and the admin session's `auth.jwt` section
//!   (ADR 0006 §1). In production it also refuses every public throwaway key
//!   of the local configurations (ADR 0034 §3). Every Loco entry point calls
//!   it before it connects anywhere: the test boot helpers, `cargo loco` and
//!   the `backend3-cli` binary.
//! - `before_routes` mounts the `/admin` router and the JSON `404` fallback.
//!   In loco-rs 1.2.0 the router returned here receives Loco's routes and is
//!   then wrapped by Loco's whole middleware stack, so `/admin` and the
//!   fallback get the same request id, logging, compression and secure
//!   headers as the API (`boot::setup_routes`). A router added in
//!   `after_routes` would get none of them (ADR 0004 §3).
//! - `routes` adds every `/api/v1` route through one builder,
//!   `controllers::v1::routes`, which puts the CORS layer on those routes
//!   only, so `/admin` and `/api/admin` never send CORS headers (ADR 0014).
//! - `middlewares` keeps Loco's default stack and puts
//!   [`NotModifiedCors`] right outside its `etag` middleware, so a `304`
//!   under `/api/v1` carries the CORS header that the `etag` middleware drops
//!   (public-api.md §9, ADR 0014).
//! - `after_routes` wraps the finished router, middleware stack included, in
//!   the HTTP request metrics layer.
//! - `serve` puts [`trim_trailing_slash`] in front of the finished router,
//!   so `/api/v1/posts/` is served exactly like `/api/v1/posts`
//!   (public-api.md DEC-10, open item O-2), and runs the `/metrics` listener
//!   of `settings.metrics` next to it ([`serve_until`]).
//!
//! The session `TimeZone = UTC` of ADR 0002 ("Session settings") and ADR 0010
//! rule 9 needs no hook: every connection backend3 opens goes through
//! sqlx-postgres (the Sea-ORM pool and Loco's Postgres queue), which sends
//! `TimeZone=UTC` as a startup parameter of every connection
//! (sqlx-postgres 0.9, `connection/establish.rs`). PostgreSQL applies startup
//! parameters after the `options` switches and above database and role
//! defaults, so nothing in the server's configuration can change it.
//! `tests/boot/session_settings.rs` pins this, so a driver change that drops
//! the parameter fails the tests instead of going unnoticed.
//!
//! The hooks that generators extend (`routes`, `connect_workers`,
//! `register_tasks`) keep the shapes `cargo loco generate` injects into, and
//! the names the injected lines use are imported below even before anything
//! uses them.

use std::{future::IntoFuture, net::SocketAddr, path::Path, sync::Arc};

use async_trait::async_trait;
use axum::{Router as AxumRouter, ServiceExt, extract::Request};
use chrono::{DateTime, Utc};
// `cargo loco generate worker` injects
// `queue.register(crate::workers::<name>::Worker::build(ctx)).await?;` into
// `connect_workers` (loco-gen 1.2.0, `templates/worker/worker.t`). `build` is
// a method of the `BackgroundWorker` trait, so the trait must be in scope
// before the first worker exists, as in Loco's own `app.rs` template.
#[allow(
    unused_imports,
    reason = "used by the lines `cargo loco generate worker` injects into connect_workers"
)]
use loco_rs::bgworker::BackgroundWorker;
use loco_rs::{
    Error, Result,
    app::{AppContext, Hooks, SharedStore},
    bgworker::Queue,
    boot::{BootResult, ServeParams, StartMode, create_app, shutdown_signal},
    config::{Config, QueueConfig},
    controller::{
        AppRoutes,
        middleware::{self, MiddlewareLayer, MiddlewareStackExt as _},
    },
    environment::Environment,
    task::Tasks,
};
use migration::Migrator;
use prometheus_client::registry::Registry;
use tokio::{net::TcpListener, sync::watch};
use tower::Layer;
use tower_http::normalize_path::{NormalizePath, NormalizePathLayer};

// `cargo loco generate task` injects `tasks.register(tasks::<name>::<Task>);`
// above the marker comment in `register_tasks` (loco-gen 1.2.0,
// `templates/task/task.t`). The path `tasks::...` names this module, not the
// `tasks` parameter (paths resolve in the type namespace), so it must be in
// scope before the first task exists, as in Loco's own `app.rs` template.
#[allow(
    unused_imports,
    reason = "used by the lines `cargo loco generate task` injects into register_tasks"
)]
use crate::tasks;
use crate::{
    controllers::{
        self,
        admin_ui::{self, DistRequirement},
        metrics::{self, HttpMetrics, Metrics, runtime::RuntimeMetrics},
        not_found,
        v1::caching::NotModifiedCors,
    },
    domain::{
        clock::Clock,
        database_target::{self, Connection},
    },
    settings::{
        METRICS_BINDING_KEY, METRICS_PORT_KEY, MetricsSettings, Settings, check_session_jwt,
    },
};

/// The Loco application.
pub struct App;

/// The clock of a running backend3: the operating system's. Its `now` is
/// the only read of the system time in `src/`; `tests/architecture.rs` allows
/// one nowhere else. The type is private, so nothing can bypass [`clock`] by
/// naming it.
#[derive(Debug, Clone, Copy)]
struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The shared-store entry of the app clock. `SharedStore` is keyed by type,
/// and this one is private, so only [`clock`] and [`set_clock`] reach it. A
/// read clones the `Arc` and releases the store's lock at once, so no guard
/// is held across an `.await`.
#[derive(Debug, Clone)]
struct AppClock(Arc<dyn Clock>);

/// Returns the clock of this app context, the one source of "now"
/// (ADR 0010 point 7, public-api.md §16.2).
///
/// - `ctx`: the app context of the request, worker or task.
///
/// Controllers, workers and tasks call it once per request or job run and
/// pass `.now()` down (ADR 0008 rules 1-3). In production it is the system
/// clock; tests replace it with [`set_clock`] (`tests/support::with_app_at`).
///
/// # Errors
///
/// [`Error::Message`] when the context carries no clock: every context Loco
/// builds has one (`App::after_context`), one built by hand does not.
pub fn clock(ctx: &AppContext) -> Result<Arc<dyn Clock>> {
    clock_in(&ctx.shared_store)
}

/// Replaces the clock of an app context.
///
/// - `ctx`: the context; every copy of it sees the new clock, including the
///   copies the router, workers and tasks took at boot (they share one
///   `SharedStore` through an `Arc`).
/// - `clock`: the clock that [`clock`] returns from now on.
///
/// Only `App::after_context` and `tests/support` call it; it is public because
/// the integration tests are a crate of their own, and `tests/architecture.rs`
/// fails on a call anywhere else in `src/`.
pub fn set_clock(ctx: &AppContext, clock: Arc<dyn Clock>) {
    set_clock_in(&ctx.shared_store, clock);
}

/// The clock held in `store`, or the error that [`clock`] documents.
fn clock_in(store: &SharedStore) -> Result<Arc<dyn Clock>> {
    store
        .get::<AppClock>()
        .map(|AppClock(clock)| clock)
        .ok_or_else(|| {
            Error::string(
                "this AppContext carries no app clock: App::after_context registers it in the \
                 shared store, so build the context through Loco's boot (create_context) or call \
                 backend3::app::set_clock on it first",
            )
        })
}

/// Stores `clock` in `store`, replacing any clock already there.
fn set_clock_in(store: &SharedStore, clock: Arc<dyn Clock>) {
    store.insert(AppClock(clock));
}

/// Wraps the finished application router so that trailing slashes are
/// trimmed from the request path *before* routing (public-api.md DEC-10:
/// `/api/v1/posts/` is served exactly like `/api/v1/posts`, with no
/// redirect).
///
/// - `router`: the router Loco built, with every route, the `/admin` router,
///   the fallback and Loco's middleware stack already applied.
///
/// Why outside the router: axum runs layers added with `Router::layer` only
/// after a route has matched, so a path rewrite there would come too late;
/// wrapping the router itself is the pattern axum documents for rewriting the
/// URI. tower-http trims every trailing `/` and collapses leading `//` to
/// `/`, keeps the query, and leaves `/` alone.
///
/// The admin UI is unaffected (public-api.md O-2): `/admin/` and `/admin`
/// both answer with `index.html`, and a client route reads the same with or
/// without the slash. [`App::serve`] uses this function, and the request
/// tests put the same wrapper in front of their test server.
#[must_use]
pub fn trim_trailing_slash(router: AxumRouter) -> NormalizePath<AxumRouter> {
    NormalizePathLayer::trim_trailing_slash().layer(router)
}

/// Builds the guard input from a loaded configuration and runs the
/// database-target guard (ADR 0024 §4, layer 3).
///
/// - `environment`: the environment the configuration was loaded for.
/// - `config`: the loaded configuration.
///
/// It checks the application database (`database.uri`, with
/// `dangerously_truncate` and `dangerously_recreate`) and, when present, the
/// Postgres job queue (`queue.uri`, with `dangerously_flush`, which empties
/// the queue table at startup).
///
/// # Errors
///
/// Returns the guard's refusal as an [`Error::Message`]. A queue kind other
/// than Postgres is refused as well: backend3 compiles no other queue
/// backend and runs no Redis (ADR 0016 §1).
pub fn enforce_database_guard(environment: &Environment, config: &Config) -> Result<()> {
    let database = &config.database;
    let mut database_flags = Vec::new();
    if database.dangerously_truncate {
        database_flags.push("database.dangerously_truncate");
    }
    if database.dangerously_recreate {
        database_flags.push("database.dangerously_recreate");
    }
    let mut connections = vec![Connection {
        uri_key: "database.uri",
        uri: &database.uri,
        enabled_destructive_flags: database_flags,
    }];

    match &config.queue {
        None => {}
        Some(QueueConfig::Postgres(queue)) => connections.push(Connection {
            uri_key: "queue.uri",
            uri: &queue.uri,
            enabled_destructive_flags: if queue.dangerously_flush {
                vec!["queue.dangerously_flush"]
            } else {
                Vec::new()
            },
        }),
        // `QueueConfig` is non-exhaustive; Redis is its only other variant
        // in this build.
        Some(_) => {
            return Err(Error::string(
                "Refusing to start: queue.kind must be Postgres. backend3 runs its job queue in \
                 PostgreSQL and has no Redis (ADR 0016); set `queue.kind: Postgres`.",
            ));
        }
    }

    database_target::check(&environment.to_string(), &connections).map_err(Error::msg)
}

/// The bound listener of `settings.metrics`, from [`bind_metrics_listener`].
///
/// A type of its own, so that [`serve_until`] cannot be handed its two
/// listeners the wrong way round: that would serve `/metrics` on the routed
/// HTTP port (ADR 0023 §9).
#[derive(Debug)]
pub struct MetricsListener(TcpListener);

impl MetricsListener {
    /// The address the listener is bound to.
    ///
    /// # Errors
    ///
    /// Returns the operating system's error when it cannot be read.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.0.local_addr()
    }
}

/// Binds the `/metrics` listener of `settings.metrics`.
///
/// - `settings`: the IP address and port to listen on. Exactly that one
///   address is bound, never a fallback: [`Settings`] accepts no host name.
///
/// # Errors
///
/// Returns an [`Error::Message`] naming the address and both setting keys
/// when the address cannot be bound, for example because a second server
/// started with the same `settings.metrics.port`.
pub async fn bind_metrics_listener(settings: &MetricsSettings) -> Result<MetricsListener> {
    let address = SocketAddr::new(settings.binding, settings.port);
    match TcpListener::bind(address).await {
        Ok(listener) => Ok(MetricsListener(listener)),
        Err(error) => Err(Error::Message(format!(
            "Refusing to start: cannot listen for /metrics on {address} \
             ({METRICS_BINDING_KEY}, {METRICS_PORT_KEY}; the port must be free): {error}"
        ))),
    }
}

/// Serves the application router on `http` and [`metrics::router`] on
/// `metrics_listener` until `shutdown` resolves, then shuts both down
/// gracefully.
///
/// - `http`: the HTTP server's listener.
/// - `app`: the router Loco built; [`trim_trailing_slash`] goes in front of
///   it, with the client address as connect info, as in Loco's `serve`.
/// - `metrics_listener`: the listener of `settings.metrics`.
/// - `metrics`: the registry that the metrics listener serves.
/// - `shutdown`: resolves when the process should stop (Loco's
///   `shutdown_signal`, or a channel in tests).
/// - `on_shutdown`: `Hooks::on_shutdown`.
///
/// The public and the metrics side take different types, so they cannot be
/// swapped. On shutdown it keeps Loco's order (log, `on_shutdown`, then
/// drain), and a watch channel stops the metrics listener on the same signal.
/// Both servers run in this task with [`tokio::try_join!`], never
/// `tokio::spawn` (ADR 0008 "Enforcement", ADR 0016 §3); an initializer's
/// `before_run` would need a spawn, because Loco awaits it before serving.
///
/// # Errors
///
/// Returns the I/O error of the server that failed first; the other is
/// dropped.
pub async fn serve_until(
    http: TcpListener,
    app: AxumRouter,
    metrics_listener: MetricsListener,
    metrics: Metrics,
    shutdown: impl Future<Output = ()> + Send + 'static,
    on_shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let (stop_metrics, mut metrics_stopping) = watch::channel(false);
    let service = ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(
        trim_trailing_slash(app),
    );
    let http_server = axum::serve(http, service).with_graceful_shutdown(async move {
        shutdown.await;
        tracing::info!("shutting down...");
        // `send_replace` succeeds even if the metrics server already ended.
        stop_metrics.send_replace(true);
        on_shutdown.await;
    });
    let metrics_server = axum::serve(metrics_listener.0, metrics::router(metrics))
        .with_graceful_shutdown(async move {
            // An error means the sender was dropped together with the HTTP
            // server's future; the metrics listener stops then as well.
            let _ = metrics_stopping.wait_for(|stopping| *stopping).await;
        });
    tokio::try_join!(http_server.into_future(), metrics_server.into_future())?;
    Ok(())
}

/// The [`Metrics`] that `after_context` put into the shared store.
fn shared_metrics(ctx: &AppContext) -> Result<Metrics> {
    ctx.shared_store.get::<Metrics>().ok_or_else(|| {
        Error::string(
            "the metrics registry is missing from the shared store; App::after_context \
             registers it, so this context was not built by App::boot",
        )
    })
}

#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    fn app_version() -> String {
        // The Containerfile passes the commit as a build argument
        // (ADR 0012 §4); local builds report `dev`.
        let commit = option_env!("SOURCE_COMMIT").filter(|commit| !commit.is_empty());
        format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            commit.unwrap_or("dev")
        )
    }

    /// Registers the system clock as the app clock, then creates the metrics
    /// registry of this context and registers every metric family on it
    /// (ADR 0023 §9). Loco calls this hook at the end of `create_context`,
    /// where every entry point builds its context (server, workers, tasks,
    /// CLI and test harness; loco-rs 1.2.0 `src/boot.rs`, `src/cli.rs`), so
    /// the clock and the registry are there before any other hook,
    /// initializer, route or worker runs. Tasks get a registry as well; an
    /// unused one costs a few allocations.
    async fn after_context(ctx: AppContext) -> Result<AppContext> {
        set_clock(&ctx, Arc::new(SystemClock));
        let mut registry = Registry::default();
        let http = HttpMetrics::register(&mut registry);
        // The pool, queue and process families: read at scrape time, from
        // the application's pool and its database (ADR 0016 §3).
        let runtime = RuntimeMetrics::register(&mut registry, ctx.db.clone());
        // Later families (logins, uploads) register here, on the same
        // registry, before it is frozen.
        ctx.shared_store
            .insert(Metrics::new(registry, http, runtime));
        Ok(ctx)
    }

    /// Loads the configuration and refuses it before anything connects: the
    /// database guard, the typed settings, the metrics port, the session's
    /// JWT section (ADR 0006 §1; D92, research: DECISIONS R15), and in
    /// production the public throwaway keys of the local environments, the
    /// TOTP key and the JWT secrets (ADR 0034 §3). Every entry point loads
    /// its configuration here.
    async fn load_config(env: &Environment) -> Result<Config> {
        let config = env.load()?;
        enforce_database_guard(env, &config)?;
        Settings::from_section(config.settings.as_ref())
            .and_then(|settings| {
                settings.check_server_port(config.server.port)?;
                let jwt_secret = check_session_jwt(config.auth.as_ref())?;
                settings.refuse_published_keys(env, &jwt_secret)
            })
            .map_err(Error::msg)?;
        Ok(config)
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    /// Serves the router like Loco's default `serve` (same listener, connect
    /// info and graceful shutdown), with [`trim_trailing_slash`] in front of
    /// it, and `/metrics` on the listener of `settings.metrics`
    /// ([`serve_until`]).
    async fn serve(app: AxumRouter, ctx: &AppContext, serve_params: &ServeParams) -> Result<()> {
        let settings = Settings::from_section(ctx.config.settings.as_ref()).map_err(Error::msg)?;
        let metrics = shared_metrics(ctx)?;
        // Same address format as loco-rs 1.2.0's default implementation.
        let http =
            TcpListener::bind(&format!("{}:{}", serve_params.binding, serve_params.port)).await?;
        let metrics_listener = bind_metrics_listener(&settings.metrics).await?;
        // Loco's banner names only the HTTP address.
        tracing::info!(
            binding = %settings.metrics.binding,
            port = settings.metrics.port,
            "serving /metrics on its own port"
        );
        let shutdown_ctx = ctx.clone();
        serve_until(
            http,
            app,
            metrics_listener,
            metrics,
            shutdown_signal(),
            async move { Self::on_shutdown(&shutdown_ctx).await },
        )
        .await?;
        Ok(())
    }

    /// Loco's default middleware stack, configured by `server.middlewares`,
    /// with [`NotModifiedCors`] inserted right after `etag`.
    ///
    /// Loco applies the stack in order and each entry wraps the router built
    /// so far, so the entry after `etag` is the layer just outside it and
    /// sees the `304` that `etag` answers. Every other entry keeps its place,
    /// so compression, the secure headers and the request id still reach the
    /// `304` from outside (loco-rs 1.2.0 `controller/app_routes.rs`,
    /// `controller/middleware/mod.rs`). `tests/requests/public_caching.rs`
    /// checks the order.
    fn middlewares(ctx: &AppContext) -> Vec<Box<dyn MiddlewareLayer>> {
        let mut stack = middleware::default_middleware_stack(ctx);
        stack.insert_after("etag", Box::new(NotModifiedCors));
        stack
    }

    /// Wraps the router, after Loco's middleware stack, in the HTTP request
    /// metrics layer ([`metrics::with_http_metrics`]).
    async fn after_routes(router: AxumRouter, ctx: &AppContext) -> Result<AxumRouter> {
        let metrics = shared_metrics(ctx)?;
        Ok(metrics::with_http_metrics(router, metrics.http().clone()))
    }

    async fn before_routes(ctx: &AppContext) -> Result<AxumRouter<AppContext>> {
        let settings = Settings::from_section(ctx.config.settings.as_ref()).map_err(Error::msg)?;
        // Production refuses to start without the built admin UI; elsewhere
        // the Vite dev server may serve it instead (ADR 0004 §4). The check
        // runs here, where the router is built, so tasks and CronJobs, which
        // never build a router, do not need the directory.
        let requirement = if ctx.environment == Environment::Production {
            DistRequirement::Required
        } else {
            DistRequirement::Optional
        };
        let admin = admin_ui::router(&settings.admin_ui, requirement).map_err(Error::msg)?;
        Ok(AxumRouter::new().merge(admin).fallback(not_found::fallback))
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        // `cargo loco generate controller` injects `.add_route(...)` lines
        // after the next line; keep the route chain in this shape.
        AppRoutes::with_default_routes() // controller routes below
            .add_route(controllers::site::routes())
            // Every /api/v1 route, with the public API's CORS and panic layers
            // (src/controllers/v1/mod.rs). A public controller merges its routes
            // there, never here: a route added here would miss those layers.
            .add_route(controllers::v1::routes())
            .add_route(controllers::admin::routes())
    }

    // `cargo loco generate worker` injects `queue.register(...)` right after
    // the signature line below, using these parameter names.
    #[allow(unused_variables)]
    async fn connect_workers(ctx: &AppContext, queue: &Queue) -> Result<()> {
        Ok(())
    }

    #[allow(unused_variables)]
    fn register_tasks(tasks: &mut Tasks) {
        // tasks-inject (do not remove)
    }

    /// Truncates the application tables. Loco calls it for `db truncate`
    /// and for `dangerously_truncate`, which backend3 keeps off everywhere.
    ///
    /// It never covers `users`, `activity_log` or `revisions`. The two
    /// history tables are append-only, and their triggers reject `TRUNCATE`
    /// (ADR 0011 §5); they reference `users`, so PostgreSQL would refuse to
    /// truncate `users` without them. Emptying the accounts alone would also
    /// leave history pointing at nobody. A development database is reset by
    /// recreating it instead (AGENTS.md, "Commands"). Each entity slice adds
    /// only its own tables here.
    async fn truncate(_ctx: &AppContext) -> Result<()> {
        Ok(())
    }

    /// Loads seed fixtures from `base`. Each slice that adds fixtures adds
    /// them here.
    async fn seed(_ctx: &AppContext, _base: &Path) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{StatusCode, Uri},
        routing::get,
    };
    use tower::ServiceExt as _;

    use super::*;
    use crate::domain::clock::FixedClock;

    #[test]
    fn a_store_without_a_clock_says_how_to_get_one() {
        let error = clock_in(&SharedStore::default())
            .expect_err("an empty store has no clock")
            .to_string();
        assert!(error.contains("App::after_context"), "{error}");
        assert!(error.contains("backend3::app::set_clock"), "{error}");
    }

    #[test]
    fn the_last_clock_stored_is_the_one_returned() {
        let store = SharedStore::default();
        let [first, second] = ["2026-03-29T00:59:59Z", "2026-03-29T01:00:00Z"]
            .map(|rfc3339| rfc3339.parse::<DateTime<Utc>>().expect("RFC 3339"));
        set_clock_in(&store, Arc::new(FixedClock(first)));
        assert_eq!(clock_in(&store).expect("a clock").now(), first);
        set_clock_in(&store, Arc::new(FixedClock(second)));
        assert_eq!(clock_in(&store).expect("a clock").now(), second);
    }

    /// A router that answers with the URI it was routed with, so a test sees
    /// what the routing layer saw.
    fn echo_router() -> AxumRouter {
        AxumRouter::new()
            .route("/", get(|| async { "root".to_owned() }))
            .route("/posts", get(|uri: Uri| async move { uri.to_string() }))
            .fallback(|uri: Uri| async move { (StatusCode::NOT_FOUND, uri.to_string()) })
    }

    /// Sends one GET for `target` through [`trim_trailing_slash`].
    async fn get_through_trim(target: &str) -> (StatusCode, String) {
        let response = trim_trailing_slash(echo_router())
            .oneshot(
                Request::get(target)
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("the router is infallible");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the body");
        (
            status,
            String::from_utf8(body.to_vec()).expect("UTF-8 body"),
        )
    }

    #[tokio::test]
    async fn trailing_slashes_are_trimmed_before_routing() {
        for target in ["/posts", "/posts/", "/posts//", "//posts/"] {
            assert_eq!(
                get_through_trim(target).await,
                (StatusCode::OK, "/posts".to_owned()),
                "{target}"
            );
        }
    }

    #[tokio::test]
    async fn the_query_survives_the_trim() {
        assert_eq!(
            get_through_trim("/posts/?page=2&q=a/b").await,
            (StatusCode::OK, "/posts?page=2&q=a/b".to_owned())
        );
    }

    #[tokio::test]
    async fn the_root_path_is_left_alone() {
        assert_eq!(
            get_through_trim("/").await,
            (StatusCode::OK, "root".to_owned())
        );
    }

    #[tokio::test]
    async fn unknown_paths_reach_the_fallback_trimmed() {
        assert_eq!(
            get_through_trim("/api/v1/nope/").await,
            (StatusCode::NOT_FOUND, "/api/v1/nope".to_owned())
        );
    }
}
