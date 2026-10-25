//! Request tests through Loco's test server, so every response passes the
//! whole middleware stack exactly as in production.

mod admin_ui;
mod metrics;
mod not_found;
mod openapi;
mod security_headers;
mod site;
mod trailing_slash;
