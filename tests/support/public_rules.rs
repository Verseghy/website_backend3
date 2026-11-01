//! The global rules of `/api/v1` (public-api.md §16.3) as checks that any
//! test server can be put through.
//!
//! `tests/requests/public_global.rs` runs them for every route of Loco's
//! route table with the samples of `tests/support/public_samples.rs`.
//! `tests/requests/public_harness.rs` runs them against probe routes that
//! break one rule each, so a check that stopped finding its fault fails
//! there. That is why a check returns what it found instead of panicking.
//!
//! [`answer_problems`] holds the rules for one answer, whatever was asked:
//!
//! - never a `5xx` for anything a client sends (§8);
//! - exactly one `Access-Control-Allow-Origin: *`, and no other CORS header,
//!   on every answer, errors included (§2.1, ADR 0014);
//! - a `405` has `Allow: GET,HEAD`, `no-store` and axum's empty body, which
//!   public-api.md §8 leaves as it is;
//! - every other `4xx` is the JSON `Error` envelope of §6.15 with a code of
//!   that status, `no-store` and no `ETag` (§8, §9);
//! - a `200` is JSON with `no-cache` and the weak `ETag` of its body, or
//!   `no-store` without one (a preview); its body matches the schema the
//!   document declares for it and holds no soft-deleted twin (ADR 0011);
//! - a `304` and every `HEAD` answer have an empty body.
//!
//! [`route_violations`] holds the rules that compare answers of one route:
//! the trailing-slash form answers the same (DEC-10), `POST` gets `405`,
//! unknown query parameters are ignored, the first of a repeated parameter
//! wins, `+` is a space, and `If-None-Match` with the returned `ETag` gets
//! `304`. [`random_input_failures`] is the property that no value of any
//! parameter, and no unknown parameter, makes a route answer `5xx`.
//!
//! A rule that compares two answers passes when they are the same, so it
//! can pass for samples that never reach what it checks. [`route_violations`]
//! therefore also reports the samples (under the rule `samples`) when the
//! base request does not get the route's cacheable `200`, when that `200`
//! lists no row whose soft-deleted twin the entry's fixtures wrote, when a
//! parameter's `other` value gets the same answer as `accepted`, and when a
//! spaced value gets the same answer with a real plus sign: each of these
//! would let a broken route pass.

use std::collections::BTreeSet;

use axum::http::{
    HeaderName, Method, StatusCode,
    header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH, ORIGIN, VARY},
};
use axum_test::{TestResponse, TestServer};
use backend3::{domain::etag::weak_etag, dtos::v1::error::ErrorCode};
use proptest::{
    prelude::*,
    test_runner::{Config, TestCaseError, TestRunner},
};
use serde_json::Value;

use super::{
    SITE_ORIGIN,
    fixtures::{Loaded, row_ids_in, twin_ids_in},
    openapi_schema::{component_errors, response_schema, schema_errors},
    public_samples::{
        QuerySample, RouteSamples, Rows, Spaced, Spaces, Target, fill_path, unknown_parameters,
    },
};

/// One answer, kept whole so that two answers can be compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The request line, for messages.
    pub request: String,
    /// The status.
    pub status: StatusCode,
    /// Every header, in order, with repeated ones kept.
    pub headers: Vec<(String, String)>,
    /// The body bytes.
    pub body: Vec<u8>,
}

impl Answer {
    /// The answer that `response` holds.
    ///
    /// - `request`: what was sent, for messages.
    /// - `response`: the test server's response.
    #[must_use]
    pub fn of(request: String, response: &TestResponse) -> Self {
        Self {
            request,
            status: response.status_code(),
            headers: response
                .iter_headers()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            body: response.as_bytes().to_vec(),
        }
    }

    /// The values of header `name`, in order.
    ///
    /// - `name`: the header to read.
    #[must_use]
    pub fn header(&self, name: &HeaderName) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key == name.as_str())
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// The parts that two answers to the same question share: the status,
    /// the body and the representation headers. `Date`, the request id and
    /// the like differ between any two answers.
    fn essence(&self) -> (StatusCode, &[u8], [Vec<&str>; 3]) {
        (
            self.status,
            &self.body,
            [
                self.header(&CONTENT_TYPE),
                self.header(&CACHE_CONTROL),
                self.header(&ETAG),
            ],
        )
    }
}

