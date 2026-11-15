# 0044. Cookie-authenticated admin writes need the admin Origin

- Status: Accepted (2026-11-15)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to the decision-register item D08 (research: DECISIONS R9), which settles how [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §3's `Origin` check treats a write without an `Origin` header, and the answers of 2026-11-15 to the third decision register's items D151 (research: DECISIONS R35 (D151)), which accepts this ADR with the six readings that the `auth-origin-check` slice built where §3 is silent (section 7), and D152 (research: DECISIONS R35 (D152)), which leaves a sign-in without a credential cookie outside the check and has section 2 say why. It amends the third bullet and the Bearer exemption of ADR 0006 §3 and one test item of ADR 0006's Consequences, and the flowchart and the status codes of [ADR 0007](0007-authorization-four-roles-enforced-on-every-operation.md) §5 (section 1). The Context quotes the three answers ([ADR 0001](0001-record-architecture-decisions.md) §6).

## Context

ADR 0006 §3 makes a check of the `Origin` header the control against pages on sibling hosts: "backend3 therefore rejects any cookie-authenticated `POST`, `PUT`, `PATCH` or `DELETE` whose `Origin` header names a different origin (403). Against cross-site requests this check is a second layer. Against sibling hosts it is the control, so it is mandatory." It says nothing about a write that carries no `Origin` header, and [ADR 0014](0014-one-host-backend3-microshift-verseghy-gimnazium-net.md) leaves that case to it ("ADR 0006 §3 is the authoritative definition of this check, including how a request without an `Origin` header is handled").

| Fact | Value | Source |
|---|---|---|
| What `SameSite` covers | Browsers treat `verseghy-gimnazium.net` and every `*.verseghy-gimnazium.net` name as one site, so a page on a sibling host makes the browser send the `SameSite=Strict` session cookie with its requests to the admin. | ADR 0014, "Site boundary"; ADR 0006 §3 |
| Pages on sibling hosts | The public Angular site on `https://verseghy-gimnazium.net`, the legacy hosts `backend.microshift.verseghy-gimnazium.net` and `backend2.microshift.verseghy-gimnazium.net`, and any other `*.microshift.verseghy-gimnazium.net` name. | ADR 0014 |
| The admin's origin | `https://backend3.microshift.verseghy-gimnazium.net`, which production also sets as `HOST`; staging sets `HOST` to its own host. | ADR 0014, "Configuration and the Angular switch" |
| When browsers send `Origin` | A browser appends `Origin` to every request whose method is neither `GET` nor `HEAD`, and to every CORS request. For a same-origin write it sends the page's origin, unless the page's referrer policy is `no-referrer`, which makes it `null`. | WHATWG Fetch standard, "append a request `Origin` header" |
| The admin's referrer policy | Every `/admin` page is served with `Referrer-Policy: strict-origin-when-cross-origin`, so its writes to the same origin carry the admin origin. | [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §6; `src/controllers/admin_ui.rs` |
| Scripts | Scripts authenticate with a Bearer token from `POST /api/admin/auth/token`, and "requests with a Bearer token are exempt from the `Origin` check, because browsers never attach Bearer tokens on their own". | ADR 0006 §3 |
| Where Loco reads the token | `auth.jwt.location` reads the session cookie `__Host-admin_session` first, then the Bearer header. The two-factor step reads its own cookie `__Host-admin_mfa`, and "the `Origin` check of section 3 applies to it". | ADR 0006 §1, §4 |
| How admin routes are mounted | Every `/api/admin` route is one entry of the admin route table, whose guard runs before each handler. | [ADR 0007](0007-authorization-four-roles-enforced-on-every-operation.md) §5 |

The maintainer answered on 2026-10-25, with the recommended option (research: DECISIONS R9). The register's question, the answer, and the consequence the register gave for that option (research: decision register D08):

- **D08**, "When a cookie-authenticated POST, PUT, PATCH or DELETE carries no Origin header, should it be rejected (Bearer stays exempt), allowed, or checked against Referer instead?": "Reject (Bearer exempt)". Its consequence: "403 origin_mismatch with no exception. The admin is unaffected, and a client that strips Origin fails closed."

Building the check, the `auth-origin-check` slice had to read ADR 0006 §3 where it is silent, in six places (section 7), and it left a first sign-in, which carries no credential cookie, outside the check (section 2). The third decision register asked the maintainer about both, and the maintainer answered on 2026-11-15 (research: DECISIONS R35), each with the recommended option. The register's questions, the answers, and the consequences the register gave for those options (research: third decision register D151, D152):

- **D151**, "ADR 0044 records your D08 answer ('Reject (Bearer exempt)') and amends ADR 0006 §3 and ADR 0007 §5. It also proposes six readings, numbered 1 to 6 below, for the places where ADR 0006 §3 is silent. Do you accept it with all six as built, accept it with some of them changed, or hold it until you have read the PR?": "Accept ADR 0044 with all six readings as built". Its consequence: "No code change. ADR 0044 becomes `Accepted (<date>)` and quotes this answer. ADR 0006's status line names it next to 0040, ADR 0007's line becomes `Amended by 0044 (<date>)`, both ADR index rows follow, and the list of acceptance dates in docs/design/README.md gets a sentence for 0044 (ADR 0001 §6). Once authz-route-table has landed, the branch is rebased onto master, keeping master's 0043 index rows. The cookie writes in tests/requests/auth_activity.rs (#300), and in tests/requests/auth_refresh.rs if auth-refresh lands first, get `Origin: ADMIN_ORIGIN`. Then its PR opens and merges. auth-bearer-bcrypt (wave 9) tests reading 1 with the tokens it issues, and totp-login-step (wave 14) adds no check of its own."
- **D152**, "Should `POST /api/admin/auth/login` also need the admin Origin when it carries no credential cookie, as on a first sign-in, or should the two conditions that keep it safe without one be written down and tested? Today it is not checked (ADR 0044 §2), because ADR 0006 §3 and D08 cover only cookie-authenticated writes. No page on another origin can post a working sign-in today, so this is defence in depth against login CSRF, not a fix for an open hole.": "Leave it unchecked, and write down and test why". Its consequence: "No code change. Before it is pushed, auth-origin-check adds three things. (1) A config-file test that no committed config, production.yaml included, turns on Loco's `cors` middleware. The preflight itself is already pinned by `routes_outside_the_public_api_never_send_cors_headers`, under config/test.yaml. (2) The 422 test sends the right credentials form-encoded and as `multipart/form-data`, the two other types an HTML form can send. (3) ADR 0044 §2 gets one sentence: other pages cannot sign in because the sign-in reads only JSON and `/api/admin` answers no preflight, so neither may change without the sign-in getting the Origin check. If the answer comes after ADR 0044 is accepted, the tests land in auth-bearer-bcrypt (wave 9) and the sentence goes in an editorial docs PR, as D84 did."

Both answers came together, so the three additions of D152 land with this ADR.

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0006 §3, third bullet: "backend3 therefore rejects any cookie-authenticated `POST`, `PUT`, `PATCH` or `DELETE` whose `Origin` header names a different origin (403)." | A write that carries no `Origin` header is refused too, and so is every `Origin` other than exactly the admin origin. Sections 2 to 6 say what "cookie-authenticated", "write" and "the admin origin" mean, where the check runs and what it answers. | 2-6 |
| ADR 0006 §3, the last sentence of the fourth bullet: "Requests with a Bearer token are exempt from the `Origin` check, because browsers never attach Bearer tokens on their own." | A request with a Bearer token is exempt only when it carries no credential cookie. One that carries the session cookie or the two-factor cookie beside its Bearer header is checked like any cookie write, because Loco reads its token from the cookie first. | 2 |
| ADR 0006, Consequences, the test item "the `Origin` check rejects cookie writes from another origin, including a sibling host such as `https://verseghy-gimnazium.net`, and ignores Bearer requests" | It also rejects a cookie write without an `Origin` header and one with a Bearer header beside the cookie, ignores Bearer requests without a credential cookie, and the test walks every write route of the route table. | Consequences |
| ADR 0007 §5, the flowchart, and the Status codes bullet "No valid session: 401." | The guard runs the `Origin` check before the admin extractor, for every route of the table. A checked write that fails it is answered `403 origin_mismatch` whatever its session, so a cookie write without a valid session and without the admin origin gets that `403`, not the `401`. Every other request without a valid session still gets `401`, and the rest of the flowchart holds in its order. | 5 |

Everything else in ADR 0006 §3 and ADR 0007 §5 holds as written, the Bearer exemption of requests that carry no credential cookie included.

### 2. Which requests are checked

A request is checked when both hold:

- **It is a write.** Its method is not one of the safe methods of RFC 9110 §9.2.1 (`GET`, `HEAD`, `OPTIONS`, `TRACE`). For the methods an admin route can be declared with, these are exactly `POST`, `PUT`, `PATCH` and `DELETE`; any other method counts as a write as well, so the rule fails closed.
- **It carries a credential cookie.** Its `Cookie` headers hold `__Host-admin_session` or `__Host-admin_mfa`, with any value, as Loco's cookie reader finds them (also beside other cookies, in a second `Cookie` header, or percent-encoded). A request that carries one is checked whatever else it carries: with a Bearer header beside the session cookie, Loco reads the token from the cookie.

Not checked: a read, which changes nothing and whose answer a page on another origin cannot read without CORS headers (ADR 0014); and a request without a credential cookie, such as a script's Bearer request (ADR 0006 §3) or a first sign-in, which carries no session that another page could borrow.

Other pages cannot sign in all the same, because the sign-in reads only JSON and `/api/admin` answers no CORS preflight, so neither may change without the sign-in getting the `Origin` check (D152): a body of any other type gets `422 invalid_body` before any lookup (research: DECISIONS R15 (D74)), and a page on another origin, a sibling host included, can send JSON only after a preflight, since CORS goes by origin, not by site (ADR 0014).

### 3. What passes

A checked request passes only when it carries exactly one `Origin` header whose value is, byte for byte, the admin origin (section 4). Everything else is refused (D08):

- no `Origin` header;
- `Origin: null`, which browsers send for opaque origins such as sandboxed frames;
- another origin: another host of the same site, another scheme or another port of the admin's host;
- a spelling that browsers never send, such as a trailing slash, an upper-case letter or the default port;
- a list of origins in one header, and two `Origin` headers.

There is no fallback to `Referer` (D08's third option).

### 4. The admin origin

The admin origin is the typed setting `settings.admin_auth.origin` ([ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md), "Typed configuration"). It is checked at boot to be an `http` or `https` origin exactly as browsers serialise it in `Origin`: the scheme, the host in lower case (an internationalised name in its `xn--` form, an IPv6 address in brackets), a port only when it is not the scheme's default, no path and no trailing slash (a dot segment such as `/.` and a backslash count as a path, although the URL parser drops or rewrites them). A value that is not stops every command, with the key, the environment variable and the spelling to write: no header could match it, so it would refuse every admin write.

| Environment | Value |
|---|---|
| Production, staging | `HOST`, the variable that `server.host` reads, so the two cannot drift apart and no deployment variable is added. Production's is `https://backend3.microshift.verseghy-gimnazium.net` (ADR 0014); staging's is its own host. |
| `e2e`, `test` | `http://localhost:5150`, the E2E server from which Playwright loads the admin. |
| `development` | `http://localhost:<PORT>`, 5150 by default: the server that `cargo loco start` runs, which serves the built admin at `/admin`. The Vite dev server's translation of section 5 makes its pages pass too. |

### 5. Where the check runs, and what it answers

- **In the admin route table's guard, first.** It runs for every route of the table, the `Public` sign-in steps included, before the guard opens a session (ADR 0007 §5): the `Origin` check, then `401 unauthorized`, then `403 forbidden`, then the handler. A refused write therefore gets the same answer whether its session is valid or not, costs no database read, and never reaches a handler or reads its body. Every admin route that a later slice adds is covered, because the table is the only way to mount one. The guard wraps each route's handler after routing, so a path the table does not declare keeps its `404`, and a method a route does not declare keeps its `405`.
- **The answer** is `403` with the admin error envelope and the new code `origin_mismatch` ([ADR 0021](0021-hungarian-only-admin-ui.md) §4), `Cache-Control: no-store`, and no `Set-Cookie`: nothing is done, and the session stays as it was. It is a `403`, not a `401`, so the admin UI does not sign the user out (ADR 0006, follow-up work); it shows the Hungarian `errors.origin_mismatch`, which asks the user to open the admin at its usual address.
- **The log.** backend3 logs each refusal at `WARN`, with the method, the path, the reason (no header, two headers, another origin), at most 120 characters of the received `Origin` and the expected origin. The admin's own writes always pass, so a refusal is a page on another origin, a client or browser extension that drops the header, or an admin origin that does not match where the admin is served, which refuses every write and must be seen at once.
- **A context without the admin origin**, which only a context built by hand can be, answers `500` to a checked write instead of letting it through.

### 6. The Vite dev server in development

In development the Vite dev server (`npm run dev`) serves the admin on its own origin and proxies `/api` to backend3 ([ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) §4). Its proxy gives a request whose one `Origin` is the dev server's own (`<scheme>://<Host>`) the origin of the backend it proxies to, and forwards every other request as it came: a page on any other origin, a sibling port of `localhost` included, is refused by backend3 as in production, and a request without `Origin` gets none. Vite's own `allowedHosts` check refuses an unknown `Host` before the proxy runs. Production serves the admin from backend3 itself, so nothing translates an origin there.

### 7. Readings beyond D08, accepted as built (D151)

The answer to D08 settles the missing header. The slice also had to read ADR 0006 §3 where it is silent, in the six places below, numbered as the register numbers them. The maintainer accepted all six as built on 2026-11-15 (research: DECISIONS R35 (D151)):

1. "Cookie-authenticated" means the request carries a credential cookie (section 2), the two-factor cookie included, whatever other header it carries; so a Bearer request is exempt only without one (section 1). A sign-in without one is not checked either, for the reasons section 2 gives (D152).
2. A write is every method that is not safe (section 2), which for the admin routes is exactly §3's four.
3. The comparison is byte for byte with the admin origin, and `null`, a list and two headers are refused (section 3).
4. The admin origin is `settings.admin_auth.origin`, which production and staging read from `HOST` (section 4).
5. The check runs first in the route table's guard, for `Public` routes too, and answers `403 origin_mismatch`, so a refused write without a valid session gets that `403` rather than ADR 0007 §5's `401` (sections 1 and 5).
6. The Vite dev server translates the origin of its own pages only (section 6).

## Consequences

Positive:

- A page on a sibling host cannot use the admin's session to write, whatever the browser sends or leaves out; a client that drops `Origin` fails closed, as D08 chose.
- The admin's own writes always pass: its pages send their origin under the referrer policy they are served with, which a test pins.
- Scripts are unaffected: a Bearer request without a credential cookie is not checked.
- Every write route of the table is covered from the moment it is declared, without a line of its own.

Negative:

- A client that sends the session cookie but strips `Origin` (an unusual proxy, a privacy extension) cannot write. Such a user has to use the admin without it, or a script uses a Bearer token.
- A wrong `HOST` refuses every cookie-authenticated write. One that is not an origin stops the boot; one that is a valid origin but not where the admin is served shows only as `WARN` lines and `origin_mismatch` answers.
- `/admin` pages must keep a referrer policy under which same-origin writes carry their origin; `no-referrer` would refuse every write of the admin.
- A sign-in without a credential cookie is not checked, so it is safe from a forged sign-in on another page (login CSRF) only while it reads JSON alone and `/api/admin` answers no preflight (section 2, D152). A change to either gives the sign-in the `Origin` check in the same PR, which needs an amending ADR; the tests below fail on the changes they can see.

Follow-up work:

- The two-factor step (`POST /api/admin/auth/totp`) reads `__Host-admin_mfa` itself (ADR 0006 §4); it is a route of the table, so this check already covers it, and its slice adds no check of its own.
- The Bearer token endpoint's slice tests the Bearer exemption with a token that endpoint issues.

What must be true in code and tests:

- The rule is pure, in `src/domain/origin.rs` ([ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md)): the admin origin's parser, the write methods, which requests are checked and the comparison, with unit tests and properties.
- The request tests check, through the whole middleware stack: against production's origin, `https://verseghy-gimnazium.net` and the other sibling hosts and spellings refused and the admin origin accepted; a cookie write without `Origin`, with `null` and with two headers refused; every write route of the route table walked, and a probe table with each method and requirement; the check before the session and the requirement; reads and writes without a credential cookie not checked; a path or method the table does not declare keeping its `404` or `405`; a context without the admin origin answering `500`.
- The two conditions of section 2 under which no page on another origin can sign in (D152): a configuration test that no committed configuration, `config/production.yaml` included, turns on Loco's `cors` middleware; the request test that no route outside `/api/v1`, the sign-in included, answers a preflight with a CORS header (under `config/test.yaml`); and the sign-in's `422` test, which sends the right credentials as `text/plain`, form-encoded and as `multipart/form-data`, the three types a page can send without a preflight, none of which signs in.
- A unit test pins the refusal's `WARN` line: its fields for each reason, and the received `Origin` cut to 120 characters.
- The settings refuse an admin origin that is not one, naming the key and `HOST`, and production stops before it connects with such a `HOST`.
- An E2E test in Chromium: a page on another origin of the same site posts the logout with the cookie, gets `403`, and the session holds; the admin's own sign-out carries the admin origin.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Allow a write that carries no `Origin` header | decision register D08 | Not chosen. It tolerates unusual proxies and extensions, but a write without `Origin` would skip the only control against sibling hosts. |
| Fall back to `Referer` when `Origin` is missing | decision register D08 | Not chosen. A second header rule to parse and test; `Referrer-Policy` can strip `Referer` too, so a final rule for a missing header is still needed. |
| Check the sign-in's `Origin` too, with or without a credential cookie | third decision register D152 | Not chosen. It would still protect the sign-in if the conditions of section 2 broke, but it widens ADR 0006 §3 past the cookie writes that D08 settled, makes every client of the sign-in send an `Origin` header, and guards a request that no page on another origin can send. It can still come as an amending ADR. |
| Check every write that has no Bearer token, with or without a cookie | considered while writing this ADR | Scripts call the Bearer token endpoint without a token and without `Origin`, so they could no longer sign in. |
| A tower layer on `/api/admin` instead of a step of the guard | considered while writing this ADR | It could read the admin origin from the shared store on each request just as well (Loco runs `App::after_context` before `App::routes`), so the reason is where it would run: before routing, so it would also answer cookie writes to paths the table does not declare, which keep their `404` as a step of the guard. The guard wraps exactly the handlers of the table's routes, `Public` ones included, and orders its steps in one place, the `Origin` check before the session. |
| A separate setting or variable for the admin origin | considered while writing this ADR | ADR 0014 already makes `HOST` the admin origin; a second variable could drift from it and needs a deployment change. |

## References

- research: DECISIONS R9 (the answer to D08, 2026-10-25); the decision register, item D08
- research: DECISIONS R35 (the answers to D151 and D152, 2026-11-15); the third decision register, items D151 and D152
- research: DECISIONS R15 (D74, `422 invalid_body` for a body that is not the endpoint's JSON)
- [ADR 0006](0006-admin-authentication-and-account-lifecycle.md) §1, §3, §4; [ADR 0014](0014-one-host-backend3-microshift-verseghy-gimnazium-net.md); [ADR 0007](0007-authorization-four-roles-enforced-on-every-operation.md) §5; [ADR 0021](0021-hungarian-only-admin-ui.md) §4; [ADR 0023](0023-operations-hardening-rate-limits-monitoring-backups-secrets.md) §6; [ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md) §4; [ADR 0008](0008-code-layout-a-pure-src-domain-beside-loco-models.md); [ADR 0001](0001-record-architecture-decisions.md) §6
- WHATWG Fetch standard, "append a request `Origin` header", "serializing a request origin" and "CORS-safelisted request-header" (<https://fetch.spec.whatwg.org/>)
- RFC 6454 §7 (the `Origin` header), RFC 9110 §9.2.1 (safe methods), RFC 6265bis (`SameSite`, cookie name prefixes)
- loco-rs@1.2.0:src/controller/extractor/auth.rs (`extract_token_from_cookie`)
- `src/domain/origin.rs`, `src/controllers/admin/origin.rs`, `src/controllers/admin/routes.rs`, `admin/scripts/dev-proxy.mjs`, `tests/requests/admin_origin.rs`, `tests/requests/auth_login.rs`, `tests/requests/public_router.rs`, `tests/boot/config_files.rs`, `admin/e2e/sign-out.spec.ts`
