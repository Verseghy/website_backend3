//! The media contract suite: what every server of `/storage/**` must do
//! (ADR 0015 §1, with the two header rows of ADR 0013 §7).
//!
//! Two servers must meet the contract: the nginx media server, configured by
//! `deploy/media/nginx.conf`, and backend3's fallback router (ADR 0015 §4).
//! [`run`] checks either one against the fixture tree of [`seed`], through
//! the [`Transport`] and at the [`Origin`] its caller passes in. The two may
//! differ where the contract says so: only nginx must send `ETag` (chosen
//! by [`Server`]), and a multi-range request may get the whole file with
//! `200` (nginx, `max_ranges 1`) or `416` (the fallback's `ServeDir`) from
//! either, because "the contract suite accepts either answer" (ADR 0015 §1,
//! Ranges).
//!
//! Callers:
//! - `nginx`: the Test job starts the pinned image with the real
//!   configuration over a seeded tree and runs the suite against it.
//! - `examples/media_contract.rs`: `seed` and `check` from the command line,
//!   for a pod behind `oc port-forward` (the co-mount test, ADR 0015 §3).
//! - The fallback router's request tests, once that router exists.
//!
//! The file checks are sent once per host, with the `Host` of each of the
//! three hosts that route `/storage` (ADR 0015 §2), because ADR 0013 §7
//! requires the headers on all three. A router picks its Route by that
//! header, so the suite checks the server itself (a pod through
//! `oc port-forward`, a container, a test router), never a Route. It
//! collects every failure instead of stopping at the first, so one run shows
//! all of them.
//!
//! # Readings
//!
//! The contract tables of ADR 0015 §1 and ADR 0013 §7 leave some answers
//! open. The suite holds both servers to these readings; each is one check
//! in `checks.rs` or below, so a different reading is a one-line change:
//!
//! - A refused path (missing, a directory, a dot-file, a traversal or a
//!   symbolic link) is not cacheable: no `Cache-Control`, or only `no-store`
//!   or `no-cache`, so a file uploaded later shows up ([`checks::refused`]).
//!   A blanket `Cache-Control` layer on every response fails this.
//! - A `405` carries `Allow` listing `GET` and `HEAD`, as RFC 9110 §15.5.6
//!   requires (`Probe::methods`).
//! - A traversal probe gets `400` or `404` (`Probe::traversal`).
//! - A path through a symbolic link gets `403` or `404`, wherever the link
//!   points, inside the tree or out of it: only regular files are served
//!   (`Probe::links`). The public media held none on 2026-10-25.
//! - `.jfif` is `image/jpeg`, as ADR 0015 counts it with the JPEGs
//!   (`fixtures::FILES`).
//! - A PDF's `304` carries no `Content-Security-Policy`, because a browser
//!   merges a 304's headers into the cached PDF (`Probe::revalidate`).
//! - Each contract header is sent exactly once, and every other
//!   single-valued header at most once ([`checks::common`]).

mod checks;
mod fixtures;
mod http;
mod rules;

#[cfg(test)]
mod nginx;

use std::fmt;

use axum::http::{Method, StatusCode};

pub use fixtures::seed;
pub use http::{HyperTransport, Origin, Transport};

use fixtures::{
    AFTER, BEFORE, DIAKTOLL, DIRECTORIES, DOWNLOAD, ENTITY_IMAGE, FILES, Fixture, HIDDEN,
    LAST_MODIFIED, MISSING, THROUGH_LINKS,
};
use http::{Request, Response};
use rules::{lists, storage_target};

/// The hosts whose `/storage` Routes reach the media server (ADR 0015 §2).
pub const HOSTS: [&str; 3] = [
    "verseghy-gimnazium.net",
    "backend.microshift.verseghy-gimnazium.net",
    "backend3.microshift.verseghy-gimnazium.net",
];

/// The public site, the origin of the Diáktoll viewer's cross-origin reads.
const SITE_ORIGIN: &str = "https://verseghy-gimnazium.net";

