//! Response validation against the served OpenAPI document (ADR 0005 §4,
//! public-api.md §16.2): every public response body is checked against the
//! schema that `GET /api/v1/openapi.json` declares for it.
//!
//! The Rust DTOs are the contract, and utoipa derives the schemas from
//! them, but a schema override (a newtype's `value_type`, a missing
//! `required = true` on an `Option` field) can still say something other
//! than what serde writes. Validating the bodies that the handlers really
//! send against the document that clients really read catches that
//! (docs/design/openapi-spike.md, P4).
//!
//! The schemas of an OpenAPI 3.1 document are JSON Schema 2020-12. The
//! validator is the `jsonschema` crate, an implementation of that draft
//! that knows nothing of utoipa, so a mistake in the derive is not repeated
//! by the check. A schema is validated inside a root object that also holds
//! the document's `components`, so its `#/components/schemas/...` references
//! resolve exactly as a client resolves them; references never leave the
//! document, and the crate is built without remote fetching. Formats are
//! validated (`date-time`, `date`, `uri`, ...); the OpenAPI-only formats
//! `int32` and `int64` are unknown to JSON Schema and ignored, as the
//! specification asks.

use axum::http::{Method, StatusCode};
use axum_test::TestServer;
use backend3::controllers::v1::openapi::DOCUMENT_ROUTE;
use serde_json::{Map, Value, json};

use super::public_samples::resolve;

/// The JSON Schema dialect of OpenAPI 3.1 schemas.
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The document as clients receive it from `GET /api/v1/openapi.json`.
///
/// - `server`: the test server.
///
/// # Panics
///
/// When the document is not served as JSON.
pub async fn served_document(server: &TestServer) -> Value {
    let response = server.get(DOCUMENT_ROUTE).await;
    assert_eq!(response.status_code(), StatusCode::OK, "{DOCUMENT_ROUTE}");
    response.json()
}

/// Every way `instance` breaks `schema`, one line each as
/// `JSON pointer: what`, or a single line when the schema itself does not
/// compile.
///
/// - `document`: the OpenAPI document whose `components` the schema's
///   references point into.
/// - `schema`: a schema object of the document, such as an operation's
///   response schema or `{"$ref": "#/components/schemas/Error"}`.
/// - `instance`: the parsed body to check.
#[must_use]
pub fn schema_errors(document: &Value, schema: &Value, instance: &Value) -> Vec<String> {
    let Some(schema) = schema.as_object() else {
        return vec![format!("the schema is not an object: {schema}")];
    };
    // The schema's own keywords next to the document's components. JSON
    // Schema ignores the `components` member as an unknown keyword, while a
    // `$ref` of `#/components/...` resolves into it as a JSON pointer into
    // the root.
    let mut root: Map<String, Value> = schema.clone();
    root.insert("$schema".to_owned(), json!(DIALECT));
    root.insert(
        "components".to_owned(),
        document.get("components").cloned().unwrap_or(json!({})),
    );
    let validator = match jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&Value::Object(root))
    {
        Ok(validator) => validator,
        Err(error) => return vec![format!("the schema does not compile: {error}")],
    };
    validator
        .iter_errors(instance)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect()
}

/// [`schema_errors`] for the component schema called `name`.
///
/// - `document`: the OpenAPI document.
/// - `name`: a key of `components.schemas`, such as `Error`.
/// - `instance`: the parsed body to check.
#[must_use]
pub fn component_errors(document: &Value, name: &str, instance: &Value) -> Vec<String> {
    if document
        .pointer(&format!("/components/schemas/{name}"))
        .is_none()
    {
        return vec![format!("the document has no component schema {name}")];
    }
    schema_errors(
        document,
        &json!({ "$ref": format!("#/components/schemas/{name}") }),
        instance,
    )
}

