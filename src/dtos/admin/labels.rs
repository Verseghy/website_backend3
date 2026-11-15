//! Wire types of the labels, „címkék” (parity LBL-01, LBL-02).
//!
//! `GET /api/admin/labels` answers a `ListResponse<LabelResponse>`
//! (`super::list`), and `GET /api/admin/labels/{id}` one
//! [`LabelResponse`]. `POST /api/admin/labels` and
//! `PUT /api/admin/labels/{id}` read a [`LabelRequest`], which the write
//! contract of `super::write` reads (only its declared members, from a JSON
//! object only, parity SHR-10), and answer the saved label as a
//! [`LabelResponse`]. A name that a live label already has is answered with
//! [`name_taken_envelope`], the `409` of ADR 0021 §4.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use super::{
    error::{ErrorCode, ErrorEnvelope, ErrorParam},
    write::deserialize_object,
};
use crate::domain::labels::NAME_FIELD;

/// A label as the admin reads it: in the label list, in the label picker of
/// the post form, and on the label's own form.
///
/// Its field names are the ones the label list sorts on (`sort=name`,
/// `sort=color`, `sort=id`).
// The doc comment above is exported into the TypeScript binding. The audit
// columns are left out until a screen shows them: the history panel reads
// them from the revisions (ADR 0011 §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct LabelResponse {
    /// The label's id, which public links to its page carry.
    #[ts(type = "number")]
    pub id: i64,
    /// The label's name, as the editors wrote it.
    pub name: String,
    /// The label's colour, `#rrggbb` (upper- or lower-case hex digits).
    pub color: String,
}

/// The body of a label save, `POST /api/admin/labels` (create) and
/// `PUT /api/admin/labels/{id}` (edit): the label's two writable fields.
///
/// Read only from a JSON object with exactly these two string members: any
/// other member, such as the record's `id`, is refused, so send only these.
/// The server trims both; the name has 1 to 64 characters, and the colour
/// is `#rrggbb`.
// The doc comment above is exported into the TypeScript binding. The
// members are spelt as `domain::labels::NAME_FIELD` and `COLOR_FIELD`, the
// `field` of a refusal; the tests below hold the two together.
#[derive(Debug, Clone, PartialEq, Eq, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct LabelRequest {
    /// The label's name.
    pub name: String,
    /// The label's colour, `#rrggbb`.
    pub color: String,
}

/// The members of a [`LabelRequest`], read by serde's derive.
///
/// The derive refuses a missing, repeated or wrongly typed member, and
/// `deny_unknown_fields` an unknown one (parity SHR-10; research: DECISIONS
/// R15 (D74)).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelFields {
    name: String,
    color: String,
}

impl<'de> Deserialize<'de> for LabelRequest {
    /// Reads the request from a JSON object only
    /// ([`deserialize_object`] says why).
    ///
    /// - `deserializer`: the JSON body.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let LabelFields { name, color } = deserialize_object(
            deserializer,
            "a JSON object with the members name and color",
        )?;
        Ok(Self { name, color })
    }
}

/// The admin error envelope for a label name that a live label already has
/// (`409 label.name_taken`, ADR 0021 §4).
///
/// - `name`: the name the save sent, trimmed, which the Hungarian message
///   shows.
///
/// `field` is `name`, so the form shows the message at the name input. The
/// names compare without regard to case, and a label in the trash does not
/// count: the unique index `labels_live_lower_name_key` decides (ADR 0009,
/// "Constraints"; ADR 0011 §3).
#[must_use]
pub fn name_taken_envelope(name: &str) -> ErrorEnvelope {
    ErrorEnvelope {
        field: Some(NAME_FIELD.to_owned()),
        params: BTreeMap::from([("name".to_owned(), ErrorParam::Text(name.to_owned()))]),
        ..ErrorEnvelope::new(ErrorCode::LabelNameTaken)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::labels::COLOR_FIELD;

    #[test]
    fn a_label_serializes_with_its_three_fields() {
        let label = LabelResponse {
            id: 7,
            name: "Ünnepség".to_owned(),
            color: "#1E88E5".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(label).expect("serializes"),
            json!({"id": 7, "name": "Ünnepség", "color": "#1E88E5"})
        );
    }

    #[test]
    fn the_id_is_a_number_in_typescript() {
        // ADR 0005 §3: no 64-bit integer becomes a `bigint`.
        let declaration = LabelResponse::decl(&ts_rs::Config::default());
        assert!(declaration.contains("id: number"), "{declaration}");
        assert!(!declaration.contains("bigint"), "{declaration}");
    }

    #[test]
    fn a_request_has_exactly_the_name_and_the_colour() {
        let request: LabelRequest =
            serde_json::from_value(json!({ NAME_FIELD: " Hírek ", COLOR_FIELD: "#1e88e5" }))
                .expect("valid");
        // Read as sent: trimming and checking are the domain's
        // (`LabelInput::check`).
        assert_eq!(
            request,
            LabelRequest {
                name: " Hírek ".to_owned(),
                color: "#1e88e5".to_owned(),
            }
        );
    }

    #[test]
    fn a_request_refuses_every_member_it_does_not_declare() {
        // SHR-10: what ra-core's record holds besides the two fields (`id`),
        // the audit and trash columns, and a typo.
        for extra in [
            "id",
            "created_by",
            "updated_at",
            "deleted_at",
            "colour",
            "Name",
        ] {
            let mut body = json!({"name": "Hírek", "color": "#1E88E5"});
            body[extra] = json!(1);
            let error = serde_json::from_value::<LabelRequest>(body).expect_err(extra);
            assert!(
                error
                    .to_string()
                    .contains(&format!("unknown field `{extra}`")),
                "{extra}: {error}"
            );
        }
    }

    #[test]
    fn a_request_refuses_a_missing_repeated_or_mistyped_member_and_non_objects() {
        for body in [
            r#"{"name":"Hírek"}"#,
            r##"{"color":"#1E88E5"}"##,
            r##"{"name":null,"color":"#1E88E5"}"##,
            r##"{"name":5,"color":"#1E88E5"}"##,
            r##"{"name":"Hírek","color":["#1E88E5"]}"##,
            r##"{"name":"Hírek","name":"Sport","color":"#1E88E5"}"##,
            r##"["Hírek","#1E88E5"]"##,
            r#""Hírek""#,
            "null",
            "{}",
        ] {
            assert!(
                serde_json::from_str::<LabelRequest>(body).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn the_request_binding_has_two_strings() {
        let declaration = LabelRequest::decl(&ts_rs::Config::default());
        assert!(declaration.contains("name: string"), "{declaration}");
        assert!(declaration.contains("color: string"), "{declaration}");
        assert!(!declaration.contains("number"), "{declaration}");
    }

    #[test]
    fn a_taken_name_is_a_409_on_the_name_with_the_name_as_sent() {
        let envelope = name_taken_envelope("HÍREK");
        assert_eq!(envelope.error.status(), 409);
        assert_eq!(
            serde_json::to_value(&envelope).expect("serializes"),
            json!({
                "error": "label.name_taken",
                "description": ErrorCode::LabelNameTaken.default_description(),
                "field": "name",
                "params": {"name": "HÍREK"},
            })
        );
    }
}
