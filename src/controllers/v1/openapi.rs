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
//! - **Caching is documented once, for every operation.** The [`Caching`]
//!   modifier adds the `ETag` and `Cache-Control` headers, the
//!   `If-None-Match` parameter and the `NotModified` (`304`) response of
//!   public-api.md §9 to the components, and refers to them from every `GET`
//!   operation: Loco's `etag` middleware and the helpers of
//!   `super::caching` treat every public route alike, so an operation
//!   declares only its own parameters and responses.
//!
//! `tests/requests/openapi.rs` pins the snapshot, the served text, the
//! licence and the route coverage in both directions.

use loco_rs::prelude::*;
use utoipa::{
    Modify, OpenApi,
    openapi::{
        Components, Object, ObjectBuilder, Ref, RefOr, Required, Type,
        header::{Header, HeaderBuilder},
        path::{Operation, ParameterBuilder, ParameterIn},
        response::{Response as ApiResponse, ResponseBuilder},
    },
};

use super::caching;

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
///
/// `modifiers(&Caching)` runs after utoipa has collected the operations, so
/// the caching rules reach every operation listed in `paths(...)`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "backend3 public read API",
        // The contract version (ADR 0005 §4): bump the minor by hand for an
        // added operation or field, in the PR that updates the snapshot.
        version = "1.2.0",
        description = DESCRIPTION,
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only"),
    ),
    components(schemas(
        crate::dtos::v1::error::Error,
        crate::dtos::v1::error::ErrorCode
    )),
    modifiers(&Caching)
)]
pub struct PublicApi;

/// The name of the `ETag` header in `components.headers`.
pub const ETAG_HEADER: &str = "ETag";

/// The name of the `Cache-Control` header in `components.headers`.
pub const CACHE_CONTROL_HEADER: &str = "Cache-Control";

/// The name of the `If-None-Match` parameter in `components.parameters`.
pub const IF_NONE_MATCH_PARAMETER: &str = "If-None-Match";

/// The name of the `304` answer in `components.responses`.
pub const NOT_MODIFIED_RESPONSE: &str = "NotModified";

/// The caching rules of public-api.md §9 in the document.
///
/// [`Modify::modify`] adds four reusable objects to the components:
///
/// - the headers [`ETAG_HEADER`] and [`CACHE_CONTROL_HEADER`];
/// - the request header [`IF_NONE_MATCH_PARAMETER`];
/// - the response [`NOT_MODIFIED_RESPONSE`], a `304` without a body that
///   carries both headers.
///
/// It then refers to them from every `GET` operation: the parameter, the
/// `304` response, and both headers on the `200` response. It leaves alone
/// what an operation declares itself under the same name, so it is
/// idempotent, and it never touches an error response, which carries
/// `no-store` and no `ETag` (§8).
#[derive(Debug, Clone, Copy, Default)]
pub struct Caching;

impl Modify for Caching {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Components::new);
        components
            .headers
            .insert(ETAG_HEADER.to_owned(), RefOr::T(etag_header()));
        components.headers.insert(
            CACHE_CONTROL_HEADER.to_owned(),
            RefOr::T(cache_control_header()),
        );
        components.parameters.insert(
            IF_NONE_MATCH_PARAMETER.to_owned(),
            RefOr::T(
                ParameterBuilder::new()
                    .name(IF_NONE_MATCH_PARAMETER)
                    .parameter_in(ParameterIn::Header)
                    .required(Required::False)
                    .description(Some(IF_NONE_MATCH_DESCRIPTION))
                    .schema(Some(Object::with_type(Type::String)))
                    .build(),
            ),
        );
        components
            .responses
            .insert(NOT_MODIFIED_RESPONSE.to_owned(), RefOr::T(not_modified()));
        for item in openapi.paths.paths.values_mut() {
            if let Some(operation) = item.get.as_mut() {
                document_revalidation(operation);
            }
        }
    }
}

