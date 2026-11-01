//! The media contract against `deploy/media/nginx.conf` in the pinned image
//! of `deploy/media/image.txt` (ADR 0015 §1, "What must be true").
//!
//! The test starts the image with docker or podman, the way the cluster runs
//! it under restricted-v2 (ADR 0023 §8): an arbitrary UID with GID 0, no
//! capabilities, no privilege escalation and a read-only root filesystem
//! with only `/tmp` writable. The configuration and a freshly seeded fixture
//! tree are mounted read-only where the Deployment mounts the `ConfigMap` and
//! the volume's `app/public`. Each run publishes on a free loopback port and
//! uses its own container name, so runs in parallel do not collide. Beyond
//! [`run`], it checks what only nginx must do: its probe, nothing outside
//! `/storage`, both logs on stdout, no alarming log line, and the loopback
//! `stub_status` listener of the media exporter ([`stub_status`]).
//!
//! It is `#[ignore]`d because it needs a container engine and pulls the
//! image. The Test job runs it in its own step:
//! `cargo nextest run --locked --workspace --all-targets --run-ignored only -E 'test(/^media_contract::nginx::/)'`.
//! [`config`] reads the file itself, without an engine, in every test run:
//! it shows where the file serves `stub_status`, whatever the request.

use std::{
    fs,
    hash::{BuildHasher, RandomState},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use axum::http::{Method, StatusCode};

use super::{HyperTransport, Origin, Server, Transport, fixtures::LINKS, http::Request, run, seed};
use crate::support::repository_root;

mod config;
mod stub_status;

/// The pinned image reference (tag and digest), the one source for the CI
/// run, the docs and `website_k8s`.
const IMAGE: &str = include_str!("../../deploy/media/image.txt");

/// An arbitrary UID that is not the image's own (1001), as restricted-v2
/// assigns one; the container runs as `UID:0`, with GID 0. It stays below
/// 65536 so that rootless podman can map it. The kernel's socket table names
/// it as the owner of nginx's listeners ([`stub_status`]).
const UID: u32 = 12345;

/// The media server's port, the one the Service and the Routes reach
/// (ADR 0015 §1).
const MEDIA_PORT: u16 = 8080;

/// The engines tried, in order: docker on the CI runner, podman (or its
/// docker shim) on a workstation.
const ENGINES: [&str; 2] = ["docker", "podman"];

/// How long nginx may take to answer `/healthz` after the container starts.
const START_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn the_image_is_pinned_by_tag_and_digest() {
    let image = IMAGE.trim();
    assert_eq!(IMAGE, format!("{image}\n"), "one line, newline-terminated");
    let (name, digest) = image.split_once("@sha256:").expect("pinned by digest");
    assert!(
        name.starts_with("registry.access.redhat.com/ubi9/nginx-126:"),
        "{name}: ubi9/nginx-126 with its tag (ADR 0015 §1)"
    );
    assert_eq!(digest.len(), 64, "{digest}");
    assert!(
        digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
}

#[tokio::test]
#[ignore = "starts the pinned nginx image with docker or podman; the Test job runs it in its own step"]
async fn the_deployed_config_meets_the_media_contract() {
    let tree = Scratch::new();
    seed(tree.path()).expect("seed the fixture tree");
    let container = Container::start(tree.path());
    let transport = HyperTransport::new(Duration::from_secs(10));
    let origin = container.wait_until_ready(&transport).await;

    let report = run(&transport, &origin, Server::Nginx).await;
    let mut failures = Vec::new();
    failures.extend(outside_storage(&transport, &origin).await);
    // Before the logs are read, so that its reads would show in them.
    failures.extend(stub_status::check(&container, &transport).await);
    let (stdout, stderr) = container.logs();
    failures.extend(stub_status::logged_reads(&stdout));
    let logs = format!("{stdout}{stderr}");
    failures.extend(
        logs.lines()
            .filter(|line| alarming(line))
            .map(|line| format!("nginx logged: {line}")),
    );
    // ADR 0015 §1, Logs: access and error logs go to stdout. The engine's
    // `logs` keeps the container's two streams apart, so a line on the wrong
    // one shows. The error log's start-up notice stands for every error
    // line, and nginx writes nothing else to stderr.
    if !stdout.contains("\"GET /healthz HTTP/1.1\" 200") {
        failures.push("no access-log line for GET /healthz on stdout".to_owned());
    }
    if !stdout.contains("[notice]") {
        failures.push("no error-log line (the start-up notices) on stdout".to_owned());
    }
    // Only nginx's own lines count: the engine's CLI may add notes of its
    // own on stderr, such as podman's docker shim.
    failures.extend(
        stderr
            .lines()
            .filter(|line| from_nginx(line))
            .map(|line| format!("nginx wrote to stderr: {line}")),
    );
    assert!(
        report.passed() && failures.is_empty(),
        "{report}nginx-only checks and log:\n  - {}\n--- container log ---\n{logs}",
        failures.join("\n  - ")
    );
}

/// The levels of nginx's error log, as each of its lines names one.
const LEVELS: [&str; 8] = [
    "[debug]", "[info]", "[notice]", "[warn]", "[error]", "[crit]", "[alert]", "[emerg]",
];

/// Whether a log line is one that nginx wrote: an error-log line, which
/// starts with `YYYY/MM/DD HH:MM:SS [level]`, or a line of the `media`
/// access-log format, which quotes an HTTP/1.x request line.
///
/// - `line`: one line of the container's stdout or stderr.
fn from_nginx(line: &str) -> bool {
    let bytes = line.as_bytes();
    let dated = bytes.len() > 20
        && [4, 7].iter().all(|&at| bytes[at] == b'/')
        && bytes[13] == b':'
        && bytes[16] == b':'
        // Bytes, not a str slice: byte 20 need not be a char boundary.
        && LEVELS
            .iter()
            .any(|level| bytes[20..].starts_with(level.as_bytes()));
    dated || line.contains(" HTTP/1.1\" ") || line.contains(" HTTP/1.0\" ")
}

#[test]
fn nginx_lines_are_told_from_an_engine_note() {
    for line in [
        "2026/10/25 16:02:39 [notice] 1#1: using the \"epoll\" event method",
        "2026/10/25 16:02:39 [emerg] 1#1: open() \"/tmp/nginx.pid\" failed",
        "10.0.1.0 \"-\" [2026-10-25T15:54:11+00:00] \"GET /healthz HTTP/1.1\" 200 3 0.000 \"-\" \"x\"",
    ] {
        assert!(from_nginx(line), "{line}");
    }
    for line in [
        "Emulate Docker CLI using podman. Create /etc/containers/nodocker to quiet msg.",
        "",
        "2026/10/25 16:02:39 something else",
        "WARNING: the container log driver dropped lines",
    ] {
        assert!(!from_nginx(line), "{line}");
    }
}

/// Whether a line of the container log shows a problem: every `[emerg]`,
/// `[alert]` and `[crit]` line does, except the one nginx writes when
/// `disable_symlinks` refuses a link of the fixture tree.
///
/// `try_files` logs every failed open that is not "not found" at crit level
/// and then answers 404, which is the refusal the contract wants. Only that
/// line, for a path of `fixtures::LINKS` and the "too many levels of
/// symbolic links" error (`ELOOP`, 40 on Linux), is expected.
///
/// - `line`: one line of `logs`.
fn alarming(line: &str) -> bool {
    if !["[emerg]", "[alert]", "[crit]"]
        .iter()
        .any(|level| line.contains(level))
    {
        return false;
    }
    let refused_link = line.contains("[crit]")
        && line.contains(" failed (40: Too many levels of symbolic links)")
        && LINKS.iter().any(|(path, _)| {
            // The served path, as nginx quotes it: the link itself, or a
            // file below a linked folder.
            let quoted = format!("\"/srv/media/storage/{path}");
            [format!("{quoted}\""), format!("{quoted}/")]
                .iter()
                .any(|start| line.contains(start.as_str()))
        });
    !refused_link
}

#[test]
fn only_the_refused_fixture_links_may_log_at_crit() {
    let refused = |path: &str| {
        format!(
            "2026/10/25 15:54:11 [crit] 2#2: *2 openat() \"/srv/media/storage/{path}\" failed \
             (40: Too many levels of symbolic links), client: 10.0.1.0, server: , request: \
             \"GET /storage/{path} HTTP/1.1\", host: \"127.0.0.1:34199\""
        )
    };
    for (path, _) in LINKS {
        assert!(!alarming(&refused(path)), "{path}");
    }
    assert!(!alarming(&refused("uploads/mappa/Órarend.PDF")));
    // The same error for a path that is not a fixture link, a prefix of one,
    // or another error for a link, is a problem.
    assert!(alarming(&refused("uploads/docs/other.pdf")));
    assert!(alarming(&refused("uploads/docs/hivatkozas.pdf.bak")));
    assert!(alarming(
        &refused("uploads/docs/hivatkozas.pdf").replace("40: Too many", "13: Permission")
    ));
    assert!(alarming(
        "2026/10/25 15:54:09 [emerg] 1#1: open() \"/tmp/nginx.pid\" failed"
    ));
    assert!(alarming(
        &refused("uploads/docs/hivatkozas.pdf").replace("[crit]", "[alert]")
    ));
    assert!(!alarming(
        "2026/10/25 15:54:09 [notice] 1#1: start worker processes"
    ));
    assert!(!alarming(
        "10.0.1.0 \"-\" [2026-10-25T15:54:11+00:00] \"GET /healthz HTTP/1.1\" 200 3 0.000"
    ));
}

/// What only the media server must do: answer its probe, and serve nothing
/// outside `/storage` (backend3 has other routes there). That includes the
/// exporter's `stub_status`, which only the loopback listener serves
/// (research: DECISIONS R15, D93): no common status path, with no `Host`
/// that names a loopback or a public host, may get the page here
/// ([`stub_status::public_port_requests`]). nginx picks the server by the
/// port a connection arrived on; [`config`] covers the requests not sent.
async fn outside_storage(transport: &HyperTransport, origin: &Origin) -> Vec<String> {
    let get = |target: &str| Request::new(Method::GET, target);
    let mut failures = Vec::new();
    let requests = [
        (get("/healthz"), StatusCode::OK, Some(b"ok\n".as_slice())),
        (get("/"), StatusCode::NOT_FOUND, None),
        (get("/storage"), StatusCode::NOT_FOUND, None),
        (get("/sitemap.xml"), StatusCode::NOT_FOUND, None),
        (
            get("/srv/media/storage/sitemap.xml"),
            StatusCode::NOT_FOUND,
            None,
        ),
    ]
    .into_iter()
    .chain(
        stub_status::public_port_requests()
            .into_iter()
            .map(|request| (request, StatusCode::NOT_FOUND, None)),
    );
    for (request, expected, body) in requests {
        match transport.send(origin, &request).await {
            Ok(response) => {
                if response.status != expected {
                    failures.push(format!(
                        "{request}: status must be {expected}, got {}",
                        response.status
                    ));
                }
                if body.is_some_and(|body| response.body.as_ref() != body) {
                    failures.push(format!("{request}: body must be {body:?}"));
                }
                if stub_status::is_page(&response.body) {
                    failures.push(format!(
                        "{request}: the media port {MEDIA_PORT} served the stub_status page, \
                         which only the loopback listener may serve"
                    ));
                }
                if response.single("access-control-allow-origin") != Some("*") {
                    failures.push(format!(
                        "{request}: every response carries one Access-Control-Allow-Origin: *"
                    ));
                }
            }
            Err(error) => failures.push(error),
        }
    }
    failures
}

/// A name suffix that no other fixture directory or container of this suite
/// uses, in this process or in any other run, finished, crashed or running.
///
/// It is built without the wall clock: recipe 7 point 4 of
/// `.claude/skills/backend3/SKILL.md` allows reads of the wall clock in tests
/// only for the session tests and the boot test of the system clock. The
/// process id keeps parallel runs apart; the counter keeps two names of one
/// process apart; and the keyed hash, whose keys `RandomState` draws from the
/// operating system's random source, keeps this run apart from a crashed
/// earlier one with a reused process id that left its directory or container
/// behind.
fn unique_suffix() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let random = RandomState::new().hash_one(sequence);
    format!("{}-{sequence}-{random:016x}", std::process::id())
}

/// A directory below Cargo's scratch directory for this test target,
/// removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("media-contract-{}", unique_suffix()));
        fs::create_dir_all(&path).expect("create the fixture directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The running media server; stopped and removed when dropped.
struct Container {
    engine: &'static str,
    name: String,
}

impl Container {
    /// Starts the image with the repository's configuration over `tree`.
    fn start(tree: &Path) -> Self {
        let config = repository_root().join("deploy/media/nginx.conf");
        let name = format!("media-contract-{}", unique_suffix());
        let args = [
            "run".to_owned(),
            "--detach".to_owned(),
            format!("--name={name}"),
            format!("--user={UID}:0"),
            "--read-only".to_owned(),
            "--tmpfs=/tmp:rw,mode=1777,size=16m".to_owned(),
            "--cap-drop=ALL".to_owned(),
            "--security-opt=no-new-privileges".to_owned(),
            // The bind mounts are read as they are; no SELinux relabelling of
            // the checkout on a workstation.
            "--security-opt=label=disable".to_owned(),
            format!("--publish=127.0.0.1::{MEDIA_PORT}"),
            // The stub_status port too, only so that the suite can try it
            // the way a Service or another pod would: docker, and podman's
            // pasta by default, forward a published port to the container's
            // own address, which the loopback listener must refuse
            // ([`stub_status::check`] says what an engine that does not
            // means).
            format!("--publish=127.0.0.1::{}", stub_status::PORT),
            format!("--volume={}:/etc/nginx/nginx.conf:ro", config.display()),
            format!("--volume={}:/srv/media/storage:ro", tree.display()),
            // The Deployment's command (deploy/media/README.md): nginx itself,
            // not the image's s2i scripts, which write to the root filesystem.
            "--entrypoint=nginx".to_owned(),
            IMAGE.trim().to_owned(),
            "-e".to_owned(),
            "/dev/stdout".to_owned(),
        ];
        let mut tried = Vec::new();
        for engine in ENGINES {
            match Command::new(engine).args(&args).output() {
                Ok(output) if output.status.success() => return Self { engine, name },
                Ok(output) => {
                    // A docker CLI with no daemon behind it, on a podman
                    // workstation, fails here: try the next engine. A run
                    // that failed after creating the container leaves it
                    // behind under this name, so remove it first.
                    let _ = Command::new(engine).args(["rm", "--force", &name]).output();
                    tried.push(format!("{engine} run failed: {}", text(&output).trim()));
                }
                Err(error) => tried.push(format!("{engine}: {error}")),
            }
        }
        panic!(
            "no container engine to start the media server: {}",
            tried.join("; ")
        );
    }

    /// The host's loopback address and port that the engine published for
    /// the container's `port`, such as `127.0.0.1:40123`.
    ///
    /// - `port`: a container port that [`Container::start`] published.
    fn published_address(&self, port: u16) -> String {
        let output = self.engine(&["port", &self.name, &format!("{port}/tcp")]);
        text(&output)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// [`Container::published_address`] as an origin.
    ///
    /// - `port`: a container port that [`Container::start`] published.
    fn published(&self, port: u16) -> Origin {
        format!("http://{}", self.published_address(port))
            .parse()
            .unwrap_or_else(|error| panic!("{} port {port}: {error}", self.engine))
    }

    /// The published media port as an origin, once `/healthz` answers.
    async fn wait_until_ready(&self, transport: &HyperTransport) -> Origin {
        let origin = self.published(MEDIA_PORT);
        let deadline = Instant::now() + START_TIMEOUT;
        let probe = Request::new(Method::GET, "/healthz");
        loop {
            match transport.send(&origin, &probe).await {
                Ok(response) if response.status == StatusCode::OK => return origin,
                outcome if Instant::now() >= deadline => {
                    let (stdout, stderr) = self.logs();
                    panic!(
                        "nginx did not answer /healthz within {START_TIMEOUT:?} ({outcome:?}); \
                         log:\n{stdout}{stderr}"
                    )
                }
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    /// What nginx wrote to stdout and to stderr, in that order.
    fn logs(&self) -> (String, String) {
        let output = self.engine(&["logs", &self.name]);
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Runs `command` inside the running container, in its network
    /// namespace, as the container's user; what it printed and its exit
    /// status, or the engine's own error.
    ///
    /// - `command`: the program and its arguments, run without a shell.
    fn exec(&self, command: &[&str]) -> Output {
        let args: Vec<&str> = ["exec", self.name.as_str()]
            .into_iter()
            .chain(command.iter().copied())
            .collect();
        self.engine(&args)
    }

    fn engine(&self, args: &[&str]) -> Output {
        Command::new(self.engine)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("{} {args:?}: {error}", self.engine))
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["stop", "--time=2", &self.name])
            .output();
        // --force removes it even when the stop above failed.
        let _ = Command::new(self.engine)
            .args(["rm", "--force", &self.name])
            .output();
    }
}

/// stdout and stderr of a finished command, as text.
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
