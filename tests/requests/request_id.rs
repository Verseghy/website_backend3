//! Every request's id is one that the server made (research: DECISIONS R23
//! (D119)): an `x-request-id` header that a caller sends never becomes the
//! id. Loco answers each request with its id in `x-request-id`; through the
//! whole router and middleware stack, that is a fresh UUID v4 on every kind
//! of path (the public API, the admin API with and without a session, the
//! admin UI, the health check and an unknown path), also when the caller
//! sent an id of its own, once or twice.

use axum_test::TestResponse;
use loco_rs::prelude::Uuid;

use crate::support::with_server;

/// A request id of the caller's choice.
const CALLER_REQUEST_ID: &str = "kovacs_janos@verseghy";

/// The response's `x-request-id`, after checking that the server made it.
fn server_made_id(response: &TestResponse, path: &str) -> String {
    let id = response
        .header("x-request-id")
        .to_str()
        .expect("ASCII")
        .to_owned();
    let uuid =
        Uuid::parse_str(&id).unwrap_or_else(|error| panic!("{path}: {id:?} is no UUID: {error}"));
    assert_eq!(uuid.get_version_num(), 4, "{path}: {id}");
    id
}

#[tokio::test]
async fn a_caller_s_request_id_never_becomes_the_id() {
    with_server(|server, _ctx| async move {
        let mut seen = Vec::new();
        for path in [
            "/_ping",
            "/api/v1/nincs-ilyen",
            "/api/admin/auth/me",
            "/api/admin/auth/nincs-ilyen",
            "/admin/",
            "/nincs-ilyen",
        ] {
            let without = server.get(path).await;
            seen.push(server_made_id(&without, path));

            let with_one = server
                .get(path)
                .add_header("x-request-id", CALLER_REQUEST_ID)
                .await;
            let id = server_made_id(&with_one, path);
            assert_ne!(id, CALLER_REQUEST_ID, "{path}");
            seen.push(id);

            let with_two = server
                .get(path)
                .add_header("x-request-id", CALLER_REQUEST_ID)
                .add_header("x-request-id", "masodik")
                .await;
            seen.push(server_made_id(&with_two, path));
        }
        // Every request got its own id.
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count);
    })
    .await;
}

#[tokio::test]
async fn a_post_with_a_caller_s_id_gets_the_server_s_id_too() {
    // The sign-in, whose entries record the id (tests/requests/auth_activity.rs
    // checks the stored value): a refused body answers before any lookup,
    // with the server's id all the same.
    with_server(|server, _ctx| async move {
        let response = server
            .post("/api/admin/auth/login")
            .add_header("x-request-id", CALLER_REQUEST_ID)
            .text("nem JSON")
            .await;
        assert_ne!(
            server_made_id(&response, "/api/admin/auth/login"),
            CALLER_REQUEST_ID
        );
    })
    .await;
}
