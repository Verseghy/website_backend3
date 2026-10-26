//! The error body of the public read API (public-api.md §6.15, §8, DEC-11).
//!
//! Every `4xx` and `5xx` response under `/api/v1` except `405` carries
//! `{ "error": <code>, "description": <text> }`, the shape of Loco's
//! `ErrorDetail`. `error` is one of the [`ErrorCode`]s of public-api.md §8,
//! which clients and tests match on; `description` is English free text for
//! humans. The OpenAPI document publishes both types as the `Error` and
//! `ErrorCode` component schemas.
//!
//! An error that backend3 builds itself is answered through [`Error`]'s
//! `IntoResponse`: the code's status, the body written by Loco's
//! `ErrorDetail` (DEC-11) and `Cache-Control: no-store` (public-api.md §8).
//! A handler returns it as `Ok(error.into_response())`; the router fallback
//! and the panic handler of `crate::controllers::v1` use it too.
//!
//! A handler never turns a client error into a `loco_rs::Error`: Loco's
//! `IntoResponse` for its error logs every one at `ERROR` level as
//! `controller_error` (loco-rs 1.2.0, `src/controller/mod.rs`), so a client's
//! mistake would be logged as a server failure. A database failure stays a
//! `loco_rs::Error` (`?`), which Loco logs on purpose and answers with the
//! same bytes as [`ErrorCode::InternalServerError`]'s body.
//!
//! `405` has no code: axum answers it with an empty body, which clients must
//! not rely on (public-api.md §8).

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
};
use loco_rs::controller::ErrorDetail;
use serde::Serialize;
use utoipa::ToSchema;

/// A stable, machine-readable error code. Clients match on this value; the
/// `description` next to it is free text that may change.
// The doc comment above is published as the schema's description; Rust-only
// notes go here. The codes, their statuses and their meanings are the table
// of public-api.md §8, and a code is added there first. `ALL` lists every
// variant; a unit test compares it with the variants utoipa sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// `400`: `limit` is not digits or is outside 1 to 100.
    InvalidLimit,
    /// `400`: `before` is not a valid cursor.
    InvalidCursor,
    /// `400`: `q` is missing, empty, too long or contains U+0000, or
    /// `featured` is neither `true` nor `false`.
    InvalidQuery,
    /// `400`: a path `{id}` is not a positive integer up to 2^53 − 1.
    InvalidId,
    /// `400`: a path `{year}` or `{month}` is out of range.
    InvalidDate,
    /// `400`: a path `{isoYear}` or `{isoWeek}` is out of range, or the week
    /// does not exist in that year.
    InvalidWeek,
    /// `404`: the resource does not exist or is not visible, or the path is
    /// unknown.
    NotFound,
    /// `500`: a database failure or a bug; the description has no internal
    /// details.
    InternalServerError,
}

impl ErrorCode {
    /// Every code, in the order of public-api.md §8.
    pub const ALL: [Self; 8] = [
        Self::InvalidLimit,
        Self::InvalidCursor,
        Self::InvalidQuery,
        Self::InvalidId,
        Self::InvalidDate,
        Self::InvalidWeek,
        Self::NotFound,
        Self::InternalServerError,
    ];

    /// The HTTP status that goes with this code (public-api.md §8).
    #[must_use]
    pub const fn status(self) -> StatusCode {
        match self {
            Self::InvalidLimit
            | Self::InvalidCursor
            | Self::InvalidQuery
            | Self::InvalidId
            | Self::InvalidDate
            | Self::InvalidWeek => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The code as it is written on the wire, the same text serde writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidLimit => "invalid_limit",
            Self::InvalidCursor => "invalid_cursor",
            Self::InvalidQuery => "invalid_query",
            Self::InvalidId => "invalid_id",
            Self::InvalidDate => "invalid_date",
            Self::InvalidWeek => "invalid_week",
            Self::NotFound => "not_found",
            Self::InternalServerError => "internal_server_error",
        }
    }

    /// The English description sent with this code when the caller has no
    /// more specific text.
    #[must_use]
    pub const fn default_description(self) -> &'static str {
        match self {
            Self::InvalidLimit => "limit must be an integer between 1 and 100",
            // backend2's text for the same error (public-api.md §8).
            Self::InvalidCursor => "Wrong cursor format",
            Self::InvalidQuery => "A query parameter is invalid",
            Self::InvalidId => "id must be a positive integer",
            Self::InvalidDate => "year must be between 1 and 9999 and month between 1 and 12",
            Self::InvalidWeek => {
                "isoYear must be between 1 and 9999 and isoWeek must exist in that year"
            }
            // Loco's own texts, so that every 404 and 500 under /api/v1
            // reads the same, whoever built it.
            Self::NotFound => "Resource was not found",
            Self::InternalServerError => "Internal Server Error",
        }
    }
}

/// The body of every error response, except `405`, which has no body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Error {
    /// The error code.
    pub error: ErrorCode,
    /// English free text for humans. Clients match on `error` only.
    pub description: String,
}