/// Sends `method target` with an `Origin`, as the public site's browser
/// code does (ADR 0014), and any `headers`.
///
/// - `server`: the test server.
/// - `method`: the request method.
/// - `target`: the path and query, already encoded.
/// - `headers`: extra request headers.
pub async fn send(
    server: &TestServer,
    method: Method,
    target: &str,
    headers: &[(HeaderName, &str)],
) -> Answer {
    let mut request = server
        .method(method.clone(), target)
        .add_header(ORIGIN, SITE_ORIGIN);
    for (name, value) in headers {
        request = request.add_header(name.clone(), *value);
    }
    let response = request.await;
    Answer::of(format!("{method} {target}"), &response)
}

/// Every rule of one answer that `answer` breaks (the module documentation
/// lists them), one line each.
///
/// - `document`: the served OpenAPI document.
/// - `method`: the request method.
/// - `route`: the route template whose declared schema a `200` body must
///   match, or `None` when the answer is not known to come from one route
///   (a random path value may reach another), which checks everything but
///   the body's schema.
/// - `answer`: the answer to check.
#[must_use]
pub fn answer_problems(
    document: &Value,
    method: &Method,
    route: Option<&str>,
    answer: &Answer,
) -> Vec<String> {
    let mut problems = Vec::new();
    let status = answer.status;
    let cors: Vec<(&str, &str)> = answer
        .headers
        .iter()
        .filter(|(name, _)| name.starts_with("access-control-"))
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    if cors != [("access-control-allow-origin", "*")] {
        problems.push(format!(
            "the CORS headers are {cors:?}; every /api/v1 answer has exactly \
             Access-Control-Allow-Origin: * (ADR 0014)"
        ));
    }
    if answer
        .header(&VARY)
        .iter()
        .any(|vary| vary.to_ascii_lowercase().contains("origin"))
    {
        problems.push("Vary names the origin, but the answer is the same for all".to_owned());
    }
    let empty_body = *method == Method::HEAD || status == StatusCode::NOT_MODIFIED;
    if empty_body && !answer.body.is_empty() {
        problems.push(format!("a {status} to {method} has a body"));
    }
    if status.is_server_error() {
        problems.push(format!(
            "answered {status}: user input never causes a 5xx (public-api.md §8)"
        ));
    } else if status == StatusCode::METHOD_NOT_ALLOWED {
        problems.extend(not_allowed_problems(answer));
    } else if status.is_client_error() {
        problems.extend(error_problems(document, method, answer));
    } else if status == StatusCode::OK {
        problems.extend(success_problems(document, method, route, answer));
    } else if status == StatusCode::NOT_MODIFIED {
        if answer.header(&ETAG).len() != 1 {
            problems.push("a 304 repeats the ETag of the 200".to_owned());
        }
        if answer.header(&CACHE_CONTROL) != ["no-cache"] {
            problems.push("a 304 repeats Cache-Control: no-cache".to_owned());
        }
    } else {
        problems.push(format!("{status} is not an answer of the public API"));
    }
    problems
}

/// The rules of axum's `405` (public-api.md §8).
fn not_allowed_problems(answer: &Answer) -> Vec<String> {
    let mut problems = Vec::new();
    let allowed: BTreeSet<&str> = answer
        .header(&ALLOW)
        .iter()
        .flat_map(|allow| allow.split(','))
        .map(str::trim)
        .collect();
    if allowed != BTreeSet::from(["GET", "HEAD"]) {
        problems.push(format!("a 405 allows {allowed:?}, not GET and HEAD"));
    }
    if !answer.body.is_empty() {
        problems.push("a 405 has axum's empty body (public-api.md §8)".to_owned());
    }
    if answer.header(&CACHE_CONTROL) != ["no-store"] {
        problems.push("a 405 is never stored (Cache-Control: no-store, §9)".to_owned());
    }
    problems
}

