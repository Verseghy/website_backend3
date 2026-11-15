//! The query of an admin list endpoint: paging, sorting and filtering, the
//! shared admin list contract (parity SHR-02, SHR-03; ADR 0005, ADR 0020 §4).
//!
//! Why this module exists: every admin resource is listed the same way.
//! ra-core's list screens, its pickers and its `getMany` call all ask one
//! `GET /api/admin/<resource>` endpoint for a page of rows and the total, and
//! the answer is one shape (`dtos::admin::list::ListResponse`). This module
//! reads the endpoint's query string into a checked [`ListQuery`]; the model
//! turns that into SQL (`models::list_page`), and the controller turns a
//! refusal into the admin error envelope. Each resource declares what may be
//! sorted and filtered in a [`ListSpec`], an allow-list: a client can never
//! name a column the spec does not list, so no request text reaches the SQL
//! as an identifier.
//!
//! The parameters, read by the `application/x-www-form-urlencoded` rules
//! that browsers write (the same decoder as the public API's,
//! `super::v1_params::Query`):
//!
//! | Parameter | Absent | Present |
//! |---|---|---|
//! | `page` | 1 | ASCII digits, 1 to 4,294,967,295 |
//! | `perPage` | 25 | one of 10, 25, 50 and 100; there is no "all" (SHR-02) |
//! | `sort` | the spec's default field | a field of the spec's allow-list, compared exactly |
//! | `order` | the spec's default direction without `sort`, else `ASC` | `ASC` or `DESC`, as ra-core spells them |
//! | a filter of the spec | no filter | its kind's values ([`FilterKind`]) |
//!
//! `page`, `perPage`, `sort` and `order` may appear once each: a second value
//! would leave the meaning to a guess, so it is refused like a wrong value.
//! Every other parameter must be a filter of the spec; an unknown one is
//! refused, so a misspelt filter never silently lists everything. Leading
//! zeros are accepted (`07` is 7), as in the public API (public-api.md
//! §2.5).
//!
//! The order of the rows ([`ListQuery::order`]): the chosen field, text
//! fields in the Hungarian alphabet (`COLLATE hu_icu`, ADR 0020 §4), then
//! `id` as the tie-break, so the order is total and a row never appears on
//! two pages. A sort on `id` itself needs no tie-break.
//!
//! The tie-break is `id` descending, whichever way the field is sorted, as
//! parity SHR-03 says ("default sort per entity, then id descending") and as
//! backend1 appended `id DESC` to every list order (research: SYNTHESIS
//! §2.2). ADR 0020 §4 wrote the admin name sorts as
//! `ORDER BY <column> COLLATE hu_icu, id`, which is ascending, so the
//! maintainer was asked whether ties should stay `id` descending after every
//! sort, be ascending, or follow the direction of the sort, and answered
//! "Keep id descending after every sort (SHR-03, as built)" on 2026-11-15
//! (research: DECISIONS R37 (D157)). ADR 0020 §4 now reads
//! `<column> COLLATE hu_icu, id DESC` for admin lists; the public lists do
//! not use this contract and keep their own orders. So among rows with equal
//! sort values the highest `id` comes first, and flipping a column header
//! reverses the sort but not the order of the tied rows. The rule is the one
//! constant [`TIE_BREAK`].
//!
//! When several parameters are wrong, the answer names the first failing
//! check in this order, whatever order the query string has: `page`,
//! `perPage`, `sort`, `order`, an unknown parameter (the first one the query
//! names), then the filters' values in the order of the spec.
//! [`ListQueryError`]'s variants are declared in that order.

use std::{borrow::Cow, collections::BTreeSet, fmt};

use url::form_urlencoded;

use super::v1_params;

/// The query parameter of the page number, counted from 1.
pub const PAGE_PARAM: &str = "page";

/// The query parameter of the page size, ra-core's name for it.
pub const PER_PAGE_PARAM: &str = "perPage";

/// The query parameter of the sort field.
pub const SORT_PARAM: &str = "sort";

/// The query parameter of the sort direction.
pub const ORDER_PARAM: &str = "order";

/// The parameters that are not filters, which no [`ListSpec`] may use as a
/// filter's name.
pub const RESERVED_PARAMS: [&str; 4] = [PAGE_PARAM, PER_PAGE_PARAM, SORT_PARAM, ORDER_PARAM];

/// The page sizes an admin list offers (parity SHR-02): no other size, and
/// no "all", which would read a whole table into one answer.
pub const PAGE_SIZES: [u8; 4] = [10, 25, 50, 100];

/// The page size when the request sets none (parity SHR-02).
pub const DEFAULT_PER_PAGE: u8 = 25;

/// The most values an id-list filter takes ([`FilterKind::Ids`]): the
/// largest page, so every row it names fits on one page of the answer when
/// the request also sends `perPage=100`. The filter does not change the
/// page size: without `perPage`, a request for 26 to 100 ids gets the first
/// 25 rows, with a `total` that counts them all. So ra-core's `getMany` and
/// `getManyReference` send `perPage=100` and at most this many ids per
/// request, and split a larger set.
pub const MAX_FILTER_IDS: usize = 100;

/// The primary-key column of every entity table (ADR 0009), which the
/// tie-break sorts on.
pub const ID_COLUMN: &str = "id";

/// The tie-break after every sort but one on `id` itself: `id` descending,
/// whatever the sort's direction, as parity SHR-03 says and as the
/// maintainer answered on 2026-11-15 (research: DECISIONS R37 (D157); the
/// module documentation says why). ADR 0020 §4 gives the same order for
/// admin lists.
pub const TIE_BREAK: OrderTerm = OrderTerm {
    column: ID_COLUMN,
    collation: Collation::Default,
    direction: Direction::Desc,
};

/// A sort direction, as ra-core spells it in `order`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// `ASC`: smallest first; A to Zs for a name.
    Asc,
    /// `DESC`: largest first.
    Desc,
}

impl Direction {
    /// Both directions.
    pub const ALL: [Self; 2] = [Self::Asc, Self::Desc];

    /// The spelling in the query string and in SQL.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }

    /// The direction spelt exactly `name`, or `None`.
    ///
    /// - `name`: the value of `order`. The comparison is exact, as ra-core
    ///   writes it: `asc` and `Desc` are no direction.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|direction| direction.name() == name)
    }
}

impl fmt::Display for Direction {
    /// Writes [`Direction::name`].
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// How a column's values compare when a list sorts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Collation {
    /// The column's own order: numbers, instants, booleans.
    Default,
    /// The Hungarian alphabet, `COLLATE hu_icu` (ADR 0020 §4): every text
    /// column, so that `cs` follows every `c…` and `zs` every `z…`.
    Hungarian,
}

