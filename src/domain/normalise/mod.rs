//! The body normaliser of the legacy import: legacy HTML in, content-v1 out
//! (data-migration.md §7, ADR 0017 §2, ADR 0013 §10).
//!
//! The legacy bodies were written by `CKEditor` 4.12.1, whose `entities`
//! plugin stored accented letters and typographic punctuation as named
//! character references, plus markup typed in its source mode that the new
//! editor does not know (research: SYNTHESIS §4.7, gap-05 §2, gap-06 §1-4).
//! A DOM-based editor would rewrite those bytes on its first save anyway,
//! and drop what it does not know. So the import rewrites every
//! `posts.content` and `pages.content` once, deterministically, into the
//! exact form the editor stores, and keeps the original bytes in the row's
//! `import_original` revision. The same function is also the
//! `content_normalize` task's, over backend3's own bodies, where it must
//! change nothing (V13).
//!
//! Bodies are HTML, never Markdown: they have been HTML since the 2020
//! conversion (parity POS-C11), so text such as `**félkövér**` or `# Cím`
//! is kept as the text it is.
//!
//! [`normalise`] parses the body and works on the DOM, not on strings.
//! One pass runs the steps of data-migration.md §7.1 that this module holds
//! so far, in their order:
//!
//! - **N0** (`dom.rs`): html5ever's fragment parse, the sanitiser's own,
//!   which decodes every character reference, lower-cases tag names and
//!   normalises line ends and void tags;
//! - **N3** (`center.rs`): `<center>` unwrapped into `text-align: center`;
//! - **N5**, its block rules (`blocks.rs`): `p[align]` to `text-align`,
//!   `table[align]` dropped, `h5` and `h6` to `h4`, an italic `h2` to
//!   `<h2><em>`, and `p[lang]` stripped;
//! - **N5**, its inline rules (`marks.rs`): the legacy marks `b`, `i`,
//!   `ins`, `strike` and `del` renamed to `strong`, `em`, `u` and `s`, `big`
//!   unwrapped, `span.marker` turned into a yellow highlight, and
//!   `span[dir]` unwrapped (a marker span loses only its `dir`);
//! - **N5b**, its image rules (`image_layout.rs`): every image that N7
//!   keeps in the canonical image paragraph, alone at the top level, its
//!   float removed, its legacy height turned into `height: auto` with the
//!   ratio in its `width` and `height` attributes, and its width kept; an
//!   image in a paragraph's running text splits the paragraph, and one
//!   anywhere else moves in front of its top-level block, without a link
//!   around it;
//! - **N6** (`dead_host.rs`): `src` and `href` on the dead host's
//!   `/storage/` rewritten to root-relative `/storage/`;
//! - **N7** (`sanitise.rs`): the sanitiser that every write uses, given as
//!   an argument, with what it removed compared with the planned N7 rules:
//!   the planned changes are counted per rule, and every removal that no
//!   step planned is reported ([`UnplannedRemoval`]), which stops the
//!   import unless the editors accept it (`N7-accept`,
//!   `domain::import_mysql::n7_review`);
//! - **N8** (`dom.rs`): html5ever's serialisation, which leaves only
//!   `&amp; &lt; &gt; &nbsp;` and, in attributes, `&quot;` as references.
//!
//! The steps that later slices add slot in where §7.1 orders them: N1 and
//! N2 (legacy embeds) between N0 and N3; N4 (inline `data:` images)
//! between N3 and N5; the rest of N5b (tables, tightening and structure)
//! after its image rules. Each step records what it changed for the
//! run report ([`report`]), and the pass also lists, without changing them,
//! the dead-host occurrences N6 leaves and the images without alt text
//! (editor.md §9), from the tree that N7 left. It also records the images
//! wider than content-v1's 2000 px bound whose width N7 drops (editor.md
//! §16 item 5), from the tree that N7 is given.
//!
//! **The fixed-point loop.** A pass's output is parsed again by the next
//! reader, and html5ever's markup for a tree does not always parse back
//! into that tree: foster parenting and the adoption agency can move what
//! the first parse placed (see `super::rich_text::structure`, "How the
//! output settles"), and the parser drops the first line feed after a
//! `pre`. So [`normalise`] repeats the pass until one returns its own
//! input, at most [`MAX_PASSES`] times, as the sanitiser does (ADR 0013 §5).
//! Its output `z` therefore has `pass(z) == z`, and normalising it again
//! returns it with no rule hits: P4, `normalise(normalise(x)) ==
//! normalise(x)` (editor.md §13.6, data-migration.md §11.2), holds for
//! every body the normaliser accepts. The rules only remove what they look
//! for (`center`; the `align` and `lang` attributes, the `h5` and `h6` tag
//! names and the italic `font-style` of N5's block rules; the legacy marks
//! of its inline rules; the dead `/storage/` prefixes; what N7's sanitiser
//! removes, whose output is its own fixed point), or write what they leave
//! alone (N5b's canonical image paragraph), so the passes after the
//! first only re-parse. A body that is still changing after the last pass
//! is refused with [`NormaliseError::NotNormalisable`], and the import
//! treats that like a failed N9 check (data-migration.md §4.2). Before N7
//! ran in the pass, that refused a `plaintext` element, whose end tag the
//! serialiser writes and no parser reads, a `pre` or `listing` whose
//! content starts with three or more line feeds (each parse drops one),
//! and tag soup that leaves a link inside another link: a link left open
//! around deeply nested lists and headings with another link inside them,
//! where each parse's adoption agency moves a copy of the outer link at
//! most eight blocks further in
//! (`a_link_left_open_around_sixteen_blocks_settles_since_n7`), or where the
//! headings around those copies still move after the nesting is gone, as
//! when N3 unwraps a centre between two headings
//! (`a_centre_between_headings_in_a_nested_link_settles_since_n7`), and a
//! link fostered out of a table into another link. The maintainer accepted
//! the rare refusal of a link left open around nested blocks as built, the
//! one that N3's centre unwrap causes included, over a new N3 rule that
//! would settle its shape now (data-migration.md §7.4; research: DECISIONS
//! R37 (D159)). N7's sanitiser now settles all of these shapes in the first
//! pass: its own fixed-point loop parses and writes the body again (ADR
//! 0013 §5), which absorbs the parser's repairs, its structural pass
//! unwraps a link still nested in another (ADR 0037), and it unwraps the
//! `pre`, `listing` and `plaintext` that content-v1 does not have, so a
//! `pre` or `listing` keeps the line feeds the parses left and a
//! `plaintext` element its unread end tag as text. Those unwraps are
//! unplanned removals that an editor reviews; the nested-link soup of the
//! tests settles with planned changes only, so it is imported, and a link
//! that the structural pass still unwraps goes to review. The maintainer
//! confirmed this as built on 2026-11-15 (research: DECISIONS R38 (D162)),
//! and D159's answer still covers any shape where N3 would add a refusal.
//! None of these is a shape `CKEditor` writes; the tests measure that tag
//! soup no longer meets a refusal, and `settle` keeps the loop's rule
//! tested.
//!
//! **N9** closes every run: the loop has already shown `normalise(out) ==
//! out` (its last pass returned its own input), and [`normalise`] then
//! sanitises the output once more and refuses it when `S(out) != out`
//! ([`NormaliseError::NotSanitiserFixedPoint`]). [`check_fixed_point`]
//! ([`fixed_point`]) runs both checks on a body from elsewhere, such as a
//! stored one.
//!
//! **Cost.** html5ever's tree builder and every walk here are iterative, so
//! nesting depth costs no stack. It does cost time: the parse grows with
//! the body's size times its element depth (the parser's scope checks walk
//! the stack of open elements), and N3 visits a node once for each `center`
//! around it, so it grows with the size times the nesting depth of
//! `center` elements; N3 answers its questions about a subtree once per
//! pass and looks at most at two children of a crowded parent (`center.rs`,
//! "Cost"). The other rules are linear: N5's block rules plan every edit in
//! one walk, the `em` and `i` inside nested italic headings included
//! (`blocks.rs`, "Cost"), and its inline rules look at each element once
//! (`marks.rs`, "Cost"). N5b's image rules walk each image's ancestors
//! once, so they grow with the number of images times their depth, like
//! the parse, move each node of a split paragraph once, and remove what
//! the moves leave empty in one walk over the body after the last move
//! (`image_layout.rs`, "Cost"). The normaliser has no limit of its own,
//! which is acceptable only because of its two inputs: the import's is the
//! fixed legacy dump, and backend3's own bodies, which `content_normalize`
//! reads, hold no `center` and are bounded by the sanitiser's size and
//! depth limits (editor.md §7.2; research: DECISIONS R11). N7 applies those
//! limits to the import too, after the rules of the first pass: a deeper or
//! larger legacy body is refused ([`NormaliseError::SanitiseRefused`]).
//! N7 adds a sanitiser run per pass and the N9 run at the end, each
//! linear in the body after its limits (editor.md §7.2). Like everything
//! under `src/domain/`, the module is pure (ADR 0008).
//!
//! ```
//! use backend3::domain::normalise::{Rule, normalise};
//! use backend3::domain::rich_text::{SanitizePolicy, Sanitizer};
//!
//! let sanitizer = Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy");
//! let out = normalise(
//!     r#"<center><h2>Eredm&eacute;nyek</h2></center><p><a href="https://backend.verseghy-gimnazium.net/storage/a.pdf">PDF</a></p><table border="1"><tbody><tr><td style="border-top: 1px solid black">x</td></tr></tbody></table>"#,
//!     &sanitizer,
//! )
//! .expect("a body that settles");
//! assert_eq!(
//!     out.html,
//!     concat!(
//!         r#"<h2 style="text-align: center">Eredmények</h2><p><a href="/storage/a.pdf" target="_blank" rel="noopener noreferrer">PDF</a></p>"#,
//!         r#"<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td style="border-bottom: none; border-left: none; border-right: none; border-top: 1px solid #000000">x</td></tr></tbody></table>"#,
//!     )
//! );
//! assert_eq!(out.report.count(Rule::CenterUnwrapped), 1);
//! assert_eq!(out.report.count(Rule::DeadHostLinkRewritten), 1);
//! assert_eq!(out.report.count(Rule::TableBorderDropped), 1);
//! assert_eq!(out.report.count(Rule::NamedColourToHex), 1);
//! assert!(!out.report.has_unplanned_removals());
//! ```

