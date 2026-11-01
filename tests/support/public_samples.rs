//! The sample registry of the global `/api/v1` suite (public-api.md §16.3).
//!
//! `tests/requests/public_global.rs` takes every `/api/v1` route from Loco's
//! route table (`AppRoutes::collect()`, what `cargo loco routes` prints) and
//! sends it the requests that its entry in [`ROUTE_SAMPLES`] describes. The
//! rules it checks are the same for every route, so a route only has to say
//! which values to send. Each public endpoint slice (ADR 0025 §3, class (c))
//! adds the entry of its route in the PR that adds the route, and
//! `every_public_route_has_registered_samples` fails until it does: for a
//! route without an entry, for an entry without a route, and for an entry
//! that leaves out a path or query parameter ([`registry_gaps`]).
//!
//! What an entry gives, and why the suite needs each part:
//!
//! - **Path values** for every `{name}` of the template, and the query
//!   values the base request sends, which together reach a live row of the
//!   entry's fixtures. The **base request** must get the route's cacheable
//!   `200` (`Cache-Control: no-cache`), or the suite reports the samples:
//!   only then are the rules of a `200` (the schema, the twins, the `ETag`,
//!   the `304`) checked, so a sample that misses its row cannot pass with
//!   those rules unchecked.
//! - **Query parameters**: exactly the ones the route's OpenAPI operation
//!   documents, each with the values of a [`QuerySample`]. A parameter's
//!   checks start from its **reference request**: the base request with the
//!   parameter set to `accepted`, on the parameter's own path values if it
//!   has them. `other` must get a different answer than `accepted`, so that
//!   sending both shows which one the route read (the first value wins,
//!   §2.1); a value the route refuses is the usual choice. A [`Spaced`]
//!   value contains a space and is sent with `+` and with `%20`, which must
//!   get the same answer, and with a real plus sign (`%2B`) for each space,
//!   which must not (form decoding, §2.1). An optional parameter that
//!   changes what the route answers, such as the preview `token` of
//!   `GET /posts/{id}` (§7.3), stays out of the base request (`in_base:
//!   false`) and names the path of the row it is accepted on (its draft).
//! - **Fixtures**: the names of the public-api.md §17.1 fixtures
//!   (`tests/support/fixtures.rs`) that the path values and query values
//!   refer to. The suite loads every fixture any entry names into one
//!   database, at the fixed instant [`SUITE_NOW`], before it sends anything.
//! - **Rows** ([`Rows`]): a route that serves rows must list, in its base
//!   answer, a row whose soft-deleted twin one of its fixtures wrote, so
//!   that the twin check of every `200` has something to miss; an empty list
//!   would pass it with nothing tested. Only a route whose answer has no row
//!   ids says so, with the reason.
//!
//! The helpers below turn an entry into request targets. Path values are
//! percent-encoded as RFC 3986 path segments and query values by the
//! `application/x-www-form-urlencoded` rules, so a value reaches the handler
//! exactly as registered.

use std::collections::BTreeSet;

use backend3::controllers::v1::openapi::DOCUMENT_ROUTE;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;
use url::form_urlencoded;

/// The value of a query parameter that shows how `+` is decoded, or why the
/// parameter has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spaced {
    /// A decoded value with at least one space. It is sent with `+` and
    /// with `%20` for each space, which must get the same answer, and with
    /// a real plus sign (`%2B`) for each space, which must get a different
    /// one: otherwise the value cannot show that the route reads `+` as a
    /// space, and the suite reports the sample. A search text that a
    /// fixture row matches with the space only is the usual choice.
    Value(&'static str),
    /// The parameter has no value that the route answers differently with a
    /// space than with a plus sign, for example a `limit`, which accepts
    /// digits only and refuses both with the same `400`, so `+` decoding
    /// cannot be seen through it. The text says why; the shared decoding
    /// of `domain::v1_params::Query` is shown through the parameters that
    /// can show it, and against a probe in `tests/requests/public_harness.rs`.
    Unobservable(&'static str),
}

/// The values the global suite sends for one query parameter of a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuerySample {
    /// The parameter's name, as the OpenAPI operation declares it.
    pub name: &'static str,
    /// Whether the base request sends `accepted`. `false` for an optional
    /// parameter that changes what the route answers, such as a preview
    /// `token`, so that the base request stays the route's cacheable `200`;
    /// the parameter's own checks still send it.
    pub in_base: bool,
    /// The path values of this parameter's checks, when it is accepted only
    /// on another row than the base request's (a preview token on its
    /// draft's id), as `(name, value)` for every path parameter; `None` for
    /// the entry's own path values.
    pub path: Option<&'static [(&'static str, &'static str)]>,
    /// A decoded value that the route accepts.
    pub accepted: &'static str,
    /// A decoded value that gets a different answer than `accepted`, for
    /// example one the route refuses with `400`.
    pub other: &'static str,
    /// The value that shows how `+` is decoded, or why there is none.
    pub spaced: Spaced,
}

