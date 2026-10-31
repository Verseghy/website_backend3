//! The structural pass of the sanitiser (editor.md §7.5, ADR 0013 §5 as
//! amended by ADR 0037).
//!
//! ammonia's attribute filter sees one element, attribute and value at a
//! time and never the ancestors, and ammonia keeps an element whose URL
//! attribute it dropped. [`restructure`] therefore runs a tree-level pass
//! after every ammonia pass, inside the same fixed-point loop
//! (`Sanitizer::sanitize`). It parses ammonia's output again, exactly as
//! ammonia parses its input (html5ever 0.40.1, a fragment in a `div`
//! context), and writes the result with html5ever's own serializer, so the
//! dialect of editor.md §3.8 does not change. It:
//!
//! - removes every child node of an `iframe`: html5ever writes iframe text
//!   raw, which is inert to an HTML parser but live markup for an XML or
//!   feed consumer, a regex rewriter or another sanitiser;
//! - removes an `img` without `src`, which ammonia keeps when the filter
//!   drops the `src`, and which the Angular site's image rewrite cannot
//!   handle;
//! - unwraps a `sub` or `sup` inside another `sub` or `sup`, because each
//!   level shrinks the text again;
//! - unwraps an `a` inside another `a` in the same table cell, or outside
//!   every cell: that nesting never parses back as it is written, because
//!   the inner `a` start tag runs the adoption agency, which moves content.
//!   With eight or more blocks between the two links the adoption agency
//!   stops before it is done and leaves a copy of the outer link open around
//!   the inner one, so even markup that closes every element can parse into
//!   the nesting (see "How the output settles" below). A link inside a cell
//!   of a table that a link holds parses back as it is written, and is
//!   kept. ADR 0037 adds this rule to the list of ADR 0013 §5;
//! - writes every element's attributes in the canonical order of editor.md
//!   §3.2, whatever order ammonia's `swap_remove` and its hash-ordered
//!   forced attributes left (§7.4), so two sanitiser instances write the
//!   same bytes.
//!
//! Two rules of editor.md §7.5 are style rules and are not here yet: the
//! one that drops `font-size` from a `span` inside a sized `span`, and the
//! one that gives an `img` without a `style` its locked declarations. Until
//! they are, a nested span keeps its `font-size`, and an `img` without a
//! `style` keeps none: the CSS grammar ([`super::css`]) sees one `style` at
//! a time, and ammonia calls it only for attributes that exist.
//!
//! It also asserts the invariants the public sites' sink chain relies on: no
//! `table` inside a `p`, and every element and attribute is in content-v1,
//! which leaves the childless `iframe` as the only raw-text element. They
//! hold by construction (ammonia admits nothing else, and html5ever in
//! no-quirks mode closes a `p` before a `table`); a [`Violation`] is a bug,
//! and the body is refused rather than written.
//!
//! The walk uses an explicit stack, like ammonia's, so a deeply nested body
//! cannot overflow the call stack.
//!
//! # How the output settles
//!
//! The pass writes the tree that html5ever built, and html5ever's markup for
//! a tree does not always parse back into that tree. Two repair steps of the
//! tree builder put a node where the rules of its start tag never ran
//! against its final ancestors: foster parenting, which moves content out of
//! a `table` to the front of it, and the adoption agency algorithm, which
//! moves the content of a misnested formatting element. The shape that kept
//! a body changing is a link inside another link, which a parse builds in
//! two ways:
//!
//! - A link fostered out of a table into another link: the outer link is
//!   out of scope behind the table, so the adoption agency gives up there,
//!   and the new link lands inside the old one. The next parse meets that
//!   inner `a` start tag while the outer link is open, and the adoption
//!   agency moves the block between them up beside the outer link, where a
//!   heading can land directly inside a heading; the parse after that
//!   closes the outer heading. Without the unwrapping of nested links the
//!   pass needed three rewrites for such input (`link-fostered-into-link`
//!   in `tests/fixtures/regressions/`).
//! - A link start tag with eight or more blocks between it and the open
//!   link. The adoption agency stops after eight rounds of its outer loop
//!   (WHATWG HTML, "adoption agency algorithm"; html5ever 0.40.1
//!   `for _ in 0..8`), and each round moves one block, so the last copy of
//!   the outer link stays open around the new one. This needs no table, and
//!   markup that closes every element builds it too. Without the rule each
//!   parse repaired eight more levels, and the number of rewrites grew with
//!   the depth (`nested-link-deeper-than-eight-blocks`, which the sanitiser
//!   refused before this rule).
//!
//! With the rule, two applications reach a fixed point:
//! `restructure(restructure(restructure(x)))` equals
//! `restructure(restructure(x))`, and the third one removes nothing. A
//! sketch of why:
//!
//! 1. The first application parses whatever it is given. Its tree can hold
//!    a heading directly inside a heading and an `li` inside an `li` (from
//!    foster parenting or the adoption agency), and nested links. The walk
//!    unwraps nested links, `sub`s and `sup`s, which can only make a heading
//!    the direct child of another: `a`, `sub` and `sup` are neither special
//!    elements nor scope boundaries, so removing one changes what a later
//!    start tag's rule finds only for the heading rule, which looks at the
//!    parent alone.
//! 2. The second application parses markup that a tree wrote. Every element
//!    is closed in order, and a `table`, `tbody` or `tr` holds nothing but
//!    its sections, rows and cells (and whitespace), so nothing is fostered.
//!    No `a` start tag finds a link open since the last cell began, so the
//!    adoption agency runs only for the end tag of a formatting element.
//!    That element is usually the current node. It need not be once end
//!    tags and elements of the same name have come apart, through the
//!    closing start tags below or a formatting element that the parser
//!    reopened after one; then only elements that are not special are open
//!    above it, because every special element written inside it is closed
//!    before its end tag. Either way the algorithm finds no furthest block,
//!    pops the element and those above it, and never moves a node. What
//!    remains are start tags that close open elements (a heading closes a
//!    heading that is the current node, an `li` an open `li`) and the end
//!    tags left over after them, which close an outer element of the same
//!    kind or are ignored. Closing only makes the following nodes children
//!    of an ancestor, inserted at the current node, where each start tag's
//!    rule has run against exactly the ancestors the node keeps; a
//!    formatting element that was closed is reopened the same way. So that
//!    tree parses back into itself, and it holds no nested link, `sub` or
//!    `sup` for the walk to unwrap.
//! 3. The third application therefore returns its input.
//!
//! This is a sketch over the rules that content-v1's elements reach, not a
//! proof over every rule of the tree builder. The proptests check it over
//! tag soup and over nesting deeper than the adoption agency's eight rounds,
//! and the sanitiser's fixed-point loop still refuses an input that keeps
//! changing. Inside the sanitiser a pass parses twice, in ammonia and here;
//! how many passes that takes is in `Sanitizer::sanitize`'s module.

