//! Host-level routes of `backend3.microshift.verseghy-gimnazium.net`
//! (ADR 0014).
//!
//! - `/` redirects to the admin UI at `/admin/`. The redirect is temporary
//!   (307), so browsers do not cache it forever if the root ever serves
//!   something else.
//! - `/robots.txt` keeps search engines away from the whole host: the admin
//!   and the API are not for indexing.

use axum::{
    http::header::CONTENT_TYPE,
    response::{IntoResponse, Redirect},
    routing::get,
};
use loco_rs::controller::Routes;

/// Where `/` sends the browser.
pub const ROOT_REDIRECT_TARGET: &str = "/admin/";

/// The body of `/robots.txt`.
pub const ROBOTS_TXT: &str = "User-agent: *\nDisallow: /\n";

/// The routes of this module, for `App::routes`.
#[must_use]
pub fn routes() -> Routes {
    Routes::new()
        .add("/", get(root))
        .add("/robots.txt", get(robots))
}

/// `GET /`: a temporary redirect to the admin UI.
async fn root() -> Redirect {
    Redirect::temporary(ROOT_REDIRECT_TARGET)
}

/// `GET /robots.txt`: disallow everything.
async fn robots() -> impl IntoResponse {
    ([(CONTENT_TYPE, "text/plain; charset=utf-8")], ROBOTS_TXT)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        http::{StatusCode, header::LOCATION},
    };

    use super::*;

    #[tokio::test]
    async fn root_redirects_to_the_admin() {
        let response = root().await.into_response();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[LOCATION], "/admin/");
    }

    #[tokio::test]
    async fn robots_disallows_everything() {
        let response = robots().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.expect("body");
        assert_eq!(body, "User-agent: *\nDisallow: /\n");
    }
}
