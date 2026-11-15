//! The labels, „címkék”, of the admin API (parity LBL-01, LBL-02): the
//! list, `GET /api/admin/labels`, one label, `GET /api/admin/labels/{id}`,
//! and the two writes, `POST /api/admin/labels` (create) and
//! `PUT /api/admin/labels/{id}` (edit).
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
//! The writes are the first writes, and follow the shared write contract
//! (recipe 12 of the project skill), in this order:
//!
//! 1. the body is a [`LabelRequest`]: a JSON object with exactly `name` and
//!    `color`, or `422 invalid_body` (parity SHR-10,
//!    `crate::dtos::admin::write`);
//! 2. its fields pass [`LabelInput::check`], trimmed (SHR-09) and within
//!    every column limit, or the first refused field is a `422` on that
//!    field (SHR-13, `field_error_envelope`);
//! 3. the model writes the row, its revision and its activity entry in one
//!    transaction (ADR 0011 §7), with the signed-in account as the actor;
//!    a name that a live label already has, compared without regard to
//!    case, is `409 label.name_taken` on `name`, and an edit of a label that
//!    is missing or in the trash `404 not_found`;
//! 4. the answer is the saved label, as the record route sends it: `201` for
//!    a create, `200` for an edit.
//!
//! An edit reads its id before its body: an id that cannot name a label is
//! `404` whatever the body.
//!
//! Who may call them: the reads ([`READ`]) an account with `edit labels`, or
//! with `edit posts`, whose post form picks labels from this list (ADR 0007
//! §4); the writes ([`WRITE`]) `edit labels` only. The admin route table
//! checks that before any handler runs, so a refused account's body is
//! never read. The access map offers every label action to `edit labels`
//! alone (`crate::domain::authz::RESOURCE_ACCESS`): `edit posts` reads labels
//! only through the picker.
//!
//! The record routes read their id as text and answer `404 not_found` for
//! one that cannot name a label (not ASCII digits, 0, or above 2^53 − 1, the
//! public API's id rule, `crate::domain::v1_params::id`): axum's own `Path`
//! refusal would be a plain-text `400`, which the admin cannot read.

use axum::{
    Extension, Json,
    extract::{
        RawQuery,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
};
use loco_rs::{controller::middleware::request_id::LocoRequestId, prelude::*};

use super::{envelope_response, error_response, json_rejection_response, session::AdminSession};
use crate::{
    domain::{
        activity::Actor,
        authz::{Permission, Requirement},
        labels::{LIST, LabelInput},
        list_query, v1_params,
    },
    dtos::admin::{
        error::ErrorCode,
        labels::{LabelRequest, LabelResponse, name_taken_envelope},
        list::{ListResponse, list_query_error_envelope},
        write::field_error_envelope,
    },
    models::labels,
};

/// What the label reads ask of the account: `edit labels`, or `edit posts`
/// for the post form's label picker (ADR 0007 §4).
pub const READ: Requirement =
    Requirement::AnyPermission(&[Permission::EditLabels, Permission::EditPosts]);

/// What the label writes ask of the account: `edit labels` (ADR 0007 §4,
/// "create, update, delete, restore").
pub const WRITE: Requirement = Requirement::Permission(Permission::EditLabels);

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

/// The label id of a record route's path, or `None` when it cannot name a
/// label.
///
/// - `id`: the path's id, as text, or why axum could not read it.
///
/// A path value that is not UTF-8 after percent-decoding cannot be an id
/// either, so axum's refusal answers like any other id that names no label.
fn label_id(id: std::result::Result<Path<String>, PathRejection>) -> Option<i64> {
    id.ok().and_then(|Path(id)| v1_params::id(&id).ok())
}

/// The request id that a write's revision and activity entry record, so the
/// rows of one request can be grouped (ADR 0011 §4, §5).
///
/// - `request_id`: the id that Loco's `request_id` middleware put into the
///   request's extensions, the one the response's `x-request-id` header
///   carries; `None` only where that middleware is off.
///
/// The handlers take the extension as an `Option`, so a write never fails
/// for a missing id: the revision and the entry then leave the column empty,
/// as a task's do.
fn request_id_of(request_id: Option<Extension<LocoRequestId>>) -> Option<String> {
    request_id.map(|Extension(id)| id.get().to_owned())
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
    let Some(id) = label_id(id) else {
        return Ok(error_response(ErrorCode::NotFound));
    };
    match labels::Model::find_live_by_id(&ctx.db, id).await {
        Ok(label) => format::json(label_response(label)),
        Err(ModelError::EntityNotFound) => Ok(error_response(ErrorCode::NotFound)),
        Err(error) => Err(error.into()),
    }
}

/// `POST /api/admin/labels`: creates a label, with its revision 1 and its
/// `create` entry, and answers `201 Created` with the saved label.
///
/// - `ctx`: the application context; the handler takes the database from
///   it.
/// - `session`: the checked admin session; its account is the actor, the
///   label's `created_by` and `updated_by`.
/// - `request_id`: Loco's request id, which the revision and the entry
///   record (a content entry holds no client address, ADR 0011 §5).
/// - `body`: the JSON body, or why axum could not read it.
///
/// The refusals, in the module documentation's order: `422 invalid_body`
/// (or `413 body_too_large`), a `422` on the first refused field, and
/// `409 label.name_taken`. A refused save writes nothing.
///
/// # Errors
///
/// An internal error (`500`) when the database fails, including a refused
/// revision or activity entry, which rolls the whole save back.
#[debug_handler]
pub async fn create(
    State(ctx): State<AppContext>,
    session: AdminSession,
    request_id: Option<Extension<LocoRequestId>>,
    body: std::result::Result<Json<LabelRequest>, JsonRejection>,
) -> Result<Response> {
    // The write contract's first two refusals, before any query: a body
    // that is not the DTO, then the first field that cannot be stored.
    let request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return Ok(json_rejection_response(&rejection)),
    };
    let input = match LabelInput::check(&request.name, &request.color) {
        Ok(input) => input,
        Err(error) => return Ok(envelope_response(field_error_envelope(&error))),
    };
    let actor = Actor::User(session.user.id);
    // A failed write drops `txn`, which rolls the row, the revision and the
    // entry back together (ADR 0011 §7).
    let txn = ctx.db.begin().await?;
    match labels::Model::create(&txn, actor, &input, request_id_of(request_id)).await {
        Ok(label) => {
            txn.commit().await?;
            format::render()
                .status(StatusCode::CREATED)
                .json(label_response(label))
        }
        Err(ModelError::EntityAlreadyExists) => {
            // The index's refusal aborted `txn`; nothing was written.
            txn.rollback().await?;
            Ok(envelope_response(name_taken_envelope(input.name())))
        }
        Err(error) => Err(error.into()),
    }
}

