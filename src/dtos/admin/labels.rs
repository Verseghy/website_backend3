//! Wire types of the labels, „címkék” (parity LBL-01).
//!
//! `GET /api/admin/labels` answers a `ListResponse<LabelResponse>`
//! (`super::list`), and `GET /api/admin/labels/{id}` one
//! [`LabelResponse`]. The label writes add their request DTOs here.

use serde::Serialize;
use ts_rs::TS;

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
}
