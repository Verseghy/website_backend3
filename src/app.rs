//! The Loco hooks of backend3: how the application loads its configuration,
//! builds its router and registers workers and tasks.
//!
//! Three hooks carry project decisions:
//!
//! - `load_config` runs the database-target guard of ADR 0024 §4 (layer 3)
//!   and validates the typed settings. Every Loco entry point calls it before
//!   it connects anywhere: the test boot helpers, `cargo loco` and the
//!   `backend3-cli` binary.
//! - `before_routes` mounts the `/admin` router and the JSON `404` fallback.
//!   In loco-rs 1.2.0 the router returned here receives Loco's routes and is
//!   then wrapped by Loco's whole middleware stack, so `/admin` and the
//!   fallback get the same request id, logging, compression and secure
//!   headers as the API (`boot::setup_routes`). A router added in
//!   `after_routes` would get none of them (ADR 0004 §3).
//! - `serve` puts [`trim_trailing_slash`] in front of the finished router,
//!   so `/api/v1/posts/` is served exactly like `/api/v1/posts`
//!   (public-api.md DEC-10, open item O-2).
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

use std::{net::SocketAddr, path::Path};

use async_trait::async_trait;
use axum::{Router as AxumRouter, ServiceExt, extract::Request};
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
    app::{AppContext, Hooks},
    bgworker::Queue,
    boot::{BootResult, ServeParams, StartMode, create_app, shutdown_signal},
    config::{Config, QueueConfig},
    controller::AppRoutes,
    environment::Environment,
    task::Tasks,
};
use migration::Migrator;
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
        not_found,
    },
    domain::database_target::{self, Connection},
    settings::Settings,
};

/// The Loco application.
pub struct App;

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

    async fn load_config(env: &Environment) -> Result<Config> {
        let config = env.load()?;
        enforce_database_guard(env, &config)?;
        Settings::from_section(config.settings.as_ref()).map_err(Error::msg)?;
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
    /// it.
    async fn serve(app: AxumRouter, ctx: &AppContext, serve_params: &ServeParams) -> Result<()> {
        // Same address format as loco-rs 1.2.0's default implementation.
        let listener = tokio::net::TcpListener::bind(&format!(
            "{}:{}",
            serve_params.binding, serve_params.port
        ))
        .await?;
        let service = ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(
            trim_trailing_slash(app),
        );
        let shutdown_ctx = ctx.clone();
        axum::serve(listener, service)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                tracing::info!("shutting down...");
                Self::on_shutdown(&shutdown_ctx).await;
            })
            .await?;
        Ok(())
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
            .add_route(controllers::v1::openapi::routes())
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
    /// and for `dangerously_truncate`; there are no application tables yet,
    /// and each slice adds its own here.
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
