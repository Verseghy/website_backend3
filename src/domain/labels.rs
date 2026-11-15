//! The pure rules of labels, „címkék” (parity LBL-01; ADR 0009, ADR 0020).
//!
//! So far the list contract of `GET /api/admin/labels` ([`LIST`]): the
//! fields a list of labels may be sorted on and the filters it takes. The
//! label writes add their validation rules here (the name's and the
//! colour's limits of ADR 0009).

use super::list_query::{Direction, FilterKey, FilterKind, ListSpec, SortKey};

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

    use super::*;
    use crate::domain::list_query::{
        Collation, FilterValue, ListQuery, ListQueryError, OrderTerm, TIE_BREAK, parse,
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
}
