//! The legacy tables that the import does not migrate, with the reason for
//! each (docs/design/data-migration.md §9.1).
//!
//! The run report lists them ([`super::report`]), so every report says what
//! was left behind on purpose and why, next to what was imported. The list
//! is the specification's, in its order; `tests/domain/import_not_migrated.rs`
//! checks that the table names here and in §9.1 are the same, so the code and
//! the document cannot drift apart. The importer reads only the tables it
//! migrates, so none of these is ever selected (NWS-01: the newsletter
//! subscribers stay in the legacy database).
//!
//! The row counts of §9.1 are reference values of one dump and are not
//! repeated here: a report states only what its own run measured.

/// A legacy table that the import leaves behind, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotMigratedTable {
    /// The table's name in the legacy `verseghy` database.
    pub table: &'static str,
    /// Why it is not migrated, as the report shows it.
    pub reason: &'static str,
}

/// The legacy tables that are not migrated, in the order of
/// data-migration.md §9.1.
pub const NOT_MIGRATED_TABLES: [NotMigratedTable; 6] = [
    NotMigratedTable {
        table: "newsletter",
        reason: "The newsletter is retired as a documented parity exception (parity.md NWS-01).",
    },
    NotMigratedTable {
        table: "password_resets",
        reason: "Account recovery becomes admin-issued one-time links or temporary passwords \
                 (ADR 0006).",
    },
    NotMigratedTable {
        table: "migrations",
        reason: "Laravel's migration bookkeeping; backend3 keeps its own in seaql_migrations.",
    },
    NotMigratedTable {
        table: "permissions",
        reason: "Permission names are defined in code (ADR 0007), which keeps 8 of the 9 legacy \
                 names and drops `edit newsletter`; the import verifies the legacy names \
                 (data-migration.md §6.15).",
    },
    NotMigratedTable {
        table: "role_has_permissions",
        reason: "The role-to-permission map is defined in code (ADR 0007); the import verifies \
                 the links of the kept permissions against it (data-migration.md §6.15).",
    },
    NotMigratedTable {
        table: "roles",
        reason: "Seeded by a backend3 migration and verified against the source, not copied \
                 (data-migration.md §6.13).",
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_list_is_the_six_tables_of_section_9_1_in_its_order() {
        let tables: Vec<&str> = NOT_MIGRATED_TABLES.iter().map(|row| row.table).collect();
        assert_eq!(
            tables,
            [
                "newsletter",
                "password_resets",
                "migrations",
                "permissions",
                "role_has_permissions",
                "roles",
            ]
        );
        let unique: BTreeSet<&str> = tables.iter().copied().collect();
        assert_eq!(unique.len(), tables.len(), "a table is listed twice");
    }

    #[test]
    fn every_reason_is_one_sentence_that_a_table_cell_can_hold() {
        for row in NOT_MIGRATED_TABLES {
            assert!(row.reason.ends_with('.'), "{}: {}", row.table, row.reason);
            // The Markdown table and the JSON report hold it as one line.
            assert!(
                !row.reason.contains(['\n', '|']),
                "{}: {}",
                row.table,
                row.reason
            );
            assert!(
                !row.reason.contains("  "),
                "{}: a line continuation left a double space",
                row.table
            );
        }
    }

    #[test]
    fn the_newsletter_is_left_behind_as_parity_nws_01_says() {
        let newsletter = NOT_MIGRATED_TABLES
            .iter()
            .find(|row| row.table == "newsletter")
            .expect("the newsletter table is listed");
        assert!(
            newsletter.reason.contains("NWS-01"),
            "{}",
            newsletter.reason
        );
    }
}
