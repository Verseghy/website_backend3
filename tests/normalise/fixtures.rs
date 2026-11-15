//! The runner of the normaliser's before-and-after fixtures in
//! `tests/fixtures/normalise/` (data-migration.md §12: "each of N0-N9 and
//! N5b with a before and after fixture").
//!
//! Each fixture is a pair of files: `<name>.html`, a legacy body, and
//! `<name>.out.html`, what the normaliser must write for it. Both are read
//! as bytes, so a trailing newline is part of the fixture. The name starts
//! with the step it shows (`n0-`, `n3-`, `n6-`, `n8-`, `n9-`) or with `alt-`
//! for the images without alt text. For every pair the runner checks:
//!
//! - the output equals the expected file, byte for byte;
//! - the expected file is the normaliser's fixed point: normalising it
//!   again returns it with no rule hit and the same observations (P4);
//! - the step the name gives fired: an `n3-` fixture records an N3 hit, an
//!   `n6-` fixture an N6 hit or a dead-host occurrence, an `alt-` fixture
//!   an image without alt text, and an `n0-`, `n8-` or `n9-` fixture no hit
//!   at all, because parsing and writing are not rule hits;
//! - N9 holds: the expected file is also the sanitiser's fixed point, except
//!   for the fixtures in [`NOT_YET_CONTENT_V1`], whose output still holds
//!   markup that a later step changes; for those the sanitiser check must
//!   fail, so that the list stays true;
//! - the expected file is in the editor's stored form as far as the runner
//!   can see, no text node with a line feed, tab, form feed or carriage
//!   return (editor.md §3.8 leaves none), unless the fixture is listed in
//!   [`NOT_YET_CONTENT_V1`] or [`NOT_YET_EDITOR_FORM`];
//! - the inputs of [`CRLF_INPUTS`] still hold `\r\n`, so that N0's line-end
//!   case stays tested whatever a checkout does to line ends (the
//!   repository's `.gitattributes` also turns conversion off for them).
//!
//! The admin's editor round trip, the third check of N9, will read the same
//! `.out.html` files (ADR 0013 §10); it must expect the outputs of both
//! lists to change. The fixtures are invented markup, never production
//! content (CLAUDE.md, public-safety rules).

use std::{collections::BTreeMap, fs, path::PathBuf};

use backend3::domain::normalise::{FixedPointError, Rule, Step, check_fixed_point, normalise};
use backend3::domain::rich_text::{SanitizePolicy, Sanitizer};

use crate::support::repository_root;

/// Fixtures whose output is not content-v1 yet, with the step that will
/// make it so. N9's sanitiser check must fail for each of them.
const NOT_YET_CONTENT_V1: &[(&str, &str)] = &[
    (
        "n3-center-around-embed",
        "N2 turns the bare YouTube iframe and its legacy attributes into the editor's YouTube node",
    ),
    (
        "n3-center-unclear-alignment",
        "N7 drops the text-align declaration that content-v1 does not keep",
    ),
];

/// Fixtures whose output is content-v1 (it passes N9's sanitiser check) but
/// not yet the form the editor stores, with the step that will make it so.
/// The editor round trip would still change them.
const NOT_YET_EDITOR_FORM: &[(&str, &str)] = &[(
    "n0-upper-case-markup",
    "N5b's whitespace rule (editor.md §3.8) strips the line feed after the `<br>`",
)];

/// Inputs that hold `\r\n` on purpose, for N0.
const CRLF_INPUTS: &[&str] = &["n0-upper-case-markup", "n3-center-around-embed"];

/// The name prefixes a fixture may start with.
const PREFIXES: &[&str] = &["n0-", "n3-", "n6-", "n8-", "n9-", "alt-"];

/// The directory of the fixtures.
fn fixtures_dir() -> PathBuf {
    repository_root().join("tests/fixtures/normalise")
}

