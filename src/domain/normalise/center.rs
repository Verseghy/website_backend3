//! N3: `<center>` is unwrapped into `text-align: center`
//! (data-migration.md §7.1 and §7.4, editor.md §9, ADR 0017 §2 step 3).
//!
//! content-v1 has no `center` element (editor.md §3.2), so the sanitiser
//! would unwrap it and lose the centring. The legacy bodies hold `center`
//! because it was typed in `CKEditor`'s source mode: around a `YouTube`
//! paragraph in 8 posts and inside a heading in one **\[500\]** (research:
//! gap-06 §2.2, §4.1, §6.1). N3 keeps what the reader saw, in the one form
//! content-v1 has for it: `text-align: center` on the blocks that carry
//! it, paragraphs, headings and table cells (`p`, `h1`-`h4`, `td`, `th`;
//! editor.md §3.3).
//!
//! A browser centres everything inside a `center` that does not set an
//! alignment of its own: it is expected to centre the text within itself
//! and to align its descendants to the centre (WHATWG HTML, "Rendering",
//! §15.3.3). So N3 gives that content the alignment explicitly, then
//! removes the `center` and lets its content take its place:
//!
//! - **An embed alone.** When the only thing a reader sees inside the
//!   `center` is an embed, the wrapper is just removed: content-v1's
//!   embed markup lays the player out by itself (§3.7). An embed is an
//!   `iframe`, an element with one of the embed markers
//!   (`data-youtube-video`, `data-padlet-embed`, or the legacy oEmbed
//!   wrapper's `data-oembed-url`), or a `p` or `div` that holds nothing
//!   visible but an embed, which is how the legacy bodies wrap a bare
//!   `YouTube` iframe. So the rule reads the same before and after N1 and N2
//!   have turned legacy embeds into the editor's nodes. An embed next to
//!   other content is kept as it is, for the same reason.
//! - **The block around it.** When the `center` is the only visible
//!   content of a paragraph, heading or cell (`<h2><center>Cím</center></h2>`,
//!   `<td><center>…</center></td>`), that block gets the alignment, unless
//!   it is centred already; no paragraph may stand inside a heading, and a
//!   cell carries `text-align` itself (editor.md §3.7, "Tightening"). An
//!   `align` attribute on the block gives way, because a `style`
//!   declaration wins over it in a browser and everything the block shows
//!   was centred. A `text-align` of its own in its `style` does not, and
//!   the content is then handled as below.
//! - **Paragraphs, headings and cells** inside get `text-align: center`,
//!   unless they set an alignment of their own (below).
//! - **Text and inline elements** (a run of them, from the first visible
//!   node to the last) are wrapped in a new `<p style="text-align: center">`
//!   where a paragraph may stand: at the top level of the body, or in a
//!   container such as a list item, a quote, a cell or a `div`. That is the
//!   paragraph the editor would wrap loose inline content in anyway (N5b),
//!   with the alignment kept.
//! - **Containers** are gone into, and their content is handled by the
//!   same rules: the blocks a paragraph may stand in (list items, quotes,
//!   `div`s, ...), lists, and tables with their sections and rows. So
//!   `<center><ul><li>pont</li></ul></center>` becomes
//!   `<ul><li><p style="text-align: center">pont</p></li></ul>`, and the
//!   cells of a centred table get `text-align: center`. A container that
//!   sets another alignment of its own keeps its content as it is, because
//!   its content never showed the centre's. A `caption` is left alone: a
//!   browser centres it by itself (WHATWG HTML §15.3.8), and N5b moves it
//!   out of its table.
//! - **Anything else** keeps its place, and its centring is dropped:
//!   inline content where no paragraph may stand and no block around it
//!   can carry the alignment (in a heading that also holds other content,
//!   in a link, directly in a list), an inline element around a block (a
//!   paragraph around it would not parse back, because the block's start
//!   tag closes the paragraph), and the blocks that have no aligned form in
//!   content-v1 (`pre`, `legend`, `summary` and the like). The content is
//!   kept, and each such child or run counts as
//!   [`Rule::CenterAlignmentDropped`], so that editors see the body in the
//!   report. Content without visible text (an image, which N5b centres
//!   anyway; an empty table) is not counted. A `center` inside an inline
//!   element inside another `center` counts its drop even where the outer
//!   one's paragraph centres the content again: the count errs towards
//!   review.
//!
//! **Own alignment.** A block's own alignment wins over the one it
//! inherits from the `center`, as in CSS, so a block that sets one is left
//! as it is. A block sets one when its `style` holds a `text-align`
//! declaration that content-v1 keeps, as the sanitiser reads it (its
//! declaration reader and its `text-align` grammar: `left`, `center`,
//! `right` or `justify`; the last one wins, as in N7), or, without such a
//! declaration, when it has an `align` attribute that a browser maps to
//! `text-align` on that element (§15.3.3, §15.3.8: `left`, `right`,
//! `center` and `justify` on `p` and `h1`-`h6`; also `middle` on `div` and
//! the table parts, and `absmiddle` on the table parts; the value matched
//! whole, ASCII case-insensitively). Any other `align` value sets nothing.
//! A `style` whose `text-align` content-v1 does not keep (a value such as
//! `inherit` or `start`, one that cannot be read, such as one with a
//! comment, or one after the reader's declaration limit) is unclear: CSS's
//! own keywords such as `inherit` and every invalid value leave the
//! centre's alignment in force, so N3 centres the block as if it set none,
//! and N7 later drops the declaration it does not keep; but `start` showed
//! the block left-aligned, and an unreadable declaration may have shown
//! anything, so each such block also counts as
//! [`Rule::CenterAlignmentUnclear`] for review.
//!
//! Nested `center` elements are handled from the innermost out, so an inner
//! one's content has its alignment before the outer one looks at it.
//!
//! `text-align: center` is written in front of an existing `style`
//! (`text-align: center; color: red`), never after it: a declaration the
//! legacy style leaves open, such as an unclosed bracket or comment, would
//! swallow anything appended. Only blocks without an alignment content-v1
//! keeps get it, so nothing in the old value that N7 keeps can override it.
//! N7 then writes the style canonically (editor.md §3.1).
//!
//! **Cost.** Whether an inline element holds a block, and whether a node
//! holds visible text, are answered once per node and pass, and kept.
//! That is sound because N3 asks them only inside the `center` it is
//! handling, where every `center` has been handled already, and only about
//! subtrees that it never edits afterwards: inline elements and the blocks
//! it does not go into, which a later `center` moves whole at most, since
//! N3 goes only into containers. Outside the `center`, N3 only asks whether
//! the parent's children are seen, which needs no look below them. The unit
//! tests check every kept answer against a fresh one. A node is still
//! visited once for each `center` around it, so the work grows with the
//! body times the nesting depth of its `center` elements (see "Cost" in
//! the documentation of [`super`]).

use std::collections::BTreeMap;

use ego_tree::iter::Edge;
use ego_tree::{NodeId, NodeRef};
use html5ever::local_name;
use scraper::{Html, Node};