/// Whether the base answer of a route lists rows that have soft-deleted
/// twins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rows {
    /// The base answer lists at least one row, found by its `id` member
    /// (`fixtures::row_ids_in`), whose twin one of the entry's fixtures
    /// wrote (`fixtures::Twin::fixture`), so that the twin check of the
    /// answer has something to miss. Every route that serves rows.
    Twinned,
    /// The answer has no row ids, so the suite cannot see whether it lists
    /// a twinned row. The text says why and which test asserts the twins'
    /// absence instead; for example the OpenAPI document, which reads no
    /// rows.
    NoIds(&'static str),
}

/// The samples of one `/api/v1` route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteSamples {
    /// The route template, exactly as Loco's route table writes it, for
    /// example `/api/v1/posts/{id}`.
    pub route: &'static str,
    /// The decoded value of each path parameter, as `(name, value)`.
    pub path: &'static [(&'static str, &'static str)],
    /// Every query parameter that the route's OpenAPI operation documents,
    /// in the order the base request sends them.
    pub query: &'static [QuerySample],
    /// The public-api.md §17.1 fixtures that the samples refer to.
    pub fixtures: &'static [&'static str],
    /// Whether the base answer lists twinned rows.
    pub rows: Rows,
}

/// The samples of every `/api/v1` route. A public endpoint slice adds the
/// entry of each route it adds, with the §17.1 fixtures its values refer to.
pub const ROUTE_SAMPLES: &[RouteSamples] = &[
    // The OpenAPI document (public-api.md §7.18): no parameters, and it
    // answers 200 on an empty database.
    RouteSamples {
        route: DOCUMENT_ROUTE,
        path: &[],
        query: &[],
        fixtures: &[],
        rows: Rows::NoIds("the OpenAPI document reads no rows"),
    },
];

/// The instant the global suite runs at (public-api.md §16.2: no test reads
/// the wall clock): a Wednesday in term time, outside the hours that the
/// Budapest daylight-saving changes repeat or skip, so that "today", "this
/// week" and notice expiries are the same on every run.
pub const SUITE_NOW: &str = "2026-10-14T10:00:00Z";

/// Query parameters that no route reads under these names, with values a
/// route that did read them would refuse or act on: the names of other
/// routes' parameters, the same names in other letter cases (names compare
/// exactly, §2.1), and the tracking parameters that links carry.
/// [`unknown_parameters`] leaves out the route's own names.
pub const UNKNOWN_PARAMETERS: &[(&str, &str)] = &[
    ("utm_source", "hirlevel"),
    ("fbclid", "IwAR0b5"),
    ("_", "1760436000000"),
    ("limit", "0"),
    ("LIMIT", "0"),
    ("Limit", "101"),
    ("before", "!"),
    ("featured", "maybe"),
    ("q", ""),
    ("token", "\u{0}"),
    ("id", "x"),
    ("page", "2"),
    ("limit[]", "0"),
];

/// [`UNKNOWN_PARAMETERS`] without the parameters that `samples` documents.
///
/// - `samples`: the route whose own query parameters are left out.
#[must_use]
pub fn unknown_parameters(samples: &RouteSamples) -> Vec<(&'static str, &'static str)> {
    UNKNOWN_PARAMETERS
        .iter()
        .copied()
        .filter(|(name, _)| samples.query.iter().all(|own| own.name != *name))
        .collect()
}

/// The characters an RFC 3986 path segment keeps as they are: letters,
/// digits and the unreserved `-._~`. Everything else is percent-encoded.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The names of the `{parameter}` segments of `template`, in order.
///
/// - `template`: a route path as Loco's route table writes it.
#[must_use]
pub fn path_parameters(template: &str) -> Vec<&str> {
    template
        .split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .collect()
}

