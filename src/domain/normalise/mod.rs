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
//! - **N6** (`dead_host.rs`): `src` and `href` on the dead host's
//!   `/storage/` rewritten to root-relative `/storage/`;
//! - **N8** (`dom.rs`): html5ever's serialisation, which leaves only
//!   `&amp; &lt; &gt; &nbsp;` and, in attributes, `&quot;` as references.
//!
//! The steps that later slices add slot in where §7.1 orders them: N1 and
//! N2 (legacy embeds) between N0 and N3; N4 (inline `data:` images)
//! between N3 and N5, and N5b (the content-v1 layout) between N5 and N6;
//! N7 (the sanitiser with its planned removals) between N6 and N8.
//! Each step records what it changed for the
//! run report ([`report`]), and the pass also lists, without changing them,
//! the dead-host occurrences N6 leaves and the images without alt text
//! (editor.md §9).
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
//! of its inline rules; the dead `/storage/` prefixes), so the passes after
//! the first only re-parse. A body that is still changing after the last
//! pass is refused with [`NormaliseError::NotNormalisable`], and the import
//! treats that like a failed N9 check (data-migration.md §4.2): for example
//! one with a `plaintext` element, whose end tag the serialiser writes and
//! no parser reads, a `pre` whose content starts with three or more line
//! feeds (each parse drops one, and the third pass still drops one), and
//! tag soup that leaves a link inside another link. That is a link left
//! open around deeply nested lists and headings with another link inside
//! them: each parse's adoption agency moves a copy of the outer link at
//! most eight blocks further in, so with sixteen or more nested blocks
//! between the two links the third pass still changes the body (the unit
//! test `a_link_left_open_around_sixteen_blocks_is_refused`), and with
//! fewer, the headings around the copies it leaves can still be moving
//! then, as when N3 unwraps a centre between two headings, a body that the
//! parser alone settles in three parses
//! (`a_centre_between_headings_in_a_nested_link_costs_a_fourth_pass`). Or
//! it is a link fostered out of a table into another link. Once N7 runs
//! here, the sanitiser repairs such nesting (its structural pass unwraps a
//! nested link, ADR 0037), and the tests expect the body to settle then.
//! Whether `apply` goes on depends on what N7 removes: an unwrapped link,
//! like every removal that no N7 rule plans, still stops it unless the
//! resolution file accepts the row (data-migration.md §7.1, N7). None of
//! these is a shape `CKEditor` writes; the tests measure how rarely tag
//! soup meets one.
//!
//! **N9** ([`fixed_point`]) checks an output against both fixed points,
//! `normalise(out) == out` and `S(out) == out`.
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
//! (`marks.rs`, "Cost"). The normaliser has no limit of its own, which is
//! acceptable only because of its two inputs: the import's is the fixed
//! legacy dump, and backend3's own bodies, which `content_normalize` reads,
//! hold no `center` and are bounded by the sanitiser's size and depth
//! limits (editor.md §7.2; research: DECISIONS R11). The sanitiser applies
//! those limits to the import too, when N7 runs. Like everything under
//! `src/domain/`, the module is pure (ADR 0008).
//!
//! ```
//! use backend3::domain::normalise::{Rule, normalise};
//!
//! let out = normalise(r#"<center><h2>Eredm&eacute;nyek</h2></center><p><a href="https://backend.verseghy-gimnazium.net/storage/a.pdf">PDF</a></p>"#)
//!     .expect("a body that settles");
//! assert_eq!(
//!     out.html,
//!     r#"<h2 style="text-align: center">Eredmények</h2><p><a href="/storage/a.pdf">PDF</a></p>"#
//! );
//! assert_eq!(out.report.count(Rule::CenterUnwrapped), 1);
//! assert_eq!(out.report.count(Rule::DeadHostLinkRewritten), 1);
//! ```

mod blocks;
mod center;
mod dead_host;
mod dom;
pub mod fixed_point;
mod images;
mod marks;
pub mod report;

use std::borrow::Cow;

pub use fixed_point::{FixedPointError, check_fixed_point};
pub use report::{
    AltState, DeadHostOccurrence, ImageWithoutAlt, NormaliseReport, Place, Rule, Step,
};

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
    /// module take it.
    pub html: String,
    /// What the run changed and observed.
    pub report: NormaliseReport,
}