use super::dom;
use super::report::{NormaliseReport, Rule};
use crate::domain::rich_text::css::declarations;
use crate::domain::rich_text::css::grammar::{self, PropertyRule, Reading};

/// The declaration N3 gives a block.
const CENTRED: &str = "text-align: center";

/// The elements that carry `text-align` in content-v1 (editor.md §3.3):
/// paragraphs, headings and table cells, plus the two headings that N5
/// maps to `h4` with their style.
const ALIGNABLE: &[&str] = &["p", "h1", "h2", "h3", "h4", "h5", "h6", "td", "th"];

/// HTML elements that a browser lays out as blocks (or table parts, list
/// items and other non-inline boxes) by default (WHATWG HTML, "Rendering",
/// §15.3). Every other element, unknown ones included, is inline content,
/// as CSS's initial `display: inline` makes it.
#[rustfmt::skip]
const BLOCKS: &[&str] = &[
    "address", "article", "aside", "blockquote", "body", "caption", "center", "col", "colgroup",
    "dd", "details", "dialog", "dir", "div", "dl", "dt", "fieldset", "figcaption", "figure",
    "footer", "form", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr",
    "html", "legend", "li", "listing", "main", "menu", "nav", "ol", "optgroup", "p", "plaintext",
    "pre", "search", "section", "summary", "table", "tbody", "td", "tfoot", "th", "thead", "tr",
    "ul", "xmp",
];

/// Elements that a reader never sees: they are left where they are, inside
/// or between runs, and never make a run on their own. Text inside them is
/// not visible.
const UNSEEN: &[&str] = &[
    "base", "link", "meta", "noscript", "script", "style", "template", "title",
];

/// Blocks in which a paragraph may stand, so that a run of inline content
/// in them can become one. The body's root is one too. Apart from `td`,
/// `th` (which carry the alignment themselves when they can) and `caption`
/// (which a browser centres by itself), N3 goes into them when they are
/// inside a `center`.
#[rustfmt::skip]
const PARAGRAPH_CONTAINERS: &[&str] = &[
    "address", "article", "aside", "blockquote", "caption", "center", "dd", "details", "dialog",
    "div", "dt", "fieldset", "figcaption", "figure", "footer", "form", "header", "hgroup", "li",
    "main", "nav", "search", "section", "td", "th",
];

/// Lists, tables, and the sections and rows of a table: N3 goes into them
/// to reach their items and cells, but no paragraph may stand directly in
/// them.
const STRUCTURES: &[&str] = &[
    "dir", "dl", "menu", "ol", "table", "tbody", "tfoot", "thead", "tr", "ul",
];

/// The marker attributes of an embed: the editor's `YouTube` and padlet nodes
/// (editor.md §3.7) and `CKEditor`'s oEmbed wrapper (data-migration.md §7.1
/// N1).
const EMBED_MARKERS: &[&str] = &["data-youtube-video", "data-padlet-embed", "data-oembed-url"];

/// What a child of a `center`, or of a container inside one, is for N3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// An embed, kept as it is.
    Embed,
    /// A paragraph, heading or cell, which can carry the alignment.
    Alignable,
    /// A `caption`, which a browser centres by itself.
    SelfCentred,
    /// A block in which a paragraph may stand, which N3 goes into.
    Container,
    /// A list, table, table section or row, which N3 goes into.
    Structure,
    /// Another block, or an inline element around a block, which keeps its
    /// place.
    OtherBlock,
    /// Text or an inline element that a reader sees.
    Inline,
    /// White space, a comment or an element nobody sees.
    Unseen,
}

/// How an element is aligned before N3 removes the `center` around it
/// (see "Own alignment" in the module documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alignment {
    /// It sets none, so it shows the centre's.
    Inherited,
    /// It sets its own alignment.
    Own {
        /// Whether that alignment is `center`.
        centred: bool,
        /// Whether it comes from an `align` attribute rather than the
        /// `style`.
        by_attribute: bool,
    },
    /// Its `style` holds a `text-align` declaration that content-v1 does
    /// not keep.
    Unclear,
}

/// Applies N3 to every `center` of `html`, innermost first, and counts what
/// it did in `report`.
///
/// - `html`: a parsed body.
/// - `report`: the report of the current run.
pub(super) fn unwrap_centres(html: &mut Html, report: &mut NormaliseReport) {
    let centres = centres(html);
    let mut centring = Centring {
        html,
        report,
        blocks: BTreeMap::new(),
        visible: BTreeMap::new(),
        hidden: false,
    };
    // Document order puts an ancestor before its descendants, so the
    // reverse handles an inner `center` before the one around it.
    for (centre, hidden) in centres.into_iter().rev() {
        centring.hidden = hidden;
        centring.unwrap_centre(centre);
    }
}

/// The `center` elements of `html` in document order, each with whether it
/// lies inside an element nobody sees (a `template`, the only one of
/// [`UNSEEN`] that a parse gives element children), where nothing is
/// visible. One walk over the tree answers it for all of them.
///
/// - `html`: a parsed body.
fn centres(html: &Html) -> Vec<(NodeId, bool)> {
    let unseen =
        |node: NodeRef<'_, Node>| dom::html_name(node).is_some_and(|n| UNSEEN.contains(&n));
    let mut depth = 0_usize;
    let mut found = Vec::new();
    for edge in html.root_element().traverse() {
        match edge {
            Edge::Open(node) if unseen(node) => depth += 1,
            Edge::Open(node) if dom::html_name(node) == Some("center") => {
                found.push((node.id(), depth > 0));
            }
            Edge::Close(node) if unseen(node) => depth -= 1,
            Edge::Open(_) | Edge::Close(_) => {}
        }
    }
    found
}

/// The state of N3 over one parsed body.
struct Centring<'a> {
    /// The body being edited.
    html: &'a mut Html,
    /// The report of the current run.
    report: &'a mut NormaliseReport,
    /// For each node asked about, whether it is an HTML block or holds one.
    blocks: BTreeMap<NodeId, bool>,
    /// For each node asked about, whether it holds visible text.
    visible: BTreeMap<NodeId, bool>,
    /// Whether the `center` being handled lies inside an element nobody
    /// sees: nothing in it is visible, so nothing in it is counted for
    /// review.
    hidden: bool,
}