/// `template` with each `{name}` segment replaced by the percent-encoded
/// value that `values` gives for it.
///
/// - `template`: a route path as Loco's route table writes it.
/// - `values`: decoded `(name, value)` pairs; extra pairs are ignored.
///
/// # Errors
///
/// Names the first parameter that `values` has no value for.
pub fn fill_path(template: &str, values: &[(&str, &str)]) -> Result<String, String> {
    template
        .split('/')
        .map(|segment| {
            let Some(name) = segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            else {
                return Ok(segment.to_owned());
            };
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| encode_segment(value))
                .ok_or_else(|| format!("no sample value for the path parameter {{{name}}}"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|segments| segments.join("/"))
}

/// `value` percent-encoded as one RFC 3986 path segment.
///
/// - `value`: the decoded segment.
#[must_use]
pub fn encode_segment(value: &str) -> String {
    utf8_percent_encode(value, SEGMENT).to_string()
}

/// How a query string writes a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spaces {
    /// `+`, as browsers write form values (`application/x-www-form-urlencoded`).
    Plus,
    /// `%20`, with every byte other than letters, digits and `-._~`
    /// percent-encoded.
    Percent,
}

/// The query string of `pairs`, without the leading `?`.
///
/// - `pairs`: decoded `(name, value)` pairs, in order; repeated names stay.
/// - `spaces`: how a space is written. Both forms decode to the same pairs.
#[must_use]
pub fn query_string<N, V>(pairs: &[(N, V)], spaces: Spaces) -> String
where
    N: AsRef<str>,
    V: AsRef<str>,
{
    match spaces {
        Spaces::Plus => form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                pairs
                    .iter()
                    .map(|(name, value)| (name.as_ref(), value.as_ref())),
            )
            .finish(),
        Spaces::Percent => pairs
            .iter()
            .map(|(name, value)| {
                format!(
                    "{}={}",
                    encode_segment(name.as_ref()),
                    encode_segment(value.as_ref())
                )
            })
            .collect::<Vec<_>>()
            .join("&"),
    }
}

/// One request target: a path and its decoded query pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The encoded path, without a query.
    pub path: String,
    /// The decoded query pairs, in order.
    pub pairs: Vec<(String, String)>,
}

impl Target {
    /// The path and query as a request line writes them, spaces as `spaces`.
    ///
    /// - `spaces`: how the query writes a space.
    #[must_use]
    pub fn render(&self, spaces: Spaces) -> String {
        if self.pairs.is_empty() {
            self.path.clone()
        } else {
            format!("{}?{}", self.path, query_string(&self.pairs, spaces))
        }
    }

    /// [`Self::render`] with `+` for spaces, as browsers send it.
    #[must_use]
    pub fn uri(&self) -> String {
        self.render(Spaces::Plus)
    }

    /// The same target with a `/` after the path (public-api.md DEC-10).
    #[must_use]
    pub fn with_trailing_slash(&self) -> Self {
        Self {
            path: format!("{}/", self.path),
            pairs: self.pairs.clone(),
        }
    }

    /// The same target with the value of the pair at `index` replaced.
    ///
    /// - `index`: the position of the pair in [`Self::pairs`].
    /// - `value`: the new decoded value.
    #[must_use]
    pub fn with_value(&self, index: usize, value: &str) -> Self {
        let mut pairs = self.pairs.clone();
        value.clone_into(&mut pairs[index].1);
        Self {
            path: self.path.clone(),
            pairs,
        }
    }

    /// The same target with `pair` inserted at `index`, so that it follows
    /// the pairs before that position.
    ///
    /// - `index`: where the new pair goes, from 0 to the number of pairs.
    /// - `pair`: the decoded `(name, value)` to insert.
    #[must_use]
    pub fn with_pair_at(&self, index: usize, pair: (&str, &str)) -> Self {
        let mut pairs = self.pairs.clone();
        pairs.insert(index, (pair.0.to_owned(), pair.1.to_owned()));
        Self {
            path: self.path.clone(),
            pairs,
        }
    }

    /// The position of the first pair called `name`, if there is one.
    ///
    /// - `name`: the decoded parameter name, compared exactly.
    #[must_use]
    pub fn position(&self, name: &str) -> Option<usize> {
        self.pairs.iter().position(|(key, _)| key == name)
    }

