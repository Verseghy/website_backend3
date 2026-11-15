//! N5, its inline rules: the legacy marks become content-v1's
//! (data-migration.md §7.1 and §7.4, editor.md §9, ADR 0017 §2 step 5,
//! ADR 0013).
//!
//! content-v1 writes text formatting with four mark elements, `strong`,
//! `em`, `u` and `s`, and colour, highlight and size on `span[style]`
//! (editor.md §3.2, §3.3). The sanitiser unwraps every other inline element
//! and keeps its text (editor.md §3.5), so without N5 the legacy elements
//! below would lose what a reader sees: the bold of a `b`, the italic of an
//! `i`, the underline of an `ins`, the line-through of a `strike` or `del`.
//! They come from markup typed in `CKEditor`'s source mode and from its
//! Styles combo, which writes `big`, `ins`, `del`, `strike` and
//! `<span class="marker">`
//! (`website_backend@0dead98:public/packages/ckeditor/styles.js`). The
//! `span` elements with a `dir` come from pastes out of Google Docs (no
//! style writes their `dir="auto"`), and the combo's two language styles
//! write `span[dir]` too (research: gap-06 §2.2, §4.1, §4.2). N5 maps them
//! as editor.md §9 lists them, and records one hit per element ([`Rule`]):
//!
//! - **Renamed**: `b` becomes `strong`, `i` becomes `em`, `ins` becomes
//!   `u`, and `strike` and `del` become `s`. The element keeps its place,
//!   its children and its attributes. content-v1 keeps no attribute on these
//!   four marks (editor.md §3.2), so N7 removes whatever the legacy element
//!   carried and lists it as a removal that no step planned
//!   (data-migration.md §7.1, N7), which stops `apply` until review accepts
//!   it. That includes a `style` that changed what the tag showed, such as
//!   `<b style="font-weight: normal">`, which a reader saw without bold:
//!   N5 does not read styles to guess what the reviewer of the dry run
//!   should decide. Google Docs wraps a whole paste in such a `b` (with an
//!   `id` of `docs-internal-guid-…`), which the editor's paste filter drops
//!   because the explicit weight overrides the tag (editor.md §5.2 step 9);
//!   if one is in the dump, its row reaches review this way rather than
//!   turning the passage bold unseen.
//! - **`big`** is unwrapped: its content takes its place, and its larger
//!   size is not kept (editor.md §9). Its attributes go with it, as they
//!   would in N7, which unwraps every element outside content-v1.
//! - **`span.marker`**, `CKEditor` 4's "Marker" style, becomes a highlight:
//!   the class `marker` is removed from the span's class list, which is
//!   split at ASCII white space and matched exactly, as a class selector
//!   matches in a document without quirks (`Marker` and `markers` are other
//!   classes), and the `class` attribute goes when nothing is left in it.
//!   Other classes stay, for N7. The span gets
//!   [`MARKER_BACKGROUND`], the yellow of `CKEditor`'s `.marker` rule, in
//!   front of its `style`, through [`dom::prepend_declaration`]. In front,
//!   because the class rule had the lowest precedence: a `background-color`
//!   or `background` in the span's own `style` showed instead of the
//!   yellow, and as the later declaration it wins in N7's cascade as well
//!   (editor.md §3.1 rule 5) when content-v1 keeps its value. A value that
//!   neither reads, such as `nope`, leaves the yellow in force in a browser
//!   and in N7 alike. A value that is valid CSS but that content-v1 does not
//!   keep (`none`, `inherit`, `initial`, a `url(…)`) cleared the yellow in a
//!   browser, but N7 drops it, so the yellow shows after the import; N7
//!   lists the dropped declaration as a removal that no step planned, so the
//!   row reaches review. The public sites have no rule for the class
//!   (research: backend1-media-editor-i18n §3.4), so the highlight that the
//!   old editor showed its authors now shows to readers too: a one-time
//!   visible change, in 1 body **\[500\]**.
//! - **`span[dir]`** is unwrapped, as editor.md §9 says, whatever the value
//!   of its `dir`: content-v1 never stores `dir` (editor.md §3.2), and the
//!   bodies hold only `ltr` and `auto` **\[500\]**, which show Hungarian
//!   text as it is. Every other attribute of the span goes with it. Without
//!   a `style`, or with a blank one ([`dom::is_blank_style`]), that loses
//!   nothing a reader saw, and the editor's serialiser unwraps such a bare
//!   span too (editor.md §6, step 2.2): [`Rule::SpanDirUnwrapped`]. A
//!   `style` that is not blank may have shown a colour, a highlight or a
//!   size that content-v1 keeps on a span, so such a span counts as
//!   [`Rule::SpanDirStyleDropped`], and the report lists the body for
//!   review. The **\[500\]** corpus has no span with a `style` (research:
//!   gap-06 §4.2). Keeping such a span would need more than a style that is
//!   not blank: a span whose style keeps no declaration in content-v1
//!   (`font-family: Arial`, or a value N7 cannot read) comes out of N7 as a
//!   bare `<span>`, which the editor's serialiser unwraps, so N9's editor
//!   round trip would fail on it.
//! - **A marker span with a `dir`** is both. The marker rule runs first, and
//!   its output is a span with a highlight, so the span stays and only its
//!   `dir` goes ([`Rule::SpanDirRemoved`]): `<span class="marker"
//!   dir="ltr">` keeps its highlight.
//!
//! Only elements in the HTML namespace are mapped: an `ins` or `del` inside
//! `svg` is an SVG element, which N7 removes with the `svg`. (`b`, `i`,
//! `big`, `strike` and `span` start tags end the `svg` in the parser, so
//! they are HTML wherever they stand.)
//!
//! **What N5 does not touch.** Other inline elements outside content-v1,
//! such as the rest of the Styles combo's (`small`, `tt`, `code`, `kbd`,
//! `samp`, `var`, `cite` and `q`), or `mark` and `font`, are not in
//! editor.md §9's list: N7 unwraps them and lists each as an unplanned
//! removal. The **\[500\]** corpus has none of them, and none of `b`, `i`,
//! `strike` or `del` either (research: gap-06 §4.1). N5 also maps each
//! element on its own, never the structure of the marks around it: it does
//! not merge a mark into the same mark around it (`<strong><b>` becomes
//! `<strong><strong>`), order nested marks, join adjacent equal ones or
//! drop empty ones. The editor does all of these on load, to legacy
//! `strong` and `em` as much as to the renamed marks, so N5b does them, as
//! the maintainer decided (D147): data-migration.md §7.1's N5b row (4) and
//! editor.md §9 give the four rules. N5b runs them after its image and embed
//! moves, which can leave a mark empty. Until N5b has them, the editor round
//! trip of N9 (data-migration.md §10 V13, editor.md §13.4) finds any body
//! where they would change the bytes.
//!
//! **After N3 and N5's block rules.** N5 runs after N3 in each pass
//! (data-migration.md §7.1), and its inline rules run right after its block
//! rules (`blocks.rs`). N3 sees a `big` or a `span[dir]` around a block as
//! an inline element around a block, which keeps its place without the
//! centring (counted as `center_alignment_dropped`); N5 then unwraps it in
//! the same pass. So `<center><big><h2>x</h2></big></center>` becomes
//! `<h2>x</h2>`, without the `text-align: center` that a browser showed.
//! The drop is in the report, never silent.
//!
//! The italic-heading rule of the block rules unwraps an `em` or `i`
//! without attributes inside the `em` it gives an italic `h2`, so such an
//! `i` is gone before the inline rules run and is not counted as
//! [`Rule::IToEm`]; the heading counts as `italic_heading_to_em`. An `i`
//! that the rule keeps, one with an attribute or one that holds a block,
//! becomes `em` here like any other. The order also decides where the `em`
//! starts when the heading's first child is a `span[dir]` that holds only
//! white space: the rule starts the `em` at that span, which the inline
//! rules then unwrap, so the `em` begins with its blank text
//! (`<h2><em> a</em></h2>`, where the other order would write
//! `<h2> <em>a</em></h2>`). N5b's whitespace rule (editor.md §3.8) strips
//! that blank in either case.
//!
//! **Unwraps that the next parse reads differently.** An unwrap can leave
//! markup that parses into another tree, as N7's unwraps can (editor.md
//! §3.5); the passes of the normaliser absorb that or refuse the body.
//! Two shapes come from N5:
//!
//! - Line feeds at the start of an unwrapped `big` or `span[dir]` that was
//!   the first child of a `pre` or `listing` become the first text of that
//!   element. The parser drops one line feed there at each parse, and
//!   html5ever's serialiser does not write one back. N7's parse in the same
//!   pass drops the first one, and its sanitiser unwraps the `pre` or
//!   `listing`, which content-v1 does not have, so the other line feeds
//!   stay as text and the body settles; the unwrapped element is an
//!   unplanned removal that an editor reviews. Before N7 ran in the pass, a
//!   body with two or more such line feeds did not settle and was refused
//!   (data-migration.md §7.4).
//! - A heading in an unwrapped `big` or `span[dir]` inside another heading
//!   ends the outer heading when it is parsed again, which leaves that one
//!   empty: `<h2><span dir="ltr"><h3>x</h3></span></h2>` becomes
//!   `<h2></h2><h3>x</h3>`.
//!
//! The text is kept in both. The renamed marks stay elements, so they move
//! nothing.
//!
//! **Cost.** One walk over the elements in document order, each looked at
//! once; an unwrap moves the element's children. The work is linear in the
//! size of the body, and no walk recurses.

