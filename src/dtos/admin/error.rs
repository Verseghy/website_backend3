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
//! Codes are added by the slice that first returns them. The bootstrap only
//! needs `not_found`, for unknown `/api/admin` paths.

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
}

impl ErrorCode {
    /// The HTTP status code that goes with this error code (ADR 0021 §4).
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::NotFound => 404,
        }
    }

    /// The English description sent with this code when the caller has no
    /// more specific text.
    #[must_use]
    pub const fn default_description(self) -> &'static str {
        match self {
            // The same text as Loco's `ErrorDetail` and the public API.
            Self::NotFound => "Resource was not found",
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
        assert_eq!(ErrorCode::NotFound.status(), 404);
        assert_eq!(
            serde_json::to_value(ErrorCode::NotFound).expect("serializes"),
            json!("not_found")
        );
    }

    #[test]
    fn typescript_declarations() {
        assert_eq!(
            ErrorCode::decl(&ts_rs::Config::default()),
            r#"type ErrorCode = "not_found";"#
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
