//! The media server's second listener: nginx's `stub_status` for the media
//! exporter, on the pod's loopback address only (`deploy/media/nginx.conf`;
//! ADR 0023 §9; research: DECISIONS R9, D34, and R15, D93).
//!
//! The exporter is another container of the media pod, so it shares the
//! pod's network namespace and reads `http://127.0.0.1:8081/stub_status`.
//! [`check`] reads it the same way: it runs `curl`, which the pinned image
//! ships, inside the running container with the engine's `exec`. It checks:
//!
//! - what the exporter reads: `200`, `text/plain`, and a body in the layout
//!   that the exporter scans ([`StubStatus::parse`]);
//! - that the listener serves nothing else: no media file, no probe;
//! - that nginx listens on exactly `0.0.0.0:8080` and `127.0.0.1:8081`,
//!   read from the kernel's socket table of the container's network
//!   namespace ([`listening`]). A socket bound to `127.0.0.1` takes only
//!   connections addressed to `127.0.0.1`, which only what runs in the pod's
//!   network namespace can open: the pod's containers, and an
//!   `oc port-forward` to the pod, which the container runtime connects to
//!   `localhost` there. A Service, a Route or another pod connects to the
//!   pod's address. This reading, not a log line or a comment, is what shows
//!   that they cannot reach the listener;
//! - the same from outside: a connection through the published port, which
//!   docker and podman's pasta (by default) forward to the container's own
//!   address as a Service does to the pod's, gets no HTTP answer at all
//!   ([`published_answer`] says what an answer means);
//! - that the media port gives no request the page: a request for it with
//!   no `Host` at all, and, in `outside_storage`, the common status paths
//!   with loopback and public `Host`s ([`public_port_requests`]);
//! - and, with [`logged_reads`], that the reads leave no access-log line
//!   (`access_log off`), so the log keeps one line per media request.
//!
//! `super::config` reads the file itself and covers every other request. The
//! pure parts (the three parsers, the judgement of a read and the messages)
//! have unit tests and proptests below; the container test runs the rest.

use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    process::Output,
    time::Duration,
};

use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use super::{Container, MEDIA_PORT, UID, text};
use crate::media_contract::{
    HOSTS, HyperTransport, Transport,
    fixtures::DIAKTOLL,
    http::{Request, Response},
    rules::storage_target,
};

/// The port of the loopback listener in `deploy/media/nginx.conf`, the one
/// that `website_k8s` gives the exporter in its scrape URI
/// (`deploy/media/README.md`).
pub(super) const PORT: u16 = 8081;

/// The path the loopback listener serves `stub_status` at.
pub(super) const PATH: &str = "/stub_status";

/// Every socket that nginx may listen on: the media server on every IPv4
/// address, and `stub_status` on the loopback address only.
const LISTENERS: [SocketAddr; 2] = [
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MEDIA_PORT)),
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, PORT)),
];

/// The `User-Agent` of every read that [`check`] sends from inside the
/// container, so that [`logged_reads`] finds these reads in the access log
/// and nothing else.
const AGENT: &str = "media-contract-stub-status";

/// How long one read inside the container may take, in seconds.
const READ_TIMEOUT_SECONDS: &str = "5";

/// The kernel's state number for a listening TCP socket, as the fourth
/// column of `/proc/net/tcp` writes it (`TCP_LISTEN`, 10).
const TCP_LISTEN: &str = "0A";

/// Where servers commonly serve the page, and other spellings of [`PATH`]:
/// none of them may get it on the media port.
const STATUS_PATHS: [&str; 6] = [
    PATH,
    "/stub_status/",
    "/nginx_status",
    "/status",
    "/basic_status",
    "/storage/stub_status",
];

/// How long the request without `Host` ([`without_host`]) may take.
const RAW_TIMEOUT: Duration = Duration::from_secs(10);

/// The most bytes [`without_host`] reads; nginx's 404 page is a few hundred.
const RAW_LIMIT: u64 = 64 * 1024;

/// Checks the loopback listener of the running `container`, and returns one
/// line per failed expectation.
///
/// - `container`: the media server, started by [`Container::start`] with
///   [`MEDIA_PORT`] and [`PORT`] published.
/// - `transport`: the client of the suite, for the attempt through the
///   published [`PORT`].
pub(super) async fn check(container: &Container, transport: &HyperTransport) -> Vec<String> {
    let mut failures = Vec::new();
    match read(container, PATH) {
        Ok(response) => failures.extend(scrape_problems(&response)),
        Err(error) => failures.push(error),
    }
    // The listener has one location: no media file, no probe and no other
    // spelling of the path.
    let media = storage_target(DIAKTOLL.path);
    for target in ["/", "/healthz", "/stub_status/", media.as_str()] {
        match read(container, target) {
            Ok(response) if response.status == StatusCode::NOT_FOUND => {}
            Ok(response) => failures.push(format!(
                "GET {target} on 127.0.0.1:{PORT} inside the container: status must be 404, \
                 got {}",
                response.status
            )),
            Err(error) => failures.push(error),
        }
    }
    let listeners = listener_problems(container);
    let listeners_as_built = listeners.is_empty();
    failures.extend(listeners);
    failures.extend(without_host(&container.published_address(MEDIA_PORT)).await);
    // Any answer, even a 404, means that the listener took the connection.
    // The engine's forwarder may accept on the host side and then close, or
    // reset: either is an error here, never a response.
    let origin = container.published(PORT);
    let request = Request::new(Method::GET, PATH);
    if let Ok(response) = transport.send(&origin, &request).await {
        failures.push(published_answer(
            &request,
            response.status,
            listeners_as_built,
        ));
    }
    failures
}

