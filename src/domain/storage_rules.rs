//! The media contract of `/storage` as pure rules, for backend3's fallback
//! router (ADR 0015 §1 and §4, with the two header rows of ADR 0013 §7).
//!
//! The fallback router (`src/controllers/storage.rs`) serves the uploads
//! volume with tower-http's `ServeDir`, which knows nothing of the contract:
//! it has no dot-file filter, follows symbolic links, answers only `GET` and
//! `HEAD`, and sets no CORS, sandbox, download or caching header. These
//! functions decide, from plain values, everything the router adds:
//!
//! - which request paths may reach the file system at all
//!   ([`parse_request_path`]);
//! - how each method is answered ([`method_answer`]);
//! - which headers each answer carries ([`response_headers`], and the
//!   constants for the headers every answer carries);
//! - whether a range answer may stand under the request's `If-Range`
//!   ([`range_answer_stands`]), which `ServeDir` ignores.
//!
//! The router itself only does the file-system work and sets what these
//! functions return. The media contract suite in `tests/media_contract/`
//! keeps its own copy of the contract tables (`rules.rs` there) and checks
//! the router over HTTP against it, so a mistake would have to be made twice
//! to pass.
//!
//! Where the fallback answers differently from nginx (`deploy/media/nginx.conf`),
//! it is only where the contract allows a choice: no `ETag`, `416` for a
//! multi-range request (ADR 0015 §1, Ranges and Caching), and these
//! readings, which the suite accepts from both servers:
//!
//! - A `.` or `..` segment is refused as it is sent, with `404`, where nginx
//!   first resolves it: ADR 0015 §1 refuses "any path segment that starts
//!   with `.`", and no link in content contains one.
//! - A `416` is the router's own short text, typed `text/plain`, with the
//!   sandbox policy. `ServeDir` would type it as the file it refused to cut
//!   (`application/pdf` for a PDF), which would exempt it from the sandbox.
//! - Only a file's own answers (`200`, `206`, `304`) carry `Cache-Control`;
//!   every refusal and error carries none, so a file uploaded later shows up
//!   (the suite's reading for refused paths).

use chrono::NaiveDateTime;
use percent_encoding::percent_decode_str;

use crate::domain::media_rules::EntityImageFolder;

/// The sandbox of ADR 0013 §7, on every response except a PDF's own.
pub const SANDBOX_CSP: &str =
    "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox";

/// ADR 0015 §1, Caching: entity images, whose names are unique per upload.
pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// ADR 0015 §1, Caching: every other file, because `uploads/` names can be
/// replaced.
pub const ONE_HOUR: &str = "public, max-age=3600";

/// ADR 0015 §1, CORS: every response, refusals included. The Diáktoll viewer
/// (pdf.js) reads its PDF cross-origin and breaks without it.
pub const ALLOW_ORIGIN: &str = "*";

/// ADR 0015 §1, CORS, as ADR 0046 amends it: the one header that a script
/// on another origin may read beyond the CORS-safelisted ones, on every
/// response.
///
/// `Accept-Ranges` is left out on purpose: pdf.js uses range requests only
/// when it can read `Accept-Ranges: bytes`, and the old site's Angular
/// service worker answers a range request with the whole file, which pdf.js
/// rejects, so a cross-origin viewer must fetch its PDF whole, as it did
/// from backend1. `Content-Length` is left out too (the Fetch Standard
/// safelists it anyway). Ranges themselves are unchanged: a client that
/// sends `Range` still gets `206` and may read `Content-Range`.
pub const EXPOSE_HEADERS: &str = "Content-Range";

/// ADR 0015 §1, Headers: every response.
pub const NOSNIFF: &str = "nosniff";

/// The methods `/storage` answers, for `Allow` on a `405` and on the
/// preflight, and for the preflight's `Access-Control-Allow-Methods`
/// (ADR 0015 §1, Methods; RFC 9110 §15.5.6 requires `Allow` on a `405`).
/// The same value as nginx's.
pub const ALLOWED_METHODS: &str = "GET, HEAD, OPTIONS";

/// The request header the preflight allows: `Range`, which a cross-origin
/// reader of a byte range sends (ADR 0015 §1, CORS; ADR 0046 keeps it).
pub const PREFLIGHT_ALLOW_HEADERS: &str = "Range";

/// How long a browser may keep the preflight answer, in seconds: 7200, the
/// most Chromium honours, as nginx sends it.
pub const PREFLIGHT_MAX_AGE: &str = "7200";

/// ADR 0013 §7, Download: the value that makes a browser download a file.
pub const ATTACHMENT: &str = "attachment";

/// The one file whose type is a download but that is shown inline: the
/// sitemap that backend3 writes at the root of the served tree (ADR 0013 §7).
pub const SITEMAP: &str = "sitemap.xml";

/// The type of the router's own `416` body, a short text.
pub const RANGE_ERROR_TYPE: &str = "text/plain; charset=utf-8";

/// The body of the router's own `416`, which replaces `ServeDir`'s.
pub const RANGE_ERROR_BODY: &str = "Range Not Satisfiable\n";

/// The media types shown inline: the four raster types and PDF
/// (ADR 0013 §7, Download).
const INLINE_TYPES: [&str; 5] = [
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "application/pdf",
];