/// The rules of a `4xx` other than `405`: the JSON envelope (public-api.md
/// §6.15, §8) with a code of that status, `no-store` and no `ETag` (§9).
fn error_problems(document: &Value, method: &Method, answer: &Answer) -> Vec<String> {
    let mut problems = Vec::new();
    let status = answer.status;
    if answer.header(&CONTENT_TYPE) != ["application/json"] {
        problems.push(format!(
            "a {status} is the JSON error envelope, but its Content-Type is {:?}",
            answer.header(&CONTENT_TYPE)
        ));
    }
    if answer.header(&CACHE_CONTROL) != ["no-store"] {
        problems.push(format!(
            "a {status} is never stored (Cache-Control: no-store)"
        ));
    }
    if !answer.header(&ETAG).is_empty() {
        problems.push(format!("a {status} has no ETag"));
    }
    if *method == Method::HEAD {
        return problems;
    }
    match serde_json::from_slice::<Value>(&answer.body) {
        Err(error) => problems.push(format!("a {status} body is not JSON: {error}")),
        Ok(body) => {
            problems.extend(
                component_errors(document, "Error", &body)
                    .into_iter()
                    .map(|error| format!("the {status} body is not an Error: {error}")),
            );
            let code = ErrorCode::ALL
                .into_iter()
                .find(|code| body.get("error").and_then(Value::as_str) == Some(code.as_str()));
            if let Some(code) = code
                && code.status() != status
            {
                problems.push(format!(
                    "the code {} belongs to {}, not to {status}",
                    code.as_str(),
                    code.status()
                ));
            }
        }
    }
    problems
}

/// The rules of a `200` (public-api.md §9, ADR 0005 §4, ADR 0011).
fn success_problems(
    document: &Value,
    method: &Method,
    route: Option<&str>,
    answer: &Answer,
) -> Vec<String> {
    let mut problems = Vec::new();
    if answer.header(&CONTENT_TYPE) != ["application/json"] {
        problems.push(format!(
            "a 200 is JSON, but its Content-Type is {:?}",
            answer.header(&CONTENT_TYPE)
        ));
    }
    let etag = answer.header(&ETAG);
    match answer.header(&CACHE_CONTROL).as_slice() {
        ["no-cache"] => {
            if etag.len() != 1 {
                problems.push(format!("a no-cache 200 has one ETag, not {etag:?}"));
            } else if *method == Method::GET && etag[0] != weak_etag(&answer.body) {
                problems.push(format!(
                    "the ETag {} is not the weak tag of the body",
                    etag[0]
                ));
            }
        }
        ["no-store"] => {
            if !etag.is_empty() {
                problems.push("a no-store 200 (a preview) has no ETag".to_owned());
            }
        }
        other => problems.push(format!(
            "a 200 has Cache-Control no-cache, or no-store for a preview, not {other:?}"
        )),
    }
    if *method == Method::HEAD {
        return problems;
    }
    let body = match serde_json::from_slice::<Value>(&answer.body) {
        Ok(body) => body,
        Err(error) => {
            problems.push(format!("the 200 body is not JSON: {error}"));
            return problems;
        }
    };
    for twin in twin_ids_in(&body) {
        problems.push(format!(
            "the body holds the soft-deleted twin {twin} (ADR 0011 §3)"
        ));
    }
    if let Some(route) = route {
        match response_schema(document, method, route, StatusCode::OK) {
            Err(problem) => problems.push(problem),
            Ok(None) => {}
            Ok(Some(schema)) => problems.extend(
                schema_errors(document, &schema, &body)
                    .into_iter()
                    .map(|error| format!("the body does not match its OpenAPI schema: {error}")),
            ),
        }
    }
    problems
}

/// What makes `actual` a different answer than `expected`, if anything.
fn differences(expected: &Answer, actual: &Answer) -> Option<String> {
    (expected.essence() != actual.essence()).then(|| {
        format!(
            "{} answered {} {:?} with {} body bytes, but {} answered {} {:?} with {}",
            expected.request,
            expected.status,
            expected.essence().2,
            expected.body.len(),
            actual.request,
            actual.status,
            actual.essence().2,
            actual.body.len()
        )
    })
}

