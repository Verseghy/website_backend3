//! N5, its block rules: legacy block markup mapped onto content-v1
//! (data-migration.md §7.1 and §7.4, editor.md §3 and §9, ADR 0017 §2
//! step 5).
//!
//! `CKEditor` 4 wrote, and its source mode let editors type, block markup
//! that content-v1 does not have (research: gap-06 §4.2-§4.3). N7's
//! sanitiser removes every attribute, element and declaration outside
//! content-v1, and a removal that no earlier step planned stops the import
//! (data-migration.md §7.1, N7). So N5 maps that markup first onto what a
//! reader saw, in the form content-v1 has for it. These are the block rules
//! of N5, as editor.md §9 lists them; its inline rules (`b`, `i`, `span`
//! and the rest) are in `marks.rs`, which runs right after these in the
//! same step.
//!
//! - **`p[align]` becomes `text-align`.** A browser maps `align` on a
//!   paragraph to `text-align` when its value is `left`, `right`, `center`
//!   or `justify`, matched whole and ASCII case-insensitively (WHATWG HTML,
//!   "Rendering", §15.3.3), as N3 reads it. That value becomes
//!   `text-align: <value>` in lower case, written in front of the
//!   paragraph's `style` as N3 writes its centring
//!   ([`dom::prepend_declaration`]), and the attribute is removed
//!   ([`Rule::ParagraphAlignMapped`]). A `style` declaration wins over the
//!   attribute in a browser, so a paragraph whose `style` sets a
//!   `text-align` that content-v1 keeps only loses the attribute, and so
//!   does one whose value a browser maps to nothing (`middle`, or ` left `
//!   with its spaces); neither changes what it shows
//!   ([`Rule::ParagraphAlignDropped`]). A `style` whose `text-align`
//!   content-v1 does not keep is unclear, as in N3: an invalid value left
//!   the attribute in force, which is what N5 keeps and N7 then confirms by
//!   dropping the invalid declaration, but `inherit` or `start` showed
//!   another alignment, so such a paragraph also counts as
//!   [`Rule::ParagraphAlignUnclear`] for review. The corpus has 23
//!   `p[align]`, 18 of them `justify` **\[500\]**.
//! - **`table[align]` is dropped** ([`Rule::TableAlignDropped`]). Both
//!   public sites centre every content-v1 table (editor.md §9), so the
//!   attribute goes whatever its value: a `left` or `right` table, which
//!   floated, is centred like the others. The corpus has 19, 17 of them
//!   `center` **\[500\]**.
//! - **`h5` and `h6` become `h4`** ([`Rule::HeadingToH4`]), the smallest
//!   heading content-v1 has (editor.md §3.2), with their attributes. The
//!   corpus has none **\[500\]**; the rule is there for the full dump.
//! - **An italic `h2` becomes `<h2><em>`** ([`Rule::ItalicHeadingToEm`]).
//!   content-v1 keeps no `font-style` on a block, and the editor's italic
//!   mark reads `font-style: italic` on any element, so such a heading could
//!   never be a fixed point (editor.md §3.4). `CKEditor`'s Styles combo
//!   wrote it, on 2 headings **\[500\]**. An `h2` is italic when its `style`
//!   makes it italic in a browser's cascade (CSS Cascading 4, §6.1, where
//!   importance comes before the order of appearance): of the `font-style`
//!   declarations a browser accepts, the last one marked `!important` wins,
//!   and the last one of all when none is. The declarations are read with
//!   the sanitiser's reader (ASCII case-insensitive, editor.md §3.1 rule 5),
//!   which strips `!important` without saying so, so N5 looks for the
//!   marker itself ([`is_important`]); one whose value a browser rejects is
//!   skipped, as a browser skips it. N5 then removes every `font-style`
//!   declaration of the heading ([`without_italic`]) and wraps its content
//!   in one `em`, from its first child that is not white space or a comment
//!   to its last, the span N3 gives its runs.
//!
//!   An `em` or `i` inside the new `em` is unwrapped when it has no
//!   attribute and holds no block: italic inside italic shows nothing more,
//!   and the editor would merge the two into one mark. Unwrapping `i` too
//!   keeps the result the same whichever of N5's rules runs first, since its
//!   inline rule turns `i` into `em`. One with an attribute stays, for N5's
//!   inline rules and N7, which report what they remove: unwrapped, a
//!   `font-style: normal` that kept its text upright, a `class` or an `id`
//!   would be lost without a word. One that holds a block (N3's list of
//!   blocks) stays too: it is tag soup, not a mark, and it is what keeps the
//!   next parse from repairing the nesting below it. Unwrapped, it could
//!   leave a heading that tag soup put inside a link inside another heading
//!   directly in that heading once the next parse has moved the link, and
//!   the parse after that would close the outer heading at the inner one's
//!   start tag: one change per pass, so the body would not settle within the
//!   three passes. The same holds for an italic heading that tag soup nests
//!   in another one (a `div` between them keeps the parse from closing the
//!   outer one): it gets an `em` of its own only when it holds a block, so
//!   that italic inside italic is one `em` there too.
//!
//!   The rule reads only `h2` and only `font-style`, because the corpus has
//!   no other `font-style` declaration (research: gap-06 §4.3): `oblique`, a
//!   CSS-wide keyword such as `inherit`, the `font` shorthand and a
//!   `font-style` on another element are left to N7, which reports what it
//!   removes. Whether the editor writes the new `em` back in the same place
//!   when the heading also holds `strong` or a link, whose marks it may nest
//!   in another order, is open (data-migration.md §13, item 10).
//! - **`p[lang]` is stripped** ([`Rule::ParagraphLangStripped`]):
//!   content-v1 stores no `lang` (editor.md §3.2). The corpus has 5
//!   **\[500\]**.
//!
//! Only HTML elements are read, as in N3. The parse never builds a `p`, a
//! `table` or a heading in SVG or `MathML`: their start tags break out of
//! foreign content (WHATWG HTML, "The rules for parsing tokens in foreign
//! content"), so `<svg><p align="center">` gives an HTML paragraph after
//! the `svg`, which the rules map like any other.
//!
//! **Order.** N5 runs after N3 and before N6, in the order of
//! data-migration.md §7.1. N3 must see the markup first: it reads a
//! paragraph's `align` as the paragraph's own alignment, and counts `h5`
//! and `h6` among the headings that carry `text-align`. So a `p[align]`
//! inside a `center` keeps its own alignment, which N5 then writes as
//! `text-align`, and an `h5` that N3 centres keeps its `text-align` as an
//! `h4`. Where the order shows, N3 first is also what a reader saw: a
//! `center` that is an italic `h2`'s only content centres the heading
//! before N5 puts its text in an `em` (the other way round, the `em` would
//! stand around the `center`, an inline element around a block, whose
//! centring N3 drops), and inside a `center` a paragraph whose `style` says
//! `text-align: inherit` ends with N3's centring last, the declaration that
//! N7 keeps.
//!
//! **Fixed point.** Each rule removes what it looks for (the attribute, the
//! tag name, the `font-style` declarations), and nothing it writes is what
//! another rule looks for, so the next pass finds nothing: P4 holds as it
//! does for N3 and N6 (see the documentation of [`super`]). The `em` does
//! change how the next parse reads tag soup that puts a heading in a link
//! in another heading: it keeps that parse from closing the italic heading
//! at a heading start tag below it, so when the adoption agency moves the
//! link in the same parse, the closing waits for the pass after. So a body
//! that also fosters a link out of a table into another link, tag soup
//! that can need more than three passes anyway, can need a fourth pass
//! where it settled in three without N5, and is then refused (the unit
//! test `a_known_soup_is_refused_with_the_em`). The tests check that the
//! rules add no refusal over their own shapes. Like every refused body,
//! such a body stops the import's `apply`, which rolls back and names the
//! row, and the dry run names it before that (data-migration.md §7.4,
//! D140).
//!
//! **Cost.** One walk over the body plans every edit ([`plan`]), and the
//! rules then edit each element they name once. The walk also decides which
//! `em` and `i` the italic headings around them unwrap, nested headings
//! included, because whether a block lies below a node is known when the
//! walk leaves the node, after its children, and no edit changes it for
//! another node: an unwrapped `em` or `i` is no block and leaves its content
//! where it was, and a heading's new `em` holds a block exactly when the
//! heading does. So no subtree is walked twice, and the step is linear in
//! the body's size, nested italic headings included
//! (`tests/normalise/cost.rs`). Like everything under `src/domain/`, the
//! step is pure (ADR 0008).

use cssparser::{Delimiter, ParseError, Parser, Token};
use ego_tree::iter::Edge;
use ego_tree::{NodeId, NodeRef};
use html5ever::local_name;
use scraper::{Html, Node};

use super::center::{Alignment, BLOCKS, style_alignment};
use super::dom;
use super::report::{NormaliseReport, Rule};
use crate::domain::rich_text::css::declarations::{self, Component};

