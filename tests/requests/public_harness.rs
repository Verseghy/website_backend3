//! The checks of the global `/api/v1` suite (`tests/support/public_rules.rs`,
//! `public_samples.rs` and `openapi_schema.rs`), tested on their own.
//!
//! The application has one public route so far, the OpenAPI document,
//! which takes no parameters, so the global suite cannot yet show that
//! its checks of repeated parameters, `+` decoding or response schemas find
//! anything. These tests put the checks against probe routes: a correct
//! probe, built like a public endpoint (the `/api/v1` layers, the parameter
//! module, the `200` helper and Loco's `etag` middleware), which must pass
//! every check, and faulty probes that each break one rule, which the check
//! of that rule must report. Samples that cannot show a rule (a base request
//! that misses its row, a list without a twinned row, values the route
//! answers alike) must be reported too, because with them a faulty route
//! would pass. A check that stops finding its fault fails here, not
//! silently in a later slice. The item probe has a preview `token`, as
//! `GET /posts/{id}` has, to show how a parameter outside the base request
//! is checked on its own row.

use std::collections::BTreeSet;

use axum::{
    Router,
    extract::{Path, RawQuery, rejection::PathRejection},
    http::{
        Method, StatusCode,
        header::{ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_TYPE, ETAG, VARY},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use axum_test::TestServer;
use backend3::{
    controllers::v1::{
        caching::{NotModifiedCors, cacheable_json, preview_json},
        openapi::{DOCUMENT_ROUTE, document_json},
        public_layers,
    },
    domain::{
        etag::weak_etag,
        v1_params::{self, Query, Rejection},
    },
    dtos::v1::error::{Error, ErrorCode},
};
use loco_rs::{
    app::AppContext,
    controller::middleware::{MiddlewareLayer, etag::Etag},
};
use percent_encoding::percent_decode_str;
use proptest::prelude::*;
use serde_json::{Value, json};
use url::form_urlencoded;

use crate::support::{
    fixtures::{Loaded, Twin, twin_id},
    openapi_schema::{
        assert_matches_operation, component_errors, response_schema, schema_errors,
        served_document, unrequired_properties,
    },
    public_rules::{Answer, answer_problems, random_input_failures, route_violations},
    public_samples::{
        QuerySample, ROUTE_SAMPLES, RouteSamples, Rows, Spaced, Spaces, Target, UNKNOWN_PARAMETERS,
        documented_query_parameters, encode_segment, fill_path, path_parameters, query_string,
        registry_gaps, unknown_parameters,
    },
    test_server, with_app, with_server,
};

/// The probe list route.
const PROBE: &str = "/api/v1/probe";
/// The probe route with a path parameter.
const PROBE_ITEM: &str = "/api/v1/probe/{id}";
/// The fixture that the probe samples name. The probes serve no table, so
/// [`probe_loaded`] says what such a fixture would have written.
const PROBE_FIXTURE: &str = "Probe rows";
/// The id of the probe's draft, which the item route serves only as a
/// preview, with [`PROBE_TOKEN`].
const PROBE_DRAFT: i64 = 8;
/// The preview token of [`PROBE_DRAFT`].
const PROBE_TOKEN: &str = "titok";

/// The probe's `limit`, with a refused `other`. It accepts digits only, so
/// no value shows how `+` is decoded.
const LIMIT: QuerySample = QuerySample {
    name: "limit",
    in_base: true,
    path: None,
    accepted: "5",
    other: "0",
    spaced: Spaced::Unobservable("limit accepts digits only and refuses ` 5` and `+5` alike"),
};
/// The probe's `q`, which it echoes, so every value gets its own answer.
const Q: QuerySample = QuerySample {
    name: "q",
    in_base: true,
    path: None,
    accepted: "ünnep",
    other: "ősz",
    spaced: Spaced::Value("őszi kirándulás"),
};
/// The item route's preview `token`, as `GET /posts/{id}` has one: outside
/// the base request, and accepted only on the draft's id.
const TOKEN: QuerySample = QuerySample {
    name: "token",
    in_base: false,
    path: Some(&[("id", "8")]),
    accepted: PROBE_TOKEN,
    other: "nem-az",
    spaced: Spaced::Unobservable("no token has a space or a plus sign, so both miss alike"),
};

/// The samples of the probe list route.
const PROBE_SAMPLES: RouteSamples = RouteSamples {
    route: PROBE,
    path: &[],
    query: &[LIMIT, Q],
    fixtures: &[PROBE_FIXTURE],
    rows: Rows::Twinned,
};
/// The samples of the probe item route.
const ITEM_SAMPLES: RouteSamples = RouteSamples {
    route: PROBE_ITEM,
    path: &[("id", "7")],
    query: &[LIMIT, Q, TOKEN],
    fixtures: &[PROBE_FIXTURE],
    rows: Rows::Twinned,
};

/// What loading [`PROBE_FIXTURE`] would have written: the twin of row 7,
/// the row that both probe routes list.
fn probe_loaded() -> Loaded {
    Loaded {
        fixtures: vec![PROBE_FIXTURE],
        twins: vec![Twin {
            fixture: PROBE_FIXTURE,
            table: "probe".to_owned(),
            live_id: 7,
            id: twin_id(7),
        }],
    }
}

/// One way a probe breaks the rules, or [`Fault::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// A correct public endpoint.
    None,
    /// Reads the last value of a repeated parameter.
    LastValueWins,
    /// Decodes `+` as a plus sign.
    PlusIsLiteral,
    /// Matches parameter names without regard to case, so `LIMIT` counts.
    NamesIgnoreCase,
    /// Answers a refused parameter with `text/plain`.
    PlainTextErrors,
    /// Writes `limit` as a string, which its schema does not allow.
    WrongShape,
    /// Lists a soft-deleted twin.
    LeaksTwin,
    /// Panics on a value with a control character, so the panic handler
    /// answers `500`.
    PanicsOnControls,
    /// Has none of the `/api/v1` layers: no CORS, no `no-store` on errors.
    NoPublicLayers,
    /// Runs without Loco's `etag` middleware, so it never answers `304`.
    NoEtagMiddleware,
    /// Runs without the trailing-slash normalisation of `App::serve`.
    NoTrailingSlash,
}

/// The decoded `(name, value)` pairs of `raw`, with `+` read as a space or,
/// for [`Fault::PlusIsLiteral`], as itself.
fn pairs(raw: Option<&str>, plus_is_space: bool) -> Vec<(String, String)> {
    let raw = raw.unwrap_or_default();
    if plus_is_space {
        return form_urlencoded::parse(raw.as_bytes())
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
    }
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |text: &str| percent_decode_str(text).decode_utf8_lossy().into_owned();
            (decode(name), decode(value))
        })
        .collect()
}

