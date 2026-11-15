//! The pure rules of labels, „címkék” (parity LBL-01, LBL-02; ADR 0009,
//! ADR 0011 §4, ADR 0020).
//!
//! - The list contract of `GET /api/admin/labels` ([`LIST`]): the fields a
//!   list of labels may be sorted on and the filters it takes.
//! - The fields of a label save ([`LabelInput::check`]): a name of 1 to
//!   [`NAME_MAX_CHARS`] characters and a colour `#rrggbb`, both trimmed,
//!   checked with the shared input rules of `super::input` (SHR-09,
//!   SHR-13). Every limit of the `labels` table is checked here, so a value
//!   the database would refuse is a field error first, never a server
//!   error. The one rule this cannot check, a name that a live label
//!   already has, is the live-name index's (`labels_live_lower_name_key`):
//!   the model turns its refusal into the `409` of the write.
//! - The revision snapshot of a label ([`LabelRevision`], version 1), which
//!   every create and update writes (ADR 0011 §4) and the legacy import's
//!   `import` revisions use too.

use serde::{Deserialize, Serialize};

use super::{
    input::{self, FieldError},
    list_query::{Direction, FilterKey, FilterKind, ListSpec, SortKey},
    revision::RevisionSnapshot,
};

/// The name field of a label save, as the request DTO spells it
/// (`dtos::admin::labels::LabelRequest`), and the column it is stored in.
pub const NAME_FIELD: &str = "name";

/// The colour field of a label save, as the request DTO spells it, and the
/// column it is stored in.
pub const COLOR_FIELD: &str = "color";

/// The most characters a label name has, once trimmed (parity LBL-02;
/// ADR 0009, "Column types": the length limit lives in domain validation,
/// not in the schema). The longest legacy label name has 24 characters
/// (ADR 0009, "Context").
pub const NAME_MAX_CHARS: usize = 64;

/// The fields of a label save, checked and trimmed: what a create or an
/// update writes, and nothing else (parity SHR-10).
///
/// The only way to build one is [`LabelInput::check`], so a value that
/// reaches the model has passed every rule; its fields are private for that
/// reason.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LabelInput {
    /// The trimmed name, 1 to [`NAME_MAX_CHARS`] characters, without U+0000.
    name: String,
    /// The trimmed colour, `#rrggbb`, kept as written.
    color: String,
}

impl LabelInput {
    /// Checks the two fields of a label save, in the order the label form
    /// shows them (the name, then the colour), and returns the values to
    /// store.
    ///
    /// - `name`: the name as the request sent it.
    /// - `color`: the colour as the request sent it.
    ///
    /// Both are trimmed (SHR-09). The name keeps its case and inner white
    /// space as written; the colour keeps the case of its digits, as the
    /// legacy colours do (the public site reads either).
    ///
    /// # Errors
    ///
    /// The [`FieldError`] of the first field that breaks a rule: the name
    /// when it is empty or only white space, holds U+0000, or is longer than
    /// [`NAME_MAX_CHARS`] characters (`super::input::required_text`); then
    /// the colour when it is empty or not `#rrggbb`
    /// (`super::input::color`).
    pub fn check(name: &str, color: &str) -> Result<Self, FieldError> {
        let name = input::required_text(NAME_FIELD, name, NAME_MAX_CHARS)?;
        let color = input::color(COLOR_FIELD, color)?;
        Ok(Self { name, color })
    }

    /// The checked name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The checked colour.
    #[must_use]
    pub fn color(&self) -> &str {
        &self.color
    }
}

/// The revision snapshot of a label (ADR 0011 §4): its two editable fields
/// as stored after the save.
///
/// Version 1. A label has no relation of its own (the posts carry their
/// labels' ids in their snapshots), and its audit columns are the revision's
/// own `created_at` and `created_by`, so they stay out. The model builds it
/// from the stored row (`models::labels::Model::revision_snapshot`), never
/// from the request, so a revision is always the record as the database
/// holds it. The legacy import writes its `import` revisions with this type
/// too.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LabelRevision {
    /// The label's name.
    pub name: String,
    /// The label's colour, `#rrggbb`.
    pub color: String,
}

impl RevisionSnapshot for LabelRevision {
    const ENTITY_TYPE: &'static str = "labels";
    const SCHEMA_VERSION: i32 = 1;
    const FIELDS: &'static [&'static str] = &[NAME_FIELD, COLOR_FIELD];
}

