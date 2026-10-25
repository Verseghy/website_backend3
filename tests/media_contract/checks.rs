//! Verdicts on single responses: each function lists what a response gets
//! wrong against the media contract, as messages that name the header, the
//! expected value and the value sent. An empty list is a pass.
//!
//! They are pure, so the suite's own judgement is unit-tested here with
//! hand-built responses, apart from any server.

use axum::http::StatusCode;

use super::{
    Server,
    fixtures::{Fixture, LAST_MODIFIED},
    http::Response,
    rules::{SANDBOX_CSP, cache_control, lists, media_type, needs_attachment, needs_sandbox},
};

/// Headers that have one value by definition, apart from the four that
/// [`common`] already requires exactly once (`Access-Control-Allow-Origin`,
/// `Access-Control-Expose-Headers`, `X-Content-Type-Options` and
/// `Content-Security-Policy`). A second line of any of them is a server bug:
/// a client may keep either value, or, for the CORS ones, reject the
/// response. nginx builds a 416's headers in two passes, which is how a
/// header can come twice (`deploy/media/nginx.conf`).
const SINGLE_VALUED: [&str; 12] = [
    "cache-control",
    "content-disposition",
    "content-type",
    "content-length",
    "content-range",
    "last-modified",
    "etag",
    "accept-ranges",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-max-age",
    "allow",
];

/// The headers every response under `/storage` carries, 404s and 416s
/// included (ADR 0015 §1, CORS and Headers; ADR 0013 §7, Sandbox), and no
/// single-valued header sent twice.
///
/// - `response`: the answer.
/// - `sandbox_type`: the type that decides the sandbox rule. That is the
///   response's own `Content-Type`, except for a 304, which sends none and
///   updates the cached file's headers, so the file's type decides.
pub fn common(response: &Response, sandbox_type: Option<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    for name in SINGLE_VALUED {
        let values = response.values(name);
        if values.len() > 1 {
            problems.push(format!("{name} must be sent at most once, got {values:?}"));
        }
    }
    exactly(response, "access-control-allow-origin", "*", &mut problems);
    exactly(response, "x-content-type-options", "nosniff", &mut problems);
    let exposed = response.single("access-control-expose-headers");
    if !exposed.is_some_and(|value| {
        ["Accept-Ranges", "Content-Range", "Content-Length"]
            .iter()
            .all(|token| lists(value, token))
    }) {
        problems.push(format!(
            "Access-Control-Expose-Headers must be sent once and list Accept-Ranges, \
             Content-Range and Content-Length, got {:?}",
            response.values("access-control-expose-headers")
        ));
    }
    if !response
        .values("access-control-allow-credentials")
        .is_empty()
    {
        problems
            .push("sends Access-Control-Allow-Credentials; /storage takes no credentials".into());
    }
    for server in response.values("server") {
        if server.chars().any(|c| c.is_ascii_digit()) {
            problems.push(format!("the Server header names a version: {server:?}"));
        }
    }
    if needs_sandbox(sandbox_type) {
        exactly(
            response,
            "content-security-policy",
            SANDBOX_CSP,
            &mut problems,
        );
    } else {
        absent(
            response,
            "content-security-policy",
            "a PDF is exempt from the sandbox (ADR 0013 §7)",
            &mut problems,
        );
    }
    problems
}

/// The headers of a file's own answer: `200`, `206` or `304`.
///
/// - `response`: the answer.
/// - `fixture`: the file that was asked for.
/// - `server`: the server under test; only nginx must send `ETag`.
pub fn file(response: &Response, fixture: &Fixture, server: Server) -> Vec<String> {
    let mut problems = Vec::new();
    exactly(
        response,
        "cache-control",
        cache_control(fixture.path),
        &mut problems,
    );
    if server == Server::Nginx {
        match response.single("etag") {
            Some(tag) if tag.starts_with('"') || tag.starts_with("W/\"") => {}
            _ => problems.push(format!(
                "nginx must send one ETag, got {:?}",
                response.values("etag")
            )),
        }
    }
    if response.status == StatusCode::NOT_MODIFIED {
        return problems;
    }
    match response.single("content-type") {
        Some(value) if media_type(value) == fixture.content_type => {}
        _ => problems.push(format!(
            "Content-Type must be {} from the extension, got {:?}",
            fixture.content_type,
            response.values("content-type")
        )),
    }
    exactly(response, "last-modified", LAST_MODIFIED, &mut problems);
    if needs_attachment(fixture.content_type, fixture.path) {
        let disposition = response.single("content-disposition");
        if disposition.is_none_or(|value| media_type(value) != "attachment") {
            problems.push(format!(
                "Content-Disposition must be attachment for {} (ADR 0013 §7), got {:?}",
                fixture.content_type,
                response.values("content-disposition")
            ));
        }
    } else {
        absent(
            response,
            "content-disposition",
            "raster images, PDF and /storage/sitemap.xml are shown inline (ADR 0013 §7)",
            &mut problems,
        );
    }
    if response.status == StatusCode::OK && response.single("accept-ranges") != Some("bytes") {
        problems.push(format!(
            "Accept-Ranges must be bytes, got {:?}",
            response.values("accept-ranges")
        ));
    }
    problems
}