/// The failure for an answer through the published [`PORT`], through which
/// no answer may come.
///
/// - `request`: what was sent, for the message.
/// - `status`: the status of the answer.
/// - `listeners_as_built`: whether the socket table showed nginx on exactly
///   [`LISTENERS`]. Then only the loopback socket could have answered, so
///   the engine handed the connection to the container's loopback instead
///   of its address. The probe then cannot stand for a Service, and the
///   message says so instead of blaming the configuration.
fn published_answer(request: &Request, status: StatusCode, listeners_as_built: bool) -> String {
    if listeners_as_built {
        format!(
            "{request} through the published port {PORT}: answered {status}, although the \
             socket table shows nginx on 127.0.0.1:{PORT} only. So the container engine handed \
             the connection to the container's loopback (pasta does with --host-lo-to-ns-lo), \
             where a Service connects to the pod's address: this probe cannot stand for a \
             Service here. Run the test with docker, or with pasta's default forwarding"
        )
    } else {
        format!(
            "{request} through the published port {PORT}: answered {status}, but the listener \
             must take no connection addressed to the container: it must listen on \
             127.0.0.1:{PORT} only (see the socket-table failure)"
        )
    }
}

/// The requests on the media port that must not get the page: each of
/// [`STATUS_PATHS`] with the transport's own `Host` (the published address)
/// and with each `Host` that names the loopback listener, the loopback or
/// one of the public hosts. nginx picks the server by the port first, so
/// none of them may reach the loopback listener. `super::config` covers the
/// paths and `Host`s that are not listed.
pub(super) fn public_port_requests() -> Vec<Request> {
    let hosts: Vec<String> = [
        "localhost".to_owned(),
        format!("localhost:{PORT}"),
        "127.0.0.1".to_owned(),
        format!("127.0.0.1:{PORT}"),
    ]
    .into_iter()
    .chain(HOSTS.iter().map(|&host| host.to_owned()))
    .collect();
    STATUS_PATHS
        .iter()
        .flat_map(|&path| {
            std::iter::once(Request::new(Method::GET, path)).chain(
                hosts
                    .iter()
                    .map(move |host| Request::new(Method::GET, path).header("host", host)),
            )
        })
        .collect()
}

/// Whether `body` is a `stub_status` page ([`StubStatus::parse`]), whatever
/// status it came with.
///
/// - `body`: a response body.
pub(super) fn is_page(body: &[u8]) -> bool {
    std::str::from_utf8(body).is_ok_and(|page| StubStatus::parse(page).is_ok())
}

/// `GET /stub_status` in HTTP/1.0 without a `Host`, on the media port at
/// `address`: it must get `404`, and not the page. nginx gives a request
/// without `Host` to the default server of the port it arrived on, a choice
/// of server that no `Host` value makes. hyper's client always sends a
/// `Host`, so this writes the request on a TCP connection of its own.
///
/// - `address`: the published media port, such as `127.0.0.1:40123`.
async fn without_host(address: &str) -> Vec<String> {
    let request = format!("GET {PATH} HTTP/1.0 without Host on {address}");
    let exchange = async {
        let mut stream = TcpStream::connect(address)
            .await
            .map_err(|error| format!("connect: {error}"))?;
        stream
            .write_all(format!("GET {PATH} HTTP/1.0\r\n\r\n").as_bytes())
            .await
            .map_err(|error| format!("write: {error}"))?;
        // nginx closes the connection after an HTTP/1.0 answer.
        let mut printed = Vec::new();
        stream
            .take(RAW_LIMIT)
            .read_to_end(&mut printed)
            .await
            .map_err(|error| format!("read: {error}"))?;
        parse_reply(&printed)
    };
    match tokio::time::timeout(RAW_TIMEOUT, exchange).await {
        Ok(Ok(response)) => public_answer_problems(&request, &response),
        Ok(Err(error)) => vec![format!("{request}: {error}")],
        Err(_) => vec![format!(
            "{request}: no complete answer within {RAW_TIMEOUT:?}"
        )],
    }
}