/// The values of a paragraph's `align` attribute that a browser maps to
/// `text-align`, each also the keyword N5 writes (WHATWG HTML, "Rendering",
/// §15.3.3).
const PARAGRAPH_ALIGNMENTS: [&str; 4] = ["left", "right", "center", "justify"];

/// What N5 does with a paragraph's `align` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParagraphAlign {
    /// It becomes `text-align: <keyword>` in front of the paragraph's
    /// `style`.
    Mapped {
        /// The `text-align` keyword, in lower case.
        keyword: &'static str,
        /// Whether the paragraph's `style` also holds a `text-align` that
        /// content-v1 does not keep.
        unclear: bool,
    },
    /// It is removed, and nothing else changes.
    Dropped,
}

/// What one declaration of a `style` says about `font-style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontStyle {
    /// It is not a `font-style` declaration that the sanitiser's reader
    /// can read.
    Other,
    /// A `font-style` declaration whose value a browser rejects, so it
    /// changes nothing.
    Rejected,
    /// A `font-style` declaration a browser applies.
    Applied {
        /// Whether its value is `italic`.
        italic: bool,
        /// Whether it is marked `!important`, which wins over the order of
        /// appearance in the cascade.
        important: bool,
    },
}

/// What N5's block rules edit in one body, found in one walk (see "Cost"
/// in the module documentation).
#[derive(Debug, Default)]
struct Plan {
    /// The paragraphs with an `align` or a `lang` attribute, in document
    /// order.
    paragraphs: Vec<NodeId>,
    /// The tables with an `align` attribute, in document order.
    tables: Vec<NodeId>,
    /// The `h5` and `h6` elements, in document order.
    minor_headings: Vec<NodeId>,
    /// The italic `h2` elements, in document order.
    italic_headings: Vec<ItalicHeading>,
    /// The `em` and `i` elements that the italic headings around them
    /// unwrap, in the order their end tags are written.
    italics: Vec<NodeId>,
}

/// An italic `h2`, and what the italic-heading rule does with it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ItalicHeading {
    /// The heading.
    id: NodeId,
    /// Its `style` without the `font-style` declarations, empty when
    /// nothing else is left ([`without_italic`]).
    style: String,
    /// Whether its content gets an `em` of its own. It does not when an
    /// italic heading around it puts it in an `em` already and it holds no
    /// block: that heading would unwrap its `em` again, which would leave
    /// the content where it was.
    em: bool,
}

/// Applies N5's block rules to `html` and counts what they did in
/// `report` (see the module documentation).
///
/// - `html`: a parsed body, after N3.
/// - `report`: the report of the current run.
pub(super) fn map_blocks(html: &mut Html, report: &mut NormaliseReport) {
    // The ids stay valid while the rules edit the tree: no rule removes an
    // element the plan names in another list, and unwrapping an `em` or `i`
    // leaves the ids of its content as they were.
    let plan = plan(html);
    for id in plan.paragraphs {
        map_paragraph(html, id, report);
    }
    for id in plan.tables {
        dom::edit_element(html, id, |table| dom::remove_attribute(table, "align"));
        report.record(Rule::TableAlignDropped);
    }
    for id in plan.minor_headings {
        dom::edit_element(html, id, |heading| dom::rename(heading, local_name!("h4")));
        report.record(Rule::HeadingToH4);
    }
    // Every new `em` first, then the unwrapping, as the rule reads: the `em`
    // takes the heading's content from its first child that is not white
    // space or a comment to its last, and then the `em` and `i` in it go.
    // Unwrapped first, an `em` that is the heading's first child would leave
    // the white space and comments at its start as the heading's first
    // children, outside the new `em`, where they were in italic before.
    for heading in &plan.italic_headings {
        emphasise_heading(html, heading);
        report.record(Rule::ItalicHeadingToEm);
    }
    for id in plan.italics {
        dom::unwrap(html, id);
    }
}

/// Finds what N5's block rules edit in `html`, in one walk (see "Cost" in
/// the module documentation).
///
/// - `html`: a parsed body, after N3.
fn plan(html: &Html) -> Plan {
    let mut plan = Plan::default();
    // One entry per open node: whether a block lies below it, and where it
    // is in `plan.italic_headings` when it is an italic heading.
    let mut open: Vec<(bool, Option<usize>)> = Vec::new();
    // How many of the open nodes are italic headings.
    let mut italic_depth = 0_usize;
    for edge in dom::node(html, dom::content(html)).traverse() {
        match edge {
            Edge::Open(node) => {
                let heading = plan.find(node);
                if heading.is_some() {
                    italic_depth += 1;
                }
                open.push((false, heading));
            }
            Edge::Close(node) => {
                let (holds_block, heading) = open.pop().expect("every node is opened first");
                if let Some(index) = heading {
                    italic_depth -= 1;
                    // The new `em` holds a block exactly when the heading
                    // does: white space and comments, the only children it
                    // may leave out, hold none.
                    plan.italic_headings[index].em = italic_depth == 0 || holds_block;
                } else if italic_depth > 0 && !holds_block && is_bare_italic(node) {
                    plan.italics.push(node.id());
                }
                if let Some((parent, _)) = open.last_mut() {
                    *parent |= holds_block || dom::html_name(node).is_some_and(is_block);
                }
            }
        }
    }
    plan
}

impl Plan {
    /// Adds `node` to the list of the rule that edits it, if one does, and
    /// returns its place in [`Plan::italic_headings`] when it is an italic
    /// heading. Its `em` is decided when the walk leaves it.
    ///
    /// - `node`: a node of the body, met in document order.
    fn find(&mut self, node: NodeRef<'_, Node>) -> Option<usize> {
        let (Some(name), Some(element)) = (dom::html_name(node), node.value().as_element()) else {
            return None;
        };
        match name {
            "p" if element.attr("align").is_some() || element.attr("lang").is_some() => {
                self.paragraphs.push(node.id());
            }
            "table" if element.attr("align").is_some() => self.tables.push(node.id()),
            "h5" | "h6" => self.minor_headings.push(node.id()),
            "h2" => {
                let style = element.attr("style").and_then(without_italic)?;
                self.italic_headings.push(ItalicHeading {
                    id: node.id(),
                    style,
                    em: true,
                });
                return Some(self.italic_headings.len() - 1);
            }
            _ => {}
        }
        None
    }
}

/// Whether `node` is an HTML `em` or `i` element without attributes, one
/// that the italic-heading rule may unwrap (see the module documentation).
///
/// - `node`: any node of a parsed body.
fn is_bare_italic(node: NodeRef<'_, Node>) -> bool {
    matches!(dom::html_name(node), Some("em" | "i"))
        && node
            .value()
            .as_element()
            .is_some_and(|element| element.attrs().next().is_none())
}

/// Whether the lower-case HTML tag name `name` is one of N3's blocks
/// ([`BLOCKS`]). A binary search, because the walk asks it for every node;
/// `BLOCKS` is sorted, which a unit test checks.
///
/// - `name`: an HTML tag name.
fn is_block(name: &str) -> bool {
    BLOCKS.binary_search(&name).is_ok()
}

/// Maps the `align` attribute of the paragraph `id` and strips its `lang`
/// attribute, and counts both.
///
/// - `html`: a parsed body.
/// - `id`: an HTML `p` element with an `align` or a `lang` attribute.
fn map_paragraph(html: &mut Html, id: NodeId, report: &mut NormaliseReport) {
    let (align, lang) = dom::edit_element(html, id, |paragraph| {
        let align = dom::remove_attribute(paragraph, "align").map(|value| {
            let outcome = paragraph_align(&value, paragraph.attr("style"));
            if let ParagraphAlign::Mapped { keyword, .. } = outcome {
                dom::prepend_declaration(paragraph, &format!("text-align: {keyword}"));
            }
            outcome
        });
        let lang = dom::remove_attribute(paragraph, "lang").is_some();
        (align, lang)
    })
    .expect("a paragraph is an element");
    match align {
        Some(ParagraphAlign::Mapped { unclear, .. }) => {
            report.record(Rule::ParagraphAlignMapped);
            if unclear {
                report.record(Rule::ParagraphAlignUnclear);
            }
        }
        Some(ParagraphAlign::Dropped) => report.record(Rule::ParagraphAlignDropped),
        None => {}
    }
    if lang {
        report.record(Rule::ParagraphLangStripped);
    }
}

/// What N5 does with a paragraph whose `align` attribute is `value` and
/// whose `style` is `style` (see the module documentation).
///
/// - `value`: the `align` attribute's value.
/// - `style`: the paragraph's `style`, if it has one.
fn paragraph_align(value: &str, style: Option<&str>) -> ParagraphAlign {
    match (paragraph_keyword(value), style.and_then(style_alignment)) {
        // A `style` declaration wins over the attribute in a browser.
        (_, Some(Alignment::Own { .. })) | (None, _) => ParagraphAlign::Dropped,
        (Some(keyword), Some(Alignment::Unclear)) => ParagraphAlign::Mapped {
            keyword,
            unclear: true,
        },
        // `style_alignment` answers `None` when the style names no
        // `text-align`; it never answers `Inherited`, which would mean the
        // same.
        (Some(keyword), None | Some(Alignment::Inherited)) => ParagraphAlign::Mapped {
            keyword,
            unclear: false,
        },
    }
}