impl Centring<'_> {
    /// Applies N3 to one `center` (see the module documentation).
    ///
    /// - `centre`: the id of a `center` element below the body's root.
    fn unwrap_centre(&mut self, centre: NodeId) {
        self.report.record(Rule::CenterUnwrapped);
        if self
            .only_seen_child(centre)
            .is_some_and(|only| self.kind(only) == Kind::Embed)
        {
            dom::unwrap(self.html, centre);
            return;
        }
        let parent = dom::node(self.html, centre)
            .parent()
            .expect("a center always has a parent")
            .id();
        if !self.parent_carries(parent) {
            let takes_paragraph = parent == dom::content(self.html)
                || dom::html_name(dom::node(self.html, parent))
                    .is_some_and(|name| PARAGRAPH_CONTAINERS.contains(&name));
            self.centre_content(centre, takes_paragraph);
        }
        dom::unwrap(self.html, centre);
    }

    /// The case of a `center` that is the only visible content of a
    /// paragraph, heading or cell (see "The block around it" in the module
    /// documentation): gives `parent` the alignment if it can carry it, and
    /// returns whether the content needs nothing more.
    ///
    /// - `parent`: the parent of the `center`.
    fn parent_carries(&mut self, parent: NodeId) -> bool {
        if !dom::html_name(dom::node(self.html, parent)).is_some_and(|n| ALIGNABLE.contains(&n))
            || self.only_seen_child(parent).is_none()
        {
            return false;
        }
        match alignment(dom::node(self.html, parent)) {
            Alignment::Own { centred: true, .. } => {}
            Alignment::Own {
                by_attribute: false,
                ..
            } => return false,
            alignment => self.centre_block(parent, alignment),
        }
        true
    }

    /// Gives the content of `root`, which shows the centre's alignment,
    /// that alignment explicitly, going into the containers below it (see
    /// the module documentation). A list rather than recursion: legacy
    /// containers can nest deeply.
    ///
    /// - `root`: a `center`.
    /// - `takes_paragraph`: whether a paragraph may stand where the
    ///   content of `root` ends up.
    fn centre_content(&mut self, root: NodeId, takes_paragraph: bool) {
        let mut containers = vec![(root, takes_paragraph)];
        while let Some((container, takes_paragraph)) = containers.pop() {
            let children: Vec<(NodeId, Kind)> = dom::children(self.html, container)
                .into_iter()
                .map(|id| (id, self.kind(id)))
                .collect();
            for &(id, kind) in &children {
                match kind {
                    Kind::Alignable => match alignment(dom::node(self.html, id)) {
                        Alignment::Own { .. } => {}
                        alignment => self.centre_block(id, alignment),
                    },
                    Kind::Container | Kind::Structure => {
                        match alignment(dom::node(self.html, id)) {
                            Alignment::Own { centred: false, .. } => {}
                            alignment => {
                                if alignment == Alignment::Unclear {
                                    self.unclear();
                                }
                                containers.push((id, kind == Kind::Container));
                            }
                        }
                    }
                    Kind::OtherBlock if self.visible(id) => {
                        self.report.record(Rule::CenterAlignmentDropped);
                    }
                    Kind::OtherBlock
                    | Kind::Embed
                    | Kind::SelfCentred
                    | Kind::Inline
                    | Kind::Unseen => {}
                }
            }
            for run in inline_runs(&children) {
                if takes_paragraph {
                    let ids: Vec<NodeId> = run.iter().map(|&(id, _)| id).collect();
                    dom::wrap(
                        self.html,
                        &ids,
                        dom::new_element(local_name!("p"), &[("style", CENTRED)]),
                    );
                    self.report.record(Rule::CenterRunWrapped);
                } else if run.iter().any(|&(id, _)| self.visible(id)) {
                    self.report.record(Rule::CenterAlignmentDropped);
                }
            }
        }
    }

    /// Gives the block `id` `text-align: center` and counts it.
    ///
    /// - `id`: a paragraph, heading or cell without an alignment that
    ///   content-v1 keeps.
    /// - `alignment`: its alignment before, to count an unclear one.
    fn centre_block(&mut self, id: NodeId, alignment: Alignment) {
        dom::edit_element(self.html, id, centre_style);
        self.report.record(Rule::CenterBlockAligned);
        if alignment == Alignment::Unclear {
            self.unclear();
        }
    }

    /// Counts an element whose own alignment is unclear, unless nobody sees
    /// it.
    fn unclear(&mut self) {
        if !self.hidden {
            self.report.record(Rule::CenterAlignmentUnclear);
        }
    }

    /// The only child of `id` that a reader sees, if it has exactly one.
    /// It stops at the second, so a parent with many children costs no
    /// more than two. It decides by [`is_seen`], never by [`Self::kind`]:
    /// the children of a `center`'s parent can still hold `center`
    /// elements that N3 has not handled, so no answer about them may be
    /// kept (module documentation, "Cost").
    ///
    /// - `id`: a node of the body.
    fn only_seen_child(&self, id: NodeId) -> Option<NodeId> {
        let mut seen = dom::node(self.html, id)
            .children()
            .filter(|child| is_seen(*child));
        match (seen.next(), seen.next()) {
            (Some(only), None) => Some(only.id()),
            _ => None,
        }
    }

    /// What the node `id` is for N3.
    ///
    /// - `id`: a node of the body.
    fn kind(&mut self, id: NodeId) -> Kind {
        let node = dom::node(self.html, id);
        match node.value() {
            Node::Text(text) if !dom::is_white_space(text) => Kind::Inline,
            Node::Element(_) => {
                let Some(name) = dom::html_name(node) else {
                    // SVG and MathML roots are inline boxes.
                    return Kind::Inline;
                };
                if is_embed(node) {
                    Kind::Embed
                } else if ALIGNABLE.contains(&name) {
                    Kind::Alignable
                } else if name == "caption" {
                    Kind::SelfCentred
                } else if PARAGRAPH_CONTAINERS.contains(&name) {
                    Kind::Container
                } else if STRUCTURES.contains(&name) {
                    Kind::Structure
                } else if BLOCKS.contains(&name) {
                    Kind::OtherBlock
                } else if UNSEEN.contains(&name) {
                    Kind::Unseen
                } else if node
                    .children()
                    .any(|child| answer(&mut self.blocks, child, block_rule))
                {
                    // An inline element around a block (`<a><div>…`) is
                    // kept whole: a paragraph around it would not parse
                    // back, since the block's start tag closes the
                    // paragraph.
                    Kind::OtherBlock
                } else {
                    Kind::Inline
                }
            }
            _ => Kind::Unseen,
        }
    }

    /// Whether the node `id` holds text that a reader sees: none does in a
    /// `center` that lies inside an element nobody sees.
    ///
    /// - `id`: a node of the body.
    fn visible(&mut self, id: NodeId) -> bool {
        !self.hidden && answer(&mut self.visible, dom::node(self.html, id), visible_rule)
    }
}

/// Whether a reader sees `node`: whether [`Centring::kind`] would give it
/// any kind but [`Kind::Unseen`]. That needs no look below the node, so
/// nothing is kept.
///
/// - `node`: a node of a parsed body.
fn is_seen(node: NodeRef<'_, Node>) -> bool {
    match node.value() {
        Node::Text(text) => !dom::is_white_space(text),
        Node::Element(_) => dom::html_name(node).is_none_or(|name| !UNSEEN.contains(&name)),
        _ => false,
    }
}

/// How [`answer`] decides a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    /// The node answers yes by itself.
    Yes,
    /// The node answers no by itself, whatever it holds.
    No,
    /// The node answers yes when one of its children does.
    Children,
}