use ego_tree::NodeId;
use html5ever::{LocalName, local_name};
use scraper::Html;

use super::dom;
use super::report::{NormaliseReport, Rule};

/// The class of `CKEditor` 4's "Marker" style, which it puts on a `span`.
const MARKER_CLASS: &str = "marker";

/// The declaration a marker span gets: the `.marker` rule of `CKEditor` 4's
/// content stylesheet sets `background-color: Yellow`
/// (`website_backend@0dead98:public/packages/ckeditor/contents.css`), and
/// content-v1 spells that colour `#ffff00` (editor.md §3.1 rule 3, §9).
const MARKER_BACKGROUND: &str = "background-color: #ffff00";

/// What N5 does with an HTML element, decided by its tag name alone.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mapping {
    /// The element is renamed to `to`, and the hit counted under `rule`.
    Rename {
        /// The content-v1 mark it becomes.
        to: LocalName,
        /// The rule that counts it.
        rule: Rule,
    },
    /// The element is replaced by its children (`big`).
    Unwrap(Rule),
    /// A `span`: the marker and `dir` rules decide.
    Span,
}

/// The mapping of the HTML element named `name`, or `None` when N5 leaves
/// it alone.
///
/// - `name`: a tag name as the parser reports it, lower-cased.
fn mapping(name: &str) -> Option<Mapping> {
    let rename = |to: LocalName, rule| Some(Mapping::Rename { to, rule });
    match name {
        "b" => rename(local_name!("strong"), Rule::BToStrong),
        "i" => rename(local_name!("em"), Rule::IToEm),
        "ins" => rename(local_name!("u"), Rule::InsToU),
        "strike" => rename(local_name!("s"), Rule::StrikeToS),
        "del" => rename(local_name!("s"), Rule::DelToS),
        "big" => Some(Mapping::Unwrap(Rule::BigUnwrapped)),
        "span" => Some(Mapping::Span),
        _ => None,
    }
}

/// Applies N5's inline rules (see the module documentation) to `html` and
/// counts each mapped element in `report`.
///
/// - `html`: a parsed body, after N3 and N5's block rules.
/// - `report`: the report of the current run.
pub(super) fn map_marks(html: &mut Html, report: &mut NormaliseReport) {
    // The decisions depend on the tag names alone, which no edit below
    // changes for another element, so they are taken first. An unwrapped
    // element's children keep their ids, and are still visited.
    let mapped: Vec<(NodeId, Mapping)> = html
        .root_element()
        .descendants()
        .filter_map(|node| Some((node.id(), mapping(dom::html_name(node)?)?)))
        .collect();
    for (id, mapping) in mapped {
        match mapping {
            Mapping::Rename { to, rule } => {
                dom::edit_element(html, id, |element| dom::rename(element, to));
                report.record(rule);
            }
            Mapping::Unwrap(rule) => {
                dom::unwrap(html, id);
                report.record(rule);
            }
            Mapping::Span => map_span(html, id, report),
        }
    }
}