/// The schema of the `application/json` body that the operation
/// `method template` declares for `status`, following a response `$ref`.
///
/// - `document`: the OpenAPI document.
/// - `method`: the request method; `HEAD` answers carry no body.
/// - `template`: the route template, which is the document's path key.
/// - `status`: the status the response had.
///
/// # Errors
///
/// When the document has the operation but it declares no response for
/// `status`, or declares one without a JSON body: either way the answer is
/// not what the document promises. `Ok(None)` when the document does not
/// describe the operation at all (only `GET /api/v1/openapi.json`, ADR 0005
/// §4) or the declared response has no body (the `304`).
pub fn response_schema(
    document: &Value,
    method: &Method,
    template: &str,
    status: StatusCode,
) -> Result<Option<Value>, String> {
    let Some(operation) = document
        .get("paths")
        .and_then(|paths| paths.get(template))
        .and_then(|item| item.get(method.as_str().to_ascii_lowercase()))
    else {
        return Ok(None);
    };
    let response = operation
        .get("responses")
        .and_then(|responses| responses.get(status.as_str()))
        .and_then(|response| resolve(document, response))
        .ok_or_else(|| format!("{method} {template} declares no {status} response"))?;
    match response.get("content") {
        None => Ok(None),
        Some(content) => content
            .get("application/json")
            .and_then(|json| json.get("schema"))
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                format!("the {status} response of {method} {template} declares no JSON schema")
            }),
    }
}

/// Every object schema of `document` that leaves a property out of
/// `required`, as `JSON pointer: property`.
///
/// public-api.md §2.1: every field of a schema is always present, an empty
/// one as `null`; serde writes every field, so the schema must require each
/// (docs/design/openapi-spike.md §3, rule 1). A field that is really left
/// out sometimes would need a serde skip and an entry in a list of
/// exceptions; there is none.
///
/// - `document`: the OpenAPI document.
#[must_use]
pub fn unrequired_properties(document: &Value) -> Vec<String> {
    let mut found = Vec::new();
    walk_schemas(document, &mut String::new(), &mut found);
    found
}

/// The recursion of [`unrequired_properties`]; `pointer` is the JSON
/// pointer of `value`.
fn walk_schemas(value: &Value, pointer: &mut String, found: &mut Vec<String>) {
    let length = pointer.len();
    match value {
        Value::Object(members) => {
            if let Some(Value::Object(properties)) = members.get("properties") {
                let required: Vec<&str> = members
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|names| names.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                for name in properties.keys() {
                    if !required.contains(&name.as_str()) {
                        found.push(format!("{pointer}: {name}"));
                    }
                }
            }
            for (key, member) in members {
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                walk_schemas(member, pointer, found);
                pointer.truncate(length);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                pointer.push('/');
                pointer.push_str(&index.to_string());
                walk_schemas(item, pointer, found);
                pointer.truncate(length);
            }
        }
        _ => {}
    }
}

/// Asserts that `body` matches the schema that `GET template` declares for
/// `status` (ADR 0005 §4), for the endpoint tests' own responses.
///
/// - `document`: the served document ([`served_document`]).
/// - `template`: the route template, for example `/api/v1/posts/{id}`.
/// - `status`: the response's status.
/// - `body`: the parsed body.
/// - `context`: what was requested, for the failure message.
///
/// # Panics
///
/// When the body breaks the schema, the operation declares no JSON body
/// for `status`, or the document does not describe the route.
pub fn assert_matches_operation(
    document: &Value,
    template: &str,
    status: StatusCode,
    body: &Value,
    context: &str,
) {
    let schema = response_schema(document, &Method::GET, template, status)
        .unwrap_or_else(|problem| panic!("{context}: {problem}"))
        .unwrap_or_else(|| panic!("{context}: GET {template} declares no {status} body"));
    let errors = schema_errors(document, &schema, body);
    assert!(
        errors.is_empty(),
        "{context}: the body does not match its OpenAPI schema:\n{}\n{body}",
        errors.join("\n")
    );
}
