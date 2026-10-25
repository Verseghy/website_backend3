//! The HTTP seam of the media contract suite.
//!
//! The suite talks to the server under test only through [`Transport`], with
//! the [`Origin`] passed in by its caller, so the same checks run against the
//! nginx container, the fallback router and a pod behind `oc port-forward`.
//! [`HyperTransport`] is the plain HTTP/1.1 implementation. It sends the
//! request target byte for byte: the suite must be able to send `..`
//! segments and percent-encoded names that a URL type would normalise before
//! the server ever saw them.

use std::{fmt, str::FromStr, time::Duration};

use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request as HttpRequest, StatusCode, Uri},
};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};

/// The largest response body the suite reads. The fixtures are a few
/// kilobytes; a larger body means the server sent something else.
const BODY_LIMIT: usize = 1024 * 1024;

/// The scheme, host and port of the server under test, such as
/// `http://127.0.0.1:8080`; the suite appends `/storage/...` itself.
///
/// It keeps the scheme, so that a [`Transport`] that speaks TLS can take an
/// `https` origin; [`HyperTransport`] refuses one ([`Transport::reaches`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// `http` or `https`.
    scheme: &'static str,
    /// `host:port`, with the scheme's default port filled in.
    authority: String,
}

impl Origin {
    /// The scheme: `http` or `https`.
    pub fn scheme(&self) -> &'static str {
        self.scheme
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme, self.authority)
    }
}

impl FromStr for Origin {
    type Err = String;

    /// Accepts `http://host[:port][/]` or `https://host[:port][/]` and
    /// nothing else, with a message that says what to change.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let url = url::Url::parse(text)
            .map_err(|error| format!("`{text}` is not a URL ({error}); pass http://host:port"))?;
        let scheme = match url.scheme() {
            "http" => "http",
            "https" => "https",
            other => {
                return Err(format!(
                    "`{text}` uses {other}; pass http://host:port, or https://host:port \
                     for a transport that speaks TLS"
                ));
            }
        };
        let Some(host) = url.host_str() else {
            return Err(format!("`{text}` has no host; pass http://host:port"));
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(format!(
                "`{text}` carries credentials; the media server takes none"
            ));
        }
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(format!(
                "`{text}` has a path, query or fragment; pass only http://host:port, \
                 the suite adds /storage/... itself"
            ));
        }
        // Both schemes have a known default port, so the fallback is never
        // taken; it only spares an unwrap.
        let port = url.port_or_known_default().unwrap_or(80);
        Ok(Self {
            scheme,
            authority: format!("{host}:{port}"),
        })
    }
}

/// One request of the suite.
#[derive(Clone, Debug)]
pub struct Request {
    /// The method, including ones the server must refuse.
    pub method: Method,
    /// The request target, already percent-encoded, sent as it is.
    pub target: String,
    /// Extra request headers; `Host` among them replaces the origin's.
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl Request {
    /// A request with no extra headers.
    ///
    /// - `method`: the HTTP method.
    /// - `target`: the encoded request target, starting with `/`.
    pub fn new(method: Method, target: impl Into<String>) -> Self {
        Self {
            method,
            target: target.into(),
            headers: Vec::new(),
        }
    }

    /// Adds one header.
    ///
    /// - `name`: a static header name, such as `"range"`.
    /// - `value`: its value; the suite's values are fixed ASCII strings.
    ///
    /// # Panics
    ///
    /// If `value` is not a valid header value, which is a bug in the suite.
    #[must_use]
    pub fn header(mut self, name: &'static str, value: &str) -> Self {
        let value = HeaderValue::from_str(value)
            .unwrap_or_else(|error| panic!("header {name}: {value:?}: {error}"));
        self.headers.push((HeaderName::from_static(name), value));
        self
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.target)?;
        for (name, value) in &self.headers {
            write!(f, " [{name}: {}]", value.to_str().unwrap_or("<binary>"))?;
        }
        Ok(())
    }
}

/// What the server answered.
#[derive(Clone, Debug)]
pub struct Response {
    /// The status code.
    pub status: StatusCode,
    /// Every header line, duplicates kept, so the suite can see a header
    /// that was sent twice.
    pub headers: HeaderMap,
    /// The whole body.
    pub body: Bytes,
}

impl Response {
    /// Every value of `name`, in order; a value that is not visible ASCII is
    /// shown as `<binary>`.
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.headers
            .get_all(name)
            .iter()
            .map(|value| value.to_str().unwrap_or("<binary>"))
            .collect()
    }

    /// The value of `name` if it was sent exactly once, else `None`.
    pub fn single(&self, name: &str) -> Option<&str> {
        match self.values(name).as_slice() {
            [value] => Some(value),
            _ => None,
        }
    }
}

/// Sends one request to the server under test.
pub trait Transport {
    /// Whether this transport can talk to `origin` at all, such as over its
    /// scheme. [`super::run`] asks once, before any request, so that an
    /// origin it cannot reach gives one clear failure instead of one per
    /// request. The default accepts every origin.
    ///
    /// - `origin`: where the server listens.
    ///
    /// # Errors
    ///
    /// A message that says why, and what to pass instead.
    fn reaches(&self, origin: &Origin) -> Result<(), String> {
        let _ = origin;
        Ok(())
    }

    /// Sends `request` to `origin` and reads the whole answer.
    ///
    /// - `origin`: where the server listens.
    /// - `request`: the request to send unchanged.
    ///
    /// # Errors
    ///
    /// A message that names the request, when the server cannot be reached,
    /// does not answer in time or sends a body over the suite's limit.
    fn send(
        &self,
        origin: &Origin,
        request: &Request,
    ) -> impl Future<Output = Result<Response, String>> + Send;
}