impl Error {
    /// An error body for `code` with the code's default description.
    ///
    /// - `code`: the error to report.
    #[must_use]
    pub fn new(code: ErrorCode) -> Self {
        Self::with_description(code, "")
    }

    /// An error body for `code` with a specific description, such as
    /// `week 53 does not exist in 2025`.
    ///
    /// - `code`: the error to report.
    /// - `description`: English text for humans. An empty text is replaced by
    ///   the code's default description: the schema requires a description,
    ///   and Loco's `ErrorDetail` would leave an empty one out of the body.
    #[must_use]
    pub fn with_description(code: ErrorCode, description: impl Into<String>) -> Self {
        let description = description.into();
        Self {
            error: code,
            description: if description.is_empty() {
                code.default_description().to_owned()
            } else {
                description
            },
        }
    }

    /// The HTTP status of the response that carries this body.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.error.status()
    }
}

/// The status of the code, the body through Loco's `ErrorDetail` (DEC-11),
/// which has the same bytes as [`Error`]'s own serialization, and
/// `Cache-Control: no-store`, because an error is never cached
/// (public-api.md §8). The CORS header comes from the `/api/v1` layers.
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ErrorDetail::new(self.error.as_str(), self.description);
        let mut response = (status, Json(body)).into_response();
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        http::header::{CONTENT_TYPE, HeaderName},
    };
    use proptest::prelude::*;
    use serde_json::{Value, json};
    use utoipa::PartialSchema;

    use super::*;

    #[test]
    fn not_found_serializes_exactly() {
        assert_eq!(
            serde_json::to_string(&Error::new(ErrorCode::NotFound)).expect("serializes"),
            r#"{"error":"not_found","description":"Resource was not found"}"#
        );
    }

    #[test]
    fn codes_have_the_statuses_and_spellings_of_the_spec() {
        let table = [
            (ErrorCode::InvalidLimit, 400, "invalid_limit"),
            (ErrorCode::InvalidCursor, 400, "invalid_cursor"),
            (ErrorCode::InvalidQuery, 400, "invalid_query"),
            (ErrorCode::InvalidId, 400, "invalid_id"),
            (ErrorCode::InvalidDate, 400, "invalid_date"),
            (ErrorCode::InvalidWeek, 400, "invalid_week"),
            (ErrorCode::NotFound, 404, "not_found"),
            (ErrorCode::InternalServerError, 500, "internal_server_error"),
        ];
        assert_eq!(
            table.map(|(code, _, _)| code),
            ErrorCode::ALL,
            "ALL follows the order of public-api.md §8"
        );
        for (code, status, wire) in table {
            assert_eq!(code.status().as_u16(), status, "{wire}");
            assert_eq!(code.as_str(), wire);
            assert_eq!(serde_json::to_value(code).expect("serializes"), json!(wire));
            assert!(!code.default_description().is_empty(), "{wire}");
        }
    }

    #[test]
    fn the_schema_lists_exactly_the_codes_of_all() {
        let schema = serde_json::to_value(ErrorCode::schema()).expect("serializes");
        let listed: Vec<Value> = ErrorCode::ALL
            .iter()
            .map(|code| json!(code.as_str()))
            .collect();
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["enum"], Value::Array(listed));
    }

    #[test]
    fn a_specific_description_is_kept() {
        let error =
            Error::with_description(ErrorCode::InvalidWeek, "week 53 does not exist in 2025");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            serde_json::to_value(&error).expect("serializes"),
            json!({ "error": "invalid_week", "description": "week 53 does not exist in 2025" })
        );
    }

    /// The values of header `name` in `response`.
    fn values(response: &Response, name: &HeaderName) -> Vec<String> {
        response
            .headers()
            .get_all(name)
            .iter()
            .map(|value| value.to_str().expect("ASCII").to_owned())
            .collect()
    }

    #[tokio::test]
    async fn every_code_renders_its_status_body_and_no_store() {
        for code in ErrorCode::ALL {
            // A specific, non-ASCII text, which `ErrorDetail` must keep as it is.
            let error = Error::with_description(code, "the term “é” is too long");
            let expected = serde_json::to_vec(&error).expect("serializes");
            let response = error.into_response();
            assert_eq!(response.status(), code.status(), "{}", code.as_str());
            assert_eq!(values(&response, &CACHE_CONTROL), ["no-store"]);
            assert_eq!(values(&response, &CONTENT_TYPE), ["application/json"]);
            let body = to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body");
            assert_eq!(body.as_ref(), expected.as_slice(), "{}", code.as_str());
        }
    }

    proptest! {
        #[test]
        fn the_description_is_never_empty(description in any::<String>()) {
            let error = Error::with_description(ErrorCode::InvalidQuery, description.clone());
            prop_assert!(!error.description.is_empty());
            if !description.is_empty() {
                prop_assert_eq!(error.description, description);
            }
        }
    }
}