/// The media type that is exempt from the sandbox (ADR 0013 §7, Sandbox).
const PDF: &str = "application/pdf";

/// The three forms of an HTTP-date that a recipient must accept
/// (RFC 9110 §5.6.7), as chrono formats: the IMF-fixdate that every server
/// sends, then the obsolete RFC 850 and asctime forms. chrono reads a
/// two-digit RFC 850 year 00-68 as 2000-2068 and 69-99 as 1969-1999; RFC 9110
/// reads it relative to "now", which a pure rule has not, and the two differ
/// only for years that no file on the volume has.
const HTTP_DATE_FORMATS: [&str; 3] = [
    "%a, %d %b %Y %H:%M:%S GMT",
    "%A, %d-%b-%y %H:%M:%S GMT",
    "%a %b %e %H:%M:%S %Y",
];

/// A request path below `/storage`, percent-decoded, that may be looked up
/// below the media root: its segments are plain names, none empty, none
/// starting with `.`, none holding a NUL.
///
/// The default is the storage root itself, `/storage`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoragePath {
    /// The decoded segments, outermost first; empty for `/storage` itself.
    segments: Vec<String>,
}

impl StoragePath {
    /// The decoded segments, outermost first; empty for `/storage` itself.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// The path below the media root, segments joined with `/`, such as
    /// `uploads/docs/Órarend.PDF`; empty for `/storage` itself.
    #[must_use]
    pub fn relative(&self) -> String {
        self.segments.join("/")
    }

    /// Whether the path is the storage root itself, which is a directory and
    /// never served.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }
}

/// Why a request path below `/storage` never reaches the file system. Every
/// refusal is answered `404`, the answer for a name the volume does not
/// serve (ADR 0015 §1: missing files, directories and dot-files are `404`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathRefusal {
    /// The path does not start right after `/storage`: neither empty nor
    /// beginning with `/`. The router's routes never pass one; refused so
    /// that a mistake there cannot widen what is served.
    #[error("the path is not below /storage")]
    NotBelowStorage,
    /// The percent-decoded path is not UTF-8. Every name on the volume is,
    /// and `ServeDir` refuses such a path too.
    #[error("the decoded path is not UTF-8")]
    NotUtf8,
    /// A decoded segment starts with `.`: a dot-file such as `.htaccess`, a
    /// file in a dot-directory, `.` or `..` (ADR 0015 §1, Dot-files). This
    /// is the dot-segment guard of ADR 0015 §4.
    #[error("a path segment starts with a dot")]
    DotSegment,
    /// A decoded segment holds a NUL, which no file name can.
    #[error("a path segment holds a NUL")]
    Nul,
}

/// The path below the media root that a request path names, or why it
/// names nothing the router serves.
///
/// - `below_storage`: the request path after the `/storage` prefix, still
///   percent-encoded, as the request sent it: empty or `/` for `/storage`
///   itself, `/uploads/docs/Felv%C3%A9teli.pdf` for a file.
///
/// The whole path is decoded first and then split on `/`, as nginx and
/// `ServeDir` both read it, so an encoded `%2F` separates segments and an
/// encoded `%2e%2e` is a `..` segment like a literal one. Empty segments
/// (`a//b`) are skipped, as both servers skip them. A path that passes has
/// only plain segments, so joining it to the root never climbs above it;
/// symbolic links are beyond a text rule, and the router checks them on the
/// file system.
///
/// # Errors
///
/// The first [`PathRefusal`] the path breaks: not below `/storage`, then
/// not UTF-8 once decoded, then per segment a leading dot or a NUL.
pub fn parse_request_path(below_storage: &str) -> Result<StoragePath, PathRefusal> {
    if !(below_storage.is_empty() || below_storage.starts_with('/')) {
        return Err(PathRefusal::NotBelowStorage);
    }
    let decoded = percent_decode_str(below_storage)
        .decode_utf8()
        .map_err(|_| PathRefusal::NotUtf8)?;
    let segments = decoded
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if segment.starts_with('.') {
                Err(PathRefusal::DotSegment)
            } else if segment.contains('\0') {
                Err(PathRefusal::Nul)
            } else {
                Ok(segment.to_owned())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StoragePath { segments })
}

/// How the router answers a request method (ADR 0015 §1, Methods).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodAnswer {
    /// `GET` or `HEAD`: look the file up and serve it.
    Serve,
    /// `OPTIONS`: the CORS preflight, `204` with [`ALLOWED_METHODS`],
    /// [`PREFLIGHT_ALLOW_HEADERS`] and [`PREFLIGHT_MAX_AGE`], for any path,
    /// as nginx answers it.
    Preflight,
    /// Any other method: `405` with `Allow: GET, HEAD, OPTIONS`.
    NotAllowed,
}

/// How the router answers a request with this method.
///
/// - `method`: the request method as sent. Methods are case-sensitive
///   (RFC 9110 §9.1), so `get` is not `GET`.
#[must_use]
pub fn method_answer(method: &str) -> MethodAnswer {
    match method {
        "GET" | "HEAD" => MethodAnswer::Serve,
        "OPTIONS" => MethodAnswer::Preflight,
        _ => MethodAnswer::NotAllowed,
    }
}