/// The value of `name` that a probe with `fault` reads.
fn read(fault: Fault, raw: Option<&str>, name: &str) -> Option<String> {
    match fault {
        // The real code path of a public handler.
        Fault::None | Fault::PlainTextErrors | Fault::WrongShape | Fault::LeaksTwin => {
            Query::parse(raw).first(name).map(str::to_owned)
        }
        Fault::LastValueWins => pairs(raw, true)
            .into_iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value),
        Fault::PlusIsLiteral => pairs(raw, false)
            .into_iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value),
        Fault::NamesIgnoreCase => pairs(raw, true)
            .into_iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value),
        Fault::PanicsOnControls
        | Fault::NoPublicLayers
        | Fault::NoEtagMiddleware
        | Fault::NoTrailingSlash => Query::parse(raw).first(name).map(str::to_owned),
    }
}

/// A refused parameter's answer.
fn refused(fault: Fault, rejection: Rejection) -> Response {
    if fault == Fault::PlainTextErrors {
        (StatusCode::BAD_REQUEST, "bad parameter").into_response()
    } else {
        Error::from(rejection).into_response()
    }
}

/// What a probe answers: its decoded `limit` and `q`, and one item, the
/// path `id` or 7. Like a real endpoint, it has rows 1 to 999 only, and
/// answers `404` for any other id. Like `GET /posts/{id}` (public-api.md
/// §7.3), the item route serves [`PROBE_DRAFT`] only as a preview, with
/// its `token` and `Cache-Control: no-store`, and a published row never
/// with a token.
fn probe(fault: Fault, raw: Option<&str>, id: Option<Result<String, PathRejection>>) -> Response {
    let item = id.is_some();
    let id = match id {
        None => 7,
        Some(Err(_)) => return refused(fault, Rejection::Id),
        Some(Ok(id)) => {
            assert!(
                fault != Fault::PanicsOnControls || !id.contains(char::is_control),
                "a control character in the id"
            );
            match v1_params::id(&id) {
                Ok(id) if id < 1000 => id,
                Ok(_) => return Error::new(ErrorCode::NotFound).into_response(),
                Err(rejection) => return refused(fault, rejection),
            }
        }
    };
    let limit = match v1_params::limit(read(fault, raw, "limit").as_deref()) {
        Ok(limit) => limit.get(),
        Err(rejection) => return refused(fault, rejection),
    };
    let token = if item {
        read(fault, raw, "token")
    } else {
        None
    };
    let preview = match v1_params::token(token.as_deref()) {
        Err(rejection) => return refused(fault, rejection),
        Ok(None) if id == PROBE_DRAFT => return Error::new(ErrorCode::NotFound).into_response(),
        Ok(None) => false,
        Ok(Some(token)) if id == PROBE_DRAFT && token == PROBE_TOKEN => true,
        Ok(Some(_)) => return Error::new(ErrorCode::NotFound).into_response(),
    };
    let q = read(fault, raw, "q");
    assert!(
        fault != Fault::PanicsOnControls
            || !q.as_deref().unwrap_or_default().contains(char::is_control),
        "a control character in q"
    );
    let mut items = vec![json!({ "id": id })];
    if fault == Fault::LeaksTwin {
        items.push(json!({ "id": twin_id(id) }));
    }
    let limit = if fault == Fault::WrongShape {
        json!(limit.to_string())
    } else {
        json!(limit)
    };
    let body = json!({ "limit": limit, "q": q, "items": items });
    if preview {
        preview_json(&body)
    } else {
        cacheable_json(&body)
    }
    .expect("a JSON body serializes")
}

/// A test server for the probe routes with `fault`, wrapped like the
/// application: the `/api/v1` layers on each route, Loco's `etag`
/// middleware with the `304` CORS middleware right outside it, and the
/// trailing-slash normalisation.
fn probe_server(ctx: AppContext, fault: Fault) -> TestServer {
    let list =
        get(move |RawQuery(raw): RawQuery| async move { probe(fault, raw.as_deref(), None) });
    let item = get(
        move |id: Result<Path<String>, PathRejection>, RawQuery(raw): RawQuery| async move {
            probe(fault, raw.as_deref(), Some(id.map(|Path(id)| id)))
        },
    );
    let (list, item) = if fault == Fault::NoPublicLayers {
        (list, item)
    } else {
        (list.layer(public_layers()), item.layer(public_layers()))
    };
    let mut router: Router<AppContext> = Router::new().route(PROBE, list).route(PROBE_ITEM, item);
    if fault != Fault::NoEtagMiddleware {
        router = Etag { enable: true }
            .apply(router)
            .expect("the etag middleware applies");
    }
    let router = NotModifiedCors
        .apply(router)
        .expect("the 304 CORS middleware applies")
        .with_state(ctx);
    if fault == Fault::NoTrailingSlash {
        TestServer::new(router).expect("start the probe server")
    } else {
        test_server(router)
    }
}

