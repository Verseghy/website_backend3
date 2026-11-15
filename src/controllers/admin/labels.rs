//! The labels, „címkék”, of the admin API (parity LBL-01): the list,
//! `GET /api/admin/labels`, and one label, `GET /api/admin/labels/{id}`.
//!
//! The list is the first admin list, and follows the shared admin list
//! contract (recipe 11 of the project skill): the query is read and checked
//! by `crate::domain::list_query` against the labels' allow-list
//! ([`crate::domain::labels::LIST`]), the live labels are read by
//! [`labels::Model::list_live_page`], and the answer is a
//! `ListResponse<LabelResponse>`. A query the contract refuses is a `422`
//! envelope that names the parameter (`crate::dtos::admin::list`). By
//! default the labels come by name in the Hungarian alphabet (`COLLATE
//! hu_icu`, ADR 0020 §4), then by id descending (SHR-03), 25 to a page
//! (SHR-02). A label in the trash is neither listed nor read: the record
//! route answers `404 not_found` for it, as for an id that never existed.
//!
//! Who may call them ([`READ`]): an account with `edit labels`, or with
//! `edit posts`, whose post form picks labels from this list (ADR 0007 §4).
//! The admin route table checks that before either handler runs. The access
//! map offers the labels' screens to `edit labels` alone
//! (`crate::domain::authz::RESOURCE_ACCESS`): `edit posts` reads labels only
//! through the picker.
//!
//! The record route reads its id as text and answers `404 not_found` for one
//! that cannot name a label (not ASCII digits, 0, or above 2^53 − 1, the
//! public API's id rule, `crate::domain::v1_params::id`): axum's own `Path`
//! refusal would be a plain-text `400`, which the admin cannot read.

use axum::extract::{RawQuery, rejection::PathRejection};
use loco_rs::prelude::*;

use super::{envelope_response, error_response};
use crate::{
    domain::{
        authz::{Permission, Requirement},
        labels::LIST,
        list_query, v1_params,
    },
    dtos::admin::{
        error::ErrorCode,
        labels::LabelResponse,
        list::{ListResponse, list_query_error_envelope},
    },
    models::labels,
};

/// What the label reads ask of the account: `edit labels`, or `edit posts`
/// for the post form's label picker (ADR 0007 §4).
pub const READ: Requirement =
    Requirement::AnyPermission(&[Permission::EditLabels, Permission::EditPosts]);

/// The wire form of a label: the allow-listed columns only (ADR 0005).
///
/// - `label`: the stored row.
fn label_response(label: labels::Model) -> LabelResponse {
    LabelResponse {
        id: label.id,
        name: label.name,
        color: label.color,
    }
}

/// `GET /api/admin/labels`: one page of the live labels and their total.
///
/// - `ctx`: the application context; the handler takes the database from
///   it.
/// - `raw`: the query string, still encoded: `page`, `perPage`, `sort`,
///   `order` and the `id` filter (the module documentation and
///   `crate::domain::labels::LIST`).
///
/// A query that the list contract refuses answers its `422` envelope before
/// the database is asked anything.
///
/// # Errors
///
/// An internal error (`500`) when the database fails.
#[debug_handler]
pub async fn list(State(ctx): State<AppContext>, RawQuery(raw): RawQuery) -> Result<Response> {
    let query = match list_query::parse(&LIST, raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return Ok(envelope_response(list_query_error_envelope(&error))),
    };
    let page = labels::Model::list_live_page(&ctx.db, &query).await?;
    format::json(ListResponse {
        data: page.rows.into_iter().map(label_response).collect(),
        total: page.total,
    })
}

/// `GET /api/admin/labels/{id}`: one live label.
///
/// - `ctx`: the application context; the handler takes the database from
///   it.
/// - `id`: the path's id, as text, or why axum could not read it.
///
/// An id that names no live label, or that cannot name one, answers
/// `404 not_found`.
///
/// # Errors
///
/// An internal error (`500`) when the database fails.
#[debug_handler]
pub async fn show(
    State(ctx): State<AppContext>,
    id: std::result::Result<Path<String>, PathRejection>,
) -> Result<Response> {
    // A path value that is not UTF-8 after percent-decoding cannot be an id
    // either, so axum's refusal answers like any other.
    let Some(id) = id.ok().and_then(|Path(id)| v1_params::id(&id).ok()) else {
        return Ok(error_response(ErrorCode::NotFound));
    };
    match labels::Model::find_live_by_id(&ctx.db, id).await {
        Ok(label) => format::json(label_response(label)),
        Err(ModelError::EntityNotFound) => Ok(error_response(ErrorCode::NotFound)),
        Err(error) => Err(error.into()),
    }
}
