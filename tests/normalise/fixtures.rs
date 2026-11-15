//! The runner of the normaliser's before-and-after fixtures in
//! `tests/fixtures/normalise/` (data-migration.md §12: "each of N0-N9 and
//! N5b with a before and after fixture").
//!
//! Each fixture is a pair of files: `<name>.html`, a legacy body, and
//! `<name>.out.html`, what the normaliser must write for it. Both are read
//! as bytes, so a trailing newline is part of the fixture. The name starts
//! with the step it shows (`n0-`, `n3-`, `n5-`, `n5b-`, `n6-`, `n7-`,
//! `n8-`, `n9-`) or with `alt-` for the images without alt text. For every
//! pair the runner checks:
//!
//! - the output equals the expected file, byte for byte;
//! - the expected file is the normaliser's fixed point: normalising it
//!   again returns it with no rule hit, no unplanned removal and the same
//!   observations (P4);
//! - the step the name gives fired: an `n3-` fixture records an N3 hit, an
//!   `n5-` fixture an N5 hit, an `n5b-` fixture an N5b hit, an `n6-`
//!   fixture an N6 hit or a dead-host occurrence, an `n7-` fixture an N7
//!   hit or an unplanned removal, an `alt-` fixture an image without alt
//!   text, and an `n0-`, `n8-` or `n9-` fixture no hit at all, because
//!   parsing and writing are not rule hits;
//! - the text a reader reads is unchanged, `text(source) == text(target)`
//!   (data-migration.md §10, V5(c)): the input and the expected file have
//!   the same search text (ADR 0020 §2), which keeps every text node in
//!   document order, separates blocks and collapses white space, so the
//!   whitespace rule of N5b keeps it too. N1, which inserts the text of the
//!   links it makes, adds the first exception;
//! - N7's unplanned removals are exactly those of the fixtures listed in
//!   [`UNPLANNED`]: a fixture whose removals a later step will plan, or
//!   one whose removals no step plans, so that the import stops on it until
//!   an editor accepts it (`N7-accept`);
//! - N9 holds: the expected file is the sanitiser's fixed point too, which
//!   [`check_fixed_point`] checks with the normaliser's;
//! - the expected file is in the editor's stored form as far as the runner
//!   can see, no text node with a line feed, tab, form feed or carriage
//!   return (editor.md §3.8 leaves none), unless the fixture is listed in
//!   [`NOT_YET_EDITOR_FORM`];
//! - the inputs of [`CRLF_INPUTS`] still hold `\r\n`, so that N0's line-end
//!   case stays tested whatever a checkout does to line ends (the
//!   repository's `.gitattributes` also turns conversion off for them).
//!
//! The admin's editor round trip, the third check of N9, reads the same
//! `.out.html` files (ADR 0013 §10, editor.md §13.4): so far the `n5b-` and
//! `alt-` ones, in `admin/src/rich-text/normaliser-round-trip.test.ts`; the
//! rest join it with N5b's structure rules. It must expect the outputs of
//! [`NOT_YET_EDITOR_FORM`] to change. The fixtures are invented markup,
//! never production content (CLAUDE.md, public-safety rules).

use std::{collections::BTreeMap, fs, path::PathBuf};

use backend3::domain::normalise::{Rule, Step, check_fixed_point, normalise};
use backend3::domain::rich_text::{SanitizePolicy, Sanitizer, html_to_search_text};

use crate::support::repository_root;

/// Fixtures whose N7 makes removals that no step plans, with why: the
/// later step that will plan them, or why no step does. Every other
/// fixture must have none.
const UNPLANNED: &[(&str, &str)] = &[
    (
        "n3-center-around-embed",
        "N2 replaces the bare YouTube iframe's legacy attributes (frameborder) with the editor's \
         YouTube node",
    ),
    (
        "n5-mark-attributes-for-review",
        "no step plans them: the attributes the renamed marks keep (D146), among them a style \
         that reset the bold, which an editor reviews before the import may drop them \
         (N7-accept)",
    ),
    (
        "n5-table-align",
        "N5b gives the tables the content-v1 table shape, without the table width",
    ),
    (
        "n5b-image-sizing-dropped",
        "no step plans them: the border and the percentage width that N5b leaves after the \
         canonical declarations, which an editor reviews before the import may drop them \
         (D168, N7-accept)",
    ),
    (
        "n5b-image-width-over-bound",
        "no step plans it: the width over the 2000 px bound that N5b keeps, which stops the \
         import until a policy change raises the bound or an N7-accept entry lets it go \
         (editor.md §16 item 5, reading (e))",
    ),
    (
        "n7-unplanned-removals",
        "no step plans them: Word's wrapper, classes, margin and font, which an editor reviews \
         before the import may drop them (N7-accept)",
    ),
];