mod blocks;
mod center;
mod dead_host;
mod dom;
pub mod fixed_point;
mod image_layout;
mod images;
mod invalid_css;
mod marks;
pub mod report;
mod sanitise;

use std::borrow::Cow;

pub use fixed_point::{FixedPointError, check_fixed_point};
pub use report::{
    AltState, DeadHostOccurrence, ImageWithoutAlt, NormaliseReport, Place, Rule, Step,
    UnplannedRemoval, WideImage,
};

use crate::domain::rich_text::{RemovalReport, SanitizeError, Sanitizer};
use report::Observations;

/// The most passes one run makes before it refuses the body. Three, as in
/// the sanitiser (ADR 0013 §5): the first pass applies the rules, a second
/// one absorbs what the first output's parse moves, and the last may only
/// confirm.
pub const MAX_PASSES: usize = 3;

/// A normalised body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalised {
    /// The body in the editor's stored form, so far as the steps of this
    /// module take it. It is content-v1: the sanitiser returns it
    /// unchanged (N9).
    pub html: String,
    /// What the run changed and observed, and the removals that no step
    /// planned ([`NormaliseReport::unplanned_removals`]).
    pub report: NormaliseReport,
}

/// Why the normaliser refused a body. Nothing is written for a refused
/// body; the import's dry run lists it with the reason, and `apply` stops
/// on it (data-migration.md §4.2, §7.4; D140).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NormaliseError {
    /// The body was still changing after [`MAX_PASSES`] passes: html5ever's
    /// parse and its serialisation of the parsed tree disagree about it.
    #[error("the body did not reach a fixed point in {passes} normaliser passes")]
    NotNormalisable {
        /// How many passes ran.
        passes: usize,
    },
    /// N7's sanitiser refused the body: it is larger or nests deeper than
    /// the sanitiser's limits, the sanitiser's own passes did not settle,
    /// or the sanitiser broke an invariant ([`SanitizeError::code`] tells
    /// them apart).
    #[error("the sanitiser refused the body: {0}")]
    SanitiseRefused(SanitizeError),
    /// N9: sanitising the normalised body once more changes it, `S(out) !=
    /// out`. The pass ends with the sanitiser and its parse, so this means
    /// the normaliser's serialisation and the sanitiser's disagree, which
    /// is a bug.
    #[error("sanitising the normalised body again changes it from byte {at}")]
    NotSanitiserFixedPoint {
        /// The byte offset of the first difference.
        at: usize,
        /// What that sanitiser run removed or rewrote.
        report: RemovalReport,
    },
}

/// Normalises one body (see the module documentation): the passes until
/// one returns its input, then N9.
///
/// - `raw`: the body as stored, legacy or content-v1. A NULL body is not
///   passed here: it stays NULL (data-migration.md §6.1, D4).
/// - `sanitizer`: the deployment's sanitiser, the one every write path uses
///   (ADR 0013 §5), for N7 and N9. Its policy's limits apply to the body.
///
/// # Errors
///
/// - [`NormaliseError::NotNormalisable`] when the body has not settled after
///   [`MAX_PASSES`] passes;
/// - [`NormaliseError::SanitiseRefused`] when N7's sanitiser refuses it;
/// - [`NormaliseError::NotSanitiserFixedPoint`] when N9 finds that the
///   sanitiser still changes the output.
pub fn normalise(raw: &str, sanitizer: &Sanitizer) -> Result<Normalised, NormaliseError> {
    normalise_with(raw, sanitizer, |input, report| {
        pass(input, sanitizer, report)
    })
}

