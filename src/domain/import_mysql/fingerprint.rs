//! The schema fingerprint of the legacy source (docs/design/data-migration.md
//! §3.3): before the import reads a row, it compares the columns, column
//! types and nullability of every table it reads with the expected legacy
//! DDL, and any difference stops the run.
//!
//! Why: the import reads the legacy tables by column name and decodes each
//! value by its expected type (§5). A source loaded from the wrong dump, a
//! dump of another schema version (backend1's migrations were not all
//! applied everywhere; research: gap-03 §4) or a hand-edited table would
//! otherwise fail somewhere in the middle of a step, or worse, import a
//! column with the wrong meaning. The comparison names every mismatch at
//! once, so the operator sees the whole difference in one run.
//!
//! This module holds the comparison only; it reads nothing (ADR 0008 rule
//! 1). The expected DDL is the constant
//! `tasks::import_mysql::expected_schema::EXPECTED_SCHEMA`, which exists
//! only with the `import-mysql` feature, and the task reads the found
//! columns from `information_schema.COLUMNS` in the locked source session
//! (`tasks::import_mysql::session`) and hands them in as [`FoundColumn`]s.
//!
//! What counts: a column's name, its `COLUMN_TYPE` exactly as MySQL 8.4
//! writes it (`int unsigned`, `varchar(191)`, `tinyint(1)`, `json`), and
//! whether it admits NULL. Column order, defaults, character sets, indexes
//! and the engine are left out: the import selects columns by name and
//! asks the session for `utf8mb4` (§3.3), so none of them changes what it
//! reads. Names and types are compared exactly, because MySQL reports them
//! exactly as the DDL created them.

use std::{collections::BTreeSet, fmt};

/// One column of the expected legacy DDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedColumn {
    /// The column's name.
    pub name: &'static str,
    /// Its `COLUMN_TYPE` as MySQL 8.4 reports it, for example `int unsigned`.
    pub column_type: &'static str,
    /// Whether it admits NULL.
    pub nullable: bool,
}

/// One table of the expected legacy DDL, with its columns in DDL order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedTable {
    /// The legacy table's name.
    pub table: &'static str,
    /// Its columns.
    pub columns: &'static [ExpectedColumn],
}

/// One column of the source, as `information_schema.COLUMNS` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundColumn {
    /// The table's name (`TABLE_NAME`).
    pub table: String,
    /// The column's name (`COLUMN_NAME`).
    pub name: String,
    /// Its type (`COLUMN_TYPE`).
    pub column_type: String,
    /// Whether it admits NULL (`IS_NULLABLE`, read by [`parse_is_nullable`]).
    pub nullable: bool,
}

/// Reads `information_schema.COLUMNS.IS_NULLABLE`, which MySQL writes as
/// `YES` or `NO`.
///
/// - `value`: the column's text.
///
/// Returns `None` for any other text, which the task refuses as an answer
/// it cannot read.
#[must_use]
pub fn parse_is_nullable(value: &str) -> Option<bool> {
    match value {
        "YES" => Some(true),
        "NO" => Some(false),
        _ => None,
    }
}

/// One difference between the source and the expected DDL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// An expected table has no column in the source: it does not exist.
    MissingTable {
        /// The table's name.
        table: String,
    },
    /// The source reported columns of a table that is not expected.
    UnexpectedTable {
        /// The table's name.
        table: String,
    },
    /// An expected column does not exist in the source.
    MissingColumn {
        /// The table's name.
        table: String,
        /// The column's name.
        column: String,
        /// The expected type.
        expected_type: String,
    },
    /// The source has a column that the DDL does not.
    UnexpectedColumn {
        /// The table's name.
        table: String,
        /// The column's name.
        column: String,
        /// The type it has in the source.
        found_type: String,
    },
    /// The column's type differs.
    TypeDiffers {
        /// The table's name.
        table: String,
        /// The column's name.
        column: String,
        /// The expected type.
        expected: String,
        /// The type it has in the source.
        found: String,
    },
    /// The column admits NULL where the DDL does not, or the other way
    /// round.
    NullabilityDiffers {
        /// The table's name.
        table: String,
        /// The column's name.
        column: String,
        /// Whether the DDL admits NULL.
        expected_nullable: bool,
    },
}

/// `NULL` or `NOT NULL`, as the DDL writes it.
const fn null_text(nullable: bool) -> &'static str {
    if nullable { "NULL" } else { "NOT NULL" }
}