/// Which server is under test, for the one header the contract asks of
/// nginx alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Server {
    /// The nginx media server (ADR 0015 §1), which must send `ETag`.
    Nginx,
    /// backend3's fallback router (ADR 0015 §4), whose `ServeDir` sends no
    /// `ETag`.
    Fallback,
}

/// The outcome of one run.
#[derive(Debug)]
pub struct Report {
    /// How many requests the run sent.
    pub requests: usize,
    /// One line per failed expectation, each naming the request.
    pub failures: Vec<String>,
}

impl Report {
    /// Whether the server met the whole contract.
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "media contract: {} requests, {} failures",
            self.requests,
            self.failures.len()
        )?;
        for failure in &self.failures {
            writeln!(f, "  - {failure}")?;
        }
        Ok(())
    }
}

/// Runs the whole suite and reports every failure.
///
/// - `transport`: the HTTP client to send each request with. If it cannot
///   reach `origin` at all ([`Transport::reaches`]), the report holds that
///   one failure and no request is sent.
/// - `origin`: where the server under test itself listens, not a Route (see
///   the module docs). Its `/storage` must serve the tree that [`seed`]
///   writes. Other files may sit beside it, but none may shadow a fixture:
///   every path of the fixture table must be as `seed` writes it, and the
///   paths it lists as missing must not exist.
/// - `server`: which server it is, for the `ETag` that only nginx must send.
pub async fn run<T: Transport>(transport: &T, origin: &Origin, server: Server) -> Report {
    if let Err(error) = transport.reaches(origin) {
        return Report {
            requests: 0,
            failures: vec![error],
        };
    }
    let mut probe = Probe {
        transport,
        origin,
        server,
        requests: 0,
        failures: Vec::new(),
    };
    probe.files().await;
    probe.head().await;
    probe.not_served().await;
    probe.traversal().await;
    probe.links().await;
    probe.ranges().await;
    probe.conditional().await;
    probe.preflight().await;
    probe.methods().await;
    Report {
        requests: probe.requests,
        failures: probe.failures,
    }
}

/// One run's state: where to send, and what failed so far.
struct Probe<'a, T> {
    transport: &'a T,
    origin: &'a Origin,
    server: Server,
    requests: usize,
    failures: Vec<String>,
}