/// What is wrong with `response`, the media port's answer to a request for
/// the page: anything but a `404` without the page.
///
/// - `request`: what was sent, for the messages.
/// - `response`: the answer.
fn public_answer_problems(request: &str, response: &Response) -> Vec<String> {
    let mut problems = Vec::new();
    if response.status != StatusCode::NOT_FOUND {
        problems.push(format!(
            "{request}: status must be 404, got {}",
            response.status
        ));
    }
    if is_page(&response.body) {
        problems.push(format!(
            "{request}: the media port {MEDIA_PORT} served the stub_status page, which only \
             the loopback listener may serve"
        ));
    }
    problems
}

/// The access-log lines on `stdout` that record a read of [`check`]: there
/// must be none, because the loopback listener logs no access.
///
/// - `stdout`: the container's standard output, where the access log goes.
pub(super) fn logged_reads(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|line| line.contains(AGENT))
        .map(|line| format!("the loopback listener logged a read (access_log off): {line}"))
        .collect()
}

/// `GET <target>` on the loopback listener from inside `container`, through
/// the image's `curl`, as the exporter reads it.
///
/// - `container`: the running media server.
/// - `target`: the request target, already percent-encoded.
fn read(container: &Container, target: &str) -> Result<Response, String> {
    let url = format!("http://127.0.0.1:{PORT}{target}");
    let output = container.exec(&[
        "curl",
        "--silent",
        "--show-error",
        // The status line and the headers before the body, for
        // `parse_reply`.
        "--include",
        "--max-time",
        READ_TIMEOUT_SECONDS,
        "--user-agent",
        AGENT,
        &url,
    ]);
    reply(&url, &output)
}

/// The response in the output of one `curl --include` run, or why there is
/// none.
///
/// - `url`: what was read, for the message.
/// - `output`: the finished `curl` run.
fn reply(url: &str, output: &Output) -> Result<Response, String> {
    if !output.status.success() {
        return Err(format!(
            "GET {url} inside the container failed ({}): {}",
            output.status,
            text(output).trim()
        ));
    }
    parse_reply(&output.stdout).map_err(|error| format!("GET {url} inside the container: {error}"))
}

/// Parses what `curl --include` prints for one HTTP/1.x exchange: the
/// status line, the header lines, an empty line, and the body.
///
/// - `printed`: curl's standard output, every byte of it.
///
/// # Errors
///
/// A message that says which part is missing or malformed.
fn parse_reply(printed: &[u8]) -> Result<Response, String> {
    const END_OF_HEAD: &[u8] = b"\r\n\r\n";
    let Some(at) = printed
        .windows(END_OF_HEAD.len())
        .position(|window| window == END_OF_HEAD)
    else {
        return Err("no empty line after the response headers".to_owned());
    };
    let head = std::str::from_utf8(&printed[..at])
        .map_err(|_| "the response head is not text".to_owned())?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = match status_line.split(' ').collect::<Vec<_>>().as_slice() {
        [version, code, ..] if version.starts_with("HTTP/1.") => code
            .parse::<u16>()
            .ok()
            .and_then(|code| StatusCode::from_u16(code).ok()),
        _ => None,
    }
    .ok_or_else(|| format!("not an HTTP/1.x status line: {status_line:?}"))?;
    let mut headers = HeaderMap::new();
    for line in lines {
        let parsed = line.split_once(':').and_then(|(name, value)| {
            Some((
                HeaderName::from_bytes(name.as_bytes()).ok()?,
                HeaderValue::from_str(value.trim()).ok()?,
            ))
        });
        let Some((name, value)) = parsed else {
            return Err(format!("not a header line: {line:?}"));
        };
        headers.append(name, value);
    }
    Ok(Response {
        status,
        headers,
        body: Bytes::copy_from_slice(&printed[at + END_OF_HEAD.len()..]),
    })
}

/// What is wrong with `response` as the exporter's read of `stub_status`.
///
/// - `response`: the answer to `GET /stub_status` on the loopback listener.
fn scrape_problems(response: &Response) -> Vec<String> {
    let request = format!("GET {PATH} on 127.0.0.1:{PORT} inside the container");
    if response.status != StatusCode::OK {
        return vec![format!(
            "{request}: status must be 200, got {}",
            response.status
        )];
    }
    let mut problems = Vec::new();
    if response.single("content-type") != Some("text/plain") {
        problems.push(format!(
            "{request}: Content-Type must be sent once as \"text/plain\", got {:?}",
            response.values("content-type")
        ));
    }
    match std::str::from_utf8(&response.body)
        .map_err(|_| "the body is not text".to_owned())
        .and_then(StubStatus::parse)
    {
        // This read is itself an open connection and a request, so a real
        // server never shows zero for either.
        Ok(status) if status.active == 0 || status.requests == 0 => problems.push(format!(
            "{request}: counts a read without its own connection or request: {status:?}"
        )),
        Ok(_) => {}
        Err(error) => problems.push(format!(
            "{request}: the body is not what the exporter reads: {error}"
        )),
    }
    problems
}