impl Collation {
    /// The SQL collation name to sort with, or `None` for the column's own
    /// order. `hu_icu` is the collation that the search-support migration
    /// creates (ADR 0020 §1).
    #[must_use]
    pub const fn sql_name(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Hungarian => Some("hu_icu"),
        }
    }
}

/// A field that a list may be sorted on: one entry of a [`ListSpec`]'s
/// allow-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SortKey {
    /// The name in `sort`, as the DTO serialises the field (ra-core sorts
    /// on the record's field names).
    pub field: &'static str,
    /// The column that holds it, a lower-case `snake_case` identifier.
    pub column: &'static str,
    /// How its values compare.
    pub collation: Collation,
}

impl SortKey {
    /// A text field, sorted in the Hungarian alphabet.
    ///
    /// - `field`: its name in `sort`.
    /// - `column`: its column.
    #[must_use]
    pub const fn text(field: &'static str, column: &'static str) -> Self {
        Self {
            field,
            column,
            collation: Collation::Hungarian,
        }
    }

    /// A field sorted in its column's own order: an id, a number or an
    /// instant.
    ///
    /// - `field`: its name in `sort`.
    /// - `column`: its column.
    #[must_use]
    pub const fn plain(field: &'static str, column: &'static str) -> Self {
        Self {
            field,
            column,
            collation: Collation::Default,
        }
    }
}

/// What a filter's values are.
///
/// One kind so far; the slices that add a list search (`q`, ADR 0020 §3) and
/// the trash filter (ADR 0011 §8) add theirs here with their checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterKind {
    /// One or more record ids, the parameter repeated once per id
    /// (`id=3&id=7`): the rows whose column holds one of them. Each id is
    /// ASCII digits from 1 to `v1_params::MAX_ID`, as a path id is, and at
    /// most [`MAX_FILTER_IDS`] are given; repeats count once. ra-core's
    /// `getMany`, which reads the records a form refers to, asks for them
    /// this way.
    Ids,
}

/// A filter that a list accepts: one entry of a [`ListSpec`]'s allow-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FilterKey {
    /// The query parameter's name.
    pub param: &'static str,
    /// The column it filters, a lower-case `snake_case` identifier.
    pub column: &'static str,
    /// What its values are.
    pub kind: FilterKind,
}

/// What one admin list accepts: its sort fields, its default order and its
/// filters. Each resource declares one as a constant beside its other pure
/// rules (for labels, `super::labels::LIST`).
///
/// [`ListSpec::problems`] holds a spec to the contract; every spec's unit
/// tests assert that it has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListSpec {
    /// The fields `sort` may name. One of them must sort on `id`, which
    /// ra-core asks for when a screen sets no sort of its own.
    pub sort_keys: &'static [SortKey],
    /// The field a request without `sort` is sorted on.
    pub default_sort: &'static str,
    /// The direction of a request without `sort` and `order`.
    pub default_direction: Direction,
    /// The filters the list accepts.
    pub filters: &'static [FilterKey],
}

impl ListSpec {
    /// The sort key of `field`, or `None` when the spec does not list it.
    ///
    /// - `field`: the value of `sort`, compared exactly.
    #[must_use]
    pub fn sort_key(&self, field: &str) -> Option<SortKey> {
        self.sort_keys
            .iter()
            .copied()
            .find(|key| key.field == field)
    }

    /// The filter called `param`, or `None` when the spec does not list it.
    ///
    /// - `param`: a query parameter's name, compared exactly.
    #[must_use]
    pub fn filter(&self, param: &str) -> Option<FilterKey> {
        self.filters.iter().copied().find(|key| key.param == param)
    }

    /// What is wrong with this spec against the list contract, one line per
    /// defect; empty for a spec the contract accepts.
    ///
    /// A spec is a constant in the code, so a defect is a programming
    /// error: each resource's unit tests assert that this is empty. The
    /// rules: at least one sort key; sort fields and filter names unique;
    /// a sort key on [`ID_COLUMN`], and every key on it plain
    /// ([`Collation::Default`]); the default sort among the keys; no
    /// filter named like a [`RESERVED_PARAMS`] parameter or an empty name;
    /// and every column a lower-case `snake_case` identifier, the only names
    /// the SQL may receive from a spec.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.sort_keys.is_empty() {
            problems.push("the spec has no sort key".to_owned());
        }
        let mut fields = BTreeSet::new();
        for key in self.sort_keys {
            if key.field.is_empty() {
                problems.push("a sort key has an empty field name".to_owned());
            }
            if !fields.insert(key.field) {
                problems.push(format!("sort field {:?} is listed twice", key.field));
            }
            if !is_identifier(key.column) {
                problems.push(format!(
                    "sort field {:?}: column {:?} is not a lower-case snake_case identifier",
                    key.field, key.column
                ));
            }
        }
        if !self.sort_keys.iter().any(|key| key.column == ID_COLUMN) {
            problems.push(format!(
                "no sort key on {ID_COLUMN:?}, which ra-core sorts on by default"
            ));
        }
        // A collation on the bigint id would make PostgreSQL refuse every
        // request sorted on it ("collations are not supported by type
        // bigint"), a 500 that no request test of another field would see.
        for key in self
            .sort_keys
            .iter()
            .filter(|key| key.column == ID_COLUMN && key.collation != Collation::Default)
        {
            problems.push(format!(
                "sort field {:?}: the sort key on {ID_COLUMN:?} must be SortKey::plain",
                key.field
            ));
        }
        if self.sort_key(self.default_sort).is_none() {
            problems.push(format!(
                "the default sort {:?} is not a sort key",
                self.default_sort
            ));
        }
        let mut params = BTreeSet::new();
        for key in self.filters {
            if key.param.is_empty() {
                problems.push("a filter has an empty name".to_owned());
            }
            if RESERVED_PARAMS.contains(&key.param) {
                problems.push(format!(
                    "filter {:?} has the name of a paging or sort parameter",
                    key.param
                ));
            }
            if !params.insert(key.param) {
                problems.push(format!("filter {:?} is listed twice", key.param));
            }
            if !is_identifier(key.column) {
                problems.push(format!(
                    "filter {:?}: column {:?} is not a lower-case snake_case identifier",
                    key.param, key.column
                ));
            }
        }
        problems
    }
}

/// The values of one filter of a request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterValue {
    /// The ids of a [`FilterKind::Ids`] filter, each once, in ascending
    /// order; never empty.
    Ids(BTreeSet<i64>),
}

