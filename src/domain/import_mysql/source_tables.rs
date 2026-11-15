//! The legacy tables that the import reads: its source allow-list
//! (docs/design/data-migration.md §3.3, §6, §9.1; parity.md NWS-01).
//!
//! The legacy `verseghy` database has 20 tables. The import reads 17 of
//! them, [`SOURCE_TABLES`]:
//!
//! - the 14 that it migrates (data-migration.md §6.1-§6.12, §6.14, §6.15);
//! - the 3 that it verifies against backend3's own code without copying
//!   them: `roles` (§6.13), and `permissions` and `role_has_permissions`
//!   (§6.15). They are on the not-migrated list of §9.1 too, because no row
//!   of theirs is copied.
//!
//! The other 3 tables of §9.1 are never read: `newsletter`, the retired
//! newsletter's subscriber list of e-mail addresses and tokens (NWS-01),
//! `password_resets` and `migrations`. No statement that the import sends
//! may select, lock or describe them.
//!
//! The rule is kept in one place, and three things hold it:
//!
//! - a read of a legacy table is a [`SourceSelect`], whose table is a
//!   [`SourceTable`], which only this module can create; [`SourceSelect`]
//!   refuses a name that is not on the list, with a message that names the
//!   table, before the task sends anything to the source
//!   (`tasks::import_mysql::source::select_rows`, the one function that
//!   sends it);
//! - only `src/tasks/import_mysql/source.rs` and the source session,
//!   `src/tasks/import_mysql/session.rs`, build or send a statement to the
//!   source: `tests/architecture.rs` fails when another file of the import
//!   task names sqlx's statement or connection API, and when a file outside
//!   the import task names sqlx at all;
//! - the source session's statements (`LOCK TABLES`, the schema
//!   fingerprint, the `AUTO_INCREMENT` and timestamp reads;
//!   `super::source_session`) name their tables from [`SOURCE_TABLES`] or
//!   check them against it, and a statement of another kind does the same
//!   and joins the architecture test's list of statement files in the PR
//!   that adds it, so that review checks this. Under `LOCK TABLES` the
//!   session can read only the tables it locked, so the lock keeps the
//!   other tables unread as well.
//!
//! The source account itself may read every legacy table (data-migration.md
//! §3.3 grants `SELECT` on the whole database), so this list, not the
//! server, is what keeps the newsletter subscribers unread.
//!
//! `tests/domain/import_source_tables.rs` checks the list against the
//! table headings of data-migration.md §6 and against the legacy DDL in
//! `tests/fixtures/legacy/schema.sql`, so the code, the specification and
//! the fixture cannot drift apart.

use std::fmt::Write as _;

use super::not_migrated::NOT_MIGRATED_TABLES;

/// A legacy table on the import's source allow-list.
///
/// The field is private: the only values are the entries of
/// [`SOURCE_TABLES`], and [`SourceTable::from_name`] is the only way from a
/// name to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceTable(&'static str);

/// The legacy tables that the import reads, in the order of
/// data-migration.md §6, with each verified table where its section reads
/// it (`roles` in §6.13, `permissions` and `role_has_permissions` in §6.15).
pub const SOURCE_TABLES: [SourceTable; 17] = [
    SourceTable("posts_data"),
    SourceTable("posts_labels"),
    SourceTable("posts_pivot_labels_data"),
    SourceTable("posts_authors"),
    SourceTable("colleagues_data"),
    SourceTable("canteen_menus"),
    SourceTable("canteen_data"),
    SourceTable("canteen_pivot_menus_data"),
    SourceTable("events_data"),
    SourceTable("pages"),
    SourceTable("menu_items"),
    SourceTable("users"),
    SourceTable("roles"),
    SourceTable("model_has_roles"),
    SourceTable("model_has_permissions"),
    SourceTable("permissions"),
    SourceTable("role_has_permissions"),
];

impl SourceTable {
    /// The table's name in the legacy database.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }

    /// The allow-listed table called `name`.
    ///
    /// - `name`: a legacy table name, compared exactly (MySQL on Linux
    ///   compares table names case-sensitively, and so does this).
    ///
    /// # Errors
    ///
    /// [`TableRefusal::LeftBehind`] for a table of data-migration.md §9.1
    /// that the import never reads, with the reason §9.1 gives;
    /// [`TableRefusal::NotASourceTable`] for any other name that is not on
    /// the list.
    pub fn from_name(name: &str) -> Result<Self, TableRefusal> {
        if let Some(table) = SOURCE_TABLES.iter().find(|table| table.0 == name) {
            return Ok(*table);
        }
        match NOT_MIGRATED_TABLES.iter().find(|row| row.table == name) {
            Some(row) => Err(TableRefusal::LeftBehind {
                table: row.table,
                reason: row.reason,
            }),
            None => Err(TableRefusal::NotASourceTable {
                table: name.to_owned(),
            }),
        }
    }
}