/// How the router treats the `Content-Type` of an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentTypeRule {
    /// Keep the one `ServeDir` set, from the file's extension.
    Keep,
    /// Replace the answer, body and type, with the router's own: the `416`,
    /// [`RANGE_ERROR_BODY`] typed [`RANGE_ERROR_TYPE`].
    RangeError,
}

/// The headers that depend on the answer, as [`response_headers`] decides
/// them. `None` means the header is not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseHeaders {
    /// `Content-Security-Policy`: [`SANDBOX_CSP`], or `None` for a PDF's own
    /// answer.
    pub content_security_policy: Option<&'static str>,
    /// `Content-Disposition`: [`ATTACHMENT`] for a download, else `None`.
    pub content_disposition: Option<&'static str>,
    /// `Cache-Control`: [`IMMUTABLE`] or [`ONE_HOUR`] on a file's own
    /// answer, else `None`.
    pub cache_control: Option<&'static str>,
    /// Whether the answer keeps its type or becomes the router's `416`.
    pub content_type: ContentTypeRule,
}

/// The headers of one answer below `/storage` that depend on the answer
/// (ADR 0015 §1, Caching; ADR 0013 §7, Sandbox and Download). The headers
/// of every answer ([`ALLOW_ORIGIN`], [`EXPOSE_HEADERS`], [`NOSNIFF`]) are
/// constants the router sets on all of them.
///
/// - `status`: the answer's status code.
/// - `response_type`: the answer's `Content-Type`, as `ServeDir` set it from
///   the file's extension, or `None` when it sends none.
/// - `file_type`: the type of the file the path names, from its extension in
///   the same way. Only a `304` uses it: a `304` sends no type, and a browser
///   merges its headers into the cached file, so the file's type decides
///   whether it may carry the sandbox (the suite's reading: a PDF's `304`
///   has no policy).
/// - `path`: the requested path below the media root.
///
/// The rules:
///
/// - A file's own answer is `200`, `206` or `304`. Only those carry
///   `Cache-Control`: [`IMMUTABLE`] for a file below `posts_images/`,
///   `authors_images/` or `colleagues_images/`, [`ONE_HOUR`] for the rest.
/// - Every answer carries [`SANDBOX_CSP`], except a file's own answer whose
///   type is `application/pdf`. A refusal or error is never the PDF, so it
///   is sandboxed whatever the path names.
/// - A `200` or `206` whose type is not one of the four raster types or PDF
///   is [`ATTACHMENT`], unless the path is the root's [`SITEMAP`]. A missing
///   type counts as a download.
/// - A `416` becomes the router's own short text ([`ContentTypeRule::RangeError`]).
#[must_use]
pub fn response_headers(
    status: u16,
    response_type: Option<&str>,
    file_type: Option<&str>,
    path: &StoragePath,
) -> ResponseHeaders {
    let file_answer = matches!(status, 200 | 206 | 304);
    let decisive_type = if status == 304 {
        file_type
    } else {
        response_type
    };
    let is_pdf = decisive_type.is_some_and(|value| media_type(value) == PDF);
    let content_security_policy = if file_answer && is_pdf {
        None
    } else {
        Some(SANDBOX_CSP)
    };
    let inline =
        response_type.is_some_and(|value| INLINE_TYPES.contains(&media_type(value).as_str()));
    let content_disposition =
        if matches!(status, 200 | 206) && !inline && path.relative() != SITEMAP {
            Some(ATTACHMENT)
        } else {
            None
        };
    let cache_control = file_answer.then(|| cache_control(path));
    let content_type = if status == 416 {
        ContentTypeRule::RangeError
    } else {
        ContentTypeRule::Keep
    };
    ResponseHeaders {
        content_security_policy,
        content_disposition,
        cache_control,
        content_type,
    }
}

/// The `Cache-Control` of a file's own answer (ADR 0015 §1, Caching):
/// [`IMMUTABLE`] below an entity image folder, [`ONE_HOUR`] elsewhere.
///
/// - `path`: the requested path below the media root.
///
/// An entity image folder is a first segment that names the disk of one of
/// [`EntityImageFolder::ALL`] (`posts_images` holds the post index images
/// too), with at least one segment below it: those names are unique per
/// upload, so they never change.
#[must_use]
pub fn cache_control(path: &StoragePath) -> &'static str {
    match path.segments() {
        [first, _, ..]
            if EntityImageFolder::ALL
                .iter()
                .any(|folder| folder.disk() == first) =>
        {
            IMMUTABLE
        }
        _ => ONE_HOUR,
    }
}