/// Every global rule (the module documentation) that the route of
/// `samples` breaks, each line naming the route and the rule. A line under
/// the rule `samples` means that the samples cannot show a rule, so the
/// entry has to change, not the route.
///
/// - `server`: the test server, with every fixture that `samples` names
///   loaded.
/// - `document`: the served OpenAPI document.
/// - `samples`: the route and the values to send.
/// - `loaded`: what loading the fixtures wrote; the twins of the entry's own
///   fixtures are the rows that its base answer must list ([`Rows`]).
pub async fn route_violations(
    server: &TestServer,
    document: &Value,
    samples: &RouteSamples,
    loaded: &Loaded,
) -> Vec<String> {
    let base_target = match samples.target() {
        Ok(target) => target,
        Err(problem) => return vec![format!("{}: samples: {problem}", samples.route)],
    };
    let mut run = RouteRun {
        server,
        document,
        samples,
        base_target,
        violations: Vec::new(),
    };
    let base = run.get(run.base_target.clone(), Spaces::Plus).await;
    run.base_rules(&base, loaded);
    run.path_and_method_rules(&base).await;
    run.unknown_parameter_rules(&base).await;
    for (index, sample) in samples.query.iter().enumerate() {
        run.parameter_rules(&base, index, sample).await;
    }
    run.revalidation_rule(&base).await;
    run.violations
}

/// One run of [`route_violations`]: what it sends to and what it found.
struct RouteRun<'a> {
    /// The test server.
    server: &'a TestServer,
    /// The served OpenAPI document.
    document: &'a Value,
    /// The route and its samples.
    samples: &'a RouteSamples,
    /// The base request: the sample path with every `accepted` value that
    /// is part of it.
    base_target: Target,
    /// Every broken rule so far.
    violations: Vec<String>,
}

