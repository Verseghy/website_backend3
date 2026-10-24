# Parity checklist (test fixture)

A small file in the shape of docs/design/parity.md, for scripts/tests/test-sync-parity-issues.sh. It has two sections (a `##` and a `###` heading), all five dispositions, an empty Notes cell, a relative link, reference links in both forms and one reference definition that no row uses. The table below is not a parity table, so its first column is not read as IDs.

| Column | Meaning |
|---|---|
| ID | Stable identifier |

## 1. Shared behaviour of the content screens (SHR)

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| SHR-01 | Admin access for signed-in accounts | port-with-fix | [0006], [0007] | Every admin request needs an active, signed-in account (ADR 0006). |
| SHR-02 | Server-side paged list table, 25 rows by default | replaced | [0004] | The cut-over steps are in [cutover.md](cutover.md). |
| SHR-03 | Inputs trimmed; empty text saved as NULL | port | none | |

## 2. Posts

### 2.1 Publishing workflow (POS-H)

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| POS-H18 | Some published posts have NULL content | port-with-fix | [api] | Served as [public-api.md][api] specifies, never as an error |
| LOG-01 | Logs screen over the application's log files | retired | [0019] | Operators use `oc logs` and the runbook. |
| PLT-39 | Parity board and `scripts/sync-parity-issues.sh` with its tests | new | [0019], [0012] | Its issue is created closed (ADR [0019] §1). |

[0004]: adr/0004-admin-ui.md
[0006]: adr/0006-authentication.md
[0007]: adr/0007-authorization.md
[0012]: adr/0012-delivery.md
[0019]: adr/0019-scope.md
[0099]: adr/0099-never-used.md
[api]: public-api.md