/// Whether `ServeDir`'s answer to a request may stand under the request's
/// `If-Range` (RFC 9110 §13.1.5), or the router must serve the whole file
/// instead. `ServeDir` ignores `If-Range` and cuts the range out of whatever
/// file it opened, so without this rule a client that resumes an old copy
/// of a replaced file (`uploads/` names can be replaced, ADR 0015 §1,
/// Caching) would splice bytes of the new file into it.
///
/// - `status`: the status of `ServeDir`'s answer.
/// - `if_range`: every `If-Range` value the request sent, as raw bytes.
/// - `last_modified`: the answer's `Last-Modified` value, which `ServeDir`
///   takes from the file it opened, so the check and the bytes it sends
///   describe the same file; `None` when the answer has none.
///
/// The rules:
///
/// - Only a range answer, `206` or `416`, depends on the condition; every
///   other answer stands (a `304` or `412`, which `ServeDir` decides
///   first, as RFC 9110 §13.2.2 orders the preconditions; a `200`, which
///   is already the whole file).
/// - A request without `If-Range` has no condition: the answer stands.
/// - One `If-Range` value that is an HTTP-date naming the same second as
///   `Last-Modified` holds: the file is the one the client has.
/// - Anything else fails, and the router serves the whole file: an entity
///   tag (the fallback sends none, so none can match), a date of another
///   second, earlier or later (the match is exact), a value that is no
///   HTTP-date, more than one value, or an answer without `Last-Modified`.
///   The whole file is always a correct answer; a range of the wrong file
///   never is.
#[must_use]
pub fn range_answer_stands(status: u16, if_range: &[&[u8]], last_modified: Option<&[u8]>) -> bool {
    if !matches!(status, 206 | 416) {
        return true;
    }
    match if_range {
        [] => true,
        [validator] => {
            let validator = header_date(validator);
            validator.is_some() && validator == last_modified.and_then(header_date)
        }
        _ => false,
    }
}

/// The instant an HTTP-date header value names, as [`parse_http_date`]
/// reads it; `None` when the bytes are not UTF-8 or not an HTTP-date.
///
/// - `value`: the raw header value.
fn header_date(value: &[u8]) -> Option<i64> {
    std::str::from_utf8(value).ok().and_then(parse_http_date)
}

/// The instant an HTTP-date names, in whole seconds since 1970-01-01 UTC
/// (RFC 9110 §5.6.7), or `None` when the text is not one: an entity tag, an
/// empty value, a date with a wrong weekday or a time zone other than `GMT`.
///
/// - `text`: the date as sent, in any of the three forms of
///   [`HTTP_DATE_FORMATS`], for example `Sun, 06 Nov 1994 08:49:37 GMT`.
///
/// chrono reads month and weekday names without regard to case, where
/// RFC 9110 writes them in one case; a client that sends another case gets
/// the same answer as for the date it means.
#[must_use]
pub fn parse_http_date(text: &str) -> Option<i64> {
    HTTP_DATE_FORMATS
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
        .map(|instant| instant.and_utc().timestamp())
}