impl<T: Transport> Probe<'_, T> {
    /// Sends `request`; a transport error is a failure, and `None`.
    async fn send(&mut self, request: &Request) -> Option<Response> {
        self.requests += 1;
        match self.transport.send(self.origin, request).await {
            Ok(response) => Some(response),
            Err(error) => {
                self.failures.push(error);
                None
            }
        }
    }

    /// Records each of `problems` against `request`.
    fn record(&mut self, request: &Request, problems: Vec<String>) {
        self.failures.extend(
            problems
                .into_iter()
                .map(|problem| format!("{request}: {problem}")),
        );
    }

    /// Records `problem` against `request` unless `holds`.
    fn expect(&mut self, request: &Request, holds: bool, problem: impl FnOnce() -> String) {
        if !holds {
            self.failures.push(format!("{request}: {}", problem()));
        }
    }

    /// Records a wrong status; `true` when the status is as expected.
    fn status(&mut self, request: &Request, response: &Response, expected: StatusCode) -> bool {
        let holds = response.status == expected;
        self.expect(request, holds, || {
            format!("status must be {expected}, got {}", response.status)
        });
        holds
    }

    /// Records what [`checks::common`] finds, judged by the response's own
    /// type.
    fn common(&mut self, request: &Request, response: &Response) {
        self.record(
            request,
            checks::common(response, response.single("content-type")),
        );
    }

    /// Records unless `name` was sent exactly once, as `expected`.
    fn header(&mut self, request: &Request, response: &Response, name: &str, expected: &str) {
        self.expect(request, response.single(name) == Some(expected), || {
            format!(
                "{name} must be sent once as {expected:?}, got {:?}",
                response.values(name)
            )
        });
    }

    /// Records unless `name` was sent exactly once and lists every token.
    fn lists_all(&mut self, request: &Request, response: &Response, name: &str, tokens: &[&str]) {
        let holds = response
            .single(name)
            .is_some_and(|value| tokens.iter().all(|token| lists(value, token)));
        self.expect(request, holds, || {
            format!(
                "{name} must be sent once and list {}, got {:?}",
                tokens.join(", "),
                response.values(name)
            )
        });
    }

    /// Records a body that differs from `expected`.
    fn body(&mut self, request: &Request, response: &Response, expected: &[u8]) {
        self.expect(request, response.body.as_ref() == expected, || {
            format!(
                "body must be the {} expected bytes, got {} bytes",
                expected.len(),
                response.body.len()
            )
        });
    }

    /// Every file on every host: path mapping, accented names, upper-case
    /// extensions, type, CORS, sandbox, download, caching headers.
    async fn files(&mut self) {
        for host in HOSTS {
            for fixture in FILES {
                let request =
                    Request::new(Method::GET, storage_target(fixture.path)).header("host", host);
                let Some(response) = self.send(&request).await else {
                    continue;
                };
                if self.status(&request, &response, StatusCode::OK) {
                    self.body(&request, &response, &fixture.body());
                    self.whole_file(&request, &response, fixture);
                }
            }
        }
    }

    /// The headers of a complete file answer.
    fn whole_file(&mut self, request: &Request, response: &Response, fixture: &Fixture) {
        self.header(
            request,
            response,
            "content-length",
            &fixture.size.to_string(),
        );
        self.common(request, response);
        self.record(request, checks::file(response, fixture, self.server));
    }

    /// HEAD answers like GET, without a body.
    async fn head(&mut self) {
        for fixture in [&DIAKTOLL, &DOWNLOAD, &ENTITY_IMAGE] {
            let request = Request::new(Method::HEAD, storage_target(fixture.path));
            let Some(response) = self.send(&request).await else {
                continue;
            };
            if self.status(&request, &response, StatusCode::OK) {
                self.body(&request, &response, b"");
                self.whole_file(&request, &response, fixture);
            }
        }
    }

    /// Dot-files, directories and names that differ only in case or
    /// normalisation: all 404, with the contract headers.
    async fn not_served(&mut self) {
        for path in HIDDEN.iter().chain(DIRECTORIES).chain(MISSING) {
            let request = Request::new(Method::GET, storage_target(path));
            let Some(response) = self.send(&request).await else {
                continue;
            };
            self.record(
                &request,
                checks::refused(&response, &[StatusCode::NOT_FOUND]),
            );
            self.common(&request, &response);
        }
    }

    /// Dot-segments, raw and encoded, never leave the storage root.
    async fn traversal(&mut self) {
        for target in [
            "/storage/../../etc/passwd",
            "/storage/%2e%2e/%2e%2e/etc/passwd",
            "/storage/uploads/..%2f..%2f..%2fetc%2fpasswd",
        ] {
            let request = Request::new(Method::GET, target);
            let Some(response) = self.send(&request).await else {
                continue;
            };
            let allowed = [StatusCode::BAD_REQUEST, StatusCode::NOT_FOUND];
            self.record(&request, checks::refused(&response, &allowed));
            let leaked = response.body.windows(5).any(|window| window == b"root:");
            self.expect(&request, !leaked, || "the body shows /etc/passwd".into());
            self.common(&request, &response);
        }
    }

    /// A path through a symbolic link is refused, wherever the link points;
    /// the in-tree targets exist, so only the refusal can explain a 404.
    async fn links(&mut self) {
        for path in THROUGH_LINKS {
            let request = Request::new(Method::GET, storage_target(path));
            let Some(response) = self.send(&request).await else {
                continue;
            };
            let allowed = [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND];
            self.record(&request, checks::refused(&response, &allowed));
            self.common(&request, &response);
        }
    }

    /// Single ranges get 206 or 416, a multi-range request either the whole
    /// file with 200 or 416, all with the contract headers.
    async fn ranges(&mut self) {
        let pdf = DIAKTOLL.body();
        let size = DIAKTOLL.size;
        let partial = [("bytes=0-99", 0, 100), ("bytes=-16", size - 16, size)];
        for (range, start, end) in partial {
            self.partial(&DIAKTOLL, range, &pdf[start..end], start)
                .await;
        }
        self.partial(&DOWNLOAD, "bytes=0-9", &DOWNLOAD.body()[..10], 0)
            .await;
        for fixture in [&DIAKTOLL, &DOWNLOAD] {
            let range = format!("bytes={}-", fixture.size);
            self.unsatisfiable(fixture, &range).await;
        }

        // ADR 0015 §1, Ranges: "the contract suite accepts either answer";
        // whichever comes is then checked in full. A 206 multipart answer,
        // which nginx gives without max_ranges 1, is neither.
        let request = Self::ranged(&DIAKTOLL, "bytes=0-9,20-29");
        let Some(response) = self.send(&request).await else {
            return;
        };
        match response.status {
            StatusCode::OK => {
                self.body(&request, &response, &pdf);
                self.whole_file(&request, &response, &DIAKTOLL);
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let content_range = format!("bytes */{size}");
                self.header(&request, &response, "content-range", &content_range);
                self.common(&request, &response);
            }
            other => self.failures.push(format!(
                "{request}: a multi-range request must get 200 with the whole file or 416 \
                 (ADR 0015 §1, Ranges), got {other}"
            )),
        }
    }

    /// A GET of `fixture` with `Range: <range>` and the site's `Origin`.
    fn ranged(fixture: &Fixture, range: &str) -> Request {
        Request::new(Method::GET, storage_target(fixture.path))
            .header("origin", SITE_ORIGIN)
            .header("range", range)
    }

    /// One satisfiable range: 206 with exactly `expected` from `start`.
    async fn partial(&mut self, fixture: &Fixture, range: &str, expected: &[u8], start: usize) {
        let request = Self::ranged(fixture, range);
        let Some(response) = self.send(&request).await else {
            return;
        };
        if !self.status(&request, &response, StatusCode::PARTIAL_CONTENT) {
            return;
        }
        self.body(&request, &response, expected);
        let end = start + expected.len() - 1;
        let content_range = format!("bytes {start}-{end}/{}", fixture.size);
        self.header(&request, &response, "content-range", &content_range);
        self.common(&request, &response);
        self.record(&request, checks::file(&response, fixture, self.server));
    }

    /// A range past the end: 416 with `Content-Range: bytes */<size>`.
    async fn unsatisfiable(&mut self, fixture: &Fixture, range: &str) {
        let request = Self::ranged(fixture, range);
        let Some(response) = self.send(&request).await else {
            return;
        };
        if !self.status(&request, &response, StatusCode::RANGE_NOT_SATISFIABLE) {
            return;
        }
        let content_range = format!("bytes */{}", fixture.size);
        self.header(&request, &response, "content-range", &content_range);
        self.common(&request, &response);
    }

    /// `If-Modified-Since` (both servers) and `If-None-Match` (nginx).
    async fn conditional(&mut self) {
        for fixture in [&DIAKTOLL, &ENTITY_IMAGE] {
            let target = storage_target(fixture.path);
            for (date, expected) in [
                (LAST_MODIFIED, StatusCode::NOT_MODIFIED),
                (AFTER, StatusCode::NOT_MODIFIED),
                (BEFORE, StatusCode::OK),
            ] {
                let request = Request::new(Method::GET, &target).header("if-modified-since", date);
                self.revalidate(&request, fixture, expected).await;
            }
            if self.server == Server::Nginx {
                let plain = Request::new(Method::GET, &target);
                let Some(tag) = self
                    .send(&plain)
                    .await
                    .and_then(|response| response.single("etag").map(str::to_owned))
                else {
                    self.failures
                        .push(format!("{plain}: nginx sent no single ETag"));
                    continue;
                };
                let request = Request::new(Method::GET, &target).header("if-none-match", &tag);
                self.revalidate(&request, fixture, StatusCode::NOT_MODIFIED)
                    .await;
            }
        }
    }

    /// One conditional GET: a 304 has no body and the file's caching and
    /// sandbox headers, a 200 is the whole file.
    async fn revalidate(&mut self, request: &Request, fixture: &Fixture, expected: StatusCode) {
        let Some(response) = self.send(request).await else {
            return;
        };
        if !self.status(request, &response, expected) {
            return;
        }
        if expected == StatusCode::NOT_MODIFIED {
            self.body(request, &response, b"");
            self.record(
                request,
                checks::common(&response, Some(fixture.content_type)),
            );
            self.record(request, checks::file(&response, fixture, self.server));
        } else {
            self.body(request, &response, &fixture.body());
            self.whole_file(request, &response, fixture);
        }
    }

    /// The CORS preflight that a cross-origin range read needs.
    async fn preflight(&mut self) {
        for fixture in [&DIAKTOLL, &ENTITY_IMAGE] {
            let request = Request::new(Method::OPTIONS, storage_target(fixture.path))
                .header("origin", SITE_ORIGIN)
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "range");
            let Some(response) = self.send(&request).await else {
                continue;
            };
            let success = response.status.is_success();
            self.expect(&request, success, || {
                format!(
                    "the preflight must succeed (200 or 204), got {}",
                    response.status
                )
            });
            let methods = "access-control-allow-methods";
            self.lists_all(&request, &response, methods, &["GET", "HEAD"]);
            let headers = "access-control-allow-headers";
            self.lists_all(&request, &response, headers, &["Range"]);
            self.body(&request, &response, b"");
            self.common(&request, &response);
        }
    }

    /// Only GET, HEAD and OPTIONS: everything else is 405 with `Allow`, and
    /// the file is unchanged afterwards.
    async fn methods(&mut self) {
        let target = storage_target(DOWNLOAD.path);
        let propfind = Method::from_bytes(b"PROPFIND").expect("a valid method token");
        for method in [
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::TRACE,
            propfind,
        ] {
            let request = Request::new(method, &target);
            let Some(response) = self.send(&request).await else {
                continue;
            };
            self.status(&request, &response, StatusCode::METHOD_NOT_ALLOWED);
            self.lists_all(&request, &response, "allow", &["GET", "HEAD"]);
            self.common(&request, &response);
        }
        let request = Request::new(Method::GET, &target);
        if let Some(response) = self.send(&request).await
            && self.status(&request, &response, StatusCode::OK)
        {
            self.body(&request, &response, &DOWNLOAD.body());
        }
    }
}

