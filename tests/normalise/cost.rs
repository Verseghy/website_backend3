//! The normaliser's running time on the shapes that once made N3 quadratic
//! (`src/domain/normalise/center.rs`, "Cost"), on the nested italic headings
//! that N5 plans in one walk (`src/domain/normalise/blocks.rs`, "Cost"), and
//! on deep legacy marks, which N5 maps in one walk
//! (`src/domain/normalise/marks.rs`, "Cost").
//!
//! The normaliser has no size or depth limit of its own, because its inputs
//! are the fixed legacy dump and backend3's own, already bounded, bodies
//! (`src/domain/normalise/mod.rs`, "Cost"). N3 still must not rescan a
//! subtree or a crowded parent once per `center`: before it kept its answers
//! and stopped at two children, the heading below took about 20 seconds in a
//! debug build, against well under one now. The bound is generous, so that
//! a loaded CI runner does not fail it, and still far below the quadratic
//! time. These tests live here, not beside the code, because `src/domain/`
//! may not read a clock (ADR 0008).

use std::time::{Duration, Instant};

use backend3::domain::normalise::{Rule, normalise};

/// How long one body may take, in any build.
const BOUND: Duration = Duration::from_secs(10);

#[test]
fn many_sibling_centres_in_a_heading_take_linear_time() {
    let body = format!("<h2>{}</h2>", "<center>x</center>".repeat(5_000));
    let started = Instant::now();
    let out = normalise(&body).expect("the body settles");
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
    // most of the time left.
    let depth = 4_000;
    let body = format!(
        "{}x{}",
        "<center><span>".repeat(depth),
        "</span></center>".repeat(depth)
    );
    let started = Instant::now();
    let out = normalise(&body).expect("the body settles");
    let elapsed = started.elapsed();
    assert!(!out.html.contains("<center"), "{}", out.html);
    assert_eq!(out.report.count(Rule::CenterUnwrapped), depth);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn nested_italic_headings_do_not_rescan_their_content() {
    // Only tag soup nests one heading in another, with a `div` between
    // them. Each italic `h2` asks which `em` and `i` below it hold no block;
    // asked per heading, that walks the content of every heading once for
    // each heading around it, so the time grows with the square of the
    // depth: this body took about 12 seconds that way in a debug build. N5
    // answers every heading in one walk. Every heading keeps its own `em`
    // here, because each holds a `div`. The `object` is a scope boundary for
    // the parser, where html5ever's scope checks stop, so that its parse of
    // this depth stays linear and the time is the normaliser's own.
    let depth = 5_000;
    let body = format!(
        "{}x{}",
        r#"<h2 style="font-style: italic"><div><object>"#.repeat(depth),
        "</object></div></h2>".repeat(depth)
    );
    let started = Instant::now();
    let out = normalise(&body).expect("the body settles");
    let elapsed = started.elapsed();
    assert_eq!(
        out.html,
        format!(
            "{}x{}",
            "<h2><em><div><object>".repeat(depth),
            "</object></div></em></h2>".repeat(depth)
        )
    );
    assert_eq!(out.report.count(Rule::ItalicHeadingToEm), depth);
    assert!(elapsed < BOUND, "{elapsed:?}");
}

#[test]
fn deep_legacy_marks_are_mapped_in_one_walk() {
    // N5 looks at each element once, and an unwrap moves only the element's
    // children, so a deep chain of unwrapped `big` and `span[dir]` elements
    // costs no more than its parse (`src/domain/normalise/marks.rs`, "Cost").
    let depth = 2_000;
    let body = format!(
        "{}x{}",
        r#"<big><span dir="ltr"><b>"#.repeat(depth),
        "</b></span></big>".repeat(depth)
    );
    let started = Instant::now();
    let out = normalise(&body).expect("the body settles");
    let elapsed = started.elapsed();
    assert_eq!(
        out.html,
        format!("{}x{}", "<strong>".repeat(depth), "</strong>".repeat(depth))
    );
    assert_eq!(out.report.count(Rule::BigUnwrapped), depth);
    assert_eq!(out.report.count(Rule::SpanDirUnwrapped), depth);
    assert_eq!(out.report.count(Rule::BToStrong), depth);
    assert!(elapsed < BOUND, "{elapsed:?}");
}
