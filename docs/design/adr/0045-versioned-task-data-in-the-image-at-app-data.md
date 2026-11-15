# 0045. Versioned task data in the image at /app/data

- Status: Accepted (2026-11-15)
- Decision questions: none: records the maintainer's answer of 2026-10-25 to the decision-register item D12 (research: DECISIONS R9), which settles where the legacy import's defect resolution file lives and that the image ships it. It amends the `Runtime` row of [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §2, as [ADR 0042](0042-licence-notices-of-the-crates-linked-into-the-backend3-binary.md) §4 gives it.

## Context

[ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md) §3 makes the editor-signed defect resolution list "a versioned file in the repository", and leaves its path and schema to the import PRs (data-migration.md §13 item 8).

| Fact | Value | Source |
|---|---|---|
| What the runtime image holds | "the binary, `config/`, `admin/dist` and `/app/licenses/`". ADR 0042 §4 calls that row of ADR 0022 §2 the list of what the image holds. | ADR 0022 §2, as ADR 0042 §4 amends it |
| Where the import runs | In a backend3 pod: the nightly rehearsal is a Kubernetes CronJob on staging, and the cut-over runs the `import_mysql` task with `oc exec` in the running backend3 Deployment. The pod reads the files that the image holds. | [cutover.md](../cutover.md) §5.2, §6 phase 4; [ADR 0016](0016-no-redis-in-memory-cache-postgres-queue-cronjob-tasks.md) |
| What the task reads | The resolution file of its `resolutions` argument, whose default is `/app/data/import/resolutions.toml`, the image's copy of the repository's `data/import/resolutions.toml`. | data-migration.md §4.1, §8.2 |
| Other task data to come | The release-1 seed (ADR 0019 §5) is proposed as a task that reads versioned seed files from the repository (data-migration.md §13 item 7). None is committed yet. | data-migration.md §13 item 7 |
| Who builds the image | Pipelines-as-Code on the hub, after a merge to `master`. GitHub Actions runs the PR checks and builds no image, so a Containerfile error first shows in the PAC build. | ADR 0022 §1 and Consequences |
| The build context | `.containerignore` leaves out `target/`, `.git/`, the admin's `node_modules/` and build output, local test output, `.env` files and local configuration overrides, not `data/`; the rust stage copies the whole context to `/src` (`COPY . .`). | `.containerignore`, `Containerfile` |

The register asked (research: decision register D12): "Is the defect resolution file data/import/resolutions.toml, shipped in the image at /app/data/import/resolutions.toml, with the TOML schema of import-resolution-schema?" Its context said that this slice "adds `COPY data/` to the image and a valid empty list", and that "the nightly and the window read the list from the image". The maintainer answered on 2026-10-25 (research: DECISIONS R9), with the recommended option:

- **D12**: "data/import/resolutions.toml in the image (TOML)". Its consequence: "The nightly, the rehearsal and the window read the same reviewed bytes. CI validates the file, and reviewers can comment on entries."

## Decision

### 1. What the image carries

The runtime stage copies the repository's `data/` directory from the rust stage to `/app/data/` (`COPY --from=rust /src/data/ /app/data/`), with the owner and modes of the rust stage's files, like `config/`:

| In the repository | In the pod | What it holds |
|---|---|---|
| `data/import/` | `/app/data/import/` | `resolutions.toml`, the defect resolution list (data-migration.md §8.2), the default of the import's `resolutions` argument; `README.md`, which says what may be committed there. |
| `data/seed/` | `/app/data/seed/` | the release-1 seed files, once data-migration.md §13 item 7 settles them; nothing yet. |

Everything under `data/` is public and ships in every image, production's included, so it follows the public-safety rules: the resolution list holds only ids, dates, choices and source values, or the SHA-256 of a source value, never names or content (`data/import/README.md`).

### 2. How it is checked

The image is built only after a merge, so `tests/image_data/` checks the Containerfile in the required `Test` job instead. It fails when:

- the runtime stage, the last stage, does not copy `/src/data/` to `/app/data/`, or the rust stage does not copy the build context into `/src`;
- a `.containerignore` pattern can leave `data/` out of the build context;
- the default `resolutions` path is not the committed file under `/app/`, or that file fails the import's validation or its size limit;
- `data/import/` holds a file that its README does not allow.

A PR that changes the Containerfile is still built locally with `podman build` (ADR 0022, Consequences).

### 3. What this ADR amends

| Amended text | Change |
|---|---|
| ADR 0022 §2, the `Runtime` row, as ADR 0042 §4 gives it | "the binary, `config/`, `admin/dist` and `/app/licenses/`" becomes "the binary, `config/`, `data/` (at `/app/data/`), `admin/dist` and `/app/licenses/`". |

ADR 0017 is not amended: §3 asks for a versioned file in the repository, and this ADR says where the image keeps it. ADR 0004's diagram stays as it is, as ADR 0042 §4 explains: the amended row above is the list.

## Consequences

**Positive**

- The nightly rehearsal, the dress rehearsals and the cut-over read the bytes that the PR reviewed and CI validated (D12's consequence).
- One line covers the data of every task that runs in a pod; the seed files need no second Containerfile change.
- A Containerfile that stops shipping the list fails its PR, not the first import after the merge.

**Negative**

- A change to the list, a re-sign close to the window included, needs a backend3 PR and a new image before a run reads it.
- Every file under `data/` reaches every image, so a file that should stay private never goes there; `data/import/README.md` and `tests/image_data/` keep `data/import/` to its two files.

**Follow-up work**

- None for this ADR. The seed task's slice adds `data/seed/` and its own README; the list's entries come from a staging dry-run report and the editors' sign-off (data-migration.md §8.2).

**What must be true in code and tests**

- `tests/image_data/` passes on the repository and fails on altered copies of the Containerfile (the copy removed, moved to another stage or given another destination) and on `.containerignore` patterns that cover `data/`.
- The committed `data/import/resolutions.toml` passes `domain::import_mysql::resolutions::parse`.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| The same path, in JSON | research: decision register D12 | Not chosen: reviewers cannot add comments to the file. |
| Outside the image, in a website_k8s ConfigMap | research: decision register D12 | Not chosen: it needs human review in another repository, and its schema version can drift from the image that validates it. |
| Copy only `data/import/` | considered for section 1 | The seed files would need a second Containerfile change, and its test a second rule; the README of each subdirectory already says what may be committed there. |

## References

- research: DECISIONS R9 (D12, the answer of 2026-10-25); research: decision register D12 (the question, its context and the options)
- [data-migration.md](../data-migration.md) §4.1, §8.2, §13 items 7 and 8; [cutover.md](../cutover.md) §5.2, §6
- `Containerfile`, `.containerignore`, `data/import/README.md`, `tests/image_data/mod.rs`
- Amended: [ADR 0022](0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md) §2, the `Runtime` row, as [ADR 0042](0042-licence-notices-of-the-crates-linked-into-the-backend3-binary.md) §4 gives it
- Related: [ADR 0017](0017-legacy-data-import-normalisation-and-defect-resolution.md) §3; [ADR 0004](0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md)