#[cfg(test)]
mod tests {
    //! [`run`] over fake transports, without a server: the request count,
    //! the early refusal, and that a server which says yes to everything
    //! fails. The container test shows that a real server can pass.

    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{
        body::Bytes,
        http::{HeaderMap, HeaderValue},
    };

    use super::*;

    /// What a [`Fake`] does with every request.
    enum Answer {
        /// An empty `200` with one `Access-Control-Allow-Origin: *`.
        Yes,
        /// A transport error that names the request.
        Unreachable,
    }

    /// A transport that answers every request the same way and counts them.
    struct Fake {
        answer: Answer,
        /// `None` accepts every origin; `Some` refuses all with that reason.
        refusal: Option<&'static str>,
        sent: AtomicUsize,
    }

    impl Fake {
        fn new(answer: Answer, refusal: Option<&'static str>) -> Self {
            Self {
                answer,
                refusal,
                sent: AtomicUsize::new(0),
            }
        }
    }

    impl Transport for Fake {
        fn reaches(&self, _origin: &Origin) -> Result<(), String> {
            self.refusal.map_or(Ok(()), |reason| Err(reason.to_owned()))
        }

        // Every answer is ready at once, so no `async fn`.
        fn send(
            &self,
            _origin: &Origin,
            request: &Request,
        ) -> impl Future<Output = Result<Response, String>> + Send {
            self.sent.fetch_add(1, Ordering::Relaxed);
            std::future::ready(match self.answer {
                Answer::Yes => {
                    let mut headers = HeaderMap::new();
                    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
                    Ok(Response {
                        status: StatusCode::OK,
                        headers,
                        body: Bytes::new(),
                    })
                }
                Answer::Unreachable => Err(format!("{request}: connection refused")),
            })
        }
    }

