# deploy/media: the media server

The media server serves the uploads volume's `app/public/` tree as
`/storage/**` on all three hosts ([ADR 0015](../../docs/design/adr/0015-media-separate-static-server-media-library-image-optimisatio.md) §1-§2).
This directory is the only source of its configuration:

| File | What it is |
|---|---|
| `nginx.conf` | The complete nginx configuration. It implements the media contract of ADR 0015 §1 and the `/storage` header rows of [ADR 0013](../../docs/design/adr/0013-rich-text-editor-and-html-sanitiser.md) §7, and serves `stub_status` to the media exporter on the pod's loopback address ([below](#the-exporters-source-stub_status-on-loopback)). Its comments say why each line is there. |
| `image.txt` | The image, `registry.access.redhat.com/ubi9/nginx-126`, by tag and digest. The maintainer chose nginx 1.26 over 1.24 on 2026-10-25 (research: DECISIONS R9, D35); the 1.26 stream retires in May 2028. |

The maintainer decided that this repository holds the configuration and that
website_k8s copies it byte for byte (research: DECISIONS R9, D15). The
contract suite therefore tests exactly the bytes that are deployed.

The pinned image runs nginx 1.26.3 as user 1001 in group 0 and exposes port
8080, as the sclorg nginx-container 1.26 README describes for the image
family (<https://github.com/sclorg/nginx-container/blob/master/1.26/README.md>,
read 2026-10-25). The configuration does not rely on the image's own user
or its s2i scripts. Its nginx is built with `stub_status` (`nginx -V` in the
pinned image lists `--with-http_stub_status_module`, read 2026-10-31), and the
image ships `curl`.

## The contract suite

`tests/media_contract/` checks a running server against the contract: paths
with accents, spaces and upper-case extensions; 404 for missing files,
directories, dot-files and names that differ only in case or Unicode
normalisation; no file served through a symbolic link; `Access-Control-Allow-Origin: *`
exactly once on every response, 404 and 416 included; the CORS preflight
for `Range`; 206, 416 and the multi-range answer (the whole file with 200,
or 416); `Last-Modified`, `ETag`, `If-Modified-Since` and `If-None-Match`;
`Cache-Control` per folder; `nosniff`; the sandbox CSP except on PDF;
`Content-Disposition: attachment` outside the four raster types and PDF,
with `/storage/sitemap.xml` exempt and never on a 404; no single-valued
header sent twice; and `GET`, `HEAD` and `OPTIONS` only. It sends the file
requests with the `Host` of each of the three hosts. Where the contract
tables leave an answer open, the suite's readings are listed in
`tests/media_contract/mod.rs`.

| Where | Command |
|---|---|
| CI | The `Test` job's "Media contract" step. |
| Locally, with docker or podman | `cargo nextest run --locked --workspace --all-targets --run-ignored only -E 'test(/^media_contract::nginx::/)'` |
| A running server, such as a pod | `cargo run --locked --example media_contract -- seed <empty directory>`, copy that tree into the served tree as below, then `cargo run --locked --example media_contract -- check http://127.0.0.1:<port> nginx` |

The local and CI run starts the image the way restricted-v2 runs it: an
arbitrary UID with GID 0, no capabilities, no privilege escalation, a
read-only root filesystem and a tmpfs at `/tmp`. Each run uses a free
loopback port and its own container name, so parallel runs do not collide.
It also checks that the access and error logs reach stdout, and the
exporter's listener (`tests/media_contract/nginx/stub_status.rs`):

- On port 8080, `/stub_status` and the other usual status paths
  (`/nginx_status`, `/status`, `/basic_status`, `/storage/stub_status`,
  `/stub_status/`) are `404`, never the page. That holds with the published
  address as `Host`, with `localhost`, `127.0.0.1` (each with and without
  `:8081`) and each of the three hosts, and for an HTTP/1.0 request without
  `Host`.
- Inside the container, `GET /stub_status` on `127.0.0.1:8081` answers
  `200 text/plain` in the layout the exporter reads. The listener serves
  nothing else and logs no access.
- The kernel's socket table shows nginx listening on exactly `0.0.0.0:8080`
  and `127.0.0.1:8081`.
- A connection to the published 8081 gets no answer. This needs an engine
  that forwards a published port to the container's own address, as a
  Service does to the pod's: docker does, and so does podman's pasta by
  default. An engine that hands host-loopback connections to the
  container's loopback instead (pasta with `--host-lo-to-ns-lo`) gets the
  page, and the test then fails with a message that says so.

`tests/media_contract/nginx/config.rs` covers the requests that the running
server is not sent. It reads `nginx.conf` itself, in every test run and
without an engine. It requires one `stub_status` directive, inside the
server whose only `listen` is `127.0.0.1:8081`; exactly two servers, that
one and the media server on `8080 default_server`, and no `listen`
elsewhere; no `include` but the image's `mime.types`; and no `*_pass`
directive, which could proxy port 8080 to the loopback listener. The `check`
command runs none of these checks.

The suite checks the server itself, never a Route: a router picks its Route
by the `Host` header that the suite varies. The `check` command speaks plain
HTTP and refuses an `https` origin. Reach the pod with
`oc port-forward pod/<media pod> <port>:8080`.

For the co-mount test (ADR 0015 §3), seed a throwaway volume only, never the
production claim:

1. Write the fixture tree with `seed` into an empty directory.
2. Copy it to the root of the served tree last, after the real folders, so
   that it overwrites any real file at a fixture path. The real folders can
   hold two of them: the Diáktoll PDF (`uploads/diaktoll/ujsag_final_2021osz.pdf`)
   and `sitemap.xml`. A real file left there fails the body checks, or, over
   1 MiB, the read.
3. Copy with a method that keeps modification times and symbolic links,
   such as `tar` (which `oc cp` uses) or `cp -a`. The suite expects the
   exact `Last-Modified` that `seed` sets and the links that `seed` creates.

Other files may sit beside the fixtures, but none of the paths that the
fixture table lists as missing may exist.

## How the cluster runs it

website_k8s owns the manifests. They must keep to what the suite tested:

| Item | Value |
|---|---|
| Image | The line of `image.txt`, by digest. |
| Command | `nginx -e /dev/stdout`. Not the image's s2i run script, which writes to the root filesystem. `-e` moves the error log that nginx opens before it reads the configuration to stdout, where the configuration's `error_log` and `access_log` also write (ADR 0015 §1, Logs). |
| Configuration | A ConfigMap whose key `nginx.conf` holds this file byte for byte, mounted read-only at `/etc/nginx/nginx.conf`. |
| Media | `backend-pv-claim`, `subPath: app/public`, read-only at `/srv/media/storage`. |
| Writable paths | An `emptyDir` at `/tmp` for the pid file and nginx's temp directories. Nothing else is written. |
| Security | restricted-v2, `readOnlyRootFilesystem: true`, `allowPrivilegeEscalation: false`, all capabilities dropped ([ADR 0023](../../docs/design/adr/0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §8). |
| Port and probe | 8080; `GET /healthz` answers `200 ok`. No Route sends anything but `/storage` here. |
| Exporter | A second container in the media pod reads `http://127.0.0.1:8081/stub_status` ([below](#the-exporters-source-stub_status-on-loopback)). 8081 is no port of the Service. |

## The exporter's source: `stub_status` on loopback

The media server has a metrics exporter (research: DECISIONS R9, D34), and
the exporter reads nginx's `stub_status` page. website_k8s copies
`nginx.conf` byte for byte, so it cannot add that page itself; the
maintainer decided that this repository adds it, on a listener bound to the
pod's loopback address (research: DECISIONS R15, D93).

- **Where.** `nginx.conf` ends with a second server that listens on
  `127.0.0.1:8081` and serves `stub_status` at `/stub_status`, and `404` for
  every other path. Bound to the loopback address, it takes no connection
  that arrives on the pod's own address, so no Service, Route or other pod
  reaches it. Only what runs in the pod's network namespace does: the
  containers of the media pod, and an `oc port-forward` to the pod, which
  the container runtime connects to `localhost` in that namespace.
  restricted-v2 forbids `hostNetwork`, so the namespace is the pod's own,
  not the node's. On port 8080, `/stub_status` is `404` like every path
  outside `/storage`, whatever the `Host`: the page stands in the loopback
  server only, which listens on no other port.
- **What website_k8s adds.** The exporter container, in the media pod, with
  its scrape URI set to `http://127.0.0.1:8081/stub_status`. The NGINX
  Prometheus Exporter takes it as `--nginx.scrape-uri`, whose default,
  `http://127.0.0.1:8080/stub_status`, is the media port and would only read
  `404`
  (<https://github.com/nginx/nginx-prometheus-exporter/blob/main/README.md>,
  read 2026-10-31). The exporter serves the metrics on a port of its own,
  which no Route targets, and runs under restricted-v2 like the media
  container ([ADR 0023](../../docs/design/adr/0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)
  §8 and §9). Its image, resources and metrics port are the deploy PR's
  choice.
- **Logs.** ADR 0015 §1, Logs, sends the access and error logs to stdout.
  The listener writes no access-log line, so the exporter's reads do not
  fill the log, which keeps one line per request to the media port. Errors
  still go to the error log on stdout.
- **By hand.** `oc exec <media pod> -c <media container> -- curl --silent http://127.0.0.1:8081/stub_status`
  prints the page: the image ships `curl`, and `oc exec` runs inside the
  pod. `oc port-forward pod/<media pod> <port>:8081` reaches it too, for a
  `curl http://127.0.0.1:<port>/stub_status` on the workstation. Both need
  the access to the pod that those commands always need. The page counts
  connections and requests since nginx started; it shows no request detail.

## How website_k8s copies the configuration

1. Pick a commit of this repository's `master` whose `Test` job passed. That
   job ran the suite against the two files at that commit.
2. Copy both files from that commit unchanged, for example
   `git show <commit>:deploy/media/nginx.conf`. Do not reformat, re-indent
   or edit the copy. The ConfigMap is generated from the copied file, so its
   data is the same bytes.
3. Record the commit next to the copy. A drift check compares the copy with
   `git show <commit>:deploy/media/nginx.conf`, and the Deployment's image
   with `image.txt` at that commit, and fails on any difference.
4. Open the copy PR for human review (ADR 0012 §2: agents never merge in
   website_k8s).

A change to the configuration or the image always starts here, as a PR that
passes the suite, and reaches the cluster only through such a copy PR. To
move the image to a newer build, read the digest of the new tag with
`skopeo inspect --no-tags docker://registry.access.redhat.com/ubi9/nginx-126:<tag>`
(`.Digest`), update `image.txt` and let the suite run.