/// Fixtures whose output is content-v1 (it passes N9's sanitiser check) but
/// not yet the form the editor stores, with the step that will make it so.
/// The editor round trip would still change them.
const NOT_YET_EDITOR_FORM: &[(&str, &str)] = &[
    (
        "n0-upper-case-markup",
        "N5b's whitespace rule (editor.md §3.8) strips the line feed after the `<br>`",
    ),
    (
        "n3-center-around-embed",
        "N2 turns the bare YouTube iframe into the editor's YouTube node, and N5b's whitespace \
         rule removes the line feeds between the blocks",
    ),
];

/// Inputs that hold `\r\n` on purpose, for N0.
const CRLF_INPUTS: &[&str] = &["n0-upper-case-markup", "n3-center-around-embed"];

/// The name prefixes a fixture may start with.
const PREFIXES: &[&str] = &[
    "n0-", "n3-", "n5-", "n5b-", "n6-", "n7-", "n8-", "n9-", "alt-",
];

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
        let once =
            normalise(input, &sanitizer).unwrap_or_else(|error| panic!("{name}: refused: {error}"));
        assert_eq!(&once.html, expected, "{name}: the output changed");
        let again = normalise(expected, &sanitizer)
            .unwrap_or_else(|error| panic!("{name}: the expected output was refused: {error}"));
        assert_eq!(&again.html, expected, "{name}: not a fixed point");
        assert!(
            !again.report.has_hits() && !again.report.has_unplanned_removals(),
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
        assert_eq!(
            again.report.wide_images(),
            [],
            "{name}: N7 dropped every width over the bound"
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
            Some("n5") => assert!(hits(Step::N5) > 0, "{name}: no N5 hit"),
            Some("n5b") => assert!(hits(Step::N5b) > 0, "{name}: no N5b hit"),
            Some("n6") => assert!(
                hits(Step::N6) > 0 || !once.report.dead_host_kept().is_empty(),
                "{name}: N6 neither rewrote nor reported anything"
            ),
            Some("n7") => assert!(
                hits(Step::N7) > 0 || once.report.has_unplanned_removals(),
                "{name}: N7 neither changed nor reported anything"
            ),
            Some("alt") => assert!(
                !once.report.images_without_alt().is_empty(),
                "{name}: no image without alt text"
            ),
            _ => panic!("{name}: the name starts with no step"),
        }

        assert_eq!(
            html_to_search_text(expected),
            html_to_search_text(input),
            "{name}: the text a reader reads changed (V5(c))"
        );

        match UNPLANNED.iter().find(|(listed, _)| listed == name) {
            None => assert!(
                !once.report.has_unplanned_removals(),
                "{name}: N7 removes what no step planned: {:?}; plan it in its step, or list \
                 the fixture in UNPLANNED with why",
                once.report.unplanned_removals().collect::<Vec<_>>()
            ),
            Some((_, why)) => assert!(
                once.report.has_unplanned_removals(),
                "{name} is listed as unplanned ({why}), but N7 removed nothing unplanned"
            ),
        }

        assert_eq!(
            check_fixed_point(expected, &sanitizer),
            Ok(()),
            "{name}: N9 fails"
        );

        if !listed(NOT_YET_EDITOR_FORM, name) {
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
fn each_inline_mark_fixture_counts_its_elements_under_its_rule() {
    // One hit per legacy element, under its own rule, and nothing else:
    // the run report's N5 counts are the element counts of data-migration.md
    // §7.1 ("1 `span.marker`, 1 `big`, 1 `ins`, 23 `span[dir]`"). Every hit
    // of the body is listed, N7's planned changes too (the table's `border`
    // and `cellpadding`, and the locked styles of the table and its
    // `tbody`), so that nothing is counted unseen.
    let fixtures = fixtures();
    let sanitizer = sanitizer();
    let expected: &[(&str, &[(Rule, usize)])] = &[
        ("n5-b-to-strong", &[(Rule::BToStrong, 2)]),
        ("n5-i-to-em", &[(Rule::IToEm, 1)]),
        ("n5-ins-to-u", &[(Rule::InsToU, 1)]),
        ("n5-strike-to-s", &[(Rule::StrikeToS, 1)]),
        ("n5-del-to-s", &[(Rule::DelToS, 1)]),
        ("n5-big-unwrapped", &[(Rule::BigUnwrapped, 1)]),
        ("n5-span-marker", &[(Rule::MarkerHighlighted, 1)]),
        ("n5-span-dir-unwrapped", &[(Rule::SpanDirUnwrapped, 2)]),
        (
            "n5-span-dir-style-dropped",
            &[(Rule::SpanDirStyleDropped, 1)],
        ),
        (
            "n5-span-marker-with-dir",
            &[(Rule::MarkerHighlighted, 1), (Rule::SpanDirRemoved, 1)],
        ),
        (
            "n5-marks-in-a-table",
            &[
                (Rule::BigUnwrapped, 1),
                (Rule::MarkerHighlighted, 1),
                (Rule::LockedMarkupAdded, 2),
                (Rule::TableBorderDropped, 1),
                (Rule::TableCellpaddingDropped, 1),
            ],
        ),
        (
            "n5-mark-attributes-for-review",
            &[(Rule::BToStrong, 1), (Rule::IToEm, 1), (Rule::StrikeToS, 1)],
        ),
    ];
    for (name, hits) in expected {
        let (input, _) = &fixtures[*name];
        let report = normalise(input, &sanitizer)
            .expect("a fixture settles")
            .report;
        assert_eq!(report.hits().collect::<Vec<_>>(), *hits, "{name}");
    }
    // The renamed marks keep their attributes for N7 (D146), which removes
    // them as nothing planned: each is listed for the editors' review.
    let unplanned: Vec<_> = normalise(&fixtures["n5-mark-attributes-for-review"].0, &sanitizer)
        .expect("a fixture settles")
        .report
        .unplanned_removals()
        .map(|(removal, count)| (removal.to_string(), count))
        .collect();
    assert_eq!(
        unplanned,
        [
            ("attribute_not_allowed em[class]".to_owned(), 1),
            ("attribute_not_allowed s[title]".to_owned(), 1),
            ("attribute_not_allowed strong[id]".to_owned(), 1),
            ("attribute_not_allowed strong[style]".to_owned(), 1),
        ]
    );
    // Every other `n5-` fixture shows N5's block rules, and no inline one.
    let inline = [
        Rule::BToStrong,
        Rule::IToEm,
        Rule::InsToU,
        Rule::StrikeToS,
        Rule::DelToS,
        Rule::BigUnwrapped,
        Rule::MarkerHighlighted,
        Rule::SpanDirUnwrapped,
        Rule::SpanDirStyleDropped,
        Rule::SpanDirRemoved,
    ];
    let listed: Vec<_> = expected.iter().map(|(name, _)| *name).collect();
    for (name, (input, _)) in &fixtures {
        if !name.starts_with("n5-") || listed.contains(&name.as_str()) {
            continue;
        }
        let report = normalise(input, &sanitizer)
            .expect("a fixture settles")
            .report;
        assert!(
            report.hits().all(|(rule, _)| !inline.contains(&rule)),
            "{name}: an inline-mark fixture; list its hits in this test"
        );
    }
    // And every inline rule has a fixture that shows it.
    for rule in inline {
        assert!(
            expected
                .iter()
                .any(|(_, hits)| hits.iter().any(|(hit, _)| *hit == rule)),
            "{rule:?} has no fixture"
        );
    }
}

#[test]
fn the_production_shapes_report_what_the_sign_off_sheet_lists() {
    use backend3::domain::normalise::AltState::{Blank, Missing};

    let fixtures = fixtures();
    let sanitizer = sanitizer();
    let report = |name: &str| {
        normalise(&fixtures[name].0, &sanitizer)
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
    let step = |name: &str, wanted| {
        report(name)
            .hits()
            .filter(|(rule, _)| rule.step() == wanted)
            .collect::<Vec<_>>()
    };
    let n5 = |name: &str| step(name, Step::N5);
    assert_eq!(
        n5("n5-paragraph-align"),
        [
            (Rule::ParagraphAlignMapped, 3),
            (Rule::ParagraphAlignDropped, 2)
        ],
        "the attribute that a style overrides and the one that maps to nothing are dropped"
    );
    assert_eq!(
        n5("n5-paragraph-align-in-center"),
        [(Rule::ParagraphAlignMapped, 1)]
    );
    assert_eq!(n5("n5-table-align"), [(Rule::TableAlignDropped, 2)]);
    assert_eq!(n5("n5-minor-headings"), [(Rule::HeadingToH4, 2)]);
    assert_eq!(n5("n5-italic-heading"), [(Rule::ItalicHeadingToEm, 2)]);
    assert_eq!(n5("n5-paragraph-lang"), [(Rule::ParagraphLangStripped, 2)]);
    // The image over the 2000 px bound is listed for the policy change of
    // editor.md §16 item 5, and the image without alt for the editors.
    let wide: Vec<_> = report("n5b-image-width-over-bound")
        .wide_images()
        .iter()
        .map(|image| (image.position, image.width.to_string()))
        .collect();
    assert_eq!(wide, [(1, "2400".to_owned())]);
    let alts: Vec<_> = report("n5b-image-without-alt")
        .images_without_alt()
        .iter()
        .map(|image| (image.position, image.alt))
        .collect();
    assert_eq!(alts, [(1, Missing)]);
    for name in fixtures
        .keys()
        .filter(|name| *name != "n5b-image-width-over-bound")
    {
        assert_eq!(report(name.as_str()).wide_images(), [], "{name}");
    }
}

#[test]
fn each_image_fixture_counts_its_rules() {
    // The exact hits of each `n5b-` pair: one per image and rule, so the
    // run report's N5b counts are image counts (data-migration.md §7.1,
    // "7 floated images", "about 11 images inside headings, inline
    // formatting or links").
    let fixtures = fixtures();
    let sanitizer = sanitizer();
    let sized = [(Rule::ImageHeightAuto, 1), (Rule::ImageShaped, 1)];
    let moved = |count| {
        [
            (Rule::ImageMovedBeforeBlock, count),
            (Rule::ImageHeightAuto, count),
            (Rule::ImageShaped, count),
        ]
    };
    let expected: &[(&str, &[(Rule, usize)])] = &[
        ("n5b-image-legacy-size", &sized),
        ("n5b-image-wide", &sized),
        ("n5b-image-without-alt", &sized),
        ("n5b-image-width-over-bound", &sized),
        (
            "n5b-image-float-style",
            &[
                (Rule::ImageParagraphSplit, 1),
                (Rule::ImageFloatRemoved, 1),
                (Rule::ImageHeightAuto, 1),
                (Rule::ImageShaped, 1),
            ],
        ),
        (
            "n5b-image-float-class",
            &[
                (Rule::ImageFloatRemoved, 2),
                (Rule::ImageHeightAuto, 1),
                (Rule::ImageShaped, 2),
            ],
        ),
        (
            "n5b-image-in-text",
            &[
                (Rule::ImageParagraphSplit, 1),
                (Rule::ImageHeightAuto, 1),
                (Rule::ImageShaped, 1),
            ],
        ),
        ("n5b-image-in-heading", &moved(1)),
        ("n5b-image-in-inline-formatting", &moved(2)),
        ("n5b-image-in-list-item", &moved(2)),
        ("n5b-image-in-cell", &moved(2)),
        ("n5b-image-in-quote", &moved(1)),
        (
            "n5b-image-in-link",
            &[
                (Rule::ImageMovedBeforeBlock, 2),
                (Rule::ImageLinkDropped, 2),
                (Rule::ImageHeightAuto, 2),
                (Rule::ImageShaped, 2),
            ],
        ),
        (
            "n5b-image-centred",
            &[
                (Rule::CenterUnwrapped, 1),
                (Rule::CenterRunWrapped, 1),
                (Rule::ImageWrapped, 1),
                (Rule::ImageHeightAuto, 1),
                (Rule::ImageShaped, 1),
            ],
        ),
        (
            "n5b-image-bare",
            &[(Rule::ImageWrapped, 1), (Rule::ImageShaped, 1)],
        ),
        // N7 reviews the first image's border (D168); N5b counts the second's.
        (
            "n5b-image-sizing-dropped",
            &[
                (Rule::ImageHeightAuto, 2),
                (Rule::ImageShaped, 2),
                (Rule::ImageSizingDropped, 1),
            ],
        ),
    ];
    for (name, hits) in expected {
        assert_eq!(all_hits(&fixtures, &sanitizer, name), *hits, "{name}");
    }
    // Every `n5b-` pair is listed here, and every N5b rule has a pair.
    let listed: Vec<_> = expected.iter().map(|(name, _)| *name).collect();
    for name in fixtures.keys().filter(|name| name.starts_with("n5b-")) {
        assert!(
            listed.contains(&name.as_str()),
            "{name}: list its hits in this test"
        );
    }
    for rule in Rule::ALL
        .into_iter()
        .filter(|rule| rule.step() == Step::N5b)
    {
        assert!(
            expected
                .iter()
                .any(|(_, hits)| hits.iter().any(|(hit, _)| *hit == rule)),
            "{rule:?} has no fixture"
        );
    }
}

#[test]
fn the_image_fixtures_for_review_list_what_n7_removes() {
    // What N7 removes from the images that no step planned, so that the
    // import stops on these rows until an `N7-accept` entry accepts them:
    // what N5b leaves for N7's review (D168), and a width over the bound
    // (reading (e)).
    let fixtures = fixtures();
    let sanitizer = sanitizer();
    let unplanned = |name: &str| -> Vec<(String, usize)> {
        normalise(&fixtures[name].0, &sanitizer)
            .expect("a fixture settles")
            .report
            .unplanned_removals()
            .map(|(removal, count)| (removal.to_string(), count))
            .collect()
    };
    assert_eq!(
        unplanned("n5b-image-sizing-dropped"),
        [
            (
                "style_declaration_rejected img[style] border-style".to_owned(),
                1
            ),
            (
                "style_declaration_rejected img[style] border-width".to_owned(),
                1
            ),
            ("style_declaration_rejected img[style] width".to_owned(), 1),
        ]
    );
    assert_eq!(
        unplanned("n5b-image-width-over-bound"),
        [("style_declaration_rejected img[style] width".to_owned(), 1)]
    );
    // These two are the `n5b-` pairs in UNPLANNED.
    let listed: Vec<_> = UNPLANNED
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| name.starts_with("n5b-"))
        .collect();
    assert_eq!(
        listed,
        ["n5b-image-sizing-dropped", "n5b-image-width-over-bound"]
    );
}

/// The N7 counts of one `n7-` fixture: the planned rule it is named after
/// (`n7-` and the rule's name with `-` for `_`), every N7 count of its body
/// in the order of `Rule`, and what the counts are.
struct N7Fixture {
    /// The rule the fixture is named after.
    rule: Rule,
    /// Every N7 count of its body.
    hits: &'static [(Rule, usize)],
    /// What the counts are, for the failure message.
    why: &'static str,
}

/// The locked styles that a legacy table and its `tbody` always get.
const TABLE: (Rule, usize) = (Rule::LockedMarkupAdded, 2);

/// One fixture per planned N7 rule (data-migration.md §7.1 N7: "These are
/// planned N7 changes, counted per rule"), with its own rule and the
/// locked markup that a legacy table or link always gets beside it.
const N7_FIXTURES: &[N7Fixture] = &[
    N7Fixture {
        rule: Rule::NamedColourToHex,
        hits: &[(Rule::NamedColourToHex, 3)],
        why: "black, windowtext and WhiteSmoke",
    },
    N7Fixture {
        rule: Rule::BorderSideToNone,
        hits: &[(Rule::BorderSideToNone, 3), TABLE],
        why: "a side that was `none` already is not counted",
    },
    N7Fixture {
        rule: Rule::LengthToPx,
        hits: &[(Rule::LengthToPx, 3), TABLE],
        why: "2.5cm, 15pt and 0.75pt",
    },
    N7Fixture {
        rule: Rule::StyleRespelled,
        hits: &[(Rule::StyleRespelled, 1)],
        why: "upper case and a trailing `;`",
    },
    N7Fixture {
        rule: Rule::AttributeRespelled,
        hits: &[(Rule::AttributeRespelled, 1)],
        why: "the default `start`",
    },
    N7Fixture {
        rule: Rule::LinkTargetReplaced,
        hits: &[(Rule::LinkTargetReplaced, 1), (Rule::LockedMarkupAdded, 1)],
        why: "`_self`, and the missing `rel`",
    },
    N7Fixture {
        rule: Rule::LockedMarkupAdded,
        hits: &[(Rule::LockedMarkupAdded, 1)],
        why: "the link; N5b writes the image's locked declarations before N7 sees it",
    },
    N7Fixture {
        rule: Rule::InvalidStyleValueDropped,
        hits: &[TABLE, (Rule::InvalidStyleValueDropped, 1)],
        why: "100%px",
    },
    N7Fixture {
        rule: Rule::UnclearAlignmentDropped,
        hits: &[(Rule::UnclearAlignmentDropped, 1)],
        why: "the `start` behind N5's `right`",
    },
    N7Fixture {
        rule: Rule::TableBorderDropped,
        hits: &[TABLE, (Rule::TableBorderDropped, 1)],
        why: "",
    },
    N7Fixture {
        rule: Rule::TableCellspacingDropped,
        hits: &[TABLE, (Rule::TableCellspacingDropped, 1)],
        why: "",
    },
    N7Fixture {
        rule: Rule::TableCellpaddingDropped,
        hits: &[TABLE, (Rule::TableCellpaddingDropped, 1)],
        why: "",
    },
    N7Fixture {
        rule: Rule::IframeTabindexDropped,
        hits: &[(Rule::IframeTabindexDropped, 1)],
        why: "",
    },
    N7Fixture {
        rule: Rule::IframeScrollingDropped,
        hits: &[(Rule::IframeScrollingDropped, 1)],
        why: "",
    },
    N7Fixture {
        rule: Rule::IframeStyleDropped,
        hits: &[(Rule::IframeStyleDropped, 1)],
        why: "`border` and `overflow`, one attribute",
    },
    N7Fixture {
        rule: Rule::CommentRemoved,
        hits: &[(Rule::CommentRemoved, 1)],
        why: "",
    },
    N7Fixture {
        rule: Rule::ScriptRemoved,
        hits: &[(Rule::ScriptRemoved, 1)],
        why: "",
    },
];

/// Every rule hit of the fixture `name`, in the order of `Rule`.
fn all_hits(
    fixtures: &BTreeMap<String, (String, String)>,
    sanitizer: &Sanitizer,
    name: &str,
) -> Vec<(Rule, usize)> {
    let (input, _) = fixtures
        .get(name)
        .unwrap_or_else(|| panic!("{name} has no fixture"));
    normalise(input, sanitizer)
        .unwrap_or_else(|error| panic!("{name}: refused: {error}"))
        .report
        .hits()
        .collect()
}

/// The N7 counts of the fixture `name`, in the order of `Rule`.
fn n7_hits(
    fixtures: &BTreeMap<String, (String, String)>,
    sanitizer: &Sanitizer,
    name: &str,
) -> Vec<(Rule, usize)> {
    let (input, _) = fixtures
        .get(name)
        .unwrap_or_else(|| panic!("{name} has no fixture"));
    normalise(input, sanitizer)
        .unwrap_or_else(|error| panic!("{name}: refused: {error}"))
        .report
        .hits()
        .filter(|(rule, _)| rule.step() == Step::N7)
        .collect()
}

#[test]
fn n7_counts_its_planned_changes_per_rule() {
    let fixtures = fixtures();
    let sanitizer = sanitizer();
    for N7Fixture { rule, hits, why } in N7_FIXTURES {
        let name = format!("n7-{}", rule.as_str().replace('_', "-"));
        assert_eq!(
            n7_hits(&fixtures, &sanitizer, &name),
            *hits,
            "{name}: {why}"
        );
    }
    // Every planned N7 rule has its fixture.
    for rule in Rule::ALL.into_iter().filter(|rule| rule.step() == Step::N7) {
        assert!(
            N7_FIXTURES.iter().any(|fixture| fixture.rule == rule),
            "{rule:?} has no fixture"
        );
    }
    assert_eq!(
        n7_hits(&fixtures, &sanitizer, "n3-center-unclear-alignment"),
        [(Rule::UnclearAlignmentDropped, 1)],
        "N3's unclear block: N7 drops the `inherit` behind N3's centring"
    );
}

#[test]
fn n7_lists_what_no_step_plans() {
    let fixtures = fixtures();
    let sanitizer = sanitizer();
    assert_eq!(n7_hits(&fixtures, &sanitizer, "n7-unplanned-removals"), []);
    let unplanned: Vec<_> = normalise(&fixtures["n7-unplanned-removals"].0, &sanitizer)
        .expect("a fixture settles")
        .report
        .unplanned_removals()
        .map(|(removal, count)| (removal.to_string(), count))
        .collect();
    assert_eq!(
        unplanned,
        [
            ("element_unwrapped div".to_owned(), 1),
            ("attribute_not_allowed p[class]".to_owned(), 1),
            ("style_rejected p[style] font-family".to_owned(), 1),
            ("style_rejected p[style] margin".to_owned(), 1),
        ]
    );
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
    for (name, _) in UNPLANNED.iter().chain(NOT_YET_EDITOR_FORM) {
        assert!(
            fixtures.contains_key(*name),
            "{name} is listed but has no fixture"
        );
    }
    for name in CRLF_INPUTS {
        assert!(
            fixtures[*name].0.contains("\r\n"),
            "{name}.html lost its \\r\\n line ends; restore them (see .gitattributes)"
        );
    }
}
