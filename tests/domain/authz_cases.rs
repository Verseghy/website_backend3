//! The access case table, `tests/fixtures/authz_cases.json`, against the
//! server's side of ADR 0007 §7: the access table of `domain::authz`
//! ([`RESOURCE_ACCESS`]), the access map that `/auth/me` sends, and the
//! record-level rules.
//!
//! Why the table is a file: the admin's `authProvider.canAccess` answers
//! from the access map that `/auth/me` sends, and mirrors the record-level
//! rules in TypeScript. The admin's Vitest test
//! (`admin/src/auth/authz-cases.test.ts`) runs the same file, so a rule that
//! changes on one side and not on the other fails one of the two tests
//! (ADR 0007 §7, "so the two cannot drift").
//!
//! Each case describes an account as `/auth/me` describes it: its effective
//! permissions and whether it holds the `admin` role. Those two are all that
//! the access map and the record-level rules read (the access map's
//! property `the_access_map_reads_only_what_the_requirements_read` in
//! `src/domain/authz.rs` says so), so the test gives the account those
//! permissions as extra grants, and the `admin` role when the case says so.
//! It then checks, for every case:
//!
//! 1. the case's `access` is exactly the list `/auth/me` sends that account
//!    for the case's resource, so the admin's side runs against the server's
//!    real answer;
//! 2. `expected` is the answer of the server's rules: the action is in the
//!    access map, and the resource's record-level rule, if any, allows the
//!    record ([`record_rule_allows`]).
//!
//! The file itself is checked too: an unknown or a missing member (`record`
//! included, which may be `null` but not left out), a repeated case name,
//! an unknown or repeated permission, a permission list out of the order
//! that `/auth/me` sends, an admin-role account without every permission
//! (the role gives them all, ADR 0007 §2), and an access list out of order
//! all fail with the case's name. Every row of the access table needs a
//! case that allows and a case that refuses, so a resource slice that adds
//! its row adds its cases with it.

use std::collections::{BTreeMap, BTreeSet};

use backend3::domain::authz::{
    AccessAction, AccountGrants, Permission, RESOURCE_ACCESS, ResourceAccess, Role, access_map,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::support::repository_root;

/// The case table's path below the repository root.
const CASES: &str = "tests/fixtures/authz_cases.json";

/// The case table: what it is, and its cases.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    /// What the file holds, for its readers.
    description: String,
    /// The cases, in no particular order.
    cases: Vec<Case>,
}

/// One access question about one account.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    /// What the case shows, unique in the file.
    name: String,
    /// The account's effective permissions, as `/auth/me` sends them.
    permissions: Vec<String>,
    /// Whether the account holds the `admin` role.
    is_admin: bool,
    /// The ra-core resource the question is about; any text, since ra-core
    /// may ask about a resource that has no row.
    resource: String,
    /// The action asked about; any text, since ra-core may ask about an
    /// action that backend3 never offers.
    action: String,
    /// The record the question is asked for, or `null` for none. The member
    /// itself is required: serde's derive would read a missing `Option`
    /// member as `None`, and `deserialize_with` turns that off, so a case
    /// that forgot its record fails like the admin's reader fails it.
    #[serde(deserialize_with = "Option::deserialize")]
    record: Option<Map<String, Value>>,
    /// The actions `/auth/me` sends the account for `resource`.
    access: Vec<String>,
    /// The answer.
    expected: bool,
}

/// Reads and parses the case table; panics with what is wrong.
fn read_cases() -> CaseFile {
    let path = repository_root().join(CASES);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!("{CASES} is not the case table its module documentation describes: {error}")
    })
}

/// The account a case describes: its permissions as extra grants, and the
/// `admin` role when the case says so. Panics, naming the case, when the
/// permissions are not what `/auth/me` would send.
fn account_of(case: &Case) -> AccountGrants {
    let name = &case.name;
    let extra: Vec<Permission> = case
        .permissions
        .iter()
        .map(|permission| {
            Permission::from_name(permission).unwrap_or_else(|| {
                panic!("{name}: {permission:?} is none of the sixteen permissions of ADR 0007 §1")
            })
        })
        .collect();
    assert!(
        extra.windows(2).all(|pair| pair[0] < pair[1]),
        "{name}: list the permissions once each, in the order of ADR 0007 §1, as /auth/me sends \
         them"
    );
    let grants = AccountGrants {
        roles: if case.is_admin {
            BTreeSet::from([Role::Admin])
        } else {
            BTreeSet::new()
        },
        extra: extra.iter().copied().collect(),
    };
    assert_eq!(
        grants.effective(),
        extra.iter().copied().collect::<BTreeSet<_>>(),
        "{name}: an admin-role account holds every permission (ADR 0007 §2); list all sixteen"
    );
    grants
}