/// The application's OpenAPI document with the two probe operations and
/// their `Probe` schema, which allows exactly what a correct probe sends.
fn probe_document() -> Value {
    let mut document: Value =
        serde_json::from_str(&document_json().expect("the document serializes"))
            .expect("the document is JSON");
    document["components"]["schemas"]["Probe"] = json!({
        "type": "object",
        "required": ["limit", "q", "items"],
        "properties": {
            "limit": { "type": "integer", "minimum": 1, "maximum": 100 },
            "q": { "type": ["string", "null"] },
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id"],
                    "properties": { "id": { "type": "integer", "format": "int64" } },
                    "additionalProperties": false
                }
            }
        },
        "additionalProperties": false
    });
    let query = json!([
        { "name": "limit", "in": "query", "schema": { "type": "integer" } },
        { "name": "q", "in": "query", "schema": { "type": "string" } },
        { "$ref": "#/components/parameters/If-None-Match" }
    ]);
    let responses = json!({
        "200": {
            "description": "the probe",
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Probe" } } }
        },
        "304": { "$ref": "#/components/responses/NotModified" }
    });
    let mut item_parameters = query.clone();
    let item_only = item_parameters.as_array_mut().expect("an array");
    item_only.push(json!({ "name": "token", "in": "query", "schema": { "type": "string" } }));
    item_only.push(
        json!({ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } }),
    );
    document["paths"][PROBE] = json!({ "get": { "parameters": query, "responses": responses } });
    document["paths"][PROBE_ITEM] =
        json!({ "get": { "parameters": item_parameters, "responses": responses } });
    document
}

/// The global suite's violations for both probe routes with `fault`.
async fn probe_violations(fault: Fault) -> Vec<String> {
    with_app(|_router, ctx| async move {
        let server = probe_server(ctx, fault);
        let document = probe_document();
        let loaded = probe_loaded();
        let mut violations = route_violations(&server, &document, &PROBE_SAMPLES, &loaded).await;
        violations.extend(route_violations(&server, &document, &ITEM_SAMPLES, &loaded).await);
        violations
    })
    .await
}

/// Asserts that the violations of `fault` include one of `rule` that says
/// `finding`.
async fn assert_found(fault: Fault, rule: &str, finding: &str) {
    let violations = probe_violations(fault).await;
    assert!(
        violations
            .iter()
            .any(|line| line.contains(rule) && line.contains(finding)),
        "{fault:?}: no `{rule}` violation saying `{finding}` in:\n{}",
        violations.join("\n")
    );
}

