//! The pure parts of the import's checks on its target, backend3's own
//! PostgreSQL database (docs/design/data-migration.md §4.2, §6.13, §6.15).
//!
//! Before the import reads the source, the task runs two checks inside the
//! import's one transaction (`tasks::import_mysql::target`):
//!
//! 1. **The code tables.** The import maps the legacy role names to the
//!    rows of `roles` (§6.13) and writes the legacy grants by name into
//!    `user_permissions` (§6.15). Both must therefore agree with the code
//!    of `src/domain/authz.rs` (ADR 0007 §1, §3): `roles` holds exactly the
//!    four roles of `Role::ALL` with their ids ([`check_roles`]), and the
//!    one CHECK on `user_permissions` admits exactly the sixteen names of
//!    `Permission::ALL`, so `edit newsletter` is not among them
//!    ([`check_permission_constraint`]). A target migrated by another build
//!    fails here, before anything is read or written.
//! 2. **The empty target.** The import never merges into existing data
//!    (§4.2, §11.2): every table it writes, [`IMPORT_TARGET_TABLES`], must
//!    be empty, and the first that holds a row is named ([`NotEmpty`]).
//!    The task locks those tables for the rest of the transaction first
//!    ([`lock_statement`]), so the check still holds when the import
//!    commits. `roles` is seeded by a migration and only read (§6.13), so
//!    it is not on the list.
//!
//! The empty target is what the pre-import invariant of `AGENTS.md` keeps
//! true in production: nothing writes a row into these tables before the
//! cut-over import (research: DECISIONS R9 (D10)).

use std::collections::BTreeSet;

use crate::domain::authz::{Permission, Role};

/// The tables the import writes, in the order of data-migration.md §4.3:
/// the target tables of §6 in foreign-key order, then `revisions` and
/// `activity_log`, which receive the `import` revisions and the per-table
/// activity entries (§5.4). The guard checks them in this order and names
/// the first that holds a row.
///
/// A table on the list that the target's schema does not have yet (an
/// entity whose migration has not merged) holds no row, so the guard skips
/// it, with a warning that names it. Two tests keep a skip from hiding a
/// table for good: `tests/domain/import_target_tables.rs` checks the list
/// against the insert order of data-migration.md §4.3, so a name on it
/// cannot be misspelt, and `tests/tasks/import_mysql_target.rs` checks that
/// every table of the migrated schema is on it, on
/// [`READ_ONLY_TARGET_TABLES`] or on its list of the tables that the import
/// never writes, so a migration cannot create an entity table under another
/// name.
pub const IMPORT_TARGET_TABLES: [&str; 16] = [
    "users",
    "user_roles",
    "user_permissions",
    "authors",
    "labels",
    "posts",
    "post_labels",
    "dishes",
    "canteen_days",
    "canteen_day_dishes",
    "events",
    "colleagues",
    "pages",
    "menu_items",
    "revisions",
    "activity_log",
];

/// The tables of the target that the import reads and never writes: `roles`,
/// seeded by its migration (§6.13).
pub const READ_ONLY_TARGET_TABLES: [&str; 1] = ["roles"];

/// The lock that keeps every table of `existing` as the guard found it until
/// the import's transaction ends: `LOCK TABLE "users", ... IN SHARE ROW
/// EXCLUSIVE MODE`.
///
/// - `existing`: the tables of [`IMPORT_TARGET_TABLES`] that the target
///   has, in that order; at least one.
///
/// `SHARE ROW EXCLUSIVE` lets others read, waits for every open write to
/// end, and keeps any other transaction from writing these tables until the
/// import commits or rolls back, while the import's own transaction may
/// still insert into them. It conflicts with itself, so two imports could
/// not both hold it, although the task lock already keeps them apart.
///
/// Returns `None` when `existing` is empty.
#[must_use]
pub fn lock_statement(existing: &[&str]) -> Option<String> {
    if existing.is_empty() {
        return None;
    }
    let tables = existing
        .iter()
        .map(|table| format!("\"{table}\""))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("LOCK TABLE {tables} IN SHARE ROW EXCLUSIVE MODE"))
}