/// [`Transport`] over plain HTTP/1.1 with hyper's pooled client.
pub struct HyperTransport {
    client: Client<HttpConnector, Body>,
    timeout: Duration,
}

impl HyperTransport {
    /// A client whose every exchange, body included, must finish within
    /// `timeout`.
    pub fn new(timeout: Duration) -> Self {
        Self {
            client: Client::builder(TokioExecutor::new()).build_http(),
            timeout,
        }
    }
}

impl Transport for HyperTransport {
    /// Plain HTTP only. The suite sends each file request with the `Host` of
    /// each of the three hosts, and a router picks its Route by that header,
    /// so the suite checks the media pod directly, through
    /// `oc port-forward`, never through a Route.
    fn reaches(&self, origin: &Origin) -> Result<(), String> {
        if origin.scheme() == "http" {
            return Ok(());
        }
        Err(format!(
            "`{origin}` uses {}; this transport speaks plain HTTP only, and the suite \
             checks the media pod directly, not through a Route: run \
             `oc port-forward pod/<media pod> <port>:8080` and pass http://127.0.0.1:<port>",
            origin.scheme()
        ))
    }

    async fn send(&self, origin: &Origin, request: &Request) -> Result<Response, String> {
        self.reaches(origin)
            .map_err(|error| format!("{request}: {error}"))?;
        let uri: Uri = format!("{origin}{}", request.target)
            .parse()
            .map_err(|error| format!("{request}: not a valid request target: {error}"))?;
        let mut builder = HttpRequest::builder()
            .method(request.method.clone())
            .uri(uri);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let outgoing = builder
            .body(Body::empty())
            .map_err(|error| format!("{request}: {error}"))?;
        let exchange = async {
            let answer = self
                .client
                .request(outgoing)
                .await
                .map_err(|error| format!("{request}: no answer from {origin}: {error}"))?;
            let (parts, incoming) = answer.into_parts();
            let body = axum::body::to_bytes(Body::new(incoming), BODY_LIMIT)
                .await
                .map_err(|error| format!("{request}: reading the body: {error}"))?;
            Ok(Response {
                status: parts.status,
                headers: parts.headers,
                body,
            })
        };
        tokio::time::timeout(self.timeout, exchange)
            .await
            .map_err(|_| format!("{request}: no complete answer within {:?}", self.timeout))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_keeps_scheme_host_and_port() {
        let origin: Origin = "http://127.0.0.1:8080".parse().expect("valid");
        assert_eq!(origin.to_string(), "http://127.0.0.1:8080");
        assert_eq!(origin.scheme(), "http");
        let origin: Origin = "http://media/".parse().expect("valid");
        assert_eq!(origin.to_string(), "http://media:80");
        let origin: Origin = "https://media.example".parse().expect("valid");
        assert_eq!(origin.to_string(), "https://media.example:443");
        assert_eq!(origin.scheme(), "https");
        let origin: Origin = "http://[::1]:8080".parse().expect("valid");
        assert_eq!(origin.to_string(), "http://[::1]:8080");
    }

    #[test]
    fn origin_refuses_what_no_transport_can_use() {
        for (text, hint) in [
            ("127.0.0.1:8080", "http"),
            ("ftp://127.0.0.1:8080", "uses ftp"),
            ("http://127.0.0.1:8080/storage", "only http://host:port"),
            ("http://127.0.0.1:8080/?a=1", "only http://host:port"),
            ("http://user:secret@127.0.0.1:8080", "credentials"),
            ("not a url", "not a URL"),
        ] {
            let error = text.parse::<Origin>().expect_err(text);
            assert!(error.contains(hint), "{text}: {error}");
        }
    }

    #[tokio::test]
    async fn hyper_transport_refuses_https_with_the_port_forward_hint() {
        let transport = HyperTransport::new(Duration::from_secs(1));
        let plain: Origin = "http://127.0.0.1:8080".parse().expect("valid");
        assert_eq!(transport.reaches(&plain), Ok(()));
        let tls: Origin = "https://backend3.microshift.verseghy-gimnazium.net"
            .parse()
            .expect("valid");
        let error = transport.reaches(&tls).expect_err("https");
        assert!(error.contains("oc port-forward"), "{error}");
        // send refuses it too, before any connection is made, naming the
        // request.
        let request = Request::new(Method::GET, "/storage/a.pdf");
        let error = transport.send(&tls, &request).await.expect_err("https");
        assert!(
            error.starts_with("GET /storage/a.pdf: ") && error.contains("plain HTTP"),
            "{error}"
        );
    }

    #[test]
    fn single_tells_a_duplicate_header_from_one() {
        let mut headers = HeaderMap::new();
        headers.append("access-control-allow-origin", HeaderValue::from_static("*"));
        let mut response = Response {
            status: StatusCode::OK,
            headers,
            body: Bytes::new(),
        };
        assert_eq!(response.single("access-control-allow-origin"), Some("*"));
        response
            .headers
            .append("access-control-allow-origin", HeaderValue::from_static("*"));
        assert_eq!(response.values("access-control-allow-origin"), ["*", "*"]);
        assert_eq!(response.single("access-control-allow-origin"), None);
        assert_eq!(response.single("content-type"), None);
    }

    #[test]
    fn request_display_names_method_target_and_headers() {
        let request = Request::new(Method::GET, "/storage/a.pdf").header("range", "bytes=0-9");
        assert_eq!(request.to_string(), "GET /storage/a.pdf [range: bytes=0-9]");
    }
}
