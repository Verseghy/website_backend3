//! The import's source allow-list (`domain::import_mysql::source_tables`)
//! against the specification and the fixture: the tables that
//! docs/design/data-migration.md §6 reads, and the legacy DDL of
//! `tests/fixtures/legacy/schema.sql`. The code, the document and the
//! fixture name the same tables, and the three that the import never reads
//! are the rest of the legacy database (NWS-01).

use std::collections::BTreeSet;

use backend3::domain::import_mysql::{
    not_migrated::NOT_MIGRATED_TABLES,
    source_tables::{SOURCE_TABLES, SourceTable, TableRefusal},
};

use crate::support::repository_root;

/// The tables of §9.1 that the import never reads.
const NEVER_READ: [&str; 3] = ["newsletter", "password_resets", "migrations"];

fn read(relative: &str) -> String {
    std::fs::read_to_string(repository_root().join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// The body of the section that starts with the heading line `heading`, up
/// to the next heading of the same or a higher level.
fn section<'a>(document: &'a str, heading: &str) -> &'a str {
    let level = heading.split(' ').next().expect("a heading marker");
    let body = document
        .split_once(&format!("\n{heading}"))
        .unwrap_or_else(|| panic!("no section {heading}"))
        .1;
    let ends = (1..=level.len())
        .filter_map(|depth| body.find(&format!("\n{} ", "#".repeat(depth))))
        .min();
    ends.map_or(body, |end| &body[..end])
}

/// The legacy tables that the headings of §6 name: "`target` from
/// `source`", and `roles`, which §6.13 verifies without copying.
fn tables_of_section_6_headings(document: &str) -> BTreeSet<String> {
    let section_6 = section(document, "## 6. Table mappings");
    let mut tables = BTreeSet::new();
    for heading in section_6.lines().filter(|line| line.starts_with("### 6.")) {
        if let Some((_, source)) = heading.split_once(" from `") {
            tables.insert(
                source
                    .split_once('`')
                    .expect("a closing backtick")
                    .0
                    .to_owned(),
            );
        } else if heading.contains("verified, not copied") {
            let name = heading
                .split_once('`')
                .and_then(|(_, rest)| rest.split_once('`'))
                .expect("a table name in backticks")
                .0;
            tables.insert(name.to_owned());
        } else {
            panic!("a §6 heading names no source table: {heading}");
        }
    }
    tables
}

/// The table names of the `CREATE TABLE` statements of the fixture.
fn tables_of_the_fixture() -> Vec<String> {
    read("tests/fixtures/legacy/schema.sql")
        .lines()
        .filter_map(|line| line.strip_prefix("CREATE TABLE `"))
        .map(|rest| {
            rest.split_once('`')
                .expect("a closing backtick")
                .0
                .to_owned()
        })
        .collect()
}

fn source_table_names() -> BTreeSet<String> {
    SOURCE_TABLES
        .iter()
        .map(|table| table.name().to_owned())
        .collect()
}

#[test]
fn the_allow_list_is_what_section_6_reads() {
    let document = read("docs/design/data-migration.md");
    let mut expected = tables_of_section_6_headings(&document);
    // §6.15 reads two more tables to verify them against the code: the
    // legacy permission names and the role-to-permission map.
    let section_6_15 = section(&document, "### 6.15 ");
    for verified in ["permissions", "role_has_permissions"] {
        assert!(
            section_6_15.contains(&format!("`{verified}`")),
            "§6.15 no longer reads {verified}: update SOURCE_TABLES and this test"
        );
        expected.insert(verified.to_owned());
    }
    assert_eq!(expected.len(), 17, "{expected:?}");
    assert_eq!(source_table_names(), expected);
}

#[test]
fn the_fixture_holds_the_allow_list_and_the_three_tables_never_read() {
    let fixture = tables_of_the_fixture();
    let unique: BTreeSet<String> = fixture.iter().cloned().collect();
    assert_eq!(fixture.len(), 20, "{fixture:?}");
    assert_eq!(unique.len(), fixture.len(), "a table is created twice");

    let mut expected = source_table_names();
    expected.extend(NEVER_READ.iter().map(|name| (*name).to_owned()));
    assert_eq!(unique, expected);
}

#[test]
fn the_tables_never_read_are_the_rest_of_section_9_1() {
    let mut never_read = Vec::new();
    for row in NOT_MIGRATED_TABLES {
        match SourceTable::from_name(row.table) {
            Ok(_) => {}
            Err(TableRefusal::LeftBehind { table, reason }) => {
                assert_eq!(reason, row.reason, "{table}");
                never_read.push(table);
            }
            Err(other) => panic!("{}: {other}", row.table),
        }
    }
    assert_eq!(never_read, NEVER_READ);
}

#[test]
fn the_fixture_names_no_database_and_holds_no_rows() {
    let fixture = read("tests/fixtures/legacy/schema.sql");
    let code: Vec<&str> = fixture
        .lines()
        .filter(|line| !line.starts_with("--") && !line.is_empty())
        .collect();
    // Every statement starts at the beginning of a line; column and index
    // lines are indented, and `)` closes a table.
    for statement in code
        .iter()
        .filter(|line| !line.starts_with(' ') && !line.starts_with(')'))
    {
        assert!(
            statement.starts_with("CREATE TABLE `")
                || *statement == "ALTER DATABASE CHARACTER SET latin1 COLLATE latin1_swedish_ci;",
            "only the schema belongs in the fixture: {statement}"
        );
    }
    // No `AUTO_INCREMENT=<n>` table option: those are one dump's counters.
    for line in &code {
        assert!(
            !line.contains("AUTO_INCREMENT="),
            "a dump's counter is left in: {line}"
        );
    }
}