impl fmt::Display for Mismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTable { table } => {
                write!(formatter, "the table `{table}` is missing")
            }
            Self::UnexpectedTable { table } => write!(
                formatter,
                "the source reported columns of `{table}`, which is not one of the expected tables"
            ),
            Self::MissingColumn {
                table,
                column,
                expected_type,
            } => write!(
                formatter,
                "`{table}`.`{column}` is missing (expected {expected_type})"
            ),
            Self::UnexpectedColumn {
                table,
                column,
                found_type,
            } => write!(
                formatter,
                "`{table}`.`{column}` ({found_type}) is not in the expected DDL"
            ),
            Self::TypeDiffers {
                table,
                column,
                expected,
                found,
            } => write!(
                formatter,
                "`{table}`.`{column}` is {found}, expected {expected}"
            ),
            Self::NullabilityDiffers {
                table,
                column,
                expected_nullable,
            } => write!(
                formatter,
                "`{table}`.`{column}` is {}, expected {}",
                null_text(!expected_nullable),
                null_text(*expected_nullable)
            ),
        }
    }
}

/// Every difference between the source and the expected DDL, in the order
/// of the expected tables and their columns, then the unexpected tables.
/// The message names each table and column (data-migration.md §4.2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct FingerprintMismatch(pub Vec<Mismatch>);

impl fmt::Display for FingerprintMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.0.len();
        write!(
            formatter,
            "refusing to import: the source schema differs from the expected legacy DDL \
             (data-migration.md §3.3) in {count} {}: ",
            if count == 1 { "place" } else { "places" }
        )?;
        for (index, mismatch) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{mismatch}")?;
        }
        formatter.write_str(
            ". Load the source from a dump of the legacy database `verseghy` (data-migration.md \
             §3.1), or, if the legacy schema really changed, update the expected DDL \
             (src/tasks/import_mysql/expected_schema.rs and tests/fixtures/legacy/schema.sql) \
             and the import steps that read the changed tables",
        )
    }
}

