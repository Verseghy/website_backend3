//! The committed policy manifest, `admin/src/rich-text/policy.json` and
//! `css-vectors.jsonl`, against what the `rich_text_manifest` task renders
//! (editor.md §3.9, §13.6 "Manifest").
//!
//! The Contract CI job is the required check: it runs the task and fails
//! when a committed copy changed, and `scripts/ci/check-rich-text-manifest.sh`
//! fails when the tag list holds a forbidden tag. This test gives the same
//! answer earlier, in every `cargo nextest run`, with the command that
//! fixes it, and it pins the parts of the file the admin and the P0 oracle
//! rely on without trusting the renderer that wrote them.

use std::fs;

use backend3::domain::rich_text::{
    SanitizePolicy,
    manifest::{DEFAULT_OUTPUT_DIR, PolicyTables, manifest_files},
    policy::FORBIDDEN_TAGS,
};
use serde_json::Value;

use crate::support::repository_root;

/// How to bring the committed copies up to date.
const FIX: &str = "run `cargo loco task rich_text_manifest` from the repository root and \
                   commit admin/src/rich-text/ (editor.md §3.9)";

/// The committed copy of `name`.
fn committed(name: &str) -> String {
    let path = repository_root().join(DEFAULT_OUTPUT_DIR).join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}; {FIX}", path.display()))
}

#[test]
fn the_committed_copies_are_the_task_output() {
    let files = manifest_files(&PolicyTables::CONTENT_V1, &SanitizePolicy::content_v1())
        .expect("content-v1 renders");
    for file in files {
        assert!(
            committed(file.name) == file.contents,
            "admin/src/rich-text/{} differs from the policy in src/domain/rich_text/: {FIX}",
            file.name
        );
    }
}

/// The guard of editor.md §3.5, on the committed file itself, written
/// without the renderer's own check: the tag list and the element map hold
/// none of the forbidden tags, in any case.
#[test]
fn the_committed_tag_list_holds_no_forbidden_tag() {
    let policy: Value = serde_json::from_str(&committed("policy.json")).expect("JSON");
    let tags: Vec<&str> = policy["tags"]
        .as_array()
        .expect("a tag list")
        .iter()
        .map(|tag| tag.as_str().expect("a tag name"))
        .collect();
    let elements: Vec<&str> = policy["elements"]
        .as_object()
        .expect("an element map")
        .keys()
        .map(String::as_str)
        .collect();
    assert_ne!(tags.len(), 0, "the tag list is empty");
    for tag in tags.iter().chain(&elements) {
        let lower = tag.to_ascii_lowercase();
        assert!(
            !FORBIDDEN_TAGS.contains(&lower.as_str()),
            "{tag} is forbidden (editor.md §3.5)"
        );
    }
    let mut sorted = tags.clone();
    sorted.sort_unstable();
    let mut element_names = elements.clone();
    element_names.sort_unstable();
    assert_eq!(
        sorted, element_names,
        "every tag has its rules, and only tags do"
    );
    assert_eq!(policy["policy_version"], "content-v2");
}

/// One JSON object per line, with the three members in order.
#[test]
fn the_committed_vectors_are_one_object_per_line() {
    let vectors = committed("css-vectors.jsonl");
    assert!(vectors.ends_with('\n'));
    for (number, line) in vectors.lines().enumerate() {
        let vector: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("line {}: {error}", number + 1));
        let object = vector.as_object().expect("an object");
        assert_eq!(object.len(), 3, "line {}", number + 1);
        assert!(vector["element"].is_string() && vector["input"].is_string());
        assert!(vector["output"].is_string() || vector["output"].is_null());
        assert!(line.starts_with("{\"element\":"), "line {}", number + 1);
    }
}