/// Every fixture as `name -> (input, expected output)`. A missing half of a
/// pair is an error, so a fixture cannot silently stop being checked.
fn fixtures() -> BTreeMap<String, (String, String)> {
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

fn sanitizer() -> Sanitizer {
    Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
}

/// Whether `list` names the fixture `name`.
fn listed(list: &[(&str, &str)], name: &str) -> bool {
    list.iter().any(|(listed, _)| *listed == name)
}

/// The text nodes of `html` that hold a line feed, tab, form feed or
/// carriage return, which the editor's whitespace rule never leaves in a
/// stored body (editor.md §3.8).
fn text_with_line_breaks(html: &str) -> Vec<String> {
    scraper::Html::parse_fragment(html)
        .root_element()
        .descendants()
        .filter_map(|node| node.value().as_text().map(|text| text.to_string()))
        .filter(|text| text.contains(['\n', '\t', '\u{c}', '\r']))
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
    let sanitizer = sanitizer();
    for (name, (input, expected)) in &fixtures {
        let once = normalise(input).unwrap_or_else(|error| panic!("{name}: refused: {error}"));
        assert_eq!(&once.html, expected, "{name}: the output changed");
        let again = normalise(expected)
            .unwrap_or_else(|error| panic!("{name}: the expected output was refused: {error}"));
        assert_eq!(&again.html, expected, "{name}: not a fixed point");
        assert!(
            !again.report.has_hits(),
            "{name}: the expected output still hits {:?}",
            again.report
        );
        assert_eq!(
            again.report.dead_host_kept(),
            once.report.dead_host_kept(),
            "{name}"
        );
        assert_eq!(
            again.report.images_without_alt(),
            once.report.images_without_alt(),
            "{name}"
        );

        let hits = |step| {
            once.report
                .hits()
                .filter(|(rule, _)| rule.step() == step)
                .count()
        };
        match name.split('-').next() {
            Some("n0" | "n8" | "n9") => {
                assert!(!once.report.has_hits(), "{name}: {:?}", once.report);
            }
            Some("n3") => assert!(hits(Step::N3) > 0, "{name}: no N3 hit"),
            Some("n6") => assert!(
                hits(Step::N6) > 0 || !once.report.dead_host_kept().is_empty(),
                "{name}: N6 neither rewrote nor reported anything"
            ),
            Some("alt") => assert!(
                !once.report.images_without_alt().is_empty(),
                "{name}: no image without alt text"
            ),
            _ => panic!("{name}: the name starts with no step"),
        }

        let checked = check_fixed_point(expected, &sanitizer);
        match NOT_YET_CONTENT_V1.iter().find(|(listed, _)| listed == name) {
            None => assert_eq!(checked, Ok(()), "{name}: N9 fails"),
            Some((_, why)) => assert!(
                matches!(checked, Err(FixedPointError::Resanitised { .. })),
                "{name} is listed as not content-v1 yet ({why}), but N9 gives {checked:?}"
            ),
        }

        if !listed(NOT_YET_CONTENT_V1, name) && !listed(NOT_YET_EDITOR_FORM, name) {
            assert_eq!(
                text_with_line_breaks(expected),
                Vec::<String>::new(),
                "{name}: the editor would strip these; list the fixture in NOT_YET_EDITOR_FORM \
                 with the step that will"
            );
        }
    }
}

#[test]
fn the_production_shapes_report_what_the_sign_off_sheet_lists() {
    use backend3::domain::normalise::AltState::{Blank, Missing};

    let fixtures = fixtures();
    let report = |name: &str| {
        normalise(&fixtures[name].0)
            .expect("a fixture settles")
            .report
    };
    let links = report("n6-dead-host-links");
    assert_eq!(links.count(Rule::DeadHostLinkRewritten), 2);
    assert_eq!(links.dead_host_kept(), []);
    let kept = report("n6-dead-host-kept");
    assert_eq!(kept.count(Rule::DeadHostLinkRewritten), 0);
    assert_eq!(kept.dead_host_kept().len(), 2, "the href and the text");
    let alts: Vec<_> = report("alt-missing-and-blank")
        .images_without_alt()
        .iter()
        .map(|image| (image.position, image.alt))
        .collect();
    assert_eq!(alts, [(1, Missing), (2, Blank), (3, Blank)]);
    let embed = report("n3-center-around-embed");
    assert_eq!(
        embed.hits().collect::<Vec<_>>(),
        [(Rule::CenterUnwrapped, 1)]
    );
    let n3 = |name: &str| {
        let report = report(name);
        [
            Rule::CenterBlockAligned,
            Rule::CenterRunWrapped,
            Rule::CenterAlignmentDropped,
            Rule::CenterAlignmentUnclear,
        ]
        .map(|rule| report.count(rule))
    };
    assert_eq!(n3("n3-center-alignment-dropped"), [0, 0, 1, 0]);
    assert_eq!(n3("n3-center-unclear-alignment"), [1, 0, 0, 1]);
    assert_eq!(n3("n3-center-around-list-and-quote"), [0, 3, 0, 0]);
    assert_eq!(n3("n3-center-around-table"), [3, 0, 0, 0]);
    assert_eq!(n3("n3-center-in-cell"), [1, 0, 0, 0]);
}

#[test]
fn the_fixture_names_and_lists_describe_their_cases() {
    let fixtures = fixtures();
    for name in fixtures.keys() {
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{name}: use lower-case words joined by `-`"
        );
        assert!(
            PREFIXES.iter().any(|prefix| name.starts_with(prefix)),
            "{name}: start with the step it shows, one of {PREFIXES:?}"
        );
    }
    for (name, _) in NOT_YET_CONTENT_V1.iter().chain(NOT_YET_EDITOR_FORM) {
        assert!(
            fixtures.contains_key(*name),
            "{name} is listed but has no fixture"
        );
    }
    for (name, _) in NOT_YET_EDITOR_FORM {
        assert!(
            !listed(NOT_YET_CONTENT_V1, name),
            "{name}: a fixture that is not content-v1 yet belongs in that list only"
        );
    }
    for name in CRLF_INPUTS {
        assert!(
            fixtures[*name].0.contains("\r\n"),
            "{name}.html lost its \\r\\n line ends; restore them (see .gitattributes)"
        );
    }
}
