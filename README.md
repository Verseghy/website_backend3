# website_backend3

Backend for the Verseghy Ferenc Gimnázium website, built on [Loco](https://loco.rs) (Rust).
It replaces both `website_backend` (the Laravel/Backpack admin) and `website_backend2` (the read-only GraphQL API):

- the **admin API** and the **admin UI** (React, [react-admin](https://marmelab.com/react-admin/) core with [shadcn-admin-kit](https://github.com/marmelab/shadcn-admin-kit)), served from `/admin`;
- the **public read API** used by the public website, served from `/api/v1`.

The design is recorded in `docs/design/` before any code lands. Work in progress.

## License

AGPL-3.0-only. See [LICENSE](LICENSE). Vendored third-party code keeps its own license file next to it.
