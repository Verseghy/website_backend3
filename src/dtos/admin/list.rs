//! The answer of every admin list endpoint, and its refusals (the shared
//! admin list contract; parity SHR-02, SHR-03; ADR 0005, ADR 0021 §4).
//!
//! A list endpoint answers one page as a [`ListResponse`]: the rows and the
//! total that ra-core's pagination needs. Its query is read by
//! `crate::domain::list_query`; a request that the contract refuses is
//! answered by the envelope of [`list_query_error_envelope`], a `422` whose
//! `field` is the query parameter at fault, so a screen can tell which of
//! its paging, sort or filter values the server did not take.

use serde::Serialize;
use ts_rs::TS;

use super::error::{ErrorCode, ErrorEnvelope};
use crate::domain::list_query::ListQueryError;

/// One page of an admin list: the rows, and the number of rows of the
/// whole list with its filters, as ra-core's `getList` returns them.
///
/// A page past the last one has no rows and still the total, so the screen
/// can go back to the last page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct ListResponse<T> {
    /// The rows of the page, in the list's order.
    pub data: Vec<T>,
    /// How many rows the whole list has with its filters.
    #[ts(type = "number")]
    pub total: u64,
}

/// The admin error envelope for a list request that the contract refused.
///
/// - `error`: the refusal, the first failing check of the request.
///
/// Each refusal has its own `422` code, and `field` names the query
/// parameter at fault (`page`, `perPage`, `sort`, `order`, or the filter's
/// name as the request wrote it), so the admin can show a message about
/// that parameter (ADR 0021 §4). The English `description` says what the
/// parameter takes.
#[must_use]
pub fn list_query_error_envelope(error: &ListQueryError) -> ErrorEnvelope {
    let code = match error {
        ListQueryError::Page => ErrorCode::InvalidPage,
        ListQueryError::PerPage => ErrorCode::InvalidPerPage,
        ListQueryError::Sort => ErrorCode::InvalidSort,
        ListQueryError::Order => ErrorCode::InvalidOrder,
        ListQueryError::UnknownFilter(_) => ErrorCode::UnknownFilter,
        ListQueryError::Filter(_) => ErrorCode::InvalidFilter,
    };
    ErrorEnvelope {
        description: error.to_string(),
        field: Some(error.parameter().to_owned()),
        ..ErrorEnvelope::new(code)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_page_serializes_as_ra_core_reads_it() {
        let page = ListResponse {
            data: vec![json!({"id": 1}), json!({"id": 2})],
            total: 42,
        };
        assert_eq!(
            serde_json::to_value(page).expect("serializes"),
            json!({"data": [{"id": 1}, {"id": 2}], "total": 42})
        );
        let past_the_end: ListResponse<u8> = ListResponse {
            data: Vec::new(),
            total: 3,
        };
        assert_eq!(
            serde_json::to_value(past_the_end).expect("serializes"),
            json!({"data": [], "total": 3})
        );
    }

    #[test]
    fn every_refusal_is_a_422_that_names_its_parameter() {
        for (error, code, field) in [
            (ListQueryError::Page, "invalid_page", "page"),
            (ListQueryError::PerPage, "invalid_per_page", "perPage"),
            (ListQueryError::Sort, "invalid_sort", "sort"),
            (ListQueryError::Order, "invalid_order", "order"),
            (
                ListQueryError::UnknownFilter("q".to_owned()),
                "unknown_filter",
                "q",
            ),
            (ListQueryError::Filter("id"), "invalid_filter", "id"),
        ] {
            let envelope = list_query_error_envelope(&error);
            assert_eq!(envelope.error.status(), 422, "{error:?}");
            assert_eq!(
                serde_json::to_value(&envelope).expect("serializes"),
                json!({
                    "error": code,
                    "description": error.to_string(),
                    "field": field,
                    "params": {}
                })
            );
        }
    }

    #[test]
    fn the_typescript_declaration_is_generic_over_the_row() {
        let declaration = ListResponse::<u8>::decl(&ts_rs::Config::default());
        assert!(declaration.contains("ListResponse<T>"), "{declaration}");
        assert!(declaration.contains("data: Array<T>"), "{declaration}");
        assert!(declaration.contains("total: number"), "{declaration}");
    }
}
