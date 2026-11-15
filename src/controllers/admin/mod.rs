//! The admin API under `/api/admin` (ADR 0005, ADR 0021 §4).
//!
//! [`routes`] is the admin route table (ADR 0007 §4, §5): every admin route
//! with the requirement an account must meet to call it, and the one
//! builder of the admin API's router, which `App::routes` adds once. Its
//! guard answers `401 unauthorized` to a request without a valid session
//! and `403 forbidden` to an account that does not meet the requirement,
//! before the handler runs, and every admin response carries
//! `Cache-Control: no-store` (ADR 0023 §6). A new admin route is one more
//! entry of [`routes::table`]; a route mounted any other way fails
//! `tests/requests/admin_routes.rs` (in `App::routes`) or
//! `tests/architecture/admin_mounts.rs` (in another hook, within the limits
//! of a text check that its documentation lists).
//!
//! A handler that needs a signed-in account takes [`session::AdminSession`],
//! which hands it the session that the guard opened and checked
//! (ADR 0006 §1), and never Loco's `JWT` or `JWTWithUser`, which skip those
//! checks.
//!
//! A handler that takes a JSON body takes it as
//! `Result<Json<T>, JsonRejection>` and answers a refused body with
//! [`json_rejection_response`], so every admin endpoint refuses a body that
//! is not its DTO the same way: `422 invalid_body`, or `413 body_too_large`
//! above the request body limit (research: DECISIONS R15 (D74)).
//!
//! A handler that writes an activity entry takes
//! [`client_context::ClientContext`] for the request id and, on a sign-in
//! or account entry, the client address, and passes them down to the model
//! (ADR 0011 §5, recipe 8 of the project skill).

use axum::{Json, extract::rejection::JsonRejection, http::StatusCode};
use loco_rs::prelude::*;

use crate::dtos::admin::error::{ErrorCode, ErrorEnvelope};

pub mod auth;
pub mod client_context;
pub mod media;
pub mod routes;
pub mod session;

/// The largest JSON request body an admin endpoint reads: 4 MiB (4,194,304
/// bytes), the per-route limit of ADR 0023 §5. The largest stored body is
/// 892,455 bytes (research: SYNTHESIS §4.3.1), so a whole page or post fits
/// with room to spare, while a JSON body never gets the 128 MiB that
/// `server.middlewares.limit_payload` allows the uploads. A larger body is
/// answered `413 body_too_large` ([`json_rejection_response`]). The route
/// table puts it on every route that is not an upload route
/// ([`routes::RequestBody`]).
pub const JSON_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// The admin error envelope for `code`, with the status that goes with it
/// (ADR 0021 §4).
///
/// - `code`: the error to report; its default English description is sent.
#[must_use]
pub fn error_response(code: ErrorCode) -> Response {
    // Every status `ErrorCode::status` returns is a valid status code; the
    // fallback only guards against a mistake in that table.
    let status = StatusCode::from_u16(code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(ErrorEnvelope::new(code))).into_response()
}

