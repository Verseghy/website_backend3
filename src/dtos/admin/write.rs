//! The wire side of the admin write contract (parity SHR-10, SHR-13;
//! ADR 0005, ADR 0021 §4): how every create and update reads its request
//! body, and how it answers a field that its validation refused.
//!
//! - **Only declared fields** (SHR-10). A request DTO reads exactly its own
//!   members: its derived field struct carries `deny_unknown_fields`, and
//!   [`deserialize_object`] hands it the members of a JSON object and of
//!   nothing else. A body with an unknown member (an `id`, a `created_by`, a
//!   typo), a repeated or missing member, a member of the wrong type, or a
//!   body that is not a JSON object, is refused whole as `422 invalid_body`
//!   (`controllers::admin::json_rejection_response`) before any field is
//!   checked or written, so a write can never set a column that its DTO
//!   does not declare.
//! - **A field error, never a server error** (SHR-13). The handler checks the
//!   DTO's fields with the resource's domain rules, which mirror every
//!   column limit (`crate::domain::input`), and answers the first refused
//!   field with [`field_error_envelope`]: a `422` whose `field` is the
//!   member and whose `params` fill the Hungarian message (ADR 0021 §4).
//!
//! The field codes belong to no resource, so they are `snake_case`
//! (ADR 0021 §4): every resource's name, title or colour is refused with the
//! same code and message, and its limit travels in `params`. A rule of one
//! resource (`label.name_taken`, `post.date_required`) gets a dotted code of
//! its own, in that resource's DTO module.

use std::{collections::BTreeMap, fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor, value::MapAccessDeserializer},
};

use super::error::{ErrorCode, ErrorEnvelope, ErrorParam};
use crate::domain::input::{FieldError, FieldRule};

/// Reads `F`, a request DTO's derived field struct, from a JSON object, and
/// from nothing else.
///
/// - `deserializer`: the request body.
/// - `expecting`: what the body must be, for serde's error message, such as
///   "a JSON object with the members name and color".
///
/// serde's derived `Deserialize` also reads a struct from a JSON array of
/// its field values in order, so `["Hírek", "#1E88E5"]` would be a label.
/// The visitor implements only `visit_map`, so anything but an object is
/// refused.
///
/// It streams, as `dtos::admin::auth::LoginRequest` does: the derive reads
/// the members as they are parsed (`MapAccessDeserializer`), refuses a body
/// at the name of its first unknown or repeated member, before that
/// member's value is parsed, and keeps only the declared values. Reading a
/// `serde_json::Map` or `Value` first would build the whole body in memory
/// before any member is checked, and keep the last of two members with the
/// same name (recipe 1 step 6 of the project skill).
///
/// # Errors
///
/// serde's error for a body that is not a JSON object, or whose members the
/// derive of `F` refuses.
pub fn deserialize_object<'de, D, F>(
    deserializer: D,
    expecting: &'static str,
) -> Result<F, D::Error>
where
    D: Deserializer<'de>,
    F: Deserialize<'de>,
{
    deserializer.deserialize_map(ObjectOnly {
        expecting,
        fields: PhantomData,
    })
}

/// The visitor of [`deserialize_object`]: a map, handed unread to `F`.
struct ObjectOnly<F> {
    /// What the body must be, for serde's error message.
    expecting: &'static str,
    /// The field struct it reads; `fn() -> F` keeps the visitor `Send` and
    /// `Sync` whatever `F` is.
    fields: PhantomData<fn() -> F>,
}

impl<'de, F: Deserialize<'de>> Visitor<'de> for ObjectOnly<F> {
    type Value = F;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.expecting)
    }

    /// Hands the map, unread, to the derive of `F`.
    ///
    /// - `map`: the members of the JSON object, read one by one.
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<F, A::Error> {
        F::deserialize(MapAccessDeserializer::new(map))
    }
}