/// Why the normaliser refused a body. Nothing is written for a refused
/// body; the import's dry run lists it with the reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NormaliseError {
    /// The body was still changing after [`MAX_PASSES`] passes: html5ever's
    /// parse and its serialisation of the parsed tree disagree about it.
    #[error("the body did not reach a fixed point in {passes} normaliser passes")]
    NotNormalisable {
        /// How many passes ran.
        passes: usize,
    },
}

/// Normalises one body (see the module documentation).
///
/// - `raw`: the body as stored, legacy or content-v1. A NULL body is not
///   passed here: it stays NULL (data-migration.md §6.1, D4).
///
/// # Errors
///
/// [`NormaliseError::NotNormalisable`] when the body has not settled after
/// [`MAX_PASSES`] passes.
pub fn normalise(raw: &str) -> Result<Normalised, NormaliseError> {
    let mut report = NormaliseReport::default();
    let mut current = Cow::Borrowed(raw);
    for _ in 0..MAX_PASSES {
        let (output, observations) = pass(&current, &mut report);
        if output == *current {
            report.observe(observations);
            return Ok(Normalised {
                html: output,
                report,
            });
        }
        current = Cow::Owned(output);
    }
    Err(NormaliseError::NotNormalisable { passes: MAX_PASSES })
}

