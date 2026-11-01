# 0042. Licence notices of the crates linked into the backend3 binary

- Status: Accepted (2026-11-01)
- Decision questions: none: records the maintainer's answers of 2026-11-01 to the third decision register's items D115 (research: DECISIONS R23), which [ADR 0038](0038-licence-notices-of-the-bundled-admin-packages.md) §3 quotes and leaves to the `binary-licence-notices` slice, and D125 (research: DECISIONS R26), which confirms as built the six places where the build reads D115 beyond its wording (§5), as D114 did for ADR 0038. §5 quotes D125 ([ADR 0001](0001-record-architecture-decisions.md) §6). It amends one bullet of [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4, two rows of [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §2, and two rows of ADR 0022 §3, one of them the `Toolchain` row as [ADR 0029](0029-no-filters-or-job-conditions-on-required-ci-jobs.md) §3 gives it.

## Context

ADR 0012 §4 says that dependencies "are checked, not listed". ADR 0038 added the npm packages bundled into `admin/dist`, whose notices every admin build writes to `admin/dist/licenses.txt`. It left out the backend3 binary, which the image ships too, and recorded the maintainer's answer to D115 for it (ADR 0038 §3).

| Fact | Value | Source |
|---|---|---|
| Who receives the binary | Pipelines-as-Code pushes an image on every `master` push, and anyone can pull it. The runtime image holds `/app/backend3-cli`, `config/`, `admin/dist` and a CA bundle; until this ADR, no notice of the code compiled into the binary. Site visitors receive the admin's JavaScript, never the binary. | `Containerfile`; ADR 0022 §1; research: third decision register D115 |
| What the binary links | `cargo tree -e normal,no-proc-macro --target x86_64-unknown-linux-gnu` lists 417 third-party crates, and Rust's standard library is linked as well, outside `Cargo.lock`. The rustc component of the toolchain installs the standard library's notices as `share/doc/rust/COPYRIGHT-library.html` (1.5 MB for 1.99.1), also under rustup's minimal profile. | this branch's `Cargo.lock`; the rustc component's manifest of Rust 1.99.1 |
| What their licences ask | MIT, ISC, Unicode-3.0 and the BSD licences ask that the copyright and permission notice go with all copies or substantial portions; the binary-form clause of BSD-3-Clause names the documentation of the binary. Apache-2.0 §4(d) asks that the attribution notices of a crate's `NOTICE` file be passed on; five linked crates ship one (`moka` and four `opendal` crates). MPL-2.0 §3.2 asks that the recipients of an executable be told how to get the source (`selectors`, `cssparser` and other crates). The IJG licence asks the documentation of a binary to say that it "is based in part on the work of the Independent JPEG Group". | the licence texts; the crate sources of this branch's `Cargo.lock`; `THIRD_PARTY.md` |
| Bundled C code | Three `-sys` crates compile C libraries under BSD-3-Clause: libwebp in `libwebp-sys`, which declares only MIT (`deny.toml` clarifies it), zstd in `zstd-sys` and libjpeg-turbo in `mozjpeg-sys`. `libdeflate-sys` declares Apache-2.0 and compiles libdeflate, whose `libdeflate/COPYING` is MIT. | the crate sources; `deny.toml`; [image-encoders.md](../image-encoders.md) |
| The tool | cargo-about 0.9.3 (2026-10-26) renders a Handlebars template from `Cargo.lock` and a `about.toml` (accepted licences, targets, `ignore-build-dependencies`, `ignore-dev-dependencies`, `private.ignore`, per-crate `accepted` and `clarify`), with `--locked`, `--offline` (`--frozen` is both) and `--fail`. Its release assets include a static Linux musl archive with a `.sha256` file (`3ad1e94e...f48f71b4` for x86_64); taiki-e/install-action v2.88.0 installs the same archive with the same hash. Its documentation and changelog name no support for `NOTICE` files. | cargo-about 0.9.3: `docs/src/cli/generate/`, `CHANGELOG.md`, release assets; install-action@86f7a0f:`manifests/cargo-about.json` |
| How it reads licences | It scans every file of each crate and keeps a file as a licence text when its detector recognises the licence with a confidence of at least 0.8, source-file headers included (ring's ISC headers, for example). When no file of a crate matches a licence of its expression, it prints the standard text of that licence, which names no copyright holder, at debug level only. A clarification names the files and their SHA-256; when a checksum stops matching, cargo-about warns and falls back to scanning, and `--fail` does not fail; it rejects the whole clarification of that crate version when any one of its files fails. It prints each distinct licence text once, for every crate whose file holds that text, under the first crate's file name. It cannot tell a procedural macro from a linked crate. | cargo-about 0.9.3: `src/generate.rs`, `src/licenses.rs` (`apply_clarification`), `src/licenses/scan.rs`, `src/lib.rs` (`get_all_crates`) |
| What it gives here | With the release target and without build and test dependencies, it lists 481 crates: the 417 linked ones, plus 64 procedural macros and crates only they use. Before any clarification, 40 crates got only standard texts. 14 of them ship a licence file that the detector does not recognise (a combined MIT and Apache-2.0 file, a narrow wrapping, a portions notice, an Apache text without its appendix), so their copyright lines would have been lost. The other 26 ship no licence text at their root: no licence file, or a one-line file such as `../LICENSE-MIT` where packaging flattened a link (`chrono-tz` 0.9.0, `sqlx-core`, `sqlx-mysql`, `sqlx-postgres`). One of them, `libwebp-sys`, holds the licence file of the libwebp it bundles in `vendor/`. | runs of cargo-about 0.9.3 on this branch |

After that context, the register asked (research: third decision register D115): "Should the image build generate a notices file for the binary and ship it in the image, also serve it under /admin, or leave the binary out?" The maintainer answered on 2026-11-01 (research: DECISIONS R23), with the recommended option:

- **D115**: "Generate it in the image build and ship it in the image". Its consequence: "One build slice. The rust stage downloads cargo-about's Linux musl release, pinned by version and SHA-256 like rustup-init, and after the release build writes the linked crates' notices to a file in the image, such as /app/licenses/crates.html. It works from a committed about.toml and template: the release target, no build or dev dependencies, a libwebp-sys clarification, and the standard text for the 17 crates without a licence file. The stage adds the NOTICE files, the IJG statement and the toolchain's COPYRIGHT-library.html beside it. A PR check runs the same generation with --fail, plus a check for NOTICE files the file does not carry, so a gap fails before master. An ADR amends ADR 0012 §4, as ADR 0038 does for the admin. No route changes."

The register counted 17 linked crates without a licence file; the measurement above finds 16 without one and 3 whose licence file is a flattened link, 19 in all, plus 5 procedural macros.

## Decision

### 1. What the image carries

The runtime stage copies one directory, `/app/licenses/`, owned by root and world-readable like every other file of the image (ADR 0023 §8):

| File | Content |
|---|---|
| `crates.html` | cargo-about's rendering of `about.hbs`: the IJG statement; the licences with their counts; every crate with its version, its licence expression and a link to its source on crates.io at that version (MPL-2.0 §3.2); and every distinct licence text with the crates that use it, each marked as the crate's own licence file or as the standard text, which these crates ship no text of, so it names no copyright holder. |
| `notices/<crate>-<version>/<file>` | the `NOTICE` files at the root of the listed crates, copied unchanged. |
| `COPYRIGHT-library.html` | the standard library's notices, from the toolchain that built the binary. |
| `README.txt` | what each file is, the source of backend3 itself (AGPL-3.0-only, at the image's revision label), every `NOTICE` file by path, the IJG statement, cargo-about's and rustc's versions, and where the admin's notices are. |

Nothing is served over HTTP, and nothing under `admin/dist` or `/admin` changes (D115: "No route changes").

### 2. How it is generated

- **The Containerfile's rust stage** downloads cargo-about's x86_64 Linux musl archive, pinned by `ARG CARGO_ABOUT_VERSION` and `ARG CARGO_ABOUT_SHA256` and checked with `sha256sum --check --strict` as rustup-init is, and installs only the binary. After `cargo build --release --locked`, it runs `bash scripts/crate-notices.sh --out /src/target/licenses`, and the runtime stage copies that directory. cargo-about never reaches the runtime image. The stage also installs `jq`, which the script needs.
- **`scripts/crate-notices.sh`** runs `cargo fetch --locked` (cargo metadata reads the packages of every platform), then cargo-about twice with `--frozen --fail --target x86_64-unknown-linux-gnu`: once as JSON for the checks of section 3, once with `about.hbs` for `crates.html`. It copies the `NOTICE` files and `COPYRIGHT-library.html` (from `rustc --print sysroot`), writes `README.txt`, makes everything world-readable, and exits non-zero on any problem of section 3 after listing them all. It never reaches the network after the fetch. Sourced, it only defines its functions, so its test can compare its reading of `about.toml` with a TOML parser's.
- **`about.toml`** at the repository root:
  - `accepted` is the allow-list of `deny.toml`, MIT first because cargo-about takes the first accepted licence of an `OR` expression and MIT is the shortest text; the crate tables add the per-crate licences of `deny.toml`'s exceptions (ADR 0027 §2). cargo-deny stays the licence policy; this list only picks the text to show.
  - `targets = ["x86_64-unknown-linux-gnu"]`, `ignore-build-dependencies` and `ignore-dev-dependencies` are true, and `private = { ignore = true }` leaves out backend3 and migration.
  - Clarifications, each naming whole files by SHA-256, one licence per file: `libwebp-sys` (`MIT AND BSD-3-Clause`, `vendor/COPYING` for BSD-3-Clause, as in `deny.toml`) and `libdeflate-sys` (`Apache-2.0 AND MIT`, with `libdeflate/COPYING` for the bundled C library, the same case); `mozjpeg-sys`, whose `LICENSE` carries libjpeg-turbo's BSD-3-Clause text while IJG and Zlib keep the standard texts, and `mozjpeg`, which declares IJG and carries the same file; `brotli-decompressor`, taken under BSD-3-Clause, the one licence of its `BSD-3-Clause OR MIT` whose text it ships; and the 12 other crates whose licence file the detector does not recognise (`async-stream`, `async-stream-impl`, `chrono`, `chrono-tz`, `curve25519-dalek`, `libm`, `ouroboros`, `ouroboros_macro`, `ryu`, `serde_regex`, `sync_wrapper`, `whoami`). The `chrono-tz` entry fits 0.10.4; 0.9.0, whose file is a flattened link, warns and keeps the standard text, which check 2 accepts because that file holds no licence text.
  - The crates that ship no licence text get the standard text, as D115 said.
- **`about.hbs`** is backend3's own template. Handlebars escapes every value for HTML, and the template never uses the unescaped form, so a licence text cannot add markup. It prints no file system path of the build.
- **The procedural macros are listed.** cargo-about cannot leave them out, and listing crates that the binary does not link costs nothing.

### 3. The checks, and where they run

`scripts/crate-notices.sh` fails, and says what to change, when:

1. cargo-about cannot read or satisfy a crate's licence expression (`--fail`);
2. a clarification of `about.toml` does not hold: it names a crate that the notices do not list (a misspelt name never applies), or, for a listed version of its crate, a file it names is missing or has another SHA-256 than `about.toml` gives; the message gives the file's current SHA-256. The one exception is a file that holds no licence text in that version, an empty file or a flattened link (`chrono-tz` 0.9.0): that version ships no licence text there and gets the standard text. Every other clarification that cargo-about reports as failed fails too. cargo-about only warns when a clarification fails, and falls back to scanning, so this is the check that keeps a changed file of bundled code (libwebp's `vendor/COPYING`, libdeflate's `libdeflate/COPYING`) from dropping its copyright notice without a failure. The script reads the clarifications itself, so `about.toml` writes each one in a single form: a `[<crate>.clarify]` table with `license`, and one `[[<crate>.clarify.files]]` table per file with `path`, `checksum` and optionally `license`, whole files only; any other form fails as unreadable;
3. the notices do not carry a licence file that they should: each file a clarification names, where it holds a licence text, and, for a crate that gets the standard text of any of its licences, each licence file at its root or in a root `LICENSES/` directory. A file counts as carried when one of the crate's texts in the notices is exactly its content (cargo-about prints a shared text once, under another crate's file name). The message gives the file's SHA-256 for a new clarification. An empty file and a one-line file that holds only a relative path upward (a flattened link) hold no licence text and do not count. A crate with no text at all fails too;
4. a `NOTICE` file lies below a crate's root, where it belongs to bundled code that the script does not copy; one at the root is copied;
5. a crate that `cargo tree -e normal,no-proc-macro` links for the release target is missing from the notices, so the graph filters of `about.toml` cannot drop a linked crate;
6. the toolchain has no `COPYRIGHT-library.html`.

Check 3 cannot tell which licence a file holds. A crate that gets the standard text of one licence fails, too, when it ships a file of a licence that its expression does not use; a clarification that names the crate's files settles that case.

The image build runs the script, so a gap fails the image. The PR check is a step of the required `cargo-deny check bans licenses sources` job, after `cargo deny`: taiki-e/install-action installs `cargo-about@<version>` (the same checksummed musl archive), and the step runs the script into the runner's temporary directory. A step adds no context, so branch protection and website_k8s's Prow configuration stay as they are (ADR 0022 §3). `scripts/ci/check-toolchain.sh` rule 5, in the `Toolchain` job, keeps the version in step: the Containerfile's `CARGO_ABOUT_VERSION` is an exact release, its `CARGO_ABOUT_SHA256` is a SHA-256, at least one workflow installs cargo-about with taiki-e/install-action, and every `cargo-about@<version>` there is the Containerfile's. Dependabot bumps the action's commit, never the tool's version, so a cargo-about bump changes both by hand in one PR.

### 4. What this ADR amends

| Amended text | Change |
|---|---|
| ADR 0012 §4, "Dependencies are checked, not listed", as ADR 0038 amended it | Still true for `THIRD_PARTY.md`. In addition, the third-party crates of the binary's dependency graph for the release target are listed with their licence texts in `/app/licenses/crates.html`, beside their `NOTICE` files, the standard library's notices and the IJG statement, in every image (this ADR). `THIRD_PARTY.md` says where they are. |
| ADR 0022 §2, the `Build stage` row | adds: the rust stage installs cargo-about, pinned by `CARGO_ABOUT_VERSION` and `CARGO_ABOUT_SHA256`, and `jq`, and runs `scripts/crate-notices.sh` after the release build. |
| ADR 0022 §2, the `Runtime` row | "the binary, `config/` and `admin/dist`" becomes "the binary, `config/`, `admin/dist` and `/app/licenses/`". |
| ADR 0022 §3, the `cargo-deny check bans licenses sources` row | "the policy in section 4" becomes "the policy in section 4, then `scripts/crate-notices.sh` with the Containerfile's cargo-about version ([ADR 0042](0042-licence-notices-of-the-crates-linked-into-the-backend3-binary.md) §3)". |
| ADR 0022 §3, the `Toolchain` row, as ADR 0029 §3 gives it | adds after rule 4: "the Containerfile pins cargo-about by an exact version and a SHA-256, and every `cargo-about@<version>` that a workflow installs is that version (rule 5; [ADR 0042](0042-licence-notices-of-the-crates-linked-into-the-backend3-binary.md) §3)". |

ADR 0038 is not amended: its §3 records D115 and says this slice records its own ADR, and the binary's notices stay outside the admin's bundle. ADR 0004 is not amended either: its §2 leaves the Rust stage's rules to ADR 0022, and the diagram there, whose runtime box shows the binary, `config/` and `admin/dist/`, illustrates the stages without listing every file; it stays as it is (ADR 0001 §6), and ADR 0022 §2's `Runtime` row, amended above, is the list.

### 5. Readings of D115, confirmed as built (D125)

The consequence of D115 names the parts above. Where the build had to choose, it read the answer in six places, listed below. Each can be reverted on its own, except that leaving the procedural macros out (5) would take a renderer of backend3's own instead of cargo-about's template (see the alternatives). The register asked the maintainer about all six at once (research: third decision register D125), and the maintainer answered on 2026-11-01 (research: DECISIONS R26), with the recommended option. The register's question, the answer, and the consequence the register gave for that option:

- **D125**, "D115 chose to generate the binary's licence notices in the image build: a libwebp-sys clarification, the standard text for the crates without a licence file, and a PR check with --fail and a NOTICE check. ADR 0042 §5 reads it beyond that wording in six places, 1 to 6 below. Each can be reverted on its own, except 5. Do you confirm all six as built, confirm only some, or keep only D115's literal parts?": "Confirm all six as built". Its consequence: "No code change. ADR 0042 §5 quotes this answer and the PR leaves draft as built: 17 clarifications, checks 1 to 6 in the image build and in the licence job, rule 5, the macros listed, and the IJG statement in both files."

The six readings, numbered as the register numbers them, which the maintainer confirmed as built:

1. "the standard text for the 17 crates without a licence file" means that a crate with a licence file keeps its own text, so the 14 crates whose file the detector misses are clarified (`mozjpeg` among them), and check 3 keeps it so. A flattened link is no licence file.
2. "a libwebp-sys clarification" covers the class it stands for, a `-sys` crate that compiles C code under a licence it does not declare, so `libdeflate-sys` is clarified too.
3. "a check for NOTICE files the file does not carry" is check 4; checks 2, 3 and 5 are added so that the other gaps that the measurement found fail before `master` as well: a clarification that no longer holds, a licence file that the notices do not carry, and a linked crate that the configuration leaves out.
4. The PR check is a step of an existing required job, not a job of its own, and `check-toolchain.sh` rule 5 keeps its cargo-about version equal to the Containerfile's.
5. `crates.html` also lists the 64 procedural macros and crates only they use, which the binary does not link, because cargo-about cannot leave them out.
6. "The stage adds ... the IJG statement" puts it in `README.txt` and at the top of `crates.html`.

## Consequences

**Positive**

- Every image carries the notices of the code in its binary, written from the `Cargo.lock` and the toolchain that built it, so they cannot drift from the binary.
- A gap fails the PR that causes it: a new licence, a licence file that cargo-about stops reading, a clarified file that changes (bundled code included), a misspelt clarification, a new `NOTICE` file in bundled code, or a linked crate that the configuration leaves out.
- No route, no admin change and no new required context.

**Negative**

- 17 clarifications to maintain. When an update changes one of those files, the PR check fails and prints the file's new SHA-256; the file is read and the checksum renewed in the same PR, which a Dependabot PR cannot do by itself.
- The PR check adds two cargo-about runs to the `cargo-deny check bans licenses sources` job; locally both took 22 to 34 seconds together on many cores, more on a 4-core runner.
- `crates.html` also lists 64 crates that the binary does not link, and some licence texts are source-file headers with a few lines of code after them, as cargo-about reads them.
- Licence files of bundled code that lie below a crate's root are checked only where a clarification names them (`libwebp-sys`, `libdeflate-sys`): no check finds bundled code that nobody has clarified yet. A new `-sys` crate is reviewed for bundled code when it arrives (recipe 5 of the project skill).
- Check 3 cannot tell which licence a file holds, so a crate that gets one standard text and ships a licence file of a licence it does not use fails until a clarification names its files; none does today.
- The script reads `about.toml`'s clarifications itself, so they keep the one form it understands; another form fails the check, and its test compares the script's reading with a TOML parser's.
- cargo-about is a second tool to bump by hand, in the Containerfile and the workflow together.

**Follow-up work**

- None for release 1. A PR that changes the Containerfile is built locally with `podman build` (ADR 0022, Consequences); this one was.

**What must be true in code and tests**

- `scripts/tests/test-crate-notices.sh` runs the script against stub `cargo`, `rustc` and `cargo-about` programs and fixture crates, with fixture JSON in cargo-about's shape (a text from a file is that file's content, and crates with the same text share one entry): the happy path and its output files, which are world-readable even under `umask 077`; each check failing on its own gap: for check 2 a changed file below the root and at the root, a missing file, a misspelt crate, a version that rejects the clarification while another named file holds text, another failure cargo-about reports, and every `about.toml` form the script cannot read, while a version whose clarified file is a flattened link or empty passes; for check 3 every licence-file name form, flattened links, empty files, a crate with its own text for one licence and an uncarried file of another, a text shared with another crate, a text from another file, and a crate without text; a `NOTICE` file below the root; a missing linked crate; a missing `COPYRIGHT-library.html`; a cargo-about failure. Also cargo-about's messages, shown once without colour codes or time stamps, and the template run's own; the usage errors and the exit codes. It also checks that `about.toml`'s target is the script's, that its accepted licences and per-crate licences are `deny.toml`'s, that its checksums are SHA-256s, that the script reads its clarifications as a TOML parser does and that they use only the keys the script checks, that the IJG sentence of the script, `about.hbs` and `THIRD_PARTY.md` agree, that `about.hbs` has no unescaped value, and that the Containerfile and the cargo-deny workflow run the script.
- `scripts/tests/test-check-toolchain.sh` covers rule 5: matching, mismatched and unversioned `cargo-about@` entries, a cargo-about entry that only a comma-separated tool list holds (matching and mismatched), `.yaml` files, a missing install, an unreadable workflow, and missing or malformed ARGs; and the repository itself passes.
- The required `cargo-deny check bans licenses sources` job runs the script on every PR.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Also serve the file under `/admin` | research: third decision register D115 | Not chosen: the maintainer chose the image only. Site visitors never receive the binary. |
| Leave the binary out | research: third decision register D115 | Not chosen: the image would keep shipping the binary without the notices its crates' licences ask for. |
| cargo-about's built-in workarounds (`chrono`, `ring`) | cargo-about 0.9.3, `docs/src/cli/generate/workarounds.md`, `src/licenses/workarounds/` | Their checksums are of other releases (ring's of the 0.16 licence layout), and they fail with a warning like any clarification. Explicit clarifications in `about.toml` are reviewed with the code. |
| A lower `--threshold` instead of clarifications | measured for this ADR | At 0.5, and at 0.3, it reads 3 of the 14 files (`async-stream`, `async-stream-impl`, `serde_regex`); the other 11 keep the standard text (the detector takes `curve25519-dalek`'s BSD file for Sleepycat), and a weaker threshold lets weaker matches count everywhere else in the graph. |
| Only D115's literal parts | research: third decision register D125 ("Keep only D115's literal parts") | Not chosen: the maintainer confirmed all six readings of §5 as built. Readings 1, 2, 3 and 6 and the rule 5 half of reading 4 would have gone, the IJG statement staying in `README.txt` only, while the procedural macros would have stayed listed, since cargo-about cannot leave them out (reading 5). |
| Only some of the readings of §5 | research: third decision register D125 ("Confirm some; name in the note which to drop") | Not chosen: the maintainer confirmed all six as built. |
| Only the D115 checks (`--fail` and `NOTICE` files) | research: third decision register D115 and D125 | Not chosen: the maintainer confirmed checks 2, 3 and 5 as built (§5, reading 3). A clarification that no longer holds (cargo-about only warns, and libwebp's copyright notice would vanish from `crates.html`), a licence file that the notices do not carry, or a filter that drops a linked crate would pass silently. Checks 2, 3 and 5 cost no extra run. |
| Fail only on cargo-about's clarification warnings | considered for check 2 | The warning is cargo-about's log text, which a release may reword, and it does not say which file failed. The script checks every named file's checksum itself and treats the warnings as a further check. |
| Copy every `NOTICE` file at any depth | considered for section 3 | It would copy files of tests and of code that is not built, without anyone reading them; none exists today, and check 4 makes a person decide when one arrives. |
| A PR check as a job of its own | considered for section 3 | A new required context needs a website_k8s change to Prow's branch protection; a step of the licence job needs none. |
| Download cargo-about in CI from the Containerfile's ARGs | considered for section 3 | A second download script; install-action already installs the same archive by checksum, and rule 5 keeps the versions equal. |
| Render the notices from cargo-about's JSON with jq, in one run | considered for section 2 | D115 names a committed template, and a hand-written renderer would replace cargo-about's documented template input. |

## References

- research: DECISIONS R23 (D115, answer of 2026-11-01); research: third decision register D115 (the question, its context and the options)
- research: DECISIONS R26 (D125, answer of 2026-11-01); research: third decision register D125 (the question, its context and the options)
- cargo-about 0.9.3 (<https://github.com/EmbarkStudios/cargo-about/releases/tag/0.9.3>): `docs/src/cli/generate/README.md`, `config.md`, `output.md`, `workarounds.md`; `src/cargo-about/generate.rs`, `src/generate.rs`, `src/licenses.rs`, `src/licenses/scan.rs`, `src/licenses/resolution.rs`, `src/lib.rs`
- taiki-e/install-action@86f7a0ffb25f40114606501a311ebcfaf882ea32: `manifests/cargo-about.json`
- The Rust 1.99.1 toolchain: `share/doc/rust/COPYRIGHT-library.html`, `lib/rustlib/manifest-rustc-x86_64-unknown-linux-gnu`
- Apache License 2.0 §4(d); Mozilla Public License 2.0 §3.2; the BSD-3-Clause, MIT and ISC licences; the IJG licence (`mozjpeg-sys`'s `LICENSE`)
- `about.toml`, `about.hbs`, `scripts/crate-notices.sh`, `Containerfile`, `.github/workflows/cargo-deny.yml`, `scripts/ci/check-toolchain.sh`, `deny.toml`, `THIRD_PARTY.md`
- Amended: [ADR 0012](0012-delivery-review-commits-and-licensing.md) §4, one bullet; [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §2, two rows, and §3, two rows; [ADR 0029](0029-no-filters-or-job-conditions-on-required-ci-jobs.md) §3, the `Toolchain` row it gives
- Related: [ADR 0027](0027-per-package-licence-exceptions-and-the-loco-advisory-ignores.md), [ADR 0038](0038-licence-notices-of-the-bundled-admin-packages.md), [image-encoders.md](../image-encoders.md)
