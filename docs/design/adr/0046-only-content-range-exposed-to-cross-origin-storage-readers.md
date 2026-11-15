# 0046. Only Content-Range exposed to cross-origin `/storage` readers

- Status: Accepted (2026-11-15)
- Decision questions: none: records the maintainer's answer of 2026-11-15, given in chat (research: DECISIONS R34), on what cross-origin PDF readers may read from `/storage` once Verseghy/website_k8s#87 had the media server answer `/storage` on both public hosts. It amends the CORS row of the media contract in [ADR 0015](0015-media-separate-static-server-media-library-image-optimisatio.md) §1, the matching line of that section's nginx sketch, and the CORS item of its "What must be true in code and tests".

## Context

[ADR 0015](0015-media-separate-static-server-media-library-image-optimisatio.md) §1 gives the media contract this CORS row: "`Access-Control-Allow-Origin: *` on every response, 404s included. `Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length`, so pdf.js can use range requests cross-origin. The preflight allows the `Range` header. No credentials." Both servers of the contract sent that list on every answer, the media server through `deploy/media/nginx.conf` and the fallback router of §4 through `src/controllers/storage.rs`, and the media contract suite required it of both.

| Fact | Value | Source |
|---|---|---|
| What the list does | `Access-Control-Expose-Headers` names the response headers that a script on another origin may read from a CORS answer, beyond the CORS-safelisted ones. For a request without credentials, as every `/storage` request is, `*` exposes every header. A same-origin page, a top-level navigation and a browser's own PDF viewer see every header whatever the list says. | Fetch Standard, "CORS-safelisted response-header name" and the CORS protocol |
| `Content-Length` is safelisted | The Fetch Standard lists `Content-Length` among the CORS-safelisted response-header names (with `Cache-Control`, `Content-Language`, `Content-Type`, `Expires`, `Last-Modified` and `Pragma`), so a browser that follows it lets a script read `Content-Length` whether or not the list names it. | Fetch Standard, "CORS-safelisted response-header name" |
| The cross-origin reader | The old Angular site's Diáktoll viewer (`ng2-pdf-viewer`, which runs pdf.js) loads its PDF from the legacy host while the page is on the apex, so it reads the PDF's answer cross-origin. | ADR 0015, "The URL contract" |
| How pdf.js chooses ranges | pdf.js switches to range requests only when it can read `Accept-Ranges: bytes` and an integer `Content-Length` of more than two chunks, with no content encoding. Otherwise it reads the file whole from its first request. | pdf.js `src/display/network_utils.js` (`validateRangeRequestCapabilities`) |
| What backend1 exposed | backend1, which served `/storage` before the media server, sent `Access-Control-Allow-Origin: *` but exposed neither `Accept-Ranges` nor `Content-Length`, so the viewer read the PDF whole. | research: DECISIONS R34 |
| The service worker and ranges | The old site's Angular service worker (`ngsw-worker.js`) answers a request that carries `Range` with `200` and the whole file. pdf.js rejects that answer to a range request, without a console error. With the service worker bypassed (`ngsw-bypass`), the media server answers the same request with `206`; checked on 2026-11-15. | research: DECISIONS R34 |
| pdf.js under the service worker | Verseghy/website_k8s#87 had the media server answer `/storage` on both public hosts on 2026-11-15. A cross-origin pdf.js that can read `Accept-Ranges: bytes` asks a large PDF for ranges, and a page that the old site's service worker controls gets the whole file in answer, which pdf.js does not use. A pdf.js that cannot read `Accept-Ranges` asks for the whole file, and the service worker's answer is the file it asked for. | research: DECISIONS R34 |

The maintainer, asked what to change, answered on 2026-11-15: "Leave it, fix forward" and "Media server: stop exposing" (research: DECISIONS R34). The first answer keeps the public `/storage` Routes on the media server; the second is this ADR's decision. This ADR records that answer and is Accepted on it. It lands in the same PR that changes the media contract, because a change that contradicts an Accepted ADR merges only with the ADR that amends it (ADR 0001 §6).

## Decision

### 1. `/storage` exposes `Content-Range` and nothing else