/// The `text-align` keyword that a paragraph's `align` attribute `value`
/// sets in a browser, in lower case, or `None` when a browser maps it to
/// nothing. The value is matched whole and ASCII case-insensitively, as
/// N3's reading of `align` matches it.
///
/// - `value`: the attribute's value.
fn paragraph_keyword(value: &str) -> Option<&'static str> {
    PARAGRAPH_ALIGNMENTS
        .into_iter()
        .find(|keyword| keyword.eq_ignore_ascii_case(value))
}

/// The `style` of an `h2` without its `font-style` declarations, when the
/// `style` makes the heading italic; `None` when it does not (see the
/// module documentation). An empty result means that nothing else was
/// left.
///
/// The declarations are cut out whole, so everything else stays byte for
/// byte, unreadable declarations included, for N7 to judge: a declaration
/// is the text up to and including the next top-level `;`, the unit that
/// the sanitiser's reader reads one at a time. Removing whole declarations
/// cannot change how the others read, because each one that is followed by
/// another ended at a top-level `;`, where no comment, string or bracket is
/// open (only the reader's limit of 32 declarations counts them; no legacy
/// heading comes near it). Leading white space and the `;` that would end
/// the result go, as N7 writes no trailing `;` either.
///
/// - `style`: the heading's `style`, with character references decoded.
fn without_italic(style: &str) -> Option<String> {
    // The last applied declaration of each importance: an important one
    // wins over any other, whatever their order.
    let (mut normal, mut important) = (None, None);
    let mut kept = Vec::new();
    for declaration in declaration_texts(style) {
        match font_style(declaration.0) {
            FontStyle::Other => kept.push(declaration),
            FontStyle::Rejected => {}
            FontStyle::Applied {
                italic,
                important: true,
            } => important = Some(italic),
            FontStyle::Applied {
                italic,
                important: false,
            } => normal = Some(italic),
        }
    }
    if important.or(normal) != Some(true) {
        return None;
    }
    let mut out = String::new();
    if let Some((&(last, ended), rest)) = kept.split_last() {
        for (text, _) in rest {
            out.push_str(text);
        }
        // Only the `;` that ends the declaration: a last one that ends
        // nothing belongs to an open comment or string, or to an escape
        // (`\;`), and stays.
        out.push_str(if ended {
            last.strip_suffix(';').unwrap_or(last)
        } else {
            last
        });
    }
    let out = out.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if out.chars().all(|c| c.is_ascii_whitespace() || c == ';') {
        Some(String::new())
    } else {
        Some(out.to_owned())
    }
}

/// Splits `style` into its declarations as written, in order: each with
/// the top-level `;` that ends it, if one does, and whether one does.
/// Together they are `style`, byte for byte. A `;` inside a comment, a
/// string or a bracket ends nothing, as in the sanitiser's reader, which
/// reads the same pieces.
///
/// - `style`: a `style` attribute value.
fn declaration_texts(style: &str) -> Vec<(&str, bool)> {
    let mut input = Parser::new(style);
    let mut texts = Vec::new();
    loop {
        // White space and comments count as input here, so that nothing of
        // `style` is left out of the pieces.
        let state = input.state();
        if input.next_including_whitespace_and_comments().is_err() {
            break;
        }
        input.reset(&state);
        let start = input.position();
        let mut body_end = start;
        let parsed: Result<(), ParseError<()>> =
            input.parse_until_after(Delimiter::Semicolon, |declaration| {
                while declaration.next_including_whitespace_and_comments().is_ok() {}
                body_end = declaration.position();
                Ok(())
            });
        // The closure never fails, so neither does the parse.
        parsed.expect("a declaration is read to its end");
        let text = input.slice_from(start);
        let ended = input.position().byte_index() > body_end.byte_index();
        texts.push((text, ended));
    }
    texts
}

/// What the declaration `text` says about `font-style`, as the sanitiser's
/// reader reads it and a browser applies it.
///
/// - `text`: one declaration as [`declaration_texts`] cuts it.
fn font_style(text: &str) -> FontStyle {
    let read = declarations::read(text);
    let ([declaration], 0) = (read.read.as_slice(), read.unreadable) else {
        return FontStyle::Other;
    };
    if declaration.name != "font-style" {
        return FontStyle::Other;
    }
    let italic = match declaration.value.as_slice() {
        [Component::Ident(keyword)] => match keyword.as_str() {
            "italic" => true,
            "normal" | "oblique" | "inherit" | "initial" | "unset" | "revert" | "revert-layer" => {
                false
            }
            _ => return FontStyle::Rejected,
        },
        // `oblique <angle>`, with an angle from -90deg to 90deg (CSS Fonts
        // 4, §3.4).
        [
            Component::Ident(keyword),
            Component::Dimension { value, unit },
        ] if keyword == "oblique" => match degrees(*value, unit) {
            Some(angle) if (-90.0..=90.0).contains(&angle) => false,
            _ => return FontStyle::Rejected,
        },
        _ => return FontStyle::Rejected,
    };
    FontStyle::Applied {
        italic,
        important: is_important(text),
    }
}

/// Whether the declaration `text` is marked `!important`. The sanitiser's
/// reader strips the marker without saying so. It reads a `!` only there,
/// followed by `important` and the end of the declaration, and it reads no
/// declaration that holds a comment, so in a declaration it reads, any `!`
/// at the top level is the marker.
///
/// - `text`: one declaration as [`declaration_texts`] cuts it, one that the
///   sanitiser's reader reads.
fn is_important(text: &str) -> bool {
    let mut input = Parser::new(text);
    // `next` skips white space, and the content of a function or a bracket
    // as a whole, so only top-level tokens are seen.
    while let Ok(token) = input.next() {
        if matches!(token, Token::Delim('!')) {
            return true;
        }
    }
    false
}

/// The angle `value` `unit` in degrees, or `None` when `unit` is not an
/// angle unit (CSS Values 4, §7.1).
///
/// - `value`: the number.
/// - `unit`: its unit, in lower case.
fn degrees(value: f64, unit: &str) -> Option<f64> {
    match unit {
        "deg" => Some(value),
        "grad" => Some(value * 0.9),
        "rad" => Some(value.to_degrees()),
        "turn" => Some(value * 360.0),
        _ => None,
    }
}

/// Gives the italic heading `heading` its `style` without the `font-style`
/// declarations (or none, when nothing else is left) and, when the plan
/// says so, wraps its content in one `em` (see the module documentation).
/// The `em` and `i` inside it are the plan's to unwrap.
///
/// - `html`: a parsed body.
/// - `heading`: an italic `h2` of the plan.
fn emphasise_heading(html: &mut Html, heading: &ItalicHeading) {
    let id = heading.id;
    dom::edit_element(html, id, |element| {
        if heading.style.is_empty() {
            dom::remove_attribute(element, "style");
        } else {
            dom::set_attribute(element, "style", &heading.style);
        }
    });
    if !heading.em {
        return;
    }
    let children = dom::children(html, id);
    let content = |child: &NodeId| match dom::node(html, *child).value() {
        Node::Text(text) => !dom::is_white_space(text),
        Node::Comment(_) => false,
        _ => true,
    };
    let (Some(first), Some(last)) = (
        children.iter().position(content),
        children.iter().rposition(content),
    ) else {
        return;
    };
    dom::wrap(
        html,
        &children[first..=last],
        dom::new_element(local_name!("em"), &[]),
    );
}

#[cfg(test)]
mod tests {
    use super::super::center::align_attribute;
    use super::super::{Normalised, normalise};
    use super::*;
    use crate::domain::rich_text::search_text::html_to_search_text;
    use crate::domain::rich_text::{SanitizePolicy, Sanitizer};
    use proptest::prelude::*;

    /// N5's block rules alone over `body`: the edited tree and the report.
    fn n5_tree(body: &str) -> (Html, NormaliseReport) {
        let mut html = dom::parse(body);
        let mut report = NormaliseReport::default();
        map_blocks(&mut html, &mut report);
        (html, report)
    }

    /// N5's block rules alone over `body`: the output and the report.
    fn n5(body: &str) -> (String, NormaliseReport) {
        let (html, report) = n5_tree(body);
        (dom::serialise(&html), report)
    }

    /// The text nodes of `html`, in document order.
    fn text_of(html: &Html) -> String {
        html.root_element()
            .descendants()
            .filter_map(|node| node.value().as_text().map(|text| text.to_string()))
            .collect()
    }

    /// The whole normaliser over `body`.
    fn normalised(body: &str) -> Normalised {
        normalise(body).expect("a body that settles")
    }