/// The rule for "is an HTML block or holds one".
///
/// - `node`: a node of the body.
fn block_rule(node: NodeRef<'_, Node>) -> Leaf {
    if dom::html_name(node).is_some_and(|name| BLOCKS.contains(&name)) {
        Leaf::Yes
    } else {
        Leaf::Children
    }
}

/// The rule for "holds text a reader sees": text that is not white space,
/// outside the elements nobody sees.
///
/// - `node`: a node of the body.
fn visible_rule(node: NodeRef<'_, Node>) -> Leaf {
    match node.value() {
        Node::Text(text) if !dom::is_white_space(text) => Leaf::Yes,
        Node::Element(_) if dom::html_name(node).is_some_and(|name| UNSEEN.contains(&name)) => {
            Leaf::No
        }
        Node::Element(_) => Leaf::Children,
        _ => Leaf::No,
    }
}

/// Answers `rule` for `root`, computing each node's answer at most once
/// and keeping it in `memo` (see "Cost" in the module documentation). A
/// list rather than recursion: legacy markup can nest deeply.
///
/// - `memo`: the answers kept for this rule so far.
/// - `root`: the node asked about.
/// - `rule`: how a node is decided.
fn answer(
    memo: &mut BTreeMap<NodeId, bool>,
    root: NodeRef<'_, Node>,
    rule: fn(NodeRef<'_, Node>) -> Leaf,
) -> bool {
    // Each node is pushed unexpanded once, by its parent, and expanded once:
    // its children are pushed above it, so they are answered first.
    let mut stack = vec![(root, false)];
    while let Some((node, expanded)) = stack.pop() {
        if memo.contains_key(&node.id()) {
            continue;
        }
        match rule(node) {
            Leaf::Yes => {
                memo.insert(node.id(), true);
            }
            Leaf::No => {
                memo.insert(node.id(), false);
            }
            Leaf::Children if expanded => {
                let value = node.children().any(|child| memo[&child.id()]);
                memo.insert(node.id(), value);
            }
            Leaf::Children => {
                stack.push((node, true));
                stack.extend(
                    node.children()
                        .filter(|child| !memo.contains_key(&child.id()))
                        .map(|child| (child, false)),
                );
            }
        }
    }
    let value = memo[&root.id()];
    // The kept answers stay true only because N3 never edits a subtree it
    // asked about (module documentation, "Cost"): every unit test of the
    // normaliser checks that against a fresh answer.
    #[cfg(test)]
    assert_eq!(value, fresh(root, rule), "a kept answer went stale");
    value
}

/// [`answer`] without the kept answers, for the tests.
#[cfg(test)]
fn fresh(node: NodeRef<'_, Node>, rule: fn(NodeRef<'_, Node>) -> Leaf) -> bool {
    match rule(node) {
        Leaf::Yes => true,
        Leaf::No => false,
        Leaf::Children => node.children().any(|child| fresh(child, rule)),
    }
}

/// Splits the children of a `center` or container into its runs of inline
/// content: the maximal sequences of [`Kind::Inline`] and [`Kind::Unseen`]
/// children, each cut to the span from its first [`Kind::Inline`] child to
/// its last. A sequence without inline content makes no run.
///
/// - `children`: the children with their kinds, in order.
fn inline_runs(children: &[(NodeId, Kind)]) -> Vec<&[(NodeId, Kind)]> {
    children
        .split(|(_, kind)| !matches!(kind, Kind::Inline | Kind::Unseen))
        .filter_map(|sequence| {
            let first = sequence
                .iter()
                .position(|(_, kind)| *kind == Kind::Inline)?;
            let last = sequence
                .iter()
                .rposition(|(_, kind)| *kind == Kind::Inline)?;
            Some(&sequence[first..=last])
        })
        .collect()
}

/// Whether `node` is an embed: an `iframe`, an element with an embed
/// marker, or a `p` or `div` whose only visible child is an embed.
///
/// - `node`: a node of a parsed body.
fn is_embed(node: NodeRef<'_, Node>) -> bool {
    let mut current = node;
    // A loop rather than recursion: a legacy wrapper can nest deeply.
    loop {
        let Some(name) = dom::html_name(current) else {
            return false;
        };
        let element = current.value().as_element().expect("an element");
        if name == "iframe"
            || EMBED_MARKERS
                .iter()
                .any(|marker| element.attr(marker).is_some())
        {
            return true;
        }
        if !matches!(name, "p" | "div") {
            return false;
        }
        let mut seen = current.children().filter(|child| match child.value() {
            Node::Text(text) => !dom::is_white_space(text),
            Node::Element(_) => dom::html_name(*child).is_none_or(|name| !UNSEEN.contains(&name)),
            _ => false,
        });
        match (seen.next(), seen.next()) {
            (Some(only), None) => current = only,
            _ => return false,
        }
    }
}

/// How the element `node` is aligned (see "Own alignment" in the module
/// documentation): its `style` first, because a declaration wins over a
/// presentational attribute, then its `align` attribute.
///
/// - `node`: a node of a parsed body.
fn alignment(node: NodeRef<'_, Node>) -> Alignment {
    let Some(element) = node.value().as_element() else {
        return Alignment::Inherited;
    };
    if let Some(alignment) = element.attr("style").and_then(style_alignment) {
        return alignment;
    }
    match dom::html_name(node)
        .zip(element.attr("align"))
        .and_then(|(name, value)| align_attribute(name, value))
    {
        Some(centred) => Alignment::Own {
            centred,
            by_attribute: true,
        },
        None => Alignment::Inherited,
    }
}

/// The alignment that the `style` attribute value `style` sets, as N7
/// reads it, or `None` when it names no `text-align`.
///
/// - `style`: the attribute value, with character references decoded.
fn style_alignment(style: &str) -> Option<Alignment> {
    let rule = text_align_rule();
    let read = declarations::read(style);
    let mut kept = None;
    let mut seen = 0;
    for declaration in read.read.iter().filter(|d| d.name == rule.name) {
        seen += 1;
        match grammar::parse(rule.grammar, &declaration.value) {
            // As in N7 and a browser, the last accepted one is in force.
            Reading::Accepted(value) => kept = Some(value.to_string() == "center"),
            // The grammar omits no default; if it ever did, the default
            // would be an alignment of its own, and not `center`.
            Reading::Default => kept = Some(false),
            Reading::Rejected => return Some(Alignment::Unclear),
        }
    }
    // A `text-align` that the reader could not read, or read after its
    // limit, is not among the declarations it returns.
    if text_align_mentions(style) > seen {
        return Some(Alignment::Unclear);
    }
    kept.map(|centred| Alignment::Own {
        centred,
        by_attribute: false,
    })
}

/// The sanitiser's `text-align` rule, the same for every element that
/// carries the property (editor.md §3.3).
fn text_align_rule() -> &'static PropertyRule {
    grammar::element_style("p")
        .and_then(|style| style.property("text-align"))
        .expect("content-v1 aligns paragraphs")
}