/// [`normalise`] with the pass given as an argument: the passes until one
/// returns its input, then N9. Apart from [`normalise`] so that the tests
/// can check that N9 refuses a fixed point the sanitiser still changes,
/// which the real pass, ending with the sanitiser, never returns.
///
/// - `raw`: the body, as for [`normalise`].
/// - `sanitizer`: the sanitiser of the N9 check.
/// - `pass`: one pass over its input, adding to the run's report.
///
/// # Errors
///
/// As [`normalise`].
fn normalise_with(
    raw: &str,
    sanitizer: &Sanitizer,
    mut pass: impl FnMut(&str, &mut NormaliseReport) -> Result<(String, Observations), NormaliseError>,
) -> Result<Normalised, NormaliseError> {
    let mut report = NormaliseReport::default();
    let (html, observations) = settle(raw, MAX_PASSES, |input| pass(input, &mut report))?;
    check_sanitiser_fixed_point(&html, sanitizer)?;
    report.observe(observations);
    Ok(Normalised { html, report })
}

/// Applies `pass` until its output equals its input, at most `max_passes`
/// times, and returns that output with its pass's observations. It is
/// generic over the pass, like the sanitiser's own loop, so that the loop's
/// rules (stop at the first fixed point, refuse after the last pass) are
/// tested without an input that the passes fail to settle: since N7 runs in
/// the pass, the tests know of none.
///
/// - `raw`: the first input.
/// - `max_passes`: how many passes may run; the last one may only confirm.
/// - `pass`: one pass; its error stops the loop.
///
/// # Errors
///
/// The error of a pass, or [`NormaliseError::NotNormalisable`] when the
/// last pass still changed its input.
fn settle(
    raw: &str,
    max_passes: usize,
    mut pass: impl FnMut(&str) -> Result<(String, Observations), NormaliseError>,
) -> Result<(String, Observations), NormaliseError> {
    let mut current = Cow::Borrowed(raw);
    for _ in 0..max_passes {
        let (output, observations) = pass(&current)?;
        if output == *current {
            return Ok((output, observations));
        }
        current = Cow::Owned(output);
    }
    Err(NormaliseError::NotNormalisable { passes: max_passes })
}

/// N9's sanitiser check on a normalised body: `S(out) == out`.
///
/// - `out`: the body the passes settled on.
/// - `sanitizer`: the sanitiser N7 used.
///
/// # Errors
///
/// [`NormaliseError::SanitiseRefused`] when the sanitiser refuses the body,
/// and [`NormaliseError::NotSanitiserFixedPoint`] when it changes it.
fn check_sanitiser_fixed_point(out: &str, sanitizer: &Sanitizer) -> Result<(), NormaliseError> {
    let again = sanitizer
        .sanitize(out)
        .map_err(NormaliseError::SanitiseRefused)?;
    if again.html.as_str() == out {
        Ok(())
    } else {
        Err(NormaliseError::NotSanitiserFixedPoint {
            at: fixed_point::first_difference(out, again.html.as_str()),
            report: again.report,
        })
    }
}

/// One pass: N0, the rules in their order, N7, the observations, N8. The
/// rules and N7 add their hits and N7 its unplanned removals to `report`;
/// the observations are returned, and only the last pass's are kept.
///
/// N7 takes a string, so the tree after N6 is written out for it and its
/// output parsed again; N8 then writes that tree, so the observations
/// describe what is stored.
///
/// - `input`: the body, or the previous pass's output.
/// - `sanitizer`: the sanitiser for N7.
/// - `report`: the report of the current run.
///
/// # Errors
///
/// [`NormaliseError::SanitiseRefused`] when N7's sanitiser refuses the
/// body.
fn pass(
    input: &str,
    sanitizer: &Sanitizer,
    report: &mut NormaliseReport,
) -> Result<(String, Observations), NormaliseError> {
    let mut html = dom::parse(input);
    let unclear_before = unclear_blocks(report);
    apply_rules(&mut html, report);
    // The images whose width is over content-v1's bound, from the tree
    // that N7 is given: N7 drops that width, so its output no longer shows
    // it (`images.rs`).
    report.record_wide_images(images::wider_than_bound(&html));
    // N7 after every rule, so that it sees what they planned: the
    // alignment N3 and N5 write in front of an unclear one, for as many
    // blocks as they counted unclear in this pass.
    let unclear = unclear_blocks(report) - unclear_before;
    let sanitised = sanitise::sanitise(&dom::serialise(&html), sanitizer, unclear, report)?;
    let html = dom::parse(&sanitised);
    let observations = Observations {
        dead_host_kept: dead_host::occurrences(&html),
        images_without_alt: images::without_alt(&html),
    };
    Ok((dom::serialise(&html), observations))
}

/// The rules of one pass in their order, N3, N5 (its block rules, then its
/// inline rules), N5b (its image rules) and N6, on the tree that N0 parsed;
/// they add their hits to `report`. A step of its own, apart from N7, so
/// that the tests can check what the rules do to a body deeper than N7's
/// sanitiser accepts (the cost tests' shapes): N7 refuses such a body
/// after the rules of the first pass, and its refusal carries no report.
///
/// - `html`: the parsed body, edited in place.
/// - `report`: the report of the current run.
fn apply_rules(html: &mut scraper::Html, report: &mut NormaliseReport) {
    center::unwrap_centres(html, report);
    // After N3, which reads a paragraph's `align`, counts `h5` and `h6`
    // among the aligned headings and must centre an italic `h2` before N5
    // puts an `em` inside it (`blocks.rs`, "Order").
    blocks::map_blocks(html, report);
    // N5's inline rules, after its block rules: an `i` that the
    // italic-heading rule unwraps is gone before the `i` rule runs, so it is
    // not also counted as `i_to_em` (`marks.rs`, "After N3 and N5's block
    // rules").
    marks::map_marks(html, report);
    // N5b's image rules, after N5: the marks around an image are
    // content-v1's by then, and N3's centred paragraph around an image
    // becomes the plain image paragraph (`image_layout.rs`, "Order").
    image_layout::shape_images(html, report);
    dead_host::rewrite_links(html, report);
}