/// The counters of one `stub_status` page (nginx's
/// `ngx_http_stub_status_module`), as the exporter reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StubStatus {
    /// Open client connections, waiting ones included.
    active: u64,
    /// Connections accepted since start.
    accepted: u64,
    /// Connections handled since start.
    handled: u64,
    /// Requests since start.
    requests: u64,
    /// Connections whose request header nginx is reading.
    reading: u64,
    /// Connections that nginx is answering.
    writing: u64,
    /// Idle keep-alive connections.
    waiting: u64,
}

impl StubStatus {
    /// Parses the page, which nginx 1.26 writes as four lines:
    ///
    /// ```text
    /// Active connections: 1
    /// server accepts handled requests
    ///  1 1 1
    /// Reading: 0 Writing: 1 Waiting: 0
    /// ```
    ///
    /// Each line may carry spaces around its words (nginx ends three of the
    /// lines with one), and every line ends with a newline. The NGINX
    /// Prometheus Exporter scans this layout with one fixed template
    /// (nginx-prometheus-exporter@4821830:client/nginx.go, `templateMetrics`).
    /// This parser is stricter than that template on two points that nginx
    /// 1.26 always meets: the template also takes a page without the final
    /// newline, and ignores lines after the fourth. So it never accepts a
    /// page that the exporter cannot read.
    ///
    /// - `page`: the whole response body.
    ///
    /// # Errors
    ///
    /// A message that names the line that differs from the layout.
    fn parse(page: &str) -> Result<Self, String> {
        let Some(page) = page.strip_suffix('\n') else {
            return Err("the page does not end with a newline".to_owned());
        };
        let lines: Vec<Vec<&str>> = page
            .split('\n')
            .map(|line| line.split_whitespace().collect())
            .collect();
        // `index` counts the lines from 0; the message counts them from 1.
        let number = |index: usize, word: &&str| {
            word.parse::<u64>()
                .map_err(|_| format!("line {}: {word:?} is not a counter", index + 1))
        };
        match lines.as_slice() {
            [active, totals_header, totals, states] => {
                let [label_active, label_connections, value_active] = active.as_slice() else {
                    return Err(format!(
                        "line 1: {active:?} is not \"Active connections: N\""
                    ));
                };
                if (*label_active, *label_connections) != ("Active", "connections:") {
                    return Err(format!(
                        "line 1: {active:?} is not \"Active connections: N\""
                    ));
                }
                if totals_header.as_slice() != ["server", "accepts", "handled", "requests"] {
                    return Err(format!(
                        "line 2: {totals_header:?} is not \"server accepts handled requests\""
                    ));
                }
                let [accepted, handled, requests] = totals.as_slice() else {
                    return Err(format!("line 3: {totals:?} is not three counters"));
                };
                let [
                    "Reading:",
                    in_reading,
                    "Writing:",
                    in_writing,
                    "Waiting:",
                    idle,
                ] = states.as_slice()
                else {
                    return Err(format!(
                        "line 4: {states:?} is not \"Reading: N Writing: N Waiting: N\""
                    ));
                };
                Ok(Self {
                    active: number(0, value_active)?,
                    accepted: number(2, accepted)?,
                    handled: number(2, handled)?,
                    requests: number(2, requests)?,
                    reading: number(3, in_reading)?,
                    writing: number(3, in_writing)?,
                    waiting: number(3, idle)?,
                })
            }
            other => Err(format!("{} lines, not 4", other.len())),
        }
    }
}

/// Reads the kernel's socket table of the container's network namespace and
/// compares the sockets that nginx listens on with [`LISTENERS`].
///
/// - `container`: the running media server.
fn listener_problems(container: &Container) -> Vec<String> {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let output = container.exec(&["cat", table]);
        let read = if output.status.success() {
            String::from_utf8(output.stdout)
                .map_err(|_| "not text".to_owned())
                .and_then(|text| listening(&text, UID))
        } else {
            Err(text(&output).trim().to_owned())
        };
        match read {
            Ok(sockets) => found.extend(sockets),
            Err(error) => problems.push(format!("{table} inside the container: {error}")),
        }
    }
    if problems.is_empty() {
        found.sort_unstable();
        let mut expected = LISTENERS.to_vec();
        expected.sort_unstable();
        if found != expected {
            problems.push(format!(
                "nginx (UID {UID}) must listen on exactly {expected:?}, so that stub_status \
                 is reachable only from the pod's network namespace; it listens on {found:?}"
            ));
        }
    }
    problems
}