/// Compares the source's columns with the expected DDL.
///
/// - `expected`: the expected tables, each with its columns.
/// - `found`: every column that `information_schema.COLUMNS` reported for
///   the tables the import reads, in any order.
///
/// # Errors
///
/// A [`FingerprintMismatch`] that lists every difference: a missing table
/// (no column found), a missing or unexpected column, a column of another
/// type or nullability, and columns of a table that is not expected.
pub fn compare(
    expected: &[ExpectedTable],
    found: &[FoundColumn],
) -> Result<(), FingerprintMismatch> {
    let mut mismatches = Vec::new();
    for table in expected {
        let columns: Vec<&FoundColumn> = found
            .iter()
            .filter(|column| column.table == table.table)
            .collect();
        if columns.is_empty() {
            mismatches.push(Mismatch::MissingTable {
                table: table.table.to_owned(),
            });
            continue;
        }
        for column in table.columns {
            match columns.iter().find(|found| found.name == column.name) {
                None => mismatches.push(Mismatch::MissingColumn {
                    table: table.table.to_owned(),
                    column: column.name.to_owned(),
                    expected_type: column.column_type.to_owned(),
                }),
                Some(found) => {
                    if found.column_type != column.column_type {
                        mismatches.push(Mismatch::TypeDiffers {
                            table: table.table.to_owned(),
                            column: column.name.to_owned(),
                            expected: column.column_type.to_owned(),
                            found: found.column_type.clone(),
                        });
                    }
                    if found.nullable != column.nullable {
                        mismatches.push(Mismatch::NullabilityDiffers {
                            table: table.table.to_owned(),
                            column: column.name.to_owned(),
                            expected_nullable: column.nullable,
                        });
                    }
                }
            }
        }
        for found in columns {
            if !table.columns.iter().any(|column| column.name == found.name) {
                mismatches.push(Mismatch::UnexpectedColumn {
                    table: table.table.to_owned(),
                    column: found.name.clone(),
                    found_type: found.column_type.clone(),
                });
            }
        }
    }
    // A table the query was not asked about; the task's statement names only
    // the expected tables, so this means the answer is not what was asked.
    let unexpected: BTreeSet<&str> = found
        .iter()
        .map(|column| column.table.as_str())
        .filter(|name| !expected.iter().any(|table| table.table == *name))
        .collect();
    mismatches.extend(
        unexpected
            .into_iter()
            .map(|table| Mismatch::UnexpectedTable {
                table: table.to_owned(),
            }),
    );
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(FingerprintMismatch(mismatches))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const POSTS: ExpectedTable = ExpectedTable {
        table: "posts_data",
        columns: &[
            ExpectedColumn {
                name: "id",
                column_type: "int unsigned",
                nullable: false,
            },
            ExpectedColumn {
                name: "date",
                column_type: "date",
                nullable: true,
            },
        ],
    };

    const LABELS: ExpectedTable = ExpectedTable {
        table: "posts_labels",
        columns: &[ExpectedColumn {
            name: "name",
            column_type: "varchar(191)",
            nullable: false,
        }],
    };

    /// The columns that a source with exactly `tables` reports.
    fn found_of(tables: &[ExpectedTable]) -> Vec<FoundColumn> {
        tables
            .iter()
            .flat_map(|table| {
                table.columns.iter().map(|column| FoundColumn {
                    table: table.table.to_owned(),
                    name: column.name.to_owned(),
                    column_type: column.column_type.to_owned(),
                    nullable: column.nullable,
                })
            })
            .collect()
    }

    #[test]
    fn is_nullable_reads_yes_and_no_only() {
        assert_eq!(parse_is_nullable("YES"), Some(true));
        assert_eq!(parse_is_nullable("NO"), Some(false));
        for other in ["yes", "no", "", "YES ", "1", "NULL"] {
            assert_eq!(parse_is_nullable(other), None, "{other:?}");
        }
    }

    #[test]
    fn an_identical_source_matches_in_any_order() {
        let mut found = found_of(&[POSTS, LABELS]);
        assert_eq!(compare(&[POSTS, LABELS], &found), Ok(()));
        found.reverse();
        assert_eq!(compare(&[POSTS, LABELS], &found), Ok(()));
    }

    #[test]
    fn every_mismatch_is_named_with_its_table_and_column() {
        let mut found = found_of(&[POSTS]);
        // `id` loses its unsigned, `date` becomes NOT NULL, and a column
        // that the DDL does not have appears.
        found[0].column_type = "int".to_owned();
        found[1].nullable = false;
        found.push(FoundColumn {
            table: "posts_data".to_owned(),
            name: "type".to_owned(),
            column_type: "smallint unsigned".to_owned(),
            nullable: false,
        });
        found.push(FoundColumn {
            table: "newsletter".to_owned(),
            name: "email".to_owned(),
            column_type: "varchar(191)".to_owned(),
            nullable: false,
        });
        let mismatch = compare(&[POSTS, LABELS], &found).expect_err("differs");
        assert_eq!(
            mismatch.0,
            [
                Mismatch::TypeDiffers {
                    table: "posts_data".to_owned(),
                    column: "id".to_owned(),
                    expected: "int unsigned".to_owned(),
                    found: "int".to_owned(),
                },
                Mismatch::NullabilityDiffers {
                    table: "posts_data".to_owned(),
                    column: "date".to_owned(),
                    expected_nullable: true,
                },
                Mismatch::UnexpectedColumn {
                    table: "posts_data".to_owned(),
                    column: "type".to_owned(),
                    found_type: "smallint unsigned".to_owned(),
                },
                Mismatch::MissingTable {
                    table: "posts_labels".to_owned(),
                },
                Mismatch::UnexpectedTable {
                    table: "newsletter".to_owned(),
                },
            ]
        );
        assert_eq!(
            mismatch.to_string(),
            "refusing to import: the source schema differs from the expected legacy DDL \
             (data-migration.md §3.3) in 5 places: `posts_data`.`id` is int, expected int \
             unsigned; `posts_data`.`date` is NOT NULL, expected NULL; `posts_data`.`type` \
             (smallint unsigned) is not in the expected DDL; the table `posts_labels` is \
             missing; the source reported columns of `newsletter`, which is not one of the \
             expected tables. Load the source from a dump of the legacy database `verseghy` \
             (data-migration.md §3.1), or, if the legacy schema really changed, update the \
             expected DDL (src/tasks/import_mysql/expected_schema.rs and \
             tests/fixtures/legacy/schema.sql) and the import steps that read the changed tables"
        );
    }

    #[test]
    fn a_missing_column_names_its_expected_type() {
        let found = found_of(&[POSTS]);
        let mismatch = compare(&[POSTS], &found[..1]).expect_err("date is missing");
        assert_eq!(
            mismatch.to_string(),
            "refusing to import: the source schema differs from the expected legacy DDL \
             (data-migration.md §3.3) in 1 place: `posts_data`.`date` is missing (expected \
             date). Load the source from a dump of the legacy database `verseghy` \
             (data-migration.md §3.1), or, if the legacy schema really changed, update the \
             expected DDL (src/tasks/import_mysql/expected_schema.rs and \
             tests/fixtures/legacy/schema.sql) and the import steps that read the changed tables"
        );
        let nullable = Mismatch::NullabilityDiffers {
            table: "t".to_owned(),
            column: "c".to_owned(),
            expected_nullable: false,
        };
        assert_eq!(nullable.to_string(), "`t`.`c` is NULL, expected NOT NULL");
    }

    #[test]
    fn an_empty_source_misses_every_table() {
        let mismatch = compare(&[POSTS, LABELS], &[]).expect_err("nothing found");
        assert_eq!(
            mismatch.0,
            [
                Mismatch::MissingTable {
                    table: "posts_data".to_owned()
                },
                Mismatch::MissingTable {
                    table: "posts_labels".to_owned()
                },
            ]
        );
    }

    /// A generated table: its name, and each column's name, type and
    /// nullability.
    type GeneratedTable = (String, Vec<(String, String, bool)>);

    /// A schema of one to four tables with one to five columns each, with
    /// distinct table names and distinct column names per table.
    fn any_schema() -> impl Strategy<Value = Vec<GeneratedTable>> {
        let column = (
            "[a-z][a-z_]{0,8}",
            "(int|varchar\\(191\\)|json|date)",
            any::<bool>(),
        );
        let table = (
            "[a-z][a-z_]{0,8}",
            proptest::collection::vec(column, 1..5).prop_map(|columns| {
                let mut seen = BTreeSet::new();
                columns
                    .into_iter()
                    .filter(|(name, _, _)| seen.insert(name.clone()))
                    .collect::<Vec<_>>()
            }),
        );
        proptest::collection::vec(table, 1..4).prop_map(|tables| {
            let mut seen = BTreeSet::new();
            tables
                .into_iter()
                .filter(|(name, _)| seen.insert(name.clone()))
                .collect()
        })
    }

    /// `schema` as leaked [`ExpectedTable`]s; leaking is fine in a test.
    fn leak(schema: &[GeneratedTable]) -> Vec<ExpectedTable> {
        schema
            .iter()
            .map(|(table, columns)| ExpectedTable {
                table: Box::leak(table.clone().into_boxed_str()),
                columns: Box::leak(
                    columns
                        .iter()
                        .map(|(name, column_type, nullable)| ExpectedColumn {
                            name: Box::leak(name.clone().into_boxed_str()),
                            column_type: Box::leak(column_type.clone().into_boxed_str()),
                            nullable: *nullable,
                        })
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                ),
            })
            .collect()
    }

    proptest! {
        #[test]
        fn only_yes_and_no_are_read_as_nullability(value in prop_oneof![
            Just("YES".to_owned()),
            Just("NO".to_owned()),
            ".{0,8}",
        ]) {
            let expected = match value.as_str() {
                "YES" => Some(true),
                "NO" => Some(false),
                _ => None,
            };
            prop_assert_eq!(parse_is_nullable(&value), expected);
        }

        #[test]
        fn a_source_with_the_expected_columns_always_matches(
            schema in any_schema(),
            rotation in any::<prop::sample::Index>(),
        ) {
            let expected = leak(&schema);
            let mut found = found_of(&expected);
            // The order of the answer does not matter.
            let by = rotation.index(found.len());
            found.rotate_left(by);
            prop_assert_eq!(compare(&expected, &found), Ok(()));
        }

        #[test]
        fn one_changed_column_gives_exactly_one_mismatch_that_names_it(
            schema in any_schema(),
            pick in any::<prop::sample::Index>(),
            change in 0_u8..4,
        ) {
            let expected = leak(&schema);
            let mut found = found_of(&expected);
            let index = pick.index(found.len());
            let table = found[index].table.clone();
            let column = found[index].name.clone();
            match change {
                0 => found[index].column_type.push_str(" unsigned"),
                1 => found[index].nullable = !found[index].nullable,
                2 => {
                    found.remove(index);
                }
                _ => found.push(FoundColumn {
                    table: table.clone(),
                    name: format!("{column}_extra9"),
                    column_type: "text".to_owned(),
                    nullable: true,
                }),
            }
            let table_now_empty = !found.iter().any(|found| found.table == table);
            let mismatch = compare(&expected, &found).expect_err("one change");
            prop_assert_eq!(mismatch.0.len(), 1, "{:?}", mismatch);
            let text = mismatch.to_string();
            prop_assert!(text.contains(&format!("`{table}`")), "{}", text);
            if !table_now_empty {
                prop_assert!(text.contains(&format!("`{column}")), "{}", text);
            }
        }
    }
}