#[tokio::test]
async fn a_correct_public_endpoint_passes_every_global_rule() {
    let violations = probe_violations(Fault::None).await;
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[tokio::test]
async fn reading_the_last_of_a_repeated_parameter_is_found() {
    assert_found(
        Fault::LastValueWins,
        "the first of a repeated parameter wins",
        "answered",
    )
    .await;
}

#[tokio::test]
async fn reading_plus_as_a_plus_sign_is_found() {
    assert_found(
        Fault::PlusIsLiteral,
        "`+` in a query value is a space",
        "answered",
    )
    .await;
}

#[tokio::test]
async fn reading_an_unknown_parameter_is_found() {
    assert_found(
        Fault::NamesIgnoreCase,
        "unknown query parameters are ignored",
        "answered",
    )
    .await;
}

#[tokio::test]
async fn an_error_outside_the_json_envelope_is_found() {
    assert_found(
        Fault::PlainTextErrors,
        "every answer",
        "is the JSON error envelope",
    )
    .await;
}

#[tokio::test]
async fn a_body_that_breaks_its_schema_is_found() {
    assert_found(
        Fault::WrongShape,
        "every answer",
        "does not match its OpenAPI schema",
    )
    .await;
}

#[tokio::test]
async fn a_soft_deleted_twin_in_a_body_is_found() {
    let twin = twin_id(7);
    assert_found(
        Fault::LeaksTwin,
        "every answer",
        &format!("/items/1/id: {twin}"),
    )
    .await;
}

#[tokio::test]
async fn missing_public_layers_are_found() {
    assert_found(
        Fault::NoPublicLayers,
        "every answer",
        "the CORS headers are []",
    )
    .await;
    // axum's 405 has no Cache-Control of its own; the layers add it.
    assert_found(
        Fault::NoPublicLayers,
        "POST answers 405",
        "a 405 is never stored",
    )
    .await;
}

#[tokio::test]
async fn a_missing_304_is_found() {
    assert_found(
        Fault::NoEtagMiddleware,
        "If-None-Match with the returned ETag gets 304",
        "not 304",
    )
    .await;
}

#[tokio::test]
async fn a_trailing_slash_that_answers_differently_is_found() {
    assert_found(
        Fault::NoTrailingSlash,
        "the trailing-slash form answers the same",
        "answered",
    )
    .await;
}

/// The global suite's violations for `samples` against the probe with
/// `fault`, with the fixtures `loaded`.
async fn sample_violations(fault: Fault, samples: RouteSamples, loaded: Loaded) -> Vec<String> {
    with_app(|_router, ctx| async move {
        let server = probe_server(ctx, fault);
        route_violations(&server, &probe_document(), &samples, &loaded).await
    })
    .await
}

/// Asserts that `violations` has a `samples` line that says `finding`.
fn assert_samples_problem(violations: &[String], finding: &str) {
    assert!(
        violations
            .iter()
            .any(|line| line.contains(": samples: ") && line.contains(finding)),
        "no `samples` violation saying `{finding}` in:\n{}",
        violations.join("\n")
    );
}

#[tokio::test]
async fn samples_that_cannot_tell_the_first_value_from_the_second_are_found() {
    // `05` is the same limit as `5`.
    let samples = RouteSamples {
        query: &[
            QuerySample {
                other: "05",
                ..LIMIT
            },
            Q,
        ],
        ..PROBE_SAMPLES
    };
    let violations = sample_violations(Fault::None, samples, probe_loaded()).await;
    assert_samples_problem(&violations, "`limit=5` and `limit=05` get the same answer");
}

#[tokio::test]
async fn a_base_request_that_misses_its_row_is_found() {
    // The item route has no row 5000, so the base request answers 404, and
    // without this check the 200 rules would not run at all: the probe
    // that leaks a twin, writes a wrong shape or never answers 304 would
    // pass. A route without query parameters gets no other warning.
    let missing = RouteSamples {
        path: &[("id", "5000")],
        query: &[],
        ..ITEM_SAMPLES
    };
    for fault in [
        Fault::None,
        Fault::LeaksTwin,
        Fault::WrongShape,
        Fault::NoEtagMiddleware,
    ] {
        let violations = sample_violations(fault, missing, probe_loaded()).await;
        assert_samples_problem(
            &violations,
            "the base request GET /api/v1/probe/5000 answered 404 Not Found",
        );
    }
    // The draft answers 404 without its token as well.
    let draft = RouteSamples {
        path: &[("id", "8")],
        ..ITEM_SAMPLES
    };
    let violations = sample_violations(Fault::None, draft, probe_loaded()).await;
    assert_samples_problem(&violations, "answered 404 Not Found");
}

#[tokio::test]
async fn a_base_answer_without_a_twinned_row_is_found() {
    // The fixtures twinned row 9 only, which neither probe route lists:
    // their twin check would pass however the route filtered the trash.
    let other_row = Loaded {
        twins: vec![Twin {
            live_id: 9,
            id: twin_id(9),
            ..probe_loaded().twins[0].clone()
        }],
        ..probe_loaded()
    };
    let violations = sample_violations(Fault::None, PROBE_SAMPLES, other_row).await;
    assert_samples_problem(&violations, "none of which has a soft-deleted twin");
    // Row 7's twin, but written by a fixture the entry does not name.
    let other_fixture = Loaded {
        twins: vec![Twin {
            fixture: "Another fixture",
            ..probe_loaded().twins[0].clone()
        }],
        ..probe_loaded()
    };
    let violations = sample_violations(Fault::None, ITEM_SAMPLES, other_fixture).await;
    assert_samples_problem(&violations, "none of which has a soft-deleted twin");
    // A route whose answer has no row ids says so, and is not held to it.
    let no_ids = RouteSamples {
        rows: Rows::NoIds("the probe pretends to have no ids"),
        ..PROBE_SAMPLES
    };
    let violations = sample_violations(Fault::None, no_ids, Loaded::default()).await;
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[tokio::test]
async fn a_spaced_value_that_cannot_show_plus_decoding_is_found() {
    // ` 5` and `+5` are both refused with the same 400, so the value
    // cannot tell a route that reads `+` as a space from one that does not.
    let samples = RouteSamples {
        query: &[
            QuerySample {
                spaced: Spaced::Value(" 5"),
                ..LIMIT
            },
            Q,
        ],
        ..PROBE_SAMPLES
    };
    for fault in [Fault::None, Fault::PlusIsLiteral] {
        let violations = sample_violations(fault, samples, probe_loaded()).await;
        assert_samples_problem(
            &violations,
            "`limit= 5` gets the same answer with a space as with a plus sign",
        );
    }
}

#[tokio::test]
async fn a_parameter_outside_the_base_request_is_checked_on_its_own_row() {
    // With the token in the base request, the base is a 404.
    let in_base = RouteSamples {
        query: &[
            LIMIT,
            Q,
            QuerySample {
                in_base: true,
                ..TOKEN
            },
        ],
        ..ITEM_SAMPLES
    };
    let violations = sample_violations(Fault::None, in_base, probe_loaded()).await;
    assert_samples_problem(&violations, "answered 404 Not Found");
    // On the base request's published row, every token is a 404, so the
    // right and the wrong token cannot be told apart.
    let own_path = RouteSamples {
        query: &[
            LIMIT,
            Q,
            QuerySample {
                path: None,
                ..TOKEN
            },
        ],
        ..ITEM_SAMPLES
    };
    let violations = sample_violations(Fault::None, own_path, probe_loaded()).await;
    assert_samples_problem(
        &violations,
        "`token=titok` and `token=nem-az` get the same answer",
    );
    // On the draft's row, the token's own checks find a route that reads
    // the last token.
    let violations = sample_violations(
        Fault::LastValueWins,
        RouteSamples {
            query: &[TOKEN],
            ..ITEM_SAMPLES
        },
        probe_loaded(),
    )
    .await;
    assert!(
        violations.iter().any(
            |line| line.contains("the first of a repeated parameter wins")
                && line.contains("token=nem-az&token=titok")
        ),
        "{}",
        violations.join("\n")
    );
}

// `random_input_failures` needs the multi-thread runtime (block_in_place).
#[tokio::test(flavor = "multi_thread")]
async fn the_random_input_property_finds_a_500_that_the_samples_miss() {
    with_app(|_router, ctx| async move {
        let server = probe_server(ctx, Fault::PanicsOnControls);
        let document = probe_document();
        // No sample has a control character, so the fixed checks pass.
        let loaded = probe_loaded();
        let mut violations = route_violations(&server, &document, &PROBE_SAMPLES, &loaded).await;
        violations.extend(route_violations(&server, &document, &ITEM_SAMPLES, &loaded).await);
        assert!(violations.is_empty(), "{}", violations.join("\n"));
        for samples in [&PROBE_SAMPLES, &ITEM_SAMPLES] {
            let failure = random_input_failures(&server, &document, samples, 256)
                .expect_err("random values with control characters reach the panic");
            assert!(
                failure.contains("user input never causes a 5xx"),
                "{failure}"
            );
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_random_input_property_passes_a_correct_endpoint() {
    with_app(|_router, ctx| async move {
        let server = probe_server(ctx, Fault::None);
        let document = probe_document();
        for samples in [&PROBE_SAMPLES, &ITEM_SAMPLES] {
            random_input_failures(&server, &document, samples, 64)
                .unwrap_or_else(|failure| panic!("{failure}"));
        }
    })
    .await;
}

// The registry check.

/// The probe routes, as the route table would list them.
fn probe_routes() -> Vec<String> {
    vec![PROBE.to_owned(), PROBE_ITEM.to_owned()]
}

/// [`registry_gaps`] of `registry` against the probe routes and document,
/// with two registered fixtures, [`PROBE_FIXTURE`] and `Known fixture`.
fn gaps(registry: &[RouteSamples]) -> Vec<String> {
    registry_gaps(
        &probe_routes(),
        &probe_document(),
        registry,
        &[PROBE_FIXTURE, "Known fixture"],
    )
}

/// Asserts that `gaps` has a line that starts with `route` and says
/// `finding`.
fn assert_gap(gaps: &[String], route: &str, finding: &str) {
    assert!(
        gaps.iter()
            .any(|gap| gap.starts_with(&format!("{route}: ")) && gap.contains(finding)),
        "no `{finding}` gap for {route} in:\n{}",
        gaps.join("\n")
    );
}

#[test]
fn a_complete_registry_has_no_gaps() {
    assert_eq!(gaps(&[PROBE_SAMPLES, ITEM_SAMPLES]), Vec::<String>::new());
    let with_fixture = RouteSamples {
        fixtures: &["Known fixture"],
        ..PROBE_SAMPLES
    };
    assert_eq!(gaps(&[with_fixture, ITEM_SAMPLES]), Vec::<String>::new());
}

#[test]
fn the_registry_check_finds_missing_stale_and_doubled_entries() {
    assert_gap(
        &gaps(&[PROBE_SAMPLES]),
        PROBE_ITEM,
        "no samples; add a RouteSamples",
    );
    let stale = RouteSamples {
        route: "/api/v1/gone",
        ..PROBE_SAMPLES
    };
    assert_gap(
        &gaps(&[PROBE_SAMPLES, ITEM_SAMPLES, stale]),
        "/api/v1/gone",
        "no such /api/v1 route",
    );
    assert_gap(
        &gaps(&[PROBE_SAMPLES, PROBE_SAMPLES, ITEM_SAMPLES]),
        PROBE,
        "registered twice",
    );
}

#[test]
fn the_registry_check_finds_every_gap_in_the_path_and_the_parameter_names() {
    for (entry, finding) in [
        (
            RouteSamples {
                path: &[],
                ..ITEM_SAMPLES
            },
            "must name each parameter of the template",
        ),
        (
            RouteSamples {
                path: &[("id", "7"), ("id", "8")],
                ..ITEM_SAMPLES
            },
            "must name each parameter of the template",
        ),
        (
            RouteSamples {
                path: &[("id", "..")],
                ..ITEM_SAMPLES
            },
            "the path samples: the value of {id} must not be empty or only dots",
        ),
        (
            RouteSamples {
                query: &[LIMIT],
                ..ITEM_SAMPLES
            },
            "must name each documented query parameter",
        ),
        (
            RouteSamples {
                query: &[
                    LIMIT,
                    Q,
                    QuerySample {
                        name: "featured",
                        ..LIMIT
                    },
                ],
                ..ITEM_SAMPLES
            },
            "must name each documented query parameter",
        ),
    ] {
        assert_gap(&gaps(&[PROBE_SAMPLES, entry]), PROBE_ITEM, finding);
    }
}

#[test]
fn the_registry_check_finds_every_gap_in_a_query_sample() {
    for (entry, finding) in [
        (
            RouteSamples {
                query: &[
                    QuerySample {
                        other: "5",
                        ..LIMIT
                    },
                    Q,
                ],
                ..ITEM_SAMPLES
            },
            "`other` must differ from `accepted`",
        ),
        (
            RouteSamples {
                query: &[
                    LIMIT,
                    QuerySample {
                        spaced: Spaced::Value("ősz"),
                        ..Q
                    },
                    TOKEN,
                ],
                ..ITEM_SAMPLES
            },
            "the `spaced` value must contain a space",
        ),
        (
            RouteSamples {
                query: &[
                    QuerySample {
                        spaced: Spaced::Unobservable(" "),
                        ..LIMIT
                    },
                    Q,
                    TOKEN,
                ],
                ..ITEM_SAMPLES
            },
            "`Spaced::Unobservable` needs the reason",
        ),
        (
            RouteSamples {
                query: &[
                    LIMIT,
                    Q,
                    QuerySample {
                        path: Some(&[("slug", "x")]),
                        ..TOKEN
                    },
                ],
                ..ITEM_SAMPLES
            },
            "`token`: its path samples {\"slug\"} must name each parameter of the template",
        ),
        (
            RouteSamples {
                query: &[
                    LIMIT,
                    Q,
                    QuerySample {
                        path: Some(&[("id", ".")]),
                        ..TOKEN
                    },
                ],
                ..ITEM_SAMPLES
            },
            "`token`: its path samples: the value of {id} must not be empty or only dots",
        ),
        (
            RouteSamples {
                fixtures: &[],
                ..ITEM_SAMPLES
            },
            "`Rows::Twinned` needs the fixtures",
        ),
        (
            RouteSamples {
                rows: Rows::NoIds(""),
                ..ITEM_SAMPLES
            },
            "`Rows::NoIds` needs the reason",
        ),
    ] {
        assert_gap(&gaps(&[PROBE_SAMPLES, entry]), PROBE_ITEM, finding);
    }
}

#[test]
fn the_registry_check_finds_every_gap_in_the_fixtures_and_rows() {
    for (entry, finding) in [
        (
            RouteSamples {
                fixtures: &["Unknown fixture"],
                ..ITEM_SAMPLES
            },
            "the fixture `Unknown fixture` is not registered",
        ),
        (
            RouteSamples {
                fixtures: &["Known fixture", "Known fixture"],
                ..ITEM_SAMPLES
            },
            "is named twice",
        ),
    ] {
        assert_gap(&gaps(&[PROBE_SAMPLES, entry]), PROBE_ITEM, finding);
    }
}

/// The entry of the document route, as `ROUTE_SAMPLES` has it.
const DOCUMENT_SAMPLES: RouteSamples = RouteSamples {
    route: DOCUMENT_ROUTE,
    path: &[],
    query: &[],
    fixtures: &[],
    rows: Rows::NoIds("the OpenAPI document reads no rows"),
};

#[test]
fn the_document_route_takes_no_query_samples() {
    assert!(ROUTE_SAMPLES.contains(&DOCUMENT_SAMPLES));
    let routes = vec![DOCUMENT_ROUTE.to_owned()];
    let document = probe_document();
    assert_eq!(
        registry_gaps(&routes, &document, &[DOCUMENT_SAMPLES], &[]),
        Vec::<String>::new()
    );
    let with_query = RouteSamples {
        query: &[LIMIT],
        ..DOCUMENT_SAMPLES
    };
    assert_gap(
        &registry_gaps(&routes, &document, &[with_query], &[]),
        DOCUMENT_ROUTE,
        "takes no query parameters",
    );
    // Any other route needs its GET operation in the document.
    let undocumented = vec!["/api/v1/undocumented".to_owned()];
    let entry = RouteSamples {
        route: "/api/v1/undocumented",
        ..PROBE_SAMPLES
    };
    assert_gap(
        &registry_gaps(&undocumented, &document, &[entry], &[]),
        "/api/v1/undocumented",
        "has no GET operation",
    );
}

#[test]
fn documented_query_parameters_follow_refs_and_skip_other_locations() {
    let document = probe_document();
    // The item route also has a path parameter and the If-None-Match
    // header, through a $ref.
    assert_eq!(
        documented_query_parameters(&document, PROBE_ITEM),
        Some(vec!["limit".to_owned(), "q".to_owned(), "token".to_owned()])
    );
    assert_eq!(documented_query_parameters(&document, "/api/v1/nope"), None);
    assert_eq!(documented_query_parameters(&document, DOCUMENT_ROUTE), None);
}

// The request helpers.

#[test]
fn path_parameters_are_read_in_order() {
    assert_eq!(
        path_parameters("/api/v1/canteen/{isoYear}/{isoWeek}"),
        ["isoYear", "isoWeek"]
    );
    assert_eq!(path_parameters(DOCUMENT_ROUTE), [] as [&str; 0]);
}

#[test]
fn path_values_are_encoded_as_segments() {
    assert_eq!(
        fill_path(PROBE_ITEM, &[("id", "a b/ő?#%")]).as_deref(),
        Ok("/api/v1/probe/a%20b%2F%C5%91%3F%23%25")
    );
    assert_eq!(
        fill_path(
            "/api/v1/x/{a}/{b}",
            &[("b", "2"), ("a", "1-._~"), ("c", "3")]
        )
        .as_deref(),
        Ok("/api/v1/x/1-._~/2")
    );
    assert_eq!(
        fill_path(PROBE_ITEM, &[]),
        Err("no sample value for the path parameter {id}".to_owned())
    );
}

#[test]
fn query_strings_write_spaces_both_ways() {
    let pairs = [("q", "őszi kirándulás"), ("a+b", "1&2=3")];
    assert_eq!(
        query_string(&pairs, Spaces::Plus),
        "q=%C5%91szi+kir%C3%A1ndul%C3%A1s&a%2Bb=1%262%3D3"
    );
    assert_eq!(
        query_string(&pairs, Spaces::Percent),
        "q=%C5%91szi%20kir%C3%A1ndul%C3%A1s&a%2Bb=1%262%3D3"
    );
    assert_eq!(query_string::<&str, &str>(&[], Spaces::Plus), "");
}

#[test]
fn targets_are_changed_one_part_at_a_time() {
    let base = ITEM_SAMPLES.target().expect("complete samples");
    assert_eq!(base.uri(), "/api/v1/probe/7?limit=5&q=%C3%BCnnep");
    assert_eq!(
        base.with_trailing_slash().uri(),
        "/api/v1/probe/7/?limit=5&q=%C3%BCnnep"
    );
    assert_eq!(
        base.with_value(0, "0").uri(),
        "/api/v1/probe/7?limit=0&q=%C3%BCnnep"
    );
    assert_eq!(
        base.with_pair_at(1, ("limit", "0")).uri(),
        "/api/v1/probe/7?limit=5&limit=0&q=%C3%BCnnep"
    );
    let no_query = Target {
        path: "/api/v1/x".to_owned(),
        pairs: Vec::new(),
    };
    assert_eq!(no_query.render(Spaces::Percent), "/api/v1/x");
    assert_eq!(
        no_query
            .with_pair_at(0, ("a", "b c"))
            .render(Spaces::Percent),
        "/api/v1/x?a=b%20c"
    );
}

#[test]
fn a_parameter_is_set_where_it_is_or_added_at_the_end() {
    let base = ITEM_SAMPLES.target().expect("complete samples");
    assert_eq!(base.position("q"), Some(1));
    assert_eq!(base.position("token"), None);
    let (set, at) = base.with_param("q", "ősz");
    assert_eq!(
        (set.uri().as_str(), at),
        ("/api/v1/probe/7?limit=5&q=%C5%91sz", 1)
    );
    let (added, at) = base.with_param("token", "x");
    assert_eq!(
        (added.uri().as_str(), at),
        ("/api/v1/probe/7?limit=5&q=%C3%BCnnep&token=x", 2)
    );
    // Only the first pair of a repeated name is set.
    let repeated = base.with_pair_at(2, ("q", "b"));
    assert_eq!(
        repeated.with_param("q", "a").0.uri(),
        "/api/v1/probe/7?limit=5&q=a&q=b"
    );
}

#[test]
fn a_reference_request_starts_from_the_base_on_the_parameters_own_path() {
    // A parameter of the base request: the base itself.
    let (limit, at) = ITEM_SAMPLES.reference(0).expect("complete samples");
    assert_eq!(
        (limit, at),
        (ITEM_SAMPLES.target().expect("complete samples"), 0)
    );
    // The token: added to the base request, on the draft's path.
    let (token, at) = ITEM_SAMPLES.reference(2).expect("complete samples");
    assert_eq!(
        (token.uri().as_str(), at),
        ("/api/v1/probe/8?limit=5&q=%C3%BCnnep&token=titok", 2)
    );
    let broken = RouteSamples {
        query: &[QuerySample {
            path: Some(&[]),
            ..TOKEN
        }],
        ..ITEM_SAMPLES
    };
    assert_eq!(
        broken.reference(0),
        Err("the path of `token`: no sample value for the path parameter {id}".to_owned())
    );
}

#[test]
fn unknown_parameters_leave_out_the_routes_own() {
    let names: BTreeSet<&str> = unknown_parameters(&PROBE_SAMPLES)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(
        !names.contains("limit") && !names.contains("q"),
        "{names:?}"
    );
    // Other letter cases stay unknown: names compare exactly (§2.1).
    assert!(
        names.contains("LIMIT") && names.contains("token"),
        "{names:?}"
    );
    assert_eq!(
        unknown_parameters(&DOCUMENT_SAMPLES).len(),
        UNKNOWN_PARAMETERS.len()
    );
}

proptest! {
    /// A path value reaches the handler as registered: the encoded segment
    /// keeps only unreserved bytes, and percent-decoding gives it back.
    #[test]
    fn encoded_segments_decode_to_the_value(value in any::<String>()) {
        let encoded = encode_segment(&value);
        prop_assert!(
            encoded.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%".contains(&b)),
            "{encoded}"
        );
        prop_assert_eq!(percent_decode_str(&encoded).decode_utf8().expect("UTF-8"), value);
    }

    /// Both query forms decode, by the WHATWG rules of §2.1, to the pairs
    /// they were built from, in order and with repeats.
    #[test]
    fn query_strings_decode_to_their_pairs(
        pairs in proptest::collection::vec((any::<String>(), any::<String>()), 0..5),
    ) {
        for spaces in [Spaces::Plus, Spaces::Percent] {
            let rendered = query_string(&pairs, spaces);
            let decoded: Vec<(String, String)> = form_urlencoded::parse(rendered.as_bytes())
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect();
            // An empty pair (`=`) is dropped by the parser only when the
            // whole pair is empty, which `name=value` never is.
            prop_assert_eq!(&decoded, &pairs, "{:?} {}", spaces, rendered);
        }
    }
}

// The answer rules, on answers made by hand.

/// An answer to `GET /api/v1/probe` with `status`, `headers` and `body`,
/// with the CORS header added.
fn answer(status: StatusCode, headers: &[(&str, &str)], body: &[u8]) -> Answer {
    let mut all = vec![(
        ACCESS_CONTROL_ALLOW_ORIGIN.as_str().to_owned(),
        "*".to_owned(),
    )];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    Answer {
        request: format!("GET {PROBE}"),
        status,
        headers: all,
        body: body.to_vec(),
    }
}

/// [`answer_problems`] of a `GET` answer of the probe list route.
fn problems(answer: &Answer) -> Vec<String> {
    answer_problems(&probe_document(), &Method::GET, Some(PROBE), answer)
}

/// Asserts that `found` has a line that contains `finding`.
fn assert_problem(found: &[String], finding: &str) {
    assert!(
        found.iter().any(|line| line.contains(finding)),
        "no `{finding}` in {found:?}"
    );
}

/// A correct `200` of the probe list route.
fn good_200() -> Answer {
    let body = br#"{"items":[{"id":7}],"limit":5,"q":null}"#;
    let tag = weak_etag(body);
    answer(
        StatusCode::OK,
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (CACHE_CONTROL.as_str(), "no-cache"),
            (ETAG.as_str(), &tag),
            (VARY.as_str(), "accept-encoding"),
        ],
        body,
    )
}

/// A correct `404` envelope.
fn good_404() -> Answer {
    answer(
        StatusCode::NOT_FOUND,
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (CACHE_CONTROL.as_str(), "no-store"),
        ],
        br#"{"error":"not_found","description":"Resource was not found"}"#,
    )
}

#[test]
fn correct_answers_have_no_problems() {
    assert_eq!(problems(&good_200()), Vec::<String>::new());
    assert_eq!(problems(&good_404()), Vec::<String>::new());
    let not_allowed = answer(
        StatusCode::METHOD_NOT_ALLOWED,
        &[("allow", "GET,HEAD"), (CACHE_CONTROL.as_str(), "no-store")],
        b"",
    );
    assert_eq!(problems(&not_allowed), Vec::<String>::new());
    let tag = weak_etag(b"{}");
    let not_modified = answer(
        StatusCode::NOT_MODIFIED,
        &[(ETAG.as_str(), &tag), (CACHE_CONTROL.as_str(), "no-cache")],
        b"",
    );
    assert_eq!(problems(&not_modified), Vec::<String>::new());
    let preview = answer(
        StatusCode::OK,
        &[
            (CONTENT_TYPE.as_str(), "application/json"),
            (CACHE_CONTROL.as_str(), "no-store"),
        ],
        br#"{"items":[],"limit":1,"q":"x"}"#,
    );
    assert_eq!(problems(&preview), Vec::<String>::new());
}

#[test]
fn each_answer_rule_is_checked() {
    let mut no_cors = good_200();
    no_cors.headers.remove(0);
    assert_problem(&problems(&no_cors), "the CORS headers are []");

    let mut credentials = good_404();
    credentials.headers.push((
        "access-control-allow-credentials".to_owned(),
        "true".to_owned(),
    ));
    assert_problem(&problems(&credentials), "the CORS headers are");

    let mut vary = good_200();
    vary.headers
        .push((VARY.as_str().to_owned(), "Origin".to_owned()));
    assert_problem(&problems(&vary), "Vary names the origin");

    let server_error = answer(
        StatusCode::INTERNAL_SERVER_ERROR,
        &[(CONTENT_TYPE.as_str(), "application/json")],
        br#"{"error":"internal_server_error","description":"Internal Server Error"}"#,
    );
    assert_problem(&problems(&server_error), "user input never causes a 5xx");

    let not_allowed = answer(StatusCode::METHOD_NOT_ALLOWED, &[("allow", "GET")], b"x");
    let found = problems(&not_allowed);
    assert_problem(&found, "allows {\"GET\"}");
    assert_problem(&found, "axum's empty body");
    assert_problem(&found, "a 405 is never stored");

    let mut plain = good_404();
    plain.headers[1].1 = "text/plain".to_owned();
    assert_problem(&problems(&plain), "is the JSON error envelope");

    let mut tagged = good_404();
    tagged
        .headers
        .push((ETAG.as_str().to_owned(), weak_etag(b"x")));
    assert_problem(&problems(&tagged), "has no ETag");

    let mut stored = good_404();
    stored.headers[2].1 = "no-cache".to_owned();
    assert_problem(&problems(&stored), "is never stored");

    let mut wrong_code = good_404();
    wrong_code.body = br#"{"error":"invalid_limit","description":"x"}"#.to_vec();
    assert_problem(&problems(&wrong_code), "belongs to 400 Bad Request");

    let mut not_an_error = good_404();
    not_an_error.body = br#"{"error":"teapot"}"#.to_vec();
    assert_problem(
        &problems(&not_an_error),
        "the 404 Not Found body is not an Error",
    );

    let mut stale_tag = good_200();
    stale_tag.headers[3].1 = weak_etag(b"other");
    assert_problem(&problems(&stale_tag), "is not the weak tag of the body");

    let mut uncached = good_200();
    uncached.headers[2].1 = "max-age=60".to_owned();
    assert_problem(&problems(&uncached), "a 200 has Cache-Control no-cache");

    let mut shape = good_200();
    shape.body = br#"{"items":[],"limit":"5","q":null}"#.to_vec();
    shape.headers[3].1 = weak_etag(&shape.body);
    assert_problem(&problems(&shape), "does not match its OpenAPI schema");

    let not_modified = answer(StatusCode::NOT_MODIFIED, &[], b"x");
    let found = problems(&not_modified);
    assert_problem(&found, "has a body");
    assert_problem(&found, "a 304 repeats the ETag");

    let mut head = good_200();
    head.request = format!("HEAD {PROBE}");
    assert_problem(
        &answer_problems(&probe_document(), &Method::HEAD, Some(PROBE), &head),
        "a 200 OK to HEAD has a body",
    );

    let redirect = answer(StatusCode::FOUND, &[], b"");
    assert_problem(&problems(&redirect), "is not an answer of the public API");
}

#[test]
fn without_a_route_a_200_body_is_not_held_to_a_schema() {
    let mut shape = good_200();
    shape.body = br#"{"anything":true}"#.to_vec();
    shape.headers[3].1 = weak_etag(&shape.body);
    assert_eq!(
        answer_problems(&probe_document(), &Method::GET, None, &shape),
        Vec::<String>::new()
    );
}

// The schema helpers.

#[test]
fn bodies_are_validated_against_component_schemas() {
    let document = probe_document();
    let error = json!({ "error": "not_found", "description": "Resource was not found" });
    assert_eq!(
        component_errors(&document, "Error", &error),
        Vec::<String>::new()
    );
    // A code that is not in the enum and a missing description: each one an
    // error, at its place.
    let errors = component_errors(&document, "Error", &json!({ "error": "teapot" }));
    assert!(
        errors.iter().any(|e| e.contains("description")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.starts_with("/error: ")),
        "{errors:?}"
    );
    assert_eq!(
        component_errors(&document, "Nope", &error),
        ["the document has no component schema Nope"]
    );
    // Formats are checked.
    let instant = json!({ "type": "string", "format": "date-time" });
    assert_eq!(
        schema_errors(&document, &instant, &json!("2026-10-14T10:00:00Z")),
        Vec::<String>::new()
    );
    assert_eq!(
        schema_errors(&document, &instant, &json!("2026-10-14 10:00:00")).len(),
        1
    );
    // OpenAPI's own integer formats are unknown to JSON Schema and ignored.
    let id = json!({ "type": "integer", "format": "int64" });
    assert_eq!(
        schema_errors(&document, &id, &json!(9_007_199_254_740_991_i64)),
        Vec::<String>::new()
    );
    assert_eq!(
        schema_errors(&document, &json!(true), &json!(1)),
        ["the schema is not an object: true"]
    );
    let broken = json!({ "$ref": "#/components/schemas/Missing" });
    assert!(
        schema_errors(&document, &broken, &json!(1))[0].starts_with("the schema does not compile"),
        "{:?}",
        schema_errors(&document, &broken, &json!(1))
    );
}

#[test]
fn an_endpoint_test_asserts_its_body_against_the_operation() {
    let body = json!({ "items": [{ "id": 7 }], "limit": 5, "q": "ünnep" });
    assert_matches_operation(&probe_document(), PROBE, StatusCode::OK, &body, "GET probe");
}

#[test]
#[should_panic(expected = "GET probe: the body does not match its OpenAPI schema")]
fn an_endpoint_test_fails_on_a_body_that_breaks_its_schema() {
    // `q` is always present, as `null` when empty (public-api.md §2.1).
    let body = json!({ "items": [], "limit": 5 });
    assert_matches_operation(&probe_document(), PROBE, StatusCode::OK, &body, "GET probe");
}

#[test]
#[should_panic(expected = "GET probe: GET /api/v1/probe declares no 304 Not Modified body")]
fn an_endpoint_test_fails_on_a_status_without_a_body() {
    assert_matches_operation(
        &probe_document(),
        PROBE,
        StatusCode::NOT_MODIFIED,
        &json!({}),
        "GET probe",
    );
}

#[test]
fn response_schemas_are_found_per_status() {
    let document = probe_document();
    assert_eq!(
        response_schema(&document, &Method::GET, PROBE, StatusCode::OK),
        Ok(Some(json!({ "$ref": "#/components/schemas/Probe" })))
    );
    // The 304 is a $ref to a response without a body.
    assert_eq!(
        response_schema(&document, &Method::GET, PROBE, StatusCode::NOT_MODIFIED),
        Ok(None)
    );
    assert_eq!(
        response_schema(&document, &Method::GET, PROBE, StatusCode::BAD_REQUEST),
        Err(format!("GET {PROBE} declares no 400 Bad Request response"))
    );
    // The document route is not described by the document (ADR 0005 §4).
    assert_eq!(
        response_schema(&document, &Method::GET, DOCUMENT_ROUTE, StatusCode::OK),
        Ok(None)
    );
    let mut no_json = document.clone();
    no_json["paths"][PROBE]["get"]["responses"]["200"]["content"] = json!({ "text/plain": {} });
    assert_eq!(
        response_schema(&no_json, &Method::GET, PROBE, StatusCode::OK),
        Err(format!(
            "the 200 OK response of GET {PROBE} declares no JSON schema"
        ))
    );
}

#[test]
fn properties_left_out_of_required_are_found() {
    let document = json!({
        "components": { "schemas": {
            "Complete": { "type": "object", "required": ["a"], "properties": { "a": {} } },
            "Partial": {
                "type": "object",
                "properties": {
                    "a": {},
                    "nested": { "type": "object", "required": [], "properties": { "b": {} } }
                },
                "required": ["nested"]
            }
        } }
    });
    assert_eq!(
        unrequired_properties(&document),
        [
            "/components/schemas/Partial: a",
            "/components/schemas/Partial/properties/nested: b"
        ]
    );
}

#[tokio::test]
async fn the_served_document_is_the_one_the_harness_reads() {
    with_server(|server, _ctx| async move {
        let served = served_document(&server).await;
        let built: Value =
            serde_json::from_str(&document_json().expect("serializes")).expect("JSON");
        assert_eq!(served, built);
    })
    .await;
}