use std::io;

use html5ever::serialize::{HtmlSerializer, SerializeOpts, Serializer};
use html5ever::{QualName, ns};
use scraper::{Html, Node};

use super::policy::element_rule;
use super::report::{RemovalReport, RemovalRule, parse_like_ammonia};

/// An invariant of editor.md §7.5 that the input of the structural pass
/// broke. ammonia's output never does, so each one is a bug in the
/// sanitiser, an ammonia upgrade or the parser.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Violation {
    /// An element outside content-v1, or outside the HTML namespace.
    #[error("the element `{element}` is not in content-v1")]
    ElementOutsidePolicy {
        /// The tag name.
        element: String,
    },
    /// An attribute that its element does not allow.
    #[error("the attribute `{attribute}` of `{element}` is not in content-v1")]
    AttributeOutsidePolicy {
        /// The tag name.
        element: String,
        /// The attribute name, with its namespace prefix if it has one.
        attribute: String,
    },
    /// A `table` with a `p` ancestor.
    #[error("a `table` is inside a `p`")]
    TableInsideParagraph,
    /// A comment, doctype or processing instruction, which ammonia never
    /// writes.
    #[error("a {kind} node is not in content-v1")]
    UnexpectedNode {
        /// `comment`, `doctype`, `processing instruction` or `document`.
        kind: &'static str,
    },
}

/// An element that a rule of the walk looks for among the ancestors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counted {
    /// A `p`: no `table` may be inside one.
    Paragraph,
    /// A `sub` or `sup`: one inside another is unwrapped.
    Script,
    /// An `a`: one inside another, in the same table cell or outside every
    /// cell, is unwrapped.
    Link,
    /// A `td` or `th`, which starts a new count of links (see [`Open`]).
    Cell,
}

impl Counted {
    /// The kind of the element `tag`, if the walk counts it.
    fn of(tag: &str) -> Option<Self> {
        match tag {
            "p" => Some(Self::Paragraph),
            "sub" | "sup" => Some(Self::Script),
            "a" => Some(Self::Link),
            "td" | "th" => Some(Self::Cell),
            _ => None,
        }
    }
}

/// The [`Counted`] elements among the ancestors of the node the walk is at.
///
/// Links are counted per table cell, the way the parser sees them: a `td`
/// or `th` puts a marker in the list of active formatting elements, and an
/// `a` start tag closes only a link opened after the last marker. A link
/// inside a cell of a table that a link holds therefore parses back as it
/// is written, and is kept (`tests/fixtures/regressions/link-around-table`).
#[derive(Debug)]
struct Open {
    paragraphs: usize,
    scripts: usize,
    /// The open links outside every cell, then one count per open cell.
    /// Never empty.
    links: Vec<usize>,
}

impl Default for Open {
    fn default() -> Self {
        Self {
            paragraphs: 0,
            scripts: 0,
            links: vec![0],
        }
    }
}

impl Open {
    /// The walk entered an element of `kind`.
    fn enter(&mut self, kind: Counted) {
        match kind {
            Counted::Paragraph => self.paragraphs += 1,
            Counted::Script => self.scripts += 1,
            Counted::Link => *self.innermost_links() += 1,
            Counted::Cell => self.links.push(0),
        }
    }