/// Applies the marker rule, then the `dir` rule, to the `span` `id`.
///
/// - `html`: a parsed body.
/// - `id`: an HTML `span` of it.
/// - `report`: the report of the current run.
fn map_span(html: &mut Html, id: NodeId, report: &mut NormaliseReport) {
    let unwrap = dom::edit_element(html, id, |element| {
        let marker = element
            .attr("class")
            .and_then(|class| without_class(class, MARKER_CLASS));
        let is_marker = marker.is_some();
        if let Some(rest) = marker {
            if rest.is_empty() {
                dom::remove_attribute(element, "class");
            } else {
                dom::set_attribute(element, "class", &rest);
            }
            dom::prepend_declaration(element, MARKER_BACKGROUND);
            report.record(Rule::MarkerHighlighted);
        }
        if dom::remove_attribute(element, "dir").is_none() {
            return false;
        }
        // Decided before the unwrap, while the span still holds its own
        // `style`; a marker's `style` now leads with the highlight.
        let rule = dir_rule(is_marker, element.attr("style"));
        report.record(rule);
        rule != Rule::SpanDirRemoved
    })
    .unwrap_or(false);
    if unwrap {
        dom::unwrap(html, id);
    }
}

/// The rule that counts a `span` that had a `dir`: a marker span stays
/// ([`Rule::SpanDirRemoved`]); any other is unwrapped, as editor.md §9
/// says, and counts as [`Rule::SpanDirStyleDropped`] when its `style` is
/// not blank, because that style goes with it, or as
/// [`Rule::SpanDirUnwrapped`] otherwise. The span is unwrapped exactly when
/// the rule is not [`Rule::SpanDirRemoved`].
///
/// - `marker`: whether the span held the marker class, which the marker
///   rule has just turned into a highlight.
/// - `style`: the span's own `style` attribute value, if it has one.
fn dir_rule(marker: bool, style: Option<&str>) -> Rule {
    if marker {
        Rule::SpanDirRemoved
    } else if style.is_some_and(|style| !dom::is_blank_style(style)) {
        Rule::SpanDirStyleDropped
    } else {
        Rule::SpanDirUnwrapped
    }
}