    fn origin() -> Origin {
        "http://127.0.0.1:9".parse().expect("valid origin")
    }

    #[tokio::test]
    async fn a_server_that_says_yes_to_everything_fails_every_part() {
        let fake = Fake::new(Answer::Yes, None);
        let report = run(&fake, &origin(), Server::Nginx).await;
        assert_eq!(report.requests, fake.sent.load(Ordering::Relaxed));
        // Every probe ran: the file loop alone sends one request per file
        // and host.
        assert!(report.requests > HOSTS.len() * FILES.len(), "{report}");
        assert!(!report.passed());
        let text = report.to_string();
        for needle in [
            "status must be 404 Not Found, got 200 OK",
            "status must be 206 Partial Content",
            "status must be 416 Range Not Satisfiable",
            "status must be 304 Not Modified",
            "status must be 405 Method Not Allowed",
            "status must be 403 Forbidden or 404 Not Found",
            "status must be 400 Bad Request or 404 Not Found",
            "nginx sent no single ETag",
            "x-content-type-options must be sent once",
            "content-security-policy must be sent once",
            "body must be the 4096 expected bytes, got 0 bytes",
            "access-control-allow-methods must be sent once and list GET, HEAD",
            "[host: backend3.microshift.verseghy-gimnazium.net]",
        ] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
        // The multi-range request got a 200, which is allowed, and is then
        // judged as a whole file: an empty body fails.
        assert!(
            text.contains(
                "GET /storage/uploads/diaktoll/ujsag_final_2021osz.pdf \
                 [origin: https://verseghy-gimnazium.net] [range: bytes=0-9,20-29]: \
                 body must be the 4096 expected bytes"
            ),
            "{text}"
        );
    }

