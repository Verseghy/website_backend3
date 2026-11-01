//! The runner of the permanent sanitiser fixtures in
//! `tests/fixtures/regressions/` (editor.md §13.6, P1).
//!
//! Each fixture is a pair of files: `<name>.html`, an input, and
//! `<name>.out.html`, what the sanitiser must write for it. Both are read as
//! bytes, so a trailing newline is part of the fixture. For every pair the
//! runner checks:
//!
//! - the output equals the expected file, byte for byte;
//! - the expected file is a fixed point: sanitising it again returns it with
//!   an empty report (P1);
//! - four independent instances write the same bytes and the same report
//!   (P5).
//!
//! A fixture is added for every shrunk P1 failure and for every input whose
//! shape is worth keeping (the structural cases of editor.md §7.5). When a
//! policy change alters an expected output, the PR that makes the change
//! updates the `.out.html` file and says why. The fixtures are invented
//! markup, never production content (CLAUDE.md, public-safety rules).

use std::{collections::BTreeMap, fs, path::PathBuf};

use super::sanitizer;
use crate::support::repository_root;

/// The directory of the fixtures.
fn fixtures_dir() -> PathBuf {
    repository_root().join("tests/fixtures/regressions")
}

/// Every fixture as `name -> (input, expected output)`. A missing half of a
/// pair is an error, so a fixture cannot silently stop being checked.
pub(super) fn fixtures() -> BTreeMap<String, (String, String)> {
    let dir = fixtures_dir();
    let mut inputs = BTreeMap::new();
    let mut outputs = BTreeMap::new();
    for entry in fs::read_dir(&dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a UTF-8 file name")
            .to_owned();
        let read = || {
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        };
        if let Some(name) = file.strip_suffix(".out.html") {
            outputs.insert(name.to_owned(), read());
        } else if let Some(name) = file.strip_suffix(".html") {
            inputs.insert(name.to_owned(), read());
        } else {
            assert_eq!(
                file, "README.md",
                "only fixture pairs and the README belong here"
            );
        }
    }
    for name in outputs.keys() {
        assert!(
            inputs.contains_key(name),
            "{name}.out.html has no {name}.html"
        );
    }
    inputs
        .into_iter()
        .map(|(name, input)| {
            let output = outputs
                .remove(&name)
                .unwrap_or_else(|| panic!("{name}.html has no {name}.out.html"));
            (name, (input, output))
        })
        .collect()
}

#[test]
fn every_fixture_comes_out_as_expected_and_stays_there() {
    let fixtures = fixtures();
    assert!(
        fixtures.len() >= 20,
        "the seed fixtures are missing: {}",
        fixtures.len()
    );
    for (name, (input, expected)) in &fixtures {
        let sanitizer = sanitizer();
        let result = sanitizer
            .sanitize(input)
            .unwrap_or_else(|error| panic!("{name}: refused: {error}"));
        assert_eq!(result.html.as_str(), expected, "{name}: the output changed");
        let again = sanitizer
            .sanitize(expected)
            .unwrap_or_else(|error| panic!("{name}: the expected output was refused: {error}"));
        assert_eq!(again.html.as_str(), expected, "{name}: not a fixed point");
        assert!(
            again.report.is_empty(),
            "{name}: the expected output reports {:?}",
            again.report
        );
        for _ in 0..3 {
            assert_eq!(
                super::sanitizer().sanitize(input).as_ref(),
                Ok(&result),
                "{name}: another instance wrote other bytes or another report"
            );
        }
    }
}

#[test]
fn the_fixture_names_describe_their_case() {
    for name in fixtures().keys() {
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{name}: use lower-case words joined by `-`"
        );
    }
}
