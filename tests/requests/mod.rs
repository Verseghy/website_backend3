//! Request tests through Loco's test server, so every response passes the
//! whole middleware stack exactly as in production.

mod admin_ui;
mod auth_login;
mod metrics;
mod not_found;
mod openapi;
mod public_caching;
mod public_router;
mod retired_routes;
mod security_headers;
mod site;
mod starter_auth;
mod trailing_slash;