/// The query whether `table` holds a row: `SELECT EXISTS (SELECT 1 FROM
/// "users")`, which stops at the first row.
///
/// - `table`: one of [`IMPORT_TARGET_TABLES`].
#[must_use]
pub fn has_rows_statement(table: &str) -> String {
    format!("SELECT EXISTS (SELECT 1 FROM \"{table}\")")
}

/// Why the import refuses a target that already holds data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "refusing to import: the target table `{table}` already holds a row. The import never \
     merges into existing data, so it runs only into a freshly migrated, empty database \
     (data-migration.md §4.2, §11.2): recreate the target database, migrate it with \
     `backend3-cli db migrate`, and run the import again before anything else writes to it"
)]
pub struct NotEmpty {
    /// The first table of [`IMPORT_TARGET_TABLES`] that holds a row.
    pub table: &'static str,
}

/// A role row: its id and name.
pub type RoleRow = (i64, String);

/// Why the target's `roles` or its `user_permissions` CHECK disagree with
/// the code. Every message names the difference.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodeTableMismatch {
    /// `roles` does not hold exactly the four roles of `Role::ALL`.
    #[error(
        "refusing to import: the target's `roles` table differs from the roles of the code \
         (src/domain/authz.rs, ADR 0007 §3), to which the import maps the legacy role names \
         (data-migration.md §6.13): {}. Migrate the target with this build's migrations, whose \
         roles_and_grants migration seeds the four roles",
        describe_roles(missing, unexpected)
    )]
    Roles {
        /// The code's rows that the table lacks, in id order.
        missing: Vec<RoleRow>,
        /// The table's rows that the code does not define, in id order.
        unexpected: Vec<RoleRow>,
    },
    /// `user_permissions` has no CHECK, or more than one.
    #[error(
        "refusing to import: `user_permissions` has {count} CHECK constraints, where the \
         migrations create exactly one, user_permissions_permission_known, which admits the \
         sixteen permission names of the code (src/domain/authz.rs, ADR 0007 §1). Migrate the \
         target with this build's migrations"
    )]
    PermissionCheckCount {
        /// How many CHECK constraints the table has.
        count: usize,
    },
    /// The CHECK is not a list of names.
    #[error(
        "refusing to import: the CHECK {name} on `user_permissions` is not a list of permission \
         names (its definition is {definition}), so the import cannot tell which names it \
         admits (ADR 0007 §1). Migrate the target with this build's migrations"
    )]
    PermissionCheckUnreadable {
        /// The constraint's name.
        name: String,
        /// Its definition, as `pg_get_constraintdef` writes it.
        definition: String,
    },
    /// The CHECK's list differs from `Permission::ALL`.
    #[error(
        "refusing to import: the CHECK {name} on `user_permissions` does not admit exactly the \
         sixteen permissions of the code (src/domain/authz.rs, ADR 0007 §1), which the import \
         writes the legacy grants as (data-migration.md §6.15): {}. Migrate the target with this \
         build's migrations",
        describe_permissions(not_admitted, unknown)
    )]
    Permissions {
        /// The constraint's name.
        name: String,
        /// The code's names that it does not admit.
        not_admitted: Vec<String>,
        /// The names it admits that the code does not define, such as
        /// `edit newsletter`.
        unknown: Vec<String>,
    },
}