/// A path that is not served: missing, a directory, a dot-file, a path
/// outside the storage root or one through a symbolic link.
///
/// - `response`: the answer, which must have one of the `allowed` statuses
///   and be neither a download nor cacheable.
/// - `allowed`: the refusals this kind of path may get, such as `404` alone
///   for a missing file.
pub fn refused(response: &Response, allowed: &[StatusCode]) -> Vec<String> {
    let mut problems = Vec::new();
    if !allowed.contains(&response.status) {
        let allowed: Vec<String> = allowed.iter().map(StatusCode::to_string).collect();
        problems.push(format!(
            "status must be {}, got {}",
            allowed.join(" or "),
            response.status
        ));
    }
    absent(
        response,
        "content-disposition",
        "a refusal is never a download (ADR 0013 §7)",
        &mut problems,
    );
    let cacheable = response
        .values("cache-control")
        .iter()
        .any(|value| !(lists(value, "no-store") || lists(value, "no-cache")));
    if cacheable {
        problems.push(format!(
            "a refusal must not be cacheable, or a file uploaded later stays missing; got \
             Cache-Control {:?}",
            response.values("cache-control")
        ));
    }
    problems
}

fn exactly(response: &Response, name: &str, expected: &str, problems: &mut Vec<String>) {
    let values = response.values(name);
    if values != [expected] {
        problems.push(format!(
            "{name} must be sent once as {expected:?}, got {values:?}"
        ));
    }
}