/// The local addresses of the listening sockets that `uid` owns, in one of
/// the kernel's socket tables (`/proc/net/tcp` or `/proc/net/tcp6`).
///
/// Each line after the header describes one socket by whitespace-separated
/// fields: the slot, the local and the remote `address:port`, the state in
/// hex, the two queue sizes, the timer, the retransmit count, and the
/// owner's UID (`proc(5)`, "/proc/net/tcp"). Sockets in any other state, such
/// as the `TIME_WAIT` remains of finished reads, and sockets of other owners,
/// such as an engine's helper, are skipped.
///
/// - `table`: the whole file.
/// - `uid`: the owner whose listeners count, as the namespace that reads the
///   file sees it.
///
/// # Errors
///
/// A message that names the line that is not in the layout above, so that a
/// change of the layout fails the check instead of passing it empty.
fn listening(table: &str, uid: u32) -> Result<Vec<SocketAddr>, String> {
    let mut lines = table.lines();
    match lines
        .next()
        .map(str::split_whitespace)
        .and_then(|mut words| words.next())
    {
        Some("sl") => {}
        _ => return Err("the first line is not the header that starts with \"sl\"".to_owned()),
    }
    let mut sockets = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [
            _slot,
            local,
            _remote,
            state,
            _queues,
            _timer,
            _retransmits,
            owner,
            ..,
        ] = fields.as_slice()
        else {
            return Err(format!("not a socket line: {line:?}"));
        };
        let owner: u32 = owner
            .parse()
            .map_err(|_| format!("{owner:?} is not a UID: {line:?}"))?;
        let local = socket_address(local).map_err(|error| format!("{error}: {line:?}"))?;
        if *state == TCP_LISTEN && owner == uid {
            sockets.push(local);
        }
    }
    Ok(sockets)
}

