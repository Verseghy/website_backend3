//! The error envelope of the admin API (ADR 0021 §4).
//!
//! Every refused or invalid admin API request is answered with one envelope:
//! a stable [`ErrorCode`], an English `description` for script clients and
//! logs, the request `field` the error belongs to, and named `params` for the
//! Hungarian message template. The admin UI shows `errors.<code>` from its
//! Hungarian message file and falls back to `description` only for a code it
//! does not know. Its message table is declared
//! `satisfies Record<ErrorCode, string>`, so a code added here without a
//! Hungarian message fails the admin type check.
//!
//! Codes are added by the slice that first returns them: `not_found` for
//! unknown `/api/admin` paths, `invalid_credentials` for every failed
//! sign-in (ADR 0006 §4), and `invalid_body` and `body_too_large` for a
//! request body that cannot be read as the endpoint's JSON DTO (the
//! maintainer's answer of 2026-10-25, research: DECISIONS R15 (D74)).

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;

/// A stable machine-readable error code of the admin API.
///
/// A code that belongs to no resource is `snake_case` and keeps the public
/// API's spelling where it means the same thing (`not_found`). A rule of one
/// resource uses a dotted `<resource>.<reason>` code, such as
/// `search.term_too_long`.
// The doc comment above is exported into the TypeScript binding; Rust-only
// notes go here. Dotted codes need an explicit `#[serde(rename = "...")]` on
// their variant, because `rename_all` cannot produce a dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../admin/src/bindings/")]
pub enum ErrorCode {
    /// `404`: the resource or the API path does not exist.
    NotFound,
    /// `401`: the sign-in failed. One code for every reason (a wrong
    /// password, an unknown e-mail address, a deactivated or trashed
    /// account), so the answer does not reveal which accounts exist.
    InvalidCredentials,
    /// `422`: the request body is not the JSON the endpoint takes: not JSON,
    /// not sent as `application/json`, or with a field that is missing,
    /// unknown or of the wrong type. An error on the whole request, so
    /// `field` is `null`.
    InvalidBody,
    /// `413`: the request body is larger than the request body limit.
    BodyTooLarge,
}

impl ErrorCode {
    /// The HTTP status code that goes with this error code (ADR 0021 §4).
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::InvalidCredentials => 401,
            // ADR 0021 §4's row for submitted values that fail validation.
            // A body that is not the DTO's JSON is read as such a value
            // (research: DECISIONS R15 (D74)).
            Self::InvalidBody => 422,
            // ADR 0021 §4 has no row for it; 413 with the JSON envelope
            // follows ADR 0015 §6, whose upload limit answers it (D74).
            Self::BodyTooLarge => 413,
        }
    }

    /// The English description sent with this code when the caller has no
    /// more specific text.
    #[must_use]
    pub const fn default_description(self) -> &'static str {
        match self {
            // The same text as Loco's `ErrorDetail` and the public API.
            Self::NotFound => "Resource was not found",
            Self::InvalidCredentials => "The e-mail address or the password is wrong",
            Self::InvalidBody => "The request body is not the JSON this endpoint takes",
            Self::BodyTooLarge => "The request body is larger than the limit",
        }
    }
}

/// A value for a placeholder of the Hungarian message template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ErrorParam {
    /// A text value, for example a slug.
    Text(String),
    /// A number, for example a count of days. Values stay below 2^53, the
    /// largest integer JSON readers handle exactly (ADR 0005 §3).
    Number(i64),
}

/// The admin API error body: `{ error, description, field, params }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct ErrorEnvelope {
    /// The stable error code.
    pub error: ErrorCode,
    /// English free text for script clients and logs.
    pub description: String,
    /// The request field the error belongs to, spelt as the DTO serializes
    /// it, or `null` for an error on the whole request.
    pub field: Option<String>,
    /// Named values for the placeholders of the message; `{}` when none.
    #[ts(type = "Record<string, string | number>")]
    pub params: BTreeMap<String, ErrorParam>,
}

impl ErrorEnvelope {
    /// An envelope for `code` with its default description, no field and no
    /// parameters.
    ///
    /// - `code`: the error code to report.
    #[must_use]
    pub fn new(code: ErrorCode) -> Self {
        Self {
            error: code,
            description: code.default_description().to_owned(),
            field: None,
            params: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_found_envelope_serializes_exactly() {
        let envelope = ErrorEnvelope::new(ErrorCode::NotFound);
        assert_eq!(
            serde_json::to_value(&envelope).expect("serializes"),
            json!({
                "error": "not_found",
                "description": "Resource was not found",
                "field": null,
                "params": {}
            })
        );
    }

    #[test]
    fn field_and_params_serialize_in_key_order() {
        let mut envelope = ErrorEnvelope::new(ErrorCode::NotFound);
        envelope.field = Some("parentId".to_owned());
        envelope
            .params
            .insert("slug".to_owned(), ErrorParam::Text("hirek".to_owned()));
        envelope
            .params
            .insert("days".to_owned(), ErrorParam::Number(12));
        assert_eq!(
            serde_json::to_string(&envelope).expect("serializes"),
            r#"{"error":"not_found","description":"Resource was not found","field":"parentId","params":{"days":12,"slug":"hirek"}}"#
        );
    }

    #[test]
    fn codes_map_to_statuses() {
        for (code, status, wire) in [
            (ErrorCode::NotFound, 404, "not_found"),
            (ErrorCode::InvalidCredentials, 401, "invalid_credentials"),
            (ErrorCode::InvalidBody, 422, "invalid_body"),
            (ErrorCode::BodyTooLarge, 413, "body_too_large"),
        ] {
            assert_eq!(code.status(), status, "{code:?}");
            assert_eq!(serde_json::to_value(code).expect("serializes"), json!(wire));
        }
    }

    #[test]
    fn the_sign_in_failure_envelope_serializes_exactly() {
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::InvalidCredentials))
                .expect("serializes"),
            r#"{"error":"invalid_credentials","description":"The e-mail address or the password is wrong","field":null,"params":{}}"#
        );
    }

    #[test]
    fn the_body_envelopes_serialize_exactly() {
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::InvalidBody)).expect("serializes"),
            r#"{"error":"invalid_body","description":"The request body is not the JSON this endpoint takes","field":null,"params":{}}"#
        );
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::BodyTooLarge))
                .expect("serializes"),
            r#"{"error":"body_too_large","description":"The request body is larger than the limit","field":null,"params":{}}"#
        );
    }

    #[test]
    fn typescript_declarations() {
        assert_eq!(
            ErrorCode::decl(&ts_rs::Config::default()),
            r#"type ErrorCode = "not_found" | "invalid_credentials" | "invalid_body" | "body_too_large";"#
        );
        let envelope = ErrorEnvelope::decl(&ts_rs::Config::default());
        assert!(envelope.contains("error: ErrorCode"), "{envelope}");
        assert!(envelope.contains("field: string | null"), "{envelope}");
        assert!(
            envelope.contains("params: Record<string, string | number>"),
            "{envelope}"
        );
    }
}
