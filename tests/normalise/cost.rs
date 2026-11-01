//! The normaliser's running time on the shapes that once made N3 quadratic
//! (`src/domain/normalise/center.rs`, "Cost").
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