    #[tokio::test]
    async fn an_origin_the_transport_cannot_reach_sends_nothing() {
        let fake = Fake::new(Answer::Yes, Some("plain HTTP only"));
        let report = run(&fake, &origin(), Server::Nginx).await;
        assert_eq!(report.requests, 0);
        assert_eq!(report.failures, ["plain HTTP only"]);
        assert_eq!(fake.sent.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn every_transport_error_is_a_failure_and_the_run_goes_on() {
        let fake = Fake::new(Answer::Unreachable, None);
        let report = run(&fake, &origin(), Server::Fallback).await;
        // The fallback needs no ETag lookup, so each request fails once.
        assert_eq!(report.requests, fake.sent.load(Ordering::Relaxed));
        assert_eq!(report.failures.len(), report.requests);
        assert!(
            report
                .failures
                .iter()
                .all(|failure| failure.ends_with(": connection refused")),
            "{report}"
        );
    }

    #[test]
    fn a_report_counts_and_lists_its_failures() {
        let clean = Report {
            requests: 2,
            failures: Vec::new(),
        };
        assert!(clean.passed());
        assert_eq!(
            clean.to_string(),
            "media contract: 2 requests, 0 failures\n"
        );
        let failed = Report {
            requests: 3,
            failures: vec!["GET /a: one".into(), "GET /b: two".into()],
        };
        assert!(!failed.passed());
        assert_eq!(
            failed.to_string(),
            "media contract: 3 requests, 2 failures\n  - GET /a: one\n  - GET /b: two\n"
        );
    }
}