/// How many times `style` names the property `text-align`: the name, ASCII
/// case-insensitive, not inside a longer name (`text-align-last`), and
/// followed by a colon after optional white space. A mention inside a
/// comment counts too, which errs towards [`Alignment::Unclear`].
///
/// - `style`: a `style` attribute value.
fn text_align_mentions(style: &str) -> usize {
    const NAME: &str = "text-align";
    let lower = style.to_ascii_lowercase();
    let in_name = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    lower
        .match_indices(NAME)
        .filter(|&(at, _)| {
            let before = lower[..at].chars().next_back();
            let after =
                lower[at + NAME.len()..].trim_start_matches(|c: char| c.is_ascii_whitespace());
            !before.is_some_and(in_name) && after.starts_with(':')
        })
        .count()
}

/// What the `align` attribute `value` of the HTML element `name` does to
/// its text alignment (WHATWG HTML, "Rendering", §15.3.3 and §15.3.8):
/// `Some(true)` when it centres, `Some(false)` when it sets another
/// alignment, `None` when a browser does not map it to `text-align` on
/// that element. The value is matched whole and ASCII case-insensitively,
/// as the specification's attribute selectors match it.
///
/// - `name`: a lower-case HTML tag name.
/// - `value`: the attribute's value.
fn align_attribute(name: &str, value: &str) -> Option<bool> {
    let values: &[&str] = match name {
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => &["left", "right", "center", "justify"],
        "div" => &["left", "right", "center", "middle", "justify"],
        "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" => {
            &["left", "right", "center", "middle", "absmiddle", "justify"]
        }
        _ => return None,
    };
    let known = values
        .iter()
        .find(|known| known.eq_ignore_ascii_case(value))?;
    Some(matches!(*known, "center" | "middle" | "absmiddle"))
}

/// Gives `element` `text-align: center` in front of its `style` (see the
/// module documentation for why in front).
///
/// - `element`: a block without an alignment that content-v1 keeps.
fn centre_style(element: &mut scraper::node::Element) {
    let value = match element.attr("style") {
        Some(style)
            if !style
                .trim_matches(|c: char| c.is_ascii_whitespace() || c == ';')
                .is_empty() =>
        {
            format!(
                "{CENTRED}; {}",
                style.trim_start_matches(|c: char| c.is_ascii_whitespace())
            )
        }
        _ => CENTRED.to_owned(),
    };
    dom::set_attribute(element, "style", &value);
}