    /// The same target with parameter `name` set to `value`, and where it
    /// is: the first pair of that name gets the value, or, without one, the
    /// pair is added at the end.
    ///
    /// - `name`: the decoded parameter name.
    /// - `value`: its new decoded value.
    #[must_use]
    pub fn with_param(&self, name: &str, value: &str) -> (Self, usize) {
        match self.position(name) {
            Some(index) => (self.with_value(index, value), index),
            None => (
                self.with_pair_at(self.pairs.len(), (name, value)),
                self.pairs.len(),
            ),
        }
    }
}

impl RouteSamples {
    /// The base request of the route: the sample path with the `accepted`
    /// value of every query parameter that is part of it (`in_base`).
    ///
    /// # Errors
    ///
    /// Names a path parameter that the entry has no value for.
    pub fn target(&self) -> Result<Target, String> {
        Ok(Target {
            path: fill_path(self.route, self.path)?,
            pairs: self
                .query
                .iter()
                .filter(|sample| sample.in_base)
                .map(|sample| (sample.name.to_owned(), sample.accepted.to_owned()))
                .collect(),
        })
    }

    /// The reference request of the query parameter at `index` of
    /// [`Self::query`]: the base request on the parameter's own path values,
    /// if it has them, with the parameter set to `accepted`. The checks of
    /// that parameter change only this request, so their answers show what
    /// the parameter does. Returns the target and the position of the
    /// parameter's pair in it.
    ///
    /// - `index`: the parameter's position in [`Self::query`].
    ///
    /// # Errors
    ///
    /// Names a path parameter that the parameter's path values or the
    /// entry's have no value for.
    ///
    /// # Panics
    ///
    /// When `index` is not a position in [`Self::query`].
    pub fn reference(&self, index: usize) -> Result<(Target, usize), String> {
        let sample = &self.query[index];
        let base = self.target()?;
        let path = match sample.path {
            Some(values) => fill_path(self.route, values)
                .map_err(|problem| format!("the path of `{}`: {problem}", sample.name))?,
            None => base.path.clone(),
        };
        Ok(Target {
            path,
            pairs: base.pairs,
        }
        .with_param(sample.name, sample.accepted))
    }
}