- **The header.** Every `/storage` answer carries `Access-Control-Expose-Headers: Content-Range`, exactly once, refusals and `416` included, as every answer already carried the longer list. The list names no other header: not `Accept-Ranges`, not `Content-Length`, not `*`.
- **What a cross-origin reader sees.** A script on another origin can no longer read `Accept-Ranges`, so pdf.js reads a PDF whole with one plain `GET`, as it did from backend1, and never sends the range requests that the old site's service worker answers with the whole file. It can still read `Content-Length` where the browser safelists it.
- **What stays.** `Content-Range` stays exposed, so a cross-origin script that sends its own `Range` can still read which bytes it got and the file's size. Everything else in the CORS row holds: `Access-Control-Allow-Origin: *` on every answer, the preflight that allows `Range` for `GET` and `HEAD`, and no credentials.
- **Ranges are unchanged.** A file's `200` still sends `Accept-Ranges: bytes` and `Content-Length`. A single range still gets `206`, or `416` when it cannot be satisfied, and a multi-range request still gets each server's answer of ADR 0015 §1, Ranges. Only what a cross-origin script may read changes; same-origin pages, browsers' own PDF viewers, download managers and other clients that need no CORS keep using ranges.
- **Both servers.** The media server's `deploy/media/nginx.conf` and the fallback router, through `EXPOSE_HEADERS` in `src/domain/storage_rules.rs`, send the same value, and the media contract suite holds both to it. The media server's image does not change.

### 2. What this amends

[ADR 0015](0015-media-separate-static-server-media-library-image-optimisatio.md):

| Amended text | Change |
|---|---|
| §1, the media contract table, the CORS row: "`Access-Control-Expose-Headers: Accept-Ranges, Content-Range, Content-Length`, so pdf.js can use range requests cross-origin." | `Access-Control-Expose-Headers: Content-Range`, and no other name (section 1). A cross-origin pdf.js reads a PDF whole; range requests remain for clients that need no CORS. |
| §1, the nginx sketch: `add_header Access-Control-Expose-Headers "Accept-Ranges, Content-Range, Content-Length" always;` | The value is `"Content-Range"`. The sketch stays a sketch; `deploy/media/nginx.conf` is the configuration. |
| "What must be true in code and tests", the media contract suite: "CORS headers, including on a 404, and the preflight" | The suite also fails an `Access-Control-Expose-Headers` that lists any name but `Content-Range`, `*` included (Consequences, "What must be true in code and tests"). |

Everything else in ADR 0015 holds, with the amendments of §7 by [ADR 0033](0033-image-optimisation-ssim-floor-reference-parsing-cpu-limits-a.md). That includes ADR 0015, "The URL contract": the Diáktoll viewer still needs `Access-Control-Allow-Origin: *`, which every answer keeps.

## Consequences

**Positive**

- A cross-origin pdf.js reads a PDF with one plain `GET`, as it did from backend1, so the Diáktoll viewer does not depend on how a service worker answers `Range`.
- Both servers send the same one-name list, and one suite holds them to it.
- Ranges keep working wherever no cross-origin script reads them: same-origin pages such as the admin on the backend3 host, top-level navigations to a PDF, browsers' own viewers and media players, and download tools.

**Negative**

- A cross-origin viewer reads the whole PDF, as it did from backend1, instead of only the parts that the pages it shows need.
- A cross-origin script cannot learn from the headers whether the server takes ranges. It can send a `Range` and look for `206` and `Content-Range`.
- The change reaches the cluster only through a website_k8s copy PR (follow-up 1), which a human reviews and merges.

**Follow-up work**

1. A website_k8s copy PR, for human review: the comment-stripped `deploy/media/nginx.conf` at the commit that merges this ADR, with that commit in the ConfigMap's annotation (`deploy/media/README.md`, "How website_k8s copies the configuration"). `deploy/media/image.txt` does not change, so neither does the media container's image.
2. Once the copy is deployed, the Diáktoll page is checked again on the apex with the service worker controlling the page, together with the `/storage` headers on both public hosts. The old site's service worker keeps `/storage` files of the legacy host in its `images` cache and answers from a cached copy first, with the header list that the copy came with; it refetches the copy in the background only once the copy is older than its `max-age` (3600 seconds for `uploads/`) (website_frontend@f74c765:apps/frontend/ngsw-config.json; `@angular/service-worker` 14.2.12, `AssetGroup.handleFetch`). A worker that cached the PDF before the copy was deployed therefore keeps answering with `Accept-Ranges` exposed until it has refetched the copy. The check starts from an empty service-worker cache, or loads the page once more after a first load has refetched a copy older than an hour, before it judges the result.