/// The description of the `ETag` header.
const ETAG_DESCRIPTION: &str = "A weak validator of the answer's JSON, the same whether the \
     body is sent compressed or not. Clients treat it as opaque and send it back in \
     `If-None-Match` to revalidate a stored answer. Every `200` carries it except a preview \
     (`token` given); an error never does.";

/// The description of the `Cache-Control` header.
const CACHE_CONTROL_DESCRIPTION: &str = "`no-cache` on every `200` and `304`: a client may store \
     the answer but revalidates it before every use, so an edit is visible on the next request. \
     `no-store` on a preview and on every error: neither may be stored.";

/// The description of the `If-None-Match` parameter.
///
/// It promises the `304` only for the tag sent back exactly as received, and
/// says that other forms *may* get the full answer. Loco 1.2.0 compares the
/// header byte for byte, so today they always do (public-api.md §9); a later
/// Loco that matches them as RFC 9110 §13.1.2 allows would then still keep
/// this published semantic, which public-api.md §2.1 "Versioning" binds to
/// `/api/v1`.
const IF_NONE_MATCH_DESCRIPTION: &str = "The `ETag` of a stored answer. Send it back exactly as \
     it was received: when it equals the `ETag` of the current answer, the answer is \
     `304 Not Modified` without a body. Other forms, such as a list of tags, `*` or the tag \
     without `W/`, may get the full answer.";

/// The description of the `304` answer.
const NOT_MODIFIED_DESCRIPTION: &str = "Not Modified: the `ETag` given in `If-None-Match` is \
     still current, so the stored answer may be used. The body is empty.";

/// The `ETag` header: a string `W/"..."`, with the tag of the body `[]` as
/// its example.
fn etag_header() -> Header {
    HeaderBuilder::new()
        .description(Some(ETAG_DESCRIPTION))
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .pattern(Some(r#"^W/"[!#-~]+"$"#))
                .examples([crate::domain::etag::weak_etag(b"[]")]),
        ))
        .build()
}

/// The `Cache-Control` header: one of the two values of public-api.md §9.
fn cache_control_header() -> Header {
    HeaderBuilder::new()
        .description(Some(CACHE_CONTROL_DESCRIPTION))
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(["no-cache", "no-store"])),
        ))
        .build()
}

/// The `304` answer: no content, both caching headers.
fn not_modified() -> ApiResponse {
    ResponseBuilder::new()
        .description(NOT_MODIFIED_DESCRIPTION)
        .header_ref(ETAG_HEADER, Ref::new(header_ref(ETAG_HEADER)))
        .header_ref(
            CACHE_CONTROL_HEADER,
            Ref::new(header_ref(CACHE_CONTROL_HEADER)),
        )
        .build()
}

/// The JSON pointer of a header in `components.headers`.
fn header_ref(name: &str) -> String {
    format!("#/components/headers/{name}")
}

/// Refers `operation` to the caching objects of [`Caching`]: the
/// `If-None-Match` parameter, the `304` response and both headers on its
/// `200`, each unless the operation already has one by that name. Header and
/// parameter names are compared without regard to ASCII case, as HTTP
/// compares them.
fn document_revalidation(operation: &mut Operation) {
    let parameter = format!("#/components/parameters/{IF_NONE_MATCH_PARAMETER}");
    let parameters = operation.parameters.get_or_insert_with(Vec::new);
    let declared = parameters.iter().any(|existing| match existing {
        RefOr::Ref(reference) => reference.ref_location == parameter,
        RefOr::T(own) => own.name.eq_ignore_ascii_case(IF_NONE_MATCH_PARAMETER),
    });
    if !declared {
        parameters.push(RefOr::Ref(Ref::new(parameter)));
    }
    let responses = &mut operation.responses.responses;
    responses
        .entry("304".to_owned())
        .or_insert_with(|| RefOr::Ref(Ref::from_response_name(NOT_MODIFIED_RESPONSE)));
    if let Some(RefOr::T(ok)) = responses.get_mut("200") {
        for name in [ETAG_HEADER, CACHE_CONTROL_HEADER] {
            // Header names are case-insensitive, which OpenAPI 3.1.0
            // §4.8.17.1 repeats for the keys of a response's `headers` map,
            // so an operation's own `etag` is the `ETag` header: adding the
            // shared one beside it would declare one header twice. The
            // parameter check above matches names the same way.
            let declared = ok.headers.keys().any(|own| own.eq_ignore_ascii_case(name));
            if !declared {
                ok.headers
                    .insert(name.to_owned(), RefOr::Ref(Ref::new(header_ref(name))));
            }
        }
    }
}

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