/// The rows of [`CodeTableMismatch::Roles`] as text: the missing rows, then
/// the unexpected ones, each as its id and its name in backticks.
fn describe_roles(missing: &[RoleRow], unexpected: &[RoleRow]) -> String {
    let list = |rows: &[RoleRow]| {
        rows.iter()
            .map(|(id, name)| format!("{id} `{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!("missing {}", list(missing)));
    }
    if !unexpected.is_empty() {
        parts.push(format!("unexpected {}", list(unexpected)));
    }
    parts.join("; ")
}

/// The names of [`CodeTableMismatch::Permissions`] as text: the code's names
/// that the CHECK does not admit, then the unknown names it admits.
fn describe_permissions(not_admitted: &[String], unknown: &[String]) -> String {
    let list = |names: &[String]| {
        names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = Vec::new();
    if !not_admitted.is_empty() {
        parts.push(format!("not admitted {}", list(not_admitted)));
    }
    if !unknown.is_empty() {
        parts.push(format!(
            "admitted but unknown to the code {}",
            list(unknown)
        ));
    }
    parts.join("; ")
}

/// Checks that the target's `roles` rows are exactly the roles of
/// `Role::ALL`, with their ids and names.
///
/// - `found`: every row of `roles` as `(id, name)`, in any order.
///
/// # Errors
///
/// [`CodeTableMismatch::Roles`] with the rows the table lacks and the rows
/// the code does not define; a row whose name changed counts as both.
pub fn check_roles(found: &[RoleRow]) -> Result<(), CodeTableMismatch> {
    let expected: BTreeSet<RoleRow> = Role::ALL
        .iter()
        .map(|role| (role.id(), role.name().to_owned()))
        .collect();
    let found: BTreeSet<RoleRow> = found.iter().cloned().collect();
    if found == expected {
        return Ok(());
    }
    Err(CodeTableMismatch::Roles {
        missing: expected.difference(&found).cloned().collect(),
        unexpected: found.difference(&expected).cloned().collect(),
    })
}

/// The names that a CHECK of the form PostgreSQL writes for `permission IN
/// ('a', 'b')` admits: `CHECK ((permission = ANY (ARRAY['a'::text,
/// 'b'::text])))`, as `pg_get_constraintdef` returns it.
///
/// - `definition`: the deparsed constraint.
///
/// A quote inside a name is written twice (`'it''s'`), as in SQL. Returns
/// `None` for any other form, so a CHECK that is not a plain list of names
/// is never read as one.
#[must_use]
pub fn admitted_permissions(definition: &str) -> Option<Vec<String>> {
    let list = definition
        .strip_prefix("CHECK ((permission = ANY (ARRAY[")?
        .strip_suffix("])))")?;
    let mut names = Vec::new();
    let mut rest = list;
    loop {
        rest = rest.strip_prefix('\'')?;
        let mut name = String::new();
        let mut characters = rest.char_indices();
        let end = loop {
            let (index, character) = characters.next()?;
            if character != '\'' {
                name.push(character);
                continue;
            }
            // A doubled quote is a quote inside the name.
            if rest[index + 1..].starts_with('\'') {
                name.push('\'');
                characters.next();
                continue;
            }
            break index + 1;
        };
        names.push(name);
        rest = rest[end..].strip_prefix("::text")?;
        if rest.is_empty() {
            return Some(names);
        }
        rest = rest.strip_prefix(", ")?;
    }
}

/// Checks that `user_permissions` has exactly one CHECK and that it admits
/// exactly the names of `Permission::ALL`.
///
/// - `checks`: every CHECK constraint of `user_permissions` as `(name,
///   definition)`, the definition as `pg_get_constraintdef` writes it.
///
/// The names are compared as sets: the order of the list does not change
/// what the CHECK admits.
///
/// # Errors
///
/// [`CodeTableMismatch::PermissionCheckCount`] for no CHECK or several,
/// [`CodeTableMismatch::PermissionCheckUnreadable`] for a CHECK that is not
/// a list of names, and [`CodeTableMismatch::Permissions`] with the code's
/// names it does not admit and the names it admits that the code does not
/// define.
pub fn check_permission_constraint(checks: &[(String, String)]) -> Result<(), CodeTableMismatch> {
    let [(name, definition)] = checks else {
        return Err(CodeTableMismatch::PermissionCheckCount {
            count: checks.len(),
        });
    };
    let admitted: BTreeSet<String> = admitted_permissions(definition)
        .ok_or_else(|| CodeTableMismatch::PermissionCheckUnreadable {
            name: name.clone(),
            definition: definition.clone(),
        })?
        .into_iter()
        .collect();
    let code: Vec<&str> = Permission::ALL
        .iter()
        .map(|permission| permission.name())
        .collect();
    let not_admitted: Vec<String> = code
        .iter()
        .filter(|name| !admitted.contains(**name))
        .map(|name| (*name).to_owned())
        .collect();
    let unknown: Vec<String> = admitted
        .iter()
        .filter(|name| !code.contains(&name.as_str()))
        .cloned()
        .collect();
    if not_admitted.is_empty() && unknown.is_empty() {
        return Ok(());
    }
    Err(CodeTableMismatch::Permissions {
        name: name.clone(),
        not_admitted,
        unknown,
    })
}

/// The code's permission CHECK as PostgreSQL deparses it, for tests and
/// messages: `CHECK ((permission = ANY (ARRAY['edit posts'::text, ...])))`.
///
/// - `names`: the names in the order of the list.
#[must_use]
pub fn permission_check_definition<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let list = names
        .into_iter()
        .map(|name| format!("'{}'::text", name.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("CHECK ((permission = ANY (ARRAY[{list}])))")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn code_roles() -> Vec<RoleRow> {
        Role::ALL
            .iter()
            .map(|role| (role.id(), role.name().to_owned()))
            .collect()
    }

    fn code_definition() -> String {
        permission_check_definition(Permission::ALL.iter().map(|permission| permission.name()))
    }

    #[test]
    fn the_target_tables_are_distinct_plain_names_from_users_to_the_history_tables() {
        assert_eq!(IMPORT_TARGET_TABLES[0], "users");
        assert_eq!(
            &IMPORT_TARGET_TABLES[IMPORT_TARGET_TABLES.len() - 2..],
            ["revisions", "activity_log"]
        );
        let unique: BTreeSet<&str> = IMPORT_TARGET_TABLES.iter().copied().collect();
        assert_eq!(unique.len(), IMPORT_TARGET_TABLES.len());
        for table in READ_ONLY_TARGET_TABLES {
            assert!(!unique.contains(table), "{table}");
        }
        for table in IMPORT_TARGET_TABLES {
            assert!(
                table
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                "{table} is written into statements in double quotes"
            );
        }
    }

    #[test]
    fn the_guard_statements_quote_each_table() {
        assert_eq!(lock_statement(&[]), None);
        assert_eq!(
            lock_statement(&["users", "activity_log"]).expect("two tables"),
            "LOCK TABLE \"users\", \"activity_log\" IN SHARE ROW EXCLUSIVE MODE"
        );
        assert_eq!(
            has_rows_statement("revisions"),
            "SELECT EXISTS (SELECT 1 FROM \"revisions\")"
        );
    }

    #[test]
    fn a_non_empty_table_is_named_with_what_to_do() {
        assert_eq!(
            NotEmpty {
                table: "activity_log"
            }
            .to_string(),
            "refusing to import: the target table `activity_log` already holds a row. The import \
             never merges into existing data, so it runs only into a freshly migrated, empty \
             database (data-migration.md §4.2, §11.2): recreate the target database, migrate it \
             with `backend3-cli db migrate`, and run the import again before anything else \
             writes to it"
        );
    }

    #[test]
    fn the_code_roles_pass_in_any_order() {
        let mut found = code_roles();
        assert_eq!(check_roles(&found), Ok(()));
        found.reverse();
        assert_eq!(check_roles(&found), Ok(()));
    }

    #[test]
    fn every_role_difference_is_named() {
        let mut found = code_roles();
        found.retain(|(id, _)| *id != 4);
        found.push((5, "editor".to_owned()));
        found[0].1 = "author".to_owned();
        let mismatch = check_roles(&found).expect_err("differs");
        assert_eq!(
            mismatch,
            CodeTableMismatch::Roles {
                missing: vec![(1, "writer".to_owned()), (4, "admin".to_owned())],
                unexpected: vec![(1, "author".to_owned()), (5, "editor".to_owned())],
            }
        );
        assert_eq!(
            mismatch.to_string(),
            "refusing to import: the target's `roles` table differs from the roles of the code \
             (src/domain/authz.rs, ADR 0007 §3), to which the import maps the legacy role names \
             (data-migration.md §6.13): missing 1 `writer`, 4 `admin`; unexpected 1 `author`, 5 \
             `editor`. Migrate the target with this build's migrations, whose roles_and_grants \
             migration seeds the four roles"
        );
        let empty = check_roles(&[]).expect_err("no roles").to_string();
        assert!(
            empty.contains(": missing 1 `writer`, 2 `secretary`, 3 `supervisor`, 4 `admin`. "),
            "{empty}"
        );
    }

    #[test]
    fn the_deparsed_check_reads_back_as_its_names() {
        // The form `tests/migration/authz.rs` reads from the migrated schema.
        assert_eq!(
            admitted_permissions(
                "CHECK ((permission = ANY (ARRAY['edit posts'::text, 'view logs'::text])))"
            ),
            Some(vec!["edit posts".to_owned(), "view logs".to_owned()])
        );
        assert_eq!(
            admitted_permissions("CHECK ((permission = ANY (ARRAY['it''s'::text])))"),
            Some(vec!["it's".to_owned()])
        );
        for unreadable in [
            "CHECK (true)",
            "CHECK ((permission <> ''::text))",
            "CHECK ((permission = ANY (ARRAY[])))",
            "CHECK ((permission = ANY (ARRAY['a'::character varying])))",
            "CHECK ((permission = ANY (ARRAY['a'::text,'b'::text])))",
            "CHECK ((permission = ANY (ARRAY['a'::text, 'b])))",
            "CHECK ((name = ANY (ARRAY['a'::text])))",
            "CHECK ((permission = ANY (ARRAY['a'::text]))) NOT VALID",
        ] {
            assert_eq!(admitted_permissions(unreadable), None, "{unreadable}");
        }
    }

    #[test]
    fn the_code_check_passes() {
        let checks = [(
            "user_permissions_permission_known".to_owned(),
            code_definition(),
        )];
        assert_eq!(check_permission_constraint(&checks), Ok(()));
    }

    #[test]
    fn a_check_that_admits_edit_newsletter_is_refused_naming_it() {
        let names: Vec<&str> = Permission::ALL
            .iter()
            .map(|permission| permission.name())
            .filter(|name| *name != "view logs")
            .chain(["edit newsletter"])
            .collect();
        let checks = [(
            "user_permissions_permission_known".to_owned(),
            permission_check_definition(names),
        )];
        let mismatch = check_permission_constraint(&checks).expect_err("differs");
        assert_eq!(
            mismatch.to_string(),
            "refusing to import: the CHECK user_permissions_permission_known on \
             `user_permissions` does not admit exactly the sixteen permissions of the code \
             (src/domain/authz.rs, ADR 0007 §1), which the import writes the legacy grants as \
             (data-migration.md §6.15): not admitted `view logs`; admitted but unknown to the \
             code `edit newsletter`. Migrate the target with this build's migrations"
        );
    }

    #[test]
    fn no_check_several_checks_and_other_forms_are_refused() {
        assert_eq!(
            check_permission_constraint(&[]),
            Err(CodeTableMismatch::PermissionCheckCount { count: 0 })
        );
        let two = [
            ("a".to_owned(), code_definition()),
            ("b".to_owned(), code_definition()),
        ];
        let message = check_permission_constraint(&two)
            .expect_err("two")
            .to_string();
        assert!(
            message.starts_with("refusing to import: `user_permissions` has 2 CHECK constraints"),
            "{message}"
        );
        let other = [(
            "loose".to_owned(),
            "CHECK ((length(permission) > 0))".to_owned(),
        )];
        assert_eq!(
            check_permission_constraint(&other)
                .expect_err("unreadable")
                .to_string(),
            "refusing to import: the CHECK loose on `user_permissions` is not a list of \
             permission names (its definition is CHECK ((length(permission) > 0))), so the \
             import cannot tell which names it admits (ADR 0007 §1). Migrate the target with \
             this build's migrations"
        );
    }

    proptest! {
        #[test]
        fn the_guard_statements_name_each_table_once_in_double_quotes(
            tables in proptest::collection::vec("[a-z_]{1,20}", 0..8),
        ) {
            let tables: Vec<&str> = tables.iter().map(String::as_str).collect();
            match lock_statement(&tables) {
                None => prop_assert!(tables.is_empty()),
                Some(lock) => {
                    let quoted: Vec<String> = tables.iter().map(|table| format!("\"{table}\"")).collect();
                    prop_assert_eq!(
                        lock,
                        format!("LOCK TABLE {} IN SHARE ROW EXCLUSIVE MODE", quoted.join(", "))
                    );
                }
            }
            for table in tables {
                prop_assert_eq!(
                    has_rows_statement(table),
                    format!("SELECT EXISTS (SELECT 1 FROM \"{table}\")")
                );
            }
        }

        #[test]
        fn a_deparsed_list_reads_back_unchanged(
            names in proptest::collection::vec("[ -~]{0,20}", 1..8),
        ) {
            let definition = permission_check_definition(names.iter().map(String::as_str));
            prop_assert_eq!(admitted_permissions(&definition), Some(names));
        }

        #[test]
        fn any_text_reads_without_panicking(definition in ".{0,80}") {
            let _ = admitted_permissions(&definition);
            let _ = admitted_permissions(&format!("CHECK ((permission = ANY (ARRAY[{definition}])))"));
        }

        #[test]
        fn the_check_passes_exactly_for_the_code_names_in_any_order(
            subset in proptest::sample::subsequence(Permission::ALL.to_vec(), 0..=16),
            extra in proptest::option::of("[a-z ]{1,16}"),
            rotation in any::<prop::sample::Index>(),
        ) {
            let mut names: Vec<String> = subset.iter().map(|permission| permission.name().to_owned()).collect();
            names.extend(extra.clone());
            if !names.is_empty() {
                let by = rotation.index(names.len());
                names.rotate_left(by);
            }
            let checks = [(
                "user_permissions_permission_known".to_owned(),
                permission_check_definition(names.iter().map(String::as_str)),
            )];
            let unknown_extra = extra.is_some_and(|name| Permission::from_name(&name).is_none());
            let passes = subset.len() == 16 && !unknown_extra;
            match check_permission_constraint(&checks) {
                Ok(()) => prop_assert!(passes),
                Err(CodeTableMismatch::Permissions { not_admitted, unknown, .. }) => {
                    prop_assert!(!passes);
                    prop_assert_eq!(not_admitted.len(), 16 - subset.len());
                    prop_assert_eq!(!unknown.is_empty(), unknown_extra);
                }
                // An empty subset with no extra deparses as an empty array.
                Err(CodeTableMismatch::PermissionCheckUnreadable { .. }) => prop_assert!(names.is_empty()),
                Err(other) => prop_assert!(false, "{}", other),
            }
        }

        #[test]
        fn the_roles_pass_exactly_for_the_code_rows(
            rows in proptest::collection::vec((1_i64..7, prop_oneof![
                Just("writer".to_owned()),
                Just("secretary".to_owned()),
                Just("supervisor".to_owned()),
                Just("admin".to_owned()),
                "[a-z]{1,8}",
            ]), 0..7),
        ) {
            let found: BTreeSet<RoleRow> = rows.iter().cloned().collect();
            let expected: BTreeSet<RoleRow> = code_roles().into_iter().collect();
            prop_assert_eq!(check_roles(&rows).is_ok(), found == expected);
        }
    }
}