impl RouteRun<'_> {
    /// Records `problems` as violations of `rule`.
    fn note(&mut self, rule: &str, problems: impl IntoIterator<Item = String>) {
        let route = self.samples.route;
        self.violations.extend(
            problems
                .into_iter()
                .map(|problem| format!("{route}: {rule}: {problem}")),
        );
    }

    /// Sends `GET target` with spaces written as `spaces`, and records what
    /// the answer breaks of [`answer_problems`].
    async fn get(&mut self, target: Target, spaces: Spaces) -> Answer {
        let answer = send(self.server, Method::GET, &target.render(spaces), &[]).await;
        let problems = answer_problems(
            self.document,
            &Method::GET,
            Some(self.samples.route),
            &answer,
        );
        self.note("every answer", problems);
        answer
    }

    /// The base answer is the route's cacheable `200`, and it lists a row
    /// whose twin one of the entry's fixtures wrote ([`Rows::Twinned`]).
    /// Without these the rules of a `200` (the schema, the twins, the
    /// `ETag`, the `304`) would check nothing and pass, so a failure is the
    /// samples' fault.
    fn base_rules(&mut self, base: &Answer, loaded: &Loaded) {
        if base.status != StatusCode::OK || base.header(&CACHE_CONTROL) != ["no-cache"] {
            self.note(
                "samples",
                [format!(
                    "the base request {} answered {} with Cache-Control {:?}, not the route's \
                     cacheable 200, so the rules of a 200 (schema, twins, ETag, 304) are not \
                     checked; choose path and `accepted` values that reach a live row of the \
                     entry's fixtures, and keep an optional parameter that changes the answer, \
                     such as a preview token, out of the base request (`in_base: false`)",
                    base.request,
                    base.status,
                    base.header(&CACHE_CONTROL)
                )],
            );
            return;
        }
        let Rows::Twinned = self.samples.rows else {
            return;
        };
        let fixtures = self.samples.fixtures;
        let twinned: BTreeSet<i64> = loaded
            .twins
            .iter()
            .filter(|twin| fixtures.contains(&twin.fixture))
            .map(|twin| twin.live_id)
            .collect();
        // A body that is not JSON is reported by `answer_problems` already.
        // The ids are compared without their tables, which a body does not
        // name: a label and a post with one id look alike. That can only let
        // weak samples pass, never fail a correct route, and the endpoint
        // tests assert the twins of each table themselves.
        let listed = serde_json::from_slice::<Value>(&base.body)
            .map(|body| row_ids_in(&body))
            .unwrap_or_default();
        if listed.is_disjoint(&twinned) {
            self.note(
                "samples",
                [format!(
                    "the base answer lists the row ids {listed:?}, none of which has a \
                     soft-deleted twin from the fixtures {fixtures:?} (they twinned the rows \
                     {twinned:?}), so the twin check has nothing to miss; name the fixtures of \
                     rows the route lists, write their twins (FixtureWriter::twin), and choose \
                     path values that reach them"
                )],
            );
        }
    }

    /// The trailing-slash form answers the same (DEC-10), and `POST` gets
    /// axum's `405` with `Allow` and an empty body (§2.1, §8).
    async fn path_and_method_rules(&mut self, base: &Answer) {
        let slash = self
            .get(self.base_target.with_trailing_slash(), Spaces::Plus)
            .await;
        self.note(
            "the trailing-slash form answers the same",
            differences(base, &slash),
        );
        let post = send(self.server, Method::POST, &self.base_target.uri(), &[]).await;
        let mut problems = answer_problems(
            self.document,
            &Method::POST,
            Some(self.samples.route),
            &post,
        );
        if post.status != StatusCode::METHOD_NOT_ALLOWED {
            problems.push(format!(
                "{} answered {}, not 405",
                post.request, post.status
            ));
        }
        self.note("POST answers 405", problems);
    }

    /// Unknown query parameters are ignored, before the known ones and
    /// after them (§2.1).
    async fn unknown_parameter_rules(&mut self, base: &Answer) {
        let unknown = unknown_parameters(self.samples);
        for at_start in [true, false] {
            let mut target = self.base_target.clone();
            for (offset, pair) in unknown.iter().enumerate() {
                let index = if at_start { offset } else { target.pairs.len() };
                target = target.with_pair_at(index, *pair);
            }
            let answer = self.get(target, Spaces::Plus).await;
            self.note(
                "unknown query parameters are ignored",
                differences(base, &answer),
            );
        }
    }

    /// For the query parameter at `index`, starting from its reference
    /// request ([`RouteSamples::reference`]): the first of a repeated
    /// parameter wins, in both orders, and `+` is a space like `%20`
    /// (§2.1).
    async fn parameter_rules(&mut self, base: &Answer, index: usize, sample: &QuerySample) {
        let (reference_target, at) = match self.samples.reference(index) {
            Ok(reference) => reference,
            Err(problem) => {
                self.note("samples", [problem]);
                return;
            }
        };
        let reference = if reference_target == self.base_target {
            base.clone()
        } else {
            self.get(reference_target.clone(), Spaces::Plus).await
        };
        let other_target = reference_target.with_value(at, sample.other);
        let other = self.get(other_target.clone(), Spaces::Plus).await;
        if differences(&reference, &other).is_none() {
            self.note(
                "samples",
                [format!(
                    "`{}={}` and `{}={}` get the same answer, so the first-value check cannot \
                     tell which one the route read; choose an `other` that is answered \
                     differently, such as a value the route refuses",
                    sample.name, sample.accepted, sample.name, sample.other
                )],
            );
        }
        let accepted_first = self
            .get(
                reference_target.with_pair_at(at + 1, (sample.name, sample.other)),
                Spaces::Plus,
            )
            .await;
        let other_first = self
            .get(
                other_target.with_pair_at(at + 1, (sample.name, sample.accepted)),
                Spaces::Plus,
            )
            .await;
        self.note(
            "the first of a repeated parameter wins",
            differences(&reference, &accepted_first)
                .into_iter()
                .chain(differences(&other, &other_first)),
        );
        let Spaced::Value(spaced) = sample.spaced else {
            return;
        };
        let spaced_target = reference_target.with_value(at, spaced);
        let plus = self.get(spaced_target.clone(), Spaces::Plus).await;
        let percent = self.get(spaced_target, Spaces::Percent).await;
        self.note(
            "`+` in a query value is a space",
            differences(&percent, &plus),
        );
        // The same text with a real plus sign (`%2B`) for each space: a
        // route that read `+` literally would answer `+` like this, so the
        // two must differ for the comparison above to show anything.
        let literal = self
            .get(
                reference_target.with_value(at, &spaced.replace(' ', "+")),
                Spaces::Percent,
            )
            .await;
        if differences(&percent, &literal).is_none() {
            self.note(
                "samples",
                [format!(
                    "`{}={spaced}` gets the same answer with a space as with a plus sign, so \
                     it cannot show that `+` is read as a space; choose a `Spaced::Value` that \
                     the route answers differently with a plus sign (a search text that only \
                     the spaced form matches), or `Spaced::Unobservable` with the reason when \
                     the parameter has none",
                    sample.name
                )],
            );
        }
    }

    /// `If-None-Match` with the `ETag` of a cacheable `200` gets `304`
    /// with an empty body and the same tag (§9).
    async fn revalidation_rule(&mut self, base: &Answer) {
        if base.status == StatusCode::OK
            && base.header(&CACHE_CONTROL) == ["no-cache"]
            && let [etag] = base.header(&ETAG).as_slice()
        {
            let revalidated = send(
                self.server,
                Method::GET,
                &self.base_target.uri(),
                &[(IF_NONE_MATCH, etag)],
            )
            .await;
            let mut problems = answer_problems(
                self.document,
                &Method::GET,
                Some(self.samples.route),
                &revalidated,
            );
            if revalidated.status != StatusCode::NOT_MODIFIED {
                problems.push(format!(
                    "If-None-Match: {etag} answered {}, not 304",
                    revalidated.status
                ));
            } else if revalidated.header(&ETAG) != [*etag] {
                problems.push(format!(
                    "the 304 carries {:?}, not the ETag {etag}",
                    revalidated.header(&ETAG)
                ));
            }
            self.note("If-None-Match with the returned ETag gets 304", problems);
        }
    }
}