/// The media type of a `Content-Type` value: the part before any parameter,
/// trimmed and lower-cased, so `text/xml; charset=utf-8` reads `text/xml`.
///
/// - `content_type`: the header value as sent.
#[must_use]
pub fn media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn path(below: &str) -> StoragePath {
        parse_request_path(below).expect("a served path")
    }

    #[test]
    fn plain_paths_decode_to_their_segments() {
        assert_eq!(
            path("/uploads/docs/Felv%C3%A9teli%20%281%29%20%5BPR%5D.pdf").segments(),
            ["uploads", "docs", "Felvételi (1) [PR].pdf"]
        );
        assert_eq!(
            path("/posts_images/index/0f1e.jpg").relative(),
            "posts_images/index/0f1e.jpg"
        );
        // A raw UTF-8 path, as a test client may send it, reads the same.
        assert_eq!(path("/uploads/kép.gif").relative(), "uploads/kép.gif");
    }

    #[test]
    fn the_storage_root_has_no_segments() {
        for below in ["", "/", "//"] {
            let root = path(below);
            assert!(root.is_root(), "{below:?}");
            assert_eq!(root.relative(), "");
        }
        assert!(!path("/uploads").is_root());
    }

    #[test]
    fn empty_segments_are_skipped_and_an_encoded_slash_separates() {
        assert_eq!(
            path("/uploads//docs/a.pdf").relative(),
            "uploads/docs/a.pdf"
        );
        assert_eq!(
            path("/uploads%2Fdocs%2fa.pdf").segments(),
            ["uploads", "docs", "a.pdf"]
        );
    }

    #[test]
    fn dot_segments_raw_or_encoded_are_refused() {
        for below in [
            "/.gitignore",
            "/uploads/.htaccess",
            "/uploads/.hidden/titkos.pdf",
            "/../../etc/passwd",
            "/%2e%2e/%2e%2e/etc/passwd",
            "/uploads/..%2f..%2f..%2fetc%2fpasswd",
            "/uploads/./docs/a.pdf",
            "/uploads/%2Ehtaccess",
            "/uploads/docs/..",
        ] {
            assert_eq!(
                parse_request_path(below),
                Err(PathRefusal::DotSegment),
                "{below}"
            );
        }
        // A dot inside or at the end of a name is an ordinary name.
        assert_eq!(
            path("/uploads/a.b./c..pdf").relative(),
            "uploads/a.b./c..pdf"
        );
    }

    #[test]
    fn nul_invalid_utf8_and_a_missing_slash_are_refused() {
        assert_eq!(
            parse_request_path("/uploads/a%00.pdf"),
            Err(PathRefusal::Nul)
        );
        assert_eq!(
            parse_request_path("/uploads/%FF.pdf"),
            Err(PathRefusal::NotUtf8)
        );
        assert_eq!(
            parse_request_path("uploads/a.pdf"),
            Err(PathRefusal::NotBelowStorage)
        );
        // The first rule broken wins: a dot-segment before a NUL.
        assert_eq!(parse_request_path("/.a/b%00"), Err(PathRefusal::DotSegment));
    }

    #[test]
    fn refusals_explain_themselves() {
        assert_eq!(
            PathRefusal::DotSegment.to_string(),
            "a path segment starts with a dot"
        );
        assert_eq!(PathRefusal::Nul.to_string(), "a path segment holds a NUL");
        assert_eq!(
            PathRefusal::NotUtf8.to_string(),
            "the decoded path is not UTF-8"
        );
        assert_eq!(
            PathRefusal::NotBelowStorage.to_string(),
            "the path is not below /storage"
        );
    }

    #[test]
    fn get_and_head_serve_options_preflights_and_the_rest_is_refused() {
        assert_eq!(method_answer("GET"), MethodAnswer::Serve);
        assert_eq!(method_answer("HEAD"), MethodAnswer::Serve);
        assert_eq!(method_answer("OPTIONS"), MethodAnswer::Preflight);
        for method in [
            "POST", "PUT", "DELETE", "PATCH", "TRACE", "CONNECT", "PROPFIND", "get", "Head",
        ] {
            assert_eq!(method_answer(method), MethodAnswer::NotAllowed, "{method}");
        }
    }

    #[test]
    fn allowed_methods_list_get_head_and_options() {
        let listed: Vec<&str> = ALLOWED_METHODS.split(", ").collect();
        assert_eq!(listed, ["GET", "HEAD", "OPTIONS"]);
        for method in listed {
            assert_ne!(method_answer(method), MethodAnswer::NotAllowed);
        }
    }

    #[test]
    fn entity_images_are_immutable_and_everything_else_one_hour() {
        for below in [
            "/posts_images/index/0f1e.jpg",
            "/posts_images/a.JPG",
            "/authors_images/a.png",
            "/colleagues_images/a.jpg",
        ] {
            assert_eq!(cache_control(&path(below)), IMMUTABLE, "{below}");
        }
        for below in [
            "/uploads/docs/a.pdf",
            "/sitemap.xml",
            "/posts_images_old/a.jpg",
            "/uploads/posts_images/a.jpg",
            "/posts_images",
            "",
        ] {
            assert_eq!(cache_control(&path(below)), ONE_HOUR, "{below}");
        }
    }

    #[test]
    fn a_jpeg_is_sandboxed_cached_and_inline() {
        let headers = response_headers(
            200,
            Some("image/jpeg"),
            Some("image/jpeg"),
            &path("/posts_images/a.jpg"),
        );
        assert_eq!(
            headers,
            ResponseHeaders {
                content_security_policy: Some(SANDBOX_CSP),
                content_disposition: None,
                cache_control: Some(IMMUTABLE),
                content_type: ContentTypeRule::Keep,
            }
        );
    }

    #[test]
    fn a_pdf_is_exempt_from_the_sandbox_only_in_its_own_answers() {
        let pdf = path("/uploads/diaktoll/ujsag.pdf");
        for status in [200, 206] {
            let headers = response_headers(status, Some(PDF), Some(PDF), &pdf);
            assert_eq!(headers.content_security_policy, None, "{status}");
            assert_eq!(headers.content_disposition, None, "{status}");
            assert_eq!(headers.cache_control, Some(ONE_HOUR), "{status}");
        }
        // A 304 sends no type: the file's type decides.
        let not_modified = response_headers(304, None, Some(PDF), &pdf);
        assert_eq!(not_modified.content_security_policy, None);
        assert_eq!(not_modified.cache_control, Some(ONE_HOUR));
        assert_eq!(not_modified.content_disposition, None);
        // A refusal or error for a PDF path is not the PDF.
        for status in [404, 405, 412, 416, 500, 204] {
            let headers = response_headers(status, Some(PDF), Some(PDF), &pdf);
            assert_eq!(
                headers.content_security_policy,
                Some(SANDBOX_CSP),
                "{status}"
            );
            assert_eq!(headers.cache_control, None, "{status}");
            assert_eq!(headers.content_disposition, None, "{status}");
        }
    }

    #[test]
    fn a_jpeg_304_keeps_the_sandbox() {
        let headers = response_headers(304, None, Some("image/jpeg"), &path("/posts_images/a.jpg"));
        assert_eq!(headers.content_security_policy, Some(SANDBOX_CSP));
        assert_eq!(headers.cache_control, Some(IMMUTABLE));
    }

    #[test]
    fn downloads_are_attachments_on_200_and_206_only() {
        let docx = path("/uploads/docs/beosztás.docx");
        let kind = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        for status in [200, 206] {
            let headers = response_headers(status, Some(kind), Some(kind), &docx);
            assert_eq!(headers.content_disposition, Some(ATTACHMENT), "{status}");
        }
        for status in [304, 404, 416] {
            let headers = response_headers(status, Some(kind), Some(kind), &docx);
            assert_eq!(headers.content_disposition, None, "{status}");
        }
        for (below, kind) in [
            ("/uploads/posts/12/rajz.svg", "image/svg+xml"),
            ("/uploads/pages/3/index.html", "text/html"),
            ("/uploads/docs/adatok.xml", "text/xml"),
            ("/uploads/docs/sitemap.xml", "text/xml"),
            ("/posts_images/f6.mp3", "audio/mpeg"),
        ] {
            let headers = response_headers(200, Some(kind), Some(kind), &path(below));
            assert_eq!(headers.content_disposition, Some(ATTACHMENT), "{below}");
            assert_eq!(
                headers.content_security_policy,
                Some(SANDBOX_CSP),
                "{below}"
            );
        }
        // No type at all is a download too.
        let untyped = response_headers(200, None, None, &path("/uploads/x"));
        assert_eq!(untyped.content_disposition, Some(ATTACHMENT));
        assert_eq!(untyped.content_security_policy, Some(SANDBOX_CSP));
    }

    #[test]
    fn only_the_root_sitemap_is_shown_inline() {
        let root = response_headers(
            200,
            Some("text/xml"),
            Some("text/xml"),
            &path("/sitemap.xml"),
        );
        assert_eq!(root.content_disposition, None);
        assert_eq!(root.content_security_policy, Some(SANDBOX_CSP));
        let nested = response_headers(
            200,
            Some("text/xml"),
            Some("text/xml"),
            &path("/uploads/sitemap.xml"),
        );
        assert_eq!(nested.content_disposition, Some(ATTACHMENT));
    }

    #[test]
    fn a_416_becomes_the_routers_own_text() {
        let headers = response_headers(416, Some(PDF), Some(PDF), &path("/uploads/a.pdf"));
        assert_eq!(headers.content_type, ContentTypeRule::RangeError);
        assert_eq!(headers.content_security_policy, Some(SANDBOX_CSP));
        assert_eq!(headers.cache_control, None);
        for status in [200, 206, 304, 404] {
            let headers = response_headers(status, Some(PDF), Some(PDF), &path("/uploads/a.pdf"));
            assert_eq!(headers.content_type, ContentTypeRule::Keep, "{status}");
        }
        assert_eq!(media_type(RANGE_ERROR_TYPE), "text/plain");
        assert!(RANGE_ERROR_BODY.is_ascii());
    }

    /// RFC 9110 §5.6.7's example instant, 1994-11-06 08:49:37 UTC.
    const RFC_EXAMPLE: i64 = 784_111_777;

    #[test]
    fn http_dates_in_all_three_forms_name_their_instant() {
        for text in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(parse_http_date(text), Some(RFC_EXAMPLE), "{text}");
        }
        // The Last-Modified that both servers send for the suite's fixtures
        // (tests/media_contract/fixtures.rs, MTIME).
        assert_eq!(
            parse_http_date("Tue, 01 Sep 2026 08:00:00 GMT"),
            Some(1_788_249_600)
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        // A two-digit RFC 850 year below 69 is in this century.
        assert_eq!(
            parse_http_date("Tuesday, 01-Sep-26 08:00:00 GMT"),
            Some(1_788_249_600)
        );
    }

    #[test]
    fn entity_tags_and_malformed_dates_are_not_http_dates() {
        for text in [
            "",
            "\"0-elavult\"",
            "W/\"0-elavult\"",
            "*",
            "Tue, 01 Sep 2026 08:00:00 UTC",
            "Tue, 01 Sep 2026 08:00:00 +0000",
            "Tue, 01 Sep 2026 08:00:00",
            "Tue, 01 Sep 2026 08:00:00 GMT trailing",
            // A weekday that the date does not fall on.
            "Mon, 01 Sep 2026 08:00:00 GMT",
            "Tue, 31 Sep 2026 08:00:00 GMT",
            "Tue, 01 Sep 2026 24:00:00 GMT",
            "1788249600",
        ] {
            assert_eq!(parse_http_date(text), None, "{text:?}");
        }
    }

    const LAST_MODIFIED: &[u8] = b"Tue, 01 Sep 2026 08:00:00 GMT";

    #[test]
    fn a_range_answer_stands_only_while_if_range_names_the_file() {
        for status in [206, 416] {
            // No condition, or the exact date in any form: the range stands.
            assert!(range_answer_stands(status, &[], Some(LAST_MODIFIED)));
            assert!(range_answer_stands(status, &[], None));
            for validator in [
                LAST_MODIFIED,
                b"Tuesday, 01-Sep-26 08:00:00 GMT".as_slice(),
                b"Tue Sep  1 08:00:00 2026".as_slice(),
            ] {
                assert!(
                    range_answer_stands(status, &[validator], Some(LAST_MODIFIED)),
                    "{status} {validator:?}"
                );
            }
            // Anything else: the whole file instead.
            for validator in [
                b"Mon, 31 Aug 2026 08:00:00 GMT".as_slice(),
                b"Tue, 01 Sep 2026 08:00:01 GMT".as_slice(),
                b"\"0-elavult\"".as_slice(),
                b"W/\"0-elavult\"".as_slice(),
                b"".as_slice(),
                b"\xFF\xFE".as_slice(),
            ] {
                assert!(
                    !range_answer_stands(status, &[validator], Some(LAST_MODIFIED)),
                    "{status} {validator:?}"
                );
            }
            assert!(!range_answer_stands(status, &[LAST_MODIFIED], None));
            assert!(!range_answer_stands(
                status,
                &[LAST_MODIFIED],
                Some(b"not a date")
            ));
            // Two values are no single validator, even if both match.
            assert!(!range_answer_stands(
                status,
                &[LAST_MODIFIED, LAST_MODIFIED],
                Some(LAST_MODIFIED)
            ));
        }
    }

    #[test]
    fn answers_that_are_not_ranges_always_stand() {
        let stale: &[&[u8]] = &[b"Mon, 31 Aug 2026 08:00:00 GMT"];
        for status in [200, 304, 404, 405, 412, 500] {
            assert!(
                range_answer_stands(status, stale, Some(LAST_MODIFIED)),
                "{status}"
            );
            assert!(range_answer_stands(status, stale, None), "{status}");
        }
    }

    #[test]
    fn media_type_drops_parameters_and_case() {
        assert_eq!(media_type("text/XML; charset=utf-8"), "text/xml");
        assert_eq!(media_type(" Application/PDF "), "application/pdf");
        assert_eq!(media_type(""), "");
    }

    #[test]
    fn the_constant_headers_are_the_contracts() {
        assert_eq!(
            SANDBOX_CSP,
            "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox"
        );
        assert_eq!(ALLOW_ORIGIN, "*");
        assert_eq!(NOSNIFF, "nosniff");
        assert_eq!(EXPOSE_HEADERS, "Content-Range");
        assert_eq!(PREFLIGHT_ALLOW_HEADERS, "Range");
        assert_eq!(PREFLIGHT_MAX_AGE.parse::<u32>(), Ok(7200));
    }

    /// ADR 0046: cross-origin scripts must not see `Accept-Ranges`, or
    /// pdf.js switches to range requests, nor `Content-Length`; a wildcard
    /// would expose both.
    #[test]
    fn cross_origin_readers_never_see_the_range_support_or_the_length() {
        let listed: Vec<&str> = EXPOSE_HEADERS.split(',').map(str::trim).collect();
        for hidden in ["Accept-Ranges", "Content-Length", "*"] {
            assert!(
                !listed.iter().any(|name| name.eq_ignore_ascii_case(hidden)),
                "{hidden} in {EXPOSE_HEADERS:?}"
            );
        }
        assert_eq!(listed, ["Content-Range"]);
    }

    /// A name that is a served segment: no `/`, no NUL, not starting with
    /// `.`, and not empty.
    fn segment() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 ()\\[\\]#?%&+éáőűÁ_~-][a-zA-Z0-9 ()\\[\\]#?%&+éáőűÁ._~-]{0,20}"
    }

    /// Percent-encodes every byte that is not unreserved, as a browser does
    /// for the names in content links.
    fn encode(text: &str) -> String {
        percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string()
    }

    /// A request path below `/storage` from already-encoded segments, each
    /// after a `/`.
    fn below<I: IntoIterator<Item = String>>(segments: I) -> String {
        let mut path = String::new();
        for segment in segments {
            path.push('/');
            path.push_str(&segment);
        }
        path
    }

    proptest! {
        #[test]
        fn encoded_segments_decode_back_to_the_stored_names(
            segments in proptest::collection::vec(segment(), 1..5),
        ) {
            let below = below(segments.iter().map(|segment| encode(segment)));
            let parsed = parse_request_path(&below).expect("served");
            prop_assert_eq!(parsed.segments(), segments.as_slice());
            prop_assert_eq!(parsed.relative(), segments.join("/"));
        }

        #[test]
        fn a_dot_segment_anywhere_is_refused_raw_or_encoded(
            before in proptest::collection::vec(segment(), 0..3),
            dotted in "\\.[a-zA-Z0-9.]{0,10}",
            after in proptest::collection::vec(segment(), 0..3),
            encoded in any::<bool>(),
        ) {
            let dot = if encoded { encode(&dotted) } else { dotted };
            let below = below(
                before
                    .iter()
                    .map(|segment| encode(segment))
                    .chain(std::iter::once(dot))
                    .chain(after.iter().map(|segment| encode(segment))),
            );
            prop_assert_eq!(parse_request_path(&below), Err(PathRefusal::DotSegment));
        }

        #[test]
        fn a_served_path_never_has_a_segment_that_could_climb_or_hide(below in "\\PC{0,40}") {
            if let Ok(parsed) = parse_request_path(&below) {
                for segment in parsed.segments() {
                    prop_assert!(!segment.is_empty());
                    prop_assert!(!segment.starts_with('.'));
                    prop_assert!(!segment.contains('/'));
                    prop_assert!(!segment.contains('\0'));
                }
            }
        }

        #[test]
        fn every_method_but_get_head_and_options_is_refused(method in "[A-Za-z-]{1,12}") {
            let expected = match method.as_str() {
                "GET" | "HEAD" => MethodAnswer::Serve,
                "OPTIONS" => MethodAnswer::Preflight,
                _ => MethodAnswer::NotAllowed,
            };
            prop_assert_eq!(method_answer(&method), expected);
        }

        #[test]
        fn only_a_pdfs_own_answer_goes_without_the_sandbox(
            status in 100u16..600,
            kind in "[a-z]{1,10}/[a-z0-9.+-]{1,20}",
            pdf in any::<bool>(),
            name in segment(),
        ) {
            let kind = if pdf { PDF.to_owned() } else { kind };
            let file = path(&format!("/uploads/{}", encode(&name)));
            let headers = response_headers(status, Some(&kind), Some(&kind), &file);
            let exempt = matches!(status, 200 | 206 | 304) && kind == PDF;
            prop_assert_eq!(headers.content_security_policy.is_none(), exempt);
        }

        #[test]
        fn only_a_files_own_answer_is_cacheable(
            status in 100u16..600,
            folder in 0usize..4,
            name in segment(),
        ) {
            let disks = ["posts_images", "authors_images", "colleagues_images", "uploads"];
            let file = path(&format!("/{}/{}", disks[folder], encode(&name)));
            let headers = response_headers(status, Some("image/png"), Some("image/png"), &file);
            let expected = match (status, folder) {
                (200 | 206 | 304, 0..=2) => Some(IMMUTABLE),
                (200 | 206 | 304, _) => Some(ONE_HOUR),
                _ => None,
            };
            prop_assert_eq!(headers.cache_control, expected);
        }

        #[test]
        fn inline_types_never_download_and_others_always_do_on_200_and_206(
            status in prop_oneof![Just(200u16), Just(206u16)],
            inline in 0usize..5,
            other in "[a-z]{1,10}/[a-z0-9.+-]{1,20}",
            parameter in proptest::option::of("[a-z]{1,8}=[a-z0-9]{1,8}"),
            pick_inline in any::<bool>(),
            name in segment(),
        ) {
            prop_assume!(!INLINE_TYPES.contains(&other.as_str()));
            let base = if pick_inline { INLINE_TYPES[inline].to_owned() } else { other };
            let kind = match parameter {
                Some(parameter) => format!("{base}; {parameter}"),
                None => base,
            };
            let file = path(&format!("/uploads/{}", encode(&name)));
            let headers = response_headers(status, Some(&kind), Some(&kind), &file);
            prop_assert_eq!(headers.content_disposition.is_some(), !pick_inline);
        }

        #[test]
        fn media_type_is_the_lower_cased_part_before_any_parameter(
            kind in "[A-Za-z]{1,12}/[A-Za-z0-9.+-]{1,24}",
            space in "[ \t]{0,3}",
            parameters in proptest::collection::vec("[a-z]{1,8}=[A-Za-z0-9-]{1,8}", 0..3),
        ) {
            let value = std::iter::once(format!("{space}{kind}{space}"))
                .chain(parameters)
                .collect::<Vec<_>>()
                .join(";");
            prop_assert_eq!(media_type(&value), kind.to_ascii_lowercase());
        }

        #[test]
        fn every_form_of_an_http_date_reads_back_its_second(
            // 1970-01-01 to 9999-12-31, the years an IMF-fixdate can write.
            seconds in 0i64..253_402_300_800,
        ) {
            let instant = chrono::DateTime::from_timestamp(seconds, 0).expect("in range");
            prop_assert_eq!(
                parse_http_date(&instant.format(HTTP_DATE_FORMATS[0]).to_string()),
                Some(seconds)
            );
            prop_assert_eq!(
                parse_http_date(&instant.format(HTTP_DATE_FORMATS[2]).to_string()),
                Some(seconds)
            );
            // RFC 850's two-digit year reads back only inside chrono's
            // 1969-2068 window.
            if (1969..=2068).contains(&chrono::Datelike::year(&instant)) {
                prop_assert_eq!(
                    parse_http_date(&instant.format(HTTP_DATE_FORMATS[1]).to_string()),
                    Some(seconds)
                );
            }
        }

        #[test]
        fn any_text_parses_or_is_refused_without_panicking(text in "\\PC{0,40}") {
            if let Some(seconds) = parse_http_date(&text) {
                // Whatever parses names a real instant.
                prop_assert!(chrono::DateTime::from_timestamp(seconds, 0).is_some());
            }
        }

        #[test]
        fn a_range_answer_stands_exactly_when_the_dates_name_one_second(
            status in prop_oneof![Just(206u16), Just(416u16)],
            file in 0i64..4_102_444_800,
            offset in -100_000i64..100_000,
        ) {
            let format = |seconds: i64| {
                chrono::DateTime::from_timestamp(seconds, 0)
                    .expect("in range")
                    .format(HTTP_DATE_FORMATS[0])
                    .to_string()
            };
            let last_modified = format(file);
            let validator = format((file + offset).max(0));
            prop_assert_eq!(
                range_answer_stands(
                    status,
                    &[validator.as_bytes()],
                    Some(last_modified.as_bytes())
                ),
                validator == last_modified
            );
        }

        #[test]
        fn other_answers_stand_whatever_the_request_sent(
            status in (100u16..600).prop_filter("not a range answer", |status| {
                !matches!(status, 206 | 416)
            }),
            validators in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..32),
                0..3
            ),
            last_modified in proptest::option::of(proptest::collection::vec(any::<u8>(), 0..32)),
        ) {
            let validators: Vec<&[u8]> = validators.iter().map(Vec::as_slice).collect();
            prop_assert!(range_answer_stands(status, &validators, last_modified.as_deref()));
        }

        #[test]
        fn cache_control_reads_only_the_first_segment(
            folder in 0usize..3,
            rest in proptest::collection::vec(segment(), 1..4),
        ) {
            let disks = ["posts_images", "authors_images", "colleagues_images"];
            let below = below(
                std::iter::once(disks[folder].to_owned())
                    .chain(rest.iter().map(|segment| encode(segment))),
            );
            prop_assert_eq!(cache_control(&path(&below)), IMMUTABLE);
            let elsewhere = format!("/uploads{below}");
            prop_assert_eq!(cache_control(&path(&elsewhere)), ONE_HOUR);
        }
    }
}
