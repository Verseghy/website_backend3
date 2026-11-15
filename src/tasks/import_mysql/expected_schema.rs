//! The expected legacy DDL that the schema fingerprint compares the source
//! with (docs/design/data-migration.md §3.3): the columns, types and
//! nullability of the 17 legacy tables the import reads, in the order of
//! `domain::import_mysql::source_tables::SOURCE_TABLES`.
//!
//! It is the production schema of 2026-10-23 (research: gap-03 §5), the
//! same that `tests/fixtures/legacy/schema.sql` holds as `CREATE TABLE`
//! statements, written the way MySQL 8.4 reports it in
//! `information_schema.COLUMNS` (`COLUMN_TYPE`, `IS_NULLABLE`). The three
//! tables the import never reads (`newsletter`, `password_resets`,
//! `migrations`; NWS-01, §9.1) are not here: the fingerprint never
//! describes them.
//!
//! Three tests keep the copies equal: the unit tests below check the tables
//! against the allow-list and the timestamp columns against
//! `SOURCE_TIMESTAMPS`; `tests/tasks/import_mysql_session.rs` checks every
//! column against `schema.sql` and, through the session, against the MySQL
//! source of compose.yaml, which loads that file. A change of the legacy
//! schema changes all three, and the steps that read the changed table.
//!
//! The constant lives with the task rather than in `src/domain/`, because
//! it describes the legacy MySQL database, which only the task reads, and
//! goes when the importer goes (data-migration.md §13 item 9); the
//! comparison itself is pure (`domain::import_mysql::fingerprint`).

use crate::domain::import_mysql::fingerprint::{ExpectedColumn, ExpectedTable};

/// A column that admits NULL.
const fn null(name: &'static str, column_type: &'static str) -> ExpectedColumn {
    ExpectedColumn {
        name,
        column_type,
        nullable: true,
    }
}

/// A column that is `NOT NULL`.
const fn not_null(name: &'static str, column_type: &'static str) -> ExpectedColumn {
    ExpectedColumn {
        name,
        column_type,
        nullable: false,
    }
}

/// `int unsigned`, the type of every legacy id and reference.
const ID: &str = "int unsigned";
/// `varchar(191)`, Laravel's string length for utf8mb4 indexes.
const VARCHAR: &str = "varchar(191)";