/// The values the random-input property sends: any Unicode text, U+0000
/// and the controls included; the bytes that mean something in a URL or a
/// query; long digit runs past every numeric range; and long texts past the
/// length limits (public-api.md §2.5).
fn random_value() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[%+&=#?/\\\\.;:,'\" \\x00-\\x1f\\x7f]{0,16}",
        "[+-]?[0-9]{0,30}",
        "\\PC{150,300}",
    ]
}

/// Whether some value of some parameter of the route of `samples`, or an
/// unknown parameter, makes it break a rule of [`answer_problems`], most
/// importantly the "never a `5xx`" of public-api.md §8. Each case replaces
/// one path value or one query value of the base request by a random
/// [`random_value`] (adding the parameter when the base request leaves it
/// out), or adds a parameter with a random name and value.
///
/// It runs proptest's runner with shrinking, so a failure names the
/// smallest value found. The runner calls a plain closure, so each request
/// runs on the test's runtime through `block_in_place`, which needs the
/// multi-thread runtime (`#[tokio::test(flavor = "multi_thread")]`).
///
/// - `server`: the test server.
/// - `document`: the served OpenAPI document.
/// - `samples`: the route and its base request.
/// - `cases`: how many random cases to run.
///
/// # Errors
///
/// The shrunk failing case and the rules it broke.
pub fn random_input_failures(
    server: &TestServer,
    document: &Value,
    samples: &RouteSamples,
    cases: u32,
) -> Result<(), String> {
    let base = samples.target()?;
    let path_slots = samples.path.len();
    let slots = path_slots + samples.query.len() + 1;
    let handle = tokio::runtime::Handle::current();
    let mut runner = TestRunner::new(Config {
        cases,
        // A manual runner has no source file to keep regressions beside.
        failure_persistence: None,
        ..Config::default()
    });
    runner
        .run(
            &(0..slots, random_value(), random_value()),
            |(slot, value, name)| {
                let target = if slot < path_slots {
                    // An empty or all-dot segment would change the route
                    // (the URL parser drops `.` and climbs `..`), so such a
                    // value gets a leading `x`.
                    let value = if value.chars().all(|c| c == '.') {
                        format!("x{value}")
                    } else {
                        value
                    };
                    let mut values: Vec<(&str, &str)> = samples.path.to_vec();
                    let (key, _) = values[slot];
                    values[slot] = (key, value.as_str());
                    let path = fill_path(samples.route, &values).map_err(TestCaseError::fail)?;
                    Target {
                        path,
                        pairs: base.pairs.clone(),
                    }
                } else if slot < slots - 1 {
                    // A parameter outside the base request is added.
                    base.with_param(samples.query[slot - path_slots].name, &value)
                        .0
                } else {
                    base.with_pair_at(base.pairs.len(), (&name, &value))
                };
                let uri = target.uri();
                let answer = tokio::task::block_in_place(|| {
                    handle.block_on(send(server, Method::GET, &uri, &[]))
                });
                // A random path value may reach another route, so the body
                // is not held to this route's schema.
                let problems = answer_problems(document, &Method::GET, None, &answer);
                if problems.is_empty() {
                    Ok(())
                } else {
                    Err(TestCaseError::fail(format!(
                        "GET {uri}: {}",
                        problems.join("; ")
                    )))
                }
            },
        )
        .map_err(|error| format!("{}: {error}", samples.route))
}
