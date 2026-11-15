//! P2 and P3 of editor.md §13.6: the cross-language golden files of
//! `tests/fixtures/e1/`.
//!
//! Each `<name>.html` there is `J(x)` for one editor fixture `x`: what the
//! admin's editor stores after loading `x` and saving it. The admin's tests
//! write the files (`admin/src/rich-text/e1-golden.test.ts`, which
//! `admin/src/test/golden-e1.ts` documents), and the Contract CI job fails
//! when they differ from the committed ones, so these tests always read what
//! the editor of the same commit writes. The server must store every one of
//! them unchanged, because the editor and the server are two writers of one
//! dialect (ADR 0013 §2, "Fixed point"; editor.md §6, "Fixed-point
//! requirements"):
//!
//! - **P2**: `S(e1) == e1`, byte for byte, with an empty removal report. A
//!   difference here means a save from the editor would be rewritten on the
//!   server, which the drift canary of §7.7 reports as `normalized: true` on
//!   every such save.
//! - **P3**: `inventory(S(e1)) == inventory(e1)`: the server removes no
//!   element, attribute or style property that the editor writes. P2 implies
//!   it; it is asserted on its own because its failure names what the server
//!   drops (`td[style] padding`, `p[class]`), which a byte difference does
//!   not.
//! - The P0 oracle (`oracle.rs`) also accepts every file, so the editor
//!   writes nothing outside `policy.json` even where the sanitiser and the
//!   editor would agree on it.
//!
//! The inventory is computed here, from the oracle's fragment parse and
//! with no code of the sanitiser, so a rule that both sides got wrong the
//! same way still cannot hide a removal: it counts elements by tag,
//! attributes by tag and name, and style properties by tag and property, as
//! each `style` attribute is written.

use std::{collections::BTreeMap, fmt, fs, path::PathBuf};

use scraper::{ElementRef, Node};

use super::{oracle, sanitizer};
use crate::support::repository_root;

/// The directory of the golden files.
fn golden_dir() -> PathBuf {
    repository_root().join("tests/fixtures/e1")
}

/// Every golden file as `name -> contents`, read as bytes in UTF-8, in name
/// order. The directory holds only `.html` files and its `README.md`.
fn golden_files() -> BTreeMap<String, String> {
    let dir = golden_dir();
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(&dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a UTF-8 file name")
            .to_owned();
        if let Some(name) = file.strip_suffix(".html") {
            let contents = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            files.insert(name.to_owned(), contents);
        } else {
            assert_eq!(
                file,
                "README.md",
                "only golden files and the README belong in {}; the admin's tests write the files (npm test in admin/)",
                dir.display()
            );
        }
    }
    files
}

/// One fact of an inventory: an element, an attribute or a style property,
/// each with the tag it is on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Fact {
    /// An element, by tag name.
    Element(String),
    /// An attribute, by tag and attribute name.
    Attribute(String, String),
    /// A style property, by tag and property name, as written.
    Property(String, String),
}

impl fmt::Display for Fact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Element(tag) => write!(f, "<{tag}>"),
            Self::Attribute(tag, name) => write!(f, "{tag}[{name}]"),
            Self::Property(tag, property) => write!(f, "{tag}[style] {property}"),
        }
    }
}

/// How many times each fact occurs in a body.
type Inventory = BTreeMap<Fact, usize>;

/// The inventory of `html`, parsed as a fragment in a `div` context, as
/// ammonia and the public sites parse it.
///
/// - `html`: the body.
fn inventory(html: &str) -> Inventory {
    let tree = oracle::parse(html);
    let mut facts = Inventory::new();
    for node in tree.root_element().descendants().skip(1) {
        let Node::Element(_) = node.value() else {
            continue;
        };
        let element = ElementRef::wrap(node).expect("an element node");
        let tag = element.value().name().to_owned();
        *facts.entry(Fact::Element(tag.clone())).or_default() += 1;
        for (name, value) in element.value().attrs() {
            *facts
                .entry(Fact::Attribute(tag.clone(), name.to_owned()))
                .or_default() += 1;
            if name == "style" {
                // The names before each `:`, as written: a stored style is
                // in the canonical alphabet, with no quotes, comments or
                // parentheses that could hold a `;` (editor.md §3.1 rule 6).
                for declaration in value.split(';') {
                    if let Some((property, _)) = declaration.split_once(':') {
                        *facts
                            .entry(Fact::Property(tag.clone(), property.trim().to_owned()))
                            .or_default() += 1;
                    }
                }
            }
        }
    }
    facts
}

/// What differs between the editor's inventory and the server's, one line
/// per fact, naming the fact and both counts; empty when they are equal.
///
/// - `editor`: the inventory of the golden file.
/// - `server`: the inventory of the sanitiser's output for it.
fn inventory_differences(editor: &Inventory, server: &Inventory) -> Vec<String> {
    let mut facts: Vec<&Fact> = editor.keys().chain(server.keys()).collect();
    facts.sort();
    facts.dedup();
    facts
        .into_iter()
        .filter_map(|fact| {
            let written = editor.get(fact).copied().unwrap_or(0);
            let kept = server.get(fact).copied().unwrap_or(0);
            (written != kept).then(|| {
                let verb = if kept < written { "drops" } else { "adds" };
                format!(
                    "the server {verb} {fact}: the editor writes {written}, the server keeps {kept}"
                )
            })
        })
        .collect()
}

