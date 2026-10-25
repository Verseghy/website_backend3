//! Route coverage of the public API's OpenAPI document (ADR 0005 §4).
//!
//! The document must describe exactly the operations that backend3 routes
//! under `/api/v1`: a route missing from it cannot be discovered by clients,
//! and an operation without a route is a promise the server does not keep.
//! [`coverage_gaps`] compares the two sets of `(method, path)` pairs in both
//! directions, and [`documented_operations`] reads the second set from the
//! document as clients receive it. The route-coverage test takes the routes
//! from Loco's route table (`AppRoutes::collect()`, what `cargo loco routes`
//! prints) and passes both sets in as plain values.
//!
//! Paths compare exactly as written: axum 0.8 and OpenAPI both write a path
//! parameter as `{id}`, and a document path without the `/api/v1` prefix
//! never matches, because the document has no `servers` entry that could add
//! it. Methods are upper-cased, because Loco reports `GET` and an OpenAPI
//! path item keys its operations by `get`. Only routes under `/api/v1` count,
//! by the rule of [`super::api_path`].

use std::{collections::BTreeSet, fmt};

use super::api_path::{ApiFamily, api_family};

/// The operation keys of an OpenAPI 3.1 path item, as the document writes
/// them (OpenAPI 3.1.0 §4.8.9).
pub const OPENAPI_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// One HTTP operation: a method and a path template.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Operation {
    /// The method in upper case, for example `GET`.
    pub method: String,
    /// The path template, for example `/api/v1/posts/{id}`.
    pub path: String,
}

impl Operation {
    /// An operation with the method upper-cased and the path kept as given.
    ///
    /// - `method`: the HTTP method in any case, for example `get` or `GET`.
    /// - `path`: the path template, compared byte for byte.
    #[must_use]
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.to_ascii_uppercase(),
            path: path.to_owned(),
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}

/// The differences between the routes under `/api/v1` and the document;
/// both lists are sorted and hold each operation once.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoverageGaps {
    /// Routes under `/api/v1` that the document does not describe.
    pub undocumented_routes: Vec<Operation>,
    /// Operations of the document that no route under `/api/v1` serves.
    pub unrouted_operations: Vec<Operation>,
}

impl CoverageGaps {
    /// Whether the routes and the document agree.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.undocumented_routes.is_empty() && self.unrouted_operations.is_empty()
    }
}

/// Compares the routes under `/api/v1` with the operations of the document.
///
/// - `routes`: every route of the application; those outside `/api/v1` are
///   ignored.
/// - `documented`: every operation the document lists. One outside `/api/v1`
///   is always reported, even when a route outside `/api/v1` matches it.
/// - `exempt`: routes that need no operation. ADR 0005 §4 names one, the
///   document route itself. An exempt route still counts as routed, so a
///   document may list it.
#[must_use]
pub fn coverage_gaps(
    routes: &[Operation],
    documented: &[Operation],
    exempt: &[Operation],
) -> CoverageGaps {
    let public_routes: BTreeSet<&Operation> = routes
        .iter()
        .filter(|route| api_family(&route.path) == Some(ApiFamily::V1))
        .collect();
    let documented: BTreeSet<&Operation> = documented.iter().collect();
    CoverageGaps {
        undocumented_routes: public_routes
            .iter()
            .filter(|route| !documented.contains(*route) && !exempt.contains(route))
            .map(|route| (*route).clone())
            .collect(),
        unrouted_operations: documented
            .difference(&public_routes)
            .map(|operation| (*operation).clone())
            .collect(),
    }
}