/// The expected legacy DDL of the tables the import reads.
pub const EXPECTED_SCHEMA: [ExpectedTable; 17] = [
    ExpectedTable {
        table: "posts_data",
        columns: &[
            not_null("id", ID),
            not_null("title", VARCHAR),
            not_null("color", "varchar(7)"),
            null("description", "longtext"),
            null("content", "longtext"),
            null("index_image", VARCHAR),
            null("author_id", ID),
            not_null("images", "json"),
            null("date", "date"),
            not_null("created_at", "datetime"),
            not_null("updated_at", "datetime"),
            not_null("featured", "tinyint(1)"),
            not_null("published", "tinyint(1)"),
            null("previewToken", VARCHAR),
        ],
    },
    ExpectedTable {
        table: "posts_labels",
        columns: &[
            not_null("id", ID),
            not_null("name", VARCHAR),
            not_null("color", VARCHAR),
        ],
    },
    ExpectedTable {
        table: "posts_pivot_labels_data",
        columns: &[not_null("labels_id", ID), not_null("posts_id", ID)],
    },
    ExpectedTable {
        table: "posts_authors",
        columns: &[
            not_null("id", ID),
            not_null("name", VARCHAR),
            null("description", "longtext"),
            null("image", VARCHAR),
        ],
    },
    ExpectedTable {
        table: "colleagues_data",
        columns: &[
            not_null("id", ID),
            null("name", VARCHAR),
            null("jobs", "longtext"),
            null("subjects", "longtext"),
            null("roles", "longtext"),
            null("awards", "longtext"),
            null("image", VARCHAR),
            null("category", "smallint unsigned"),
        ],
    },
    ExpectedTable {
        table: "canteen_menus",
        columns: &[
            not_null("id", ID),
            not_null("menu", VARCHAR),
            not_null("type", "smallint unsigned"),
            not_null("created_at", "datetime"),
            not_null("updated_at", "datetime"),
        ],
    },
    ExpectedTable {
        table: "canteen_data",
        columns: &[
            not_null("id", ID),
            not_null("date", "date"),
            not_null("created_at", "datetime"),
            not_null("updated_at", "datetime"),
        ],
    },
    ExpectedTable {
        table: "canteen_pivot_menus_data",
        columns: &[not_null("data_id", ID), not_null("menu_id", ID)],
    },
    ExpectedTable {
        table: "events_data",
        columns: &[
            not_null("id", ID),
            not_null("date_from", "datetime"),
            not_null("date_to", "datetime"),
            not_null("title", VARCHAR),
            null("description", VARCHAR),
            not_null("color", "varchar(7)"),
            not_null("created_at", "datetime"),
            not_null("updated_at", "datetime"),
        ],
    },
    ExpectedTable {
        table: "pages",
        columns: &[
            not_null("id", ID),
            not_null("template", VARCHAR),
            not_null("name", VARCHAR),
            not_null("title", VARCHAR),
            not_null("slug", VARCHAR),
            not_null("content", "text"),
            not_null("extras", "json"),
            not_null("created_at", "datetime"),
            not_null("updated_at", "datetime"),
            null("deleted_at", "datetime"),
        ],
    },
    ExpectedTable {
        table: "menu_items",
        columns: &[
            not_null("id", ID),
            not_null("name", "varchar(100)"),
            null("type", "varchar(20)"),
            null("link", "varchar(255)"),
            null("page_id", ID),
            null("parent_id", ID),
            null("lft", ID),
            null("rgt", ID),
            null("depth", ID),
            null("created_at", "timestamp"),
            null("updated_at", "timestamp"),
            null("deleted_at", "timestamp"),
        ],
    },
    ExpectedTable {
        table: "users",
        columns: &[
            not_null("id", ID),
            not_null("name", VARCHAR),
            not_null("email", VARCHAR),
            not_null("password", VARCHAR),
            null("remember_token", "varchar(100)"),
            null("created_at", "timestamp"),
            null("updated_at", "timestamp"),
        ],
    },
    ExpectedTable {
        table: "roles",
        columns: &[
            not_null("id", ID),
            not_null("name", VARCHAR),
            not_null("guard_name", VARCHAR),
            null("created_at", "timestamp"),
            null("updated_at", "timestamp"),
        ],
    },
    ExpectedTable {
        table: "model_has_roles",
        columns: &[
            not_null("role_id", ID),
            not_null("model_type", VARCHAR),
            not_null("model_id", "bigint unsigned"),
        ],
    },
    ExpectedTable {
        table: "model_has_permissions",
        columns: &[
            not_null("permission_id", ID),
            not_null("model_type", VARCHAR),
            not_null("model_id", "bigint unsigned"),
        ],
    },
    ExpectedTable {
        table: "permissions",
        columns: &[
            not_null("id", ID),
            not_null("name", VARCHAR),
            not_null("guard_name", VARCHAR),
            null("created_at", "timestamp"),
            null("updated_at", "timestamp"),
        ],
    },
    ExpectedTable {
        table: "role_has_permissions",
        columns: &[not_null("permission_id", ID), not_null("role_id", ID)],
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::import_mysql::{
        fingerprint::{FoundColumn, compare},
        source_session::SOURCE_TIMESTAMPS,
        source_tables::SOURCE_TABLES,
    };

    #[test]
    fn the_expected_tables_are_the_allow_list_in_its_order() {
        let expected: Vec<&str> = EXPECTED_SCHEMA.iter().map(|table| table.table).collect();
        let allowed: Vec<&str> = SOURCE_TABLES.iter().map(|table| table.name()).collect();
        assert_eq!(expected, allowed);
        for table in EXPECTED_SCHEMA {
            let names: BTreeSet<&str> = table.columns.iter().map(|column| column.name).collect();
            assert_eq!(names.len(), table.columns.len(), "{}", table.table);
        }
    }

    #[test]
    fn every_write_instant_column_is_a_source_timestamp() {
        // The DATETIME and TIMESTAMP columns that are not write instants:
        // when an event takes place (data-migration.md §5.2).
        let schedule = [("events_data", "date_from"), ("events_data", "date_to")];
        let mut instants = BTreeSet::new();
        for table in EXPECTED_SCHEMA {
            for column in table.columns {
                if matches!(column.column_type, "datetime" | "timestamp")
                    && !schedule.contains(&(table.table, column.name))
                {
                    instants.insert((table.table, column.name));
                }
            }
        }
        let listed: BTreeSet<(&str, &str)> = SOURCE_TIMESTAMPS
            .iter()
            .flat_map(|entry| entry.columns.iter().map(|column| (entry.table, *column)))
            .collect();
        assert_eq!(listed, instants);
    }

    #[test]
    fn the_expected_schema_matches_itself() {
        let found: Vec<FoundColumn> = EXPECTED_SCHEMA
            .iter()
            .flat_map(|table| {
                table.columns.iter().map(|column| FoundColumn {
                    table: table.table.to_owned(),
                    name: column.name.to_owned(),
                    column_type: column.column_type.to_owned(),
                    nullable: column.nullable,
                })
            })
            .collect();
        assert_eq!(compare(&EXPECTED_SCHEMA, &found), Ok(()));
    }
}