/// What the admin list of labels accepts (the shared list contract of
/// `super::list_query`).
///
/// - Sort fields: `name` and `color`, text in the Hungarian alphabet
///   (ADR 0020 §4), and `id`, which ra-core sorts on when a screen sets no
///   sort of its own. A list shows a label's name and colour (LBL-01).
/// - Default order: by name, A to Zs (LBL-01, "ordered by name"; ADR 0020
///   §4), then by id descending (SHR-03).
/// - Filter: `id`, the labels a post form refers to, read through ra-core's
///   `getMany` for the label picker.
///
/// The field names are those of the label DTO
/// (`dtos::admin::labels::LabelResponse`), because ra-core sorts on a
/// record's fields, and each is also the column's name.
pub const LIST: ListSpec = ListSpec {
    sort_keys: &[
        SortKey::text("name", "name"),
        SortKey::text("color", "color"),
        SortKey::plain("id", "id"),
    ],
    default_sort: "name",
    default_direction: Direction::Asc,
    filters: &[FilterKey {
        param: "id",
        column: "id",
        kind: FilterKind::Ids,
    }],
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use serde_json::{Value, json};

    use super::*;
    use crate::domain::{
        input::{FieldRule, char_count, is_color, trim},
        list_query::{
            Collation, FilterValue, ListQuery, ListQueryError, OrderTerm, TIE_BREAK, parse,
        },
        revision::{check_snapshot_type, read_snapshot, snapshot_object},
    };

    #[test]
    fn the_label_list_meets_the_list_contract() {
        assert_eq!(LIST.problems(), Vec::<String>::new());
    }

    #[test]
    fn labels_are_listed_by_name_in_the_hungarian_alphabet_then_by_id_descending() {
        let query = parse(&LIST, None).expect("the default");
        assert_eq!(query.page, 1);
        assert_eq!(query.per_page, 25);
        assert_eq!(
            query.order(),
            [
                OrderTerm {
                    column: "name",
                    collation: Collation::Hungarian,
                    direction: Direction::Asc,
                },
                TIE_BREAK,
            ]
        );
    }

    #[test]
    fn name_colour_and_id_are_the_sortable_fields() {
        for (field, collation) in [
            ("name", Collation::Hungarian),
            ("color", Collation::Hungarian),
            ("id", Collation::Default),
        ] {
            let query = parse(&LIST, Some(&format!("sort={field}&order=DESC"))).expect(field);
            assert_eq!(query.sort.column, field);
            assert_eq!(query.sort.collation, collation);
            assert_eq!(query.direction, Direction::Desc);
        }
        // The audit columns are not shown, so they are not sortable.
        for field in ["createdAt", "created_at", "deleted_at", "colour"] {
            assert_eq!(
                parse(&LIST, Some(&format!("sort={field}"))),
                Err(ListQueryError::Sort),
                "{field}"
            );
        }
    }

    #[test]
    fn the_picker_reads_labels_by_id_and_nothing_else_filters() {
        let query = parse(&LIST, Some("id=4&id=2")).expect("the picker's getMany");
        assert_eq!(query.filters.len(), 1);
        assert_eq!(query.filters[0].key.column, "id");
        assert_eq!(
            query.filters[0].value,
            FilterValue::Ids(BTreeSet::from([2, 4]))
        );
        // Search comes with its own slice (ADR 0020 §3), the trash filter
        // with the trash (ADR 0011 §8).
        for param in ["q", "name", "deleted", "trashed"] {
            assert_eq!(
                parse(&LIST, Some(&format!("{param}=x"))),
                Err(ListQueryError::UnknownFilter(param.to_owned())),
                "{param}"
            );
        }
    }

    proptest! {
        #[test]
        fn every_sort_of_the_label_list_ends_on_the_id(
            field in proptest::sample::select(vec!["name", "color", "id"]),
            descending in any::<bool>(),
            page in 1u32..1000,
        ) {
            let order = if descending { "DESC" } else { "ASC" };
            let query: ListQuery = parse(
                &LIST,
                Some(&format!("sort={field}&order={order}&page={page}")),
            )
            .expect("accepted");
            prop_assert_eq!(query.page, page);
            prop_assert_eq!(query.order().last().map(|term| term.column), Some("id"));
        }
    }

    /// The refusal of `field` for `rule`.
    fn refused(field: &'static str, rule: FieldRule) -> Result<LabelInput, FieldError> {
        Err(FieldError::new(field, rule))
    }

    #[test]
    fn a_label_is_stored_trimmed_and_otherwise_as_written() {
        let input = LabelInput::check("  Ünnepség és Sport \n", "\t#1e88E5 ").expect("valid");
        assert_eq!(input.name(), "Ünnepség és Sport");
        assert_eq!(input.color(), "#1e88E5");
        // A byte-order mark pasted with the name goes, as the admin trims it,
        // so it never makes a second label that looks like the first.
        let pasted = LabelInput::check("\u{feff}Hírek", "#000000").expect("valid");
        assert_eq!(pasted.name(), "Hírek");
    }

    #[test]
    fn the_name_takes_one_to_sixty_four_characters() {
        let longest = "Ő".repeat(NAME_MAX_CHARS);
        assert_eq!(
            LabelInput::check(&longest, "#000000").map(|input| input.name().to_owned()),
            Ok(longest.clone())
        );
        assert_eq!(
            LabelInput::check(&format!("{longest}x"), "#000000"),
            refused(NAME_FIELD, FieldRule::TooLong { max: 64 })
        );
        assert_eq!(
            LabelInput::check("x", "#000000").map(|input| input.name().to_owned()),
            Ok("x".to_owned())
        );
        for empty in ["", "   ", "\u{a0}\n", "\u{feff}"] {
            assert_eq!(
                LabelInput::check(empty, "#000000"),
                refused(NAME_FIELD, FieldRule::Required),
                "{empty:?}"
            );
        }
        assert_eq!(
            LabelInput::check("Hí\0rek", "#000000"),
            refused(NAME_FIELD, FieldRule::InvalidCharacter)
        );
    }

    #[test]
    fn the_colour_is_required_and_hash_and_six_hex_digits() {
        assert_eq!(
            LabelInput::check("Hírek", ""),
            refused(COLOR_FIELD, FieldRule::Required)
        );
        for invalid in ["1E88E5", "#1E88E", "#1E88E5F", "#GGGGGG", "piros"] {
            assert_eq!(
                LabelInput::check("Hírek", invalid),
                refused(COLOR_FIELD, FieldRule::InvalidColor),
                "{invalid}"
            );
        }
    }

    #[test]
    fn the_name_is_checked_before_the_colour() {
        // The form shows the name first; the envelope names one field.
        assert_eq!(
            LabelInput::check("", "piros"),
            refused(NAME_FIELD, FieldRule::Required)
        );
        assert_eq!(
            LabelInput::check(&"x".repeat(65), ""),
            refused(NAME_FIELD, FieldRule::TooLong { max: 64 })
        );
    }

    #[test]
    fn the_revision_snapshot_is_version_one_of_the_name_and_the_colour() {
        check_snapshot_type::<LabelRevision>().expect("a valid snapshot type");
        assert_eq!(LabelRevision::ENTITY_TYPE, "labels");
        assert_eq!(LabelRevision::SCHEMA_VERSION, 1);
        assert!(LabelRevision::UPCASTS.is_empty());
        let revision = LabelRevision {
            name: "Hírek".to_owned(),
            color: "#1E88E5".to_owned(),
        };
        let stored = snapshot_object(&revision).expect("the allow-list");
        assert_eq!(
            Value::Object(stored.clone()),
            json!({"name": "Hírek", "color": "#1E88E5"})
        );
        assert_eq!(
            read_snapshot::<LabelRevision>(1, Value::Object(stored)),
            Ok(revision)
        );
    }

    proptest! {
        #[test]
        fn an_accepted_label_is_trimmed_and_within_every_column_limit(
            name in any::<String>(),
            color in any::<String>(),
        ) {
            if let Ok(input) = LabelInput::check(&name, &color) {
                prop_assert_eq!(input.name(), trim(&name));
                prop_assert!(!input.name().is_empty());
                prop_assert!(char_count(input.name()) <= NAME_MAX_CHARS);
                prop_assert!(!input.name().contains('\0'));
                prop_assert_eq!(input.color(), trim(&color));
                prop_assert!(is_color(input.color()));
                // A stored label checks again to itself.
                prop_assert_eq!(
                    LabelInput::check(input.name(), input.color()),
                    Ok(input.clone())
                );
            }
        }

        #[test]
        fn names_up_to_the_limit_with_a_colour_are_accepted_and_longer_ones_refused(
            name in "[a-zA-ZáéíóöőúüűÁÉÍÓÖŐÚÜŰ0-9]{1,64}",
            extra in "[a-zőű]{1,20}",
            color in "#[0-9A-Fa-f]{6}",
        ) {
            prop_assert!(LabelInput::check(&name, &color).is_ok());
            let longer = format!("{}{extra}", "a".repeat(NAME_MAX_CHARS));
            prop_assert_eq!(
                LabelInput::check(&longer, &color),
                refused(NAME_FIELD, FieldRule::TooLong { max: NAME_MAX_CHARS })
            );
        }

        #[test]
        fn a_revision_of_any_label_reads_back_as_itself(
            name in any::<String>(),
            color in "#[0-9A-Fa-f]{6}",
        ) {
            let revision = LabelRevision { name, color };
            let stored = snapshot_object(&revision).expect("the allow-list");
            prop_assert_eq!(
                read_snapshot::<LabelRevision>(1, Value::Object(stored)),
                Ok(revision)
            );
        }
    }
}