**What must be true in code and tests**

- `deploy/media/nginx.conf` sends `Access-Control-Expose-Headers: Content-Range` on every response of the media server's port, a `416` included, exactly once.
- `EXPOSE_HEADERS` in `src/domain/storage_rules.rs` is `Content-Range`, and the fallback router sets it on every answer; a unit test pins that it names neither `Accept-Ranges`, nor `Content-Length`, nor `*`.
- The media contract suite (`tests/media_contract/checks.rs`, `common`) fails a response whose `Access-Control-Expose-Headers` is missing, is sent twice, or lists any name but `Content-Range` (`rules::lists_exactly`, with unit tests and a property). It runs against nginx in its pinned image and against the fallback router (`tests/requests/storage.rs`), and its unit tests pin ADR 0015's former list, `*` and a second header line as failures.
- The suite's range probes do not change: `206` and `416` for single ranges, the multi-range answer of each server and `If-Range`, all sent with the site's `Origin`.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Move the public `/storage` Routes back to backend1 until the media contract changes | research: DECISIONS R34 | Not chosen: the maintainer answered "Leave it, fix forward": the Routes stay on the media server, and its contract changes instead. |
| Keep the list and change the old site instead (pdf.js's `disableRange` in the viewer, or a service worker that passes range requests to the network) | not offered; considered while writing this ADR | The maintainer chose to change the media server ("Media server: stop exposing", R34). The old site is website_frontend, which this repository does not change, and the fix would wait for a new frontend build. |
| Drop `Accept-Ranges` only and keep `Content-Length` | not offered; considered while writing this ADR | The answer stops exposing both, matching backend1. A browser that follows the Fetch Standard lets a script read `Content-Length` anyway, so listing it adds nothing there. |
| Send no `Access-Control-Expose-Headers` at all | not offered; considered while writing this ADR | `Content-Range` stays in the answer (R34), so that a cross-origin script that asks for a range can read which bytes it got. |
| `Access-Control-Expose-Headers: *` | not offered; considered while writing this ADR | For a request without credentials it exposes every header, `Accept-Ranges` included, which is the one name this ADR keeps from cross-origin scripts. |

## References

- research: DECISIONS R34 (the maintainer's answer of 2026-11-15, how pdf.js and the old site's service worker meet, backend1's headers and the `ngsw-bypass` check)
- Verseghy/website_k8s#87 (the media server answers `/storage` on both public hosts); Verseghy/website_k8s#83 (the comment-stripped copy of `nginx.conf`)
- pdf.js, `src/display/network_utils.js`, `validateRangeRequestCapabilities` (<https://github.com/mozilla/pdf.js/blob/master/src/display/network_utils.js>, read 2026-11-15)
- Fetch Standard, "CORS-safelisted response-header name" and the CORS protocol's `Access-Control-Expose-Headers` (<https://fetch.spec.whatwg.org/#cors-safelisted-response-header-name>, read 2026-11-15)
- website_frontend@f74c765:apps/frontend/ngsw-config.json; website_frontend@f74c765:apps/frontend/src/app/modules/misc/modules/diaktoll-viewer/components/viewer/viewer.component.html (the service worker's configuration and the Diáktoll viewer, as ADR 0015 cites them)
- `@angular/service-worker` 14.2.12, the version that website_frontend@f74c765:yarn.lock pins: `ngsw-worker.js`, `AssetGroup.handleFetch` and `needToRevalidate` (a cached copy is answered first and refetched in the background once it is older than its `max-age`; read 2026-11-15)
- `deploy/media/nginx.conf`, `src/domain/storage_rules.rs` (`EXPOSE_HEADERS`), `tests/media_contract/` and `deploy/media/README.md`
- Amended: [ADR 0015](0015-media-separate-static-server-media-library-image-optimisatio.md) §1 (the CORS row and the nginx sketch) and "What must be true in code and tests"
- Related: [ADR 0001](0001-record-architecture-decisions.md) §6; [ADR 0013](0013-rich-text-editor-and-html-sanitiser.md) §7 (the other `/storage` header rows, unchanged); [ADR 0033](0033-image-optimisation-ssim-floor-reference-parsing-cpu-limits-a.md)