/// The admin error envelope for a field that a write's validation refused
/// (ADR 0021 §4).
///
/// - `error`: the refused field and the rule it broke.
///
/// The status is `422`. `field` is the member as the request DTO spells it,
/// so the form shows the message at that input; the English `description`
/// names the field and the rule; `params` holds what the Hungarian message
/// needs, `max` (in characters) for a text that is too long and nothing for
/// the other rules.
#[must_use]
pub fn field_error_envelope(error: &FieldError) -> ErrorEnvelope {
    let (code, params) = match error.rule {
        FieldRule::Required => (ErrorCode::FieldRequired, BTreeMap::new()),
        FieldRule::TooLong { max } => (
            ErrorCode::FieldTooLong,
            // Every limit is a small constant; the fallback only keeps the
            // conversion total.
            BTreeMap::from([(
                "max".to_owned(),
                ErrorParam::Number(i64::try_from(max).unwrap_or(i64::MAX)),
            )]),
        ),
        FieldRule::InvalidCharacter => (ErrorCode::FieldInvalidCharacter, BTreeMap::new()),
        FieldRule::InvalidColor => (ErrorCode::FieldInvalidColor, BTreeMap::new()),
    };
    ErrorEnvelope {
        description: error.to_string(),
        field: Some(error.field.to_owned()),
        params,
        ..ErrorEnvelope::new(code)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A stand-in field struct, refusing unknown members like every request
    /// DTO's.
    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Probe {
        name: String,
        color: String,
    }

    /// A stand-in request DTO read through [`deserialize_object`].
    #[derive(Debug, PartialEq)]
    struct ProbeRequest(Probe);

    impl<'de> Deserialize<'de> for ProbeRequest {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserialize_object(deserializer, "a probe object").map(Self)
        }
    }

    fn read(body: &str) -> Result<ProbeRequest, serde_json::Error> {
        serde_json::from_str(body)
    }

    #[test]
    fn an_object_with_exactly_the_declared_members_is_read() {
        assert_eq!(
            read(r##"{"color":"#123456","name":"Hírek"}"##).expect("valid"),
            ProbeRequest(Probe {
                name: "Hírek".to_owned(),
                color: "#123456".to_owned(),
            })
        );
    }

    #[test]
    fn anything_but_an_object_is_refused_with_the_expected_shape() {
        for body in [
            r##"["Hírek","#123456"]"##,
            r#""Hírek""#,
            "5",
            "null",
            "true",
        ] {
            let error = read(body).expect_err(body);
            assert_eq!(
                error.classify(),
                serde_json::error::Category::Data,
                "{body}"
            );
            assert!(
                error.to_string().contains("a probe object"),
                "{body}: {error}"
            );
        }
    }

    #[test]
    fn an_unknown_repeated_missing_or_mistyped_member_is_refused() {
        for (body, message) in [
            (
                r##"{"name":"a","color":"#123456","id":7}"##,
                "unknown field `id`",
            ),
            (
                r##"{"name":"a","color":"#123456","created_by":1}"##,
                "unknown field `created_by`",
            ),
            (
                r##"{"name":"a","name":"b","color":"#123456"}"##,
                "duplicate field `name`",
            ),
            (r#"{"name":"a"}"#, "missing field `color`"),
            (r##"{"name":5,"color":"#123456"}"##, "invalid type"),
            (r##"{"name":null,"color":"#123456"}"##, "invalid type"),
        ] {
            let error = read(body).expect_err(body);
            assert!(error.to_string().contains(message), "{body}: {error}");
        }
    }

    #[test]
    fn a_body_is_refused_at_its_first_unknown_member_without_reading_on() {
        // What follows the unknown member is not JSON at all: a data error
        // that names the member shows that nothing after it was parsed.
        let error = read(r#"{"id": this is not JSON"#).expect_err("refused");
        assert_eq!(error.classify(), serde_json::error::Category::Data);
        assert!(error.to_string().contains("unknown field `id`"), "{error}");
    }

    #[test]
    fn every_field_rule_is_a_422_on_its_field_with_its_params() {
        for (rule, code, params) in [
            (FieldRule::Required, "field_required", json!({})),
            (
                FieldRule::TooLong { max: 64 },
                "field_too_long",
                json!({"max": 64}),
            ),
            (
                FieldRule::InvalidCharacter,
                "field_invalid_character",
                json!({}),
            ),
            (FieldRule::InvalidColor, "field_invalid_color", json!({})),
        ] {
            let error = FieldError::new("name", rule);
            let envelope = field_error_envelope(&error);
            assert_eq!(envelope.error.status(), 422, "{rule:?}");
            assert_eq!(
                serde_json::to_value(&envelope).expect("serializes"),
                json!({
                    "error": code,
                    "description": error.to_string(),
                    "field": "name",
                    "params": params,
                })
            );
        }
    }
}