/// Decodes one `address:port` field of a kernel socket table.
///
/// The kernel prints the address as the 32-bit words of its network-order
/// bytes, each read in the host's byte order and written as 8 hex digits:
/// one word for IPv4, four for IPv6. So `127.0.0.1` reads `0100007F` on a
/// little-endian host. The port is the number itself, in 4 hex digits.
///
/// - `field`: for example `0100007F:1F91`.
///
/// # Errors
///
/// A message that says what is malformed.
fn socket_address(field: &str) -> Result<SocketAddr, String> {
    let malformed = || format!("{field:?} is not a hex address:port");
    let (address, port) = field.split_once(':').ok_or_else(malformed)?;
    // Digits only: `from_str_radix` would also take a leading `+`.
    let hex = |digits: &str| digits.bytes().all(|byte| byte.is_ascii_hexdigit());
    if port.len() != 4 || address.len() % 8 != 0 || !hex(port) || !hex(address) {
        return Err(malformed());
    }
    let port = u16::from_str_radix(port, 16).map_err(|_| malformed())?;
    let mut bytes = Vec::with_capacity(16);
    for at in (0..address.len()).step_by(8) {
        let word = address
            .get(at..at + 8)
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .ok_or_else(malformed)?;
        bytes.extend(word.to_ne_bytes());
    }
    if let Ok(octets) = <[u8; 4]>::try_from(bytes.as_slice()) {
        Ok(SocketAddr::from((Ipv4Addr::from(octets), port)))
    } else if let Ok(octets) = <[u8; 16]>::try_from(bytes.as_slice()) {
        Ok(SocketAddr::from((Ipv6Addr::from(octets), port)))
    } else {
        Err(malformed())
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use proptest::prelude::*;

    use super::*;

    /// The header line of both socket tables, as Linux writes it.
    const HEADER: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when \
                          retrnsmt   uid  timeout inode";

    /// A socket line in the kernel's layout, the way `/proc/net/tcp` writes
    /// it on this host.
    fn socket_line(local: SocketAddr, state: &str, uid: u32) -> String {
        let words: Vec<[u8; 4]> = match local {
            SocketAddr::V4(address) => vec![address.ip().octets()],
            SocketAddr::V6(address) => address.ip().octets().as_chunks::<4>().0.to_vec(),
        };
        let mut address = String::new();
        for word in words {
            write!(address, "{:08X}", u32::from_ne_bytes(word)).expect("a String takes any text");
        }
        format!(
            "   0: {address}:{:04X} 00000000:0000 {state} 00000000:00000000 00:00000000 \
             00000000 {uid:>5}        0 8734648 1 000000003ba368a4 100 0 0 10 0",
            local.port()
        )
    }

    /// A read's answer with `status`, at most one `Content-Type` and `body`.
    fn response(status: StatusCode, content_type: Option<&str>, body: &str) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(value) = content_type {
            headers.insert(
                "content-type",
                HeaderValue::from_str(value).expect("a header value"),
            );
        }
        Response {
            status,
            headers,
            body: Bytes::copy_from_slice(body.as_bytes()),
        }
    }

    /// The page that nginx 1.26.3 of the pinned image wrote for one read,
    /// byte for byte.
    const PAGE: &str = "Active connections: 1 \nserver accepts handled requests\n 1 1 1 \n\
                        Reading: 0 Writing: 1 Waiting: 0 \n";

    #[test]
    fn the_page_of_the_pinned_image_parses() {
        assert_eq!(
            StubStatus::parse(PAGE),
            Ok(StubStatus {
                active: 1,
                accepted: 1,
                handled: 1,
                requests: 1,
                reading: 0,
                writing: 1,
                waiting: 0,
            })
        );
    }

    #[test]
    fn a_page_in_another_layout_is_refused_with_its_line() {
        for (page, message) in [
            ("", "does not end with a newline"),
            (PAGE.trim_end(), "does not end with a newline"),
            ("Active connections: 1 \n", "1 lines, not 4"),
            (&format!("{PAGE}extra\n"), "5 lines, not 4"),
            (&PAGE.replace("Active", "Open"), "line 1:"),
            (
                &PAGE.replace("connections: 1", "connections: x"),
                "line 1: \"x\" is not a counter",
            ),
            (&PAGE.replace("handled", "dropped"), "line 2:"),
            (&PAGE.replace(" 1 1 1 ", " 1 1 "), "line 3:"),
            (
                &PAGE.replace(" 1 1 1 ", " 1 -1 1 "),
                "line 3: \"-1\" is not a counter",
            ),
            (&PAGE.replace("Writing", "Sending"), "line 4:"),
            (
                &PAGE.replace("Waiting: 0", "Waiting: 0 Closing: 0"),
                "line 4:",
            ),
            (
                &PAGE.replace("Reading: 0", "Reading: 0.5"),
                "line 4: \"0.5\" is not a counter",
            ),
        ] {
            let error = StubStatus::parse(page).expect_err(page);
            assert!(error.contains(message), "{page:?}: {error}");
        }
    }

    #[test]
    fn a_read_of_the_pinned_image_passes() {
        let read = response(StatusCode::OK, Some("text/plain"), PAGE);
        assert_eq!(scrape_problems(&read), Vec::<String>::new());
    }

    #[test]
    fn a_read_that_the_exporter_cannot_use_fails() {
        let missing = response(StatusCode::NOT_FOUND, Some("text/html"), "<html>");
        let problems = scrape_problems(&missing);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("status must be 200, got 404"),
            "{problems:?}"
        );

        let html = response(StatusCode::OK, Some("text/html"), PAGE);
        assert!(scrape_problems(&html)[0].contains("Content-Type must be sent once"));
        let untyped = response(StatusCode::OK, None, PAGE);
        assert!(scrape_problems(&untyped)[0].contains("Content-Type must be sent once"));

        let garbled = response(StatusCode::OK, Some("text/plain"), "ok\n");
        assert!(scrape_problems(&garbled)[0].contains("not what the exporter reads"));

        let idle = response(
            StatusCode::OK,
            Some("text/plain"),
            &PAGE.replace("connections: 1", "connections: 0"),
        );
        assert!(scrape_problems(&idle)[0].contains("without its own connection"));
    }

    #[test]
    fn curl_output_parses_into_status_headers_and_body() {
        let printed = format!(
            "HTTP/1.1 200 OK\r\nServer: nginx\r\nContent-Type: text/plain\r\n\
             Content-Length: 97\r\nConnection: keep-alive\r\n\r\n{PAGE}"
        );
        let reply = parse_reply(printed.as_bytes()).expect("a reply");
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.single("content-type"), Some("text/plain"));
        assert_eq!(reply.single("content-length"), Some("97"));
        assert_eq!(reply.body.as_ref(), PAGE.as_bytes());

        let not_found =
            parse_reply(b"HTTP/1.1 404 Not Found\r\nServer: nginx\r\n\r\n<html>").expect("a reply");
        assert_eq!(not_found.status, StatusCode::NOT_FOUND);
        assert_eq!(not_found.body.as_ref(), b"<html>");
    }

    #[test]
    fn curl_output_that_is_no_http_reply_is_refused() {
        for (printed, message) in [
            (b"".as_slice(), "no empty line"),
            (b"HTTP/1.1 200 OK\r\nServer: nginx\r\n", "no empty line"),
            (b"HTTP/2 200\r\n\r\n", "not an HTTP/1.x status line"),
            (b"HTTP/1.1 OK\r\n\r\n", "not an HTTP/1.x status line"),
            (b"HTTP/1.1 99 Odd\r\n\r\n", "not an HTTP/1.x status line"),
            (b"HTTP/1.1 200 OK\r\nno colon\r\n\r\n", "not a header line"),
            (
                b"HTTP/1.1 200 OK\r\nBad Name: x\r\n\r\n",
                "not a header line",
            ),
            (b"HTTP/1.1 200 OK\r\nX: \xff\r\n\r\n", "not text"),
        ] {
            let error = parse_reply(printed).expect_err("refused");
            assert!(error.contains(message), "{printed:?}: {error}");
        }
    }

    #[test]
    fn only_the_reads_of_the_check_count_as_logged_reads() {
        let stdout = format!(
            "2026/10/31 22:34:22 [notice] 1#1: start worker processes\n\
             10.0.1.0 \"-\" [2026-10-31T22:34:24+00:00] \"GET /stub_status HTTP/1.1\" 404 146 \
             0.000 \"-\" \"hyper\"\n\
             127.0.0.1 \"-\" [2026-10-31T22:34:24+00:00] \"GET /stub_status HTTP/1.1\" 200 97 \
             0.000 \"-\" \"{AGENT}\"\n"
        );
        let logged = logged_reads(&stdout);
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert!(logged[0].contains("127.0.0.1"), "{logged:?}");
        assert_eq!(logged_reads(""), Vec::<String>::new());
    }

    #[test]
    fn an_answer_through_the_published_port_names_its_cause() {
        let request = Request::new(Method::GET, PATH);
        let engine = published_answer(&request, StatusCode::OK, true);
        assert!(engine.contains("answered 200 OK"), "{engine}");
        assert!(engine.contains("--host-lo-to-ns-lo"), "{engine}");
        assert!(engine.contains("cannot stand for a Service"), "{engine}");
        let config = published_answer(&request, StatusCode::NOT_FOUND, false);
        assert!(config.contains("answered 404 Not Found"), "{config}");
        assert!(
            config.contains("must listen on 127.0.0.1:8081 only"),
            "{config}"
        );
        assert!(!config.contains("--host-lo-to-ns-lo"), "{config}");
    }

    #[test]
    fn the_media_port_is_asked_for_every_status_path_with_every_host() {
        let requests = public_port_requests();
        let shown: Vec<String> = requests.iter().map(ToString::to_string).collect();
        // Each path once with the transport's own Host, and once per Host.
        assert_eq!(requests.len(), STATUS_PATHS.len() * (1 + 4 + HOSTS.len()));
        assert!(requests.iter().all(|request| request.method == Method::GET));
        for path in STATUS_PATHS {
            let mine = requests
                .iter()
                .filter(|request| request.target == path)
                .collect::<Vec<_>>();
            assert_eq!(mine.len(), 1 + 4 + HOSTS.len(), "{path}: {shown:?}");
            assert_eq!(
                mine.iter()
                    .filter(|request| request.headers.is_empty())
                    .count(),
                1,
                "{path}"
            );
        }
        for host in ["localhost", "localhost:8081", "127.0.0.1", "127.0.0.1:8081"]
            .into_iter()
            .chain(HOSTS)
        {
            let with_host = requests
                .iter()
                .filter(|request| {
                    request
                        .headers
                        .iter()
                        .any(|(name, value)| name == "host" && value == host)
                })
                .count();
            assert_eq!(with_host, STATUS_PATHS.len(), "{host}: {shown:?}");
        }
    }

    #[test]
    fn only_a_stub_status_page_counts_as_the_page() {
        assert!(is_page(PAGE.as_bytes()));
        for body in [
            b"".as_slice(),
            b"<html><head><title>404 Not Found</title></head></html>\n",
            b"ok\n",
            b"\xff\xfe",
        ] {
            assert!(!is_page(body), "{body:?}");
        }
    }

    #[test]
    fn the_media_port_must_answer_404_without_the_page() {
        let request = "GET /stub_status HTTP/1.0 without Host on 127.0.0.1:1";
        let refused = response(StatusCode::NOT_FOUND, Some("text/html"), "<html>");
        assert_eq!(
            public_answer_problems(request, &refused),
            Vec::<String>::new()
        );
        let served = response(StatusCode::OK, Some("text/plain"), PAGE);
        let problems = public_answer_problems(request, &served);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("status must be 404, got 200 OK"));
        assert!(problems[1].contains("served the stub_status page"));
        // The page under another status is the page all the same.
        let hidden = response(StatusCode::NOT_FOUND, Some("text/plain"), PAGE);
        let problems = public_answer_problems(request, &hidden);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("served the stub_status page"));
    }

    /// The table of the pinned image under the container test, as Linux
    /// wrote it on a little-endian host: the two listeners and the
    /// `TIME_WAIT` remains of two reads, which belong to no user.
    #[cfg(target_endian = "little")]
    #[test]
    fn the_table_of_the_pinned_image_shows_the_two_listeners() {
        let table = format!(
            "{HEADER}\n\
             \x20  0: 0100007F:1F91 00000000:0000 0A 00000000:00000000 00:00000000 00000000 12345        0 8734648 1 000000003ba368a4 100 0 0 10 0\n\
             \x20  1: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 12345        0 8734647 1 0000000056aba9e7 100 0 0 10 0\n\
             \x20  2: 0100007F:AACA 0100007F:1F91 06 00000000:00000000 03:0000175C 00000000     0        0 0 3 00000000f5efff98\n"
        );
        assert_eq!(
            listening(&table, 12345),
            Ok(vec![
                "127.0.0.1:8081".parse().expect("an address"),
                "0.0.0.0:8080".parse().expect("an address"),
            ])
        );
        assert_eq!(listening(&table, 0), Ok(Vec::new()));
        // An IPv6 listener on ::1 and one on every address.
        let table6 = format!(
            "{HEADER}\n\
             \x20  0: 00000000000000000000000001000000:1F91 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000 12345        0 1 1 0 100 0 0 10 0\n\
             \x20  1: 00000000000000000000000000000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000 12345        0 2 1 0 100 0 0 10 0\n"
        );
        assert_eq!(
            listening(&table6, 12345),
            Ok(vec![
                "[::1]:8081".parse().expect("an address"),
                "[::]:8080".parse().expect("an address"),
            ])
        );
    }

    #[test]
    fn an_empty_table_has_no_listeners() {
        assert_eq!(listening(&format!("{HEADER}\n"), UID), Ok(Vec::new()));
    }

    #[test]
    fn a_table_in_another_layout_is_refused() {
        let line = socket_line(LISTENERS[1], TCP_LISTEN, UID);
        for (table, message) in [
            (String::new(), "the first line is not the header"),
            (line.clone(), "the first line is not the header"),
            (
                format!("{HEADER}\n   0: 0100007F:1F91 0A"),
                "not a socket line",
            ),
            (
                format!("{HEADER}\n{}", line.replace(" 12345 ", " root ")),
                "is not a UID",
            ),
            (
                format!("{HEADER}\n{}", line.replacen(":1F91", ":1F9", 1)),
                "is not a hex address:port",
            ),
            (
                format!("{HEADER}\n{}", line.replacen(":1F91", ":ZZZZ", 1)),
                "is not a hex address:port",
            ),
        ] {
            let error = listening(&table, UID).expect_err(&table);
            assert!(error.contains(message), "{table:?}: {error}");
        }
    }

    #[test]
    fn addresses_of_other_lengths_or_digits_are_refused() {
        for field in [
            "0100007F",
            "0100007F:",
            "0100007F:1F91:00",
            "0100007:1F91",
            "0100007F00:1F91",
            "0100007F0100007F:1F91",
            "0100007G:1F91",
            "+100007F:1F91",
            "Ö100007F:1F91",
        ] {
            assert!(socket_address(field).is_err(), "{field}");
        }
    }

    proptest! {
        #[test]
        fn every_page_nginx_writes_parses_back(
            values in proptest::array::uniform7(any::<u64>()),
        ) {
            let [active, accepted, handled, requests, reading, writing, waiting] = values;
            let page = format!(
                "Active connections: {active} \nserver accepts handled requests\n \
                 {accepted} {handled} {requests} \nReading: {reading} Writing: {writing} \
                 Waiting: {waiting} \n"
            );
            prop_assert_eq!(
                StubStatus::parse(&page),
                Ok(StubStatus { active, accepted, handled, requests, reading, writing, waiting })
            );
            prop_assert!(is_page(page.as_bytes()));
        }

        #[test]
        fn the_parsers_never_panic(page in "\\PC*\n?", bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
            let _ = StubStatus::parse(&page);
            let _ = listening(&page, UID);
            let _ = parse_reply(&bytes);
            let _ = is_page(&bytes);
        }

        #[test]
        fn a_listener_of_the_owner_reads_back_from_its_line(
            ipv4 in any::<[u8; 4]>(),
            ipv6 in any::<[u8; 16]>(),
            port in any::<u16>(),
            owner in any::<u32>(),
            other in any::<u32>(),
            state in prop_oneof![Just("0A"), Just("01"), Just("06"), Just("07")],
        ) {
            prop_assume!(owner != other);
            for local in [
                SocketAddr::from((Ipv4Addr::from(ipv4), port)),
                SocketAddr::from((Ipv6Addr::from(ipv6), port)),
            ] {
                let table = format!("{HEADER}\n{}\n", socket_line(local, state, owner));
                let expected = if state == TCP_LISTEN { vec![local] } else { Vec::new() };
                prop_assert_eq!(listening(&table, owner), Ok(expected));
                prop_assert_eq!(listening(&table, other), Ok(Vec::new()));
            }
        }

        #[test]
        fn a_reply_keeps_its_body_byte_for_byte(
            code in 100u16..600,
            body in proptest::collection::vec(any::<u8>(), 0..300),
        ) {
            let mut printed = format!("HTTP/1.1 {code} Reason\r\nServer: nginx\r\n\r\n").into_bytes();
            printed.extend(&body);
            let reply = parse_reply(&printed).expect("a reply");
            prop_assert_eq!(reply.status.as_u16(), code);
            prop_assert_eq!(reply.single("server"), Some("nginx"));
            prop_assert_eq!(reply.body.as_ref(), body.as_slice());
        }
    }
}