    /// The walk left an element of `kind`, which it entered before.
    fn leave(&mut self, kind: Counted) {
        match kind {
            Counted::Paragraph => self.paragraphs -= 1,
            Counted::Script => self.scripts -= 1,
            Counted::Link => *self.innermost_links() -= 1,
            Counted::Cell => {
                self.links.pop();
            }
        }
    }

    /// Whether an element of `kind` is an ancestor; for [`Counted::Link`],
    /// one inside the innermost open cell, or outside every cell when no
    /// cell is open.
    fn inside(&self, kind: Counted) -> bool {
        match kind {
            Counted::Paragraph => self.paragraphs > 0,
            Counted::Script => self.scripts > 0,
            Counted::Link => self.links.last().is_some_and(|&links| links > 0),
            Counted::Cell => self.links.len() > 1,
        }
    }

    /// The rule that unwraps an element of `kind` here, because an element
    /// of the same kind holds it: a `sub` or `sup` in another, or a link in
    /// a link in the same cell. `None` when the element stays.
    fn unwraps(&self, kind: Counted) -> Option<RemovalRule> {
        match kind {
            Counted::Script if self.inside(kind) => Some(RemovalRule::NestedSubSupUnwrapped),
            Counted::Link if self.inside(kind) => Some(RemovalRule::NestedLinkUnwrapped),
            _ => None,
        }
    }

    /// The link count of the innermost open cell, or of the body.
    fn innermost_links(&mut self) -> &mut usize {
        self.links
            .last_mut()
            .expect("the count outside every cell is never popped")
    }
}

/// One step of the walk: enter a node, or close an element that was opened.
enum Step<N> {
    Enter(N),
    Exit {
        name: QualName,
        /// The element's kind, if [`Open`] counts it.
        counted: Option<Counted>,
    },
}

/// Runs the structural pass over `html`, one ammonia output, and records
/// what it removed in `report`.
///
/// - `html`: the output of one ammonia pass.
/// - `report`: the report of the current sanitiser run; the pass adds its
///   [`RemovalRule::IframeContentRemoved`],
///   [`RemovalRule::ImageWithoutSrcRemoved`],
///   [`RemovalRule::NestedSubSupUnwrapped`] and
///   [`RemovalRule::NestedLinkUnwrapped`] entries.
///
/// Its output may still change once more when it is applied to it; the
/// output of that second application is a fixed point (see "How the output
/// settles" in the module documentation).
///
/// # Errors
///
/// A [`Violation`] when `html` breaks an invariant of editor.md §7.5. The
/// report may then hold entries for the part already walked.
pub fn restructure(html: &str, report: &mut RemovalReport) -> Result<String, Violation> {
    walk(&parse_like_ammonia(html), html.len(), report)
}

/// The walk of [`restructure`] over a parsed tree; `capacity` is a guess at
/// the output length. Apart from tests, which build trees that no parse
/// produces, the tree is always a fresh parse of ammonia's output.
fn walk(parsed: &Html, capacity: usize, report: &mut RemovalReport) -> Result<String, Violation> {
    let mut out = Vec::with_capacity(capacity);
    let mut serializer = HtmlSerializer::new(&mut out, SerializeOpts::default());
    let mut stack: Vec<Step<_>> = parsed
        .root_element()
        .children()
        .rev()
        .map(Step::Enter)
        .collect();
    let mut open = Open::default();
    while let Some(step) = stack.pop() {
        let node = match step {
            Step::Exit { name, counted } => {
                if let Some(kind) = counted {
                    open.leave(kind);
                }
                written(serializer.end_elem(name));
                continue;
            }
            Step::Enter(node) => node,
        };
        let element = match node.value() {
            Node::Text(text) => {
                written(serializer.write_text(text));
                continue;
            }
            Node::Element(element) => element,
            Node::Comment(_) => return Err(Violation::UnexpectedNode { kind: "comment" }),
            Node::Doctype(_) => return Err(Violation::UnexpectedNode { kind: "doctype" }),
            Node::ProcessingInstruction(_) => {
                return Err(Violation::UnexpectedNode {
                    kind: "processing instruction",
                });
            }
            Node::Document | Node::Fragment => {
                return Err(Violation::UnexpectedNode { kind: "document" });
            }
        };
        let tag = element.name();
        let rule = element_rule(tag)
            .filter(|_| element.name.ns == ns!(html))
            .ok_or_else(|| Violation::ElementOutsidePolicy {
                element: tag.to_owned(),
            })?;
        for (name, _) in &element.attrs {
            if name.ns != ns!() || rule.attribute(&name.local).is_none() {
                return Err(Violation::AttributeOutsidePolicy {
                    element: tag.to_owned(),
                    attribute: match &name.prefix {
                        Some(prefix) => format!("{prefix}:{}", name.local),
                        None => name.local.to_string(),
                    },
                });
            }
        }
        let counted = Counted::of(tag);
        if let Some(rule) = counted.and_then(|kind| open.unwraps(kind)) {
            // The children take the element's place; they stay inside the
            // outer element of the same kind, so the counter is unchanged.
            report.record(rule, tag, None);
            stack.extend(node.children().rev().map(Step::Enter));
            continue;
        }
        match tag {
            "img" if element.attr("src").is_none() => {
                report.record(RemovalRule::ImageWithoutSrcRemoved, tag, None);
                continue;
            }
            "table" if open.inside(Counted::Paragraph) => {
                return Err(Violation::TableInsideParagraph);
            }
            _ => {}
        }
        // The attributes in the order of the element's rule: every one of
        // them is in it, as checked above.
        let attributes = rule.attributes.iter().filter_map(|allowed| {
            element
                .attrs
                .iter()
                .find(|(name, _)| &*name.local == allowed.name)
                .map(|(name, value)| (name, &**value))
        });
        written(serializer.start_elem(element.name.clone(), attributes));
        if tag == "iframe" {
            if node.has_children() {
                report.record(RemovalRule::IframeContentRemoved, tag, None);
            }
            written(serializer.end_elem(element.name.clone()));
            continue;
        }
        if let Some(kind) = counted {
            open.enter(kind);
        }
        stack.push(Step::Exit {
            name: element.name.clone(),
            counted,
        });
        stack.extend(node.children().rev().map(Step::Enter));
    }
    drop(serializer);
    // The serializer writes only `&str` data and ASCII markup.
    Ok(String::from_utf8(out).expect("html5ever's serializer writes UTF-8"))
}