/// Whether the record-level rule of `resource` allows `action` on `record`
/// for an account with `grants`: the Rust side of the rules that the
/// admin's `RECORD_RULES` (`admin/src/auth/access.ts`) mirrors.
///
/// - `resource`: the case's resource.
/// - `action`: the case's action, one the access map allowed.
/// - `record`: the case's record.
/// - `grants`: the case's account.
///
/// No resource has a record-level rule yet, so every record passes. The
/// slices that add one (published posts, the user guards of ADR 0007 §6)
/// read the case's record into the rule's input here and call the rule's
/// function in `src/domain/authz.rs`, the one their handlers call, and add
/// the same rule to the admin's mirror.
fn record_rule_allows(
    resource: &str,
    action: AccessAction,
    record: &Map<String, Value>,
    grants: &AccountGrants,
) -> bool {
    let _ = (resource, action, record, grants);
    true
}

/// The server's answer to a case: the action is in the account's access
/// map for the resource, and the record, if any, passes the resource's
/// record-level rule.
fn server_answer(case: &Case, grants: &AccountGrants, allowed: &[AccessAction]) -> bool {
    let Some(action) = AccessAction::from_name(&case.action) else {
        return false;
    };
    allowed.contains(&action)
        && case
            .record
            .as_ref()
            .is_none_or(|record| record_rule_allows(&case.resource, action, record, grants))
}

#[test]
fn the_case_table_agrees_with_the_server() {
    let file = read_cases();
    assert!(
        file.description.contains("ADR 0007"),
        "{CASES}: the description says what the file is"
    );
    assert!(!file.cases.is_empty(), "{CASES} holds no case");
    let mut names = BTreeSet::new();
    for case in &file.cases {
        let name = &case.name;
        assert!(!name.trim().is_empty(), "{CASES}: a case without a name");
        assert!(
            names.insert(name.as_str()),
            "{name}: two cases have this name"
        );

        let grants = account_of(case);
        let map = access_map(RESOURCE_ACCESS, &grants);
        let allowed: Vec<AccessAction> = map
            .get(case.resource.as_str())
            .map(|actions| actions.iter().copied().collect())
            .unwrap_or_default();
        let sent: Vec<&str> = allowed.iter().map(|action| action.name()).collect();
        assert_eq!(
            case.access, sent,
            "{name}: /auth/me sends this account {sent:?} for {:?}; write that in the case's \
             access",
            case.resource
        );

        assert_eq!(
            server_answer(case, &grants, &allowed),
            case.expected,
            "{name}: the server's rules answer the other way"
        );
    }
    assert_eq!(
        uncovered_rows(RESOURCE_ACCESS, &file.cases),
        Vec::<String>::new(),
        "{CASES}"
    );
}

/// The rows of `table` that lack a case that allows or a case that refuses,
/// one line per missing kind; empty when every row has both.
///
/// - `table`: the access rows; `RESOURCE_ACCESS`, or a probe table.
/// - `cases`: the cases; a case counts for the row whose resource it asks
///   about, and a case about a resource without a row counts for none.
fn uncovered_rows(table: &[ResourceAccess], cases: &[Case]) -> Vec<String> {
    let mut outcomes: BTreeMap<&str, BTreeSet<bool>> = BTreeMap::new();
    for case in cases {
        outcomes
            .entry(case.resource.as_str())
            .or_default()
            .insert(case.expected);
    }
    let mut problems = Vec::new();
    for row in table {
        let seen = outcomes.get(row.resource);
        for (outcome, kind) in [(true, "allows"), (false, "refuses")] {
            if !seen.is_some_and(|seen| seen.contains(&outcome)) {
                problems.push(format!(
                    "add a case that {kind} on {:?}, the access table's row",
                    row.resource
                ));
            }
        }
    }
    problems
}

/// A case about `resource` whose answer is `expected`, for the coverage
/// check's own tests; the rest of it is never read there.
fn probe_case(resource: &str, expected: bool) -> Case {
    Case {
        name: format!("{resource} {expected}"),
        permissions: Vec::new(),
        is_admin: false,
        resource: resource.to_owned(),
        action: "list".to_owned(),
        record: None,
        access: Vec::new(),
        expected,
    }
}