/// One filter of a request: which filter, and its checked values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Filter {
    /// The spec's filter.
    pub key: FilterKey,
    /// Its values.
    pub value: FilterValue,
}

/// One term of the `ORDER BY` of a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderTerm {
    /// The column, from the spec.
    pub column: &'static str,
    /// How its values compare.
    pub collation: Collation,
    /// The direction.
    pub direction: Direction,
}

/// A checked list request: which page, how large, in which order, and
/// filtered how.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListQuery {
    /// The page, counted from 1.
    pub page: u32,
    /// The page size, one of [`PAGE_SIZES`].
    pub per_page: u8,
    /// The sort field.
    pub sort: SortKey,
    /// The sort direction.
    pub direction: Direction,
    /// The filters, at most one per filter of the spec, in the spec's
    /// order; the rows must match all of them.
    pub filters: Vec<Filter>,
}

impl ListQuery {
    /// How many rows come before the page: `(page - 1) × perPage`. At most
    /// about 4.3 × 10¹¹, so it never overflows.
    #[must_use]
    pub fn offset(&self) -> u64 {
        (u64::from(self.page) - 1) * u64::from(self.per_page)
    }

    /// How many rows the page holds at most: `perPage`.
    #[must_use]
    pub fn limit(&self) -> u64 {
        u64::from(self.per_page)
    }

    /// The `ORDER BY` terms: the sort field in the requested direction, then
    /// [`TIE_BREAK`] unless the sort field is `id` itself.
    #[must_use]
    pub fn order(&self) -> Vec<OrderTerm> {
        let sort = OrderTerm {
            column: self.sort.column,
            collation: self.sort.collation,
            direction: self.direction,
        };
        if self.sort.column == ID_COLUMN {
            vec![sort]
        } else {
            vec![sort, TIE_BREAK]
        }
    }
}

/// Why a list request was refused: the first failing check (the module
/// documentation gives the order, which the variants follow, so the derived
/// `Ord` is that order).
///
/// The message is the English `description` of the admin error envelope
/// (ADR 0021 §4).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, thiserror::Error)]
pub enum ListQueryError {
    /// `page` is not a whole number from 1 to 4,294,967,295, or appears
    /// twice.
    #[error("page must appear at most once, as a whole number from 1 to 4294967295")]
    Page,
    /// `perPage` is not one of [`PAGE_SIZES`], or appears twice.
    #[error("perPage must appear at most once, as 10, 25, 50 or 100")]
    PerPage,
    /// `sort` names no sort field of the list, or appears twice.
    #[error("sort must appear at most once, naming a field this list can be sorted on")]
    Sort,
    /// `order` is neither `ASC` nor `DESC`, or appears twice.
    #[error("order must appear at most once, as ASC or DESC")]
    Order,
    /// A parameter that is neither a paging or sort parameter nor a filter
    /// of the list; it holds the parameter's name as the request sent it.
    #[error("the parameter {0:?} is not a filter of this list")]
    UnknownFilter(String),
    /// A filter's values are not what its kind takes; it holds the
    /// filter's name. Its message describes [`FilterKind::Ids`], the only
    /// kind so far: a slice that adds a kind gives the kind's refusal its
    /// own text.
    // The limits come from their constants, so that the description
    // follows a change of either.
    #[error(
        "the filter {0:?} takes one to {max} record ids, each a whole number from 1 to {id}",
        max = MAX_FILTER_IDS,
        id = v1_params::MAX_ID
    )]
    Filter(&'static str),
}

impl ListQueryError {
    /// The query parameter the refusal is about, as the request names it:
    /// the envelope's `field` (ADR 0021 §4).
    #[must_use]
    pub fn parameter(&self) -> &str {
        match self {
            Self::Page => PAGE_PARAM,
            Self::PerPage => PER_PAGE_PARAM,
            Self::Sort => SORT_PARAM,
            Self::Order => ORDER_PARAM,
            Self::UnknownFilter(name) => name,
            Self::Filter(name) => name,
        }
    }
}

/// Reads and checks the query string of a list request.
///
/// - `spec`: what the list accepts.
/// - `raw`: the query string as the request carried it, without the leading
///   `?` and still percent-encoded (axum's `RawQuery`), or `None` when the
///   request had none.
///
/// # Errors
///
/// The first [`ListQueryError`] in the order the module documentation gives.
pub fn parse(spec: &ListSpec, raw: Option<&str>) -> Result<ListQuery, ListQueryError> {
    let pairs: Vec<(Cow<'_, str>, Cow<'_, str>)> = raw
        .map(|raw| form_urlencoded::parse(raw.as_bytes()).collect())
        .unwrap_or_default();
    let values_of = |name: &str| -> Vec<&str> {
        pairs
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_ref())
            .collect()
    };

    let page = match values_of(PAGE_PARAM).as_slice() {
        [] => 1,
        [value] => whole_number(value)
            .and_then(|page| u32::try_from(page).ok())
            .filter(|page| *page >= 1)
            .ok_or(ListQueryError::Page)?,
        _ => return Err(ListQueryError::Page),
    };
    let per_page = match values_of(PER_PAGE_PARAM).as_slice() {
        [] => DEFAULT_PER_PAGE,
        [value] => whole_number(value)
            .and_then(|size| u8::try_from(size).ok())
            .filter(|size| PAGE_SIZES.contains(size))
            .ok_or(ListQueryError::PerPage)?,
        _ => return Err(ListQueryError::PerPage),
    };
    let sort_values = values_of(SORT_PARAM);
    let sort = match sort_values.as_slice() {
        [] => spec
            .sort_key(spec.default_sort)
            .ok_or(ListQueryError::Sort)?,
        [value] => spec.sort_key(value).ok_or(ListQueryError::Sort)?,
        _ => return Err(ListQueryError::Sort),
    };
    let direction = match values_of(ORDER_PARAM).as_slice() {
        // Without `sort`, the list's own default order; with it, ascending,
        // the first click on a column header.
        [] if sort_values.is_empty() => spec.default_direction,
        [] => Direction::Asc,
        [value] => Direction::from_name(value).ok_or(ListQueryError::Order)?,
        _ => return Err(ListQueryError::Order),
    };

    if let Some((name, _)) = pairs
        .iter()
        .find(|(name, _)| !RESERVED_PARAMS.contains(&name.as_ref()) && spec.filter(name).is_none())
    {
        return Err(ListQueryError::UnknownFilter(name.clone().into_owned()));
    }
    let mut filters = Vec::new();
    for key in spec.filters {
        let values = values_of(key.param);
        if values.is_empty() {
            continue;
        }
        let value = match key.kind {
            FilterKind::Ids => {
                FilterValue::Ids(ids(&values).ok_or(ListQueryError::Filter(key.param))?)
            }
        };
        filters.push(Filter { key: *key, value });
    }

    Ok(ListQuery {
        page,
        per_page,
        sort,
        direction,
        filters,
    })
}

