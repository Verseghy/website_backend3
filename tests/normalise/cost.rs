//! The normaliser's running time on the shapes that once made N3 quadratic
//! (`src/domain/normalise/center.rs`, "Cost"), on the nested italic headings
//! that N5 plans in one walk (`src/domain/normalise/blocks.rs`, "Cost"), on
//! deep legacy marks, which N5 maps in one walk
//! (`src/domain/normalise/marks.rs`, "Cost"), and on the images that N5b
//! places: many in one paragraph, which it splits in one go, many in marks
//! or spans of one block, whose leftovers it removes in one walk, and one
//! deep in inline elements (`src/domain/normalise/image_layout.rs`,
//! "Cost").
//!
//! The normaliser has no size or depth limit of its own, because its inputs
//! are the fixed legacy dump and backend3's own, already bounded, bodies
//! (`src/domain/normalise/mod.rs`, "Cost"). N7's sanitiser applies its depth
//! limit only after the rules of the first pass, so N3 and N5 still walk a
//! deep body before N7 refuses it: the deep bodies below are refused as
//! `content_too_deep`, and the time until then is still the rules' own.
//! What the rules make of every level of these bodies is checked by the
//! unit test `the_rules_handle_every_level_of_a_deep_body` in
//! `src/domain/normalise/mod.rs`, on the rules alone, without a clock.
//! The images in marks and spans are timed the same way: N5b's output for
//! them is larger than the sanitiser's size limit, so N7 refuses them as
//! `content_too_large` after the rules, and the unit test
//! `many_images_in_marks_and_spans_leave_one_clean_up` in
//! `src/domain/normalise/image_layout.rs` checks what N5b made of them.
//! N3 still must not rescan a subtree or a crowded parent once per
//! `center`: before it kept its answers and stopped at two children, the
//! heading below took about 20 seconds in a debug build, against well under
//! one now. The bound is generous, so that a loaded CI runner does not fail
//! it, and still far below the quadratic time. These tests live here, not
//! beside the code, because `src/domain/` may not read a clock (ADR 0008).

use std::time::{Duration, Instant};

use backend3::domain::normalise::{NormaliseError, Rule, normalise};
use backend3::domain::rich_text::{SanitizePolicy, Sanitizer};

/// How long one body may take, in any build.
const BOUND: Duration = Duration::from_secs(10);

fn sanitizer() -> Sanitizer {
    Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
}

/// Asserts that `result` is N7's refusal of a body deeper than the
/// sanitiser's limit.
fn assert_too_deep<T: std::fmt::Debug>(result: Result<T, NormaliseError>) {
    match result {
        Err(NormaliseError::SanitiseRefused(error)) => {
            assert_eq!(error.code(), Some("content_too_deep"), "{error}");
        }
        other => panic!("expected content_too_deep, got {other:?}"),
    }
}

/// Asserts that `result` is N7's refusal of a body larger than the
/// sanitiser's limit.
fn assert_too_large<T: std::fmt::Debug>(result: Result<T, NormaliseError>) {
    match result {
        Err(NormaliseError::SanitiseRefused(error)) => {
            assert_eq!(error.code(), Some("content_too_large"), "{error}");
        }
        other => panic!("expected content_too_large, got {other:?}"),
    }
}