/// `GET /api/v1/openapi.json`: the document as a cacheable `200`
/// (`super::caching::cacheable_json_bytes`): `application/json`,
/// `Cache-Control: no-cache` and the weak `ETag` of its text (public-api.md
/// §9), so a client that revalidates gets a `304`. It is built per request,
/// which needs no I/O. The CORS header comes from the layers of
/// [`super::routes`], so browser-based API explorers on other origins can
/// read the document too.
async fn serve_document() -> Result<Response> {
    caching::cacheable_json_bytes(document_json()?.into_bytes())
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

    /// A document with one `GET` operation that answers `200` and `400`, as
    /// an endpoint slice's `#[utoipa::path]` would produce it, and a `POST`
    /// operation, which the public API never has.
    fn document_with_an_operation() -> utoipa::openapi::OpenApi {
        use utoipa::openapi::{
            HttpMethod, InfoBuilder, OpenApiBuilder,
            path::{OperationBuilder, PathItem, PathsBuilder},
        };
        let operation = || {
            OperationBuilder::new()
                .operation_id(Some("example"))
                .response("200", ResponseBuilder::new().description("The answer"))
                .response("400", ResponseBuilder::new().description("A bad parameter"))
                .build()
        };
        let mut item = PathItem::new(HttpMethod::Get, operation());
        item.post = Some(operation());
        OpenApiBuilder::new()
            .info(InfoBuilder::new().title("t").version("0").build())
            .paths(PathsBuilder::new().path("/api/v1/example", item))
            .build()
    }

    /// The `GET` operation of [`document_with_an_operation`] as JSON.
    fn get_operation(document: &utoipa::openapi::OpenApi) -> serde_json::Value {
        serde_json::to_value(document).expect("serialises")["paths"]["/api/v1/example"]["get"]
            .clone()
    }

    #[test]
    fn every_get_operation_documents_revalidation() {
        let mut document = document_with_an_operation();
        Caching.modify(&mut document);
        let get = get_operation(&document);
        assert_eq!(
            get["parameters"],
            serde_json::json!([{ "$ref": "#/components/parameters/If-None-Match" }])
        );
        assert_eq!(
            get["responses"]["304"],
            serde_json::json!({ "$ref": "#/components/responses/NotModified" })
        );
        assert_eq!(
            get["responses"]["200"]["headers"],
            serde_json::json!({
                "Cache-Control": { "$ref": "#/components/headers/Cache-Control" },
                "ETag": { "$ref": "#/components/headers/ETag" }
            })
        );
        // Errors carry no-store and no ETag (public-api.md §8): their
        // responses get no caching header.
        assert!(get["responses"]["400"].get("headers").is_none(), "{get}");
        // Only GET operations are revalidated.
        let post = &serde_json::to_value(&document).expect("serialises")["paths"]["/api/v1/example"]
            ["post"];
        assert!(post.get("parameters").is_none(), "{post}");
        assert!(post["responses"].get("304").is_none(), "{post}");
    }

    #[test]
    fn the_modifier_is_idempotent_and_keeps_what_an_operation_declares() {
        let mut document = document_with_an_operation();
        Caching.modify(&mut document);
        let once = serde_json::to_value(&document).expect("serialises");
        Caching.modify(&mut document);
        assert_eq!(serde_json::to_value(&document).expect("serialises"), once);

        // An operation that documents its own If-None-Match, 304 or ETag
        // keeps it.
        let mut document = document_with_an_operation();
        let get = document
            .paths
            .paths
            .get_mut("/api/v1/example")
            .and_then(|item| item.get.as_mut())
            .expect("the GET operation");
        get.parameters = Some(vec![RefOr::T(
            ParameterBuilder::new()
                .name("if-none-match")
                .parameter_in(ParameterIn::Header)
                .build(),
        )]);
        get.responses.responses.insert(
            "304".to_owned(),
            RefOr::T(ResponseBuilder::new().description("own").build()),
        );
        if let Some(RefOr::T(ok)) = get.responses.responses.get_mut("200") {
            ok.headers
                .insert(ETAG_HEADER.to_owned(), RefOr::T(Header::default()));
        }
        Caching.modify(&mut document);
        let get = get_operation(&document);
        assert_eq!(get["parameters"].as_array().map(Vec::len), Some(1));
        assert_eq!(get["parameters"][0]["name"], "if-none-match");
        assert_eq!(get["responses"]["304"]["description"], "own");
        assert_eq!(
            get["responses"]["200"]["headers"]["ETag"]["schema"]["type"],
            "string"
        );
        assert_eq!(
            get["responses"]["200"]["headers"]["Cache-Control"]["$ref"],
            "#/components/headers/Cache-Control"
        );

        // Header names are matched without regard to case (OpenAPI 3.1.0
        // §4.8.17.1): an operation's own `etag` and `CACHE-CONTROL` are the
        // two headers, so the shared ones are not added beside them.
        let mut document = document_with_an_operation();
        let Some(RefOr::T(ok)) = document
            .paths
            .paths
            .get_mut("/api/v1/example")
            .and_then(|item| item.get.as_mut())
            .and_then(|get| get.responses.responses.get_mut("200"))
        else {
            panic!("the GET operation answers an inline 200");
        };
        for name in ["etag", "CACHE-CONTROL"] {
            ok.headers
                .insert(name.to_owned(), RefOr::T(Header::default()));
        }
        Caching.modify(&mut document);
        let headers = &get_operation(&document)["responses"]["200"]["headers"];
        let mut names: Vec<&str> = headers
            .as_object()
            .expect("the 200's headers")
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["CACHE-CONTROL", "etag"], "{headers}");
        assert_eq!(headers["etag"]["schema"]["type"], "string", "{headers}");
    }

    #[test]
    fn the_document_declares_the_caching_components() {
        let document = serde_json::to_value(document()).expect("serialises");
        let components = &document["components"];
        for (kind, name) in [
            ("headers", ETAG_HEADER),
            ("headers", CACHE_CONTROL_HEADER),
            ("parameters", IF_NONE_MATCH_PARAMETER),
            ("responses", NOT_MODIFIED_RESPONSE),
        ] {
            assert!(components[kind][name].is_object(), "{kind} {name}");
        }
        // The two values match what the helpers send.
        assert_eq!(caching::NO_CACHE, "no-cache");
        assert_eq!(caching::NO_STORE, "no-store");
        assert_eq!(
            components["headers"][CACHE_CONTROL_HEADER]["schema"]["enum"],
            serde_json::json!(["no-cache", "no-store"])
        );
        // The example is a real tag, and the pattern's character class is
        // RFC 9110's `etagc` without obs-text.
        let example = crate::domain::etag::weak_etag(b"[]");
        assert_eq!(
            components["headers"][ETAG_HEADER]["schema"]["examples"][0],
            example.as_str()
        );
        assert_eq!(
            example,
            r#"W/"4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945""#
        );
    }
}