/// Every way the server changes one golden file, empty when it keeps it.
///
/// - `html`: the golden file's contents.
fn server_changes(html: &str) -> Vec<String> {
    let sanitized = match sanitizer().sanitize(html) {
        Ok(sanitized) => sanitized,
        Err(error) => return vec![format!("the server refuses it: {error}")],
    };
    let output = sanitized.html.as_str();
    let mut changes = inventory_differences(&inventory(html), &inventory(output));
    if output != html {
        changes.push(format!("the server rewrites it as: {output}"));
    }
    if !sanitized.report.is_empty() {
        changes.push(format!("the removal report is {:?}", sanitized.report));
    }
    changes
}

#[test]
fn the_admin_wrote_the_golden_files() {
    let files = golden_files();
    // The admin writes more than a hundred today (every regression fixture,
    // the canonical bodies, the table shapes and inputs, the command
    // fixtures); far fewer means the directory was emptied or never written.
    assert!(
        files.len() >= 100,
        "{} golden files in tests/fixtures/e1; run `npm test` in admin/ and commit what it writes",
        files.len()
    );
    for group in ["canonical-", "command-", "regression-", "table-"] {
        assert!(
            files.keys().any(|name| name.starts_with(group)),
            "no golden file of the group {group}"
        );
    }
}

#[test]
fn the_server_stores_every_editor_output_unchanged() {
    let failures: Vec<String> = golden_files()
        .iter()
        .filter_map(|(name, html)| {
            let changes = server_changes(html);
            (!changes.is_empty()).then(|| format!("{name}.html:\n  {}", changes.join("\n  ")))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "P2 and P3 (editor.md §13.6): the server changes what the editor writes. Either the editor writes \
         something content-v1 does not store (fix the editor in admin/src/rich-text/ and run `npm test` \
         in admin/), or the policy lacks something the editor needs (a policy change, editor.md §3):\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_editor_output_passes_the_oracle() {
    let failures: Vec<String> = golden_files()
        .iter()
        .filter_map(|(name, html)| {
            oracle::assert_content_v1(html)
                .err()
                .map(|found| format!("{name}.html: {found}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "golden files outside content-v1 (P0 on the editor's output):\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_inventory_counts_elements_attributes_and_properties() {
    let facts = inventory(
        r#"<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td colspan="2" style="text-align: center; width: 8px">a</td><td>b</td></tr></tbody></table>"#,
    );
    let count = |fact: Fact| facts.get(&fact).copied().unwrap_or(0);
    let element = |tag: &str| Fact::Element(tag.to_owned());
    let attribute = |tag: &str, name: &str| Fact::Attribute(tag.to_owned(), name.to_owned());
    let property = |tag: &str, name: &str| Fact::Property(tag.to_owned(), name.to_owned());
    assert_eq!(count(element("td")), 2);
    assert_eq!(count(element("table")), 1);
    assert_eq!(count(element("div")), 0, "the parse context is not counted");
    assert_eq!(count(attribute("td", "colspan")), 1);
    assert_eq!(count(attribute("td", "style")), 1);
    assert_eq!(count(property("td", "text-align")), 1);
    assert_eq!(count(property("td", "width")), 1);
    assert_eq!(count(property("table", "max-width")), 1);
    assert_eq!(count(property("tbody", "display")), 1);
    assert_eq!(facts.values().sum::<usize>(), 14);
}

#[test]
fn a_change_names_what_the_server_drops() {
    // Markup the editor never writes, standing in for an editor bug: a cell
    // padding, a class, a colgroup and a comment. Each is named.
    let changes = server_changes(
        r#"<table style="border-collapse: collapse; max-width: fit-content"><colgroup><col></colgroup><tbody style="display: table-row-group"><tr><td class="x" style="padding: 5px; text-align: center">a<!-- b --></td></tr></tbody></table>"#,
    );
    for expected in [
        "the server drops td[style] padding: the editor writes 1, the server keeps 0",
        "the server drops td[class]: the editor writes 1, the server keeps 0",
        "the server drops <colgroup>: the editor writes 1, the server keeps 0",
        "the server drops <col>: the editor writes 1, the server keeps 0",
    ] {
        assert!(
            changes.iter().any(|change| change == expected),
            "{expected:?} is missing from {changes:#?}"
        );
    }
    assert!(
        changes
            .iter()
            .any(|change| change.starts_with("the server rewrites it as: ")),
        "{changes:#?}"
    );
    assert!(
        changes
            .iter()
            .any(|change| change.starts_with("the removal report is ")),
        "{changes:#?}"
    );
}

#[test]
fn a_body_the_server_keeps_has_no_change() {
    assert_eq!(
        server_changes(r#"<p style="text-align: center">a <strong>b</strong></p>"#),
        Vec::<String>::new()
    );
}

#[test]
fn an_addition_is_named_too() {
    // The server adds the locked declarations of an image that has none
    // (editor.md §7.5), so an inventory can grow; the line says so.
    let changes = server_changes(r#"<p><img src="/storage/uploads/a.png" alt="a"></p>"#);
    assert!(
        changes.contains(
            &"the server adds img[style]: the editor writes 0, the server keeps 1".to_owned()
        ),
        "{changes:#?}"
    );
}

#[test]
fn a_refused_body_is_reported() {
    let deep = "<blockquote>".repeat(300);
    let changes = server_changes(&deep);
    assert_eq!(changes.len(), 1, "{changes:#?}");
    assert!(
        changes[0].starts_with("the server refuses it: "),
        "{changes:#?}"
    );
}