fn absent(response: &Response, name: &str, why: &str, problems: &mut Vec<String>) {
    let values = response.values(name);
    if !values.is_empty() {
        problems.push(format!("{name} must be absent: {why}; got {values:?}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_contract::fixtures::{DIAKTOLL, DOWNLOAD, ENTITY_IMAGE};
    use crate::media_contract::rules::{IMMUTABLE, ONE_HOUR};
    use axum::{
        body::Bytes,
        http::{HeaderMap, HeaderName, HeaderValue},
    };

    const EXPOSE: &str = "Accept-Ranges, Content-Range, Content-Length";

    fn response(status: StatusCode, headers: &[(&'static str, &str)]) -> Response {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.append(
                HeaderName::from_static(name),
                HeaderValue::from_str(value).expect("value"),
            );
        }
        Response {
            status,
            headers: map,
            body: Bytes::new(),
        }
    }

    fn base(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut headers = vec![
            ("access-control-allow-origin", "*"),
            ("access-control-expose-headers", EXPOSE),
            ("x-content-type-options", "nosniff"),
            ("server", "nginx"),
        ];
        headers.extend_from_slice(extra);
        headers
    }

    #[test]
    fn a_compliant_jpeg_passes() {
        let headers = base(&[
            ("content-type", "image/jpeg"),
            ("content-security-policy", SANDBOX_CSP),
            ("cache-control", IMMUTABLE),
            ("last-modified", LAST_MODIFIED),
            ("etag", "\"68b55380-12c\""),
            ("accept-ranges", "bytes"),
        ]);
        let answer = response(StatusCode::OK, &headers);
        assert_eq!(common(&answer, Some("image/jpeg")), Vec::<String>::new());
        assert_eq!(
            file(&answer, &ENTITY_IMAGE, Server::Nginx),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_duplicate_allow_origin_fails() {
        let headers = base(&[
            ("access-control-allow-origin", "*"),
            ("content-security-policy", SANDBOX_CSP),
        ]);
        let problems = common(
            &response(StatusCode::RANGE_NOT_SATISFIABLE, &headers),
            Some("text/html"),
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("access-control-allow-origin must be sent once"));
    }

    #[test]
    fn a_pdf_must_not_be_sandboxed_and_html_must() {
        let pdf = response(
            StatusCode::OK,
            &base(&[("content-security-policy", SANDBOX_CSP)]),
        );
        assert!(common(&pdf, Some("application/pdf"))[0].contains("exempt"));
        let html = response(StatusCode::NOT_FOUND, &base(&[]));
        assert!(common(&html, Some("text/html"))[0].contains("content-security-policy"));
    }

    #[test]
    fn missing_cors_nosniff_and_a_versioned_server_are_named() {
        let answer = response(
            StatusCode::OK,
            &[
                ("server", "nginx/1.26.3"),
                ("access-control-allow-credentials", "true"),
            ],
        );
        let problems = common(&answer, Some("application/pdf")).join("\n");
        for needle in [
            "access-control-allow-origin",
            "x-content-type-options",
            "Access-Control-Expose-Headers",
            "Access-Control-Allow-Credentials",
            "names a version",
        ] {
            assert!(problems.contains(needle), "{needle}: {problems}");
        }
    }

    #[test]
    fn a_download_needs_attachment_and_an_image_must_not_have_it() {
        let docx = response(
            StatusCode::PARTIAL_CONTENT,
            &[
                ("content-type", DOWNLOAD.content_type),
                ("cache-control", ONE_HOUR),
                ("last-modified", LAST_MODIFIED),
            ],
        );
        let problems = file(&docx, &DOWNLOAD, Server::Fallback);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("must be attachment"));

        let image = response(
            StatusCode::OK,
            &[
                ("content-type", "image/jpeg"),
                ("cache-control", IMMUTABLE),
                ("last-modified", LAST_MODIFIED),
                ("accept-ranges", "bytes"),
                ("content-disposition", "attachment"),
            ],
        );
        let problems = file(&image, &ENTITY_IMAGE, Server::Fallback);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("shown inline"));
    }

    #[test]
    fn etag_is_required_from_nginx_only() {
        let answer = response(StatusCode::NOT_MODIFIED, &[("cache-control", ONE_HOUR)]);
        assert_eq!(
            file(&answer, &DIAKTOLL, Server::Fallback),
            Vec::<String>::new()
        );
        assert!(file(&answer, &DIAKTOLL, Server::Nginx)[0].contains("ETag"));
    }

    #[test]
    fn wrong_type_cache_and_date_are_named() {
        let answer = response(
            StatusCode::OK,
            &[
                ("content-type", "application/octet-stream"),
                ("cache-control", ONE_HOUR),
                ("last-modified", "Thu, 01 Jan 1970 00:00:00 GMT"),
                ("accept-ranges", "bytes"),
            ],
        );
        let problems = file(&answer, &ENTITY_IMAGE, Server::Fallback).join("\n");
        for needle in [
            "Content-Type must be image/jpeg",
            "cache-control",
            "last-modified",
        ] {
            assert!(problems.contains(needle), "{needle}: {problems}");
        }
    }

    #[test]
    fn a_refusal_is_neither_a_download_nor_cached() {
        let not_found = [StatusCode::NOT_FOUND];
        assert_eq!(
            refused(&response(StatusCode::NOT_FOUND, &[]), &not_found),
            Vec::<String>::new()
        );
        let no_store = response(StatusCode::NOT_FOUND, &[("cache-control", "no-store")]);
        assert_eq!(refused(&no_store, &not_found), Vec::<String>::new());
        let problems = refused(
            &response(
                StatusCode::OK,
                &[
                    ("content-disposition", "attachment"),
                    ("cache-control", ONE_HOUR),
                ],
            ),
            &not_found,
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("status must be 404 Not Found, got 200 OK"));
    }

    #[test]
    fn a_refusal_may_have_any_of_the_allowed_statuses() {
        let either = [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND];
        for status in either {
            assert_eq!(
                refused(&response(status, &[]), &either),
                Vec::<String>::new()
            );
        }
        let problems = refused(&response(StatusCode::OK, &[]), &either);
        assert_eq!(
            problems,
            ["status must be 403 Forbidden or 404 Not Found, got 200 OK"]
        );
    }

    #[test]
    fn a_single_valued_header_sent_twice_is_named() {
        // What a 416 looks like when nginx's second header pass repeats the
        // first: each duplicate is its own problem, named after the header.
        let headers = base(&[
            ("content-security-policy", SANDBOX_CSP),
            ("cache-control", ONE_HOUR),
            ("cache-control", ONE_HOUR),
            ("content-disposition", "attachment"),
            ("content-disposition", "attachment"),
        ]);
        let problems = common(
            &response(StatusCode::RANGE_NOT_SATISFIABLE, &headers),
            Some("text/html"),
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("cache-control must be sent at most once"));
        assert!(problems[1].starts_with("content-disposition must be sent at most once"));
        for name in SINGLE_VALUED {
            let mut headers = base(&[("content-security-policy", SANDBOX_CSP)]);
            headers.extend([(name, "x"), (name, "x")]);
            let problems = common(&response(StatusCode::OK, &headers), Some("text/html"));
            assert_eq!(problems.len(), 1, "{name}: {problems:?}");
        }
    }

    #[test]
    fn a_whole_file_without_accept_ranges_is_named() {
        let answer = response(
            StatusCode::OK,
            &[
                ("content-type", "image/jpeg"),
                ("cache-control", IMMUTABLE),
                ("last-modified", LAST_MODIFIED),
            ],
        );
        let problems = file(&answer, &ENTITY_IMAGE, Server::Fallback);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("Accept-Ranges must be bytes"));
        // A 206 is itself the proof of range support, so it needs none.
        let partial = response(
            StatusCode::PARTIAL_CONTENT,
            &[
                ("content-type", "image/jpeg"),
                ("cache-control", IMMUTABLE),
                ("last-modified", LAST_MODIFIED),
            ],
        );
        assert_eq!(
            file(&partial, &ENTITY_IMAGE, Server::Fallback),
            Vec::<String>::new()
        );
    }
}