/// The style string N3 writes for a block whose `style` is `style`; the
/// pure core of [`centre_style`], for the tests.
#[cfg(test)]
fn centred_style(style: Option<&str>) -> String {
    let mut html = dom::parse("<p></p>");
    let p = dom::elements_named(&html, "p")[0];
    dom::edit_element(&mut html, p, |element| {
        if let Some(style) = style {
            dom::set_attribute(element, "style", style);
        }
        centre_style(element);
        element.attr("style").expect("a style").to_owned()
    })
    .expect("an element")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// N3 over `body`: the output and the report.
    fn n3(body: &str) -> (String, NormaliseReport) {
        let mut html = dom::parse(body);
        let mut report = NormaliseReport::default();
        unwrap_centres(&mut html, &mut report);
        (dom::serialise(&html), report)
    }

    /// The counts of the five N3 rules: unwrapped, aligned, wrapped,
    /// dropped, unclear.
    fn counts(report: &NormaliseReport) -> [usize; 5] {
        [
            report.count(Rule::CenterUnwrapped),
            report.count(Rule::CenterBlockAligned),
            report.count(Rule::CenterRunWrapped),
            report.count(Rule::CenterAlignmentDropped),
            report.count(Rule::CenterAlignmentUnclear),
        ]
    }

    /// The alignment of the first element of `body` named `name`.
    fn alignment_of(body: &str, name: &str) -> Alignment {
        let html = dom::parse(body);
        alignment(dom::node(&html, dom::elements_named(&html, name)[0]))
    }

    const fn own(centred: bool, by_attribute: bool) -> Alignment {
        Alignment::Own {
            centred,
            by_attribute,
        }
    }

    #[test]
    fn a_centre_around_an_embed_is_just_removed() {
        for body in [
            "<center>\n<p><iframe src=\"https://www.youtube.com/embed/AbCdEfGhIjK\"></iframe></p>\n</center>",
            "<center><iframe src=\"x\"></iframe></center>",
            "<center><div data-oembed-url=\"https://youtu.be/x\"><div><iframe></iframe></div></div></center>",
            "<center><p data-youtube-video=\"\"><iframe></iframe></p><!-- x --></center>",
            "<center><div>\n<p> <iframe></iframe> </p></div></center>",
            // In a heading too: the embed rule comes first.
            "<h2><center><iframe src=\"x\"></iframe></center></h2>",
        ] {
            let (out, report) = n3(body);
            let expected = dom::serialise(&dom::parse(
                &body.replace("<center>", "").replace("</center>", ""),
            ));
            assert_eq!(out, expected, "{body:?}");
            assert_eq!(counts(&report), [1, 0, 0, 0, 0], "{body:?}");
        }
    }

    #[test]
    fn paragraphs_and_headings_get_the_alignment() {
        let (out, report) = n3("<center><h2>Eredmények</h2><p>Szöveg</p></center>");
        assert_eq!(
            out,
            r#"<h2 style="text-align: center">Eredmények</h2><p style="text-align: center">Szöveg</p>"#
        );
        assert_eq!(counts(&report), [1, 2, 0, 0, 0]);
    }

    #[test]
    fn an_existing_style_keeps_its_declarations_after_the_alignment() {
        let (out, _) = n3(r#"<center><P STYLE="color: red">a</P></center>"#);
        assert_eq!(out, r#"<p style="text-align: center; color: red">a</p>"#);
    }

    #[test]
    fn a_block_with_its_own_alignment_keeps_it() {
        let (out, report) = n3(concat!(
            r#"<center><p style="TEXT-ALIGN: justify">a</p>"#,
            r#"<p align="Right">b</p><p style="text-align: center">c</p>"#,
            r#"<p style="text-align: bogus; text-align: left">d</p></center>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<p style="TEXT-ALIGN: justify">a</p><p align="Right">b</p>"#,
                r#"<p style="text-align: center">c</p>"#,
                // A rejected declaration makes the block unclear, even
                // before an accepted one.
                r#"<p style="text-align: center; text-align: bogus; text-align: left">d</p>"#,
            )
        );
        assert_eq!(counts(&report), [1, 1, 0, 0, 1]);
    }

    #[test]
    fn an_align_value_that_sets_nothing_leaves_the_block_to_inherit() {
        // `middle` aligns a `div` and the table parts, never a paragraph or
        // a heading, and a value with white space around it matches
        // nothing (WHATWG HTML §15.3.8).
        let (out, report) = n3(concat!(
            r#"<center><p align="bogus">a</p><p align="middle">b</p>"#,
            r#"<h3 align=" left ">c</h3></center>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<p style="text-align: center" align="bogus">a</p>"#,
                r#"<p style="text-align: center" align="middle">b</p>"#,
                r#"<h3 style="text-align: center" align=" left ">c</h3>"#,
            )
        );
        assert_eq!(counts(&report), [1, 3, 0, 0, 0]);
    }

    #[test]
    fn an_alignment_content_v1_does_not_keep_is_centred_and_counted() {
        // `inherit` and an invalid value showed the centre's alignment;
        // `start`, a commented value and one after the reader's limit may
        // not have. N7 drops each of them later.
        let many: String = (0..declarations::MAX_DECLARATIONS)
            .map(|_| "color: red; ")
            .collect();
        for style in [
            "text-align: inherit".to_owned(),
            "text-align: start".to_owned(),
            "text-align: bogus".to_owned(),
            "text-align: url(x)".to_owned(),
            "text-align:justify /* c */".to_owned(),
            format!("{many}text-align: right"),
        ] {
            let (out, report) = n3(&format!(r#"<center><p style="{style}">a</p></center>"#));
            assert_eq!(
                out,
                format!(r#"<p style="text-align: center; {style}">a</p>"#),
                "{style}"
            );
            assert_eq!(counts(&report), [1, 1, 0, 0, 1], "{style}");
        }
    }

    #[test]
    fn alignment_is_read_as_the_sanitiser_and_a_browser_read_it() {
        let p = |attributes: &str| alignment_of(&format!("<p {attributes}>x</p>"), "p");
        assert_eq!(p(""), Alignment::Inherited);
        assert_eq!(p(r#"style="color: red""#), Alignment::Inherited);
        assert_eq!(p(r#"style="text-align: CENTER""#), own(true, false));
        assert_eq!(
            p(r#"style="text-align: center; text-align: left""#),
            own(false, false),
            "the last one wins"
        );
        assert_eq!(
            p(r#"style="text-align-last: center""#),
            Alignment::Inherited
        );
        assert_eq!(
            p(r#"style="text-align: left" align="center""#),
            own(false, false)
        );
        assert_eq!(p(r#"align="CENTER""#), own(true, true));
        assert_eq!(p(r#"align="justify""#), own(false, true));
        let div = |attributes: &str| alignment_of(&format!("<div {attributes}>x</div>"), "div");
        assert_eq!(div(r#"align="middle""#), own(true, true));
        assert_eq!(div(r#"align="absmiddle""#), Alignment::Inherited);
        assert_eq!(div(r#"style="text-align: right""#), own(false, false));
        let cell = alignment_of(
            r#"<table><tr><td align="absmiddle">x</td></tr></table>"#,
            "td",
        );
        assert_eq!(cell, own(true, true));
        assert_eq!(
            alignment_of(r#"<ul align="right"><li>x</li></ul>"#, "ul"),
            Alignment::Inherited,
            "a list maps no align"
        );
    }

    #[test]
    fn text_align_mentions_count_the_property_name_only() {
        assert_eq!(text_align_mentions(""), 0);
        assert_eq!(text_align_mentions("TEXT-ALIGN : x; text-align:y"), 2);
        assert_eq!(text_align_mentions("text-align-last: x"), 0);
        assert_eq!(text_align_mentions("-webkit-text-align: x"), 0);
        assert_eq!(text_align_mentions("text-alignment: x"), 0);
        assert_eq!(text_align_mentions("/* text-align: x */"), 1);
    }

    #[test]
    fn align_attributes_map_as_the_rendering_rules_say() {
        assert_eq!(align_attribute("p", "center"), Some(true));
        assert_eq!(align_attribute("h6", "Left"), Some(false));
        assert_eq!(align_attribute("p", "middle"), None);
        assert_eq!(align_attribute("div", "MIDDLE"), Some(true));
        assert_eq!(align_attribute("div", "absmiddle"), None);
        assert_eq!(align_attribute("tr", "absmiddle"), Some(true));
        assert_eq!(align_attribute("th", "justify"), Some(false));
        assert_eq!(align_attribute("li", "center"), None);
        assert_eq!(
            align_attribute("table", "center"),
            None,
            "it places the table"
        );
        assert_eq!(align_attribute("p", "center "), None);
    }

    #[test]
    fn inline_content_is_wrapped_in_a_centred_paragraph() {
        let (out, report) =
            n3("<center>\n Szöveg <strong>középen</strong>\n<p>bekezdés</p>után<br></center>");
        assert_eq!(
            out,
            concat!(
                // A run starts and ends at a visible node, so the white
                // space after `strong` stays outside; the text node with the
                // first word is one node and moves whole.
                "<p style=\"text-align: center\">\n Szöveg <strong>középen</strong></p>",
                "\n",
                r#"<p style="text-align: center">bekezdés</p>"#,
                r#"<p style="text-align: center">után<br></p>"#,
            )
        );
        assert_eq!(counts(&report), [1, 1, 2, 0, 0]);
    }

    #[test]
    fn a_centre_in_an_item_wraps_its_text_and_one_in_a_cell_aligns_the_cell() {
        let (out, report) = n3("<ul><li><center>pont</center></li></ul>");
        assert_eq!(
            out,
            r#"<ul><li><p style="text-align: center">pont</p></li></ul>"#
        );
        assert_eq!(counts(&report), [1, 0, 1, 0, 0]);
        // A cell carries `text-align` itself, the form the editor stores
        // (editor.md §3.7, "Tightening").
        let (out, report) = n3(concat!(
            "<table><tbody><tr><td><center>x</center></td>",
            r#"<th style="color: red"> <center>y</center> </th></tr></tbody></table>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<table><tbody><tr><td style="text-align: center">x</td>"#,
                r#"<th style="text-align: center; color: red"> y </th></tr></tbody></table>"#,
            )
        );
        assert_eq!(counts(&report), [2, 2, 0, 0, 0]);
        // A cell with more than the centre keeps its alignment, and the
        // centred part becomes a paragraph.
        let (out, report) = n3("<table><tr><td>a<center>b</center></td></tr></table>");
        assert_eq!(
            out,
            r#"<table><tbody><tr><td>a<p style="text-align: center">b</p></td></tr></tbody></table>"#
        );
        assert_eq!(counts(&report), [1, 0, 1, 0, 0]);
    }

    #[test]
    fn a_centre_inside_a_heading_aligns_the_heading() {
        let (out, report) = n3("<h2>\n<center>Cím</center>\n</h2>");
        assert_eq!(out, "<h2 style=\"text-align: center\">\nCím\n</h2>");
        assert_eq!(counts(&report), [1, 1, 0, 0, 0]);
        // An `align` attribute gives way: everything the heading shows was
        // centred.
        let (out, report) = n3(r#"<h2 align="left"><center>Cím</center></h2>"#);
        assert_eq!(
            out,
            r#"<h2 style="text-align: center" align="left">Cím</h2>"#
        );
        assert_eq!(counts(&report), [1, 1, 0, 0, 0]);
        // A heading that is centred already needs nothing, and loses
        // nothing.
        for body in [
            r#"<h2 align="center"><center>Cím</center></h2>"#,
            r#"<h2 style="text-align: center"><center>Cím</center></h2>"#,
        ] {
            let (out, report) = n3(body);
            assert_eq!(out, body.replace("<center>", "").replace("</center>", ""));
            assert_eq!(counts(&report), [1, 0, 0, 0, 0], "{body}");
        }
        let (out, report) = n3(r#"<h2 style="text-align: inherit"><center>Cím</center></h2>"#);
        assert_eq!(
            out,
            r#"<h2 style="text-align: center; text-align: inherit">Cím</h2>"#
        );
        assert_eq!(counts(&report), [1, 1, 0, 0, 1]);
    }

    #[test]
    fn a_centre_that_cannot_be_kept_is_counted() {
        // A heading that holds more than the centre, a heading with its own
        // alignment in its style, and a link (inline) as the parent. (In
        // `<p><a><center>` the `center` start tag closes the paragraph, so a
        // `div` holds the link here.)
        for body in [
            "<h2>Cím: <center>közép</center></h2>",
            r#"<h2 style="text-align: left"><center>közép</center></h2>"#,
            r#"<div><a href="x"><center>közép</center></a></div>"#,
        ] {
            let (out, report) = n3(body);
            assert!(!out.contains("center"), "{out}");
            assert_eq!(counts(&report), [1, 0, 0, 1, 0], "{body}");
        }
        // A link around a block, a `pre`, and text directly in a list; an
        // image alone in a heading has no text, so it is not counted.
        let (out, report) = n3(concat!(
            r#"<center><a href="x"><div>c</div></a><pre>kód</pre><hr></center>"#,
            "<ul><center>d</center></ul><h2>Cím<center><img src=\"/a.jpg\"></center></h2>",
        ));
        assert_eq!(
            out,
            concat!(
                r#"<a href="x"><div>c</div></a><pre>kód</pre><hr>"#,
                r#"<ul>d</ul><h2>Cím<img src="/a.jpg"></h2>"#,
            )
        );
        assert_eq!(counts(&report), [3, 0, 0, 3, 0]);
    }

    #[test]
    fn lists_quotes_and_divs_inside_a_centre_are_gone_into() {
        let (out, report) = n3(concat!(
            "<center><ul><li>a</li><li><p>b</p><ol><li>c</li></ol></li></ul>",
            "<blockquote>d</blockquote><div><div>e</div><h3>f</h3></div>",
            "<dl><dt>g</dt><dd>h</dd></dl></center>",
        ));
        assert_eq!(
            out,
            concat!(
                r#"<ul><li><p style="text-align: center">a</p></li>"#,
                r#"<li><p style="text-align: center">b</p><ol><li><p style="text-align: center">c</p></li></ol></li></ul>"#,
                r#"<blockquote><p style="text-align: center">d</p></blockquote>"#,
                r#"<div><div><p style="text-align: center">e</p></div><h3 style="text-align: center">f</h3></div>"#,
                r#"<dl><dt><p style="text-align: center">g</p></dt><dd><p style="text-align: center">h</p></dd></dl>"#,
            )
        );
        assert_eq!(counts(&report), [1, 2, 6, 0, 0]);
    }

    #[test]
    fn a_container_that_sets_another_alignment_keeps_its_content() {
        let (out, report) = n3(concat!(
            r#"<center><div align="right">a</div><blockquote style="text-align: left">b</blockquote>"#,
            r#"<div align="center">c</div><ul style="text-align: inherit"><li>d</li></ul></center>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<div align="right">a</div><blockquote style="text-align: left">b</blockquote>"#,
                // A centred one is gone into, so the sanitiser's unwrapping
                // of the `div` keeps the centring; an unclear one too.
                r#"<div align="center"><p style="text-align: center">c</p></div>"#,
                r#"<ul style="text-align: inherit"><li><p style="text-align: center">d</p></li></ul>"#,
            )
        );
        assert_eq!(counts(&report), [1, 0, 2, 0, 1]);
    }

    #[test]
    fn the_cells_of_a_centred_table_get_the_alignment() {
        let (out, report) = n3(concat!(
            "<center><table><caption>Felirat</caption><tbody>",
            r#"<tr><th>Fej</th><td style="text-align: left">a</td><td align="right">b</td></tr>"#,
            r#"<tr align="left"><td>c</td></tr><tr><td><p>d</p></td></tr></tbody></table></center>"#,
        ));
        assert_eq!(
            out,
            concat!(
                "<table><caption>Felirat</caption><tbody>",
                r#"<tr><th style="text-align: center">Fej</th><td style="text-align: left">a</td><td align="right">b</td></tr>"#,
                // A row that sets its own alignment is left as it is; a
                // cell's paragraph inherits the cell's.
                r#"<tr align="left"><td>c</td></tr><tr><td style="text-align: center"><p>d</p></td></tr></tbody></table>"#,
            )
        );
        assert_eq!(counts(&report), [1, 2, 0, 0, 0]);
    }

    #[test]
    fn nested_centres_are_handled_from_the_inside() {
        let (out, report) = n3("<center><center><p>x</p>y</center>z</center>");
        assert_eq!(
            out,
            concat!(
                r#"<p style="text-align: center">x</p>"#,
                r#"<p style="text-align: center">y</p>"#,
                r#"<p style="text-align: center">z</p>"#,
            )
        );
        assert_eq!(counts(&report), [2, 1, 2, 0, 0]);
        let (out, report) = n3("<center><ul><li><center>x</center></li></ul></center>");
        assert_eq!(
            out,
            r#"<ul><li><p style="text-align: center">x</p></li></ul>"#
        );
        assert_eq!(counts(&report), [2, 0, 1, 0, 0]);
    }

    #[test]
    fn unseen_content_stays_and_makes_no_run() {
        let (out, report) = n3("<center><!-- c --><script>x</script> <p>a</p></center>");
        assert_eq!(
            out,
            r#"<!-- c --><script>x</script> <p style="text-align: center">a</p>"#
        );
        assert_eq!(counts(&report), [1, 1, 0, 0, 0]);
        let (out, report) = n3("<center></center><center> </center>");
        assert_eq!(out, " ");
        assert_eq!(counts(&report), [2, 0, 0, 0, 0]);
    }

    #[test]
    fn nothing_inside_an_unseen_element_is_counted_for_review() {
        // A `template` is the one unseen element whose content a parse
        // keeps as elements. Its `center` goes too, but no reader sees what
        // it held, so nothing is counted for review.
        let (out, report) = n3(concat!(
            "<template><h2>a<center>b</center></h2>",
            r#"<center><p style="text-align: start">c</p></center></template>"#,
        ));
        assert_eq!(
            out,
            r#"<template><h2>ab</h2><p style="text-align: center; text-align: start">c</p></template>"#
        );
        assert_eq!(counts(&report), [2, 1, 0, 0, 0]);
    }

    #[test]
    fn foreign_content_is_inline() {
        // An `svg` root is an inline box, so it is wrapped like any other.
        // (A `center` start tag breaks out of SVG, so every `center` that a
        // parse makes is HTML.)
        let (out, report) = n3("<center><svg><circle></circle></svg></center>");
        assert_eq!(
            out,
            r#"<p style="text-align: center"><svg><circle></circle></svg></p>"#
        );
        assert_eq!(counts(&report), [1, 0, 1, 0, 0]);
    }

    #[test]
    fn many_sibling_centres_cost_no_more_than_two_each() {
        // Each `center` in a crowded heading looks at two of its siblings
        // at most, so this stays fast in a debug build (the integration
        // test `tests/normalise/cost.rs` times it).
        let body = format!("<h2>{}</h2>", "<center>x</center>".repeat(500));
        let (out, report) = n3(&body);
        assert_eq!(out, format!("<h2>{}</h2>", "x".repeat(500)));
        assert_eq!(counts(&report), [500, 0, 0, 500, 0]);
    }

    #[test]
    fn siblings_that_still_hold_a_centre_are_not_asked_below() {
        // When `c` is handled, the `span` beside it still holds `d`: an
        // answer about the `b` in between, kept then, would be wrong once
        // `d` is gone (`answer` checks every kept one).
        let (out, report) = n3(concat!(
            "<h2><center>e</center><span><b><center>d</center></b></span>",
            "<center>c</center></h2>",
        ));
        assert_eq!(out, "<h2>e<span><b>d</b></span>c</h2>");
        assert_eq!(counts(&report), [3, 0, 0, 3, 0]);
    }

    #[test]
    fn the_style_goes_in_front_of_the_old_one() {
        assert_eq!(centred_style(None), "text-align: center");
        assert_eq!(centred_style(Some("")), "text-align: center");
        assert_eq!(centred_style(Some(" ; ;")), "text-align: center");
        assert_eq!(
            centred_style(Some("  color: red;")),
            "text-align: center; color: red;"
        );
        // An unclosed comment would swallow an appended declaration.
        let style = centred_style(Some("color: red; /* x"));
        assert_eq!(style, "text-align: center; color: red; /* x");
        assert!(
            declarations::read(&style)
                .read
                .iter()
                .any(|declaration| declaration.name == "text-align")
        );
    }

    /// Bodies built from `center` and the shapes N3 tells apart.
    fn arb_body() -> impl Strategy<Value = String> {
        let pieces = prop::sample::select(
            &[
                "<center>",
                "</center>",
                "<p>",
                "</p>",
                "<h2>",
                "</h2>",
                "<ul><li>",
                "</li></ul>",
                "<li>",
                "<blockquote>",
                "</blockquote>",
                "<td>",
                "<table><tr><td>",
                "</td></tr></table>",
                "<table><caption>",
                "<a href=\"x\">",
                "</a>",
                "<strong>",
                "</strong>",
                "<span>",
                "</span>",
                "<span><b><center>",
                "</center></b></span>",
                "<iframe src=\"x\"></iframe>",
                "<p style=\"text-align: right\">",
                "<p style=\"text-align: inherit\">",
                "<p style=\"color: red; /*\">",
                "<div align=\"center\">",
                "<div style=\"text-align: left\">",
                "<div>",
                "</div>",
                "<pre>",
                "<template>",
                "</template>",
                "<br>",
                "<img src=\"/storage/uploads/a.jpg\">",
                "<!-- c -->",
                "szöveg",
                " ",
                "\n",
                "<script>x</script>",
            ][..],
        );
        prop::collection::vec(pieces, 0..24).prop_map(|pieces| pieces.concat())
    }

    /// The text of `html`'s body, in document order, white space removed.
    fn text_of(html: &str) -> String {
        dom::parse(html)
            .root_element()
            .descendants()
            .filter_map(|node| node.value().as_text().map(|text| text.to_string()))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect()
    }

    proptest! {
        // No `center` is left, every one is counted, and the text is kept
        // in its order. (Every kept answer is checked against a fresh one
        // on the way, in `answer`.)
        #[test]
        fn every_centre_goes_and_the_text_stays(body in arb_body()) {
            let html = dom::parse(&body);
            let centres = dom::elements_named(&html, "center").len();
            let (out, report) = n3(&body);
            prop_assert!(dom::elements_named(&dom::parse(&out), "center").is_empty(), "{:?}", out);
            prop_assert_eq!(report.count(Rule::CenterUnwrapped), centres);
            prop_assert_eq!(text_of(&out), text_of(&dom::serialise(&html)));
        }

        // `is_seen` tells the same as `kind` for every node, without
        // looking below it.
        #[test]
        fn is_seen_agrees_with_the_kind(body in arb_body()) {
            let mut html = dom::parse(&body);
            let ids: Vec<NodeId> = html.root_element().descendants().map(|node| node.id()).collect();
            let mut report = NormaliseReport::default();
            let mut centring = Centring {
                html: &mut html,
                report: &mut report,
                blocks: BTreeMap::new(),
                visible: BTreeMap::new(),
                hidden: false,
            };
            for id in ids {
                let seen = is_seen(dom::node(centring.html, id));
                prop_assert_eq!(seen, centring.kind(id) != Kind::Unseen, "{:?}", body);
            }
        }

        // Without a `center`, N3 changes nothing and counts nothing.
        #[test]
        fn a_body_without_a_centre_is_unchanged(body in arb_body()) {
            let body = body.replace("<center>", "").replace("</center>", "");
            let (out, report) = n3(&body);
            prop_assert_eq!(out, dom::serialise(&dom::parse(&body)));
            prop_assert!(!report.has_hits());
        }

        // The alignment always reads back from the style N3 writes.
        #[test]
        fn the_written_style_always_aligns(style in proptest::option::of(any::<String>())) {
            let written = centred_style(style.as_deref());
            let read = declarations::read(&written);
            prop_assert_eq!(read.read.first().map(|d| d.name.as_str()), Some("text-align"), "{:?}", written);
        }

        // A block that N3 centres reads as centred, or unclear, afterwards:
        // the declaration it writes is the first one, so only a later
        // `text-align` in the old value can stand in its way, and N3 writes
        // it only where there is none that content-v1 keeps.
        #[test]
        fn a_centred_block_reads_centred_or_unclear(style in "[a-z: ;/*-]{0,24}") {
            let body = format!(r#"<center><p style="{style}">x</p></center>"#);
            let (out, _) = n3(&body);
            let after = alignment_of(&out, "p");
            match alignment_of(&format!(r#"<p style="{style}">x</p>"#), "p") {
                Alignment::Own { .. } => prop_assert_eq!(&out, &dom::serialise(&dom::parse(&format!(r#"<p style="{style}">x</p>"#)))),
                _ => prop_assert!(matches!(after, Alignment::Own { centred: true, .. } | Alignment::Unclear), "{:?}", out),
            }
        }

        // Mentions are never fewer than the declarations the reader reads,
        // so a readable style without a rejected value is never unclear.
        #[test]
        fn mentions_cover_every_read_declaration(style in "[a-zA-Z: ;-]{0,32}") {
            let read = declarations::read(&style)
                .read
                .iter()
                .filter(|d| d.name == "text-align")
                .count();
            prop_assert!(text_align_mentions(&style) >= read, "{:?}", style);
        }
    }
}