/// The class list `classes` without the class `class`: `None` when the list
/// does not hold it; otherwise the other classes, in their order, joined by
/// one space, which is empty when none is left. The list is split at ASCII
/// white space and each class compared exactly, as HTML's class attribute
/// and a selector in a document without quirks read it.
///
/// - `classes`: a `class` attribute value.
/// - `class`: the class to remove, without white space.
fn without_class(classes: &str, class: &str) -> Option<String> {
    let all = classes.split_ascii_whitespace();
    if !all.clone().any(|token| token == class) {
        return None;
    }
    Some(
        all.filter(|token| *token != class)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{Rule, normalise};
    use super::*;
    use crate::domain::rich_text::{SanitizePolicy, Sanitizer};
    use proptest::prelude::*;
    use scraper::Node;
    use std::sync::LazyLock;

    /// The content-v1 sanitiser, built once for the tests of this module:
    /// the whole normaliser runs it as N7, and the proptests normalise
    /// thousands of bodies.
    static SANITIZER: LazyLock<Sanitizer> = LazyLock::new(sanitizer);

    /// N5's inline rules alone over `body`: the output and the report.
    fn n5(body: &str) -> (String, NormaliseReport) {
        let mut html = dom::parse(body);
        let mut report = NormaliseReport::default();
        map_marks(&mut html, &mut report);
        (dom::serialise(&html), report)
    }

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy")
    }

    /// The text of every text node of `html`, in document order: what a
    /// reader reads, without the markup.
    fn text(html: &Html) -> String {
        html.root_element()
            .descendants()
            .filter_map(|node| match node.value() {
                Node::Text(text) => Some(&**text),
                _ => None,
            })
            .collect()
    }

    /// How many HTML elements of `html` satisfy `test`.
    fn count(html: &Html, test: impl Fn(&str, &scraper::node::Element) -> bool) -> usize {
        html.root_element()
            .descendants()
            .filter(|node| {
                dom::html_name(*node)
                    .zip(node.value().as_element())
                    .is_some_and(|(name, element)| test(name, element))
            })
            .count()
    }

    /// Whether the HTML `span` `element` holds the marker class.
    fn is_marker(name: &str, element: &scraper::node::Element) -> bool {
        name == "span"
            && element
                .attr("class")
                .is_some_and(|class| class.split_ascii_whitespace().any(|c| c == MARKER_CLASS))
    }

    #[test]
    fn b_and_i_become_strong_and_em_with_their_children_and_attributes() {
        let (out, report) = n5(r#"<p><b>a <i>b</i></b> <i class="x" title="t">c</i></p>"#);
        assert_eq!(
            out,
            r#"<p><strong>a <em>b</em></strong> <em class="x" title="t">c</em></p>"#
        );
        assert_eq!(report.count(Rule::BToStrong), 1);
        assert_eq!(report.count(Rule::IToEm), 2);
        assert_eq!(report.hits().count(), 2, "{report:?}");
    }

    #[test]
    fn ins_strike_and_del_become_u_and_s() {
        let (out, report) = n5(
            r#"<p><ins>aláhúzott</ins> <strike>áthúzott</strike> <del datetime="2020-01-01">törölt</del></p>"#,
        );
        assert_eq!(
            out,
            r#"<p><u>aláhúzott</u> <s>áthúzott</s> <s datetime="2020-01-01">törölt</s></p>"#
        );
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::InsToU, 1), (Rule::StrikeToS, 1), (Rule::DelToS, 1)]
        );
    }

    #[test]
    fn a_style_that_resets_the_tag_is_left_to_n7_and_review() {
        // The reader saw no bold here. N5 renames the element anyway, and N7
        // lists the style it removes from `strong` as an unplanned removal.
        let (out, _) = n5(r#"<p><b style="font-weight: normal">x</b></p>"#);
        assert_eq!(
            out,
            r#"<p><strong style="font-weight: normal">x</strong></p>"#
        );
        let sanitised = sanitizer().sanitize(&out).expect("within the limits");
        assert_eq!(sanitised.html.as_str(), "<p><strong>x</strong></p>");
        assert!(!sanitised.report.is_empty(), "N7 reports the removal");
    }

    #[test]
    fn big_is_unwrapped_with_its_attributes() {
        let (out, report) =
            n5(r#"<p><big>Nagy <big class="x">nagyobb</big></big> <BIG>szöveg</BIG></p>"#);
        assert_eq!(out, "<p>Nagy nagyobb szöveg</p>");
        assert_eq!(report.hits().collect::<Vec<_>>(), [(Rule::BigUnwrapped, 3)]);
        // Around a block, too: the block takes its place.
        assert_eq!(n5("<big><h2>Cím</h2></big>").0, "<h2>Cím</h2>");
    }

    #[test]
    fn a_marker_span_becomes_a_yellow_highlight() {
        let (out, report) = n5(r#"<p>A <span class="marker">kiemelt</span> rész</p>"#);
        assert_eq!(
            out,
            r#"<p>A <span style="background-color: #ffff00">kiemelt</span> rész</p>"#
        );
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::MarkerHighlighted, 1)]
        );
        let sanitised = sanitizer().sanitize(&out).expect("within the limits");
        assert_eq!(sanitised.html.as_str(), out, "content-v1 as it is");
    }

    #[test]
    fn a_marker_keeps_the_other_classes_and_the_style_for_n7() {
        let (out, _) = n5(r#"<span class=" other  marker x " style="color: #c00000">a</span>"#);
        assert_eq!(
            out,
            r#"<span style="background-color: #ffff00; color: #c00000" class="other x">a</span>"#
        );
        // N7 keeps the highlight and the colour, and drops the classes.
        let sanitised = sanitizer().sanitize(&out).expect("within the limits");
        assert_eq!(
            sanitised.html.as_str(),
            r#"<span style="background-color: #ffff00; color: #c00000">a</span>"#
        );
    }

    #[test]
    fn a_background_of_the_span_s_own_wins_over_the_marker() {
        let sanitizer = sanitizer();
        let n7 = |body: &str| {
            let (out, _) = n5(body);
            sanitizer
                .sanitize(&out)
                .expect("within the limits")
                .html
                .as_str()
                .to_owned()
        };
        // An inline style wins over a class rule in a browser, and the later
        // declaration wins in N7's cascade.
        assert_eq!(
            n7(r#"<span class="marker" style="background-color: #00ff00">a</span>"#),
            r#"<span style="background-color: #00ff00">a</span>"#
        );
        assert_eq!(
            n7(r#"<span class="marker" style="background: #00ff00">a</span>"#),
            r#"<span style="background-color: #00ff00">a</span>"#
        );
        // A value neither reads leaves the class's yellow in force, in both.
        assert_eq!(
            n7(r#"<span class="marker" style="background-color: nope">a</span>"#),
            r#"<span style="background-color: #ffff00">a</span>"#
        );
    }

    #[test]
    fn a_background_that_content_v1_rejects_leaves_the_yellow_for_review() {
        // Valid CSS, so in a browser each of these cleared the class's
        // yellow. content-v1 keeps none of them: N7 drops the declaration,
        // the yellow shows after the import, and N7 lists the dropped
        // declaration as a removal that no step planned, so the row reaches
        // review instead of changing unseen.
        let sanitizer = sanitizer();
        for style in [
            "background: none",
            "background-color: inherit",
            "background-color: initial",
            "background: url(x)",
        ] {
            let (out, _) = n5(&format!(r#"<span class="marker" style="{style}">a</span>"#));
            let sanitised = sanitizer.sanitize(&out).expect("within the limits");
            assert_eq!(
                sanitised.html.as_str(),
                r#"<span style="background-color: #ffff00">a</span>"#,
                "{style:?}"
            );
            assert_eq!(
                sanitised
                    .report
                    .count(crate::domain::rich_text::RemovalRule::StyleDeclarationRejected),
                1,
                "{style:?}"
            );
        }
    }

    #[test]
    fn only_the_exact_marker_class_on_a_span_is_a_marker() {
        for body in [
            r#"<span class="Marker">a</span>"#,
            r#"<span class="markers">a</span>"#,
            r#"<span class="high-marker">a</span>"#,
            r#"<span class="">a</span>"#,
            r#"<p class="marker">a</p>"#,
            r#"<strong class="marker">a</strong>"#,
        ] {
            let (out, report) = n5(body);
            assert_eq!(out, body, "{body:?}");
            assert!(!report.has_hits(), "{body:?}");
        }
        // Any ASCII white space separates classes.
        assert_eq!(
            n5("<span class=\"a\tmarker\nb\">x</span>").0,
            r#"<span style="background-color: #ffff00" class="a b">x</span>"#
        );
    }

    #[test]
    fn a_span_with_dir_and_no_style_is_unwrapped() {
        let (out, report) = n5(concat!(
            r#"<p><span dir="ltr">Szöveg</span> <span dir="auto">másik</span> "#,
            r#"<span dir="rtl" lang="en" title="t" class="c">harmadik</span> "#,
            r#"<span dir="">negyedik</span> <span dir="ltr" style=" ; ">ötödik</span></p>"#,
        ));
        assert_eq!(out, "<p>Szöveg másik harmadik negyedik ötödik</p>");
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::SpanDirUnwrapped, 5)]
        );
    }

    #[test]
    fn a_span_with_dir_and_a_style_is_unwrapped_and_counted_for_review() {
        // editor.md §9 unwraps every `span[dir]`; the style goes with it,
        // which the report shows under a rule of its own.
        let (out, report) = n5(r#"<p><span dir="ltr" style="color: #c00000">piros</span></p>"#);
        assert_eq!(out, "<p>piros</p>");
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::SpanDirStyleDropped, 1)]
        );
        // So does a style that content-v1 keeps nothing of. Kept, it would
        // leave a bare `<span>` after N7, which the editor's serialiser
        // unwraps (editor.md §6 step 2.2): N9's round trip would fail.
        let sanitizer = sanitizer();
        for style in ["font-family: Arial", "color: nope", "/* */", "\u{a0}"] {
            let raw = format!(r#"<p><span dir="ltr" style="{style}">x</span></p>"#);
            let out = normalise(&raw, &SANITIZER).expect("a body that settles");
            assert_eq!(out.html, "<p>x</p>", "{style:?}");
            assert_eq!(
                out.report.hits().collect::<Vec<_>>(),
                [(Rule::SpanDirStyleDropped, 1)],
                "{style:?}"
            );
            assert_eq!(
                super::super::check_fixed_point(&out.html, &sanitizer),
                Ok(()),
                "{style:?}"
            );
        }
    }

    #[test]
    fn the_marker_rule_runs_before_the_dir_rule() {
        let (out, report) = n5(r#"<span dir="ltr" class="marker">a</span>"#);
        assert_eq!(out, r#"<span style="background-color: #ffff00">a</span>"#);
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::MarkerHighlighted, 1), (Rule::SpanDirRemoved, 1)]
        );
        // A marker keeps its own style beside the highlight, for N7.
        let (out, report) = n5(r#"<span dir="rtl" class="marker" style="color: #c00000">a</span>"#);
        assert_eq!(
            out,
            r#"<span style="background-color: #ffff00; color: #c00000">a</span>"#
        );
        assert_eq!(report.count(Rule::SpanDirRemoved), 1);
    }

    #[test]
    fn the_dir_rule_depends_on_the_marker_and_the_style_alone() {
        for style in [None, Some(""), Some(" ; \t"), Some("color: #c00000")] {
            assert_eq!(dir_rule(true, style), Rule::SpanDirRemoved, "{style:?}");
        }
        for style in [None, Some(""), Some(";"), Some(" \n; ")] {
            assert_eq!(dir_rule(false, style), Rule::SpanDirUnwrapped, "{style:?}");
        }
        for style in ["color: #c00000", "font-family: Arial", "x", "/* */"] {
            assert_eq!(
                dir_rule(false, Some(style)),
                Rule::SpanDirStyleDropped,
                "{style:?}"
            );
        }
    }

    #[test]
    fn nested_spans_are_each_handled() {
        let (out, report) = n5(concat!(
            r#"<span dir="ltr"><span class="marker"><span dir="auto">a</span></span>"#,
            r#"<span dir="ltr" style="color: #c00000">b</span>"#,
            r#"<span class="marker" dir="ltr">c</span></span>"#,
        ));
        assert_eq!(
            out,
            r#"<span style="background-color: #ffff00">a</span>b<span style="background-color: #ffff00">c</span>"#
        );
        assert_eq!(report.count(Rule::SpanDirUnwrapped), 2);
        assert_eq!(report.count(Rule::SpanDirStyleDropped), 1);
        assert_eq!(report.count(Rule::SpanDirRemoved), 1);
        assert_eq!(report.count(Rule::MarkerHighlighted), 2);
    }

    #[test]
    fn content_v1_marks_and_spans_are_left_alone() {
        let body = r#"<p><strong>a</strong> <em>b</em> <u>c</u> <s>d</s> <sub>e</sub> <span style="color: #c00000">f</span> <span>g</span></p>"#;
        let (out, report) = n5(body);
        assert_eq!(out, body);
        assert!(!report.has_hits(), "{report:?}");
    }

    #[test]
    fn foreign_elements_are_not_mapped() {
        // `ins` and `del` do not end an `svg`, so these are SVG elements.
        let body = r"<svg><ins>a</ins><del>b</del></svg>";
        let (out, report) = n5(body);
        assert_eq!(out, body);
        assert!(!report.has_hits());
        // `b` and `big` do end it, so they are HTML and mapped.
        let (out, report) = n5("<svg><b>a</b><big>b</big></svg>");
        assert_eq!(out, "<svg></svg><strong>a</strong>b");
        assert_eq!(report.count(Rule::BToStrong), 1);
        assert_eq!(report.count(Rule::BigUnwrapped), 1);
    }

    #[test]
    fn the_normaliser_runs_n5_after_n3_and_settles() {
        let raw = concat!(
            r#"<CENTER><B>Fontos</B></CENTER>"#,
            r#"<p><span class="marker">s&aacute;rga</span> <big>nagy</big> <span dir="ltr">sz&ouml;veg</span></p>"#,
        );
        let out = normalise(raw, &SANITIZER).expect("a body that settles");
        assert_eq!(
            out.html,
            concat!(
                r#"<p style="text-align: center"><strong>Fontos</strong></p>"#,
                r#"<p><span style="background-color: #ffff00">sárga</span> nagy szöveg</p>"#,
            )
        );
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [
                (Rule::CenterUnwrapped, 1),
                (Rule::CenterRunWrapped, 1),
                (Rule::BToStrong, 1),
                (Rule::BigUnwrapped, 1),
                (Rule::MarkerHighlighted, 1),
                (Rule::SpanDirUnwrapped, 1),
            ]
        );
        assert_eq!(
            normalise(&out.html, &SANITIZER).map(|again| again.html),
            Ok(out.html.clone())
        );
        assert_eq!(
            super::super::check_fixed_point(&out.html, &sanitizer()),
            Ok(())
        );
    }

    #[test]
    fn n3_drops_the_centring_around_a_block_in_a_mark_that_n5_unwraps() {
        // N3 runs first (data-migration.md §7.1) and sees an inline element
        // around a block, which keeps its place without the centring; N5
        // unwraps it afterwards in the same pass. Pinned, so that a change
        // of the step order fails here: with N5 first, the block would be
        // centred.
        for (raw, block, rule) in [
            (
                "<center><big><h2>x</h2></big></center>",
                "<h2>x</h2>",
                Rule::BigUnwrapped,
            ),
            (
                r#"<center><span dir="ltr"><p>x</p></span></center>"#,
                "<p>x</p>",
                Rule::SpanDirUnwrapped,
            ),
        ] {
            let out = normalise(raw, &SANITIZER).expect("a body that settles");
            assert_eq!(out.html, block, "{raw:?}");
            assert_eq!(
                out.report.hits().collect::<Vec<_>>(),
                [
                    (Rule::CenterUnwrapped, 1),
                    (Rule::CenterAlignmentDropped, 1),
                    (rule, 1),
                ],
                "{raw:?}"
            );
        }
    }

    #[test]
    fn the_inline_rules_run_after_the_block_rules() {
        // N5's block rules run first in the pass: the italic-heading rule
        // unwraps the bare `i` inside its new `em` before the `i` rule sees
        // it, so only the `i` with an attribute, which it keeps, counts as
        // `i_to_em`. Pinned, so that a change of the order fails here. N7
        // then removes the kept `class` as nothing planned (D146).
        let raw = r#"<h2 style="font-style: italic"><i>a</i> <i class="x">b</i></h2>"#;
        let out = normalise(raw, &SANITIZER).expect("a body that settles");
        assert_eq!(out.html, "<h2><em>a <em>b</em></em></h2>");
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [(Rule::ItalicHeadingToEm, 1), (Rule::IToEm, 1)]
        );
        assert_eq!(
            out.report
                .unplanned_removals()
                .map(|(removal, count)| (removal.to_string(), count))
                .collect::<Vec<_>>(),
            [("attribute_not_allowed em[class]".to_owned(), 1)]
        );
    }

    #[test]
    fn a_leading_blank_span_dir_ends_up_inside_the_italic_headings_em() {
        // The other shape whose output depends on that order. The
        // italic-heading rule starts its `em` at the first child that is not
        // white space, which is still the `span` when the block rules run
        // first; the `span[dir]` rule then unwraps it inside the `em`, so the
        // `em` starts with the span's blank text. The inline rules first
        // would leave the blank text outside: `<h2> <em>a</em></h2>`. Both
        // are fixed points, and N5b's whitespace rule (editor.md §3.8),
        // which strips white space at the start of a textblock, turns
        // either into `<h2><em>a</em></h2>`. Pinned, so that a change of
        // the order fails here too.
        let raw = r#"<h2 style="font-style: italic"><span dir="ltr"> </span>a</h2>"#;
        let out = normalise(raw, &SANITIZER).expect("a body that settles");
        assert_eq!(out.html, "<h2><em> a</em></h2>");
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [(Rule::ItalicHeadingToEm, 1), (Rule::SpanDirUnwrapped, 1)]
        );
        let again = normalise(&out.html, &SANITIZER).expect("the output settles");
        assert_eq!(again.html, out.html);
        assert!(!again.report.has_hits(), "{:?}", again.report);
    }

    #[test]
    fn an_unwrap_can_leave_markup_that_the_next_parse_reads_differently() {
        let normalised = |raw: &str| normalise(raw, &SANITIZER).map(|out| out.html);
        // Line feeds that an unwrap puts first in a `pre` or `listing`: the
        // parser drops one there at each parse, and html5ever's serialiser
        // does not write it back. N7's parse in the same pass drops the
        // first one, and its sanitiser unwraps the `pre` or `listing`,
        // which content-v1 does not have, so the rest stay as text and the
        // body settles. Before N7 ran in the pass, two or more kept the
        // body from settling; the unwrapped element is an unplanned
        // removal, which puts the body in front of an editor.
        for (raw, out, unwrapped) in [
            ("<pre><big>\nx</big></pre>", "x", "pre"),
            ("<pre><span dir=\"ltr\">\nx</span></pre>", "x", "pre"),
            ("<pre><big>\n\nx</big></pre>", "\nx", "pre"),
            (
                "<pre><span dir=\"ltr\">\n\n\nx</span></pre>",
                "\n\nx",
                "pre",
            ),
            ("<listing><big>\n\nx</big></listing>", "\nx", "listing"),
        ] {
            let normalised = normalise(raw, &SANITIZER).expect("a body that settles");
            assert_eq!(normalised.html, out, "{raw:?}");
            assert_eq!(
                normalised
                    .report
                    .unplanned_removals()
                    .map(|(removal, count)| (removal.to_string(), count))
                    .collect::<Vec<_>>(),
                [(format!("element_unwrapped {unwrapped}"), 1)],
                "{raw:?}"
            );
            assert_eq!(
                normalise(&normalised.html, &SANITIZER).map(|again| again.html),
                Ok(normalised.html.clone()),
                "{raw:?}"
            );
        }
        // Not first in the `pre`, the line feeds stay.
        assert_eq!(
            normalised("<pre>a<big>\n\nx</big></pre>"),
            Ok("a\n\nx".to_owned())
        );
        // A renamed mark stays an element, so it moves nothing.
        assert_eq!(
            normalised("<pre><b>\n\n\nx</b></pre>"),
            Ok("<strong>\n\n\nx</strong>".to_owned())
        );
        // A heading in an unwrapped mark inside another heading ends the
        // outer one when parsed again, which is left empty.
        for raw in [
            r#"<h2><span dir="ltr"><h3>x</h3></span></h2>"#,
            "<h2><big><h3>x</h3></big></h2>",
        ] {
            assert_eq!(
                normalised(raw),
                Ok("<h2></h2><h3>x</h3>".to_owned()),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn every_mapped_name_has_its_mapping() {
        let rename = |to: &str, rule| {
            Some(Mapping::Rename {
                to: LocalName::from(to),
                rule,
            })
        };
        assert_eq!(mapping("b"), rename("strong", Rule::BToStrong));
        assert_eq!(mapping("i"), rename("em", Rule::IToEm));
        assert_eq!(mapping("ins"), rename("u", Rule::InsToU));
        assert_eq!(mapping("strike"), rename("s", Rule::StrikeToS));
        assert_eq!(mapping("del"), rename("s", Rule::DelToS));
        assert_eq!(mapping("big"), Some(Mapping::Unwrap(Rule::BigUnwrapped)));
        assert_eq!(mapping("span"), Some(Mapping::Span));
        for name in [
            "strong", "em", "u", "s", "small", "mark", "font", "B", "", "bdo", "q",
        ] {
            assert_eq!(mapping(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_class_is_removed_from_its_list() {
        assert_eq!(without_class("marker", "marker"), Some(String::new()));
        assert_eq!(
            without_class(" marker  marker ", "marker"),
            Some(String::new())
        );
        assert_eq!(
            without_class("a marker\tb\nmarker", "marker"),
            Some("a b".to_owned())
        );
        assert_eq!(without_class("Marker markers", "marker"), None);
        assert_eq!(without_class("", "marker"), None);
        // U+00A0 is no separator: `marker\u{a0}` is one other class.
        assert_eq!(without_class("marker\u{a0}", "marker"), None);
    }

    /// Legacy markup around the marks: the elements N5 maps, in the shapes
    /// and spellings of data-migration.md §7.1 and the Styles combo, the
    /// content-v1 marks next to them, and the blocks, links, centres and
    /// tables they stand in.
    #[rustfmt::skip]
    const PIECES: &[&str] = &[
        "<b>", "</b>", "<B>", "<i>", "</i>", "<I STYLE=\"color: red\">", "<ins>", "</ins>",
        "<strike>", "</strike>", "<del datetime=\"2020-01-01\">", "</del>", "<big>", "</big>", "<BIG>",
        "<span class=\"marker\">", "<span class=\"Marker\">", "<span class=\"marker other\">",
        "<span class=\" marker \" style=\"color: #c00000\">", "<span dir=\"ltr\">",
        "<span dir=\"auto\" style=\"color: #c00000\">", "<span dir=\"rtl\" class=\"marker\">",
        "<span dir=\"ltr\" style=\" ; \">", "<span lang=\"en\" dir=\"ltr\">", "</span>",
        "<span style=\"color: #c00000\">", "<strong>", "</strong>", "<em>", "</em>", "<u>", "</u>",
        "<s>", "</s>", "<p>", "</p>", "<h2>", "</h2>", "<center>", "</center>", "<ul><li>", "</li>",
        "</ul>", "<blockquote>", "</blockquote>", "<div>", "</div>", "<a href=\"/x\">", "</a>",
        "<br>", "<table><tr><td>", "</td><td>", "</td></tr></table>", "<svg>", "</svg>", "<svg><del>",
        "sz&ouml;veg", "&eacute;", "&nbsp;", "&amp;", " ", "\r\n", "x",
    ];

    fn arb_legacy() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(PIECES), 0..40)
            .prop_map(|pieces| pieces.concat())
    }

    /// The pieces without those whose parse moves text when the output is
    /// parsed again (a table fosters it out) or keeps it in another
    /// namespace, for the property about the text a reader sees.
    fn arb_plain_legacy() -> impl Strategy<Value = String> {
        let pieces: Vec<&str> = PIECES
            .iter()
            .copied()
            .filter(|piece| {
                !["table", "<td", "svg"]
                    .iter()
                    .any(|tag| piece.contains(tag))
            })
            .collect();
        prop::collection::vec(prop::sample::select(pieces), 0..40)
            .prop_map(|pieces| pieces.concat())
    }

    /// Bodies of content-v1 marks and spans only, which N5 must not touch.
    fn arb_content_v1() -> impl Strategy<Value = String> {
        let pieces = prop::sample::select(
            &[
                "<strong>",
                "</strong>",
                "<em>",
                "</em>",
                "<u>",
                "</u>",
                "<s>",
                "</s>",
                "<sub>",
                "</sub>",
                "<span style=\"color: #c00000\">",
                "<span>",
                "</span>",
                "<p>",
                "</p>",
                "<a href=\"/x\">",
                "</a>",
                "a",
                " ",
                "é",
            ][..],
        );
        prop::collection::vec(pieces, 0..30).prop_map(|pieces| pieces.concat())
    }

    #[test]
    fn the_generated_markup_reaches_every_rule() {
        // The properties below hold for the bodies the normaliser accepts,
        // so the generator must reach each rule and must not hide behind
        // refusals.
        use proptest::strategy::ValueTree;
        let mut runner = proptest::test_runner::TestRunner::deterministic();
        let strategy = arb_legacy();
        let mut refused = 0;
        let mut totals = NormaliseReport::default();
        for _ in 0..1_000 {
            let raw = strategy
                .new_tree(&mut runner)
                .expect("the generator always produces a value")
                .current();
            match normalise(&raw, &SANITIZER) {
                Err(_) => refused += 1,
                Ok(out) => {
                    for (rule, hits) in out.report.hits() {
                        for _ in 0..hits {
                            totals.record(rule);
                        }
                    }
                }
            }
        }
        assert!(refused <= 5, "{refused} of 1000 refused");
        for rule in INLINE_RULES {
            assert_eq!(rule.step(), super::super::Step::N5, "{rule:?}");
            assert!(totals.count(rule) >= 50, "{rule:?}: {}", totals.count(rule));
        }
    }

    /// The rules this module records. N5's other rules are its block rules
    /// (`blocks.rs`), which the pieces above do not aim at.
    const INLINE_RULES: [Rule; 10] = [
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

    #[test]
    fn the_inline_rules_record_only_their_own_rules() {
        // Over the pieces above, `map_marks` records no rule outside
        // `INLINE_RULES`, so the list is the whole of what it counts.
        use proptest::strategy::ValueTree;
        let mut runner = proptest::test_runner::TestRunner::deterministic();
        let strategy = arb_legacy();
        for _ in 0..200 {
            let raw = strategy
                .new_tree(&mut runner)
                .expect("the generator always produces a value")
                .current();
            let (_, report) = n5(&raw);
            for (rule, _) in report.hits() {
                assert!(INLINE_RULES.contains(&rule), "{raw:?}: {rule:?}");
            }
        }
    }

    /// At least 512 cases, as the normaliser's other properties run, or
    /// more when `PROPTEST_CASES` asks for more.
    fn config() -> ProptestConfig {
        let default = ProptestConfig::default();
        ProptestConfig {
            cases: default.cases.max(512),
            ..default
        }
    }

    proptest! {
        #![proptest_config(config())]

        // P4 (editor.md §13.6, data-migration.md §11.2) over legacy marks:
        // normalising the output again returns it, with no rule hit and the
        // same observations.
        #[test]
        fn normalising_twice_equals_normalising_once(raw in arb_legacy()) {
            if let Ok(once) = normalise(&raw, &SANITIZER) {
                let twice = normalise(&once.html, &SANITIZER).map_err(|error| {
                    TestCaseError::fail(format!("{raw:?}: the output was refused: {error}"))
                })?;
                prop_assert_eq!(&twice.html, &once.html, "{:?}", raw);
                prop_assert!(!twice.report.has_hits(), "{:?}: {:?}", raw, twice.report);
                prop_assert_eq!(twice.report.images_without_alt(), once.report.images_without_alt());
                prop_assert_eq!(twice.report.dead_host_kept(), once.report.dead_host_kept());
            }
        }

        // N5 leaves nothing for itself: no legacy mark, no `span` with a
        // `dir` or the marker class, whatever the input.
        #[test]
        fn no_legacy_mark_is_left(raw in arb_legacy()) {
            if let Ok(out) = normalise(&raw, &SANITIZER) {
                let html = dom::parse(&out.html);
                let left = count(&html, |name, element| {
                    matches!(name, "b" | "i" | "ins" | "strike" | "del" | "big")
                        || (name == "span" && element.attr("dir").is_some())
                        || is_marker(name, element)
                });
                prop_assert_eq!(left, 0, "{:?}", out.html);
            }
        }

        // N5 changes markup only: on the tree, every text node keeps its
        // content and its place in document order, tables included.
        #[test]
        fn the_rules_keep_every_text_node(raw in arb_legacy()) {
            let mut html = dom::parse(&raw);
            let before = text(&html);
            map_marks(&mut html, &mut NormaliseReport::default());
            prop_assert_eq!(text(&html), before);
        }

        // V5(c) through the whole normaliser: text(source) == text(target).
        #[test]
        fn the_text_a_reader_reads_is_unchanged(raw in arb_plain_legacy()) {
            let out = normalise(&raw, &SANITIZER).map_err(|error| {
                TestCaseError::fail(format!("{raw:?} was refused: {error}"))
            })?;
            prop_assert_eq!(text(&dom::parse(&out.html)), text(&dom::parse(&raw)), "{:?}", out.html);
        }

        // Every mapped element is counted once, under its own rule.
        #[test]
        fn each_element_is_counted_once(raw in arb_legacy()) {
            let mut html = dom::parse(&raw);
            let named = |name: &str| count(&html, |n, _| n == name);
            let expected = [
                (Rule::BToStrong, named("b")),
                (Rule::IToEm, named("i")),
                (Rule::InsToU, named("ins")),
                (Rule::StrikeToS, named("strike")),
                (Rule::DelToS, named("del")),
                (Rule::BigUnwrapped, named("big")),
                (Rule::MarkerHighlighted, count(&html, is_marker)),
            ];
            // Each `span[dir]` under exactly one of the three `dir` rules:
            // a marker stays, a span with a style that is not blank loses
            // it, and any other is unwrapped.
            let with_dir = |name: &str, element: &scraper::node::Element| {
                name == "span" && element.attr("dir").is_some()
            };
            let styled = |element: &scraper::node::Element| {
                element.attr("style").is_some_and(|style| !dom::is_blank_style(style))
            };
            let dir_counts = [
                (Rule::SpanDirRemoved, count(&html, |n, e| with_dir(n, e) && is_marker(n, e))),
                (
                    Rule::SpanDirStyleDropped,
                    count(&html, |n, e| with_dir(n, e) && !is_marker(n, e) && styled(e)),
                ),
                (
                    Rule::SpanDirUnwrapped,
                    count(&html, |n, e| with_dir(n, e) && !is_marker(n, e) && !styled(e)),
                ),
            ];
            let mut report = NormaliseReport::default();
            map_marks(&mut html, &mut report);
            for (rule, elements) in expected.into_iter().chain(dir_counts) {
                prop_assert_eq!(report.count(rule), elements, "{:?}", rule);
            }
        }

        // Content-v1 marks and spans are no legacy markup.
        #[test]
        fn content_v1_marks_are_never_touched(body in arb_content_v1()) {
            let mut html = dom::parse(&body);
            let before = dom::serialise(&html);
            let mut report = NormaliseReport::default();
            map_marks(&mut html, &mut report);
            prop_assert_eq!(dom::serialise(&html), before);
            prop_assert!(!report.has_hits());
        }

        // Removing a class leaves exactly the other classes, in order.
        #[test]
        fn removing_a_class_keeps_the_others(
            classes in prop::collection::vec(prop::sample::select(&["marker", "Marker", "a", "b-c", "markers"][..]), 0..6),
            separators in prop::collection::vec(prop::sample::select(&[" ", "  ", "\t", "\n", "\u{c}", "\r"][..]), 6),
        ) {
            let mut list = String::new();
            for (class, separator) in classes.iter().zip(&separators) {
                list.push_str(separator);
                list.push_str(class);
            }
            let others: Vec<&str> = classes.iter().copied().filter(|c| *c != MARKER_CLASS).collect();
            let expected = classes.contains(&MARKER_CLASS).then(|| others.join(" "));
            prop_assert_eq!(without_class(&list, MARKER_CLASS), expected);
        }

        // No other tag name is mapped (the parser lower-cases them).
        #[test]
        fn other_names_are_never_mapped(name in "[a-z][a-z0-9-]{0,8}") {
            let mapped = ["b", "i", "ins", "strike", "del", "big", "span"].contains(&name.as_str());
            prop_assert_eq!(mapping(&name).is_some(), mapped);
        }
    }
}