/// One pass: N0, the rules in their order, the observations, N8. The rules
/// add their hits to `report`; the observations are returned, and only the
/// last pass's are kept.
///
/// - `input`: the body, or the previous pass's output.
/// - `report`: the report of the current run.
fn pass(input: &str, report: &mut NormaliseReport) -> (String, Observations) {
    let mut html = dom::parse(input);
    center::unwrap_centres(&mut html, report);
    // After N3, which reads a paragraph's `align`, counts `h5` and `h6`
    // among the aligned headings and must centre an italic `h2` before N5
    // puts an `em` inside it (`blocks.rs`, "Order").
    blocks::map_blocks(&mut html, report);
    // N5's inline rules, after its block rules: an `i` that the
    // italic-heading rule unwraps is gone before the `i` rule runs, so it is
    // not also counted as `i_to_em` (`marks.rs`, "After N3 and N5's block
    // rules").
    marks::map_marks(&mut html, report);
    dead_host::rewrite_links(&mut html, report);
    let observations = Observations {
        dead_host_kept: dead_host::occurrences(&html),
        images_without_alt: images::without_alt(&html),
    };
    (dom::serialise(&html), observations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::search_text::html_to_search_text;
    use crate::domain::rich_text::{RemovalRule, SanitizePolicy, Sanitizer};
    use proptest::prelude::*;

    fn normalised(raw: &str) -> String {
        normalise(raw).expect("a body that settles").html
    }

    #[test]
    fn entity_encoded_bodies_become_literal_utf8() {
        // POS-C06: CKEditor's named references, decoded once.
        let raw = "<p>Az &eacute;v v&eacute;g&eacute;n &ndash; &bdquo;Eredm&eacute;nyek&rdquo; &amp; d&iacute;jak&nbsp;&hellip; &Ouml;r&ouml;m, &#337;sz, &#x171;r</p>";
        let out = normalise(raw).expect("a body that settles");
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
        );
        let out = normalise(raw).expect("a body that settles");
        assert_eq!(
            out.html,
            concat!(
                "\n<p><iframe src=\"https://www.youtube.com/embed/AbCdEfGhIjK\"></iframe></p>\n",
                r#"<h2 style="text-align: center">Cím</h2>"#,
                r#"<p><img src="/storage/uploads/posts/12/a.jpg"></p>"#,
                r#"<p><a href="https://backend.verseghy-gimnazium.net/admin">x</a></p>"#,
            )
        );
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [
                (Rule::CenterUnwrapped, 2),
                (Rule::CenterBlockAligned, 1),
                (Rule::DeadHostLinkRewritten, 1),
            ]
        );
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
    fn a_body_the_parser_moves_settles_in_a_later_pass() {
        // A link fostered out of a table into another link: the first
        // output nests one link in another, which the next parse undoes.
        let raw = r#"<a href="/1"><table><tr><td>x</td></tr><a href="/2">y</table></a>"#;
        let once = normalise(raw).expect("a body that settles");
        assert_eq!(
            once.html,
            r#"<a href="/1"></a><a href="/2">y</a><table><tbody><tr><td>x</td></tr></tbody></table>"#
        );
        assert_ne!(
            once.html,
            dom::serialise(&dom::parse(raw)),
            "a second pass ran"
        );
        assert_eq!(normalise(&once.html), Ok(once));
        // Two line feeds after `pre`: each parse drops the first one.
        assert_eq!(normalised("<pre>\n\nx</pre>"), "<pre>x</pre>");
    }

    #[test]
    fn a_body_that_never_settles_is_refused() {
        // A `pre` whose content starts with three line feeds is the
        // shortest such shape: two settle (above).
        for raw in ["<plaintext>x", "<pre>\n\n\nx</pre>", "<pre>\n\n\n\nx</pre>"] {
            assert_eq!(
                normalise(raw),
                Err(NormaliseError::NotNormalisable { passes: MAX_PASSES }),
                "{raw:?}"
            );
        }
        assert_eq!(
            NormaliseError::NotNormalisable { passes: 3 }.to_string(),
            "the body did not reach a fixed point in 3 normaliser passes"
        );
    }

    #[test]
    fn a_link_left_open_around_sixteen_blocks_is_refused() {
        // At the inner `<a>` the parser runs the adoption agency, whose
        // eight rounds move the open link out of one block each, so a copy
        // of it stays open around the inner link, eight blocks further in.
        // Each later parse moves it eight blocks more: with sixteen blocks
        // the third pass still changes the body, and it is refused (see
        // "The fixed-point loop" in the module documentation), with or
        // without the rules. The first body is the one the properties over
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
            assert_eq!(
                normalise(body),
                Err(NormaliseError::NotNormalisable { passes: MAX_PASSES }),
                "{body:?}"
            );
            assert!(!the_parser_alone_settles(body), "{body:?}");
            // The parse nests the links, so the generator leaves the body
            // out (`arb_plain_legacy`), and so does the next pass's parse.
            assert!(nests_a_link(body), "{body:?}");
            assert!(nests_a_link(&once(body)), "{body:?}");
            // The sanitiser repairs the nesting, here with its structural
            // pass's nested-link rule (ADR 0037), after which the body
            // settles: once N7 runs inside the pass, this refusal goes, and
            // this test and the generator's filter with it. Whether `apply`
            // then goes on depends on what N7 removes: an unwrapped link,
            // like every removal that no N7 rule plans, still stops it
            // unless the resolution file accepts the row
            // (data-migration.md §7.1, N7).
            let sanitised = Sanitizer::new(SanitizePolicy::content_v1())
                .expect("a valid policy")
                .sanitize(body)
                .expect("a body within the limits");
            assert_eq!(
                sanitised.report.count(RemovalRule::NestedLinkUnwrapped),
                1,
                "{body:?}"
            );
            assert!(normalise(sanitised.html.as_str()).is_ok(), "{body:?}");
        }
        // Fewer blocks settle, unless headings among them still move after
        // the nesting is gone (the next test): the first parse nests the
        // links from eight blocks on and the second undoes it, so seven
        // lists, fourteen blocks, still settle in the third pass. The
        // generator's filter leaves these bodies out as well, so this checks
        // for them what its properties check: the text and the references
        // (V5(c), V5(d)).
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
            let settled = normalise(&shallower).expect("fewer than sixteen blocks settle");
            assert!(only_five_references(&settled.html), "{:?}", settled.html);
            assert_eq!(
                search_words(&settled.html),
                search_words(&shallower),
                "{shallower:?}"
            );
            assert_eq!(
                normalise(&settled.html).map(|again| again.html),
                Ok(settled.html),
                "{shallower:?}"
            );
        }
    }

    #[test]
    fn a_centre_between_headings_in_a_nested_link_costs_a_fourth_pass() {
        // Nine blocks between the links: the first parse nests them and
        // leaves a copy of the outer link around the inner heading, inside
        // the centre. N3 unwraps the centre, so that copy and the heading in
        // it now sit right inside the outer heading. The next parse takes
        // the inner heading out of that link (the adoption agency again, at
        // the link inside the heading), and the one after closes the outer
        // heading before the inner one: the third pass still changes the
        // body, and it is refused, although the links no longer nest after
        // the first pass. Without N3 it settles: the parser alone settles it
        // in three parses, and so does `normalise` with a `div` in place of
        // the centre, which keeps the inner heading out of the outer one.
        // The filter of `arb_plain_legacy` leaves it out, because its first
        // parse nests the links; leaving out only the bodies whose links
        // stay nested after a pass would miss it.
        let body = r#"<a href="/x"><ul><li><ul><li><ul><li><h2><center><h2><a href="/x">"#;
        assert_eq!(
            normalise(body),
            Err(NormaliseError::NotNormalisable { passes: MAX_PASSES })
        );
        assert!(nests_a_link(body));
        assert!(!nests_a_link(&once(body)));
        assert!(the_parser_alone_settles(body));
        assert!(normalise(&body.replace("<center>", "<div>")).is_ok());
        // The sanitiser repairs this one too, here with no link to unwrap,
        // so it should settle once N7 runs inside the pass.
        let sanitised = Sanitizer::new(SanitizePolicy::content_v1())
            .expect("a valid policy")
            .sanitize(body)
            .expect("a body within the limits");
        assert_eq!(sanitised.report.count(RemovalRule::NestedLinkUnwrapped), 0);
        assert!(normalise(sanitised.html.as_str()).is_ok());
    }

    #[test]
    fn centring_inside_containers_survives_the_sanitiser() {
        // A `div` and a list inside a `center` render centred, so their text
        // stays centred once the sanitiser unwraps the `div` (N7, which runs
        // here by hand until its slice adds it to the pass).
        let sanitizer = Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy");
        let out = normalised("<center><div>Sz&ouml;veg</div><ul><li>pont</li></ul></center>");
        assert_eq!(
            out,
            r#"<div><p style="text-align: center">Szöveg</p></div><ul><li><p style="text-align: center">pont</p></li></ul>"#
        );
        let sanitised = sanitizer.sanitize(&out).expect("a body within the limits");
        assert_eq!(
            sanitised.html.as_str(),
            r#"<p style="text-align: center">Szöveg</p><ul><li><p style="text-align: center">pont</p></li></ul>"#
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
        let out = normalise(body).expect("content-v1 settles");
        assert_eq!(out.html, body);
        assert_eq!(out.report, NormaliseReport::default());
        let sanitizer = Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy");
        assert_eq!(check_fixed_point(&out.html, &sanitizer), Ok(()));
    }

    #[test]
    fn an_empty_body_stays_empty() {
        assert_eq!(
            normalise(""),
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
    /// keeps the pieces above: its properties expect no refusal, and the
    /// extra headings and paragraphs here would make more likely the one
    /// tag soup among them that the parser repairs a few levels per pass (a
    /// link left open around deeply nested lists and headings, with another
    /// link inside them), which is refused with or without N5 and which
    /// [`arb_plain_legacy`] leaves out (see there). `blocks.rs` checks that
    /// its own shapes settle.
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
        pass(raw, &mut NormaliseReport::default()).0
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
    /// It also leaves out every body whose parse nests a link in another
    /// link ([`nests_a_link`]): a link left open around eight or more
    /// nested blocks, with another link inside them. That is the one
    /// family among these pieces that the normaliser refuses (see "The
    /// fixed-point loop" in the module documentation), in two kinds, each
    /// pinned by a unit test: with sixteen or more blocks the nesting
    /// outlasts the passes (`a_link_left_open_around_sixteen_blocks_is_refused`),
    /// and with fewer, the headings around the copies of the link that the
    /// parse leaves can still be moving in the third pass after the nesting
    /// is gone, as when N3 unwraps a centre between two headings
    /// (`a_centre_between_headings_in_a_nested_link_costs_a_fourth_pass`).
    /// Such a legacy body stops the import's `apply` (data-migration.md
    /// §7.4, D140). Once N7 runs inside the pass, the sanitiser repairs
    /// both kinds, and those tests expect the body to settle then: they
    /// fail, and this filter goes with them. Whether `apply` then goes on depends on what N7
    /// removes: a nested link it unwraps (ADR 0037), like every removal
    /// that no N7 rule plans, still stops it unless an `N7-accept` entry
    /// covers the row (data-migration.md §7.1, N7).
    ///
    /// The filter takes the nested links that settle as well, because
    /// leaving out only the bodies whose links stay nested after a pass
    /// misses the second kind. So the properties below see no nested link:
    /// `a_link_left_open_around_sixteen_blocks_is_refused` checks the text
    /// and the references of nested links that settle, and [`arb_legacy`],
    /// the generator of P4 and the other rule properties, keeps every
    /// nested link. In a sample of 1,260,000 bodies of these pieces, 3,451
    /// nested a link (about 1 in 365), and the normaliser refused 12, all
    /// among them: 4 of the first kind and 8 of the second, 7 of which the
    /// parser alone settled. The other 3,439 settled with their text and
    /// references as these properties expect.
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
            .prop_filter("a link the parse nests in a link is left to N7", |raw| {
                !nests_a_link(raw)
            })
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
        // rules fire, later passes run, and almost every body settles. The
        // few refusals are links that tag soup nests in other links, fostered
        // out of tables or left open around deeply nested lists and headings
        // (see "Fixed point" in the module documentation).
        use proptest::strategy::ValueTree;
        let mut runner = proptest::test_runner::TestRunner::deterministic();
        let strategy = arb_legacy();
        let (mut refused, mut later, mut centred, mut mapped, mut rewritten) = (0, 0, 0, 0, 0);
        for _ in 0..2_000 {
            let raw = strategy
                .new_tree(&mut runner)
                .expect("the generator always produces a value")
                .current();
            match normalise(&raw) {
                Err(_) => refused += 1,
                Ok(out) => {
                    if pass(&raw, &mut NormaliseReport::default()).0 != out.html {
                        later += 1;
                    }
                    centred += out.report.count(Rule::CenterUnwrapped);
                    mapped += out
                        .report
                        .hits()
                        .filter(|(rule, _)| rule.step() == Step::N5)
                        .map(|(_, count)| count)
                        .sum::<usize>();
                    rewritten += out.report.count(Rule::DeadHostLinkRewritten);
                }
            }
        }
        assert!(refused <= 10, "{refused} of 2000 refused");
        assert!(later >= 20, "only {later} bodies needed a second pass");
        assert!(
            centred >= 500 && mapped >= 500 && rewritten >= 500,
            "{centred} {mapped} {rewritten}"
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
        // output again returns it, with no rule hits and the same
        // observations.
        #[test]
        fn normalising_twice_equals_normalising_once(raw in arb_legacy()) {
            if let Ok(once) = normalise(&raw) {
                let twice = normalise(&once.html)
                    .map_err(|error| TestCaseError::fail(format!("{raw:?}: the output was refused: {error}")))?;
                prop_assert_eq!(&twice.html, &once.html, "{:?}", raw);
                prop_assert!(!twice.report.has_hits(), "{:?}: {:?}", raw, twice.report);
                prop_assert_eq!(twice.report.dead_host_kept(), once.report.dead_host_kept());
                prop_assert_eq!(twice.report.images_without_alt(), once.report.images_without_alt());
            }
        }

        // P4 over arbitrary strings, which reach every tokenizer state.
        #[test]
        fn normalising_any_text_twice_equals_once(raw in any::<String>()) {
            if let Ok(once) = normalise(&raw) {
                let twice = normalise(&once.html).expect("an output settles");
                prop_assert_eq!(twice.html, once.html);
                prop_assert!(!twice.report.has_hits());
            }
        }

        // Legacy markup settles: the refusal is for pathological bodies.
        // The one refused family among the plain pieces, a link the parse
        // leaves inside another link, is left out by the generator, and its
        // two kinds are pinned by `a_link_left_open_around_sixteen_blocks_is_refused`
        // and `a_centre_between_headings_in_a_nested_link_costs_a_fourth_pass`.
        #[test]
        fn legacy_markup_is_never_refused(raw in arb_plain_legacy()) {
            prop_assert!(normalise(&raw).is_ok(), "{:?}", raw);
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
            if let Ok(out) = normalise(&raw) {
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
            if let Ok(out) = normalise(&raw) {
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
            prop_assert_eq!(normalise(&raw), normalise(&raw));
        }
    }
}