/// The ids of an id-list filter's values: each a path-style id
/// (`v1_params::id`), at most [`MAX_FILTER_IDS`] values, repeats counted
/// once; `None` when any value is not an id or there are too many.
fn ids(values: &[&str]) -> Option<BTreeSet<i64>> {
    if values.len() > MAX_FILTER_IDS {
        return None;
    }
    values
        .iter()
        .map(|value| v1_params::id(value).ok())
        .collect()
}

/// The value of `raw` when it is ASCII digits only, leading zeros allowed;
/// `None` otherwise or when the value does not fit in a `u64`.
///
/// Rust's integer parsing accepts a leading `+`, so the digits are checked
/// first.
fn whole_number(raw: &str) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let significant = raw.trim_start_matches('0');
    if significant.is_empty() {
        return Some(0);
    }
    significant.parse().ok()
}

/// Whether `name` is a lower-case `snake_case` SQL identifier: a lower-case
/// ASCII letter or `_`, then lower-case ASCII letters, digits and `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && chars.all(|rest| rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '_')
}

#[cfg(test)]
mod tests {
    use proptest::{prelude::*, sample::subsequence};

    use super::*;

    /// A probe spec: two text fields, an id and an instant, a default sort
    /// that is descending, and two id-list filters. Invented names: no
    /// resource slice owns them.
    const PROBE: ListSpec = ListSpec {
        sort_keys: &[
            SortKey::text("title", "title"),
            SortKey::text("authorName", "author_name"),
            SortKey::plain("id", "id"),
            SortKey::plain("createdAt", "created_at"),
        ],
        default_sort: "createdAt",
        default_direction: Direction::Desc,
        filters: &[
            FilterKey {
                param: "id",
                column: "id",
                kind: FilterKind::Ids,
            },
            FilterKey {
                param: "authorId",
                column: "author_id",
                kind: FilterKind::Ids,
            },
        ],
    };

    /// `parse` over [`PROBE`].
    fn probe(raw: &str) -> Result<ListQuery, ListQueryError> {
        parse(&PROBE, Some(raw))
    }

    /// The probe's query with only the paging and the sort set.
    fn query(page: u32, per_page: u8, field: &str, direction: Direction) -> ListQuery {
        ListQuery {
            page,
            per_page,
            sort: PROBE.sort_key(field).expect("a probe field"),
            direction,
            filters: Vec::new(),
        }
    }

    #[test]
    fn the_probe_spec_meets_the_contract() {
        assert_eq!(PROBE.problems(), Vec::<String>::new());
    }