/// `PUT /api/admin/labels/{id}`: saves the name and the colour of a live
/// label, with its next revision and an `update` entry that lists the
/// changed fields, and answers `200` with the saved label.
///
/// - `ctx`: the application context; the handler takes the database from
///   it.
/// - `session`: the checked admin session; its account is the actor, the
///   label's `updated_by`.
/// - `request_id`: Loco's request id, which the revision and the entry
///   record.
/// - `id`: the path's id, as text, or why axum could not read it.
/// - `body`: the JSON body, or why axum could not read it.
///
/// The refusals, in order: `404 not_found` for an id that cannot name a
/// label, `422 invalid_body` (or `413 body_too_large`), a `422` on the first
/// refused field, `404 not_found` for a label that is missing or in the
/// trash, and `409 label.name_taken` for a name that another live label
/// has. A refused save writes nothing. A label may change the case of its
/// own name.
///
/// # Errors
///
/// An internal error (`500`) when the database fails, including a refused
/// revision or activity entry, which rolls the whole save back.
#[debug_handler]
pub async fn update(
    State(ctx): State<AppContext>,
    session: AdminSession,
    request_id: Option<Extension<LocoRequestId>>,
    id: std::result::Result<Path<String>, PathRejection>,
    body: std::result::Result<Json<LabelRequest>, JsonRejection>,
) -> Result<Response> {
    let Some(id) = label_id(id) else {
        return Ok(error_response(ErrorCode::NotFound));
    };
    // The write contract's first two refusals, before any query: a body
    // that is not the DTO, then the first field that cannot be stored.
    let request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return Ok(json_rejection_response(&rejection)),
    };
    let input = match LabelInput::check(&request.name, &request.color) {
        Ok(input) => input,
        Err(error) => return Ok(envelope_response(field_error_envelope(&error))),
    };
    let actor = Actor::User(session.user.id);
    let txn = ctx.db.begin().await?;
    // The edit's `UPDATE` matches only a live row, so a label trashed
    // between the read and the write is a `404` too.
    let saved = match labels::Model::find_live_by_id(&txn, id).await {
        Ok(label) => {
            label
                .into_active_model()
                .edit(&txn, actor, &input, request_id_of(request_id))
                .await
        }
        Err(error) => Err(error),
    };
    match saved {
        Ok(label) => {
            txn.commit().await?;
            format::json(label_response(label))
        }
        Err(ModelError::EntityNotFound) => {
            txn.rollback().await?;
            Ok(error_response(ErrorCode::NotFound))
        }
        Err(ModelError::EntityAlreadyExists) => {
            // The index's refusal aborted `txn`; nothing was written.
            txn.rollback().await?;
            Ok(envelope_response(name_taken_envelope(input.name())))
        }
        Err(error) => Err(error.into()),
    }
}