/// The result of one serializer call. The serializer writes into a
/// `Vec<u8>`, whose `io::Write` never fails.
fn written(result: io::Result<()>) {
    result.expect("writing into a Vec<u8> cannot fail");
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn run(html: &str) -> (String, RemovalReport) {
        let mut report = RemovalReport::default();
        let output = restructure(html, &mut report).expect("no violation");
        (output, report)
    }

    fn rewritten(html: &str) -> String {
        run(html).0
    }

    fn counts(report: &RemovalReport) -> Vec<(RemovalRule, String, usize)> {
        report
            .entries()
            .map(|(removal, count)| (removal.rule, removal.element.clone(), count))
            .collect()
    }

    const IFRAME_TAIL: &str = r#"allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin""#;

    #[test]
    fn iframe_children_are_removed() {
        let (output, report) = run(&format!(
            r#"<p data-padlet-embed=""><iframe src="https://padlet.com/embed/k" {IFRAME_TAIL}>&lt;b&gt;szöveg&lt;/b&gt;</iframe></p>"#
        ));
        assert_eq!(
            output,
            format!(
                r#"<p data-padlet-embed=""><iframe src="https://padlet.com/embed/k" {IFRAME_TAIL}></iframe></p>"#
            )
        );
        assert_eq!(
            counts(&report),
            [(RemovalRule::IframeContentRemoved, "iframe".into(), 1)]
        );
        // Raw text that looks like markup is a child too, and so is text
        // that the parser kept after a broken end tag.
        assert_eq!(
            rewritten("<iframe><p>x</p><script>y</script></iframe><iframe>a</iframe</iframe>"),
            "<iframe></iframe><iframe></iframe>"
        );
        let (_, empty) = run("<iframe></iframe>");
        assert!(empty.is_empty());
    }

    #[test]
    fn an_image_without_src_is_removed() {
        let (output, report) = run(
            r#"<p>a<img alt="x" width="10">b</p><p><img src="/storage/uploads/a.jpg" alt="y"></p>"#,
        );
        assert_eq!(
            output,
            r#"<p>ab</p><p><img src="/storage/uploads/a.jpg" alt="y"></p>"#
        );
        assert_eq!(
            counts(&report),
            [(RemovalRule::ImageWithoutSrcRemoved, "img".into(), 1)]
        );
        // An empty `src` is a `src`; the URL rule, not this pass, decides it.
        assert_eq!(rewritten(r#"<img src="">"#), r#"<img src="">"#);
    }

    #[test]
    fn nested_sub_and_sup_are_unwrapped() {
        let cases = [
            ("<sub>a<sup>b</sup>c</sub>", "<sub>abc</sub>", 1),
            ("<sup>a<sub>b<sup>c</sup></sub></sup>", "<sup>abc</sup>", 2),
            (
                "<sub><em><sub>x</sub></em></sub>",
                "<sub><em>x</em></sub>",
                1,
            ),
            ("<sub>a</sub><sup>b</sup>", "<sub>a</sub><sup>b</sup>", 0),
            (
                "<p>H<sub>2</sub>O x<sup>2</sup></p>",
                "<p>H<sub>2</sub>O x<sup>2</sup></p>",
                0,
            ),
        ];
        for (input, expected, unwrapped) in cases {
            let (output, report) = run(input);
            assert_eq!(output, expected, "{input}");
            assert_eq!(
                report.count(RemovalRule::NestedSubSupUnwrapped),
                unwrapped,
                "{input}"
            );
        }
    }

    /// `n` applications of [`restructure`] to `html`: the input first, then
    /// each output.
    fn applications(html: &str, n: usize) -> Vec<String> {
        let mut outputs = vec![html.to_owned()];
        for _ in 0..n {
            outputs.push(rewritten(outputs.last().expect("the input")));
        }
        outputs
    }

    #[test]
    fn nested_links_are_unwrapped() {
        // No parse nests a link in a link by itself; foster parenting does,
        // when a table stands between them. The rule removes the inner `a`
        // and keeps its text inside the outer link.
        let cases = [
            (
                r#"<a href="/x">a<table><a href="/y">b</a></table>c</a>"#,
                r#"<a href="/x">ab<table></table></a>c"#,
                1,
            ),
            // In a cell, the parser starts a new list of formatting
            // elements, so a link there parses back as written and stays.
            (
                r#"<a href="/x"><table><tbody><tr><td><a href="/y">b</a></td></tr></tbody></table></a>"#,
                r#"<a href="/x"><table><tbody><tr><td><a href="/y">b</a></td></tr></tbody></table></a>"#,
                0,
            ),
            // A header cell starts a new list just as a `td` does.
            (
                r#"<a href="/x"><table><tbody><tr><th><a href="/y">b</a></th></tr></tbody></table></a>"#,
                r#"<a href="/x"><table><tbody><tr><th><a href="/y">b</a></th></tr></tbody></table></a>"#,
                0,
            ),
            // A link nested in the cell's own link goes; the cell's link
            // stays.
            (
                r#"<a href="/x"><table><tbody><tr><td><a href="/y"><table><a href="/z">b"#,
                r#"<a href="/x"><table><tbody><tr><td><a href="/y">b<table></table></a></td></tr></tbody></table></a>"#,
                1,
            ),
            (
                r#"<a href="/x"><table><tbody><tr><th><a href="/y"><table><a href="/z">b"#,
                r#"<a href="/x"><table><tbody><tr><th><a href="/y">b<table></table></a></th></tr></tbody></table></a>"#,
                1,
            ),
            // Siblings are not nested.
            (
                r#"<a href="/x">a</a><a href="/y">b</a>"#,
                r#"<a href="/x">a</a><a href="/y">b</a>"#,
                0,
            ),
        ];
        for (input, expected, unwrapped) in cases {
            let (output, report) = run(input);
            assert_eq!(output, expected, "{input}");
            assert_eq!(
                report.count(RemovalRule::NestedLinkUnwrapped),
                unwrapped,
                "{input}"
            );
            assert_eq!(rewritten(&output), output, "a fixed point: {input}");
        }
    }

    // The shrunk failure of `the_output_keeps_the_invariants_and_settles`
    // before nested links were unwrapped: the input needed three rewrites
    // (`tests/fixtures/regressions/link-fostered-into-link.html`).
    #[test]
    fn a_link_fostered_into_a_link_settles_in_one_rewrite() {
        let input = r#"<p></em><a rel="noopener noreferrer" href="/x"><h2><sub><h2><table><a rel="noopener noreferrer" href="/x">"#;
        let (output, report) = run(input);
        // The parse fosters the second link out of the table into the inner
        // heading, inside the first link; the walk unwraps it. Before, the
        // next parse moved the inner heading up into the outer one, and the
        // parse after that closed the outer heading.
        assert_eq!(
            output,
            r#"<p><a href="/x" rel="noopener noreferrer"></a></p><h2><a href="/x" rel="noopener noreferrer"><sub><h2><table></table></h2></sub></a></h2>"#
        );
        assert_eq!(
            counts(&report),
            [(RemovalRule::NestedLinkUnwrapped, "a".into(), 1)]
        );
        assert_eq!(rewritten(&output), output);
        // The minimal form of the same shape.
        assert_eq!(
            applications(r#"<h2><a href="/x"><h3><table><a href="/x">"#, 2)[1..],
            [
                r#"<h2><a href="/x"><h3><table></table></h3></a></h2>"#,
                r#"<h2><a href="/x"><h3><table></table></h3></a></h2>"#,
            ]
        );
    }

    #[test]
    fn an_unwrapped_link_can_leave_a_heading_for_the_next_parse() {
        // The fostered link holds a heading; unwrapping it makes the two
        // headings parent and child, which the next parse closes. That is
        // the second rewrite the module documentation allows, and the last.
        assert_eq!(
            applications(r#"<a href="/x"><h2><table><a href="/y"><h3>x"#, 3)[1..],
            [
                r#"<a href="/x"><h2><h3>x</h3><table></table></h2></a>"#,
                r#"<a href="/x"><h2></h2><h3>x</h3><table></table></a>"#,
                r#"<a href="/x"><h2></h2><h3>x</h3><table></table></a>"#,
            ]
        );
    }

    #[test]
    fn nested_links_settle_beyond_the_adoption_agencys_eight_rounds() {
        // Before the rule, each parse repaired eight levels: 7 quotes took
        // two rewrites, 8 took three, 16 took four, 24 took five and 200 took
        // 27.
        for depth in [1, 7, 8, 9, 16, 17, 24, 200] {
            let input = format!(
                r#"<a href="/x">{}<table><a href="/y">x"#,
                "<blockquote>".repeat(depth)
            );
            let (output, report) = run(&input);
            assert_eq!(
                output,
                format!(
                    r#"<a href="/x">{}x<table></table>{}</a>"#,
                    "<blockquote>".repeat(depth),
                    "</blockquote>".repeat(depth)
                ),
                "{depth}"
            );
            assert_eq!(report.count(RemovalRule::NestedLinkUnwrapped), 1);
            assert_eq!(rewritten(&output), output, "{depth}");
        }
    }

    #[test]
    fn closed_markup_nests_links_from_eight_blocks() {
        // Each round of the adoption agency moves one block out of the outer
        // link. With seven quotes the eight rounds suffice and the inner
        // link keeps its own text; with eight the last copy of the outer
        // link stays open around the inner one, so even markup that closes
        // every element parses into a link inside a link, which the walk
        // unwraps.
        let closed = |depth: usize| {
            format!(
                r#"<a href="/x">{}<a href="/y">x</a>{}</a>"#,
                "<blockquote>".repeat(depth),
                "</blockquote>".repeat(depth)
            )
        };
        let copies = |depth: usize| r#"<blockquote><a href="/x"></a>"#.repeat(depth);
        let cases = [
            (
                7,
                format!(
                    r#"<a href="/x"></a>{}<a href="/y">x</a>{}"#,
                    copies(7),
                    "</blockquote>".repeat(7)
                ),
                0,
            ),
            (
                8,
                format!(
                    r#"<a href="/x"></a>{}<blockquote><a href="/x">x</a></blockquote>{}"#,
                    copies(7),
                    "</blockquote>".repeat(7)
                ),
                1,
            ),
        ];
        for (depth, expected, unwrapped) in cases {
            let (output, report) = run(&closed(depth));
            assert_eq!(output, expected, "{depth}");
            assert_eq!(
                report.count(RemovalRule::NestedLinkUnwrapped),
                unwrapped,
                "{depth}"
            );
            assert_eq!(rewritten(&output), output, "{depth}");
        }
    }

    #[test]
    fn attributes_are_written_in_canonical_order() {
        // The orders ammonia's `swap_remove` and its hash-ordered forced
        // attributes can leave behind.
        let cases = [
            (
                r#"<a rel="noopener noreferrer" target="_blank" href="/x">x</a>"#,
                r#"<a href="/x" target="_blank" rel="noopener noreferrer">x</a>"#,
            ),
            (
                r#"<iframe referrerpolicy="strict-origin-when-cross-origin" allowfullscreen="" src="https://padlet.com/embed/k" title="t" height="608" width="100%" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"></iframe>"#,
                &format!(
                    r#"<iframe src="https://padlet.com/embed/k" width="100%" height="608" title="t" {IFRAME_TAIL}></iframe>"#
                ),
            ),
            (
                r#"<img height="2" alt="a" src="/storage/uploads/a.jpg" width="1">"#,
                r#"<img src="/storage/uploads/a.jpg" alt="a" width="1" height="2">"#,
            ),
            (
                r#"<ol type="a" start="5"><li>x</li></ol>"#,
                r#"<ol start="5" type="a"><li>x</li></ol>"#,
            ),
            (
                r#"<table><tbody><tr><td rowspan="2" colspan="3">x</td></tr></tbody></table>"#,
                r#"<table><tbody><tr><td colspan="3" rowspan="2">x</td></tr></tbody></table>"#,
            ),
            (
                r#"<p data-padlet-embed="" data-youtube-video="">x</p>"#,
                r#"<p data-youtube-video="" data-padlet-embed="">x</p>"#,
            ),
        ];
        for (input, expected) in cases {
            let (output, report) = run(input);
            assert_eq!(output, expected);
            assert!(report.is_empty(), "a reorder is not a removal");
        }
    }

    // Structural table cases: the shapes that reach the pass from ammonia,
    // and what the fresh parse makes of them.
    #[test]
    fn tables_take_the_shape_the_parser_gives_them() {
        let cases = [
            // Canonical: unchanged.
            (
                r#"<table><tbody><tr><th colspan="2">Fej</th></tr><tr><td>a</td><td>b</td></tr></tbody></table>"#,
                r#"<table><tbody><tr><th colspan="2">Fej</th></tr><tr><td>a</td><td>b</td></tr></tbody></table>"#,
            ),
            // Rows left directly in the table by an unwrapped `thead` get the
            // `tbody` the parser inserts.
            (
                "<table><tr><td>a</td></tr><tbody><tr><td>b</td></tr></tbody></table>",
                "<table><tbody><tr><td>a</td></tr></tbody><tbody><tr><td>b</td></tr></tbody></table>",
            ),
            // Text left in the table by an unwrapped `caption` moves in
            // front of it (foster parenting).
            (
                "<table>Cím<tbody><tr><td>x</td></tr></tbody></table>",
                "Cím<table><tbody><tr><td>x</td></tr></tbody></table>",
            ),
            // A table that ammonia's tree left inside a `p` comes out after
            // it: the parser closes the `p` first.
            (
                "<p>a<table><tbody><tr><td>x</td></tr></tbody></table>b</p>",
                "<p>a</p><table><tbody><tr><td>x</td></tr></tbody></table>b<p></p>",
            ),
            // Nested tables keep their cells.
            (
                "<table><tbody><tr><td><table><tbody><tr><td>x</td></tr></tbody></table></td></tr></tbody></table>",
                "<table><tbody><tr><td><table><tbody><tr><td>x</td></tr></tbody></table></td></tr></tbody></table>",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(rewritten(input), expected, "{input}");
            assert_eq!(rewritten(expected), expected, "a fixed point: {expected}");
        }
    }

    #[test]
    fn the_dialect_is_html5evers() {
        // Text escaping (editor.md §3.8): only &, <, > and U+00A0.
        assert_eq!(
            rewritten("<p>a &amp; b &lt; c &gt; d\u{a0}e \"f\" 'g'</p>"),
            "<p>a &amp; b &lt; c &gt; d&nbsp;e \"f\" 'g'</p>"
        );
        assert_eq!(
            rewritten(r#"<img src="/storage/uploads/a.jpg" alt="&quot;x&quot; &amp; <y>">"#),
            r#"<img src="/storage/uploads/a.jpg" alt="&quot;x&quot; &amp; &lt;y&gt;">"#
        );
        assert_eq!(rewritten("a<br>b<hr>"), "a<br>b<hr>");
        assert_eq!(rewritten(""), "");
    }

    #[test]
    fn invariant_violations_are_reported() {
        let violation = |html: &str| restructure(html, &mut RemovalReport::default()).unwrap_err();
        assert_eq!(
            violation("<div>x</div>"),
            Violation::ElementOutsidePolicy {
                element: "div".into()
            }
        );
        assert_eq!(
            violation("<p><svg><a>x</a></svg></p>"),
            Violation::ElementOutsidePolicy {
                element: "svg".into()
            }
        );
        assert_eq!(
            violation(r#"<p class="x">y</p>"#),
            Violation::AttributeOutsidePolicy {
                element: "p".into(),
                attribute: "class".into()
            }
        );
        assert_eq!(
            violation("<p>a<!--c-->b</p>"),
            Violation::UnexpectedNode { kind: "comment" }
        );
        assert_eq!(
            violation("<script>x</script>").to_string(),
            "the element `script` is not in content-v1"
        );
    }

    #[test]
    fn a_table_inside_a_paragraph_is_a_violation() {
        // No parse builds this tree (the table cases above show the parser
        // closing the `p`), so the test moves the table into the paragraph
        // by hand.
        let mut tree = parse_like_ammonia(
            "<p>a<span>b</span></p><table><tbody><tr><td>x</td></tr></tbody></table>",
        );
        let root = tree.root_element();
        let span = root.first_child().unwrap().last_child().unwrap().id();
        let table = root.last_child().unwrap().id();
        tree.tree.get_mut(span).unwrap().append_id(table);
        let mut report = RemovalReport::default();
        assert_eq!(
            walk(&tree, 0, &mut report),
            Err(Violation::TableInsideParagraph)
        );
        // A `p` inside a cell is closed with the cell, so a later table at
        // the top level is outside every paragraph.
        let html = "<table><tbody><tr><td><p>a</p></td></tr></tbody></table><p>b</p>";
        assert_eq!(rewritten(html), html);
    }

    #[test]
    fn the_walk_survives_deep_nesting() {
        let deep = format!("{}x{}", "<span>".repeat(20_000), "</span>".repeat(20_000));
        assert_eq!(rewritten(&deep), deep);
    }

    proptest! {
        #[test]
        fn restructure_never_panics(input in any::<String>()) {
            let _ = restructure(&input, &mut RemovalReport::default());
        }

        // What the pass writes has no iframe children, no image without
        // `src`, no nested `sub` or `sup` and no link nested in a link in
        // the same cell. A tree that only tag soup gives (an `li` that a
        // `table` fosters inside another `li`, a heading that a heading
        // holds) is written as the parser built it and parses differently
        // once, so the second application may still change it; its output
        // is a fixed point that the third application returns with nothing
        // removed ("How the output settles" in the module documentation).
        #[test]
        fn the_output_keeps_the_invariants_and_settles(parts in prop::collection::vec(prop::sample::select(&[
            "<p>", "</p>", "<sub>", "</sub>", "<sup>", "</sup>", "<em>", "</em>", "x", "&amp;",
            "<img alt=\"a\">", "<img alt=\"a\" src=\"/storage/uploads/a.jpg\" width=\"1\">",
            "<iframe height=\"1\" src=\"https://padlet.com/embed/k\">", "</iframe>",
            "<a rel=\"noopener noreferrer\" href=\"/x\">", "</a>", "<table>", "<tr>", "<td rowspan=\"2\" colspan=\"2\">", "</td>", "<th>", "</th>", "</table>",
            "<ol type=\"a\" start=\"2\">", "<li>", "</ol>", "<br>", "<h2>", "</h2>",
            "<h3>", "<blockquote>", "</blockquote>",
        ][..]), 0..30)) {
            let input = parts.concat();
            settles_in_two(&input)?;
        }

        // Nesting deeper than the adoption agency repairs in one parse: a
        // link around up to 240 levels of blocks and inline elements, with a
        // table at the bottom that fosters a second link into the first.
        #[test]
        fn nested_links_settle_at_any_depth(
            before in prop::collection::vec(prop::sample::select(NESTING), 0..4),
            levels in prop::collection::vec(prop::sample::select(NESTING), 0..120),
        ) {
            let input = format!(
                r#"{}<a href="/x">{}<table><a href="/y">x"#,
                before.concat(),
                levels.concat()
            );
            settles_in_two(&input)?;
        }
    }

    /// Start tags for [`nested_links_settle_at_any_depth`]: special
    /// elements, which the adoption agency moves one per round, and inline
    /// ones, which it copies or closes.
    const NESTING: &[&str] = &[
        "<blockquote>",
        "<ul><li>",
        "<ol><li>",
        "<li>",
        "<h2>",
        "<h3>",
        "<p>",
        "<em>",
        "<strong>",
        "<sub>",
        "<span>",
        r#"<a href="/z">"#,
    ];

    /// Checks that two applications of [`restructure`] to `input` keep the
    /// invariants and reach a fixed point: a third application returns the
    /// second output and removes nothing.
    fn settles_in_two(input: &str) -> Result<(), TestCaseError> {
        let mut outputs = vec![input.to_owned()];
        for _ in 0..2 {
            let mut report = RemovalReport::default();
            let output = restructure(outputs.last().expect("the input"), &mut report)
                .expect("only content-v1 in the pool");
            assert_invariants(&output)?;
            outputs.push(output);
        }
        let mut last = RemovalReport::default();
        let settled = restructure(&outputs[2], &mut last);
        prop_assert_eq!(settled.as_ref(), Ok(&outputs[2]), "{:?}", outputs);
        prop_assert!(last.is_empty(), "{:?}", last);
        Ok(())
    }

    /// The invariants of [`restructure`]'s output, checked on a fresh parse.
    fn assert_invariants(output: &str) -> Result<(), TestCaseError> {
        let parsed = parse_like_ammonia(output);
        for node in parsed.root_element().descendants().skip(1) {
            let Node::Element(element) = node.value() else {
                continue;
            };
            let tag = element.name();
            if tag == "iframe" {
                prop_assert!(!node.has_children(), "{}", output);
            }
            if tag == "img" {
                prop_assert!(element.attr("src").is_some(), "{}", output);
            }
            if matches!(tag, "sub" | "sup") {
                let nested = node.ancestors().any(
                    |a| matches!(a.value(), Node::Element(e) if matches!(e.name(), "sub" | "sup")),
                );
                prop_assert!(!nested, "{}", output);
            }
            if tag == "a" {
                // The ancestors up to the innermost cell hold no link.
                let nested = node
                    .ancestors()
                    .filter_map(|a| match a.value() {
                        Node::Element(e) => Some(e.name()),
                        _ => None,
                    })
                    .take_while(|name| !matches!(*name, "td" | "th"))
                    .any(|name| name == "a");
                prop_assert!(!nested, "{}", output);
            }
        }
        // Canonical attribute order, read from the bytes: a parse does not
        // keep the order.
        for (tag, names) in start_tags(output) {
            let rule = element_rule(&tag).expect("content-v1");
            let positions: Vec<Option<usize>> = names
                .iter()
                .map(|name| rule.attributes.iter().position(|a| a.name == name))
                .collect();
            prop_assert!(positions.iter().all(Option::is_some), "{}", output);
            prop_assert!(positions.is_sorted(), "{}: {:?}", output, names);
        }
        Ok(())
    }

    /// The tag name and attribute names of every start tag of `output`, in
    /// the order written. The serializer escapes `<` and `>` in text and
    /// `"` in values, and writes no raw text (no `iframe` keeps a child), so
    /// every `<` starts a tag and the first `>` ends it.
    fn start_tags(output: &str) -> Vec<(String, Vec<String>)> {
        output
            .split('<')
            .skip(1)
            .filter(|tag| !tag.starts_with('/'))
            .map(|tag| {
                let inside = &tag[..tag.find('>').expect("a closed tag")];
                let (name, mut rest) = inside.split_once(' ').unwrap_or((inside, ""));
                let mut names = Vec::new();
                while let Some((attribute, after)) = rest.split_once("=\"") {
                    names.push(attribute.trim().to_owned());
                    rest = after.split_once('"').map_or("", |(_, tail)| tail);
                }
                (name.to_owned(), names)
            })
            .collect()
    }

    #[test]
    fn start_tags_reads_the_written_order() {
        assert_eq!(
            start_tags(r#"<a href="/x" target="_blank">a &lt; b</a><br><td colspan="2">"#),
            [
                ("a".to_owned(), vec!["href".to_owned(), "target".to_owned()]),
                ("br".to_owned(), vec![]),
                ("td".to_owned(), vec!["colspan".to_owned()]),
            ]
        );
    }
}