    /// `body` as the parse and the writer alone leave it.
    fn written(body: &str) -> String {
        dom::serialise(&dom::parse(body))
    }

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy")
    }

    #[test]
    fn a_paragraph_alignment_becomes_text_align() {
        let (out, report) = n5(concat!(
            r#"<p align="center">a</p><p align="RIGHT">b</p><P ALIGN="Justify">c</P>"#,
            r#"<p align="left" style="color: red">d</p>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<p style="text-align: center">a</p><p style="text-align: right">b</p>"#,
                r#"<p style="text-align: justify">c</p>"#,
                r#"<p style="text-align: left; color: red">d</p>"#,
            )
        );
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::ParagraphAlignMapped, 4)]
        );
    }

    #[test]
    fn an_alignment_that_shows_nothing_is_dropped() {
        // A browser maps no other value on a paragraph, and an own
        // `text-align` in the style wins over the attribute.
        for (body, expected) in [
            (r#"<p align="middle">a</p>"#, "<p>a</p>"),
            (r#"<p align=" left ">a</p>"#, "<p>a</p>"),
            (r#"<p align="">a</p>"#, "<p>a</p>"),
            (
                r#"<p align="right" style="text-align: left">a</p>"#,
                r#"<p style="text-align: left">a</p>"#,
            ),
            (
                r#"<p align="justify" style="TEXT-ALIGN: Center">a</p>"#,
                r#"<p style="TEXT-ALIGN: Center">a</p>"#,
            ),
        ] {
            let (out, report) = n5(body);
            assert_eq!(out, expected, "{body}");
            assert_eq!(
                report.hits().collect::<Vec<_>>(),
                [(Rule::ParagraphAlignDropped, 1)],
                "{body}"
            );
        }
    }

    #[test]
    fn an_unclear_style_keeps_the_attribute_alignment_and_is_counted() {
        for style in [
            "text-align: inherit",
            "text-align: start",
            "text-align: bogus; text-align: left",
            "text-align: /* x */ left",
        ] {
            let (out, report) = n5(&format!(r#"<p align="right" style="{style}">a</p>"#));
            assert_eq!(
                out,
                format!(r#"<p style="text-align: right; {style}">a</p>"#),
                "{style}"
            );
            assert_eq!(
                report.hits().collect::<Vec<_>>(),
                [
                    (Rule::ParagraphAlignMapped, 1),
                    (Rule::ParagraphAlignUnclear, 1)
                ],
                "{style}"
            );
        }
        // An unclear style beside a value that maps to nothing: only the
        // attribute goes.
        let (out, report) = n5(r#"<p align="middle" style="text-align: inherit">a</p>"#);
        assert_eq!(out, r#"<p style="text-align: inherit">a</p>"#);
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::ParagraphAlignDropped, 1)]
        );
    }

    #[test]
    fn the_paragraph_decision_follows_the_style() {
        let mapped = |keyword, unclear| ParagraphAlign::Mapped { keyword, unclear };
        assert_eq!(paragraph_align("CENTER", None), mapped("center", false));
        assert_eq!(paragraph_align("left", Some("")), mapped("left", false));
        assert_eq!(
            paragraph_align("justify", Some("color: red")),
            mapped("justify", false)
        );
        assert_eq!(
            paragraph_align("right", Some("text-align: start")),
            mapped("right", true)
        );
        assert_eq!(
            paragraph_align("right", Some("text-align: right")),
            ParagraphAlign::Dropped
        );
        assert_eq!(paragraph_align("absmiddle", None), ParagraphAlign::Dropped);
        assert_eq!(paragraph_keyword("Left"), Some("left"));
        assert_eq!(paragraph_keyword("leftx"), None);
    }

    #[test]
    fn a_paragraph_loses_its_language() {
        let (out, report) = n5(concat!(
            r#"<p lang="en">English</p><p lang="hu-HU" style="text-align: justify">Magyar</p>"#,
            r#"<p lang="en" align="center">Both</p><span lang="en">kept</span>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<p>English</p><p style="text-align: justify">Magyar</p>"#,
                r#"<p style="text-align: center">Both</p><span lang="en">kept</span>"#,
            )
        );
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [
                (Rule::ParagraphAlignMapped, 1),
                (Rule::ParagraphLangStripped, 3)
            ]
        );
        // `xml:lang` is another attribute, for N7.
        let (out, report) = n5(r#"<p xml:lang="en">x</p>"#);
        assert_eq!(out, r#"<p xml:lang="en">x</p>"#);
        assert!(!report.has_hits());
    }

    #[test]
    fn a_table_loses_its_alignment_whatever_it_was() {
        let (out, report) = n5(concat!(
            r#"<table align="center" border="1"><tr><td align="right">a</td></tr></table>"#,
            r#"<table align="left" style="width: 500px"><tr><td>b</td></tr></table>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<table border="1"><tbody><tr><td align="right">a</td></tr></tbody></table>"#,
                r#"<table style="width: 500px"><tbody><tr><td>b</td></tr></tbody></table>"#,
            ),
            "a cell's align is the table layout's, not this rule's"
        );
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::TableAlignDropped, 2)]
        );
    }

    #[test]
    fn minor_headings_become_h4_with_their_attributes() {
        let (out, report) = n5(concat!(
            "<h5>Alcím</h5>",
            r#"<H6 STYLE="text-align: center" id="x">Kisebb <strong>cím</strong></H6>"#,
            "<h4>Marad</h4>",
        ));
        assert_eq!(
            out,
            concat!(
                "<h4>Alcím</h4>",
                r#"<h4 style="text-align: center" id="x">Kisebb <strong>cím</strong></h4>"#,
                "<h4>Marad</h4>",
            )
        );
        assert_eq!(report.hits().collect::<Vec<_>>(), [(Rule::HeadingToH4, 2)]);
    }

    #[test]
    fn an_italic_heading_wraps_its_content_in_em() {
        for (body, expected) in [
            (
                r#"<h2 style="font-style:italic">Cím</h2>"#,
                "<h2><em>Cím</em></h2>",
            ),
            (
                r#"<h2 style="font-style: italic; text-align: center">Cím <strong>vastag</strong></h2>"#,
                r#"<h2 style="text-align: center"><em>Cím <strong>vastag</strong></em></h2>"#,
            ),
            (
                r#"<h2 style="text-align: center; FONT-STYLE: Italic !important;">Cím</h2>"#,
                r#"<h2 style="text-align: center"><em>Cím</em></h2>"#,
            ),
            // The important declaration wins, as in a browser.
            (
                r#"<h2 style="font-style: italic !important; color: red; font-style: normal">Cím</h2>"#,
                r#"<h2 style="color: red"><em>Cím</em></h2>"#,
            ),
            // White space and comments at either end stay outside, as N3's
            // runs leave them; the editor's whitespace rule is N5b's.
            (
                "<h2 style=\"font-style: italic\">\n<!-- c --> Cím <br> <!-- d -->\n</h2>",
                "<h2>\n<!-- c --><em> Cím <br></em> <!-- d -->\n</h2>",
            ),
            // Italic inside italic: one `em`.
            (
                r#"<h2 style="font-style: italic"><em>Egy</em> <i>kettő</i> <a href="/x"><em>három</em></a></h2>"#,
                r#"<h2><em>Egy kettő <a href="/x">három</a></em></h2>"#,
            ),
            // The `em` goes in before the `i` goes, so what the `i` held at
            // its start stays in italic.
            (
                "<h2 style=\"font-style: italic\"><i><!-- c --> Cím</i>\n</h2>",
                "<h2><em><!-- c --> Cím</em>\n</h2>",
            ),
            // Nothing to wrap: only the style goes.
            (r#"<h2 style="font-style: italic"> </h2>"#, "<h2> </h2>"),
        ] {
            let (out, report) = n5(body);
            assert_eq!(out, expected, "{body}");
            assert_eq!(
                report.hits().collect::<Vec<_>>(),
                [(Rule::ItalicHeadingToEm, 1)],
                "{body}"
            );
        }
    }

    #[test]
    fn nested_italic_headings_end_with_one_em() {
        // Tag soup: the `div` keeps the parse from closing the outer heading
        // at the inner one's start tag.
        let (out, report) = n5(concat!(
            r#"<h2 style="font-style: italic">Külső<div>"#,
            r#"<h2 style="font-style:italic">Belső <i>dőlt</i></h2></div></h2>"#,
        ));
        assert_eq!(out, "<h2><em>Külső<div><h2>Belső dőlt</h2></div></em></h2>");
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::ItalicHeadingToEm, 2)]
        );
    }

    #[test]
    fn an_italic_element_around_a_block_stays() {
        // Tag soup, not a mark: only the `em` and `i` that hold inline
        // content alone are unwrapped, at any depth.
        let (out, report) = n5(concat!(
            r#"<h2 style="font-style: italic"><em>a<div>b</div></em> "#,
            "<i><strong>c<em>d</em></strong></i> <em><span><ul><li><i>e</i></li></ul></span></em></h2>",
        ));
        assert_eq!(
            out,
            concat!(
                "<h2><em><em>a<div>b</div></em> <strong>cd</strong> ",
                "<em><span><ul><li>e</li></ul></span></em></em></h2>",
            )
        );
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::ItalicHeadingToEm, 1)]
        );
    }

    #[test]
    fn an_italic_element_with_attributes_stays() {
        // Each of them says more than italic, or something else: they stay
        // for N5's inline rules and N7, which report what they remove. Only
        // the bare `em` goes.
        let body = concat!(
            r#"<h2 style="font-style:italic"><i style="font-style: normal">álló</i> "#,
            r#"<em class="k" id="a">x</em> <i class="fa fa-star"></i> <em>y</em></h2>"#,
        );
        let (out, report) = n5(body);
        assert_eq!(
            out,
            concat!(
                r#"<h2><em><i style="font-style: normal">álló</i> "#,
                r#"<em class="k" id="a">x</em> <i class="fa fa-star"></i> y</em></h2>"#,
            )
        );
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::ItalicHeadingToEm, 1)]
        );
        // The output is a fixed point of the normaliser too.
        let once = normalised(body);
        assert_eq!(normalised(&once.html).html, once.html);
    }

    #[test]
    fn a_known_soup_is_refused_with_the_em() {
        // The link in the row is fostered out of the table into the open
        // link around the italic heading. Without N5 the next parse closes
        // the italic heading at the fostered `h6` while it moves the links,
        // and the body settles in three passes. With the `em` around the
        // heading's content, the `h4` stays inside it while the links move,
        // which leaves the italic heading directly in the outer one; only
        // the parse after that closes the outer heading at its start tag,
        // and the fourth pass would be the first to return its input. See
        // "Fixed point" in the module documentation.
        let body = concat!(
            r#"<h2><a href="/x"><h2 style="font-style:italic"><table><h6><tr>"#,
            r#"<a href="/x">"#,
        );
        assert_eq!(
            normalise(body),
            Err(super::super::NormaliseError::NotNormalisable { passes: 3 })
        );
        let twin = body.replace(r#" style="font-style:italic""#, "");
        assert!(normalise(&twin).is_ok(), "settles without the italic rule");
    }

    #[test]
    fn every_shape_has_a_twin_the_rules_leave_alone() {
        for (shape, twin) in TWINS {
            assert!(n5(shape).1.has_hits(), "{shape} is a shape");
            assert!(!n5(twin).1.has_hits(), "{twin} is no shape");
            assert_eq!(
                dom::elements_named(&dom::parse(shape), "p").len(),
                dom::elements_named(&dom::parse(twin), "p").len(),
                "{shape} and {twin} build the same paragraphs"
            );
        }
        for piece in SHAPES.iter().chain(MOVING).chain(OTHER) {
            assert_eq!(
                n5(piece).1.has_hits(),
                TWINS.iter().any(|(shape, _)| shape == piece),
                "{piece}"
            );
        }
    }

    #[test]
    fn a_heading_nested_by_tag_soup_settles() {
        // A link left open around nested headings: the parse moves the
        // inner link out of the outer one, taking the inner `h5` with it,
        // and only the `em` around that `h5` keeps the next parse from
        // closing the outer `h5` at its start tag. Unwrapped, the body
        // changed in every pass and was refused.
        let body = concat!(
            r#"<a href="/x"><h5><center><ol><li><h2 style="font-style:italic"><div>"#,
            r#"<blockquote><h5><em><h5><a href="/x">"#,
        );
        let out = normalise(body).expect("the body settles");
        let again = normalise(&out.html).expect("the output settles");
        assert_eq!(again.html, out.html);
        assert!(!again.report.has_hits(), "{:?}", again.report);
        assert!(
            out.html
                .contains(r#"<em><h4><a href="/x"></a><a href="/x"></a></h4></em>"#),
            "{}",
            out.html
        );
        assert_eq!(out.report.count(Rule::ItalicHeadingToEm), 1);
    }

    #[test]
    fn only_an_italic_h2_is_converted() {
        for body in [
            r#"<h2 style="font-style: normal">a</h2>"#,
            r#"<h2 style="font-style: italic; font-style: normal">a</h2>"#,
            r#"<h2 style="font-style: normal !important; font-style: italic">a</h2>"#,
            r#"<h2 style="font-style: oblique">a</h2>"#,
            r#"<h2 style="font-style: inherit">a</h2>"#,
            r#"<h2 style="font-style: /* c */ italic">a</h2>"#,
            r#"<h2 style="font: italic 12px serif">a</h2>"#,
            r"<h2>a</h2>",
            r#"<h3 style="font-style: italic">a</h3>"#,
            r#"<p style="font-style: italic">a</p>"#,
        ] {
            let (out, report) = n5(body);
            assert_eq!(out, written(body), "{body}");
            assert!(!report.has_hits(), "{body}");
        }
    }

    #[test]
    fn the_browser_cascade_decides() {
        for (style, expected) in [
            ("font-style: italic", Some("")),
            ("font-style: normal; font-style: italic", Some("")),
            // An important declaration wins over a later one that is not,
            // and among important ones the last applied one wins.
            (
                "font-style: italic !important; font-style: normal",
                Some(""),
            ),
            ("font-style: normal !important; font-style: italic", None),
            (
                "font-style: normal !important; font-style: italic ! IMPORTANT",
                Some(""),
            ),
            (
                "font-style: italic !important; font-style: normal !important",
                None,
            ),
            (
                "font-style: italic !important; font-style: bogus !important",
                Some(""),
            ),
            ("font-style: italic; font-style: inherit !important", None),
            ("font-style: italic; font-style: bogus", Some("")),
            ("font-style: italic; font-style: oblique 100deg", Some("")),
            ("font-style: italic; font-style: oblique 1px", Some("")),
            ("font-style: italic; font-style: italic bold", Some("")),
            ("font-style: italic; font-style: oblique", None),
            ("font-style: italic; font-style: oblique 10deg", None),
            ("font-style: italic; font-style: oblique -0.25turn", None),
            ("font-style: italic; font-style: oblique 1rad", None),
            ("font-style: italic; font-style: oblique 100grad", None),
            ("font-style: italic; font-style: revert-layer", None),
            ("font-style: italic; font-style: initial", None),
            ("color: red", None),
            ("", None),
        ] {
            assert_eq!(without_italic(style).as_deref(), expected, "{style}");
        }
    }

    #[test]
    fn the_important_marker_is_read_from_the_declaration() {
        for (text, expected) in [
            ("font-style: italic !important", true),
            ("font-style: italic!IMPORTANT;", true),
            ("font-style: italic ! important ; ", true),
            ("font-style: italic", false),
            ("font-style: italic;", false),
            (" font-style : oblique 10deg ;", false),
        ] {
            assert_eq!(is_important(text), expected, "{text}");
        }
    }

    #[test]
    fn n3s_blocks_are_sorted_for_the_binary_search() {
        assert!(BLOCKS.windows(2).all(|pair| pair[0] < pair[1]));
        for name in BLOCKS {
            assert!(is_block(name), "{name}");
        }
        for name in ["em", "i", "span", "a", "strong", "", "H2", "h7"] {
            assert!(!is_block(name), "{name}");
        }
    }

    #[test]
    fn the_plan_names_every_edit_once() {
        let html = dom::parse(concat!(
            r#"<p align="center" lang="en">a</p><table align="left"><tr><td>b</td></tr></table>"#,
            r#"<h6>c</h6><h2 style="font-style: italic; color: red"><em>d</em><div>"#,
            r#"<h2 style="font-style: italic"><i>e</i></h2><h2 style="font-style: italic">"#,
            "<ul><li>f</li></ul></h2></div></h2>",
        ));
        let found = plan(&html);
        let names = |ids: &[NodeId]| {
            ids.iter()
                .map(|id| dom::html_name(dom::node(&html, *id)).map(str::to_owned))
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&found.paragraphs), [Some("p".to_owned())]);
        assert_eq!(names(&found.tables), [Some("table".to_owned())]);
        assert_eq!(names(&found.minor_headings), [Some("h6".to_owned())]);
        // The outer heading gets its `em`; the first inner one holds no
        // block and shares it, the second holds a list and keeps its own.
        assert_eq!(
            found
                .italic_headings
                .iter()
                .map(|heading| (heading.style.as_str(), heading.em))
                .collect::<Vec<_>>(),
            [("color: red", true), ("", false), ("", true)]
        );
        // The `em` and the `i`, in the order their end tags are written.
        assert_eq!(
            names(&found.italics),
            [Some("em".to_owned()), Some("i".to_owned())]
        );
    }

    #[test]
    fn the_other_declarations_stay_as_written() {
        for (style, expected) in [
            ("font-style: italic; text-align:center", "text-align:center"),
            (
                "text-align: center; font-style: italic",
                "text-align: center",
            ),
            (
                "text-align: center; font-style: italic;",
                "text-align: center",
            ),
            (
                "  color: red ; font-style: italic ;width: 1cm;",
                "color: red ;width: 1cm",
            ),
            ("font-style: italic;;", ""),
            (" ; font-style: italic ; ", ""),
            // A `;` inside brackets, a string or a comment ends nothing.
            (
                "background: url(a;b); font-style: italic",
                "background: url(a;b)",
            ),
            (
                "content: \"a;b\"; font-style: italic; color: red",
                "content: \"a;b\"; color: red",
            ),
            (
                "font-style: italic; color: red; /* x; y",
                "color: red; /* x; y",
            ),
            // An escaped `;` belongs to its declaration.
            ("font-style: italic; a: b\\;c", "a: b\\;c"),
        ] {
            assert_eq!(without_italic(style).as_deref(), Some(expected), "{style}");
        }
    }

    #[test]
    fn declarations_are_cut_as_written() {
        assert_eq!(
            declaration_texts("a: b; c: (d; e); /* f; */ g"),
            [
                ("a: b;", true),
                (" c: (d; e);", true),
                (" /* f; */ g", false)
            ]
        );
        assert_eq!(declaration_texts(""), []);
        assert_eq!(declaration_texts(";"), [(";", true)]);
        assert_eq!(declaration_texts("  "), [("  ", false)]);
        assert_eq!(
            declaration_texts("a: b;  "),
            [("a: b;", true), ("  ", false)]
        );
    }

    #[test]
    fn angles_convert_to_degrees() {
        assert_eq!(degrees(45.0, "deg"), Some(45.0));
        assert_eq!(degrees(100.0, "grad"), Some(90.0));
        assert_eq!(degrees(0.25, "turn"), Some(90.0));
        assert!(degrees(std::f64::consts::PI, "rad").is_some_and(|d| (d - 180.0).abs() < 1e-9));
        assert_eq!(degrees(1.0, "px"), None);
    }

    #[test]
    fn the_rules_follow_n3_in_one_pass() {
        // Inside a `center`, a paragraph's `align` is its own alignment, so
        // N3 leaves it and N5 writes it, whether it centres or not; an `h5`
        // is a heading N3 aligns, and keeps that alignment as an `h4`.
        let out = normalised(concat!(
            r#"<center><p align="justify">Sorkizárt</p><p align="center">Középen</p>"#,
            "<p>Középre</p></center><center><h5>Alcím</h5></center>",
        ));
        assert_eq!(
            out.html,
            concat!(
                r#"<p style="text-align: justify">Sorkizárt</p><p style="text-align: center">Középen</p>"#,
                r#"<p style="text-align: center">Középre</p><h4 style="text-align: center">Alcím</h4>"#,
            )
        );
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [
                (Rule::CenterUnwrapped, 2),
                (Rule::CenterBlockAligned, 2),
                (Rule::ParagraphAlignMapped, 2),
                (Rule::HeadingToH4, 1),
            ]
        );
        // A `p` holds no `center`: the parse closes the paragraph at its
        // start tag, so the `align` stays on an empty paragraph.
        let out = normalised(r#"<p align="left"><center>Középre</center></p>"#);
        assert_eq!(
            out.html,
            r#"<p style="text-align: left"></p><p style="text-align: center">Középre</p><p></p>"#
        );
        // A `center` that is an italic heading's only content aligns the
        // heading first, so the `em` goes around the text. Run the other
        // way round, the `em` would stand around the `center`, an inline
        // element around a block, whose centring N3 drops.
        let out = normalised(r#"<h2 style="font-style:italic"><center>Dőlt cím</center></h2>"#);
        assert_eq!(
            out.html,
            r#"<h2 style="text-align: center"><em>Dőlt cím</em></h2>"#
        );
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [
                (Rule::CenterUnwrapped, 1),
                (Rule::CenterBlockAligned, 1),
                (Rule::ItalicHeadingToEm, 1),
            ]
        );
        // Inside a `center`, a `style` whose `text-align` content-v1 does
        // not keep shows the centre's alignment: N3 centres the paragraph
        // and N5 writes the attribute's in front, so N7 keeps the last one,
        // N3's, and both count the paragraph as unclear for review.
        let out = normalised(
            r#"<center><p align="right" style="text-align: inherit">Örökölt</p></center>"#,
        );
        assert_eq!(
            out.html,
            r#"<p style="text-align: right; text-align: center; text-align: inherit">Örökölt</p>"#
        );
        assert_eq!(
            out.report.hits().collect::<Vec<_>>(),
            [
                (Rule::CenterUnwrapped, 1),
                (Rule::CenterBlockAligned, 1),
                (Rule::CenterAlignmentUnclear, 1),
                (Rule::ParagraphAlignMapped, 1),
                (Rule::ParagraphAlignUnclear, 1),
            ]
        );
        let clean = sanitizer().sanitize(&out.html).expect("a small body").html;
        assert_eq!(
            clean.as_str(),
            r#"<p style="text-align: center">Örökölt</p>"#
        );
    }

    #[test]
    fn the_outputs_are_content_v1_and_settle() {
        let sanitizer = sanitizer();
        for body in [
            r#"<p align="center">K&ouml;z&eacute;pen</p>"#,
            r#"<p lang="en" align="right">Right</p>"#,
            r"<h6>Kicsi</h6>",
            r#"<h2 style="font-style:italic">D&#337;lt <i>c&iacute;m</i></h2>"#,
        ] {
            let out = normalised(body);
            assert_eq!(
                super::super::check_fixed_point(&out.html, &sanitizer),
                Ok(()),
                "{body}: {}",
                out.html
            );
        }
    }

    /// The shapes N5's block rules look for.
    const SHAPES: &[&str] = &[
        "<p align=\"center\">",
        "<P ALIGN=\"JUSTIFY\">",
        "<p align=\"middle\">",
        "<p align=\"left\" style=\"text-align: right\">",
        "<p align=\"right\" style=\"text-align: inherit\">",
        "<p align=\"right\" style=\"color: red; /*\">",
        "<p lang=\"en\">",
        "<p lang=\"en\" align=\"right\">",
        "<h5>",
        "<H6 style=\"text-align: center\">",
        "<h2 style=\"font-style:italic\">",
        "<h2 style=\"font-style: italic; text-align: center\">",
        "<h2 style=\"font-style: italic !important; font-style: normal\">",
    ];

    /// Pieces the rules do not look for: the N3 shapes they meet, the
    /// markup around them, and text.
    const OTHER: &[&str] = &[
        "<p>",
        "</p>",
        "</h5>",
        "</h6>",
        "<h2 style=\"font-style: normal\">",
        "<h2 style=\"font-style: normal !important; font-style: italic\">",
        "<h2>",
        "</h2>",
        "<h4>",
        "<em>",
        "</em>",
        "<i>",
        "</i>",
        "<em class=\"k\">",
        "<i style=\"font-style: normal\">",
        "<strong>",
        "</strong>",
        "<a href=\"/x\">",
        "</a>",
        "<center>",
        "</center>",
        "<ul><li>",
        "</li></ul>",
        "<div>",
        "</div>",
        "<br>",
        "<!-- c -->",
        "szöveg",
        "&eacute;",
        " ",
        "\n",
    ];

    /// Tables, whose parse moves text out of them, and foreign content.
    const MOVING: &[&str] = &[
        "<table align=\"center\" border=\"1\"><tr><td>",
        "<table align=\"left\"><tr><td align=\"right\">",
        "<table><tr><td>",
        "</td></tr></table>",
        "<svg><p align=\"center\">",
        "<svg><h2 style=\"font-style: italic\">",
        "</svg>",
    ];

    /// Bodies of up to 32 pieces from `lists`, as their pieces.
    fn arb_pieces(lists: &[&[&'static str]]) -> impl Strategy<Value = Vec<&'static str>> {
        let pieces: Vec<&'static str> = lists.concat();
        prop::collection::vec(prop::sample::select(pieces), 0..32)
    }

    /// Bodies of up to 32 pieces from `lists`.
    fn arb_from(lists: &[&[&'static str]]) -> impl Strategy<Value = String> {
        arb_pieces(lists).prop_map(|pieces| pieces.concat())
    }

    /// Each piece that N5's block rules look for, with a twin that they do
    /// not look for and that parses the same way: without the `align`,
    /// `lang` or italic `font-style`, and with `h4` for `h5` and `h6`, which the
    /// parser treats alike. N3 may style a twin paragraph differently, but
    /// it builds the same elements.
    const TWINS: &[(&str, &str)] = &[
        ("<p align=\"center\">", "<p>"),
        ("<P ALIGN=\"JUSTIFY\">", "<P>"),
        ("<p align=\"middle\">", "<p>"),
        (
            "<p align=\"left\" style=\"text-align: right\">",
            "<p style=\"text-align: right\">",
        ),
        (
            "<p align=\"right\" style=\"text-align: inherit\">",
            "<p style=\"text-align: inherit\">",
        ),
        (
            "<p align=\"right\" style=\"color: red; /*\">",
            "<p style=\"color: red; /*\">",
        ),
        ("<p lang=\"en\">", "<p>"),
        ("<p lang=\"en\" align=\"right\">", "<p>"),
        ("<h5>", "<h4>"),
        (
            "<H6 style=\"text-align: center\">",
            "<h4 style=\"text-align: center\">",
        ),
        ("<h2 style=\"font-style:italic\">", "<h2>"),
        (
            "<h2 style=\"font-style: italic; text-align: center\">",
            "<h2 style=\"text-align: center\">",
        ),
        (
            "<h2 style=\"font-style: italic !important; font-style: normal\">",
            "<h2 style=\"font-style: normal\">",
        ),
        (
            "<table align=\"center\" border=\"1\"><tr><td>",
            "<table border=\"1\"><tr><td>",
        ),
        (
            "<table align=\"left\"><tr><td align=\"right\">",
            "<table><tr><td align=\"right\">",
        ),
        // `p` and `h2` break out of `svg`: the parse closes it at their start
        // tags, so both are HTML elements the rules see.
        ("<svg><p align=\"center\">", "<svg><p>"),
        ("<svg><h2 style=\"font-style: italic\">", "<svg><h2>"),
    ];

    /// The body of `pieces` with every piece that N5's block rules look for
    /// replaced by its twin from [`TWINS`].
    fn twin_body(pieces: &[&str]) -> String {
        pieces
            .iter()
            .map(|piece| {
                TWINS
                    .iter()
                    .find(|(shape, _)| shape == piece)
                    .map_or(*piece, |(_, twin)| twin)
            })
            .collect()
    }

    /// Bodies built from the shapes N5's block rules tell apart, with the
    /// N3 shapes they meet and tag soup around them.
    fn arb_body() -> impl Strategy<Value = String> {
        arb_from(&[SHAPES, OTHER, MOVING])
    }

    /// The same without tables and foreign content, for the properties
    /// about the text through the writer and the next reader.
    fn arb_plain_body() -> impl Strategy<Value = String> {
        arb_from(&[SHAPES, OTHER])
    }

    /// Bodies without the shapes.
    fn arb_neutral_body() -> impl Strategy<Value = String> {
        arb_from(&[OTHER])
    }

    /// The elements of `html` that N5's block rules look for.
    fn leftovers(html: &Html) -> Vec<String> {
        html.root_element()
            .descendants()
            .filter_map(|node| {
                let element = node.value().as_element()?;
                let left = match dom::html_name(node)? {
                    "p" => element.attr("align").is_some() || element.attr("lang").is_some(),
                    "table" => element.attr("align").is_some(),
                    "h5" | "h6" => true,
                    "h2" => element.attr("style").and_then(without_italic).is_some(),
                    _ => false,
                };
                left.then(|| format!("{element:?}"))
            })
            .collect()
    }

    /// At least 512 cases, as the normaliser's own P4 runs, or more when
    /// `PROPTEST_CASES` asks for more.
    fn config() -> ProptestConfig {
        let default = ProptestConfig::default();
        ProptestConfig {
            cases: default.cases.max(512),
            ..default
        }
    }

    /// Styles built from declarations that tell the `font-style` cases
    /// apart, and from pieces the reader cannot read; at most 16, below the
    /// reader's limit of 32.
    fn arb_style() -> impl Strategy<Value = String> {
        let pieces = prop::sample::select(
            &[
                "font-style: italic",
                "FONT-STYLE:ITALIC",
                "font-style: italic !important",
                "font-style: normal !important",
                "font-style: italic ! IMPORTANT",
                "font-style: bogus !important",
                "font-style: normal",
                "font-style: oblique",
                "font-style: oblique 20deg",
                "font-style: oblique 120deg",
                "font-style: bogus",
                "font-style: inherit",
                "text-align: center",
                "text-align:left",
                "color: red",
                "width: 1cm",
                "background: url(a;b)",
                "content: \"x;y\"",
                "a: b\\;c",
                "/* c */",
                "/* open",
                "\"open",
                "(open",
                "",
                " ",
            ][..],
        );
        (
            prop::collection::vec(pieces, 0..16),
            prop::collection::vec(prop::sample::select(&[";", "; ", " ;", ";;"][..]), 16),
        )
            .prop_map(|(pieces, separators)| {
                pieces
                    .into_iter()
                    .zip(separators)
                    .flat_map(|(piece, separator)| [piece, separator])
                    .collect::<String>()
            })
    }

    /// The model of [`without_italic`]'s decision, read from the end: the
    /// last applied `font-style` declaration marked `!important` is
    /// `italic`, or, when none is marked, the last applied one.
    fn italic_by_model(style: &str) -> bool {
        let applied: Vec<(bool, bool)> = declaration_texts(style)
            .into_iter()
            .filter_map(|(text, _)| match font_style(text) {
                FontStyle::Applied { italic, important } => Some((italic, important)),
                FontStyle::Other | FontStyle::Rejected => None,
            })
            .collect();
        applied
            .iter()
            .rev()
            .find(|(_, important)| *important)
            .or(applied.last())
            .is_some_and(|(italic, _)| *italic)
    }

    proptest! {
        #![proptest_config(config())]

        // P4 (editor.md §13.6, data-migration.md §11.2) over the block
        // shapes, with N3 before them: normalising the output again returns
        // it, with no rule hit.
        #[test]
        fn normalising_twice_equals_normalising_once(body in arb_body()) {
            if let Ok(once) = normalise(&body) {
                let twice = normalise(&once.html)
                    .map_err(|error| TestCaseError::fail(format!("{body:?}: the output was refused: {error}")))?;
                prop_assert_eq!(&twice.html, &once.html, "{:?}", body);
                prop_assert!(!twice.report.has_hits(), "{:?}: {:?}", body, twice.report);
            }
        }

        // The block rules add no refusal: a body is refused only when its
        // twin, which they leave alone, is refused too. Some tag soup that
        // nests links needs more than three passes with or without N5 (the
        // documentation of `super`, "Fixed point"), so "never refused"
        // would not hold; one shape where N5 adds a refusal is pinned in
        // `a_known_soup_is_refused_with_the_em`.
        #[test]
        fn the_block_rules_add_no_refusal(pieces in arb_pieces(&[SHAPES, OTHER, MOVING])) {
            let body = pieces.concat();
            if normalise(&body).is_err() {
                let twin = twin_body(&pieces);
                prop_assert!(normalise(&twin).is_err(), "{:?} is refused, but not {:?}", body, twin);
            }
        }

        // The twins hold nothing for the rules.
        #[test]
        fn a_twin_holds_no_shape(pieces in arb_pieces(&[SHAPES, OTHER, MOVING])) {
            let (out, report) = n5(&twin_body(&pieces));
            prop_assert!(!report.has_hits(), "{:?}: {:?}", out, report);
        }

        // Nothing that N5's block rules look for is left, and every element
        // they found is counted: once, or twice for a paragraph that had
        // both an `align` and a `lang`.
        #[test]
        fn nothing_for_the_rules_is_left(body in arb_body()) {
            let (tree, report) = n5_tree(&body);
            prop_assert_eq!(leftovers(&tree), Vec::<String>::new(), "{:?}", body);
            let found = leftovers(&dom::parse(&body)).len();
            let hits = report.hits()
                .filter(|(rule, _)| *rule != Rule::ParagraphAlignUnclear)
                .map(|(_, count)| count)
                .sum::<usize>();
            prop_assert!(found <= hits && hits <= 2 * found, "{} hits for {} elements in {:?}", hits, found, body);
            prop_assert!(report.count(Rule::ParagraphAlignUnclear) <= report.count(Rule::ParagraphAlignMapped));
        }

        // V5(c): the text a reader sees is unchanged, in its order; the
        // rules move no text node.
        #[test]
        fn the_text_is_unchanged(body in arb_body()) {
            let (tree, _) = n5_tree(&body);
            prop_assert_eq!(text_of(&tree), text_of(&dom::parse(&body)), "{:?}", body);
        }

        // V5(c) through the writer and the next reader, as the search text
        // of ADR 0020 sees the stored body.
        #[test]
        fn the_search_text_is_unchanged(body in arb_plain_body()) {
            // A refused body is the property above's.
            if let Ok(out) = normalise(&body) {
                let words = |html: &str| html_to_search_text(html).split(' ').map(str::to_owned).collect::<Vec<_>>().concat();
                prop_assert_eq!(words(&out.html), words(&body), "{:?}", out.html);
            }
        }

        // Without the shapes, the rules change and count nothing.
        #[test]
        fn a_body_without_the_shapes_is_unchanged(body in arb_neutral_body()) {
            let (out, report) = n5(&body);
            prop_assert_eq!(out, written(&body));
            prop_assert!(!report.has_hits());
        }

        // A paragraph's `align` maps as N3 reads it: centred exactly where
        // N3 sees it centred, and to nothing where N3 sees nothing.
        #[test]
        fn the_paragraph_keyword_agrees_with_n3(value in "(?i)(left|right|center|justify|middle|absmiddle| left|[a-z]{0,8})") {
            prop_assert_eq!(
                paragraph_keyword(&value).map(|keyword| keyword == "center"),
                align_attribute("p", &value),
                "{:?}", value
            );
            if let Some(keyword) = paragraph_keyword(&value) {
                prop_assert!(keyword.eq_ignore_ascii_case(&value));
            }
        }

        // A mapped paragraph reads back with the attribute's alignment, or
        // as unclear when its old style was; a dropped one keeps its style.
        #[test]
        fn a_mapped_paragraph_reads_back_aligned(
            value in "(?i)(left|right|center|justify|middle)",
            style in proptest::option::of("[a-z: ;/*-]{0,24}"),
        ) {
            let attribute = style.as_deref().map(|style| format!(r#" style="{style}""#)).unwrap_or_default();
            let (out, _) = n5(&format!(r#"<p align="{value}"{attribute}>x</p>"#));
            let html = dom::parse(&out);
            let p = dom::elements_named(&html, "p")[0];
            let after = dom::node(&html, p).value().as_element().expect("a p").attr("style").map(str::to_owned);
            match paragraph_align(&value, style.as_deref()) {
                ParagraphAlign::Mapped { keyword, unclear } => {
                    let first = declarations::read(after.as_deref().expect("a style"))
                        .read
                        .into_iter()
                        .next()
                        .expect("a declaration");
                    prop_assert_eq!(first.name, "text-align");
                    prop_assert_eq!(first.value, vec![Component::Ident(keyword.to_owned())]);
                    prop_assert_eq!(unclear, style.as_deref().and_then(style_alignment) == Some(Alignment::Unclear));
                }
                // The generated style holds no character reference or
                // quote, so the attribute's value is the style itself.
                ParagraphAlign::Dropped => prop_assert_eq!(after, style),
            }
        }

        // The pieces are the whole style, each `;` where it ends one.
        #[test]
        fn the_pieces_are_the_whole_style(style in any::<String>()) {
            let texts = declaration_texts(&style);
            prop_assert_eq!(texts.iter().map(|(text, _)| *text).collect::<String>(), style.clone());
            for (index, (text, ended)) in texts.iter().enumerate() {
                if *ended {
                    prop_assert!(text.ends_with(';'), "{:?}", text);
                } else {
                    prop_assert_eq!(index, texts.len() - 1, "only the last one ends unended");
                }
            }
        }

        // The pieces read as the whole style reads: the same declarations,
        // in order, so a piece judged alone is judged as N7 judges it.
        #[test]
        fn the_pieces_read_as_the_style(style in arb_style()) {
            let whole = declarations::read(&style);
            let mut read = Vec::new();
            let mut unreadable = 0;
            for (text, _) in declaration_texts(&style) {
                let piece = declarations::read(text);
                read.extend(piece.read);
                unreadable += piece.unreadable;
            }
            prop_assert_eq!(read, whole.read);
            prop_assert_eq!(unreadable, whole.unreadable);
        }

        // An italic style loses every `font-style` declaration and keeps
        // every other one, in order; a style that is not italic is left.
        #[test]
        fn only_font_style_is_cut(style in arb_style()) {
            let before = declarations::read(&style);
            match without_italic(&style) {
                None => prop_assert!(!italic_by_model(&style), "{:?}", style),
                Some(rest) => {
                    prop_assert!(italic_by_model(&style), "{:?}", style);
                    let after = declarations::read(&rest);
                    let others: Vec<_> = before.read.into_iter().filter(|d| d.name != "font-style").collect();
                    prop_assert_eq!(after.read, others, "{:?} -> {:?}", style, rest);
                    prop_assert_eq!(after.unreadable, before.unreadable, "{:?} -> {:?}", style, rest);
                    prop_assert!(!rest.starts_with(|c: char| c.is_ascii_whitespace()), "{:?}", rest);
                    prop_assert!(without_italic(&rest).is_none(), "{:?}", rest);
                }
            }
        }

        // The one walk plans what the rule says node by node, whatever the
        // tree: an `em` or `i` is unwrapped exactly when it has no
        // attribute, holds no block and lies inside an italic heading, and
        // an italic heading gets an `em` of its own unless it holds no block
        // and lies inside another one. The model asks each node about its
        // whole subtree and its ancestors, with N3's list of blocks.
        #[test]
        fn the_plan_is_the_rule_node_by_node(body in arb_body()) {
            let html = dom::parse(&body);
            let found = plan(&html);
            let italic = |node: NodeRef<'_, Node>| {
                dom::html_name(node) == Some("h2")
                    && node
                        .value()
                        .as_element()
                        .and_then(|element| element.attr("style"))
                        .and_then(without_italic)
                        .is_some()
            };
            let holds_block = |node: NodeRef<'_, Node>| {
                node.descendants()
                    .skip(1)
                    .any(|below| dom::html_name(below).is_some_and(|name| BLOCKS.contains(&name)))
            };
            let inside_italic = |node: NodeRef<'_, Node>| node.ancestors().any(italic);
            let root = dom::node(&html, dom::content(&html));
            let mut expected: Vec<NodeId> = root
                .descendants()
                .filter(|node| {
                    matches!(dom::html_name(*node), Some("em" | "i"))
                        && node.value().as_element().is_some_and(|element| element.attrs().next().is_none())
                        && !holds_block(*node)
                        && inside_italic(*node)
                })
                .map(|node| node.id())
                .collect();
            let mut italics = found.italics.clone();
            italics.sort();
            italics.dedup();
            prop_assert_eq!(italics.len(), found.italics.len(), "each one once");
            expected.sort();
            prop_assert_eq!(italics, expected, "{:?}", body);
            let headings: Vec<(NodeId, bool)> = root
                .descendants()
                .filter(|node| italic(*node))
                .map(|node| (node.id(), holds_block(node) || !inside_italic(node)))
                .collect();
            prop_assert_eq!(
                found.italic_headings.iter().map(|heading| (heading.id, heading.em)).collect::<Vec<_>>(),
                headings,
                "{:?}", body
            );
        }

        // The binary search over N3's list of blocks answers as a scan of
        // it does.
        #[test]
        fn a_block_is_one_of_n3s_blocks(name in "(h[1-7]|t[a-z]|[a-z]{1,4}|plaintext|xmp|zzz)") {
            prop_assert_eq!(is_block(&name), BLOCKS.contains(&name.as_str()), "{:?}", name);
        }

        // `!important` is found however it is spelled, and the declaration
        // reads the same with or without it.
        #[test]
        fn the_marker_is_found_as_the_reader_strips_it(
            value in prop::sample::select(&["italic", "NORMAL", "oblique", "oblique 10deg", "inherit", "bogus"][..]),
            marker in prop::sample::select(&["", " ", "!important", " !important", "! IMPORTANT", " !\tImportant "][..]),
            end in prop::sample::select(&["", ";", " ; "][..]),
        ) {
            let text = format!("font-style: {value}{marker}{end}");
            let plain = format!("font-style: {value}{end}");
            let marked = marker.contains('!');
            match (font_style(&text), font_style(&plain)) {
                (
                    FontStyle::Applied { italic, important },
                    FontStyle::Applied { italic: unmarked, important: false },
                ) => {
                    prop_assert_eq!(italic, unmarked, "{:?}", text);
                    prop_assert_eq!(important, marked, "{:?}", text);
                }
                (read, unmarked) => prop_assert_eq!(read, unmarked, "{:?}", text),
            }
            prop_assert_eq!(is_important(&text), marked, "{:?}", text);
        }

        // The block rules leave the body's structure to N3 and N5b: every
        // element they edit is still there, the minor headings are `h4`
        // now, and the only new elements are the `em` of italic headings,
        // at most one each, which take in the `em` and `i` below them.
        #[test]
        fn only_an_em_is_added(body in arb_body()) {
            let before = dom::parse(&body);
            let (after, report) = n5_tree(&body);
            let count = |html: &Html, name: &str| dom::elements_named(html, name).len();
            for name in ["p", "table", "h2", "strong", "a", "td", "li", "div"] {
                prop_assert_eq!(count(&after, name), count(&before, name), "{} in {:?}", name, body);
            }
            prop_assert_eq!(count(&after, "h5") + count(&after, "h6"), 0);
            prop_assert_eq!(
                count(&after, "h4"),
                count(&before, "h4") + report.count(Rule::HeadingToH4),
                "{:?}", body
            );
            let italics = |html: &Html| count(html, "em") + count(html, "i");
            prop_assert!(
                italics(&after) <= italics(&before) + report.count(Rule::ItalicHeadingToEm),
                "{:?}", body
            );
        }
    }
}
