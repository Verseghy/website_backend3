//! The not-migrated tables of the import (`domain::import_mysql::not_migrated`)
//! are the tables of docs/design/data-migration.md §9.1, so the run report
//! and the specification name the same tables in the same order.

use backend3::domain::import_mysql::not_migrated::NOT_MIGRATED_TABLES;

use crate::support::repository_root;

/// The legacy table names of the first column of §9.1's table, in order.
fn tables_of_section_9_1() -> Vec<String> {
    let document = std::fs::read_to_string(repository_root().join("docs/design/data-migration.md"))
        .expect("docs/design/data-migration.md");
    let section = document
        .split_once("\n### 9.1 Tables\n")
        .expect("section 9.1")
        .1;
    let section = section
        .split_once("\n### ")
        .map_or(section, |(body, _)| body);
    section
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .map(|rest| {
            rest.split_once('`')
                .expect("a closing backtick")
                .0
                .to_owned()
        })
        .collect()
}

#[test]
fn the_report_lists_the_tables_of_section_9_1_in_its_order() {
    let documented = tables_of_section_9_1();
    let listed: Vec<String> = NOT_MIGRATED_TABLES
        .iter()
        .map(|row| row.table.to_owned())
        .collect();
    assert!(!documented.is_empty(), "no table found in section 9.1");
    assert_eq!(listed, documented);
}
