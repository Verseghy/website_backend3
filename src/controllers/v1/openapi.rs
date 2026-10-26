//! The OpenAPI document of the public read API, served at
//! `GET /api/v1/openapi.json` (ADR 0005 §4, public-api.md §7.18).
//!
//! The Rust DTOs are the contract of `/api/v1` (ADR 0005 §1). This module is
//! the one place that turns them into a published description: the utoipa
//! derive on [`PublicApi`] lists the public operations, [`document`] builds
//! the document, and [`routes`] serves exactly the text of
//! [`document_json`]. The ADR 0005 §5 spike tried utoipa 6 with Loco 1.2.0
//! `Routes`; `docs/design/openapi-spike.md` records the result and the rules
//! for the DTOs and operations that join the document.
//!
//! Choices that hold for every slice that extends the document:
//!
//! - **OpenAPI 3.1.0**, utoipa 6's default; 3.2.0 is opt-in and not used.
//! - **No `servers` entry.** The paths carry `/api/v1` themselves, and a
//!   client resolves them against the host it fetched the document from
//!   (OpenAPI 3.1.0 §4.8.1: the default server is `/`). So one document fits
//!   production, staging and a developer's machine, and the served text equals
//!   the committed snapshot in every environment.
//! - **`info`** carries the contract version, which starts at `1.0.0` and
//!   whose minor a slice bumps by hand when it adds an operation or a field
//!   (ADR 0005 §4); the licence, `AGPL-3.0-only` as name and SPDX identifier;
//!   and the link to the public source, the AGPL-3.0 §13 source offer for API
//!   users (ADR 0012 §4).
//! - **Sorted output.** utoipa's `preserve_path_order` feature is off, so
//!   paths are serialized by name, whatever the order of the derive's lists;
//!   `preserve_order` is off, so a schema's properties are listed by name,
//!   not in the order of the struct's fields; and component schemas are
//!   always sorted by name.
//!
//! `tests/requests/openapi.rs` pins the snapshot, the served text, the
//! licence and the route coverage in both directions.

use axum::http::{
    HeaderValue,
    header::{CACHE_CONTROL, CONTENT_TYPE},
};
use loco_rs::prelude::*;
use utoipa::OpenApi;

/// Where the document is served (public-api.md §7.18). It is the only route
/// under `/api/v1` that the document does not describe (ADR 0005 §4).
pub const DOCUMENT_ROUTE: &str = "/api/v1/openapi.json";

/// The public source of backend3, from `repository` in `Cargo.toml`.
pub const SOURCE_REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// The licence of backend3 and of this document, as an SPDX identifier
/// (ADR 0012 §4).
pub const LICENSE: &str = "AGPL-3.0-only";

/// `info.description`, which links to the source repository (AGPL-3.0 §13).
/// OpenAPI descriptions are Markdown, where a bare URL stays plain text unless
/// the renderer adds links itself; the angle brackets make it a link in all.
const DESCRIPTION: &str = concat!(
    "The public, read-only JSON API of the Verseghy Ferenc Gimnázium website.\n\n",
    "backend3 is free software under the GNU Affero General Public License, ",
    "version 3 only (AGPL-3.0-only). Its source code is published at <",
    env!("CARGO_PKG_REPOSITORY"),
    ">."
);

/// The utoipa derive that lists the operations of `/api/v1`.
///
/// A public endpoint joins the document by being listed here, as
/// `paths(crate::controllers::v1::<module>::<handler>, ...)`; utoipa collects
/// the component schemas the operations use on its own.
///
/// `components(schemas(...))` lists the error body of public-api.md §6.15
/// and its code enum explicitly. Every error response of every operation
/// refers to them (ADR 0005 §3), but the document has no operation yet, and
/// the schemas are part of the contract from the first public route on.
///
/// `info(...)` sets every field the document carries, because utoipa fills
/// the ones left out from Cargo's package metadata (`CARGO_PKG_*`), which
/// describes the crate: the crate version is not the contract version.
/// `license(...)` accepts string literals only, so it repeats [`LICENSE`],
/// and the tests check that the two agree.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "backend3 public read API",
        // The contract version (ADR 0005 §4): bump the minor by hand for an
        // added operation or field, in the PR that updates the snapshot.
        version = "1.1.0",
        description = DESCRIPTION,
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only"),
    ),
    components(schemas(
        crate::dtos::v1::error::Error,
        crate::dtos::v1::error::ErrorCode
    ))
)]
pub struct PublicApi;

/// Builds the OpenAPI document of `/api/v1`.
#[must_use]
pub fn document() -> utoipa::openapi::OpenApi {
    PublicApi::openapi()
}

/// The document as it is served and snapshotted: pretty-printed JSON.
///
/// # Errors
///
/// Returns `serde_json`'s error if the document cannot be serialized. The
/// OpenAPI types have string keys only, so this does not happen in practice;
/// the error is passed on rather than unwrapped, so that a future type cannot
/// turn it into a panic inside a request.
pub fn document_json() -> serde_json::Result<String> {
    document().to_pretty_json()
}

/// The route of the document, written in full and without a prefix, for
/// [`super::routes`], which adds the CORS and panic layers of every
/// `/api/v1` route.
#[must_use]
pub fn routes() -> Routes {
    // `Routes::get` records the verb directly, so `AppRoutes::collect()` and
    // the route-coverage test see `GET` without Loco parsing the handler's
    // `Debug` output.
    Routes::new().get(DOCUMENT_ROUTE, serve_document)
}

/// `GET /api/v1/openapi.json`: the document as `application/json` with
/// `Cache-Control: no-cache` (public-api.md §9). It is built per request,
/// which needs no I/O; the weak `ETag` of §9 comes with the slice that adds
/// it to every public response. The CORS header comes from the layers of
/// [`super::routes`], so browser-based API explorers on other origins can
/// read the document too.
async fn serve_document() -> Result<Response> {
    let mut response = document_json()?.into_response();
    let headers = response.headers_mut();
    // A `String` response is `text/plain`. JSON has no charset parameter
    // (RFC 8259 §11), which is also how Loco's `format::json` sends it.
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::api_path::{ApiFamily, api_family};

    // The rest of the document, `info` included, is pinned by the snapshot
    // and the request tests of `tests/requests/openapi.rs`.
    #[test]
    fn the_document_route_is_the_published_url_under_the_public_api() {
        assert_eq!(DOCUMENT_ROUTE, "/api/v1/openapi.json");
        assert_eq!(api_family(DOCUMENT_ROUTE), Some(ApiFamily::V1));
    }
}