#[test]
fn the_coverage_check_wants_an_allowing_and_a_refusing_case_per_row() {
    use backend3::domain::authz::{ActionRule, Requirement};

    const LIST: &[ActionRule] = &[ActionRule::new(
        AccessAction::List,
        &[Requirement::Permission(Permission::EditLabels)],
    )];
    // Invented resource names: no resource slice owns them.
    const PROBE_TABLE: &[ResourceAccess] = &[
        ResourceAccess {
            resource: "probe-both",
            rules: LIST,
        },
        ResourceAccess {
            resource: "probe-refused",
            rules: LIST,
        },
        ResourceAccess {
            resource: "probe-allowed",
            rules: LIST,
        },
        ResourceAccess {
            resource: "probe-none",
            rules: LIST,
        },
    ];
    let cases = [
        probe_case("probe-both", true),
        probe_case("probe-both", false),
        probe_case("probe-refused", false),
        probe_case("probe-allowed", true),
        // A case about a resource without a row covers no row.
        probe_case("probe-elsewhere", true),
        probe_case("probe-elsewhere", false),
    ];
    assert_eq!(
        uncovered_rows(PROBE_TABLE, &cases),
        [
            r#"add a case that allows on "probe-refused", the access table's row"#,
            r#"add a case that refuses on "probe-allowed", the access table's row"#,
            r#"add a case that allows on "probe-none", the access table's row"#,
            r#"add a case that refuses on "probe-none", the access table's row"#,
        ]
    );
    assert_eq!(
        uncovered_rows(&PROBE_TABLE[..1], &cases),
        Vec::<String>::new()
    );
    assert_eq!(uncovered_rows(&[], &[]), Vec::<String>::new());
}

#[test]
fn every_access_list_of_the_cases_names_known_actions_in_order() {
    // What the admin's reader of /auth/me accepts (admin/src/auth/authz.ts):
    // known actions, each once; the server sends them in AccessAction order.
    for case in read_cases().cases {
        let actions: Vec<AccessAction> = case
            .access
            .iter()
            .map(|action| {
                AccessAction::from_name(action)
                    .unwrap_or_else(|| panic!("{}: {action:?} is no access action", case.name))
            })
            .collect();
        assert!(
            actions.windows(2).all(|pair| pair[0] < pair[1]),
            "{}: list the actions once each, in AccessAction order",
            case.name
        );
    }
}

#[test]
fn the_file_check_refuses_a_case_that_is_not_what_me_sends() {
    // The checks of `account_of`, each on a case built to fail it.
    let case = |permissions: &[&str], is_admin: bool| Case {
        name: "probe".to_owned(),
        permissions: permissions.iter().map(|name| (*name).to_owned()).collect(),
        is_admin,
        resource: "probes".to_owned(),
        action: "list".to_owned(),
        record: None,
        access: Vec::new(),
        expected: false,
    };
    for (permissions, is_admin, message) in [
        (&["edit newsletter"][..], false, "none of the sixteen"),
        (&["edit labels", "edit posts"][..], false, "in the order"),
        (&["edit posts", "edit posts"][..], false, "in the order"),
        (&["edit posts"][..], true, "list all sixteen"),
    ] {
        let probe = case(permissions, is_admin);
        let panic =
            std::panic::catch_unwind(|| account_of(&probe)).expect_err("the case is refused");
        let text = panic.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(text.contains(message), "{permissions:?}: {text}");
    }
    let all: Vec<&str> = Permission::ALL.iter().map(|p| p.name()).collect();
    assert_eq!(
        account_of(&case(&all, true)).roles,
        BTreeSet::from([Role::Admin])
    );
    assert!(account_of(&case(&[], false)).effective().is_empty());
}

#[test]
fn an_unknown_member_or_a_missing_one_is_refused() {
    let case = serde_json::json!({
        "name": "probe",
        "permissions": [],
        "is_admin": false,
        "resource": "probes",
        "action": "list",
        "record": null,
        "access": [],
        "expected": false
    });
    assert!(serde_json::from_value::<Case>(case.clone()).is_ok());
    let mut unknown = case.clone();
    unknown["roles"] = serde_json::json!(["writer"]);
    assert!(serde_json::from_value::<Case>(unknown).is_err());
    // Every member is required, `record` too, although it may be null.
    for member in [
        "name",
        "permissions",
        "is_admin",
        "resource",
        "action",
        "record",
        "access",
        "expected",
    ] {
        let mut missing = case.clone();
        missing.as_object_mut().expect("an object").remove(member);
        let error = serde_json::from_value::<Case>(missing).expect_err(member);
        assert!(
            error
                .to_string()
                .contains(&format!("missing field `{member}`")),
            "{member}: {error}"
        );
    }
    let mut with_record = case;
    with_record["record"] = serde_json::json!({"id": 1, "published": true});
    let read = serde_json::from_value::<Case>(with_record).expect("a record is read");
    assert_eq!(
        read.record
            .and_then(|record| record.get("published").cloned()),
        Some(serde_json::json!(true))
    );
}