/// Reads the operations that an OpenAPI 3.1 document lists, from its JSON
/// form: every key of a path item that is one of [`OPENAPI_METHODS`]. Other
/// keys (`summary`, `parameters`, `servers`, `x-...`) are not operations.
///
/// - `document`: the parsed document, as served.
///
/// # Errors
///
/// Returns a message naming the part that does not have the OpenAPI shape:
/// a missing or non-object `paths`, or a path item that is not an object.
pub fn documented_operations(document: &serde_json::Value) -> Result<Vec<Operation>, String> {
    let paths = document
        .get("paths")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "the document has no `paths` object".to_owned())?;
    let mut operations = BTreeSet::new();
    for (path, item) in paths {
        let item = item
            .as_object()
            .ok_or_else(|| format!("the path item of `{path}` is not an object"))?;
        for method in OPENAPI_METHODS
            .iter()
            .filter(|method| item.contains_key(**method))
        {
            operations.insert(Operation::new(method, path));
        }
    }
    Ok(operations.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::btree_set, prelude::*, sample::select};
    use serde_json::json;

    fn op(method: &str, path: &str) -> Operation {
        Operation::new(method, path)
    }

    #[test]
    fn methods_are_upper_cased_and_paths_kept() {
        assert_eq!(op("get", "/A").method, "GET");
        assert_eq!(op("Get", "/A"), op("GET", "/A"));
        assert_ne!(op("GET", "/a"), op("GET", "/A"));
        assert_eq!(op("get", "/api/v1/x").to_string(), "GET /api/v1/x");
    }

    #[test]
    fn gaps_are_reported_in_both_directions() {
        let routes = [
            op("GET", "/api/v1/posts"),
            op("GET", "/api/v1/posts"),
            op("POST", "/api/v1/posts"),
            op("GET", "/api/v1/openapi.json"),
            op("GET", "/_ping"),
            op("GET", "/api/v1x/posts"),
        ];
        // `/posts` lacks the prefix, so it never matches a route, and
        // `/_ping` is routed but not public, so it does not count as routed.
        let documented = [
            op("get", "/api/v1/posts"),
            op("get", "/posts"),
            op("get", "/_ping"),
        ];
        let gaps = coverage_gaps(&routes, &documented, &[op("GET", "/api/v1/openapi.json")]);
        assert_eq!(gaps.undocumented_routes, [op("POST", "/api/v1/posts")]);
        assert_eq!(
            gaps.unrouted_operations,
            [op("GET", "/_ping"), op("GET", "/posts")]
        );
        assert!(!gaps.is_empty());
        assert!(coverage_gaps(&routes[..1], &documented[..1], &[]).is_empty());
    }

    #[test]
    fn an_exempt_operation_without_a_route_is_still_reported() {
        let document_route = op("GET", "/api/v1/openapi.json");
        let exempt = [document_route.clone()];
        assert!(coverage_gaps(&exempt, &exempt, &exempt).is_empty());
        let gaps = coverage_gaps(&[], &exempt, &exempt);
        assert_eq!(gaps.unrouted_operations, [document_route]);
    }

    #[test]
    fn documented_operations_reads_method_keys_only() {
        let document = json!({ "paths": {
            "/api/v1/posts": { "summary": "s", "parameters": [], "get": {}, "head": {}, "x-y": {} },
            "/api/v1/posts/{id}": { "get": {} },
            "/api/v1/empty": {}
        }});
        assert_eq!(
            documented_operations(&document),
            Ok(vec![
                op("GET", "/api/v1/posts"),
                op("GET", "/api/v1/posts/{id}"),
                op("HEAD", "/api/v1/posts"),
            ])
        );
        assert_eq!(documented_operations(&json!({ "paths": {} })), Ok(vec![]));
        let no_paths = Err("the document has no `paths` object".to_owned());
        assert_eq!(
            documented_operations(&json!({ "openapi": "3.1.0" })),
            no_paths
        );
        assert_eq!(documented_operations(&json!({ "paths": [] })), no_paths);
        assert_eq!(
            documented_operations(&json!({ "paths": { "/api/v1/x": "get" } })),
            Err("the path item of `/api/v1/x` is not an object".to_owned())
        );
    }

    /// A method and a path, under `/api/v1` when `public` is true.
    fn operation(public: bool) -> impl Strategy<Value = Operation> {
        let path = if public {
            "/api/v1(/([a-z]{1,6}|\\{[a-z]{1,4}\\})){1,3}"
        } else {
            "/(api/admin|_ping|admin|api/v1x|api/v2)(/[a-z]{1,6}){0,2}"
        };
        (select(OPENAPI_METHODS.as_slice()), path).prop_map(|(method, path)| op(method, &path))
    }

    proptest! {
        #[test]
        fn new_upper_cases_the_method_and_keeps_the_path(
            method in "\\PC{0,10}",
            path in "\\PC{0,24}",
        ) {
            let operation = Operation::new(&method, &path);
            prop_assert_eq!(&operation.method, &method.to_ascii_uppercase());
            prop_assert_eq!(&operation.path, &path);
            prop_assert_eq!(&operation, &Operation::new(&method.to_ascii_lowercase(), &path));
            prop_assert_eq!(operation.to_string(), format!("{} {path}", operation.method));
        }

        #[test]
        fn the_gaps_are_the_two_set_differences(
            routed in btree_set(operation(true), 0..12),
            documented in btree_set(operation(true), 0..12),
            other in btree_set(operation(false), 0..6),
        ) {
            let routes: Vec<_> = routed.iter().chain(&other).cloned().collect();
            let listed: Vec<_> = documented.iter().cloned().collect();
            let gaps = coverage_gaps(&routes, &listed, &[]);
            let undocumented: Vec<_> = routed.difference(&documented).cloned().collect();
            let unrouted: Vec<_> = documented.difference(&routed).cloned().collect();
            prop_assert_eq!(&gaps.undocumented_routes, &undocumented);
            prop_assert_eq!(&gaps.unrouted_operations, &unrouted);
            prop_assert_eq!(gaps.is_empty(), undocumented.is_empty() && unrouted.is_empty());
            // Exempting every route leaves only the operations without one.
            let exempted = coverage_gaps(&routes, &listed, &routes);
            prop_assert!(exempted.undocumented_routes.is_empty());
            prop_assert_eq!(&exempted.unrouted_operations, &unrouted);
        }

        #[test]
        fn documented_operations_reads_back_what_a_document_lists(
            operations in btree_set((select(OPENAPI_METHODS.as_slice()), "/[a-z{}/]{0,12}"), 0..12),
        ) {
            let mut paths = serde_json::Map::new();
            for (method, path) in &operations {
                let item = paths.entry(path.clone()).or_insert_with(|| json!({ "summary": "s" }));
                item[*method] = json!({});
            }
            let expected: BTreeSet<_> =
                operations.iter().map(|(method, path)| op(method, path)).collect();
            prop_assert_eq!(
                documented_operations(&json!({ "paths": paths })),
                Ok(expected.into_iter().collect::<Vec<_>>())
            );
        }
    }
}