/// How many blocks N3 and N5 have counted as unclear in the run so far
/// (`center_alignment_unclear`, `paragraph_align_unclear`): each wrote an
/// alignment in front of a `text-align` that content-v1 does not keep, for
/// N7 to drop as planned.
///
/// - `report`: the report of the current run.
fn unclear_blocks(report: &NormaliseReport) -> usize {
    report.count(Rule::CenterAlignmentUnclear) + report.count(Rule::ParagraphAlignUnclear)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::SanitizePolicy;
    use crate::domain::rich_text::search_text::html_to_search_text;
    use proptest::prelude::*;
    use std::sync::LazyLock;

    /// The content-v1 sanitiser, built once for the tests of this module:
    /// the proptests normalise thousands of bodies.
    static SANITIZER: LazyLock<Sanitizer> = LazyLock::new(|| {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
    });

    /// [`normalise`] with the content-v1 sanitiser.
    fn run(raw: &str) -> Result<Normalised, NormaliseError> {
        normalise(raw, &SANITIZER)
    }

    fn normalised(raw: &str) -> String {
        run(raw).expect("a body that settles").html
    }

    #[test]
    fn entity_encoded_bodies_become_literal_utf8() {
        // POS-C06: CKEditor's named references, decoded once.
        let raw = "<p>Az &eacute;v v&eacute;g&eacute;n &ndash; &bdquo;Eredm&eacute;nyek&rdquo; &amp; d&iacute;jak&nbsp;&hellip; &Ouml;r&ouml;m, &#337;sz, &#x171;r</p>";
        let out = run(raw).expect("a body that settles");
        assert_eq!(
            out.html,
            "<p>Az év végén – „Eredmények” &amp; díjak&nbsp;… Öröm, ősz, űr</p>"
        );
        assert!(!out.report.has_hits(), "N0 and N8 are not rule hits");
        // The search text of ADR 0020 now matches the accented words.
        assert!(html_to_search_text(&out.html).contains("Eredmények"));
        assert_eq!(html_to_search_text(raw), html_to_search_text(&out.html));
    }

    #[test]
    fn bodies_are_html_never_markdown() {
        // POS-C11: bodies have been HTML since the 2020 Markdown conversion,
        // so Markdown syntax is text, kept as it is, and tags are tags.
        let raw = "<p># Nem c&iacute;m</p><p>**nem f&eacute;lk&ouml;v&eacute;r** _nem d&#337;lt_ [nem link](https://example.org/) `k&oacute;d`</p><p>* nem lista<br>1. nem sz&aacute;moz&aacute;s</p><p>&gt; nem id&eacute;zet</p>";
        assert_eq!(
            normalised(raw),
            "<p># Nem cím</p><p>**nem félkövér** _nem dőlt_ [nem link](https://example.org/) `kód`</p><p>* nem lista<br>1. nem számozás</p><p>&gt; nem idézet</p>"
        );
        assert_eq!(normalised("Szöveg *csillaggal*"), "Szöveg *csillaggal*");
    }

    #[test]
    fn legacy_upper_case_markup_is_parsed() {
        assert_eq!(
            normalised(
                "<P STYLE=\"text-align: justify\">Sz&ouml;veg<BR />\r\nm&aacute;sodik sor</P>\r\n"
            ),
            "<p style=\"text-align: justify\">Szöveg<br>\nmásodik sor</p>\n"
        );
    }

    #[test]
    fn the_rules_run_in_one_pass_and_report() {
        let raw = concat!(
            "<center>\n<p><iframe src=\"https://www.youtube.com/embed/AbCdEfGhIjK\"></iframe></p>\n</center>",
            "<center><h2>C&iacute;m</h2></center>",
            r#"<p><img src="https://backend.verseghy-gimnazium.net/storage/uploads/posts/12/a.jpg"></p>"#,
            r#"<p><a href="https://backend.verseghy-gimnazium.net/admin">x</a></p>"#,
            r#"<table border="1"><tr><td style="border-top: 1px solid black">y<!-- z --></td></tr></table>"#,
        );
        let out = run(raw).expect("a body that settles");
        // N7 gives the iframe, the link, the table and its `tbody` what
        // content-v1 always writes on them; adding is no removal, but it is
        // a planned change, counted per element. N5b has already given the
        // image its locked declarations.
        assert_eq!(
            out.html,
            concat!(
                "\n<p><iframe src=\"https://www.youtube.com/embed/AbCdEfGhIjK\" allow=\"accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share\" allowfullscreen=\"\" referrerpolicy=\"strict-origin-when-cross-origin\"></iframe></p>\n",
                r#"<h2 style="text-align: center">Cím</h2>"#,
                r#"<p><img src="/storage/uploads/posts/12/a.jpg" style="display: block; height: auto; margin: 0 auto; max-width: 100%"></p>"#,
                r#"<p><a href="https://backend.verseghy-gimnazium.net/admin" target="_blank" rel="noopener noreferrer">x</a></p>"#,
                r#"<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td style="border-bottom: none; border-left: none; border-right: none; border-top: 1px solid #000000">y</td></tr></tbody></table>"#,
            )
        );
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [
                (Rule::CenterUnwrapped, 2),
                (Rule::CenterBlockAligned, 1),
                (Rule::ImageShaped, 1),
                (Rule::DeadHostLinkRewritten, 1),
                (Rule::NamedColourToHex, 1),
                (Rule::LockedMarkupAdded, 4),
                (Rule::TableBorderDropped, 1),
                (Rule::CommentRemoved, 1),
            ]
        );
        assert!(!out.report.has_unplanned_removals());
        assert_eq!(out.report.wide_images(), []);
        assert_eq!(out.report.dead_host_kept().len(), 1);
        assert_eq!(
            out.report.images_without_alt(),
            [ImageWithoutAlt {
                position: 1,
                src: Some("/storage/uploads/posts/12/a.jpg".to_owned()),
                alt: AltState::Missing,
            }]
        );
    }

    #[test]
    fn what_n7_removes_unplanned_is_reported_and_the_body_is_still_written() {
        // The import decides what to do with these (`n7_review`); the
        // normaliser reports them and writes content-v1.
        let out = run(r#"<div class="doboz"><p style="font-family: Arial">Szöveg</p></div>"#)
            .expect("a body that settles");
        assert_eq!(out.html, "<p>Szöveg</p>");
        let unplanned: Vec<_> = out
            .report
            .unplanned_removals()
            .map(|(removal, count)| (removal.to_string(), count))
            .collect();
        assert_eq!(
            unplanned,
            [
                ("element_unwrapped div".to_owned(), 1),
                ("style_rejected p[style] font-family".to_owned(), 1),
            ]
        );
        assert!(!out.report.has_hits());
    }

    #[test]
    fn a_body_the_parser_moves_settles_in_its_first_pass() {
        // A link fostered out of a table into another link: the parse after
        // N6 nests one link in another, and N7's sanitiser settles it in
        // the same pass (its own passes absorb what the parser moves, ADR
        // 0013 §5); the next pass only confirms. Both links and the table
        // get their locked markup.
        let raw = r#"<a href="/1"><table><tr><td>x</td></tr><a href="/2">y</table></a>"#;
        let once = run(raw).expect("a body that settles");
        assert_eq!(
            once.html,
            concat!(
                r#"<a href="/1" target="_blank" rel="noopener noreferrer"></a><a href="/2" target="_blank" rel="noopener noreferrer">y</a>"#,
                r#"<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td>x</td></tr></tbody></table>"#,
            )
        );
        let first = pass(raw, &SANITIZER, &mut NormaliseReport::default())
            .expect("a pass over a small body")
            .0;
        assert_eq!(first, once.html, "the first pass settled it");
        assert_eq!(
            once.report.hits().collect::<Vec<_>>(),
            [(Rule::LockedMarkupAdded, 4)]
        );
        assert_eq!(
            run(&once.html),
            Ok(Normalised {
                html: once.html,
                report: NormaliseReport::default(),
            })
        );
        // Line feeds after `pre`: each parse drops the first one, and N7
        // unwraps the `pre`, which content-v1 does not have.
        assert_eq!(normalised("<pre>\n\nx</pre>"), "x");
        assert_eq!(normalised("<pre>\n\n\nx</pre>"), "\nx");
    }

    #[test]
    fn a_plaintext_element_settles_as_text_and_is_reported() {
        // The writer gives `plaintext` an end tag that no parser reads, so
        // N7's parse takes it as text; N7 unwraps the element, and the
        // unplanned removal puts the body in front of an editor.
        let out = run("<plaintext>x").expect("a body that settles");
        assert_eq!(out.html, "x&lt;/plaintext&gt;");
        assert_eq!(
            out.report
                .unplanned_removals()
                .map(|(removal, count)| (removal.to_string(), count))
                .collect::<Vec<_>>(),
            [("element_unwrapped plaintext".to_owned(), 1)]
        );
    }

    #[test]
    fn the_loop_stops_at_the_first_fixed_point_and_refuses_after_the_last_pass() {
        let observed = || Observations::default();
        // A pass that drops one leading `x` settles after the `x`s are gone.
        let mut calls = 0;
        let settled = settle("xxa", 3, |input| {
            calls += 1;
            Ok((
                input.strip_prefix('x').unwrap_or(input).to_owned(),
                observed(),
            ))
        });
        assert_eq!(settled, Ok(("a".to_owned(), observed())));
        assert_eq!(calls, 3, "two changes and one confirmation");
        // One more `x` and the third pass still changes it.
        let refused = settle("xxxa", 3, |input| {
            Ok((
                input.strip_prefix('x').unwrap_or(input).to_owned(),
                observed(),
            ))
        });
        assert_eq!(
            refused,
            Err(NormaliseError::NotNormalisable { passes: MAX_PASSES })
        );
        // A pass's error stops the loop.
        let deep = SanitizeError::TooDeep { limit: 1, line: 1 };
        assert_eq!(
            settle("a", 3, |_| Err(NormaliseError::SanitiseRefused(
                deep.clone()
            ))),
            Err(NormaliseError::SanitiseRefused(deep.clone()))
        );
        assert_eq!(
            NormaliseError::NotNormalisable { passes: 3 }.to_string(),
            "the body did not reach a fixed point in 3 normaliser passes"
        );
        assert_eq!(
            NormaliseError::SanitiseRefused(deep).to_string(),
            "the sanitiser refused the body: the content nests elements more than 1 levels deep (line 1)"
        );
        assert_eq!(
            NormaliseError::NotSanitiserFixedPoint {
                at: 4,
                report: RemovalReport::default()
            }
            .to_string(),
            "sanitising the normalised body again changes it from byte 4"
        );
    }

    #[test]
    fn the_sanitisers_limits_refuse_a_legacy_body() {
        let deep = format!("{}x", "<div>".repeat(300));
        let Err(NormaliseError::SanitiseRefused(error)) = run(&deep) else {
            panic!("a body deeper than the sanitiser's limit is refused");
        };
        assert_eq!(error.code(), Some("content_too_deep"));
    }

    #[test]
    fn the_rules_handle_every_level_of_a_deep_body() {
        // The deep shapes of `tests/normalise/cost.rs`, at the same depths.
        // N7 refuses each of them as too deep after the rules of the first
        // pass, and its refusal carries no report, so what the rules made of
        // every level is checked here, on the rules alone; the cost tests
        // time the whole normaliser.
        let rules = |raw: &str| {
            let mut html = dom::parse(raw);
            let mut report = NormaliseReport::default();
            apply_rules(&mut html, &mut report);
            (dom::serialise(&html), report)
        };
        let nested = |open: &str, close: &str, depth: usize| {
            format!("{}x{}", open.repeat(depth), close.repeat(depth))
        };

        // N3 unwraps every `center` around the deep inline chain.
        let depth = 4_000;
        let (out, report) = rules(&nested("<center><span>", "</span></center>", depth));
        assert!(!out.contains("<center"), "{out}");
        assert_eq!(report.count(Rule::CenterUnwrapped), depth);

        // N5 gives every nested italic heading its own `em`.
        let depth = 5_000;
        let (out, report) = rules(&nested(
            r#"<h2 style="font-style: italic"><div><object>"#,
            "</object></div></h2>",
            depth,
        ));
        assert_eq!(
            out,
            nested("<h2><em><div><object>", "</object></div></em></h2>", depth)
        );
        assert_eq!(report.count(Rule::ItalicHeadingToEm), depth);

        // N5 maps every level of the deep chain of legacy marks.
        let depth = 2_000;
        let (out, report) = rules(&nested(
            r#"<big><span dir="ltr"><b>"#,
            "</b></span></big>",
            depth,
        ));
        assert_eq!(out, nested("<strong>", "</strong>", depth));
        assert_eq!(report.count(Rule::BigUnwrapped), depth);
        assert_eq!(report.count(Rule::SpanDirUnwrapped), depth);
        assert_eq!(report.count(Rule::BToStrong), depth);

        // And each of them is deeper than N7's sanitiser accepts.
        for raw in [
            nested("<center><span>", "</span></center>", 4_000),
            nested(r#"<big><span dir="ltr"><b>"#, "</b></span></big>", 2_000),
        ] {
            let Err(NormaliseError::SanitiseRefused(error)) = run(&raw) else {
                panic!("a body deeper than the sanitiser's limit is refused");
            };
            assert_eq!(error.code(), Some("content_too_deep"));
        }
    }

    #[test]
    fn normalise_ends_with_n9_after_the_passes_settle() {
        // The real pass ends with the sanitiser, so it never settles on a
        // body that the sanitiser still changes; a pass that changes nothing
        // shows that the run checks N9 after the loop all the same.
        let unchanged =
            |input: &str, _: &mut NormaliseReport| Ok((input.to_owned(), Observations::default()));
        assert!(matches!(
            normalise_with(r#"<p class="x">a</p>"#, &SANITIZER, unchanged),
            Err(NormaliseError::NotSanitiserFixedPoint { at: 2, .. })
        ));
        assert_eq!(
            normalise_with("<p>a</p>", &SANITIZER, unchanged).map(|out| out.html),
            Ok("<p>a</p>".to_owned())
        );
    }

    #[test]
    fn n9_accepts_only_what_the_sanitiser_leaves_unchanged() {
        assert_eq!(check_sanitiser_fixed_point("<p>a</p>", &SANITIZER), Ok(()));
        let Err(NormaliseError::NotSanitiserFixedPoint { at, report }) =
            check_sanitiser_fixed_point(r#"<p class="x">a</p>"#, &SANITIZER)
        else {
            panic!("a body outside content-v1 fails N9");
        };
        assert_eq!(at, 2);
        assert!(!report.is_empty());
        assert!(matches!(
            check_sanitiser_fixed_point(&"<div>".repeat(300), &SANITIZER),
            Err(NormaliseError::SanitiseRefused(_))
        ));
    }

    #[test]
    fn a_link_left_open_around_sixteen_blocks_settles_since_n7() {
        // At the inner `<a>` the parser runs the adoption agency, whose
        // eight rounds move the open link out of one block each, so a copy
        // of it stays open around the inner link, eight blocks further in.
        // Each later parse moves it eight blocks more: with sixteen blocks
        // the parser alone still changes the body in its third parse, and
        // before N7 ran in the pass the normaliser refused it, a rare
        // refusal that the maintainer accepted as built (D159,
        // data-migration.md §7.4). N7's sanitiser repairs the nesting in the
        // first pass: its own passes (ADR 0013 §5) absorb what the adoption
        // agency moves, and a link still nested at its structural pass
        // would be unwrapped (ADR 0037) as an unplanned removal. Here none
        // is left to unwrap: the body settles with planned changes only,
        // every copy of the link that the parse made getting its locked
        // markup and its dead-host address rewritten, so it is imported
        // (D162). The first body is the one the properties over
        // `arb_plain_legacy` found (shrunk by proptest; its seeds are in
        // `proptest-regressions/domain/normalise/mod.txt`), the second the
        // shortest of its kind: eight lists of one item each.
        let found = concat!(
            r#"<a href="https://backend.verseghy-gimnazium.net/storage/uploads/a.pdf">"#,
            "<ol><li><CENTER><ol><li><h2><ol><li><ul><li><ol><li><ol><li><ul><li><p>",
            r#"<a href="https://backend.verseghy-gimnazium.net/storage/uploads/a.pdf">"#,
        );
        let shortest = format!(r#"<a href="/x">{}<a href="/x">"#, "<ul><li>".repeat(8));
        for body in [found, shortest.as_str()] {
            assert!(!the_parser_alone_settles(body), "{body:?}");
            assert!(nests_a_link(body), "{body:?}");
            let out = run(body).expect("N7 settles the nested links");
            assert!(
                !out.report.has_unplanned_removals(),
                "{body:?}: {:?}",
                out.report
            );
            assert!(
                out.report.count(Rule::LockedMarkupAdded) >= 2,
                "{:?}",
                out.report
            );
            assert!(!nests_a_link(&out.html), "{:?}", out.html);
            assert!(only_five_references(&out.html), "{:?}", out.html);
            assert_eq!(search_words(&out.html), search_words(body), "{body:?}");
            let again = run(&out.html).expect("the output settles");
            assert_eq!(again.html, out.html, "{body:?}");
            assert!(
                !again.report.has_hits() && !again.report.has_unplanned_removals(),
                "{:?}",
                again.report
            );
        }
        // Fewer blocks settled before N7 too: the first parse nests the
        // links from eight blocks on and the second undoes it, so seven
        // lists, fourteen blocks, settle in the third pass. The text and the
        // references stay (V5(c), V5(d)).
        let lists = |count: usize| {
            format!(
                r#"<a href="/x">Az &eacute;v{}v&eacute;ge &ndash; <a href="/x">k&eacute;p"#,
                "<ul><li>".repeat(count)
            )
        };
        for count in [3, 4, 7] {
            let shallower = lists(count);
            assert_eq!(nests_a_link(&shallower), count >= 4, "{shallower:?}");
            assert!(!nests_a_link(&once(&shallower)), "{shallower:?}");
            let settled = run(&shallower).expect("fewer than sixteen blocks settle");
            assert!(only_five_references(&settled.html), "{:?}", settled.html);
            assert_eq!(
                search_words(&settled.html),
                search_words(&shallower),
                "{shallower:?}"
            );
            assert_eq!(
                run(&settled.html).map(|again| again.html),
                Ok(settled.html),
                "{shallower:?}"
            );
        }
    }

    #[test]
    fn a_centre_between_headings_in_a_nested_link_settles_since_n7() {
        // Nine blocks between the links: the first parse nests them and
        // leaves a copy of the outer link around the inner heading, inside
        // the centre. N3 unwraps the centre, so that copy and the heading in
        // it now sit right inside the outer heading. Without N7, the next
        // parse took the inner heading out of that link and the one after
        // closed the outer heading before the inner one, so the third pass
        // still changed the body and it was refused, although the links no
        // longer nest after the first pass and the parser alone settles it
        // in three parses. The maintainer accepted that rare refusal as
        // built, over a new N3 rule (D159, data-migration.md §7.4). N7's
        // sanitiser, with its own passes in the first one, absorbs those
        // repairs, so the body settles, and with planned changes only, so
        // it is imported as the maintainer confirmed (D162): an unplanned
        // removal here would send it to review like a `pre`.
        let body = r#"<a href="/x"><ul><li><ul><li><ul><li><h2><center><h2><a href="/x">"#;
        assert!(nests_a_link(body));
        assert!(!nests_a_link(&once(body)));
        assert!(the_parser_alone_settles(body));
        let out = run(body).expect("N7 settles the headings");
        assert_eq!(out.report.count(Rule::CenterUnwrapped), 1);
        assert!(
            !out.report.has_unplanned_removals(),
            "only planned changes: {:?}",
            out.report
        );
        let again = run(&out.html).expect("the output settles");
        assert_eq!(again.html, out.html);
        assert!(
            !again.report.has_hits() && !again.report.has_unplanned_removals(),
            "{:?}",
            again.report
        );
        assert_eq!(search_words(&out.html), search_words(body));
    }

    #[test]
    fn centring_inside_containers_survives_the_sanitiser() {
        // A `div` and a list inside a `center` render centred, so their text
        // stays centred once N7 unwraps the `div`.
        let out = run("<center><div>Sz&ouml;veg</div><ul><li>pont</li></ul></center>")
            .expect("a body that settles");
        assert_eq!(
            out.html,
            r#"<p style="text-align: center">Szöveg</p><ul><li><p style="text-align: center">pont</p></li></ul>"#
        );
        // The `div` was legacy markup that no step plans to remove.
        assert_eq!(
            out.report
                .unplanned_removals()
                .map(|(removal, count)| (removal.to_string(), count))
                .collect::<Vec<_>>(),
            [("element_unwrapped div".to_owned(), 1)]
        );
    }

    #[test]
    fn a_content_v1_body_is_unchanged_and_passes_n9() {
        let body = concat!(
            r#"<h2 style="text-align: center">Eredmények</h2>"#,
            r#"<p>Szöveg <strong>félkövér</strong>, <span style="color: #c00000; font-size: 1.25em">piros és nagy</span>.</p>"#,
            r#"<ol start="5" type="a"><li>első</li><li><p>két</p><p>bekezdés</p></li></ol>"#,
            r#"<ul style="list-style-type: circle"><li>pont</li></ul>"#,
            r#"<blockquote>Egy bekezdéses idézet</blockquote>"#,
            r#"<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group"><tr><td colspan="2" style="background-color: #d9d9d9; border-bottom: 1px solid #000000; border-left: none; border-right: 1px solid #000000; border-top: 1px solid #000000; box-sizing: content-box; height: 20px; text-align: center; vertical-align: middle; white-space: nowrap; width: 128px"><strong>Eredmény</strong></td></tr></tbody></table>"#,
            r#"<p><img src="/storage/uploads/posts/612/kep.jpg" alt="A díjazottak a színpadon" width="350" height="495" style="display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px"></p>"#,
            r#"<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=30" width="640" height="360" title="Videó" style="aspect-ratio: 16 / 9; height: auto; max-width: 640px; min-height: 200px; width: 100%" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>"#,
            r#"<p><a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a></p>"#,
        );
        let out = run(body).expect("content-v1 settles");
        assert_eq!(out.html, body);
        assert_eq!(out.report, NormaliseReport::default());
        let sanitizer = Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy");
        assert_eq!(check_fixed_point(&out.html, &sanitizer), Ok(()));
    }

    #[test]
    fn an_empty_body_stays_empty() {
        assert_eq!(
            run(""),
            Ok(Normalised {
                html: String::new(),
                report: NormaliseReport::default(),
            })
        );
    }

    /// Legacy markup: the shapes of data-migration.md §7.1 and §12, the
    /// references `CKEditor` wrote, and tag soup that reaches the parser's
    /// repair rules (foster parenting, the adoption agency, raw text).
    #[rustfmt::skip]
    const PIECES: &[&str] = &[
        "<p>", "</p>", "<P STYLE=\"text-align: justify\">", "</P>", "<h2>", "</h2>", "<h5>",
        "<center>", "</center>", "<CENTER>", "<div>", "</div>", "<br />", "<br>", "\r\n", "\n", " ",
        "<strong>", "</strong>", "<b>", "</b>", "<em>", "<span style=\"color: red\">", "</span>",
        "<ul><li>", "</li>", "</ul>", "<blockquote>", "</blockquote>",
        "<table border=\"1\" cellspacing=\"0\"><tr><td>", "</td><td>", "</td></tr></table>",
        "<table>", "<tr>", "<td>", "</table>", "<caption>",
        "<a href=\"https://backend.verseghy-gimnazium.net/storage/uploads/a.pdf\">",
        "<a href=\"http://backend.verseghy-gimnazium.net/storage/b.pdf\">", "<a href=\"/x\">",
        "<a href=\"https://backend.verseghy-gimnazium.net/\">", "</a>",
        "<img src=\"https://backend.verseghy-gimnazium.net/storage/uploads/c.jpg\" alt=\"\" />",
        "<img src=\"/storage/uploads/d.jpg\">", "<img src=\"data:image/jpeg;base64,/9j/AA\" alt=\"K&eacute;p\">",
        "<iframe width=\"560\" height=\"315\" src=\"https://www.youtube.com/embed/AbCdEfGhIjK\" frameborder=\"0\" allowfullscreen></iframe>",
        "<div data-oembed-url=\"https://youtu.be/AbCdEfGhIjK\"><!-- Iframely -->", "<!-- c -->",
        "<script>x</script>", "<noscript><b>&amp;</b></noscript>", "<svg><p>", "</svg>",
        "&eacute;", "&Ouml;", "&#337;", "&ndash;", "&bdquo;", "&rdquo;", "&hellip;", "&nbsp;",
        "&amp;", "&lt;", "&gt;", "&quot;", "&#39;", "&", "\"", "<", ">", "sz&ouml;veg",
        "backend.verseghy-gimnazium.net", "**nem markdown**", "<pre>", "</pre>", "<form>", "</form>",
        "<font size=\"2\">", "</font>", "<xmp>", "</xmp>",
        "<ol><li>", "<dl><dd>", "<td align=\"left\">", "<p style=\"text-align: inherit\">",
    ];

    /// N5's block shapes, for [`arb_legacy`] only. [`arb_plain_legacy`]
    /// keeps the pieces above for its properties about text and references;
    /// the extra headings and paragraphs here would make the tag soup that
    /// the parser repairs a few levels per pass (a link left open around
    /// deeply nested lists and headings, with another link inside them)
    /// much more likely there. `blocks.rs` checks that its own shapes
    /// settle.
    #[rustfmt::skip]
    const BLOCK_PIECES: &[&str] = &[
        "<p align=\"center\">", "<P ALIGN=\"JUSTIFY\">", "<p lang=\"en\">", "<table align=\"left\"><tr><td>",
        "<h2 style=\"font-style:italic\">", "<h6>",
    ];

    fn arb_legacy() -> impl Strategy<Value = String> {
        let pieces: Vec<&str> = PIECES.iter().chain(BLOCK_PIECES).copied().collect();
        prop::collection::vec(prop::sample::select(pieces), 0..40)
            .prop_map(|pieces| pieces.concat())
    }

    /// Whether html5ever's parse of `raw` leaves a link inside another
    /// link, the nesting that the sanitiser's structural pass unwraps (ADR
    /// 0037). Without a table, as in [`arb_plain_legacy`], the parse builds
    /// it only through the adoption agency's limit: with eight or more
    /// blocks between a link start tag and a link still open, a copy of the
    /// open link stays around the new one.
    fn nests_a_link(raw: &str) -> bool {
        let html = dom::parse(raw);
        dom::elements_named(&html, "a").into_iter().any(|link| {
            dom::node(&html, link)
                .ancestors()
                .any(|ancestor| dom::html_name(ancestor) == Some("a"))
        })
    }

    /// The first pass's output, which the second pass parses.
    fn once(raw: &str) -> String {
        pass(raw, &SANITIZER, &mut NormaliseReport::default())
            .expect("a pass over a small body")
            .0
    }

    /// Whether html5ever alone, parsing and serialising with no rule in
    /// between, reaches a fixed point within [`MAX_PASSES`] parses: what
    /// [`normalise`] would do if no rule changed anything.
    fn the_parser_alone_settles(raw: &str) -> bool {
        let mut current = raw.to_owned();
        for _ in 0..MAX_PASSES {
            let next = dom::serialise(&dom::parse(&current));
            if next == current {
                return true;
            }
            current = next;
        }
        false
    }

    /// The words of `html`'s search text (ADR 0020), joined without the
    /// spaces, which the parse may move: what V5(c) compares.
    fn search_words(html: &str) -> String {
        html_to_search_text(html).split(' ').collect()
    }

    /// The legacy pieces without raw-text elements, whose content the
    /// writer keeps byte for byte, and without the tags whose parse moves
    /// text (tables foster it out, `pre` drops a line feed, `form` may be
    /// ignored), for the properties about text and references.
    ///
    /// Before N7 ran in the pass, this generator also left out every body
    /// whose parse nests a link in another link ([`nests_a_link`]): a link
    /// left open around eight or more nested blocks, with another link
    /// inside them, the one family among these pieces that the normaliser
    /// refused, a rare refusal that the maintainer accepted as built
    /// (D159). N7's sanitiser now repairs the nesting in the first pass
    /// (`a_link_left_open_around_sixteen_blocks_settles_since_n7`,
    /// `a_centre_between_headings_in_a_nested_link_settles_since_n7`; D162),
    /// so the properties below see those bodies too: they settle, keep their
    /// text and references, and any link the sanitiser unwraps is an
    /// unplanned removal for the editors' review. In a sample of 1,260,000
    /// bodies of these pieces, about 1 in 365 nested a link.
    fn arb_plain_legacy() -> impl Strategy<Value = String> {
        let pieces: Vec<&str> = PIECES
            .iter()
            .copied()
            // A bare `<` before a letter would start a tag whose name holds the
            // next piece's `&`, which is no reference.
            .filter(|piece| *piece != "<")
            .filter(|piece| {
                ![
                    "script", "noscript", "xmp", "pre", "table", "<tr", "<td", "caption", "form",
                    "svg", "iframe", "oembed", "<!--",
                ]
                .iter()
                .any(|tag| piece.contains(tag))
            })
            .collect();
        prop::collection::vec(prop::sample::select(pieces), 0..40)
            .prop_map(|pieces| pieces.concat())
    }

    /// Whether every `&` in `html` starts one of the five references N8
    /// writes.
    fn only_five_references(html: &str) -> bool {
        html.match_indices('&').all(|(at, _)| {
            ["&amp;", "&lt;", "&gt;", "&nbsp;", "&quot;"]
                .iter()
                .any(|reference| html[at..].starts_with(reference))
        })
    }

    #[test]
    fn the_soup_reaches_the_rules_and_is_rarely_refused() {
        // P4 below holds for the bodies the normaliser accepts, so the
        // generator must not hide behind refusals: on a fixed sample, the
        // rules and N7 fire, both planned and unplanned removals occur, and
        // almost every body settles. Before N7 ran in the pass, the few
        // refusals were links that tag soup nests in other links; N7's
        // sanitiser settles those, its own passes absorbing the parser's
        // repairs (see "The fixed-point loop" in the module documentation),
        // and the sample has no refusal left, but the bound stays loose so
        // that a generator change does not fail it.
        use proptest::strategy::ValueTree;
        let mut runner = proptest::test_runner::TestRunner::deterministic();
        let strategy = arb_legacy();
        let (mut refused, mut centred, mut mapped, mut rewritten) = (0, 0, 0, 0);
        let (mut planned, mut unplanned) = (0, 0);
        for _ in 0..2_000 {
            let raw = strategy
                .new_tree(&mut runner)
                .expect("the generator always produces a value")
                .current();
            match run(&raw) {
                Err(_) => refused += 1,
                Ok(out) => {
                    centred += out.report.count(Rule::CenterUnwrapped);
                    let step = |wanted| {
                        out.report
                            .hits()
                            .filter(|(rule, _)| rule.step() == wanted)
                            .map(|(_, count)| count)
                            .sum::<usize>()
                    };
                    mapped += step(Step::N5);
                    planned += step(Step::N7);
                    unplanned += usize::from(out.report.has_unplanned_removals());
                    rewritten += out.report.count(Rule::DeadHostLinkRewritten);
                }
            }
        }
        assert!(refused <= 10, "{refused} of 2000 refused");
        assert!(
            centred >= 500 && mapped >= 500 && rewritten >= 500 && planned >= 500,
            "{centred} {mapped} {rewritten} {planned}"
        );
        assert!(
            (200..1_800).contains(&unplanned),
            "{unplanned} of 2000 bodies with unplanned removals"
        );
    }

    /// At least 512 cases, as P1 runs (editor.md §13.6), or more when
    /// `PROPTEST_CASES` asks for more.
    fn config() -> ProptestConfig {
        let default = ProptestConfig::default();
        ProptestConfig {
            cases: default.cases.max(512),
            ..default
        }
    }

    proptest! {
        #![proptest_config(config())]

        // P4 (editor.md §13.6, data-migration.md §11.2): normalising the
        // output again returns it, with no rule hits, no unplanned removal
        // and the same observations.
        #[test]
        fn normalising_twice_equals_normalising_once(raw in arb_legacy()) {
            if let Ok(once) = run(&raw) {
                let twice = run(&once.html)
                    .map_err(|error| TestCaseError::fail(format!("{raw:?}: the output was refused: {error}")))?;
                prop_assert_eq!(&twice.html, &once.html, "{:?}", raw);
                prop_assert!(!twice.report.has_hits(), "{:?}: {:?}", raw, twice.report);
                prop_assert!(!twice.report.has_unplanned_removals(), "{:?}: {:?}", raw, twice.report);
                prop_assert_eq!(twice.report.dead_host_kept(), once.report.dead_host_kept());
                prop_assert_eq!(twice.report.images_without_alt(), once.report.images_without_alt());
                // N7 dropped every width over the bound, so none is left to list.
                prop_assert_eq!(twice.report.wide_images(), []);
            }
        }

        // P4 over arbitrary strings, which reach every tokenizer state.
        #[test]
        fn normalising_any_text_twice_equals_once(raw in any::<String>()) {
            if let Ok(once) = run(&raw) {
                let twice = run(&once.html).expect("an output settles");
                prop_assert_eq!(twice.html, once.html);
                prop_assert!(!twice.report.has_hits());
                prop_assert!(!twice.report.has_unplanned_removals());
            }
        }

        // Legacy markup settles: the refusal is for pathological bodies.
        // The one family among the plain pieces that was refused before N7
        // ran in the pass, a link the parse leaves inside another link (a
        // rare refusal the maintainer accepted as built, D159), settles now
        // (`a_link_left_open_around_sixteen_blocks_settles_since_n7`,
        // `a_centre_between_headings_in_a_nested_link_settles_since_n7`;
        // D162).
        #[test]
        fn legacy_markup_is_never_refused(raw in arb_plain_legacy()) {
            prop_assert!(run(&raw).is_ok(), "{:?}", raw);
        }

        // N8 and V5(d): no reference but the five survives, and N0 leaves
        // no carriage return.
        #[test]
        fn only_five_references_survive(raw in arb_plain_legacy()) {
            let out = normalised(&raw);
            prop_assert!(only_five_references(&out), "{:?} -> {:?}", raw, out);
            prop_assert!(!out.contains('\r'), "{:?}", out);
        }

        // V5(c): the text a reader sees is unchanged.
        #[test]
        fn the_search_text_is_unchanged(raw in arb_plain_legacy()) {
            let out = normalised(&raw);
            prop_assert_eq!(search_words(&out), search_words(&raw), "{:?}", out);
        }

        // N3 and N6 leave nothing for themselves: no `center`, no `src` or
        // `href` on the dead `/storage/`, whatever the input.
        #[test]
        fn no_centre_or_dead_storage_link_is_left(raw in arb_legacy()) {
            if let Ok(out) = run(&raw) {
                let html = dom::parse(&out.html);
                prop_assert!(dom::elements_named(&html, "center").is_empty(), "{:?}", out.html);
                for node in html.root_element().descendants() {
                    if let Some(element) = node.value().as_element() {
                        for name in ["src", "href"] {
                            prop_assert!(element.attr(name).and_then(dead_host::rewritten).is_none());
                        }
                    }
                }
            }
        }

        // N5's block rules leave nothing for themselves either, after the
        // parse repairs and the other rules: no `align` or `lang` on a
        // paragraph, no `align` on a table, and no `h5` or `h6`.
        // `blocks.rs` checks the italic headings over its own shapes.
        #[test]
        fn no_block_shape_is_left(raw in arb_legacy()) {
            if let Ok(out) = run(&raw) {
                let html = dom::parse(&out.html);
                for node in html.root_element().descendants() {
                    let (Some(name), Some(element)) = (dom::html_name(node), node.value().as_element()) else {
                        continue;
                    };
                    let left = match name {
                        "p" => element.attr("align").or(element.attr("lang")),
                        "table" => element.attr("align"),
                        "h5" | "h6" => Some(name),
                        _ => None,
                    };
                    prop_assert!(left.is_none(), "{:?}: {} in {:?}", raw, name, out.html);
                }
            }
        }

        // Two runs over the same body agree byte for byte (V12).
        #[test]
        fn the_normaliser_is_deterministic(raw in arb_legacy()) {
            prop_assert_eq!(run(&raw), run(&raw));
        }
    }
}