/// The allow-list as text, for messages: the names in backticks, separated
/// by commas.
fn source_table_list() -> String {
    let mut list = String::new();
    for (index, table) in SOURCE_TABLES.iter().enumerate() {
        if index > 0 {
            list.push_str(", ");
        }
        let _ = write!(list, "`{}`", table.0);
    }
    list
}

/// Why a table name was refused. The message names the table.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TableRefusal {
    /// A table of data-migration.md §9.1 that the import never reads.
    #[error(
        "refusing to read the legacy table `{table}` from the source: the import never reads it \
         (data-migration.md §9.1: {reason}); it reads only the tables of its source allow-list"
    )]
    LeftBehind {
        /// The table's name.
        table: &'static str,
        /// The reason of §9.1, as the run report shows it.
        reason: &'static str,
    },
    /// A name that is not one of the tables the import reads.
    #[error(
        "refusing to read {table:?} from the source: it is not one of the legacy tables that the \
         import reads ({})",
        source_table_list()
    )]
    NotASourceTable {
        /// The refused name, as given.
        table: String,
    },
}

/// The longest identifier MySQL accepts for a column.
const MAX_IDENTIFIER_LEN: usize = 64;

/// Whether `name` may be written into a statement as a column: 1 to 64
/// ASCII letters, digits and underscores, not starting with a digit. Every
/// legacy column name has this form (`previewToken` included), and none of
/// these needs escaping inside backticks.
fn is_plain_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    name.len() <= MAX_IDENTIFIER_LEN
        && (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Why a read of a legacy table was refused. Every message names the table.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceQueryError {
    /// The table is not on the allow-list.
    #[error(transparent)]
    Table(#[from] TableRefusal),
    /// No column was named.
    #[error(
        "refusing to read the legacy table `{table}`: name at least one column; the import \
         selects the columns it knows, never `*`"
    )]
    NoColumns {
        /// The table's name.
        table: &'static str,
    },
    /// No `ORDER BY` column was named.
    #[error(
        "refusing to read the legacy table `{table}` unordered: name at least one ORDER BY \
         column, so that every run reads the rows in the same order (data-migration.md §11.2)"
    )]
    Unordered {
        /// The table's name.
        table: &'static str,
    },
    /// A column name that cannot be written into the statement as it is.
    #[error(
        "refusing to read the legacy table `{table}`: {identifier:?} is not a column name the \
         import writes into a statement (1 to 64 ASCII letters, digits and underscores, not \
         starting with a digit)"
    )]
    BadIdentifier {
        /// The table's name.
        table: &'static str,
        /// The refused column name, as given.
        identifier: String,
    },
}

/// A `SELECT` of one allow-listed legacy table, checked and rendered.
///
/// The fields are private and [`SourceSelect::new`] checks every part, so a
/// value of this type is a statement the import may send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSelect {
    table: SourceTable,
    sql: String,
}