    #[test]
    fn the_spec_looks_up_fields_and_filters_exactly() {
        assert_eq!(
            PROBE.sort_key("authorName"),
            Some(SortKey::text("authorName", "author_name"))
        );
        assert_eq!(PROBE.filter("authorId"), Some(PROBE.filters[1]));
        // By the request's name, never by the column's, and case-sensitive.
        for name in ["author_name", "AuthorName", "authorname", ""] {
            assert_eq!(PROBE.sort_key(name), None, "{name:?}");
        }
        for name in ["author_id", "AUTHORID", "ids", ""] {
            assert_eq!(PROBE.filter(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_request_without_a_query_gets_the_first_page_of_25_in_the_default_order() {
        let expected = query(1, 25, "createdAt", Direction::Desc);
        assert_eq!(parse(&PROBE, None), Ok(expected.clone()));
        assert_eq!(probe(""), Ok(expected.clone()));
        // A lone `&` is no parameter.
        assert_eq!(probe("&&"), Ok(expected));
        assert_eq!(DEFAULT_PER_PAGE, 25);
    }

    #[test]
    fn the_page_sizes_are_those_of_shr_02() {
        assert_eq!(PAGE_SIZES, [10, 25, 50, 100]);
        for size in PAGE_SIZES {
            assert_eq!(
                probe(&format!("perPage={size}")).map(|query| query.per_page),
                Ok(size)
            );
        }
        assert_eq!(probe("perPage=0025").map(|query| query.per_page), Ok(25));
        for refused in [
            "0", "1", "5", "20", "24", "26", "99", "101", "1000", "256", "-1", "+25", " 25", "25 ",
            "", "all", "All", "25.0", "2.5e1", "0x19",
        ] {
            assert_eq!(
                probe(&format!("perPage={refused}")),
                Err(ListQueryError::PerPage),
                "{refused:?}"
            );
        }
        // No value at all is an empty value.
        assert_eq!(probe("perPage"), Err(ListQueryError::PerPage));
    }

    #[test]
    fn pages_count_from_one_to_the_32_bit_limit() {
        for (raw, page) in [("1", 1), ("2", 2), ("007", 7), ("4294967295", u32::MAX)] {
            assert_eq!(
                probe(&format!("page={raw}")).map(|query| query.page),
                Ok(page),
                "{raw:?}"
            );
        }
        for refused in [
            "0",
            "000",
            "4294967296",
            "99999999999999999999999",
            "-1",
            "+1",
            "1.5",
            " 1",
            "",
            "egy",
            "\u{ff11}",
        ] {
            assert_eq!(
                probe(&format!("page={refused}")),
                Err(ListQueryError::Page),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn the_offset_and_limit_follow_the_page() {
        let first = query(1, 25, "id", Direction::Asc);
        assert_eq!((first.offset(), first.limit()), (0, 25));
        let third = query(3, 10, "id", Direction::Asc);
        assert_eq!((third.offset(), third.limit()), (20, 10));
        let last = query(u32::MAX, 100, "id", Direction::Asc);
        assert_eq!(last.offset(), (u64::from(u32::MAX) - 1) * 100);
    }

    #[test]
    fn sort_names_a_field_of_the_allow_list_exactly() {
        assert_eq!(
            probe("sort=title&order=DESC"),
            Ok(query(1, 25, "title", Direction::Desc))
        );
        assert_eq!(
            probe("sort=authorName"),
            Ok(query(1, 25, "authorName", Direction::Asc))
        );
        for refused in [
            "Title",
            "title ",
            "author_name",
            "createdat",
            "password_hash",
            "",
            "id,title",
        ] {
            assert_eq!(
                probe(&format!("sort={refused}")),
                Err(ListQueryError::Sort),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn order_is_asc_or_desc_and_defaults_by_whether_sort_was_given() {
        // Neither: the spec's default field and direction.
        assert_eq!(probe(""), Ok(query(1, 25, "createdAt", Direction::Desc)));
        // `order` alone turns the default field around.
        assert_eq!(
            probe("order=ASC"),
            Ok(query(1, 25, "createdAt", Direction::Asc))
        );
        // `sort` alone is ascending, even on the default field.
        assert_eq!(
            probe("sort=createdAt"),
            Ok(query(1, 25, "createdAt", Direction::Asc))
        );
        for refused in ["asc", "Desc", "DESCENDING", "", "1", " ASC"] {
            assert_eq!(
                probe(&format!("order={refused}")),
                Err(ListQueryError::Order),
                "{refused:?}"
            );
        }
        assert_eq!(Direction::from_name("ASC"), Some(Direction::Asc));
        assert_eq!(Direction::Desc.to_string(), "DESC");
    }

    #[test]
    fn a_paging_or_sort_parameter_given_twice_is_refused() {
        for (raw, error) in [
            ("page=1&page=1", ListQueryError::Page),
            ("page=1&page=2", ListQueryError::Page),
            ("perPage=25&perPage=25", ListQueryError::PerPage),
            ("sort=title&sort=id", ListQueryError::Sort),
            ("order=ASC&order=ASC", ListQueryError::Order),
        ] {
            assert_eq!(probe(raw), Err(error), "{raw}");
        }
    }

    #[test]
    fn an_unknown_parameter_is_refused_and_named() {
        for (raw, name) in [
            ("q=alma", "q"),
            ("ID=1", "ID"),
            ("filter=%7B%7D", "filter"),
            ("_sort=title", "_sort"),
            ("per_page=25", "per_page"),
            ("=1", ""),
            ("a%20b=1", "a b"),
            // The first unknown one, in request order.
            ("x=1&y=2", "x"),
            ("page=2&y=2&x=1", "y"),
        ] {
            assert_eq!(
                probe(raw),
                Err(ListQueryError::UnknownFilter(name.to_owned())),
                "{raw}"
            );
        }
    }

    #[test]
    fn an_id_list_filter_reads_each_id_once() {
        let filtered = probe("id=7&id=3&id=7&authorId=0012").expect("accepted");
        assert_eq!(
            filtered.filters,
            [
                Filter {
                    key: PROBE.filters[0],
                    value: FilterValue::Ids(BTreeSet::from([3, 7])),
                },
                Filter {
                    key: PROBE.filters[1],
                    value: FilterValue::Ids(BTreeSet::from([12])),
                },
            ]
        );
        // The filters come in the spec's order, whatever the query's.
        assert_eq!(
            probe("authorId=1&id=2").expect("accepted").filters,
            probe("id=2&authorId=1").expect("accepted").filters
        );
        let hundred: Vec<String> = (1..=100).map(|id| format!("id={id}")).collect();
        let at_the_cap = probe(&hundred.join("&")).expect("100 ids");
        assert_eq!(
            at_the_cap.filters[0].value,
            FilterValue::Ids((1..=100).collect())
        );
        assert_eq!(MAX_FILTER_IDS, 100);
    }

    #[test]
    fn an_id_list_filter_refuses_what_is_no_id_and_more_than_100() {
        for value in [
            "0",
            "-1",
            "+1",
            "",
            "abc",
            "1.0",
            "9007199254740992",
            "1,2",
            " 1",
        ] {
            assert_eq!(
                probe(&format!("id={value}")),
                Err(ListQueryError::Filter("id")),
                "{value:?}"
            );
        }
        assert_eq!(probe("id=1&id=x&id=2"), Err(ListQueryError::Filter("id")));
        assert_eq!(
            probe("id=9007199254740991")
                .expect("the largest id")
                .filters[0]
                .value,
            FilterValue::Ids(BTreeSet::from([v1_params::MAX_ID]))
        );
        // 101 values are refused, even when they repeat one id.
        let too_many = vec!["id=1"; 101].join("&");
        assert_eq!(probe(&too_many), Err(ListQueryError::Filter("id")));
    }

    #[test]
    fn the_first_failing_check_wins_whatever_the_query_order() {
        assert_eq!(
            probe("q=x&order=up&sort=nope&perPage=7&page=0"),
            Err(ListQueryError::Page)
        );
        assert_eq!(
            probe("q=x&order=up&sort=nope&perPage=7"),
            Err(ListQueryError::PerPage)
        );
        assert_eq!(probe("q=x&order=up&sort=nope"), Err(ListQueryError::Sort));
        assert_eq!(probe("id=x&q=x&order=up"), Err(ListQueryError::Order));
        // An unknown name before a bad value, wherever it is.
        assert_eq!(
            probe("id=x&q=x"),
            Err(ListQueryError::UnknownFilter("q".to_owned()))
        );
        // Then the filters in the spec's order.
        assert_eq!(probe("authorId=x&id=x"), Err(ListQueryError::Filter("id")));
        // The derived order is the check order.
        let mut errors = [
            ListQueryError::Filter("id"),
            ListQueryError::UnknownFilter("q".to_owned()),
            ListQueryError::Order,
            ListQueryError::Sort,
            ListQueryError::PerPage,
            ListQueryError::Page,
        ];
        errors.sort();
        assert_eq!(
            errors,
            [
                ListQueryError::Page,
                ListQueryError::PerPage,
                ListQueryError::Sort,
                ListQueryError::Order,
                ListQueryError::UnknownFilter("q".to_owned()),
                ListQueryError::Filter("id"),
            ]
        );
    }

    #[test]
    fn the_query_is_decoded_as_a_browser_encodes_it() {
        // `+` is a space and `%2B` a plus, so neither is a valid number.
        assert_eq!(probe("page=%2B1"), Err(ListQueryError::Page));
        assert_eq!(probe("page=+1"), Err(ListQueryError::Page));
        // A percent-encoded name is the decoded name.
        assert_eq!(probe("per%50age=10").map(|query| query.per_page), Ok(10));
        // Bytes that are not UTF-8 become U+FFFD, which names nothing.
        assert_eq!(probe("sort=%FF"), Err(ListQueryError::Sort));
    }

    #[test]
    fn every_refusal_names_its_parameter_and_says_what_is_accepted() {
        for (error, parameter, words) in [
            (ListQueryError::Page, "page", "1 to 4294967295"),
            (ListQueryError::PerPage, "perPage", "10, 25, 50 or 100"),
            (ListQueryError::Sort, "sort", "can be sorted on"),
            (ListQueryError::Order, "order", "ASC or DESC"),
            (
                ListQueryError::UnknownFilter("q".to_owned()),
                "q",
                "\"q\" is not a filter",
            ),
            (ListQueryError::Filter("id"), "id", "one to 100 record ids"),
        ] {
            assert_eq!(error.parameter(), parameter, "{error:?}");
            let message = error.to_string();
            assert!(message.contains(words), "{message}");
        }
        // The filter's limits are read from their constants.
        assert_eq!(
            ListQueryError::Filter("id").to_string(),
            format!(
                "the filter \"id\" takes one to {MAX_FILTER_IDS} record ids, each a whole number \
                 from 1 to {}",
                v1_params::MAX_ID
            )
        );
    }

    #[test]
    fn the_order_ends_with_the_id_tie_break() {
        assert_eq!(
            query(1, 25, "title", Direction::Asc).order(),
            [
                OrderTerm {
                    column: "title",
                    collation: Collation::Hungarian,
                    direction: Direction::Asc,
                },
                TIE_BREAK,
            ]
        );
        // The tie-break is descending whatever the sort's direction (SHR-03).
        assert_eq!(
            query(1, 25, "createdAt", Direction::Desc).order(),
            [
                OrderTerm {
                    column: "created_at",
                    collation: Collation::Default,
                    direction: Direction::Desc,
                },
                TIE_BREAK,
            ]
        );
        assert_eq!(
            TIE_BREAK,
            OrderTerm {
                column: "id",
                collation: Collation::Default,
                direction: Direction::Desc,
            }
        );
        // A sort on the id is total by itself.
        for direction in Direction::ALL {
            assert_eq!(
                query(1, 25, "id", direction).order(),
                [OrderTerm {
                    column: "id",
                    collation: Collation::Default,
                    direction,
                }]
            );
        }
    }

    #[test]
    fn text_sorts_in_the_hungarian_alphabet() {
        assert_eq!(Collation::Hungarian.sql_name(), Some("hu_icu"));
        assert_eq!(Collation::Default.sql_name(), None);
        assert_eq!(
            SortKey::text("name", "name").collation,
            Collation::Hungarian
        );
        assert_eq!(SortKey::plain("id", "id").collation, Collation::Default);
    }

    #[test]
    fn the_spec_check_names_every_defect() {
        // Constants, because a `const fn` call in a `&[...]` is not promoted
        // to `'static`, which a spec's slices need.
        const ID: SortKey = SortKey::plain("id", "id");
        const NAME: SortKey = SortKey::text("name", "name");
        const EMPTY_FIELD: SortKey = SortKey::plain("", "x");
        const CAPITAL_COLUMN: SortKey = SortKey::text("name", "Name");
        const INJECTED_COLUMN: SortKey = SortKey::text("name", "name; DROP");
        // PostgreSQL refuses a collation on the bigint id.
        const TEXT_ID: SortKey = SortKey::text("id", "id");
        const IDS: FilterKey = FilterKey {
            param: "id",
            column: "id",
            kind: FilterKind::Ids,
        };
        const NAMED_SORT: FilterKey = FilterKey {
            param: "sort",
            column: "id",
            kind: FilterKind::Ids,
        };
        const UNNAMED: FilterKey = FilterKey {
            param: "",
            column: "id",
            kind: FilterKind::Ids,
        };
        const DIGIT_COLUMN: FilterKey = FilterKey {
            param: "x",
            column: "1x",
            kind: FilterKind::Ids,
        };
        let spec = |sort_keys: &'static [SortKey],
                    default_sort: &'static str,
                    filters: &'static [FilterKey]| ListSpec {
            sort_keys,
            default_sort,
            default_direction: Direction::Asc,
            filters,
        };
        for (defective, defect) in [
            (spec(&[], "id", &[]), "no sort key"),
            (spec(&[ID, ID], "id", &[]), "listed twice"),
            (spec(&[EMPTY_FIELD, ID], "id", &[]), "empty field"),
            (
                spec(&[CAPITAL_COLUMN, ID], "id", &[]),
                "not a lower-case snake_case identifier",
            ),
            (
                spec(&[INJECTED_COLUMN, ID], "id", &[]),
                "not a lower-case snake_case identifier",
            ),
            (spec(&[NAME], "name", &[]), "no sort key on \"id\""),
            (
                spec(&[NAME, TEXT_ID], "name", &[]),
                "the sort key on \"id\" must be SortKey::plain",
            ),
            (spec(&[ID], "name", &[]), "default sort"),
            (
                spec(&[ID], "id", &[IDS, IDS]),
                "filter \"id\" is listed twice",
            ),
            (spec(&[ID], "id", &[NAMED_SORT]), "paging or sort parameter"),
            (spec(&[ID], "id", &[UNNAMED]), "empty name"),
            (
                spec(&[ID], "id", &[DIGIT_COLUMN]),
                "not a lower-case snake_case identifier",
            ),
        ] {
            let problems = defective.problems();
            assert!(
                problems.iter().any(|problem| problem.contains(defect)),
                "{defect}: {problems:?}"
            );
        }
        assert_eq!(spec(&[ID], "id", &[IDS]).problems(), Vec::<String>::new());
    }

    #[test]
    fn a_spec_without_its_default_sort_refuses_a_request_without_sort() {
        // `problems` catches this before any request; the parser still
        // answers it without panicking.
        const BROKEN: ListSpec = ListSpec {
            sort_keys: &[SortKey::plain("id", "id")],
            default_sort: "name",
            default_direction: Direction::Asc,
            filters: &[],
        };
        let broken = BROKEN;
        assert_eq!(parse(&broken, None), Err(ListQueryError::Sort));
    }

    /// A query string over [`PROBE`] that `parse` accepts, with the query it
    /// stands for: the parameters in a shuffled order, ids possibly
    /// repeated.
    fn valid_query() -> impl Strategy<Value = (String, ListQuery)> {
        (
            proptest::option::of(1u32..),
            proptest::option::of(proptest::sample::select(PAGE_SIZES.to_vec())),
            proptest::option::of(proptest::sample::select(PROBE.sort_keys.to_vec())),
            proptest::option::of(proptest::sample::select(Direction::ALL.to_vec())),
            proptest::collection::vec(1..=v1_params::MAX_ID, 0..=MAX_FILTER_IDS),
            // Few distinct values, so that repeats occur.
            proptest::collection::vec(1..=5i64, 0..=6),
        )
            .prop_map(|(page, per_page, sort, direction, ids, author_ids)| {
                let mut params = Vec::new();
                if let Some(page) = page {
                    params.push(format!("page={page}"));
                }
                if let Some(per_page) = per_page {
                    params.push(format!("perPage={per_page}"));
                }
                if let Some(sort) = sort {
                    params.push(format!("sort={}", sort.field));
                }
                if let Some(direction) = direction {
                    params.push(format!("order={direction}"));
                }
                params.extend(ids.iter().map(|id| format!("id={id}")));
                params.extend(author_ids.iter().map(|id| format!("authorId={id}")));
                let mut filters = Vec::new();
                if !ids.is_empty() {
                    filters.push(Filter {
                        key: PROBE.filters[0],
                        value: FilterValue::Ids(ids.iter().copied().collect()),
                    });
                }
                if !author_ids.is_empty() {
                    filters.push(Filter {
                        key: PROBE.filters[1],
                        value: FilterValue::Ids(author_ids.iter().copied().collect()),
                    });
                }
                let direction = direction.unwrap_or(if sort.is_none() {
                    PROBE.default_direction
                } else {
                    Direction::Asc
                });
                let query = ListQuery {
                    page: page.unwrap_or(1),
                    per_page: per_page.unwrap_or(DEFAULT_PER_PAGE),
                    sort: sort.unwrap_or_else(|| {
                        PROBE.sort_key(PROBE.default_sort).expect("the default")
                    }),
                    direction,
                    filters,
                };
                (params, query)
            })
            .prop_flat_map(|(params, query)| (Just(params).prop_shuffle(), Just(query)))
            .prop_map(|(params, query)| (params.join("&"), query))
    }

    /// A lower-case `snake_case` identifier other than `id` and the names of
    /// the paging and sort parameters, so that a generated spec stays valid.
    fn identifier() -> impl Strategy<Value = String> {
        "[a-z_][a-z0-9_]{0,11}".prop_filter("neither id nor a reserved name", |name| {
            name != ID_COLUMN && !RESERVED_PARAMS.contains(&name.as_str())
        })
    }

    /// A spec built from generated names: a plain key on `id`, a text key
    /// per column of `columns` (its field named like the column), an
    /// id-list filter per column of `filter_columns` (named like it), and
    /// the default sort picked by `default` among the keys. Its slices are
    /// leaked to get the `'static` slices a spec holds; a test process ends
    /// soon.
    ///
    /// - `columns`: the text columns.
    /// - `filter_columns`: the filtered columns.
    /// - `default`: picks the default sort, modulo the number of keys.
    /// - `direction`: the default direction.
    fn spec_of(
        columns: &BTreeSet<String>,
        filter_columns: &BTreeSet<String>,
        default: usize,
        direction: Direction,
    ) -> ListSpec {
        let leak = |name: &String| -> &'static str { String::leak(name.clone()) };
        let mut sort_keys = vec![SortKey::plain(ID_COLUMN, ID_COLUMN)];
        sort_keys.extend(columns.iter().map(|column| {
            let column = leak(column);
            SortKey::text(column, column)
        }));
        let filters: Vec<FilterKey> = filter_columns
            .iter()
            .map(|column| {
                let column = leak(column);
                FilterKey {
                    param: column,
                    column,
                    kind: FilterKind::Ids,
                }
            })
            .collect();
        let default_sort = sort_keys[default % sort_keys.len()].field;
        ListSpec {
            sort_keys: Vec::leak(sort_keys),
            default_sort,
            default_direction: direction,
            filters: Vec::leak(filters),
        }
    }

    /// A name that the probe lists (a sort field, a filter, or one of their
    /// columns), as it is or with some letters in the other case, or any
    /// name at all.
    fn listed_or_any_name() -> impl Strategy<Value = String> {
        let listed: Vec<&'static str> = PROBE
            .sort_keys
            .iter()
            .flat_map(|key| [key.field, key.column])
            .chain(PROBE.filters.iter().flat_map(|key| [key.param, key.column]))
            .collect();
        prop_oneof![
            proptest::sample::select(listed.clone()).prop_map(str::to_owned),
            (
                proptest::sample::select(listed),
                proptest::collection::vec(any::<bool>(), 12),
            )
                .prop_map(|(name, flips)| {
                    name.chars()
                        .zip(flips.into_iter().chain(std::iter::repeat(false)))
                        .map(|(letter, flip)| match (flip, letter.is_ascii_lowercase()) {
                            (false, _) => letter,
                            (true, true) => letter.to_ascii_uppercase(),
                            (true, false) => letter.to_ascii_lowercase(),
                        })
                        .collect()
                }),
            "\\PC{0,12}",
        ]
    }

    /// A query string over the probe that is often refused: up to five
    /// parameters, each a name the probe knows or an unknown one with a
    /// value that may be wrong for it; or any text.
    fn refused_or_any_query() -> impl Strategy<Value = String> {
        let names = vec![
            "page", "perPage", "sort", "order", "id", "authorId", "q", "",
        ];
        let values = vec![
            "1", "0", "25", "30", "title", "nope", "ASC", "asc", "7", "x", "",
        ];
        prop_oneof![
            proptest::collection::vec(
                (
                    proptest::sample::select(names),
                    proptest::sample::select(values),
                ),
                0..6,
            )
            .prop_map(|pairs| {
                pairs
                    .iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join("&")
            }),
            "\\PC{0,40}",
        ]
    }

    proptest! {
        #[test]
        fn every_valid_query_reads_back_as_the_query_it_stands_for(
            (raw, expected) in valid_query(),
        ) {
            prop_assert_eq!(probe(&raw), Ok(expected));
        }

        #[test]
        fn any_query_string_is_answered_without_a_panic(raw in "\\PC{0,80}") {
            // Whatever comes back obeys the contract.
            if let Ok(query) = probe(&raw) {
                prop_assert!(query.page >= 1);
                prop_assert!(PAGE_SIZES.contains(&query.per_page));
                prop_assert!(PROBE.sort_keys.contains(&query.sort));
                for filter in &query.filters {
                    prop_assert!(PROBE.filters.contains(&filter.key));
                    let FilterValue::Ids(ids) = &filter.value;
                    prop_assert!(!ids.is_empty() && ids.len() <= MAX_FILTER_IDS);
                    prop_assert!(ids.iter().all(|id| (1..=v1_params::MAX_ID).contains(id)));
                }
            }
        }

        #[test]
        fn one_wrong_parameter_is_the_one_refused(
            (raw, _) in valid_query(),
            wrong in prop_oneof![
                Just(("page=0", ListQueryError::Page)),
                Just(("perPage=30", ListQueryError::PerPage)),
                Just(("sort=secret", ListQueryError::Sort)),
                Just(("order=desc", ListQueryError::Order)),
                Just(("bogus=1", ListQueryError::UnknownFilter("bogus".to_owned()))),
            ],
        ) {
            // Replacing (or adding) the one parameter makes exactly its check
            // fail, wherever it stands.
            let name = wrong.0.split('=').next().unwrap_or_default();
            let kept: Vec<&str> = raw
                .split('&')
                .filter(|pair| !pair.is_empty() && pair.split('=').next() != Some(name))
                .collect();
            for at in [0, kept.len()] {
                let mut params = kept.clone();
                params.insert(at, wrong.0);
                prop_assert_eq!(probe(&params.join("&")), Err(wrong.1.clone()));
            }
        }

        #[test]
        fn a_non_digit_page_is_always_refused(raw in "[^0-9&=#%+]{1,12}") {
            prop_assert_eq!(probe(&format!("page={raw}")), Err(ListQueryError::Page));
        }

        #[test]
        fn the_order_is_total(field in proptest::sample::select(PROBE.sort_keys.to_vec()),
                              direction in proptest::sample::select(Direction::ALL.to_vec())) {
            let order = ListQuery {
                page: 1,
                per_page: 25,
                sort: field,
                direction,
                filters: Vec::new(),
            }
            .order();
            // It starts with the chosen field and ends on the id, once.
            prop_assert_eq!(order[0].column, field.column);
            prop_assert_eq!(order[0].direction, direction);
            prop_assert_eq!(order.last().map(|term| term.column), Some(ID_COLUMN));
            prop_assert_eq!(order.iter().filter(|term| term.column == ID_COLUMN).count(), 1);
        }

        #[test]
        fn the_offset_is_the_rows_of_the_pages_before(
            page in 1u32..,
            per_page in proptest::sample::select(PAGE_SIZES.to_vec()),
        ) {
            let query = ListQuery {
                page,
                per_page,
                sort: SortKey::plain("id", "id"),
                direction: Direction::Asc,
                filters: Vec::new(),
            };
            prop_assert_eq!(query.offset(), u64::from(page - 1) * u64::from(per_page));
            prop_assert_eq!(query.limit(), u64::from(per_page));
        }

        #[test]
        fn identifiers_are_lower_case_snake_case(name in "\\PC{0,12}") {
            let expected = name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            prop_assert_eq!(is_identifier(&name), expected);
        }

        #[test]
        fn direction_names_are_exact(name in "\\PC{0,6}") {
            match Direction::from_name(&name) {
                Some(direction) => prop_assert_eq!(direction.name(), name.as_str()),
                None => prop_assert!(name != "ASC" && name != "DESC"),
            }
        }

        #[test]
        fn a_spec_of_valid_identifiers_meets_the_contract(
            columns in proptest::collection::btree_set(identifier(), 0..6),
            filter_columns in proptest::collection::btree_set(identifier(), 0..4),
            default in any::<usize>(),
            direction in proptest::sample::select(Direction::ALL.to_vec()),
        ) {
            let spec = spec_of(&columns, &filter_columns, default, direction);
            prop_assert_eq!(spec.problems(), Vec::<String>::new());
        }

        #[test]
        fn a_column_that_is_no_identifier_is_named_as_a_defect(
            columns in proptest::collection::btree_set(identifier(), 0..4),
            filter_columns in proptest::collection::btree_set(identifier(), 0..3),
            bad in "\\PC{0,12}".prop_filter("no identifier", |name| !is_identifier(name)),
            in_a_filter in any::<bool>(),
        ) {
            let bad: &'static str = String::leak(bad);
            // The bad column under a field or filter name of its own, which
            // no generated name can be (it has a capital letter).
            let mut spec = spec_of(&columns, &filter_columns, 0, Direction::Asc);
            if in_a_filter {
                let mut filters = spec.filters.to_vec();
                filters.push(FilterKey {
                    param: "Bad",
                    column: bad,
                    kind: FilterKind::Ids,
                });
                spec.filters = Vec::leak(filters);
            } else {
                let mut sort_keys = spec.sort_keys.to_vec();
                sort_keys.push(SortKey::text("Bad", bad));
                spec.sort_keys = Vec::leak(sort_keys);
            }
            let problems = spec.problems();
            let named = format!("column {bad:?} is not a lower-case snake_case identifier");
            prop_assert!(
                problems.iter().any(|problem| problem.contains(&named)),
                "{:?}",
                problems
            );
            // That is the spec's only defect.
            prop_assert_eq!(problems.len(), 1, "{:?}", problems);
        }

        #[test]
        fn the_spec_finds_exactly_its_listed_names(name in listed_or_any_name()) {
            // By the request's name, compared exactly and case-sensitively:
            // a name in another case or a column's name finds nothing.
            let sort_key = PROBE.sort_key(&name);
            prop_assert_eq!(
                sort_key,
                PROBE.sort_keys.iter().copied().find(|key| key.field == name)
            );
            if let Some(key) = sort_key {
                prop_assert_eq!(key.field, name.as_str());
            }
            let filter = PROBE.filter(&name);
            prop_assert_eq!(
                filter,
                PROBE.filters.iter().copied().find(|key| key.param == name)
            );
            if let Some(key) = filter {
                prop_assert_eq!(key.param, name.as_str());
            }
        }

        #[test]
        fn an_unknown_filter_names_the_parameter_as_sent(name in "\\PC{0,24}") {
            let error = ListQueryError::UnknownFilter(name.clone());
            prop_assert_eq!(error.parameter(), name.as_str());
        }

        #[test]
        fn every_refusal_names_a_parameter_the_query_carries(raw in refused_or_any_query()) {
            // The envelope's `field` always points at something the client
            // sent: a check never refuses a parameter that is absent.
            if let Err(error) = probe(&raw) {
                let sent: Vec<String> = form_urlencoded::parse(raw.as_bytes())
                    .map(|(name, _)| name.into_owned())
                    .collect();
                prop_assert!(
                    sent.iter().any(|name| name == error.parameter()),
                    "{:?} in {:?}",
                    error,
                    sent
                );
            }
        }

        #[test]
        fn subsets_of_ids_read_back_sorted_and_once(
            ids in subsequence((1..=40i64).collect::<Vec<_>>(), 1..=40),
            repeats in 1usize..3,
        ) {
            let raw: Vec<String> = ids
                .iter()
                .rev()
                .cycle()
                .take(ids.len() * repeats)
                .map(|id| format!("id={id}"))
                .collect();
            let query = probe(&raw.join("&")).expect("accepted");
            prop_assert_eq!(
                &query.filters[0].value,
                &FilterValue::Ids(ids.iter().copied().collect())
            );
        }
    }
}
