//! backend1's public REST API is not ported. No frontend calls it: the
//! Angular site reads backend2's GraphQL API, which `/api/v1` replaces
//! (research: SYNTHESIS §4.8). backend3 answers every route of backend1's
//! `routes/api.php` (`website_backend@0dead98:routes/api.php`) with the JSON
//! `404` of the public API, without its CORS header (public-api.md §8), and
//! serves none of them under `/api/v1` either. ADR 0032 records the
//! retirement as parity PUB-14, and these are the request tests its closing
//! rule names: a `404` for every route of that file.
//!
//! The two newsletter routes among them are parity NWS-02: the newsletter is
//! retired with its table (ADR 0019 §3, "Newsletter subscribers"), and
//! `/api/v1` has no newsletter path.

use axum::http::Method;
use backend3::{app::App, domain::api_path::RETIRED_API_PREFIXES};
use loco_rs::app::Hooks;

use crate::support::{
    assert_no_cors, assert_public_cors, assert_public_not_found, public_get, with_server,
};

/// Every route of `website_backend@0dead98:routes/api.php`, under the `/api`
/// prefix that Laravel's `RouteServiceProvider` gives that file. All are
/// `GET` routes; `/api/user` was the only one behind authentication.
const BACKEND1_ROUTES: [&str; 18] = [
    "/api/user",
    "/api/posts/getPost",
    "/api/posts/getPostsByAuthor",
    "/api/posts/getPostsByLabel",
    "/api/posts/listPosts",
    "/api/posts/search",
    "/api/posts/listFeaturedPosts",
    "/api/posts/getPreview",
    "/api/posts/getPostsByYearMonth",
    "/api/posts/getCountByMonth",
    "/api/newsletter/subscribe",
    "/api/newsletter/unsubscribe",
    "/api/events/getEventsByMonth",
    "/api/canteen/getCanteenMenus",
    "/api/canteen/getCanteenByWeek",
    "/api/colleagues/listColleagues",
    "/api/pages/getPageBySlug",
    "/api/menu/getMenuItems",
];

#[test]
fn every_retired_prefix_is_a_route_group_of_backend1() {
    // The JSON 404 covers backend1's route groups and nothing wider, so each
    // prefix must hold at least one of its routes.
    for prefix in RETIRED_API_PREFIXES {
        let below = format!("{prefix}/");
        assert!(
            BACKEND1_ROUTES
                .iter()
                .any(|route| *route == prefix || route.starts_with(&below)),
            "{prefix}"
        );
    }
}

#[tokio::test]
async fn every_backend1_public_route_answers_the_json_404() {
    with_server(|request, _ctx| async move {
        for route in BACKEND1_ROUTES {
            // As backend1's clients called them: with query parameters, and
            // also with a trailing slash (trimmed before routing, DEC-10).
            for path in [
                route.to_owned(),
                format!("{route}?id=1&page=2"),
                format!("{route}/"),
            ] {
                let response = public_get(&request, &path).await;
                assert_public_not_found(&response, &path);
                // Outside /api/v1, so no CORS header (ADR 0014).
                assert_no_cors(&response, &path);
            }
            let post = request.method(Method::POST, route).await;
            assert_public_not_found(&post, &format!("POST {route}"));
        }
    })
    .await;
}

#[tokio::test]
async fn no_backend1_route_is_served_under_v1() {
    with_server(|request, _ctx| async move {
        for route in BACKEND1_ROUTES {
            let path = route.replacen("/api/", "/api/v1/", 1);
            let response = public_get(&request, &path).await;
            assert_public_not_found(&response, &path);
            assert_public_cors(&response, &path);
        }
    })
    .await;
}

#[tokio::test]
async fn the_newsletter_is_served_nowhere() {
    with_server(|request, ctx| async move {
        // Its legacy routes are in the list above, which also maps them under
        // /api/v1; here with the token query of backend1 and both methods.
        for path in [
            "/api/newsletter/unsubscribe?token=abc",
            "/api/v1/newsletter",
            "/api/v1/newsletter/subscribe?token=abc",
        ] {
            for method in [Method::GET, Method::POST] {
                let response = request.method(method.clone(), path).await;
                assert_public_not_found(&response, &format!("{method} {path}"));
            }
        }
        // Not only unanswered: no route of the table serves a newsletter or
        // any of backend1's paths.
        for route in App::routes(&ctx).collect() {
            assert!(!route.uri.contains("newsletter"), "{}", route.uri);
            assert!(
                !BACKEND1_ROUTES.contains(&route.uri.as_str()),
                "{}",
                route.uri
            );
        }
    })
    .await;
}