impl SourceSelect {
    /// Checks and renders `SELECT <columns> FROM <table> ORDER BY <order_by>`.
    ///
    /// - `table`: the legacy table; it must be on the allow-list.
    /// - `columns`: the columns to read, in the order the rows hold them; at
    ///   least one.
    /// - `order_by`: the columns that order the rows, ascending (normally
    ///   the primary key); at least one.
    ///
    /// Every identifier is written in backticks, so a legacy column whose
    /// name is a MySQL keyword (`date`, `type`) is read as a column.
    ///
    /// # Errors
    ///
    /// A [`SourceQueryError`] that names the table: a table that is not on
    /// the allow-list ([`TableRefusal`]), no column, no order, or a column
    /// name outside the plain-identifier form.
    pub fn new(table: &str, columns: &[&str], order_by: &[&str]) -> Result<Self, SourceQueryError> {
        let table = SourceTable::from_name(table)?;
        if columns.is_empty() {
            return Err(SourceQueryError::NoColumns { table: table.0 });
        }
        if order_by.is_empty() {
            return Err(SourceQueryError::Unordered { table: table.0 });
        }
        if let Some(identifier) = columns
            .iter()
            .chain(order_by)
            .find(|name| !is_plain_identifier(name))
        {
            return Err(SourceQueryError::BadIdentifier {
                table: table.0,
                identifier: (*identifier).to_owned(),
            });
        }
        let quoted = |names: &[&str]| {
            names
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let sql = format!(
            "SELECT {} FROM `{}` ORDER BY {}",
            quoted(columns),
            table.0,
            quoted(order_by)
        );
        Ok(Self { table, sql })
    }

    /// The table the statement reads.
    #[must_use]
    pub fn table(&self) -> SourceTable {
        self.table
    }

    /// The statement's text.
    #[must_use]
    pub fn sql(&self) -> &str {
        &self.sql
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use proptest::prelude::*;

    /// The tables of §9.1 that the import never reads.
    const LEFT_BEHIND: [&str; 3] = ["newsletter", "password_resets", "migrations"];

    #[test]
    fn seventeen_distinct_tables_are_read() {
        let names: BTreeSet<&str> = SOURCE_TABLES.iter().map(|table| table.name()).collect();
        assert_eq!(names.len(), SOURCE_TABLES.len(), "a table is listed twice");
        for name in names {
            assert_eq!(
                SourceTable::from_name(name).map(SourceTable::name),
                Ok(name)
            );
        }
    }

    #[test]
    fn the_newsletter_password_resets_and_migrations_are_never_read() {
        for name in LEFT_BEHIND {
            let refusal = SourceTable::from_name(name).expect_err(name);
            assert!(
                matches!(refusal, TableRefusal::LeftBehind { table, .. } if table == name),
                "{refusal:?}"
            );
            let message = refusal.to_string();
            assert!(
                message.starts_with(&format!(
                    "refusing to read the legacy table `{name}` from the source: the import never \
                     reads it (data-migration.md §9.1: "
                )),
                "{message}"
            );
        }
        let message = SourceTable::from_name("newsletter")
            .expect_err("refused")
            .to_string();
        assert!(message.contains("NWS-01"), "{message}");
    }

    #[test]
    fn the_verified_tables_of_section_9_1_are_read_and_the_rest_are_left_behind() {
        for row in NOT_MIGRATED_TABLES {
            let read = SourceTable::from_name(row.table).is_ok();
            assert_eq!(read, !LEFT_BEHIND.contains(&row.table), "{}", row.table);
        }
    }

    #[test]
    fn other_names_are_refused_with_the_list() {
        for name in [
            "newsletter_subscribers",
            "Newsletter",
            "posts_data ",
            "POSTS_DATA",
            "verseghy.posts_data",
            "`posts_data`",
            "information_schema",
            "",
        ] {
            let refusal = SourceTable::from_name(name).expect_err(name);
            assert_eq!(
                refusal,
                TableRefusal::NotASourceTable {
                    table: name.to_owned()
                }
            );
            let message = refusal.to_string();
            assert!(
                message.starts_with(&format!("refusing to read {name:?} from the source")),
                "{message}"
            );
            assert!(
                message.contains("`posts_data`, `posts_labels`"),
                "{message}"
            );
            assert!(message.contains("`role_has_permissions`)"), "{message}");
            assert!(!message.contains("`newsletter`"), "{message}");
        }
    }

    #[test]
    fn a_select_quotes_every_identifier() {
        let select = SourceSelect::new("posts_data", &["id", "date", "previewToken"], &["id"])
            .expect("a valid select");
        assert_eq!(select.table().name(), "posts_data");
        assert_eq!(
            select.sql(),
            "SELECT `id`, `date`, `previewToken` FROM `posts_data` ORDER BY `id`"
        );
        let pivot = SourceSelect::new(
            "posts_pivot_labels_data",
            &["labels_id", "posts_id"],
            &["labels_id", "posts_id"],
        )
        .expect("a valid select");
        assert_eq!(
            pivot.sql(),
            "SELECT `labels_id`, `posts_id` FROM `posts_pivot_labels_data` ORDER BY `labels_id`, \
             `posts_id`"
        );
    }

    #[test]
    fn a_select_of_a_table_off_the_list_is_refused_first() {
        // The table check comes before every other check, so even a
        // malformed read names the table that is off the list.
        assert_eq!(
            SourceSelect::new("newsletter", &[], &[]),
            Err(SourceQueryError::Table(
                SourceTable::from_name("newsletter").expect_err("left behind")
            ))
        );
        assert!(matches!(
            SourceSelect::new("newsletter_subscribers", &["email"], &["id"]),
            Err(SourceQueryError::Table(
                TableRefusal::NotASourceTable { .. }
            ))
        ));
    }

    #[test]
    fn a_select_needs_columns_an_order_and_plain_identifiers() {
        assert_eq!(
            SourceSelect::new("users", &[], &["id"]),
            Err(SourceQueryError::NoColumns { table: "users" })
        );
        assert_eq!(
            SourceSelect::new("users", &["id"], &[]),
            Err(SourceQueryError::Unordered { table: "users" })
        );
        let too_long = "c".repeat(65);
        for identifier in [
            "*",
            "",
            "1id",
            "id`",
            "id, password",
            "id FROM newsletter --",
            "e-mail",
            "naïve",
            too_long.as_str(),
        ] {
            assert_eq!(
                SourceSelect::new("users", &["id"], &[identifier]),
                Err(SourceQueryError::BadIdentifier {
                    table: "users",
                    identifier: identifier.to_owned(),
                }),
                "{identifier:?}"
            );
            assert!(SourceSelect::new("users", &[identifier], &["id"]).is_err());
        }
        assert!(SourceSelect::new("users", &["c".repeat(64).as_str()], &["_id9"]).is_ok());
        let message = SourceSelect::new("users", &["e-mail"], &["id"])
            .expect_err("refused")
            .to_string();
        assert!(
            message.starts_with("refusing to read the legacy table `users`: \"e-mail\""),
            "{message}"
        );
    }

    /// A plain identifier.
    fn identifier() -> impl Strategy<Value = String> {
        "[A-Za-z_][A-Za-z0-9_]{0,63}"
    }

    proptest! {
        #[test]
        fn only_the_listed_names_are_tables(name in prop_oneof![
            any::<String>(),
            proptest::sample::select(SOURCE_TABLES.map(SourceTable::name).to_vec())
                .prop_map(str::to_owned),
            proptest::sample::select(LEFT_BEHIND.to_vec()).prop_map(str::to_owned),
        ]) {
            let listed = SOURCE_TABLES.iter().any(|table| table.name() == name);
            match SourceTable::from_name(&name) {
                Ok(table) => prop_assert!(listed && table.name() == name),
                Err(TableRefusal::LeftBehind { table, .. }) => {
                    prop_assert!(!listed && LEFT_BEHIND.contains(&table) && table == name);
                }
                Err(TableRefusal::NotASourceTable { table }) => {
                    prop_assert!(!listed && !LEFT_BEHIND.contains(&name.as_str()));
                    prop_assert_eq!(table, name);
                }
            }
        }

        #[test]
        fn a_select_of_plain_identifiers_renders_each_in_backticks_in_order(
            table in proptest::sample::select(SOURCE_TABLES.to_vec()),
            columns in proptest::collection::vec(identifier(), 1..8),
            order_by in proptest::collection::vec(identifier(), 1..3),
        ) {
            let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
            let order_by: Vec<&str> = order_by.iter().map(String::as_str).collect();
            let select = SourceSelect::new(table.name(), &columns, &order_by)
                .expect("plain identifiers");
            let quote = |names: &[&str]| {
                names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
            };
            prop_assert_eq!(select.table(), table);
            prop_assert_eq!(
                select.sql(),
                format!(
                    "SELECT {} FROM `{}` ORDER BY {}",
                    quote(&columns),
                    table.name(),
                    quote(&order_by)
                )
            );
        }

        #[test]
        fn a_column_with_any_other_character_is_refused(
            table in proptest::sample::select(SOURCE_TABLES.to_vec()),
            prefix in "[A-Za-z_]{1,10}",
            bad in any::<char>().prop_filter("not plain", |c| !c.is_ascii_alphanumeric() && *c != '_'),
            in_order in any::<bool>(),
        ) {
            let identifier = format!("{prefix}{bad}");
            let (columns, order_by) = if in_order {
                (vec!["id"], vec![identifier.as_str()])
            } else {
                (vec![identifier.as_str()], vec!["id"])
            };
            prop_assert_eq!(
                SourceSelect::new(table.name(), &columns, &order_by),
                Err(SourceQueryError::BadIdentifier { table: table.name(), identifier: identifier.clone() })
            );
        }

        #[test]
        fn a_table_off_the_list_is_refused_whatever_the_columns(
            table in prop_oneof![
                proptest::sample::select(LEFT_BEHIND.to_vec()).prop_map(str::to_owned),
                "[a-z_]{1,30}",
            ],
            columns in proptest::collection::vec(".{0,10}", 0..4),
        ) {
            prop_assume!(SOURCE_TABLES.iter().all(|listed| listed.name() != table));
            let columns: Vec<&str> = columns.iter().map(String::as_str).collect();
            prop_assert!(matches!(
                SourceSelect::new(&table, &columns, &columns),
                Err(SourceQueryError::Table(_))
            ));
        }
    }
}