/// The admin error envelope for a request body that axum's `Json` extractor
/// refused (ADR 0021 §4; the maintainer's answer of 2026-10-25, research:
/// DECISIONS R15 (D74)).
///
/// - `rejection`: why the extractor refused the body.
///
/// A body above the request body limit answers `413 body_too_large`. Every
/// other refusal answers `422 invalid_body`: a body that is not JSON, one sent
/// without the `application/json` content type, and one with a field that is
/// missing, unknown or of the wrong type. axum itself would answer these with
/// 400, 415 or 422 and a plain-text body; the admin UI reads only the
/// envelope. ADR 0021 §4 has no 400, 413 or 415 row, so this is a reading of
/// it: a body that is not JSON, or not sent as JSON, counts as submitted
/// values that fail validation (`422`), and the size limit answers `413`, as
/// ADR 0015 §6's upload limit does. `field` stays `null`: serde's message names the field in the
/// wire spelling only for some of these cases, and ADR 0021 §4's per-field
/// errors are the validation of fields that did parse.
#[must_use]
pub fn json_rejection_response(rejection: &JsonRejection) -> Response {
    // axum gives the length-limit refusal, and only that one, the status
    // 413 (axum-core 0.5.6 `LengthLimitError`), so the status is the test.
    // A body that the client cut off answers 400 there and `invalid_body`
    // here.
    let code = if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ErrorCode::BodyTooLarge
    } else {
        ErrorCode::InvalidBody
    };
    error_response(code)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        extract::DefaultBodyLimit,
        http::{Request, header::CONTENT_TYPE},
    };
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::*;

    /// The body limit of the probe router: small, so the boundary is cheap.
    const LIMIT: usize = 64;

    /// A stand-in DTO, refusing unknown fields like the admin request DTOs.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Probe {
        name: String,
    }

    /// A handler that reads its body the way the admin handlers do.
    async fn probe(body: Result<Json<Probe>, JsonRejection>) -> Response {
        match body {
            Ok(Json(probe)) => (StatusCode::OK, probe.name).into_response(),
            Err(rejection) => json_rejection_response(&rejection),
        }
    }

    /// POSTs `body` with `content_type` through axum's body limit of
    /// [`LIMIT`] bytes, and returns the status and the body.
    async fn send(content_type: Option<&str>, body: String) -> (StatusCode, Vec<u8>) {
        let router = axum::Router::new()
            .route("/", axum::routing::post(probe))
            .layer(DefaultBodyLimit::max(LIMIT));
        let mut request = Request::post("/");
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let response = router
            .oneshot(request.body(Body::from(body)).expect("request"))
            .await
            .expect("infallible");
        let status = response.status();
        let body = to_bytes(response.into_body(), 4096).await.expect("body");
        (status, body.to_vec())
    }

    fn envelope(code: ErrorCode) -> (StatusCode, Vec<u8>) {
        (
            StatusCode::from_u16(code.status()).expect("a status"),
            serde_json::to_vec(&ErrorEnvelope::new(code)).expect("serializes"),
        )
    }

    /// A JSON `Probe` body of exactly `len` bytes: `{"name":"x"}` padded with
    /// spaces, which JSON allows between tokens.
    fn probe_body_of(len: usize) -> String {
        let head = r#"{"name":"x""#;
        format!("{head}{}}}", " ".repeat(len - head.len() - 1))
    }

    #[tokio::test]
    async fn a_body_that_is_not_the_dto_answers_invalid_body() {
        let json = Some("application/json");
        for (case, content_type, body) in [
            ("not JSON", json, "name=x"),
            ("cut-off JSON", json, r#"{"name":"#),
            ("empty", json, ""),
            // Whether an array is refused is the DTO's choice: a derived one
            // like `Probe` reads `["x"]` as its fields in order, and
            // `LoginRequest` refuses it (its own tests).
            ("a JSON number", json, "5"),
            ("missing field", json, "{}"),
            ("unknown field", json, r#"{"name":"x","admin":true}"#),
            ("wrong type", json, r#"{"name":5}"#),
            ("text/plain", Some("text/plain"), r#"{"name":"x"}"#),
            ("no content type", None, r#"{"name":"x"}"#),
        ] {
            assert_eq!(
                send(content_type, body.to_owned()).await,
                envelope(ErrorCode::InvalidBody),
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn a_body_above_the_limit_answers_body_too_large() {
        // At the limit the body is read; one byte more is refused.
        assert_eq!(
            send(Some("application/json"), probe_body_of(LIMIT)).await,
            (StatusCode::OK, b"x".to_vec())
        );
        assert_eq!(
            send(Some("application/json"), probe_body_of(LIMIT + 1)).await,
            envelope(ErrorCode::BodyTooLarge)
        );
        // axum checks the content type before it reads the body, so an
        // oversized body of another type is `invalid_body`.
        assert_eq!(
            send(Some("text/plain"), probe_body_of(LIMIT + 1)).await,
            envelope(ErrorCode::InvalidBody)
        );
    }

    #[tokio::test]
    async fn an_error_response_has_the_code_s_status_and_envelope() {
        for code in [
            ErrorCode::NotFound,
            ErrorCode::InvalidCredentials,
            ErrorCode::InvalidBody,
            ErrorCode::BodyTooLarge,
            ErrorCode::Unauthorized,
            ErrorCode::Forbidden,
        ] {
            let response = error_response(code);
            assert_eq!(response.status().as_u16(), code.status(), "{code:?}");
            let body = to_bytes(response.into_body(), 4096).await.expect("body");
            assert_eq!(
                body,
                serde_json::to_vec(&ErrorEnvelope::new(code)).expect("serializes")
            );
        }
    }
}