#[test]
fn many_sibling_centres_in_a_heading_take_linear_time() {
    let body = format!("<h2>{}</h2>", "<center>x</center>".repeat(5_000));
    let started = Instant::now();
    let out = normalise(&body, &sanitizer()).expect("the body settles");
    let elapsed = started.elapsed();
    assert_eq!(out.html, format!("<h2>{}</h2>", "x".repeat(5_000)));
    assert_eq!(out.report.count(Rule::CenterAlignmentDropped), 5_000);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn centres_around_a_deep_inline_chain_do_not_rescan_it() {
    // Each `center` asks whether the `span` chain below it holds a block,
    // and whether it holds visible text; without kept answers that rescans
    // the whole chain once per `center`, so the time grows with the square
    // of the depth. html5ever's parse of this depth is itself above
    // linear (the module documentation of `normalise`, "Cost"), and takes
    // most of the time left. N7 then refuses the depth; the unit test
    // `the_rules_handle_every_level_of_a_deep_body` checks what N3 made of
    // every level.
    let depth = 4_000;
    let body = format!(
        "{}x{}",
        "<center><span>".repeat(depth),
        "</span></center>".repeat(depth)
    );
    let started = Instant::now();
    let result = normalise(&body, &sanitizer());
    let elapsed = started.elapsed();
    assert_too_deep(result);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn nested_italic_headings_do_not_rescan_their_content() {
    // Only tag soup nests one heading in another, with a `div` between
    // them. Each italic `h2` asks which `em` and `i` below it hold no block;
    // asked per heading, that walks the content of every heading once for
    // each heading around it, so the time grows with the square of the
    // depth: this body took about 12 seconds that way in a debug build. N5
    // answers every heading in one walk. The `object` is a scope boundary
    // for the parser, where html5ever's scope checks stop, so that its parse
    // of this depth stays linear and the time is the normaliser's own. N7
    // then refuses the depth; the unit test
    // `the_rules_handle_every_level_of_a_deep_body` checks that every
    // heading got its `em`.
    let depth = 5_000;
    let body = format!(
        "{}x{}",
        r#"<h2 style="font-style: italic"><div><object>"#.repeat(depth),
        "</object></div></h2>".repeat(depth)
    );
    let started = Instant::now();
    let result = normalise(&body, &sanitizer());
    let elapsed = started.elapsed();
    assert_too_deep(result);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn n7_reviews_a_large_legacy_table_within_the_bound() {
    // N7 reads every `style` a second time to judge its declarations; a
    // table of many styled cells, as Excel pastes them, must not make that
    // reading rescan the table per cell, which would grow with the square
    // of its 2,000 cells. One size against the bound, like the tests
    // above: a ratio of two timings would fail on a loaded runner.
    let cell = r#"<td style="border-top: 1pt solid windowtext; border-bottom: medium none; width: 100%px; font-family: Calibri">x</td>"#;
    let body = format!(
        r#"<table border="1" cellspacing="0" cellpadding="0"><tbody>{}</tbody></table>"#,
        format!("<tr>{}</tr>", cell.repeat(20)).repeat(100)
    );
    let started = Instant::now();
    let out = normalise(&body, &sanitizer()).expect("the body settles");
    let elapsed = started.elapsed();
    assert_eq!(out.report.count(Rule::LengthToPx), 2_000);
    assert_eq!(out.report.count(Rule::InvalidStyleValueDropped), 2_000);
    assert_eq!(out.report.count(Rule::TableBorderDropped), 1);
    assert!(
        out.report.has_unplanned_removals(),
        "font-family is reviewed"
    );
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn deep_legacy_marks_are_mapped_in_one_walk() {
    // N5 looks at each element once, and an unwrap moves only the element's
    // children, so a deep chain of unwrapped `big` and `span[dir]` elements
    // costs no more than its parse (`src/domain/normalise/marks.rs`, "Cost").
    // N7 then refuses the 2,000 `strong` elements left nested in each
    // other; the unit test `the_rules_handle_every_level_of_a_deep_body`
    // checks what the rules made of them.
    let depth = 2_000;
    let body = format!(
        "{}x{}",
        r#"<big><span dir="ltr"><b>"#.repeat(depth),
        "</b></span></big>".repeat(depth)
    );
    let started = Instant::now();
    let result = normalise(&body, &sanitizer());
    let elapsed = started.elapsed();
    assert_too_deep(result);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

/// The four locked declarations of every stored image (editor.md §3.3).
const IMAGE_STYLE: &str = "display: block; height: auto; margin: 0 auto; max-width: 100%";

#[test]
fn many_images_split_their_paragraph_in_one_go() {
    // N5b splits a paragraph at all of its images at once, so each node of
    // the paragraph moves once (`src/domain/normalise/image_layout.rs`,
    // "Cost"); one split per image would move the rest of the paragraph
    // each time, and grow with the square of the number of images.
    let count = 5_000;
    let image = r#"<img src="/storage/uploads/a.jpg" alt="a">"#;
    let body = format!("<p>{}</p>", format!("x {image} ").repeat(count));
    let started = Instant::now();
    let out = normalise(&body, &sanitizer()).expect("the body settles");
    let elapsed = started.elapsed();
    let paragraph = format!(
        r#"<p>x</p><p><img src="/storage/uploads/a.jpg" alt="a" style="{IMAGE_STYLE}"></p>"#
    );
    assert_eq!(out.html, paragraph.repeat(count));
    assert_eq!(out.report.count(Rule::ImageParagraphSplit), count);
    assert_eq!(out.report.count(Rule::ImageShaped), count);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn many_images_in_marks_leave_their_paragraph_in_one_clean_up() {
    // Each image moves out of its `strong` in front of the paragraph, and
    // leaves its white space and its emptied mark behind, before the
    // marks still to come. Asking the paragraph whether it still shows
    // something after each move would scan that growing run once per
    // image, which grows with the square of the number of images (about
    // 5 seconds for 8,000 in a debug build); the clean-up after the last
    // move asks each node once (`src/domain/normalise/image_layout.rs`,
    // "Cost"). With 20,000 images, the body that N5b gives N7 is larger
    // than the sanitiser's limit, so N7 refuses it right after the rules
    // of the first pass, and the time is the rules' own; the unit test
    // `many_images_in_marks_and_spans_leave_one_clean_up` in that file
    // checks what N5b makes of it.
    let count = 20_000;
    let image = r#"<img src="/storage/uploads/a.jpg" alt="a">"#;
    let body = format!(
        "<p>{}</p>",
        format!(" <strong>{image}</strong>").repeat(count)
    );
    let started = Instant::now();
    let result = normalise(&body, &sanitizer());
    let elapsed = started.elapsed();
    assert_too_large(result);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn many_images_in_spans_leave_their_block_in_one_clean_up() {
    // The same in a `div`, with an empty `span` beside each image's own:
    // the empty elements that stay are asked once too (about 10 seconds
    // for 8,000 in a debug build when asked once per image). N7 refuses
    // this body as too large too, after the rules.
    let count = 16_000;
    let image = r#"<img src="/storage/uploads/a.jpg" alt="a">"#;
    let body = format!(
        "<div>{}szöveg</div>",
        format!("<span></span><span>{image}</span>").repeat(count)
    );
    let started = Instant::now();
    let result = normalise(&body, &sanitizer());
    let elapsed = started.elapsed();
    assert_too_large(result);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn an_image_deep_in_inline_elements_moves_out_once() {
    // The image walks its ancestors once to find its top-level block, and
    // the clean-up removes each emptied element once, innermost first.
    let depth = 2_000;
    let body = format!(
        r#"<h2>{}<img src="/storage/uploads/a.jpg" alt="a">{}</h2>"#,
        "<span>".repeat(depth),
        "</span>".repeat(depth)
    );
    let started = Instant::now();
    let out = normalise(&body, &sanitizer()).expect("the body settles");
    let elapsed = started.elapsed();
    assert_eq!(
        out.html,
        format!(r#"<p><img src="/storage/uploads/a.jpg" alt="a" style="{IMAGE_STYLE}"></p>"#)
    );
    assert_eq!(out.report.count(Rule::ImageMovedBeforeBlock), 1);
    assert!(elapsed < BOUND, "{elapsed:?}");
}
