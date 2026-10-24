# website_backend3

Backend for the Verseghy Ferenc Gimnázium website, built on [Loco](https://loco.rs) (Rust) and PostgreSQL.
It replaces both `website_backend` (the Laravel/Backpack admin) and `website_backend2` (the read-only GraphQL API):

- the **admin API** at `/api/admin` and the **admin UI** (React, [react-admin](https://marmelab.com/react-admin/) core with [shadcn-admin-kit](https://github.com/marmelab/shadcn-admin-kit)), served from `/admin`;
- the **public read API** used by the public website, served from `/api/v1`.

Work in progress: this is the application skeleton. Features arrive as vertical slices, tracked against the parity checklist.

## Design

The design is recorded in [`docs/design/`](docs/design/README.md) before the code: the architecture overview, the [ADRs](docs/design/adr/README.md), the [parity checklist](docs/design/parity.md), the [public API contract](docs/design/public-api.md), the [data-migration rules](docs/design/data-migration.md), the [cut-over plan](docs/design/cutover.md) and the [editor design](docs/design/editor.md). When code and an ADR disagree, the ADR wins.

Contributors and agents start with [`AGENTS.md`](AGENTS.md); [`CLAUDE.md`](CLAUDE.md) holds the project rules.

## Development quickstart

You need the Rust toolchain manager `rustup` (it installs the release pinned in `rust-toolchain.toml`), Podman or Docker with Compose, and Node.js at the LTS major in `admin/.node-version`.

```sh
podman-compose up -d postgres   # throwaway local PostgreSQL 18 on 127.0.0.1:5432
cargo loco db migrate           # migrate backend3_development
cargo loco start                # http://localhost:5150 (health check: /_ping)

cd admin
npm ci
npm run dev                     # admin UI dev server; proxies /api to backend3
```

The checks that CI runs, and how to run the end-to-end tests, are listed in [`AGENTS.md`](AGENTS.md).

## License

AGPL-3.0-only. See [LICENSE](LICENSE). Vendored third-party code keeps its own license file next to it, indexed in [THIRD_PARTY.md](THIRD_PARTY.md).
