//! `validate_content` and `html_to_search_text` through the public API
//! (editor.md §7.6, ADR 0020 §2), over the P1 generator's tag soup and the
//! permanent regression fixtures.
//!
//! The validator gives a finding its index from a walk of the original
//! input, and trusts that walk only where it agrees with the removal report
//! and the sanitised output; otherwise the finding has no index. The
//! module documentation of `domain::rich_text::validate` argues that they
//! always agree. These tests check it where the parser's repair rules are
//! (foster parenting, the adoption agency, raw text, SVG and `MathML`,
//! `annotation-xml` integration points): every image and embed finding of
//! every soup body has its index. The unit tests and proptests of each
//! function live next to the code.

use backend3::domain::rich_text::{
    ContentError, SanitizePolicy, html_to_search_text, refusal, validate_content,
};
use proptest::prelude::*;

use super::properties::{arb_html, config};
use super::regressions::fixtures;
use super::sanitizer;

/// The findings of an admin write of `input`, or of the sanitiser's
/// refusal.
fn findings(input: &str) -> Vec<ContentError> {
    match sanitizer().sanitize(input) {
        Ok(result) => validate_content(
            input,
            &result.html,
            &result.report,
            &SanitizePolicy::content_v1(),
        )
        .err()
        .unwrap_or_default(),
        Err(error) => refusal(&error).into_iter().collect(),
    }
}

/// Every finding about one image or embed carries its position.
fn assert_traced(input: &str, findings: &[ContentError]) -> Result<(), TestCaseError> {
    for finding in findings {
        let about_an_element = matches!(
            finding,
            ContentError::ImageAltMissing { .. }
                | ContentError::ImageSrcRejected { .. }
                | ContentError::EmbedRejected { .. }
                | ContentError::IframeContent { .. }
        );
        prop_assert!(
            !about_an_element || finding.index().is_some(),
            "{:?} has no index for {:?}",
            finding,
            input
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn every_finding_of_tag_soup_has_its_index(input in arb_html()) {
        assert_traced(&input, &findings(&input))?;
    }

    #[test]
    fn the_search_text_of_tag_soup_is_one_clean_line(input in arb_html()) {
        let text = html_to_search_text(&input);
        prop_assert_eq!(text.trim(), text.as_str());
        prop_assert!(!text.contains("  "), "{:?}", text);
        prop_assert!(text.chars().all(|c| c == ' ' || !c.is_whitespace()), "{:?}", text);
        prop_assert!(!text.contains('\0'));
    }
}

#[test]
fn every_fixture_validates_with_traced_findings() {
    for (name, (input, expected)) in fixtures() {
        assert_traced(&input, &findings(&input)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_traced(&expected, &findings(&expected))
            .unwrap_or_else(|error| panic!("{name}.out: {error}"));
    }
}

#[test]
fn the_canonical_shapes_are_a_valid_admin_write() {
    // Every editor.md §3.7 shape is what the editor writes, so an admin
    // write of it is accepted: its images have alt text and kept sources,
    // and its embeds are allowed and empty.
    let (_, expected) = &fixtures()["canonical-shapes"];
    assert!(expected.contains("<img"), "the fixture has an image");
    assert!(expected.contains("<iframe"), "the fixture has an embed");
    assert_eq!(findings(expected), []);
}

#[test]
fn the_search_text_of_the_canonical_shapes_holds_only_their_words() {
    let (_, expected) = &fixtures()["canonical-shapes"];
    let text = html_to_search_text(expected);
    assert_ne!(text, "");
    for markup in [
        "<", "style", "storage", "youtube", "padlet", "noopener", "#",
    ] {
        assert!(!text.contains(markup), "{markup} in {text:?}");
    }
}