/// The names of the query parameters that the operation `GET template`
/// declares in `document`, following `$ref`s into `components.parameters`,
/// or `None` when the document has no such operation.
///
/// - `document`: the served OpenAPI document.
/// - `template`: the route template, which is also the document's path key.
#[must_use]
pub fn documented_query_parameters(document: &Value, template: &str) -> Option<Vec<String>> {
    let operation = document.get("paths")?.get(template)?.get("get")?;
    let parameters = operation
        .get("parameters")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    Some(
        parameters
            .iter()
            .filter_map(|parameter| resolve(document, parameter))
            .filter(|parameter| parameter.get("in").and_then(Value::as_str) == Some("query"))
            .filter_map(|parameter| parameter.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect(),
    )
}

/// `value` itself, or the object its local `$ref` (`#/...`) points at.
///
/// - `document`: the document the reference is resolved in.
/// - `value`: an object that may be a reference.
#[must_use]
pub fn resolve<'a>(document: &'a Value, value: &'a Value) -> Option<&'a Value> {
    match value.get("$ref").and_then(Value::as_str) {
        Some(reference) => document.pointer(reference.strip_prefix('#')?),
        None => Some(value),
    }
}

/// Everything that keeps the registry from matching the routes, one line
/// per problem, each with the fix.
///
/// - `routes`: every `/api/v1` route template of Loco's route table.
/// - `document`: the served OpenAPI document, which says which query
///   parameters each route has.
/// - `registry`: the entries to check, normally [`ROUTE_SAMPLES`].
/// - `fixtures`: the names of every registered fixture.
#[must_use]
pub fn registry_gaps(
    routes: &[String],
    document: &Value,
    registry: &[RouteSamples],
    fixtures: &[&str],
) -> Vec<String> {
    let mut gaps = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in registry {
        if !seen.insert(entry.route) {
            gaps.push(format!(
                "{}: registered twice; keep one entry per route",
                entry.route
            ));
        }
        if !routes.iter().any(|route| route == entry.route) {
            gaps.push(format!(
                "{}: registered, but Loco's route table has no such /api/v1 route; remove the \
                 stale entry or fix its template",
                entry.route
            ));
            continue;
        }
        gaps.extend(
            entry_gaps(entry, document, fixtures)
                .into_iter()
                .map(|gap| format!("{}: {gap}", entry.route)),
        );
    }
    for route in routes {
        if !seen.contains(route.as_str()) {
            gaps.push(format!(
                "{route}: no samples; add a RouteSamples entry for it to ROUTE_SAMPLES in \
                 tests/support/public_samples.rs, with a value for each path parameter and each \
                 documented query parameter"
            ));
        }
    }
    gaps
}

/// The problems of one entry whose route exists.
fn entry_gaps(entry: &RouteSamples, document: &Value, fixtures: &[&str]) -> Vec<String> {
    let mut gaps = path_gaps(entry.route, entry.path, "the path samples");
    let registered: Vec<&str> = entry.query.iter().map(|sample| sample.name).collect();
    match documented_query_parameters(document, entry.route) {
        Some(documented) => {
            let documented: BTreeSet<&str> = documented.iter().map(String::as_str).collect();
            let named: BTreeSet<&str> = registered.iter().copied().collect();
            if documented != named || named.len() != registered.len() {
                gaps.push(format!(
                    "the query samples {registered:?} must name each documented query \
                     parameter {documented:?} once"
                ));
            }
        }
        None if entry.route == DOCUMENT_ROUTE => {
            if !registered.is_empty() {
                gaps.push("the document route takes no query parameters".to_owned());
            }
        }
        None => gaps.push(
            "the OpenAPI document has no GET operation for the route (the route-coverage test \
             of tests/requests/openapi.rs says how to add it)"
                .to_owned(),
        ),
    }
    for sample in entry.query {
        let name = sample.name;
        if sample.accepted == sample.other {
            gaps.push(format!(
                "`{name}`: `other` must differ from `accepted`, so that the first-value check \
                 can tell them apart"
            ));
        }
        match sample.spaced {
            Spaced::Value(value) if !value.contains(' ') => gaps.push(format!(
                "`{name}`: the `spaced` value must contain a space, so that `+`, `%20` and \
                 `%2B` are all sent"
            )),
            Spaced::Unobservable(reason) if reason.trim().is_empty() => gaps.push(format!(
                "`{name}`: `Spaced::Unobservable` needs the reason why no value of the \
                 parameter tells a space from a plus sign"
            )),
            Spaced::Value(_) | Spaced::Unobservable(_) => {}
        }
        if let Some(values) = sample.path {
            gaps.extend(path_gaps(
                entry.route,
                values,
                &format!("`{name}`: its path samples"),
            ));
        }
    }
    let mut named = BTreeSet::new();
    for fixture in entry.fixtures {
        if !fixtures.contains(fixture) {
            gaps.push(format!(
                "the fixture `{fixture}` is not registered in FIXTURES \
                 (tests/support/fixtures.rs)"
            ));
        }
        if !named.insert(fixture) {
            gaps.push(format!("the fixture `{fixture}` is named twice"));
        }
    }
    match entry.rows {
        Rows::Twinned if entry.fixtures.is_empty() => gaps.push(
            "`Rows::Twinned` needs the fixtures whose rows, and their soft-deleted twins, the \
             base request lists"
                .to_owned(),
        ),
        Rows::NoIds(reason) if reason.trim().is_empty() => gaps.push(
            "`Rows::NoIds` needs the reason why the answer has no row ids, and where the twins' \
             absence is tested instead"
                .to_owned(),
        ),
        Rows::Twinned | Rows::NoIds(_) => {}
    }
    gaps
}

/// The problems of the path values `values` for `route`; each line starts
/// with `what`, which names the values in the message.
fn path_gaps(route: &str, values: &[(&str, &str)], what: &str) -> Vec<String> {
    let mut gaps = Vec::new();
    let template: BTreeSet<&str> = path_parameters(route).into_iter().collect();
    let sampled: BTreeSet<&str> = values.iter().map(|(name, _)| *name).collect();
    if template != sampled || sampled.len() != values.len() {
        gaps.push(format!(
            "{what} {sampled:?} must name each parameter of the template {template:?} once"
        ));
    }
    for (name, value) in values {
        // An empty or all-dot segment would change the route: the URL parser
        // drops `.` and climbs `..`, and the trailing slash is trimmed.
        if value.chars().all(|c| c == '.') {
            gaps.push(format!(
                "{what}: the value of {{{name}}} must not be empty or only dots"
            ));
        }
    }
    gaps
}
