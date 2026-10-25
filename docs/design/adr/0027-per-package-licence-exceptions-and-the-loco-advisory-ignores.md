# 0027. Per-package licence exceptions and the Loco advisory ignores

- Status: Accepted (2026-10-25)
- Decision questions: none: records the maintainer's answers of 2026-10-25 to the decision-register items D21 and D22 (research: DECISIONS R9), which settle the licences and advisory ignores that the bootstrap PR raised for confirmation. It amends [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §4, the licence policy of the PR checks (Q34b), and two sub-bullets of [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4, which records the licence (Q14).

## Context

[ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §4 sets backend3's licence and advisory policy, and [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4 summarises it. These are the parts this ADR touches:

| Rule | Where | What it says |
|---|---|---|
| The Rust allow-list | ADR 0022 §4, the `[licenses]` row | `MIT`, `Apache-2.0`, `ISC`, `BSD-2-Clause`, `BSD-3-Clause`, `Zlib`, `BSL-1.0`, `MPL-2.0`, `Unicode-3.0` |
| The Rust exceptions | ADR 0022 §4, the `[licenses]` exceptions row; ADR 0012 §4 | "`CDLA-Permissive-2.0` for `webpki-roots` only". ADR 0012 §4: "`cargo-deny` uses backend2's licence allow-list, the `webpki-roots` exception, the `openssl` ban and the source rules." |
| Adding a licence | ADR 0022 §4, rules | It "needs a PR that states that reason". |
| The npm list | ADR 0022 §4, `Admin licenses`; ADR 0012 §4 | "It fails on any license outside the list and on any package without one." "The list follows the same rule as `deny.toml`." Its initial content comes from the real tree, in the bootstrap PR. ADR 0012 §4: the job "fails when a production npm dependency has a licence outside the npm allow-list". |
| Advisory ignores | ADR 0022 §4, the `[advisories]` row | "every `ignore` entry needs a comment and a linked issue" |

The bootstrap PR (Verseghy/website_backend3#186, merged on 2026-10-24) wrote `deny.toml` and `admin/license-policy.json` from the real dependency trees. Those trees need six licence entries beyond the table. #186 listed them "for the maintainer to confirm", each with its reason, and Verseghy/website_backend3#195 item 1 collected them for the final review (PLT-36):

| File | Licence | Package, at `ad496da` | How it arrives | As built |
|---|---|---|---|---|
| `deny.toml` | CC0-1.0 | `notify` 8.2.0 | a non-optional dependency of loco-rs 1.2.0 | an exception for this crate |
| `deny.toml` | 0BSD | `quoted_printable` 0.5.2 | through `lettre` 0.11.23, a non-optional dependency of loco-rs 1.2.0 | an exception for this crate |
| `admin/license-policy.json` | 0BSD | `tslib` 2.8.1 | a production dependency, through cmdk's dialog and the `shadcn` package's tooling; the bundle built from `ad496da` holds none of its code | on `allow`, for every package |
| `admin/license-policy.json` | BlueOak-1.0.0 | `isexe` 3.1.5, `minimatch` 10.2.6 | build tooling of the `shadcn` package | on `allow`, for every package |
| `admin/license-policy.json` | OFL-1.1 | `@fontsource-variable/inter` 5.3.0 | the font of the kit's preset; its files ship in `admin/dist` | an exception for this package |
| `admin/license-policy.json` | CC-BY-4.0 | `caniuse-lite` 1.0.30001819 | data that browserslist reads at build time; none of it is bundled | an exception for this package |

The two files scoped the extra licences differently. `deny.toml` admits each one for a single crate. `admin/license-policy.json` put 0BSD and BlueOak-1.0.0 on its global `allow` list, although it supports per-package `exceptions` and uses them for Inter and caniuse-lite. Any later npm package under either licence would have entered without review. The installed admin tree already holds nine more BlueOak-1.0.0 packages, all of them development-only today, for example glob 13, lru-cache 11, minipass 7 and tar 7 (`website_backend3@ad496da:admin/package-lock.json`).

The bootstrap also added three advisory ignores to `deny.toml`. Each has a comment and links the upstream issues or pull requests that track its cause; #192, #193 and #194 now track them:

| Advisory | Crate, at `ad496da` | Path into the graph | Why backend3 is not affected | Issue |
|---|---|---|---|---|
| RUSTSEC-2023-0071: the Marvin attack, a timing side channel in RSA private-key operations | `rsa` 0.9.10 | loco-rs 1.2.0's `auth` feature, then jsonwebtoken 10.4.0 with its `rust_crypto` backend | backend3 signs and verifies only HS512 tokens ([ADR 0006](0006-admin-authentication-and-account-lifecycle.md)) and holds no RSA key, so the affected code never runs | #192 |
| RUSTSEC-2026-0387: timing variability in operations meant to be constant-time; fixed in 0.7.6 | `crypto-bigint` 0.5.5 | the same backend, then p256 and p384 0.13 and elliptic-curve 0.13 | backend3 does no ECDSA operation, for the same reason | #193 |
| RUSTSEC-2024-0449: unmaintained; informational, no known vulnerability | `unsafe-libyaml` 0.2.11 | serde_yaml_ng 0.10.0 (Loco's config loader) and serde_yaml 0.9.34 (loco-gen, through rrgen) | it parses only the repository's own `config/*.yaml` and seed fixtures, never request input | #194 |

None of the three has a patched release that loco-rs 1.2.0's dependency tree can use. `Cargo.toml` pins `loco-rs = "=1.2.0"`, and only a dedicated bump PR changes Loco's tree ([ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §3). A `[patch]` override inside that tree would conflict with the exact pin and with the vendored-skill check, and no fixed rsa 0.9 release exists (research: decision register D22).

The maintainer answered both questions on 2026-10-25 (research: DECISIONS R9):

- D21, "Do you accept the six licence entries from the bootstrap? In deny.toml: CC0-1.0 (notify) and 0BSD (quoted_printable). In admin/license-policy.json: 0BSD (tslib), BlueOak-1.0.0 (isexe, minimatch), OFL-1.1 (@fontsource-variable/inter) and CC-BY-4.0 (caniuse-lite). Should the two global npm entries be scoped per package?": "Accept, with 0BSD and BlueOak-1.0.0 scoped per package". The register gave that option's consequence: both licences move into `exceptions` (0BSD for tslib; BlueOak-1.0.0 for isexe and minimatch), and a dependency update that brings in another package under either licence fails `Admin licenses` until a one-line exception PR names it. It recommended the option for this reason: "All six licences are AGPL-3.0-only compatible by the cited reasoning. ADR 0022 §4 makes the npm list follow deny.toml, which admits every licence beyond the ADR's table for one named crate only, 0BSD included. Scoping makes both files follow one rule, at the cost of a one-line PR when a new package needs one of these licences." (research: decision register D21). Sections 2 and 3 state that one rule for later licences too.
- D22, "Do you accept the three advisory ignores (RUSTSEC-2023-0071 rsa, RUSTSEC-2026-0387 crypto-bigint 0.5, RUSTSEC-2024-0449 unsafe-libyaml) until a Loco bump removes them?": "Accept until a Loco bump".

Both answers took the recommended option.

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0022 §4, the `[licenses]` exceptions row: "`CDLA-Permissive-2.0` for `webpki-roots` only" | Three crate exceptions: that one, CC0-1.0 for `notify` and 0BSD for `quoted_printable`. | 2 |
| ADR 0022 §4, `Admin licenses`: "It fails on any license outside the list and on any package without one." | It fails on any licence that neither `allow` nor an exception for that package admits, and on any package without one. | 3 |
| ADR 0022 §4, `Admin licenses`: "The list follows the same rule as `deny.toml`. Its initial content is set in the bootstrap PR from the real dependency tree" | The rule is made exact: `allow` holds only licences of the `[licenses]` row, and every other licence is an exception for named packages. Section 3 lists the five exceptions. | 3 |
| ADR 0022 §4, the `[advisories]` row | The three ignores above are accepted until the Loco bump that removes their cause. | 4 |
| ADR 0012 §4: "`cargo-deny` uses backend2's licence allow-list, the `webpki-roots` exception, the `openssl` ban and the source rules." | It uses the crate exceptions of section 2, not the `webpki-roots` exception alone. | 2 |
| ADR 0012 §4: "The admin CI job fails when a production npm dependency has a licence outside the npm allow-list" | It fails when the licence is outside the allow-list and outside the exceptions for that package. | 3 |

Everything else in both ADRs holds as written. That includes the `[licenses]` allow row, `confidence-threshold = 1.0`, `[licenses.private]`, the bans and the source rules, the `Admin licenses` tool, and the rule that adding a licence needs a PR that states the reason. It also includes ADR 0012 §4's "Both allow-lists contain only permissive licences and MPL-2.0": the `allow` lists hold only licences of the table. The exceptions are not on them, and each one states its own reason in section 2 or 3.

### 2. Rust: crate exceptions in `deny.toml`

| Licence | Crate | Reason |
|---|---|---|
| CDLA-Permissive-2.0 | `webpki-roots` | Unchanged from ADR 0022 §4: a short permissive data licence, for the Mozilla CA root data. |
| CC0-1.0 | `notify` | The FSF list calls CC0 a public domain dedication with "a lax, permissive license as a fallback" and says that both "are compatible with the GNU GPL". It does not recommend CC0 for software, because CC0 "does not grant you any patent licenses", and it encourages users "to be careful about using software under this license". That is a caution about patents, not a bar to combining CC0 code with AGPL-3.0-only code. D21 accepts this exception. |
| 0BSD | `quoted_printable` | The FSF list names the "Zero-clause BSD license" among the GPL-compatible free software licences. It grants every permission with no condition, so it does not require keeping the copyright and licence notices. The FSF adds that "if the copyright notice or license notice is removed from the program, it is not possible to safely rely on the license". crates.io delivers `quoted_printable` with its `LICENSE` file. |

Each exception holds for its crate only. A licence that is not on ADR 0022 §4's `[licenses]` allow row never goes on `allow`.

The bootstrap PR (#186) and its `deny.toml` comment read the FSF's advice on CC0 as being only about choosing a licence for one's own program. The FSF list also cautions users of CC0 software, as quoted above, and the `deny.toml` comment now says so.

### 3. npm: the same rule as `deny.toml`

1. `allow` in `admin/license-policy.json` admits a licence for every package. It lists only licences of ADR 0022 §4's `[licenses]` row; today these are MIT, ISC, BSD-2-Clause, BSD-3-Clause, Apache-2.0 and MPL-2.0.
2. Every other licence is an `exceptions` entry with one `package` and a reason. A licence that several packages need gets one entry per package. An entry names a package, not a version, so an update of that package under the same licence passes.
3. No licence is in both. `validatePolicy` rejects an exception for a licence that `allow` admits, because such an exception would never be relied on, and the message says which entry to remove.
4. When a package fails, the `Admin licenses` message names this route: replace the dependency, or add an `exceptions` entry for that package in a PR that states the reason. It also says that `allow` takes only the licences of the `[licenses]` row.

The exceptions:

| Licence | Package | Reason |
|---|---|---|
| 0BSD | `tslib` | As for `quoted_printable` in section 2. npm delivers tslib with its `LICENSE.txt` and the notice at the top of each file. The minified admin JavaScript keeps no source comments, so tslib code that reaches `admin/dist` would not carry that notice there. The bundle built from `ad496da` holds none of its code. |
| BlueOak-1.0.0 | `isexe` | The Blue Oak Model License 1.0.0 is OSI-approved and permissive. The FSF list does not name it, so the reasoning is given here, as ADR 0022 §4 does for Unicode-3.0. It grants copyright and patent permission on one condition: everyone who gets a copy also gets the licence text or a link to it. That is the same notice duty as MIT, so nothing in it conflicts with AGPL-3.0-only. |
| BlueOak-1.0.0 | `minimatch` | As for `isexe`. |
| OFL-1.1 | `@fontsource-variable/inter` | The FSF list calls the SIL Open Font License 1.1 "a free copyleft license for fonts". The font files ship in `admin/dist` as separate works served beside the program, an aggregate under AGPL-3.0 §5, not combined with it. The scope keeps code from arriving under a font licence. |
| CC-BY-4.0 | `caniuse-lite` | The FSF list says CC BY 4.0 "is compatible with all versions of the GNU GPL". It is a licence for data, which browserslist reads at build time; none of it is bundled. |

A dependency update that brings in another package under one of these licences fails `Admin licenses` until a PR adds an exception that names the package and states its reason.

### 4. Advisory ignores until the Loco bump

- RUSTSEC-2023-0071, RUSTSEC-2026-0387 and RUSTSEC-2024-0449 stay ignored until the Loco bump that removes their cause. Each keeps its comment, its `reason`, its upstream links and its tracking issue (#192, #193, #194).
- The Loco bump is the dedicated PR of ADR 0024 §3 (`build(api): bump loco-rs to X.Y.Z`). It runs `cargo deny check advisories`, which reports an ignore whose advisory no longer occurs in the graph as `advisory-not-detected` (cargo-deny 0.20.2). The bump removes each such entry and closes its tracking issue with `Closes #N`. An entry whose cause the bump keeps stays, and its issue stays open.
- No `[patch]` override, fork or vendored copy inside loco-rs's tree removes an entry before that.
- This acceptance covers these three entries only. A later ignore follows ADR 0022 §4 as written.

### 5. Where the files cite this ADR

- `deny.toml`: the comments of the CC0-1.0 and 0BSD exceptions, the `[advisories]` comment and each ignore entry. The file header states the rule of section 2.
- `admin/license-policy.json`: the `$comment` states the rule of section 3, and the reason of every exception cites this ADR.
- `admin/CLAUDE.md`, in its dependency rule, and `THIRD_PARTY.md`.

## Consequences

**Positive**

- `deny.toml` and `admin/license-policy.json` follow one rule: a licence beyond ADR 0022 §4's table enters for named crates or packages only. #195's "scoped as narrowly as the tree allows" now holds in both files.
- The final review (PLT-36) finds the accepted licences, their scopes and reasons, and the accepted ignores in one Accepted ADR rather than in a PR body.
- The validator stops an exception from looking scoped while `allow` still admits its licence for every package.

**Negative**

- A dependency update that brings 0BSD or BlueOak-1.0.0 into another production package fails `Admin licenses` until a one-line exception PR names it. BlueOak-1.0.0 is common in npm tooling: nine development-only packages of today's tree use it.
- The three ignores stay for as long as loco-rs 1.2.0 is the pin. When they go depends on upstream releases (the upstream issues and pull requests linked in `deny.toml`). Until then, `cargo-deny check advisories` passes with them; a new advisory still fails it.
- #192, #193 and #194 stay open as reminders.
- 0BSD does not require keeping notices, and the minified admin JavaScript keeps none, so tslib code that reaches `admin/dist` would carry no notice there (section 3). This ADR does not decide how `admin/dist` carries the notices of bundled packages.

**Follow-up work, in the same PR as this ADR**

- `admin/license-policy.json`: 0BSD and BlueOak-1.0.0 leave `allow` and become the three exceptions of section 3.
- `admin/scripts/license-policy.mjs`: `validatePolicy` rejects an exception for a licence that `allow` admits, and the failure message names the per-package route (section 3, item 4).
- `deny.toml`, `admin/CLAUDE.md` and `THIRD_PARTY.md` cite this ADR (section 5).
- The Status lines of ADR 0022 and ADR 0012, both ADR indexes and the overview's ADR list.

**What must be true in code and tests**

- `npm run licenses` passes on the installed tree.
- `admin/scripts/license-policy.test.mjs` checks:
  - that `deny.toml`'s `allow` equals ADR 0022 §4's table, and that the committed policy's `allow` holds only licences of that table;
  - that each licence of section 3 passes for its named packages and fails for any other package, with a message that names an `exceptions` entry for the failing package;
  - that `validatePolicy` rejects an exception for a licence on `allow`.
- `cargo deny check bans licenses sources` and `cargo deny check advisories` pass.
- Both checks warn, without failing, about an exception that no package needs any more: cargo-deny 0.20.2 as `license-exception-not-encountered`, and `npm run licenses` as an exception that "matches no installed package". So the PR that drops such a package sees which entry to remove.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Accept all six as built | decision register D21 | Not chosen: 0BSD and BlueOak-1.0.0 would stay global in the npm list, so any later package under them would enter without review, unlike in `deny.toml`. |
| Accept, but move the build-only packages to `devDependencies` | decision register D21 | Not chosen: BlueOak-1.0.0 and CC-BY-4.0 might leave the gated tree, but `Admin licenses` would then stop covering packages that feed the bundle at build time. |
| Reject an entry | decision register D21 | Not chosen: that package would have to be replaced. For `notify` or `quoted_printable` this means a Loco change, which the `=1.2.0` pin and ADR 0024 §3 allow only in a dedicated bump. |
| Patch the dependency tree now | decision register D22 | Not chosen: `[patch]` overrides inside loco-rs's tree conflict with the exact pin and the vendored-skill check (ADR 0024 §3), and no fixed rsa 0.9 release exists. |

## References

- research: DECISIONS R9, items D21 and D22 (answers of 2026-10-25)
- research: decision register D21 and D22 (the questions, their context and the options)
- Verseghy/website_backend3#186 (the bootstrap PR, "Licences beyond ADR 0022 section 4's table"), Verseghy/website_backend3#195 item 1; the tracking issues Verseghy/website_backend3#192, #193 and #194
- FSF, "Various Licenses and Comments about Them" (Zero-clause BSD, CC0, CC BY 4.0, SIL OFL 1.1): <https://www.gnu.org/licenses/license-list.html>
- Blue Oak Model License 1.0.0: <https://blueoakcouncil.org/license/1.0.0>; SPDX License List 3.29.0 (BlueOak-1.0.0 and 0BSD are OSI-approved): <https://spdx.org/licenses/>
- RustSec: <https://rustsec.org/advisories/RUSTSEC-2023-0071.html>, <https://rustsec.org/advisories/RUSTSEC-2026-0387.html>, <https://rustsec.org/advisories/RUSTSEC-2024-0449.html>
- cargo-deny@0.20.2:src/advisories/diags.rs (`advisory-not-detected`), src/licenses/diags.rs (`license-exception-not-encountered`), and src/advisories/cfg.rs and src/licenses/cfg.rs (both lints default to `warn`)
- website_backend3@ad496da:Cargo.lock and admin/package-lock.json (the versions in Context)
- The admin bundle built from `ad496da` with `vite build` (Vite 8.3.5) on Node.js 24: no tslib code, and no source comments in the minified JavaScript
- Amended: [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §4; [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4, two sub-bullets
- Related: [ADR 0006](0006-admin-authentication-and-account-lifecycle.md), [ADR 0024](0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md) §3
