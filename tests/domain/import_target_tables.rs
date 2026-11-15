//! The tables that the import writes (`domain::import_mysql::target`) are
//! those of docs/design/data-migration.md §4.3: the insert order of the
//! entity tables, then `revisions` and `activity_log`. The empty-target
//! guard of §4.2 checks the list's tables and skips one the target's
//! schema does not have, so a misspelt name would turn the guard off for
//! that table without a word; this test keeps the list and the
//! specification equal instead.

use backend3::domain::import_mysql::target::{IMPORT_TARGET_TABLES, READ_ONLY_TARGET_TABLES};

use crate::support::repository_root;

/// The table names in backticks on the `Insert order:` line of §4.3, up to
/// `(parents before children)`, in their order.
fn insert_order_of_section_4_3() -> Vec<String> {
    let document = std::fs::read_to_string(repository_root().join("docs/design/data-migration.md"))
        .expect("docs/design/data-migration.md");
    let section = document
        .split_once("\n### 4.3 Order of operations\n")
        .expect("section 4.3")
        .1;
    let section = section
        .split_once("\n### ")
        .map_or(section, |(body, _)| body);
    let line = section
        .lines()
        .find_map(|line| line.strip_prefix("Insert order: "))
        .expect("the insert order line of section 4.3");
    let tables = line
        .split_once(" (parents before children)")
        .expect("the end of the table list")
        .0;
    tables
        .split(", ")
        .map(|name| {
            name.strip_prefix('`')
                .and_then(|name| name.strip_suffix('`'))
                .unwrap_or_else(|| panic!("{name} is not a table name in backticks"))
                .to_owned()
        })
        .collect()
}

#[test]
fn the_guard_checks_the_tables_of_the_insert_order_then_the_history_tables() {
    let documented = insert_order_of_section_4_3();
    assert_eq!(
        documented.len() + 2,
        IMPORT_TARGET_TABLES.len(),
        "{documented:?}"
    );
    let (entities, history) = IMPORT_TARGET_TABLES.split_at(documented.len());
    assert_eq!(entities, documented);
    // The revisions and the activity-log entry of §5.4 follow the rows.
    assert_eq!(history, ["revisions", "activity_log"]);
    // `roles` is seeded by a migration and only read.
    assert_eq!(READ_ONLY_TARGET_TABLES, ["roles"]);
    assert!(!documented.iter().any(|table| table == "roles"));
}
