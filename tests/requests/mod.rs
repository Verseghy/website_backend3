//! Request tests through Loco's test server, so every response passes the
//! whole middleware stack exactly as in production.

mod admin_routes;
mod admin_ui;
mod auth_activity;
mod auth_login;
mod auth_logout;
mod auth_refresh;
mod auth_session;
mod media_upload;
mod metrics;
mod not_found;
mod openapi;
mod public_caching;
mod public_fixtures;
mod public_global;
mod public_harness;
mod public_router;
mod request_id;
mod retired_routes;
mod security_headers;
mod site;
mod starter_auth;
mod storage;
mod trailing_slash;
