//! N5b, its image rules: every image becomes the canonical image paragraph
//! of content-v1 (data-migration.md §7.1 and §7.4, editor.md §3.7, §5.2
//! step 11a and §9, ADR 0017 §2 step 6, decision 15b).
//!
//! content-v1 stores an image in exactly one shape, a paragraph of its own
//! at the top level of the body (editor.md §3.7):
//!
//! ```html
//! <p><img src="/storage/uploads/…" alt="…" width="350" height="495" style="display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px"></p>
//! ```
//!
//! The legacy bodies hold `CKEditor` 4's images instead: sized by a
//! `style` with a fixed `height` and `width`, some floated, some inside
//! headings, links, marks, cells, list items or quotes (research: gap-06
//! §4.2-§4.3). The admin editor's image node is an atom at the top level
//! only, so loading such a body would move each image itself, and
//! `ProseMirror`'s own placement could cut a table or a list in two around
//! it. The import therefore writes the image's place and shape itself, so
//! that the editor finds nothing to change (N9, ADR 0013 §2). These rules
//! do that; the rest of N5b (tables, tightening, empty links, top-level
//! inline content, the editor's mark structure and the whitespace rule) is
//! a later slice's.
//!
//! **Which images.** Every HTML `img` that N7 keeps, which are the images
//! the editor's node reads: one whose `src` the sanitiser's image rule
//! keeps ([`url_rules::image_src`], editor.md §3.6), read as N6 will leave
//! it (a `src` on the dead host's `/storage/` counts as its root-relative
//! path, which N6 writes after these rules). Every other image is left
//! where it is, for N7, which removes it: an `img` without `src`, or with
//! one that the rule refuses (another host, a path outside the uploads, an
//! empty value), so that no paragraph is split and no link dropped for an
//! image that N7 then removes, leaving an empty paragraph behind; one inside an
//! element that the sanitiser removes with its content (a `template`,
//! which no reader sees, an `object`, a `video`, a `button`, ...;
//! [`REMOVED_WITH_CONTENT`]); and one inside foreign content (an `img` in
//! an SVG `foreignObject`), which N7 removes together with its `svg`. An
//! inline `data:` image is left for N7 too until N4 joins the pass: N4
//! runs before N5b and turns every one it keeps into a file path, which
//! these rules then place like any other (data-migration.md §7.1, N4).
//!
//! **The shape.** The image keeps its `src` and its `alt`, missing or
//! blank ones included: the importer never invents alt text, and the
//! editor asks for it before the next save (ADR 0013 §2; the report lists
//! these images, `images.rs`). The rest is written from what the legacy
//! markup showed:
//!
//! - **The width** is the box's width: the last `width` declaration of the
//!   `style`, when it is a length (px, or `pt`, `cm` and the other absolute
//!   units, converted to px and rounded to hundredths as the sanitiser
//!   reads them, editor.md §3.1 rule 4), else the `width` attribute (a whole
//!   number of px, written as the sanitiser reads the attribute: ASCII
//!   white space around one to ten ASCII digits, from 1 up). It is kept
//!   whatever its size, also above the 700 px column (decision 15b). A
//!   width above content-v1's bound of 2000 px (editor.md §3.3), from the
//!   `style` or from the attribute (also one above the attribute's own
//!   range, 1-5000), is kept too, in the `style`, because every legacy
//!   value over a limit is reported and never truncated (data-migration.md
//!   §5): the report lists the image ([`super::WideImage`]), and N7's
//!   sanitiser drops such a width, a removal that no step planned, so the
//!   import stops on the body until a policy change raises the bound
//!   before the import (editor.md §16 item 5) or an `N7-accept` entry lets
//!   the width go (data-migration.md §7.1, N7; §7.4, reading (e), which
//!   the maintainer confirmed as built on 2026-11-15, research: DECISIONS
//!   R39 (D169)). An image without a width keeps its natural width.
//! - **The height** becomes `height: auto`, and the ratio the legacy markup
//!   showed is kept in the `width` and `height` attributes
//!   ([`Rule::ImageHeightAuto`]): with a `height` declaration that is a
//!   length, the ratio of the box the `style` set, that height over the
//!   width above; without one, the ratio of the `width` and `height`
//!   attributes, the editor's own reading of them. Without either, the
//!   image has no `height` attribute and keeps its natural ratio. A height
//!   that cannot become a ratio is lost, and the image counts as
//!   [`Rule::ImageSizingDropped`]: the legacy markup fixed a height (a
//!   `height` declaration that is a length, or, when the `style` declares
//!   no height, the `height` attribute) but gave no width
//!   (`style="height: 300px"` or `height="300"` alone), or the height the
//!   ratio gives falls outside the attributes' range. Nothing could keep
//!   it, since content-v1 locks `height: auto`, so this is the one sizing
//!   that N5b drops itself, counted, without N7's review (data-migration.md
//!   §7.4, reading (g), confirmed as built: D169).
//! - **The attributes** are the editor's: `width` is the width rounded to a
//!   whole px, half up, and `height` that width times the ratio, rounded
//!   the same way, each written only within the attributes' range, as the
//!   admin's `imageSize` writes them, and neither for a width over the
//!   bound, for which the admin's node writes none either. So the editor
//!   reads back the same two numbers, and the shape is a fixed point of its
//!   round trip; an image over the bound comes out of N7 without a width,
//!   which the editor also saves as it is, and which the next run leaves
//!   alone.
//! - **The `style`** is the canonical one (editor.md §3.1): the four locked
//!   declarations, then the width in px. The sanitiser writes it
//!   ([`sanitize_style`]), so it is exactly what N7 keeps; only a width over
//!   the bound, which the sanitiser drops, is appended by hand in the same
//!   spelling. What no image rule plans for follows, byte for byte (below).
//! - **The float** goes ([`Rule::ImageFloatRemoved`]): a `float`
//!   declaration other than `none`, the classes `left` and `right` (split at
//!   ASCII white space and matched exactly, as N5's marker rule matches its
//!   class; the other classes stay for N7), and an `align` of `left` or
//!   `right` (ASCII case-insensitive, the values a browser floats an image
//!   by, WHATWG HTML, "Rendering", §15.4.3). The locked `margin: 0 auto`
//!   centres the image instead. An `align` with any other value stays for
//!   N7.
//! - **What N7 reviews.** N5b cuts out whole only the declarations it plans
//!   for, as N5's italic-heading rule cuts `font-style` (`blocks.rs`): the
//!   `height` and `float` declarations, every `width` declaration but a
//!   last one that is no width it keeps (an earlier one showed nothing),
//!   and the legacy `display`, `margin` (and its sides) and `max-width`
//!   that the locked declarations replace, which only placed the image
//!   beside the text. Everything else stays after the canonical
//!   declarations, byte for byte, in its order: a `display: none`, which
//!   hid the image and which the locked `display: block` would show, every
//!   other declaration (a border, a padding, a `vertical-align`), a last
//!   `width` declaration that is not a length of at least 1 px (`auto`,
//!   `50%`, `20em`, `0px`), and a declaration the reader cannot read. A
//!   last `width: auto` stays too: it showed the image at its natural
//!   width over a `width` attribute, which the shape takes instead. A
//!   `width` attribute that is not a whole number of px stays as
//!   well when nothing else sized the image. N7 removes each of these as a
//!   removal that no step planned, except a value it drops as certainly
//!   invalid (`width: 100%px`, D160 reading 2), so `apply` stops on the row
//!   until an `N7-accept` entry accepts it or an editor fixes the post
//!   before the final dump. The maintainer chose N7's review over N5b's own
//!   counted removal on 2026-11-15, "Leave them to N7's review, as in
//!   D146", the answer given for the attributes of the marks N5 renames
//!   (data-migration.md §7.4, reading (f); research: DECISIONS R39 (D168)).
//!   Any other attribute (`title`, `border`, `hspace`, a class other than
//!   `left` and `right`, ...) is left in place for N7 too, as for every
//!   attribute outside content-v1.
//! - A change to `width`, `height` or `style` counts as
//!   [`Rule::ImageShaped`], so a body whose images change always has a hit
//!   (V5(b)).
//!
//! **The place.** Each image ends up alone in a `<p>` at the top level,
//! without attributes once N7 has removed what N5b left for its review:
//!
//! - **At the top level.** An image that stands there outside a paragraph
//!   gets one ([`Rule::ImageWrapped`]).
//! - **In a top-level paragraph's running text**, the image's parent being
//!   that paragraph, the image splits the paragraph, as the editor's own
//!   media placement does (editor.md §5.2 step 11a, which §9 points N5b
//!   to): the text before it stays in the paragraph, the text after it
//!   moves into a new one with a copy of the paragraph's attributes (its
//!   alignment), and the image paragraph stands between them. The halves
//!   are trimmed at the split, ASCII white space (space, tab, LF, FF, CR)
//!   off the text that meets the image, going into the inline elements at
//!   that edge, and an inline element that held only that white space goes
//!   with it; the walk stops at a `br` or any other element that is not
//!   inline formatting. A half that shows nothing (no text but white space,
//!   no `br`, no other element than empty inline ones) is dropped
//!   ([`Rule::ImageParagraphSplit`]). U+00A0 is text, as in the editor's
//!   whitespace rule (editor.md §3.8) and its paste filter's trimming, so
//!   a half that holds only `&nbsp;` stays, and so does a half with a `br`
//!   at the split (`<p><img>&nbsp;</p>` gives `<p>&nbsp;</p>` after the
//!   image paragraph, `<p><img><br>szöveg</p>` gives `<p><br>szöveg</p>`):
//!   each adds a line that the legacy inline layout did not show, counted
//!   only as the split (data-migration.md §7.4, reading (h)). The
//!   paragraph's images are placed in one go, in their order. An image
//!   alone in its paragraph with nothing but white space, comments or other
//!   images beside it, or in an aligned paragraph, is given a paragraph of
//!   its own ([`Rule::ImageWrapped`]): the old one without its alignment,
//!   the `text-align` declarations of its `style`, which show nothing more
//!   once the image is centred. Its other attributes (a class, an `id`,
//!   another declaration) stay, for N7's review (D168), so a paragraph that
//!   holds only the image and no alignment is left as it is.
//! - **Anywhere else**, inside a heading, a link, inline formatting, a
//!   cell, a list item, a quote or any other element, the image moves into
//!   its own paragraph in front of the top-level block that holds it
//!   ([`Rule::ImageMovedBeforeBlock`]), which is also where Angular's page
//!   renderer shows it today (it moves every `<img>` in front of its parent
//!   element, ADR 0013). A link around it is dropped: the image no longer
//!   links anywhere, and the body is counted for review
//!   ([`Rule::ImageLinkDropped`]; an `a` without `href` is no link). What
//!   the image leaves behind is tidied as the paste filter tidies a removed
//!   picture (editor.md §5.3): an inline element, a paragraph, a `div`, a
//!   heading or a table's `caption` on its way up to the top-level block
//!   that shows nothing without it goes with it, so no empty link, mark,
//!   heading or caption is left; a list item, a cell or a quote keeps its
//!   place, left empty. One emptied paragraph stays, emptied: a list item's
//!   paragraph in front of the item's lists, with nothing before it, which
//!   content-v1 keeps (`<li><p></p><ul>…</ul></li>`, editor.md §3.7,
//!   "Tightening", D148), because a list item that holds only its lists
//!   loads in the editor as an empty item and a sibling list. A list item
//!   that the move leaves with only its lists in another way
//!   (`<li><img><ul>…</ul></li>`) becomes `<li><ul>…</ul></li>`, the legacy
//!   shape to which N5b's tightening rule, which normaliser-tables builds,
//!   gives that empty paragraph (data-migration.md §7.1, N5b (4)). The
//!   white space on either side of the image stays where it was, for N5b's
//!   whitespace rule (editor.md §3.8), which normaliser-tables builds too.
//!
//! "Inside inline formatting" and "inside running text" are read apart, as
//! §7.1 and §9 list them: an image whose parent is the top-level paragraph
//! splits it, and one inside a mark, a span or a link inside that paragraph
//! moves in front of it, with the mark's text staying together. A lone
//! image in a mark (`<p><strong><img></strong></p>`) ends up the same
//! either way.
//!
//! **Order.** The image rules run after N5, so the marks they see are
//! content-v1's (`strong` for `b`, the italic heading's `em`), and after N3,
//! whose centred paragraph around an image (`<center><img></center>`
//! becomes `<p style="text-align: center"><img></p>`) they turn into the
//! plain image paragraph; the image is centred anyway. N6 runs after them
//! and rewrites the dead host's `src` in place.
//!
//! **Fixed point.** The canonical image paragraph is what the rules write,
//! and they leave it as it is, with no hit: its paragraph has no attributes
//! and holds only the image, and its shape reads back as itself. Each split
//! or move puts the image into that paragraph, and what the rules leave for
//! N7's review (a declaration, a `width` attribute, a lone paragraph's
//! attributes) and a width over the bound are gone once N7 has run, so the
//! next pass finds nothing to place: P4 holds as for the other steps (see
//! the documentation of [`super`]). The rules alone are idempotent too: what
//! they leave is read the same way again.
//!
//! **Cost.** Each image walks its ancestors once, to find its top-level
//! block, any link on the way and the elements its move may leave empty.
//! The images of one paragraph split it in one go, so each node of the
//! paragraph moves once, and trimming looks only at the edges of each half.
//! The elements that the moves leave empty are removed in one walk over the
//! body after every image has its place, from the inside out, which asks
//! each node once whether it shows something and keeps the answer: a block
//! that many images leave, with the white space and empty marks of each
//! left beside the others, is not scanned again once per image, which
//! would grow with the square of the number of images. The step takes time
//! in proportion to the body's size plus its images times their depth
//! (`tests/normalise/cost.rs`). Like everything under `src/domain/`, it is
//! pure (ADR 0008).

use std::collections::{HashMap, HashSet};

use ego_tree::{NodeId, NodeRef};
use html5ever::local_name;
use html5ever::tendril::StrTendril;
use scraper::node::Element;
use scraper::{Html, Node};

use super::blocks;
use super::dead_host;
use super::dom;
use super::marks::without_class;
use super::report::{NormaliseReport, Rule};
use crate::domain::rich_text::css::declarations::{self, Component, Declaration};
use crate::domain::rich_text::css::grammar::{self, Grammar};
use crate::domain::rich_text::css::sanitize_style;
use crate::domain::rich_text::css::values::{self, Hundredths};
use crate::domain::rich_text::policy::{
    REMOVED_WITH_CONTENT, SanitizePolicy, ValueRule, element_rule,
};
use crate::domain::rich_text::url_rules;

/// The inline elements that show nothing once they hold nothing: marks,
/// spans, links and the other phrasing elements that only style or annotate
/// their text. A `br`, an image, a `q` (which shows its quotation marks) or
/// any block shows something even when empty.
#[rustfmt::skip]
const INLINE_WRAPPERS: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "big", "cite", "code", "data", "del", "dfn", "em", "font",
    "i", "ins", "kbd", "mark", "s", "samp", "small", "span", "strike", "strong", "sub", "sup",
    "time", "tt", "u", "var",
];

/// The blocks that go with an image when it was all they showed: a
/// paragraph, a `div`, a heading and a table's `caption`, whose text N5b's
/// caption rule would otherwise turn into an empty paragraph. A list item,
/// a cell and a quote keep their place, as the paste filter keeps an
/// emptied cell (editor.md §5.3).
const EMPTIED_BLOCKS: &[&str] = &["p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "caption"];

/// The largest whole number of px that a [`Hundredths`] holds, and so the
/// widest `width` attribute N5b reads: a wider one is no width it can keep.
const MAX_WHOLE_PX: u32 = u32::MAX / 100;

/// The classes that floated a legacy image (data-migration.md §7.1, N5:
/// `img.left` and `img.right`).
const FLOAT_CLASSES: [&str; 2] = ["left", "right"];

/// The `align` values that float an image in a browser (WHATWG HTML,
/// "Rendering", §15.4.3).
const FLOAT_ALIGNS: [&str; 2] = ["left", "right"];

/// The `style` properties whose legacy value the locked declarations of the
/// canonical image replace, as planned changes (see "What N7 reviews" in the
/// module documentation).
#[rustfmt::skip]
const REPLACED_PROPERTIES: &[&str] = &[
    "display", "margin", "margin-top", "margin-right", "margin-bottom", "margin-left", "max-width",
];

/// What content-v1 allows an image's size to be, read from the policy, so
/// that a policy change moves these bounds too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Bounds {
    /// The narrowest width an image's `style` may declare (1 px).
    min_width: Hundredths,
    /// The widest width an image's `style` may declare (2000 px, editor.md
    /// §3.3); a wider legacy width is kept and reported.
    pub(super) max_width: Hundredths,
    /// The smallest value of the `width` and `height` attributes (1).
    min_size: u32,
    /// The largest value of the `width` and `height` attributes (5000,
    /// editor.md §3.2).
    max_size: u32,
    /// The style of an image without a width: its four locked declarations.
    locked: String,
}

impl Bounds {
    /// The bounds of the content-v1 policy.
    ///
    /// # Panics
    ///
    /// When the policy no longer sizes an image by a `width` declaration in
    /// px and integer `width` and `height` attributes, which its own tests
    /// pin; this step would then need to change with it.
    pub(super) fn of_policy() -> Self {
        let width = grammar::element_style("img")
            .and_then(|style| style.property("width"))
            .expect("content-v1 sizes an image by its width");
        let Grammar::Px {
            min: min_width,
            max: max_width,
        } = width.grammar
        else {
            panic!(
                "content-v1's image width is a range of px, not {:?}",
                width.grammar
            );
        };
        let size = element_rule("img")
            .and_then(|rule| rule.attribute("width"))
            .expect("content-v1 keeps an image's width attribute");
        let ValueRule::Integer {
            min: min_size,
            max: max_size,
            ..
        } = size.rule
        else {
            panic!(
                "content-v1's image width attribute is an integer, not {:?}",
                size.rule
            );
        };
        let locked = sanitize_style("img", "")
            .css
            .expect("content-v1 locks four declarations of every image");
        Self {
            min_width,
            max_width,
            min_size,
            max_size,
            locked,
        }
    }

    /// The width that the `style` attribute value `style` declares, as N5b
    /// reads it (see "The width" in the module documentation), or `None`.
    ///
    /// - `style`: an image's `style` attribute value.
    pub(super) fn styled_width(&self, style: &str) -> Option<Hundredths> {
        LegacyStyle::read(style, self).width
    }

    /// Whether `value` lies in the range of the `width` and `height`
    /// attributes.
    ///
    /// - `value`: a whole number of px.
    fn holds_size(&self, value: u32) -> bool {
        (self.min_size..=self.max_size).contains(&value)
    }
}

/// Applies N5b's image rules (see the module documentation) to every image
/// of `html`, in document order, and counts what they changed in `report`.
///
/// - `html`: a parsed body, after N5.
/// - `report`: the report of the current run.
pub(super) fn shape_images(html: &mut Html, report: &mut NormaliseReport) {
    let bounds = Bounds::of_policy();
    let policy = SanitizePolicy::content_v1();
    // Taken before any edit: a move or a split never removes an image, and
    // an image keeps its id wherever it goes.
    let images: Vec<NodeId> = dom::elements_named(html, "img")
        .into_iter()
        .filter(|&id| is_placeable(dom::node(html, id), &policy))
        .collect();
    // The ancestors that the moved images left, which may show nothing any
    // more. They are removed in one walk once every image has its place
    // ("Cost" in the module documentation). Until then an emptied mark
    // stays where it was: a later split walks past it as past any empty
    // inline element, and counts it as showing nothing, so the result is
    // the same as removing it at once.
    let mut left = HashSet::new();
    for image in images {
        place(html, image, &policy, &mut left, report);
        dom::edit_element(html, image, |element| shape(element, &bounds, report));
    }
    remove_emptied(html, &left);
}

/// Whether `node` is an image that N5b places and shapes (see "Which
/// images" in the module documentation): an HTML `img` whose `src` the
/// sanitiser keeps ([`keeps_src`]), outside every element that the
/// sanitiser removes with its content and every foreign element.
///
/// - `node`: a node of a parsed body.
/// - `policy`: the sanitiser's policy, whose image rule judges the `src`.
pub(super) fn is_placeable(node: NodeRef<'_, Node>, policy: &SanitizePolicy) -> bool {
    dom::html_name(node) == Some("img")
        && node
            .value()
            .as_element()
            .and_then(|image| image.attr("src"))
            .is_some_and(|src| keeps_src(src, policy))
        && node.ancestors().all(|ancestor| match ancestor.value() {
            // A foreign element has no HTML name, so `svg`, `math` and an
            // HTML element inside their `foreignObject` all fail here.
            Node::Element(_) => {
                dom::html_name(ancestor).is_some_and(|name| !REMOVED_WITH_CONTENT.contains(&name))
            }
            _ => true,
        })
}

/// Whether the sanitiser keeps an image whose `src` is `src` once N6 has
/// run: its image rule ([`url_rules::image_src`]) accepts the value, read
/// as the root-relative path that N6 writes for one on the dead host's
/// `/storage/` ([`dead_host::rewritten`]). N6 runs after N5b in the same
/// pass, so without that reading a dead-host image would wait one pass to
/// be placed.
///
/// - `src`: an image's `src`, with character references decoded.
/// - `policy`: the sanitiser's policy.
fn keeps_src(src: &str, policy: &SanitizePolicy) -> bool {
    url_rules::image_src(policy, dead_host::rewritten(src).unwrap_or(src)).is_some()
}

/// Puts the image `image` into its own paragraph at the top level (see
/// "The place" in the module documentation).
///
/// - `html`: a parsed body.
/// - `image`: a placeable image of it.
/// - `policy`: the sanitiser's policy, which tells the placeable images.
/// - `left`: the ancestors that moved images left, for [`remove_emptied`].
/// - `report`: the report of the current run.
fn place(
    html: &mut Html,
    image: NodeId,
    policy: &SanitizePolicy,
    left: &mut HashSet<NodeId>,
    report: &mut NormaliseReport,
) {
    let root = dom::content(html);
    let node = dom::node(html, image);
    let parent = node
        .parent()
        .expect("an image of the body has a parent")
        .id();
    if parent == root {
        dom::wrap(html, &[image], dom::new_element(local_name!("p"), &[]));
        report.record(Rule::ImageWrapped);
        return;
    }
    let top = node
        .ancestors()
        .find(|ancestor| ancestor.parent().is_some_and(|above| above.id() == root))
        .expect("an image below the top level lies inside a top-level node")
        .id();
    if parent == top && dom::html_name(dom::node(html, top)) == Some("p") {
        split_paragraph(html, top, policy, left, report);
    } else {
        move_before(html, image, top, left, report);
    }
}

/// Splits the top-level paragraph `paragraph` at every image that is its
/// child, and gives each its own paragraph (see "In a top-level
/// paragraph's running text" in the module documentation). A paragraph that
/// is already the canonical image paragraph is left alone.
///
/// - `html`: a parsed body.
/// - `paragraph`: a `p` at the top level that has an image as a child.
/// - `policy`: the sanitiser's policy, which tells the placeable images.
/// - `left`: the ancestors that moved images left; one of them that shows
///   nothing is read as already gone, as [`remove_emptied`] removes it.
/// - `report`: the report of the current run.
fn split_paragraph(
    html: &mut Html,
    paragraph: NodeId,
    policy: &SanitizePolicy,
    left: &HashSet<NodeId>,
    report: &mut NormaliseReport,
) {
    // The children between the images: one segment more than images. An
    // emptied mark that an earlier image left here stays in the paragraph
    // and out of every segment: it is removed at the end, and a paragraph
    // that holds nothing else beside its image is already the image's own,
    // as it would be had the mark gone at once.
    let mut images = Vec::new();
    let mut segments = vec![Vec::new()];
    for child in dom::node(html, paragraph).children() {
        if is_placeable(child, policy) {
            images.push(child.id());
            segments.push(Vec::new());
        } else if (!left.contains(&child.id()) || shows(child))
            && let Some(segment) = segments.last_mut()
        {
            segment.push(child.id());
        }
    }
    let element = dom::node(html, paragraph)
        .value()
        .as_element()
        .expect("a paragraph is an element")
        .clone();
    // The paragraph of an image that shows nothing beside it: the old one
    // without its alignment, which the centred image shows anyway. Its
    // other attributes stay for N7's review (D168).
    let mut lone = element.clone();
    let aligned = cut_alignment(&mut lone);
    if !aligned && images.len() == 1 && segments.iter().all(Vec::is_empty) {
        return;
    }
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            trim(html, segment, Side::Start);
        }
        if index < images.len() {
            trim(html, segment, Side::End);
        }
    }
    // Trimming may have removed a segment's white space; what is left is
    // still a child of the paragraph.
    let segments: Vec<Vec<NodeId>> = segments
        .into_iter()
        .map(|segment| {
            segment
                .into_iter()
                .filter(|&id| is_child_of(html, id, paragraph))
                .collect()
        })
        .collect();
    let shown: Vec<bool> = segments
        .iter()
        .map(|segment| segment.iter().any(|&id| shows(dom::node(html, id))))
        .collect();
    let rule = if shown.contains(&true) {
        Rule::ImageParagraphSplit
    } else {
        Rule::ImageWrapped
    };
    let mut anchor = paragraph;
    for (index, &image) in images.iter().enumerate() {
        // In a split, the halves keep the paragraph's attributes, so the
        // image's own paragraph has none.
        let own = if rule == Rule::ImageWrapped {
            lone.clone()
        } else {
            dom::new_element(local_name!("p"), &[])
        };
        anchor = insert_after(html, anchor, own);
        append(html, anchor, &[image]);
        report.record(rule);
        let after = &segments[index + 1];
        if shown[index + 1] {
            anchor = insert_after(html, anchor, element.clone());
            append(html, anchor, after);
        } else {
            for &id in after {
                detach(html, id);
            }
        }
    }
    if !shown[0] {
        detach(html, paragraph);
    }
}

/// Cuts the alignment out of the paragraph `paragraph`, the `text-align`
/// declarations of its `style`, and returns whether it had any. They are
/// cut out whole, as N5's italic-heading rule cuts `font-style`
/// (`blocks.rs`), so the rest of the `style` stays byte for byte for N7,
/// and a `style` left with nothing goes. By N5b, the paragraph holds its
/// alignment only there: N5 has mapped or dropped its `align`, and N3
/// wrote a centre it inherited in front of its `style`.
///
/// - `paragraph`: the paragraph element, edited in place.
fn cut_alignment(paragraph: &mut Element) -> bool {
    let Some(style) = paragraph.attr("style") else {
        return false;
    };
    let pieces = blocks::declaration_texts(style);
    let kept: Vec<(&str, bool)> = pieces
        .iter()
        .copied()
        .filter(|(text, _)| {
            !matches!(Piece::read(text), Piece::Read(declaration) if declaration.name == "text-align")
        })
        .collect();
    if kept.len() == pieces.len() {
        return false;
    }
    let rest = blocks::join_declarations(&kept);
    if rest.is_empty() {
        dom::remove_attribute(paragraph, "style");
    } else {
        dom::set_attribute(paragraph, "style", &rest);
    }
    true
}

/// Moves the image `image` into a new paragraph in front of the top-level
/// block `top`, drops a link around it, and notes in `left` the ancestors
/// it left, which may show nothing any more (see "Anywhere else" in the
/// module documentation).
///
/// - `html`: a parsed body.
/// - `image`: a placeable image below `top`.
/// - `top`: the top-level node that holds it.
/// - `left`: the ancestors that moved images left, for [`remove_emptied`].
/// - `report`: the report of the current run.
fn move_before(
    html: &mut Html,
    image: NodeId,
    top: NodeId,
    left: &mut HashSet<NodeId>,
    report: &mut NormaliseReport,
) {
    let mut linked = false;
    for ancestor in dom::node(html, image).ancestors() {
        linked |= is_link(ancestor);
        left.insert(ancestor.id());
        if ancestor.id() == top {
            break;
        }
    }
    html.tree
        .get_mut(top)
        .expect("an id of this tree")
        .insert_before(Node::Element(dom::new_element(local_name!("p"), &[])))
        .append_id(image);
    report.record(Rule::ImageMovedBeforeBlock);
    if linked {
        report.record(Rule::ImageLinkDropped);
    }
}

/// Whether `node` is a link: an HTML `a` with an `href`. An `a` without one
/// is an anchor, which links nowhere.
///
/// - `node`: a node of a parsed body.
fn is_link(node: NodeRef<'_, Node>) -> bool {
    dom::html_name(node) == Some("a")
        && node
            .value()
            .as_element()
            .is_some_and(|link| link.attr("href").is_some())
}

/// What [`remove_emptied`] does with one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emptied {
    /// The node stays; whether a reader sees anything of it.
    Stays(bool),
    /// An element that a moved image left and that shows nothing: it goes,
    /// with what is left inside it.
    Goes,
    /// The paragraph a list item needs in front of its lists: it stays,
    /// emptied of what is left inside it.
    Kept,
}

/// Removes every node of `left` that is an inline element, a paragraph, a
/// `div`, a heading or a caption and shows nothing any more (see "Anywhere
/// else" in the module documentation), except a list item's paragraph in
/// front of the item's lists, which stays, emptied ([`opens_list_item`]).
///
/// One walk over the body, from the inside out, asks each node once and
/// keeps the answer, so the white space and the empty marks that many
/// images leave in one block are looked at once, not once per image
/// ("Cost" in the module documentation). The result is the one that
/// removing what each image leaves, right after its move and from its old
/// parent upwards, would give: a node of `left` is asked only after
/// everything below it has had its turn, and an element that stays, such
/// as a list item, shows something to the elements above it.
///
/// - `html`: a parsed body, with every image in its place.
/// - `left`: the ancestors that moved images left, up to and including
///   their top-level blocks.
fn remove_emptied(html: &mut Html, left: &HashSet<NodeId>) {
    if left.is_empty() {
        return;
    }
    // Pre-order puts every node before its descendants, so the reverse
    // order meets each node after everything below it.
    let order: Vec<NodeId> = dom::node(html, dom::content(html))
        .descendants()
        .map(|node| node.id())
        .collect();
    let mut showing: HashMap<NodeId, bool> = HashMap::with_capacity(order.len());
    for id in order.into_iter().rev() {
        let outcome = {
            let node = dom::node(html, id);
            match node.value() {
                Node::Text(text) => Emptied::Stays(!dom::is_white_space(text)),
                Node::Element(_) => {
                    let name = dom::html_name(node);
                    let inline = name.is_some_and(|name| INLINE_WRAPPERS.contains(&name));
                    let removable = left.contains(&id)
                        && name.is_some_and(|name| inline || EMPTIED_BLOCKS.contains(&name));
                    if !inline && !removable {
                        // Any other element shows something, even empty.
                        Emptied::Stays(true)
                    } else {
                        // A child that went is no longer a child, and every
                        // child left was asked before its parent.
                        let content = node
                            .children()
                            .any(|child| showing.get(&child.id()).copied().unwrap_or(false));
                        if removable && !content {
                            if opens_list_item(node) {
                                Emptied::Kept
                            } else {
                                Emptied::Goes
                            }
                        } else {
                            Emptied::Stays(!inline || content)
                        }
                    }
                }
                _ => Emptied::Stays(false),
            }
        };
        match outcome {
            Emptied::Stays(shown) => {
                showing.insert(id, shown);
            }
            Emptied::Goes => detach(html, id),
            Emptied::Kept => {
                for child in dom::children(html, id) {
                    detach(html, child);
                }
                showing.insert(id, true);
            }
        }
    }
}

/// Whether the emptied paragraph `paragraph` is a list item's paragraph in
/// front of the item's lists, with nothing shown before it: content-v1
/// keeps it, empty (`<li><p></p><ul>…</ul></li>`; editor.md §3.7,
/// "Tightening", D148), because an item that holds only its lists loads in
/// the editor as an empty item followed by a sibling list, so the next save
/// would change the bytes.
///
/// - `paragraph`: an element that shows nothing.
fn opens_list_item(paragraph: NodeRef<'_, Node>) -> bool {
    if dom::html_name(paragraph) != Some("p")
        || paragraph
            .parent()
            .is_none_or(|item| dom::html_name(item) != Some("li"))
        || paragraph.prev_siblings().any(shows)
    {
        return false;
    }
    let mut lists = 0_usize;
    for sibling in paragraph.next_siblings() {
        if matches!(dom::html_name(sibling), Some("ul" | "ol")) {
            lists += 1;
        } else if shows(sibling) {
            return false;
        }
    }
    lists > 0
}

/// Whether a reader sees anything of `node`: text that is not ASCII white
/// space, or an element that shows something even when empty (anything but
/// the [`INLINE_WRAPPERS`]: a `br`, an image, a block, a foreign element).
/// Comments show nothing.
///
/// - `node`: a node of a parsed body.
fn shows(node: NodeRef<'_, Node>) -> bool {
    node.descendants()
        .any(|descendant| match descendant.value() {
            Node::Text(text) => !dom::is_white_space(text),
            Node::Element(_) => {
                dom::html_name(descendant).is_none_or(|name| !INLINE_WRAPPERS.contains(&name))
            }
            _ => false,
        })
}

/// The edge of a run of nodes that [`trim`] works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Where the run starts.
    Start,
    /// Where the run ends.
    End,
}

/// Strips the ASCII white space at one edge of `run` (see "In a top-level
/// paragraph's running text" in the module documentation): text nodes that
/// hold only white space go, the first text with anything else loses its
/// white space at that edge, and the walk goes into the inline elements on
/// its way and stops at any other element. An inline element that held
/// something and shows nothing afterwards goes too.
///
/// - `html`: a parsed body.
/// - `run`: siblings in document order.
/// - `side`: the edge to trim.
fn trim(html: &mut Html, run: &[NodeId], side: Side) {
    // The top of the stack is the node nearest the edge.
    let mut stack: Vec<NodeId> = match side {
        Side::End => run.to_vec(),
        Side::Start => run.iter().rev().copied().collect(),
    };
    let mut entered = Vec::new();
    while let Some(id) = stack.pop() {
        let node = dom::node(html, id);
        match node.value() {
            Node::Text(text) => {
                let kept = match side {
                    Side::End => text.trim_end_matches(|c: char| c.is_ascii_whitespace()),
                    Side::Start => text.trim_start_matches(|c: char| c.is_ascii_whitespace()),
                };
                if kept.len() == text.len() {
                    break;
                }
                if kept.is_empty() {
                    detach(html, id);
                } else {
                    let kept = StrTendril::from(kept);
                    set_text(html, id, kept);
                    break;
                }
            }
            Node::Element(_)
                if dom::html_name(node).is_some_and(|name| INLINE_WRAPPERS.contains(&name)) =>
            {
                let children = dom::children(html, id);
                // An element that held nothing to begin with, such as an
                // anchor, is no wrapper of the trimmed white space.
                if !children.is_empty() {
                    entered.push(id);
                }
                match side {
                    Side::End => stack.extend(children),
                    Side::Start => stack.extend(children.into_iter().rev()),
                }
            }
            Node::Element(_) => break,
            // A comment shows nothing: the walk goes past it.
            _ => {}
        }
    }
    // The walk enters an element before the elements inside it, so the
    // reverse order asks each one after those inside it: the content of
    // an element around an emptied one is gone by then, and the answer for
    // an element that stays is kept, so a deep chain of inline elements at
    // the edge is not scanned again for each element around it.
    let mut kept: HashSet<NodeId> = HashSet::new();
    for id in entered.into_iter().rev() {
        let visible = dom::node(html, id)
            .children()
            .any(|child| kept.contains(&child.id()) || shows(child));
        if visible {
            kept.insert(id);
        } else {
            detach(html, id);
        }
    }
}

/// What the last `height` declaration of an image's `style` says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum DeclaredHeight {
    /// The `style` declares no height that the reader reads, so the
    /// `height` attribute decides.
    #[default]
    None,
    /// `height: auto`, the image's own height for its width.
    Auto,
    /// A length, in px.
    Length(Hundredths),
    /// Another value, such as a percentage: not `auto`, and no length
    /// either.
    Other,
}

impl DeclaredHeight {
    /// The height in px, when the declaration is a length.
    fn length(self) -> Option<Hundredths> {
        match self {
            Self::Length(height) => Some(height),
            Self::None | Self::Auto | Self::Other => None,
        }
    }

    /// Whether a declaration other than `auto` set the height, which
    /// `height: auto` replaces ([`Rule::ImageHeightAuto`]).
    fn is_fixed(self) -> bool {
        matches!(self, Self::Length(_) | Self::Other)
    }
}

/// What the legacy `style` of an image says about its size, and what in it
/// no image rule plans for, which N5b leaves for N7's review (see "The
/// shape" and "What N7 reviews" in the module documentation).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LegacyStyle {
    /// The width of the last `width` declaration, when that is a length of
    /// at least the policy's narrowest width.
    width: Option<Hundredths>,
    /// The last `height` declaration. Any declaration that a browser reads
    /// overrides the `height` attribute, whatever its value.
    height: DeclaredHeight,
    /// Whether the last `float` declaration is anything but `none`.
    floated: bool,
    /// The declarations that no image rule plans for, as written and in
    /// their order, joined as [`blocks::join_declarations`] joins them;
    /// empty when there are none. N5b writes them after the canonical
    /// declarations, byte for byte, for N7 to judge (D168).
    unplanned: String,
}

/// One declaration of a `style`, as the sanitiser's declaration reader
/// reads it on its own.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    /// White space or a lone `;`: no declaration at all.
    Nothing,
    /// A declaration the reader cannot read (a comment, `calc()`, a
    /// missing value, ...).
    Unreadable,
    /// A declaration the reader reads.
    Read(Declaration),
}

impl Piece {
    /// Reads `text`, one declaration as [`blocks::declaration_texts`] cuts
    /// it.
    ///
    /// - `text`: the declaration as written, with its `;` if one ends it.
    fn read(text: &str) -> Self {
        let mut read = declarations::read(text);
        match (read.read.pop(), read.unreadable) {
            (Some(declaration), 0) if read.read.is_empty() => Self::Read(declaration),
            (None, 0) => Self::Nothing,
            _ => Self::Unreadable,
        }
    }
}

impl LegacyStyle {
    /// Reads an image's `style` with the sanitiser's declaration reader
    /// (editor.md §7.3), one declaration at a time, as N5's italic-heading
    /// rule reads a heading's (`blocks.rs`). Of several declarations of one
    /// property the last one counts, as in N7's cascade.
    ///
    /// The declarations that the image rules plan for are cut out whole:
    /// every `width` but a last one that is no width N5b keeps (an earlier
    /// one showed nothing, in a browser and in N7's cascade), every
    /// `height` and `float`, and the `display`, `margin` (with its sides)
    /// and `max-width` that the locked declarations replace. Every other
    /// one stays, byte for byte, in [`Self::unplanned`]: a border, a
    /// padding, a `vertical-align`, a `display: none` that hid the image, a
    /// last `width` that is not a length of at least the narrowest width
    /// (`auto`, `50%`, `20em`, `0px`), and a declaration the reader cannot
    /// read (data-migration.md §7.4, D168).
    ///
    /// - `style`: the attribute value, with character references decoded.
    /// - `bounds`: the policy's bounds on an image's size.
    fn read(style: &str, bounds: &Bounds) -> Self {
        let pieces: Vec<((&str, bool), Piece)> = blocks::declaration_texts(style)
            .into_iter()
            .map(|text| (text, Piece::read(text.0)))
            .collect();
        // Only the last `width` declaration sized the image.
        let last_width = pieces.iter().rposition(
            |(_, piece)| matches!(piece, Piece::Read(declaration) if declaration.name == "width"),
        );
        let mut legacy = Self::default();
        let mut unplanned = Vec::new();
        for (index, (text, piece)) in pieces.iter().enumerate() {
            let declaration = match piece {
                Piece::Nothing => continue,
                Piece::Unreadable => {
                    unplanned.push(*text);
                    continue;
                }
                Piece::Read(declaration) => declaration,
            };
            let single = match declaration.value.as_slice() {
                [component] => Some(component),
                _ => None,
            };
            let auto = matches!(single, Some(Component::Ident(value)) if value == "auto");
            match declaration.name.as_str() {
                "width" if Some(index) == last_width => {
                    legacy.width = single
                        .and_then(values::length_px)
                        .filter(|&width| width >= bounds.min_width);
                    // A last width that is no length N5b keeps showed a
                    // size that the canonical shape does not: N7 reviews
                    // it. `auto` included, since it showed the natural
                    // width over a `width` attribute, which the shape
                    // takes instead.
                    if legacy.width.is_none() {
                        unplanned.push(*text);
                    }
                }
                "width" => {}
                "height" => {
                    legacy.height = match single.and_then(values::length_px) {
                        Some(height) => DeclaredHeight::Length(height),
                        None if auto => DeclaredHeight::Auto,
                        None => DeclaredHeight::Other,
                    };
                }
                "float" => {
                    legacy.floated =
                        !matches!(single, Some(Component::Ident(value)) if value == "none");
                }
                // `display: none` hid the image, which the locked
                // `display: block` would show: no rule plans that, so N7
                // reviews it.
                "display" if matches!(single, Some(Component::Ident(value)) if value == "none") => {
                    unplanned.push(*text);
                }
                name if REPLACED_PROPERTIES.contains(&name) => {}
                _ => unplanned.push(*text),
            }
        }
        legacy.unplanned = blocks::join_declarations(&unplanned);
        legacy
    }
}

/// Height over width, as two whole numbers of the same unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ratio {
    /// The height.
    height: u64,
    /// The width; never zero.
    width: u64,
}

impl Ratio {
    /// The ratio `height` over `width`, or `None` when `width` is zero.
    ///
    /// - `height`, `width`: the two sides, in one unit.
    fn new(height: u32, width: u32) -> Option<Self> {
        (width > 0).then_some(Self {
            height: u64::from(height),
            width: u64::from(width),
        })
    }

    /// The height of an image `width` px wide with this ratio, rounded to
    /// a whole px, half up, or `None` when it does not fit in a `u32`.
    ///
    /// - `width`: a width in whole px.
    fn height_at(self, width: u32) -> Option<u32> {
        // round(w × h / d) = floor((2 × w × h + d) / (2 × d)). With w, h
        // and d each below 2^32, the numerator can pass 2^64, so it is
        // computed in u128.
        let numerator = 2 * u128::from(width) * u128::from(self.height) + u128::from(self.width);
        u32::try_from(numerator / (2 * u128::from(self.width))).ok()
    }
}

/// The `width`, `height` and `style` attributes of an image's canonical
/// shape (see "The shape" in the module documentation).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shape {
    /// The `width` attribute, or `None` for none.
    width: Option<u32>,
    /// The `height` attribute, or `None` for none.
    height: Option<u32>,
    /// Whether the attributes keep the ratio: a `height` attribute within
    /// their range follows from the width, whether or not it is written. A
    /// width over the bound writes no attributes, but its ratio is not lost
    /// to N5b: N7 reviews the dropped width (see "The width" in the module
    /// documentation).
    keeps_ratio: bool,
    /// The `style` attribute.
    style: String,
}

impl Shape {
    /// The canonical shape of an image shown `display` px wide with the
    /// ratio `ratio`.
    ///
    /// - `display`: the width in px, or `None` for the natural width.
    /// - `ratio`: the ratio to keep, or `None` for the natural ratio.
    /// - `bounds`: the policy's bounds on an image's size.
    fn canonical(display: Option<Hundredths>, ratio: Option<Ratio>, bounds: &Bounds) -> Self {
        let over_bound = display.is_some_and(|width| width > bounds.max_width);
        let style = match display {
            None => bounds.locked.clone(),
            // The sanitiser drops a width over the bound; it is kept here,
            // in the place and spelling the canonical order gives it, for
            // N7 to drop and report (editor.md §16 item 5).
            Some(width) if over_bound => format!("{}; width: {width}px", bounds.locked),
            Some(width) => sanitize_style("img", &format!("width: {width}px"))
                .css
                .expect("a width within the bounds is kept"),
        };
        let width = display
            .map(round_half_up)
            .filter(|&width| bounds.holds_size(width));
        let height = width
            .zip(ratio)
            .and_then(|(width, ratio)| ratio.height_at(width))
            .filter(|&height| bounds.holds_size(height));
        // The admin's node writes no attributes for a width over the
        // bound, which it does not read as a display width either: N7's
        // output, without that width, is then what the editor saves.
        let written = |value: Option<u32>| value.filter(|_| !over_bound);
        Self {
            width: written(width),
            height: written(height),
            keeps_ratio: height.is_some(),
            style,
        }
    }
}

/// `value` rounded to a whole number, half up, as the admin's `imageSize`
/// rounds a display width with `Math.round`.
///
/// - `value`: a non-negative number with two decimals.
fn round_half_up(value: Hundredths) -> u32 {
    let hundredths = value.hundredths();
    hundredths / 100 + u32::from(hundredths % 100 >= 50)
}

/// Gives the image `image` its canonical shape (see "The shape" in the
/// module documentation) and counts what changed.
///
/// - `image`: a placeable image.
/// - `bounds`: the policy's bounds on an image's size.
/// - `report`: the report of the current run.
fn shape(image: &mut Element, bounds: &Bounds, report: &mut NormaliseReport) {
    let legacy = image
        .attr("style")
        .map(|style| LegacyStyle::read(style, bounds))
        .unwrap_or_default();
    let floated = remove_float_markers(image) || legacy.floated;
    let size = |name: &str| {
        image
            .attr(name)
            .and_then(|value| size_attribute(value, bounds))
    };
    let (width_attribute, height_attribute) = (size("width"), size("height"));
    let display = legacy.width.or(width_attribute.map(Hundredths::whole));
    let ratio = match (legacy.height.length(), display) {
        (Some(height), Some(width)) => Ratio::new(height.hundredths(), width.hundredths()),
        _ => width_attribute
            .zip(height_attribute)
            .and_then(|(width, height)| Ratio::new(height, width)),
    };
    // The height the legacy markup fixed: a `height` declaration in a
    // length, or the `height` attribute when the `style` declares no
    // height, since any height declaration overrides the attribute.
    let fixed_height = match legacy.height {
        DeclaredHeight::None => height_attribute.is_some(),
        declared => declared.length().is_some(),
    };
    let target = Shape::canonical(display, ratio, bounds);
    // A fixed height that the attributes do not keep as a ratio is lost:
    // without a width there is no ratio, and a height outside the
    // attributes' range cannot be written (data-migration.md §7.4, reading
    // (g)).
    let lost_height = fixed_height && !target.keeps_ratio;
    // A `width` attribute that N5b cannot read, when nothing else sized
    // the image, stays for N7's review (D168); beside a width that N5b
    // reads, it is rewritten from that width.
    let width = match (target.width, display) {
        (Some(width), _) => Some(width.to_string()),
        (None, None) => image.attr("width").map(str::to_owned),
        (None, Some(_)) => None,
    };
    // What no image rule plans for follows the canonical declarations,
    // byte for byte, for N7's review (D168).
    let style = if legacy.unplanned.is_empty() {
        target.style
    } else {
        format!("{}; {}", target.style, legacy.unplanned)
    };
    let changed = [
        put(image, "width", width.as_deref()),
        put(
            image,
            "height",
            target.height.map(|height| height.to_string()).as_deref(),
        ),
        put(image, "style", Some(style.as_str())),
    ];
    if floated {
        report.record(Rule::ImageFloatRemoved);
    }
    if legacy.height.is_fixed() {
        report.record(Rule::ImageHeightAuto);
    }
    if lost_height {
        report.record(Rule::ImageSizingDropped);
    }
    if changed.contains(&true) {
        report.record(Rule::ImageShaped);
    }
}

/// Removes the float classes and a floating `align` from `image`, and
/// returns whether it had any (see "The float" in the module
/// documentation).
///
/// - `image`: an image element.
fn remove_float_markers(image: &mut Element) -> bool {
    let mut floated = false;
    for class in FLOAT_CLASSES {
        let rest = image
            .attr("class")
            .and_then(|classes| without_class(classes, class));
        if let Some(rest) = rest {
            floated = true;
            if rest.is_empty() {
                dom::remove_attribute(image, "class");
            } else {
                dom::set_attribute(image, "class", &rest);
            }
        }
    }
    let floating_align = image.attr("align").is_some_and(|align| {
        FLOAT_ALIGNS
            .iter()
            .any(|float| align.eq_ignore_ascii_case(float))
    });
    if floating_align {
        dom::remove_attribute(image, "align");
        floated = true;
    }
    floated
}

/// The value of a `width` or `height` attribute in whole px, read with the
/// syntax by which the sanitiser (`attrs.rs`) and the admin's
/// `readIntegerAttribute` read it, ASCII white space trimmed and one to ten
/// ASCII digits, from the attributes' smallest value (1) up to
/// [`MAX_WHOLE_PX`]; `None` otherwise. A value above the attributes'
/// largest (5000) is read too: a browser showed it, and as a width it is
/// kept in the `style` and reported (see "The width" in the module
/// documentation), while [`Shape::canonical`] writes the attributes only
/// within their range.
///
/// - `value`: the attribute's value.
/// - `bounds`: the policy's bounds on an image's size.
fn size_attribute(value: &str, bounds: &Bounds) -> Option<u32> {
    let digits = value.trim_ascii();
    if digits.is_empty() || digits.len() > 10 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u64 = digits.parse().ok()?;
    u32::try_from(number)
        .ok()
        .filter(|&number| (bounds.min_size..=MAX_WHOLE_PX).contains(&number))
}

/// Sets the attribute `name` of `element` to `value`, or removes it for
/// `None`, and returns whether that changed it.
///
/// - `element`: the element to change.
/// - `name`: the attribute's name, in no namespace.
/// - `value`: its new value, or `None` to remove it.
fn put(element: &mut Element, name: &str, value: Option<&str>) -> bool {
    if element.attr(name) == value {
        return false;
    }
    match value {
        Some(value) => dom::set_attribute(element, name, value),
        None => {
            dom::remove_attribute(element, name);
        }
    }
    true
}

/// Whether the node `id` is still a child of `parent`.
///
/// - `html`: a parsed body.
/// - `id`, `parent`: two nodes of it.
fn is_child_of(html: &Html, id: NodeId, parent: NodeId) -> bool {
    dom::node(html, id)
        .parent()
        .is_some_and(|above| above.id() == parent)
}

/// Inserts `element`, without children, right after the node `anchor`, and
/// returns its id.
///
/// - `html`: a parsed body.
/// - `anchor`: a node that has a parent.
/// - `element`: the new element.
fn insert_after(html: &mut Html, anchor: NodeId, element: Element) -> NodeId {
    html.tree
        .get_mut(anchor)
        .expect("an id of this tree")
        .insert_after(Node::Element(element))
        .id()
}

/// Moves the nodes `children`, in their order, to the end of `parent`.
///
/// - `html`: a parsed body.
/// - `parent`: the new parent.
/// - `children`: nodes of the tree, none of them `parent` or above it.
fn append(html: &mut Html, parent: NodeId, children: &[NodeId]) {
    let mut node = html.tree.get_mut(parent).expect("an id of this tree");
    for &child in children {
        node.append_id(child);
    }
}

/// Removes the node `id` from the tree, with everything below it.
///
/// - `html`: a parsed body.
/// - `id`: a node of it.
fn detach(html: &mut Html, id: NodeId) {
    html.tree.get_mut(id).expect("an id of this tree").detach();
}

/// Replaces the content of the text node `id` with `text`.
///
/// - `html`: a parsed body.
/// - `id`: a text node of it.
/// - `text`: the new content.
fn set_text(html: &mut Html, id: NodeId, text: StrTendril) {
    if let Node::Text(node) = html.tree.get_mut(id).expect("an id of this tree").value() {
        node.text = text;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{NormaliseError, WideImage, check_fixed_point, normalise};
    use super::*;
    use crate::domain::rich_text::search_text::html_to_search_text;
    use crate::domain::rich_text::{SanitizePolicy, Sanitizer};
    use proptest::prelude::*;
    use std::sync::LazyLock;

    /// The content-v1 sanitiser, built once for the tests of this module:
    /// the whole normaliser runs it as N7, and the proptests normalise
    /// thousands of bodies.
    static SANITIZER: LazyLock<Sanitizer> = LazyLock::new(sanitizer);

    /// The four locked declarations of every stored image (editor.md §3.3).
    const LOCKED: &str = "display: block; height: auto; margin: 0 auto; max-width: 100%";

    /// The image source the tests use.
    const SRC: &str = "/storage/uploads/posts/12/kep.jpg";

    /// N5b's image rules alone over `body`: the output and the report.
    fn n5b(body: &str) -> (String, NormaliseReport) {
        let mut html = dom::parse(body);
        let mut report = NormaliseReport::default();
        shape_images(&mut html, &mut report);
        (dom::serialise(&html), report)
    }

    /// The hits of `report`, in rule order.
    fn hits(report: &NormaliseReport) -> Vec<(Rule, usize)> {
        report.hits().collect()
    }

    /// N7's removals that no step planned in `report`, as the run report
    /// shows them, once per removal made, in the order of
    /// [`UnplannedRemoval`](super::super::UnplannedRemoval).
    fn unplanned(report: &NormaliseReport) -> Vec<String> {
        report
            .unplanned_removals()
            .flat_map(|(removal, count)| std::iter::repeat_n(removal.to_string(), count))
            .collect()
    }

    /// N7's removal of an image's `width` declaration over the bound.
    const WIDTH_DROPPED: &str = "style_declaration_rejected img[style] width";

    /// N7's removal of an image's `width` attribute that is no whole
    /// number of px within the attributes' range.
    const WIDTH_ATTRIBUTE_REMOVED: &str = "attribute_value_rejected img[width]";

    /// N7's removal of a `display: none` that hid an image.
    const DISPLAY_DROPPED: &str = "style_declaration_rejected img[style] display";

    /// The canonical image paragraph of [`SRC`] with the alt text `alt`,
    /// `width` px wide and `height` px high.
    fn canonical(alt: &str, width: u32, height: u32) -> String {
        format!(
            r#"<p><img src="{SRC}" alt="{alt}" width="{width}" height="{height}" style="{LOCKED}; width: {width}px"></p>"#
        )
    }

    /// The canonical image paragraph of [`SRC`] with the alt text `alt` and
    /// no size.
    fn without_size(alt: &str) -> String {
        format!(r#"<p><img src="{SRC}" alt="{alt}" style="{LOCKED}"></p>"#)
    }

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
    }

    #[test]
    fn the_canonical_image_paragraph_is_left_alone() {
        for body in [canonical("Kép", 350, 495), without_size("Kép")] {
            assert_eq!(n5b(&body), (body.clone(), NormaliseReport::default()));
        }
        // A missing or blank alt is kept as it is (ADR 0013 §2).
        for alt in [r#" alt="""#, ""] {
            let body = format!(
                r#"<p><img src="{SRC}"{alt} width="350" height="495" style="{LOCKED}; width: 350px"></p>"#
            );
            assert_eq!(n5b(&body).0, body);
        }
    }

    #[test]
    fn the_policy_gives_the_bounds() {
        let bounds = Bounds::of_policy();
        assert_eq!(bounds.min_width, Hundredths::whole(1));
        assert_eq!(bounds.max_width, Hundredths::whole(2000));
        assert_eq!((bounds.min_size, bounds.max_size), (1, 5000));
        assert_eq!(bounds.locked, LOCKED);
        // `height` has the range of `width`, which the shape relies on.
        let height = element_rule("img")
            .and_then(|rule| rule.attribute("height"))
            .expect("content-v1 keeps an image's height attribute");
        assert_eq!(
            height.rule,
            ValueRule::Integer {
                min: 1,
                max: 5000,
                default: None
            }
        );
    }

    #[test]
    fn a_legacy_size_becomes_height_auto_with_the_ratio_in_the_attributes() {
        // CKEditor 4's image dialog writes the size into the style.
        let (out, report) = n5b(&format!(
            r#"<p><img alt="Kép" src="{SRC}" style="height:300px; width:400px"></p>"#
        ));
        assert_eq!(out, canonical("Kép", 400, 300));
        assert_eq!(
            hits(&report),
            [(Rule::ImageHeightAuto, 1), (Rule::ImageShaped, 1)]
        );
    }

    #[test]
    fn a_width_above_the_column_is_kept() {
        let (out, _) = n5b(&format!(
            r#"<p><img alt="Tabló" src="{SRC}" style="width: 1200px; height: 800px"></p>"#
        ));
        assert_eq!(out, canonical("Tabló", 1200, 800));
    }

    #[test]
    fn a_width_above_the_bound_is_kept_for_n7_and_reported() {
        // N5b keeps the width in the style, as data-migration.md §5 keeps
        // every legacy value over a limit, and writes no attributes, as
        // the admin's node writes none for a width over the bound.
        let raw = format!(
            r#"<p><img alt="Panoráma" src="{SRC}" style="height:600px; width:2400px"></p>"#
        );
        let (out, report) = n5b(&raw);
        assert_eq!(
            out,
            format!(r#"<p><img src="{SRC}" alt="Panoráma" style="{LOCKED}; width: 2400px"></p>"#)
        );
        assert_eq!(
            hits(&report),
            [(Rule::ImageHeightAuto, 1), (Rule::ImageShaped, 1)],
            "N5b loses no ratio: N7 reviews the width"
        );
        // N7 drops the width, a removal that no step planned, so the import
        // stops on the body until a policy change raises the bound or an
        // `N7-accept` entry lets the width go (editor.md §16 item 5;
        // data-migration.md §7.4, reading (e)). The report lists the image.
        let out = normalise(&raw, &SANITIZER).expect("a body that settles");
        assert_eq!(out.html, without_size("Panoráma"));
        assert_eq!(unplanned(&out.report), [WIDTH_DROPPED]);
        assert_eq!(
            out.report.wide_images(),
            [WideImage {
                position: 1,
                src: SRC.to_owned(),
                width: Hundredths::whole(2400),
            }]
        );
        // What is left is the normaliser's fixed point, with no hit and no
        // wide image: the next run finds no width to drop.
        assert_eq!(check_fixed_point(&out.html, &SANITIZER), Ok(()));
        let again = normalise(&out.html, &SANITIZER).expect("a body that settles");
        assert!(!again.report.has_hits() && !again.report.has_unplanned_removals());
        assert_eq!(again.report.wide_images(), []);
        // Wider than the attributes allow: the same, and the ratio, which
        // no attribute could write, is lost (reading (g)).
        let (out, report) = n5b(&format!(
            r#"<p><img alt="a" src="{SRC}" style="height:3000px; width:6000px"></p>"#
        ));
        let kept = format!(r#"<p><img src="{SRC}" alt="a" style="{LOCKED}; width: 6000px"></p>"#);
        assert_eq!(out, kept);
        assert_eq!(report.count(Rule::ImageSizingDropped), 1);
        // A width only in the attribute is a width too, above the
        // attribute's own range as well: kept in the style for N7 and
        // listed, never dropped by N5b.
        for width in ["2400", "6000", " 42949672 "] {
            let raw = format!(r#"<p><img alt="a" src="{SRC}" width="{width}"></p>"#);
            let px = width.trim();
            assert_eq!(
                n5b(&raw).0,
                format!(r#"<p><img src="{SRC}" alt="a" style="{LOCKED}; width: {px}px"></p>"#),
                "{width}"
            );
            let out = normalise(&raw, &SANITIZER).expect("a body that settles");
            assert_eq!(out.html, without_size("a"), "{width}");
            assert_eq!(unplanned(&out.report), [WIDTH_DROPPED], "{width}");
            assert_eq!(
                out.report.wide_images(),
                [WideImage {
                    position: 1,
                    src: SRC.to_owned(),
                    width: Hundredths::whole(px.parse().expect("a number")),
                }],
                "{width}"
            );
            assert_eq!(out.report.count(Rule::ImageSizingDropped), 0, "{width}");
        }
        // Beyond what a width in hundredths of a px holds, it is no width
        // N5b reads: the attribute stays for N7, which removes it.
        let raw = format!(r#"<p><img alt="a" src="{SRC}" width="42949673"></p>"#);
        let (out, report) = n5b(&raw);
        assert_eq!(
            out,
            format!(r#"<p><img src="{SRC}" alt="a" width="42949673" style="{LOCKED}"></p>"#)
        );
        assert_eq!(hits(&report), [(Rule::ImageShaped, 1)]);
        let out = normalise(&raw, &SANITIZER).expect("a body that settles");
        assert_eq!(out.html, without_size("a"));
        assert_eq!(unplanned(&out.report), [WIDTH_ATTRIBUTE_REMOVED]);
        assert_eq!(out.report.wide_images(), []);
    }

    #[test]
    fn the_ratio_is_the_box_the_legacy_markup_showed() {
        let shaped =
            |attributes: &str| n5b(&format!(r#"<p><img src="{SRC}" alt="a"{attributes}></p>"#)).0;
        // The style's own box wins over the natural size in the attributes.
        assert_eq!(
            shaped(r#" width="700" height="990" style="height: 300px; width: 400px""#),
            canonical("a", 400, 300)
        );
        // Without a height in the style, the attributes give the ratio, as
        // the editor reads them.
        assert_eq!(
            shaped(r#" width="700" height="990" style="width: 350px""#),
            canonical("a", 350, 495)
        );
        assert_eq!(
            shaped(r#" width="350" height="263""#),
            canonical("a", 350, 263)
        );
        // A height in the style and a width in the attribute make one box.
        assert_eq!(
            shaped(r#" width="500" style="height: 250px""#),
            canonical("a", 500, 250)
        );
        // No ratio: a width alone.
        assert_eq!(
            shaped(r#" style="width: 350px""#),
            format!(
                r#"<p><img src="{SRC}" alt="a" width="350" style="{LOCKED}; width: 350px"></p>"#
            )
        );
        // No width: the natural size, whatever the height was; the lost
        // height is counted (data-migration.md §7.4, reading (g)).
        let report =
            |attributes: &str| n5b(&format!(r#"<p><img src="{SRC}" alt="a"{attributes}></p>"#)).1;
        assert_eq!(shaped(r#" style="height: 300px""#), without_size("a"));
        assert_eq!(
            hits(&report(r#" style="height: 300px""#)),
            [
                (Rule::ImageHeightAuto, 1),
                (Rule::ImageShaped, 1),
                (Rule::ImageSizingDropped, 1)
            ]
        );
        assert_eq!(shaped(r#" height="300""#), without_size("a"));
        assert_eq!(
            hits(&report(r#" height="300""#)),
            [(Rule::ImageShaped, 1), (Rule::ImageSizingDropped, 1)]
        );
        // A height the ratio cannot write is lost too: 6000 px is above
        // the attributes' range.
        assert_eq!(
            shaped(r#" width="1500" height="6000""#),
            format!(
                r#"<p><img src="{SRC}" alt="a" width="1500" style="{LOCKED}; width: 1500px"></p>"#
            )
        );
        assert_eq!(
            report(r#" width="1500" height="6000""#).count(Rule::ImageSizingDropped),
            1
        );
        // A height declaration overrides the attribute, so an `auto` one
        // fixed no height, and nothing is lost.
        assert_eq!(
            shaped(r#" height="300" style="height: auto""#),
            without_size("a")
        );
        assert_eq!(
            hits(&report(r#" height="300" style="height: auto""#)),
            [(Rule::ImageShaped, 1)]
        );
    }

    #[test]
    fn the_last_height_declaration_decides() {
        // `height: auto` after a fixed height leaves nothing to turn into
        // `height: auto`, and the attributes keep their own ratio.
        let (out, report) = n5b(&format!(
            r#"<p><img src="{SRC}" alt="a" width="300" height="150" style="width: 300px; height: 200px; height: auto"></p>"#
        ));
        assert_eq!(out, canonical("a", 300, 150));
        assert_eq!(hits(&report), [(Rule::ImageShaped, 1)]);
        // The other way round, the fixed height is the box.
        let (out, report) = n5b(&format!(
            r#"<p><img src="{SRC}" alt="a" width="300" height="150" style="width: 300px; height: auto; height: 200px"></p>"#
        ));
        assert_eq!(out, canonical("a", 300, 200));
        assert_eq!(
            hits(&report),
            [(Rule::ImageHeightAuto, 1), (Rule::ImageShaped, 1)]
        );
    }

    #[test]
    fn a_height_that_does_not_divide_is_rounded_half_up() {
        // 100 × 2 / 3 = 66.67, written as 67, as the admin's `imageSize`
        // rounds it with `Math.round`; rounding down would write 66.
        assert_eq!(
            Ratio::new(2, 3).and_then(|ratio| ratio.height_at(100)),
            Some(67)
        );
        assert_eq!(
            Ratio::new(1, 3).and_then(|ratio| ratio.height_at(100)),
            Some(33)
        );
        assert_eq!(
            Ratio::new(1, 2).and_then(|ratio| ratio.height_at(5)),
            Some(3)
        );
        assert_eq!(
            n5b(&format!(
                r#"<p><img src="{SRC}" alt="a" width="300" height="200" style="width: 100px"></p>"#
            ))
            .0,
            canonical("a", 100, 67)
        );
    }

    #[test]
    fn other_length_units_are_read_in_px_and_rounded_as_the_editor_rounds() {
        let shaped = |style: &str| {
            n5b(&format!(
                r#"<p><img src="{SRC}" alt="a" style="{style}"></p>"#
            ))
            .0
        };
        assert_eq!(
            shaped("width: 300pt; height: 150pt"),
            canonical("a", 400, 200)
        );
        // 10 cm is 377.95 px: the style keeps the hundredths, the attribute
        // is rounded half up, and the height follows the rounded width.
        assert_eq!(
            shaped("width: 10cm; height: 5cm"),
            format!(
                r#"<p><img src="{SRC}" alt="a" width="378" height="189" style="{LOCKED}; width: 377.95px"></p>"#
            )
        );
        assert_eq!(round_half_up(Hundredths::from_hundredths(37_749)), 377);
        assert_eq!(round_half_up(Hundredths::from_hundredths(37_750)), 378);
        assert_eq!(
            round_half_up(Hundredths::from_hundredths(u32::MAX)),
            42_949_673
        );
    }

    #[test]
    fn a_float_is_removed_so_the_image_is_centred() {
        let (out, report) = n5b(&format!(
            r#"<p><img alt="a" src="{SRC}" style="float:left; height:200px; margin: 5px 10px; width:300px"></p>"#
        ));
        assert_eq!(out, canonical("a", 300, 200));
        assert_eq!(
            hits(&report),
            [
                (Rule::ImageFloatRemoved, 1),
                (Rule::ImageHeightAuto, 1),
                (Rule::ImageShaped, 1)
            ]
        );
        // The classes `left` and `right`, matched exactly; other classes
        // stay for N7.
        let body = canonical("a", 350, 495);
        let with = |attribute: &str| body.replacen("<img ", &format!("<img {attribute} "), 1);
        for (attribute, left) in [
            (r#"class="left""#, ""),
            (r#"class="right""#, ""),
            (r#"class="kiemelt  left""#, r#" class="kiemelt""#),
            (r#"align="RIGHT""#, ""),
            (r#"align="left""#, ""),
        ] {
            let (out, report) = n5b(&with(attribute));
            assert_eq!(
                out,
                body.replace(r#"px"></p>"#, &format!(r#"px"{left}></p>"#)),
                "{attribute}"
            );
            assert_eq!(hits(&report), [(Rule::ImageFloatRemoved, 1)], "{attribute}");
        }
        // Class names are case-sensitive, and other `align` values do not
        // float an image: both stay for N7.
        for attribute in [r#"class="Left""#, r#"class="lefty""#, r#"align="middle""#] {
            let (out, report) = n5b(&with(attribute));
            assert!(out.contains(attribute), "{attribute}: {out}");
            assert!(!report.has_hits(), "{attribute}");
        }
        // `float: none` floats nothing; the style is still rewritten.
        let (_, report) = n5b(&format!(
            r#"<p><img alt="a" src="{SRC}" style="float: none; width: 350px"></p>"#
        ));
        assert_eq!(hits(&report), [(Rule::ImageShaped, 1)]);
    }

    #[test]
    fn what_no_image_rule_plans_is_left_for_n7s_review() {
        // N5b cuts only the declarations it plans for and writes the rest
        // after the canonical ones, byte for byte, and leaves a `width`
        // attribute it cannot read when nothing else sized the image; N7
        // removes each as a removal that no step planned, so the import
        // stops on the row until an `N7-accept` entry accepts it
        // (data-migration.md §7.4, D168).
        let cases: [(&str, &str, &[&str]); 14] = [
            (
                r#" style="border-style: solid; border-width: 2px; width: 350px""#,
                r#" width="350" style="LOCKED; width: 350px; border-style: solid; border-width: 2px""#,
                &[
                    "style_declaration_rejected img[style] border-style",
                    "style_declaration_rejected img[style] border-width",
                ],
            ),
            (
                r#" style="vertical-align: middle; width: 350px""#,
                r#" width="350" style="LOCKED; width: 350px; vertical-align: middle""#,
                &["style_declaration_rejected img[style] vertical-align"],
            ),
            // A last width that is no length gives way to the attribute in
            // the shape, and stays after it for N7.
            (
                r#" width="200" height="100" style="height: 100px; width: 50%""#,
                r#" width="200" height="100" style="LOCKED; width: 200px; width: 50%""#,
                &[WIDTH_DROPPED],
            ),
            (
                r#" style="width: 20em""#,
                r#" style="LOCKED; width: 20em""#,
                &[WIDTH_DROPPED],
            ),
            (
                r#" style="width: 0px""#,
                r#" style="LOCKED; width: 0px""#,
                &[WIDTH_DROPPED],
            ),
            // `width: auto` showed the natural width over the attribute,
            // which the shape takes instead; without an attribute, N7
            // reviews it all the same, as it reviews any valid value that
            // content-v1 drops (D160 reading 2).
            (
                r#" width="350" style="width: auto""#,
                r#" width="350" style="LOCKED; width: 350px; width: auto""#,
                &[WIDTH_DROPPED],
            ),
            (
                r#" style="width: auto""#,
                r#" style="LOCKED; width: auto""#,
                &[WIDTH_DROPPED],
            ),
            // The last width declaration is the one that showed; the one
            // before it showed nothing and goes as planned.
            (
                r#" style="width: 350px; width: 50%""#,
                r#" style="LOCKED; width: 50%""#,
                &[WIDTH_DROPPED],
            ),
            // A declaration the reader cannot read stays as written.
            (
                r#" style="width: calc(100% - 2px)""#,
                r#" style="LOCKED; width: calc(100% - 2px)""#,
                &["style_declaration_rejected img[style] (unreadable)"],
            ),
            // A hidden image would be shown by the locked `display: block`.
            (
                r#" style="display: none; width: 350px""#,
                r#" width="350" style="LOCKED; width: 350px; display: none""#,
                &[DISPLAY_DROPPED],
            ),
            (
                r#" width="auto""#,
                r#" width="auto" style="LOCKED""#,
                &[WIDTH_ATTRIBUTE_REMOVED],
            ),
            (
                r#" width="35px""#,
                r#" width="35px" style="LOCKED""#,
                &[WIDTH_ATTRIBUTE_REMOVED],
            ),
            (
                r#" width="0""#,
                r#" width="0" style="LOCKED""#,
                &[WIDTH_ATTRIBUTE_REMOVED],
            ),
            // A value that is invalid CSS is a planned drop of N7's, so
            // nothing goes to review (D160, reading 2).
            (
                r#" style="width: 100%px""#,
                r#" style="LOCKED; width: 100%px""#,
                &[],
            ),
        ];
        for (attributes, shaped, removals) in cases {
            let raw = format!(r#"<p><img src="{SRC}" alt="a"{attributes}></p>"#);
            let (out, report) = n5b(&raw);
            assert_eq!(
                out,
                format!(
                    r#"<p><img src="{SRC}" alt="a"{}></p>"#,
                    shaped.replace("LOCKED", LOCKED)
                ),
                "{attributes}"
            );
            assert_eq!(report.count(Rule::ImageSizingDropped), 0, "{attributes}");
            let normalised = normalise(&raw, &SANITIZER).expect("a body that settles");
            assert_eq!(unplanned(&normalised.report), removals, "{attributes}");
            assert_eq!(
                check_fixed_point(&normalised.html, &SANITIZER),
                Ok(()),
                "{attributes}"
            );
        }
    }

    #[test]
    fn what_the_image_rules_plan_goes_without_review() {
        // A fixed height that cannot become a ratio is still N5b's own
        // counted drop (reading (g), D169): content-v1 locks
        // `height: auto`, so nothing could keep it.
        for attributes in [r#" style="height: 300px""#, r#" height="300""#] {
            let raw = format!(r#"<p><img src="{SRC}" alt="a"{attributes}></p>"#);
            let (out, report) = n5b(&raw);
            assert_eq!(out, without_size("a"), "{attributes}");
            assert_eq!(report.count(Rule::ImageSizingDropped), 1, "{attributes}");
            let normalised = normalise(&raw, &SANITIZER).expect("a body that settles");
            assert!(!normalised.report.has_unplanned_removals(), "{attributes}");
        }
        // The locked declarations replace these as planned, and an invalid
        // width attribute beside a width in the style is rewritten from
        // that width: nothing is left for N7.
        for attributes in [
            r#" style="display: inline; margin: 5px; margin-left: 3px; max-width: 50%; float: none; width: 350px""#,
            r#" width="auto" style="width: 350px""#,
            // An earlier width that a later one overrides showed nothing.
            r#" style="width: 50%; width: 350px""#,
            r#" style="width: auto; width: 350px""#,
            // A height with a width becomes the ratio.
            r#" width="350" height="300""#,
            r#" style="height: 300px; width: 400px""#,
        ] {
            let raw = format!(r#"<p><img src="{SRC}" alt="a"{attributes}></p>"#);
            let (out, report) = n5b(&raw);
            assert_eq!(report.count(Rule::ImageSizingDropped), 0, "{attributes}");
            for gone in ["inline", "5px", "3px", "50%", "float", "auto\""] {
                assert!(!out.contains(gone), "{attributes}: {out}");
            }
            let normalised = normalise(&raw, &SANITIZER).expect("a body that settles");
            assert!(
                !normalised.report.has_unplanned_removals(),
                "{attributes}: {:?}",
                normalised.report
            );
        }
    }

    #[test]
    fn other_attributes_are_left_for_n7() {
        let raw = format!(
            r#"<p><img src="{SRC}" alt="a" title="Cím" border="0" hspace="5" style="height:100px; width:100px"></p>"#
        );
        let (out, report) = n5b(&raw);
        assert_eq!(
            out,
            format!(
                r#"<p><img src="{SRC}" alt="a" width="100" height="100" style="{LOCKED}; width: 100px" border="0" hspace="5" title="Cím"></p>"#
            )
        );
        assert_eq!(
            hits(&report),
            [(Rule::ImageHeightAuto, 1), (Rule::ImageShaped, 1)]
        );
        let normalised = normalise(&raw, &SANITIZER).expect("a body that settles");
        assert_eq!(normalised.html, canonical("a", 100, 100));
        assert_eq!(
            unplanned(&normalised.report),
            [
                "attribute_not_allowed img[border]",
                "attribute_not_allowed img[hspace]",
                "attribute_not_allowed img[title]",
            ]
        );
    }

    #[test]
    fn a_top_level_image_gets_its_own_paragraph() {
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        let (out, report) = n5b(&format!("{bare}<p>Szöveg</p>{bare}"));
        assert_eq!(out, format!("{image}<p>Szöveg</p>{image}"));
        assert_eq!(hits(&report), [(Rule::ImageWrapped, 2)]);
    }

    #[test]
    fn a_paragraph_that_shows_only_the_image_becomes_the_image_paragraph() {
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        for body in [
            format!(r#"<p style="text-align: center">{bare}</p>"#),
            format!(r#"<p style="text-align: center; text-align: inherit">{bare}</p>"#),
            format!("<p> \n{bare}<!-- c --> </p>"),
        ] {
            let (out, report) = n5b(&body);
            assert_eq!(out, image, "{body}");
            assert_eq!(hits(&report), [(Rule::ImageWrapped, 1)], "{body}");
        }
        // Two images, nothing else: one paragraph each.
        let (out, report) = n5b(&format!("<p>{bare}{bare}</p>"));
        assert_eq!(out, format!("{image}{image}"));
        assert_eq!(hits(&report), [(Rule::ImageWrapped, 2)]);
    }

    #[test]
    fn a_lone_image_paragraph_keeps_its_other_attributes_for_n7() {
        // Only the alignment goes, as planned; a class, an `id` and any
        // other declaration stay, byte for byte, for N7's review (D168).
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        let body = format!(
            r#"<p class="x" id="y" style="text-align: center; color: #c00000;text-align:left">{bare}</p>"#
        );
        let (out, report) = n5b(&body);
        assert_eq!(
            out,
            format!(r#"<p style="color: #c00000" class="x" id="y">{bare}</p>"#)
        );
        assert_eq!(hits(&report), [(Rule::ImageWrapped, 1)]);
        let normalised = normalise(&body, &SANITIZER).expect("a body that settles");
        assert_eq!(normalised.html, image);
        assert_eq!(
            unplanned(&normalised.report),
            [
                "attribute_not_allowed p[class]",
                "attribute_not_allowed p[id]",
                "style_rejected p[style] color",
            ]
        );
        // Without an alignment, N5b changes nothing: the paragraph is the
        // image's own, and its attributes are N7's.
        let body = format!(r#"<p class="x">{bare}</p>"#);
        assert_eq!(n5b(&body), (body.clone(), NormaliseReport::default()));
        let normalised = normalise(&body, &SANITIZER).expect("a body that settles");
        assert_eq!(normalised.html, image);
        assert_eq!(
            unplanned(&normalised.report),
            ["attribute_not_allowed p[class]"]
        );
        // With white space or other images beside it, each image gets a
        // copy of the paragraph without its alignment.
        let (out, report) = n5b(&format!(
            r#"<p class="x" style="text-align: center"> {bare}{bare} </p>"#
        ));
        assert_eq!(
            out,
            format!(r#"<p class="x">{bare}</p><p class="x">{bare}</p>"#)
        );
        assert_eq!(hits(&report), [(Rule::ImageWrapped, 2)]);
        // In a split, the halves keep the attributes and the image's own
        // paragraph has none, as before.
        let (out, report) = n5b(&format!(
            r#"<p class="x" style="text-align: center">Szöveg {bare}</p>"#
        ));
        assert_eq!(
            out,
            format!(r#"<p style="text-align: center" class="x">Szöveg</p>{image}"#)
        );
        assert_eq!(hits(&report), [(Rule::ImageParagraphSplit, 1)]);
    }

    #[test]
    fn an_image_in_running_text_splits_the_paragraph() {
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        // Both halves keep the paragraph's attributes, trimmed at the split.
        let (out, report) = n5b(&format!(
            r#"<p style="text-align: justify">Előtte {bare} utána</p>"#
        ));
        assert_eq!(
            out,
            format!(
                r#"<p style="text-align: justify">Előtte</p>{image}<p style="text-align: justify">utána</p>"#
            )
        );
        assert_eq!(hits(&report), [(Rule::ImageParagraphSplit, 1)]);
        // A half that shows nothing goes; a `br` shows something.
        for (body, expected) in [
            (
                format!("<p>{bare} Szöveg</p>"),
                format!("{image}<p>Szöveg</p>"),
            ),
            (
                format!("<p>Szöveg\n{bare}</p>"),
                format!("<p>Szöveg</p>{image}"),
            ),
            (
                format!("<p>sor<br>{bare}</p>"),
                format!("<p>sor<br></p>{image}"),
            ),
            (
                format!("<p><strong>Félkövér </strong> {bare}<em> dőlt</em></p>"),
                format!("<p><strong>Félkövér</strong></p>{image}<p><em>dőlt</em></p>"),
            ),
            (
                format!("<p>a<strong> <em>\t</em></strong>{bare}</p>"),
                format!("<p>a</p>{image}"),
            ),
            (
                format!("<p>a&nbsp;{bare}</p>"),
                format!("<p>a&nbsp;</p>{image}"),
            ),
        ] {
            let (out, report) = n5b(&body);
            assert_eq!(out, expected, "{body}");
            assert_eq!(hits(&report), [(Rule::ImageParagraphSplit, 1)], "{body}");
        }
        // An empty anchor at the edge is no wrapper of the white space.
        let (out, _) = n5b(&format!(r#"<p>a<a name="x"></a> {bare}</p>"#));
        assert_eq!(out, format!(r#"<p>a<a name="x"></a></p>{image}"#));
    }

    #[test]
    fn a_half_with_a_line_break_or_a_no_break_space_stays() {
        // Step 11a keeps a half with text or a `br`, trims only ASCII white
        // space, and U+00A0 is text (data-migration.md §7.4, reading (h)).
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        for (body, expected) in [
            // The trim stops at the `br`: the space before it stays.
            (
                format!("<p>a <br> {bare}</p>"),
                format!("<p>a <br></p>{image}"),
            ),
            (format!("<p><br>{bare}</p>"), format!("<p><br></p>{image}")),
            (
                format!("<p>{bare}<br>szöveg</p>"),
                format!("{image}<p><br>szöveg</p>"),
            ),
            // CKEditor 4 writes `&nbsp;` beside an image in its own line.
            (
                format!("<p>{bare}&nbsp;</p>"),
                format!("{image}<p>&nbsp;</p>"),
            ),
            (
                format!("<p>&nbsp; {bare}</p>"),
                format!("<p>&nbsp;</p>{image}"),
            ),
        ] {
            let (out, report) = n5b(&body);
            assert_eq!(out, expected, "{body}");
            assert_eq!(hits(&report), [(Rule::ImageParagraphSplit, 1)], "{body}");
        }
        // The trim goes past an inline element only.
        let (out, _) = n5b(&format!("<p>a <strong>b <br> </strong>{bare}</p>"));
        assert_eq!(out, format!("<p>a <strong>b <br></strong></p>{image}"));
    }

    #[test]
    fn a_deep_chain_of_inline_elements_at_the_split_is_trimmed_once() {
        // The edge's white space is inside the innermost element; the text
        // that stays keeps every element around it.
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        let depth = 50;
        let (out, _) = n5b(&format!(
            "<p>{bare}{}  szöveg{}</p>",
            "<em><strong>".repeat(depth),
            "</strong></em>".repeat(depth)
        ));
        assert_eq!(
            out,
            format!(
                "{image}<p>{}szöveg{}</p>",
                "<em><strong>".repeat(depth),
                "</strong></em>".repeat(depth)
            )
        );
        // Siblings at the edge: the emptied one goes, the other stays.
        let (out, _) = n5b(&format!("<p>{bare}<strong> </strong><em> x</em></p>"));
        assert_eq!(out, format!("{image}<p><em>x</em></p>"));
    }

    #[test]
    fn the_images_of_a_paragraph_split_it_in_their_order() {
        let first = canonical("első", 350, 495);
        let second = canonical("második", 175, 175);
        let bare = |image: &str| {
            image
                .trim_start_matches("<p>")
                .trim_end_matches("</p>")
                .to_owned()
        };
        let (out, report) = n5b(&format!("<p>a {} b {} c</p>", bare(&first), bare(&second)));
        assert_eq!(out, format!("<p>a</p>{first}<p>b</p>{second}<p>c</p>"));
        assert_eq!(hits(&report), [(Rule::ImageParagraphSplit, 2)]);
        // A split image first, then one in a mark of the rest: the order of
        // the images stays, and the hole stays for the whitespace rule.
        let (out, report) = n5b(&format!(
            "<p>a {} b <strong>{}</strong> c</p>",
            bare(&first),
            bare(&second)
        ));
        assert_eq!(out, format!("<p>a</p>{first}{second}<p>b  c</p>"));
        assert_eq!(
            hits(&report),
            [
                (Rule::ImageParagraphSplit, 1),
                (Rule::ImageMovedBeforeBlock, 1)
            ]
        );
    }

    #[test]
    fn an_image_in_a_heading_or_inline_formatting_moves_before_its_block() {
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        for (body, expected) in [
            (
                format!("<h2>Cím{bare}</h2>"),
                format!("{image}<h2>Cím</h2>"),
            ),
            // A heading that showed only the image goes with it.
            (
                format!("<h2>{bare}</h2><p>x</p>"),
                format!("{image}<p>x</p>"),
            ),
            (
                format!("<p>Szöveg <strong>{bare}kiemelt</strong> vége</p>"),
                format!("{image}<p>Szöveg <strong>kiemelt</strong> vége</p>"),
            ),
            (
                format!(r#"<p><span style="color: #c00000"><em>{bare}</em></span></p>"#),
                image.clone(),
            ),
            (
                format!("<h2><em>{bare}Dőlt cím</em></h2>"),
                format!("{image}<h2><em>Dőlt cím</em></h2>"),
            ),
        ] {
            let (out, report) = n5b(&body);
            assert_eq!(out, expected, "{body}");
            assert_eq!(hits(&report), [(Rule::ImageMovedBeforeBlock, 1)], "{body}");
        }
    }

    #[test]
    fn a_link_around_an_image_is_dropped_and_counted() {
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        // A thumbnail linked to the large picture: the link goes with it.
        let (out, report) = n5b(&format!(
            r#"<p><a href="/storage/uploads/posts/12/nagy.jpg" target="_blank" rel="noopener noreferrer">{bare}</a></p>"#
        ));
        assert_eq!(out, image);
        assert_eq!(
            hits(&report),
            [
                (Rule::ImageMovedBeforeBlock, 1),
                (Rule::ImageLinkDropped, 1)
            ]
        );
        // A link with text keeps it.
        let (out, report) = n5b(&format!(r#"<p><a href="/hirek/12">Hír{bare}</a> vége</p>"#));
        assert_eq!(
            out,
            format!(r#"{image}<p><a href="/hirek/12">Hír</a> vége</p>"#)
        );
        assert_eq!(report.count(Rule::ImageLinkDropped), 1);
        // A top-level link, and one further up than the mark.
        let (out, report) = n5b(&format!(r#"<a href="/x"><strong>{bare}</strong></a>"#));
        assert_eq!(out, image);
        assert_eq!(report.count(Rule::ImageLinkDropped), 1);
        // An anchor links nowhere.
        let (out, report) = n5b(&format!(r#"<p><a name="kep">{bare}</a></p>"#));
        assert_eq!(out, image);
        assert_eq!(hits(&report), [(Rule::ImageMovedBeforeBlock, 1)]);
    }

    #[test]
    fn an_image_in_a_cell_item_or_quote_moves_before_the_top_level_block() {
        let first = canonical("első", 350, 495);
        let second = canonical("második", 175, 175);
        let a = first.trim_start_matches("<p>").trim_end_matches("</p>");
        let b = second.trim_start_matches("<p>").trim_end_matches("</p>");
        for (body, expected) in [
            (
                format!("<ul><li>pont{a}</li><li>{b}</li></ul>"),
                format!("{first}{second}<ul><li>pont</li><li></li></ul>"),
            ),
            (
                format!("<blockquote><p>idézet</p><p>{a}</p></blockquote>"),
                format!("{first}<blockquote><p>idézet</p></blockquote>"),
            ),
            (
                format!("<blockquote>idézet{a}</blockquote>"),
                format!("{first}<blockquote>idézet</blockquote>"),
            ),
            (
                format!("<table><tbody><tr><td>a{a}</td><td>{b}</td></tr></tbody></table>"),
                format!(
                    "{first}{second}<table><tbody><tr><td>a</td><td></td></tr></tbody></table>"
                ),
            ),
            (
                format!("<div><ul><li><p><strong>{a}</strong></p></li></ul></div>"),
                format!("{first}<div><ul><li></li></ul></div>"),
            ),
            (
                format!("<div>{a}</div><p>x</p>"),
                format!("{first}<p>x</p>"),
            ),
        ] {
            let (out, report) = n5b(&body);
            assert_eq!(out, expected, "{body}");
            assert!(
                report
                    .hits()
                    .all(|(rule, _)| rule == Rule::ImageMovedBeforeBlock),
                "{body}: {report:?}"
            );
        }
    }

    #[test]
    fn a_list_item_keeps_the_paragraph_in_front_of_its_lists() {
        // `<li><ul>` alone would load as an empty item and a sibling list
        // (D148), so the emptied paragraph stays, emptied.
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        for (body, expected) in [
            (
                format!("<ul><li><p><strong>{bare}</strong> </p><ul><li>x</li></ul></li></ul>"),
                format!("{image}<ul><li><p></p><ul><li>x</li></ul></li></ul>"),
            ),
            (
                format!(
                    "<ol><li>\n<p>{bare}</p>\n<ol><li>x</li></ol><ul><li>y</li></ul></li></ol>"
                ),
                format!(
                    "{image}<ol><li>\n<p></p>\n<ol><li>x</li></ol><ul><li>y</li></ul></li></ol>"
                ),
            ),
            // Two emptied paragraphs: only the first one is the item's own.
            (
                format!("<ul><li><p>{bare}</p><p>{bare}</p><ul><li>x</li></ul></li></ul>"),
                format!("{image}{image}<ul><li><p></p><ul><li>x</li></ul></li></ul>"),
            ),
            // With text of its own, or no list after it, the item needs no
            // empty paragraph.
            (
                format!("<ul><li>pont<p>{bare}</p><ul><li>x</li></ul></li></ul>"),
                format!("{image}<ul><li>pont<ul><li>x</li></ul></li></ul>"),
            ),
            (
                format!("<ul><li><p>{bare}</p></li></ul>"),
                format!("{image}<ul><li></li></ul>"),
            ),
            (
                format!("<ul><li><p>{bare}</p><p>szöveg</p><ul><li>x</li></ul></li></ul>"),
                format!("{image}<ul><li><p>szöveg</p><ul><li>x</li></ul></li></ul>"),
            ),
            // A paragraph after the lists is not the item's first one; the
            // item is left as `<li><ul>`, for N5b's tightening rule
            // (normaliser-tables), as is an item whose image stood loose.
            (
                format!("<ul><li><ul><li>x</li></ul><p>{bare}</p></li></ul>"),
                format!("{image}<ul><li><ul><li>x</li></ul></li></ul>"),
            ),
            (
                format!("<ul><li>{bare}<ul><li>x</li></ul></li></ul>"),
                format!("{image}<ul><li><ul><li>x</li></ul></li></ul>"),
            ),
        ] {
            let (out, report) = n5b(&body);
            assert_eq!(out, expected, "{body}");
            assert!(
                report
                    .hits()
                    .all(|(rule, _)| rule == Rule::ImageMovedBeforeBlock),
                "{body}: {report:?}"
            );
        }
    }

    #[test]
    fn an_emptied_caption_goes() {
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        let (out, _) = n5b(&format!(
            "<table><caption><strong>{bare}</strong></caption><tbody><tr><td>a</td></tr></tbody></table>"
        ));
        assert_eq!(
            out,
            format!("{image}<table><tbody><tr><td>a</td></tr></tbody></table>")
        );
        let (out, _) = n5b(&format!(
            "<table><caption>Cím{bare}</caption><tbody><tr><td>a</td></tr></tbody></table>"
        ));
        assert_eq!(
            out,
            format!(
                "{image}<table><caption>Cím</caption><tbody><tr><td>a</td></tr></tbody></table>"
            )
        );
    }

    #[test]
    fn many_images_leave_a_block_in_one_clean_up() {
        // What the moves leave is removed once every image has its place:
        // the empty marks go, the white space between them stays for the
        // whitespace rule, and a block that shows nothing else goes.
        let image = canonical("a", 350, 495);
        let bare = image.trim_start_matches("<p>").trim_end_matches("</p>");
        let (out, report) = n5b(&format!(
            "<p> <strong>{bare}</strong> <em><span>{bare}</span></em> </p>"
        ));
        assert_eq!(out, format!("{image}{image}"));
        assert_eq!(hits(&report), [(Rule::ImageMovedBeforeBlock, 2)]);
        let (out, _) = n5b(&format!(
            "<div><span></span><span>{bare}</span>szöveg<span></span><span>{bare}</span></div>"
        ));
        assert_eq!(
            out,
            format!("{image}{image}<div><span></span>szöveg<span></span></div>")
        );
        // An image moved out of a mark, then one in the same paragraph's
        // running text: the emptied mark counts as gone, so the paragraph
        // is already the second image's own, as the same body placed one
        // image at a time would give.
        let (out, report) = n5b(&format!("<p><strong>{bare}</strong>{bare}</p>"));
        assert_eq!(out, format!("{image}{image}"));
        assert_eq!(hits(&report), [(Rule::ImageMovedBeforeBlock, 1)]);
        let (out, report) = n5b(&format!("<p>a<strong>{bare}</strong> {bare} b</p>"));
        assert_eq!(out, format!("{image}<p>a</p>{image}<p>b</p>"));
        assert_eq!(
            hits(&report),
            [
                (Rule::ImageParagraphSplit, 1),
                (Rule::ImageMovedBeforeBlock, 1)
            ]
        );
    }

    #[test]
    fn images_no_reader_sees_or_without_source_are_left_for_n7() {
        for body in [
            r#"<p>x<img alt="nincs forrás">y</p>"#.to_owned(),
            format!(r#"<template><p>a<img src="{SRC}"></p></template>"#),
            format!(r#"<svg><foreignObject><p>a<img src="{SRC}"></p></foreignObject></svg>"#),
            // Elements the sanitiser removes with their content.
            format!(r#"<p>a<object><img src="{SRC}"></object>b</p>"#),
            format!(r#"<p>a<button><img src="{SRC}"></button>b</p>"#),
            format!(r#"<video><img src="{SRC}"></video>"#),
        ] {
            let (out, report) = n5b(&body);
            assert_eq!(out, dom::serialise(&dom::parse(&body)), "{body}");
            assert!(!report.has_hits(), "{body}");
        }
    }

    #[test]
    fn images_whose_source_n7_refuses_are_left_where_they_are() {
        // N7 removes them, so placing them first would split a paragraph or
        // drop a link for nothing and leave an empty paragraph behind.
        for src in [
            "https://example.com/kep.jpg",
            "",
            "kep.jpg",
            "/storage/uploads/posts/12/kep.svg",
            "/storage/uploads/posts/12/kep.jpg?v=2",
            "data:image/png;base64,iVBO",
        ] {
            for body in [
                format!(r#"<p>a <img src="{src}" alt="k" style="float: left"> b</p>"#),
                format!(r#"<p><a href="/x"><img src="{src}" alt="k"></a></p>"#),
            ] {
                let (out, report) = n5b(&body);
                assert_eq!(out, dom::serialise(&dom::parse(&body)), "{body}");
                assert!(!report.has_hits(), "{body}");
            }
        }
        // The dead host's `/storage/` is N6's to rewrite, and an absolute
        // address on an own host the sanitiser's to: both are placed.
        for src in [
            "https://backend.verseghy-gimnazium.net/storage/uploads/posts/12/kep.jpg",
            "https://verseghy-gimnazium.net/storage/uploads/posts/12/kep.jpg",
        ] {
            let (out, report) = n5b(&format!(r#"<p>a <img src="{src}" alt="k"> b</p>"#));
            assert_eq!(
                out,
                format!(r#"<p>a</p><p><img src="{src}" alt="k" style="{LOCKED}"></p><p>b</p>"#),
                "{src}"
            );
            assert_eq!(
                hits(&report),
                [(Rule::ImageParagraphSplit, 1), (Rule::ImageShaped, 1)],
                "{src}"
            );
        }
    }

    #[test]
    fn the_shapes_pass_the_sanitiser_and_settle() {
        let sanitizer = sanitizer();
        for raw in [
            format!(
                r#"<p><img alt="a" src="{SRC}" style="float:left; height:300px; width:400px" />Szöveg</p>"#
            ),
            format!(r#"<h2>Cím<img alt="a" src="{SRC}" width="350" height="263"></h2>"#),
            format!(
                r#"<ul><li>pont<img alt="a" src="{SRC}" style="width: 10cm; height: 5cm"></li></ul>"#
            ),
            format!(
                r#"<center><img alt="a" src="{SRC}" style="height:200px; width:300px"></center>"#
            ),
            format!(
                r#"<IMG SRC="https://backend.verseghy-gimnazium.net{SRC}" ALT="a" CLASS="right">"#
            ),
        ] {
            let out = normalise(&raw, &SANITIZER).expect("a body that settles");
            assert_eq!(
                check_fixed_point(&out.html, &sanitizer),
                Ok(()),
                "{raw}: {}",
                out.html
            );
        }
    }

    /// The image paragraph of [`SRC`] with the shape `shape`, as the rules
    /// write it.
    fn shaped_body(shape: &Shape) -> String {
        use std::fmt::Write as _;
        let mut attributes = String::new();
        for (name, value) in [("width", shape.width), ("height", shape.height)] {
            if let Some(value) = value {
                let _ = write!(attributes, r#" {name}="{value}""#);
            }
        }
        format!(
            r#"<p><img src="{SRC}" alt="a"{attributes} style="{}"></p>"#,
            shape.style
        )
    }

    #[test]
    fn a_shape_reads_back_as_itself() {
        let bounds = Bounds::of_policy();
        let ratios = [
            None,
            Ratio::new(495, 350),
            Ratio::new(1, 3),
            Ratio::new(3, 1),
        ];
        for width in [1, 2, 175, 350, 2000, 2001, 4999, 5000, 5001] {
            for ratio in ratios {
                let shape = Shape::canonical(Some(Hundredths::whole(width)), ratio, &bounds);
                let body = shaped_body(&shape);
                assert_eq!(
                    n5b(&body),
                    (body.clone(), NormaliseReport::default()),
                    "{body}"
                );
            }
        }
        assert_eq!(Ratio::new(1, 0), None);
        assert_eq!(
            Ratio::new(1, 3).and_then(|ratio| ratio.height_at(u32::MAX)),
            Some(1_431_655_765)
        );
        assert_eq!(
            Ratio::new(u32::MAX, 1).and_then(|ratio| ratio.height_at(2)),
            None
        );
    }

    #[test]
    fn size_attributes_are_read_as_the_sanitiser_reads_them() {
        let bounds = Bounds::of_policy();
        assert_eq!(size_attribute("350", &bounds), Some(350));
        assert_eq!(size_attribute(" 0350\n", &bounds), Some(350));
        assert_eq!(size_attribute("5000", &bounds), Some(5000));
        // Above the attributes' range: still a width a browser showed.
        assert_eq!(size_attribute("5001", &bounds), Some(5001));
        assert_eq!(size_attribute("42949672", &bounds), Some(MAX_WHOLE_PX));
        for bad in [
            "0",
            "00",
            "42949673",
            "4294967296",
            "-1",
            "+3",
            "35px",
            "3.5",
            "",
            " ",
            "99999999999",
            "١٢",
        ] {
            assert_eq!(size_attribute(bad, &bounds), None, "{bad:?}");
        }
    }

    #[test]
    fn a_body_with_images_is_not_refused() {
        // The moves and splits make no tag soup of their own.
        let raw = format!(
            r#"<a href="/x"><table><tr><td><img src="{SRC}"></td></tr></table></a><p><b><img src="{SRC}">x</b></p>"#
        );
        assert!(!matches!(
            normalise(&raw, &SANITIZER),
            Err(NormaliseError::NotNormalisable { .. })
        ));
    }

    #[test]
    fn many_images_in_marks_and_spans_leave_one_clean_up() {
        // The bodies of `tests/normalise/cost.rs` that time the clean-up:
        // N7 refuses them as too large once N5b has shaped their images,
        // so what N5b makes of them is checked here, on the rules alone.
        let image = format!(r#"<img src="{SRC}" alt="a">"#);
        let paragraph = without_size("a");
        let count = 20_000;
        let (out, report) = n5b(&format!(
            "<p>{}</p>",
            format!(" <strong>{image}</strong>").repeat(count)
        ));
        assert_eq!(out, paragraph.repeat(count));
        assert_eq!(
            hits(&report),
            [
                (Rule::ImageMovedBeforeBlock, count),
                (Rule::ImageShaped, count)
            ]
        );
        assert!(out.len() > SanitizePolicy::content_v1().max_input_bytes);
        let count = 16_000;
        let (out, report) = n5b(&format!(
            "<div>{}szöveg</div>",
            format!("<span></span><span>{image}</span>").repeat(count)
        ));
        assert_eq!(
            out,
            format!(
                "{}<div>{}szöveg</div>",
                paragraph.repeat(count),
                "<span></span>".repeat(count)
            )
        );
        assert_eq!(
            hits(&report),
            [
                (Rule::ImageMovedBeforeBlock, count),
                (Rule::ImageShaped, count)
            ]
        );
        assert!(out.len() > SanitizePolicy::content_v1().max_input_bytes);
    }

    /// Legacy image markup and the places it stands in.
    #[rustfmt::skip]
    const PIECES: &[&str] = &[
        "<img src=\"/storage/uploads/a.jpg\" alt=\"a\" style=\"height:300px; width:400px\">",
        "<img src=\"/storage/uploads/b.jpg\" class=\"left\" style=\"float: right; width: 2400px; height: 10px\">",
        "<img src=\"/storage/uploads/c.jpg\" align=\"left\" width=\"350\" height=\"495\">",
        "<IMG SRC=\"/storage/uploads/d.jpg\" STYLE=\"width: 300pt\" />", "<img alt=\"forrás nélkül\">",
        "<img src=\"https://example.com/e.jpg\" style=\"float: left\">",
        "<p>", "</p>", "<p style=\"text-align: center\">", "<h2>", "</h2>", "<a href=\"/x\">", "</a>",
        "<strong>", "</strong>", "<b>", "</b>", "<span style=\"color: #c00000\">", "</span>",
        "<ul><li>", "</li>", "</ul>", "<blockquote>", "</blockquote>", "<center>", "</center>",
        "<table><tr><td>", "</td></tr></table>", "<div>", "</div>", "<br>",
        "szöveg", "más", " ", "\n", "&nbsp;", "<!-- c -->",
    ];

    fn arb_body() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(PIECES), 0..30)
            .prop_map(|pieces| pieces.concat())
    }

    /// The pieces without tables, whose parse moves text out of them, for
    /// the property about words (as `arb_plain_legacy` in `super`).
    fn arb_plain_body() -> impl Strategy<Value = String> {
        let pieces: Vec<&str> = PIECES
            .iter()
            .copied()
            .filter(|piece| !piece.contains("table"))
            .collect();
        prop::collection::vec(prop::sample::select(pieces), 0..30)
            .prop_map(|pieces| pieces.concat())
    }

    /// The words of `html`'s search text, with no space between them.
    fn words(html: &str) -> String {
        html_to_search_text(html).split(' ').collect()
    }

    #[test]
    fn the_generated_markup_reaches_every_placement() {
        use proptest::strategy::ValueTree;
        let mut runner = proptest::test_runner::TestRunner::deterministic();
        let strategy = arb_body();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2_000 {
            let raw = strategy
                .new_tree(&mut runner)
                .expect("the generator always produces a value")
                .current();
            if let Ok(out) = normalise(&raw, &SANITIZER) {
                seen.extend(out.report.hits().map(|(rule, _)| rule));
            }
        }
        for rule in [
            Rule::ImageWrapped,
            Rule::ImageParagraphSplit,
            Rule::ImageMovedBeforeBlock,
            Rule::ImageLinkDropped,
            Rule::ImageFloatRemoved,
            Rule::ImageHeightAuto,
            Rule::ImageShaped,
        ] {
            assert!(seen.contains(&rule), "{rule:?} never fired");
        }
    }

    /// At least 512 cases, as the normaliser's other properties.
    fn config() -> ProptestConfig {
        let default = ProptestConfig::default();
        ProptestConfig {
            cases: default.cases.max(512),
            ..default
        }
    }

    proptest! {
        #![proptest_config(config())]

        // Every image the rules place ends up alone in a plain top-level
        // paragraph, in its canonical style, and the output settles (P4).
        #[test]
        fn every_image_stands_alone_in_a_top_level_paragraph(raw in arb_body()) {
            if let Ok(out) = normalise(&raw, &SANITIZER) {
                let html = dom::parse(&out.html);
                let root = dom::content(&html);
                let policy = SanitizePolicy::content_v1();
                for id in dom::elements_named(&html, "img") {
                    let image = dom::node(&html, id);
                    if !is_placeable(image, &policy) {
                        continue;
                    }
                    let paragraph = image.parent().expect("a parent");
                    prop_assert_eq!(dom::html_name(paragraph), Some("p"), "{:?}", out.html);
                    prop_assert_eq!(paragraph.parent().map(|p| p.id()), Some(root), "{:?}", out.html);
                    prop_assert!(paragraph.value().as_element().is_some_and(|p| p.attrs.is_empty()), "{:?}", out.html);
                    prop_assert_eq!(paragraph.children().count(), 1, "{:?}", out.html);
                    let style = image.value().as_element().and_then(|i| i.attr("style")).unwrap_or_default();
                    prop_assert!(style.starts_with(LOCKED), "{:?}", out.html);
                }
                let twice = normalise(&out.html, &SANITIZER)
                    .map_err(|error| TestCaseError::fail(format!("{raw:?}: the output was refused: {error}")))?;
                prop_assert_eq!(&twice.html, &out.html, "{:?}", raw);
                prop_assert!(!twice.report.has_hits(), "{:?}: {:?}", raw, twice.report);
                prop_assert!(!twice.report.has_unplanned_removals(), "{:?}: {:?}", raw, twice.report);
                // N7 dropped every width over the bound, so none is left to
                // list.
                prop_assert_eq!(twice.report.wide_images(), []);
            }
        }

        // The words a reader reads are kept: the rules move images and
        // remove only white space and what shows nothing.
        #[test]
        fn the_words_are_kept(raw in arb_plain_body()) {
            if let Ok(out) = normalise(&raw, &SANITIZER) {
                prop_assert_eq!(words(&out.html), words(&raw), "{:?}", out.html);
            }
        }

        // Any width and ratio the rules write read back as themselves.
        #[test]
        fn any_shape_is_a_fixed_point(width in 100u32..600_000, ratio in proptest::option::of((0u32..1_000_000, 1u32..1_000_000))) {
            let bounds = Bounds::of_policy();
            let ratio = ratio.and_then(|(height, width)| Ratio::new(height, width));
            let shape = Shape::canonical(Some(Hundredths::from_hundredths(width)), ratio, &bounds);
            let body = shaped_body(&shape);
            let (out, report) = n5b(&body);
            prop_assert_eq!(&out, &body);
            prop_assert!(!report.has_hits(), "{:?}", report);
        }

        // N5b cuts only the declarations its rules plan for and leaves the
        // rest after the canonical ones, byte for byte and in their order,
        // for N7's review (D168); what it leaves reads the same way again.
        #[test]
        fn what_no_rule_plans_follows_the_canonical_declarations(
            declarations in prop::collection::vec(prop::sample::select(STYLE_PIECES), 0..8),
        ) {
            let style = declarations.iter().map(|(text, _)| *text).collect::<Vec<_>>().join("; ");
            // The last `width` that the reader reads is planned when N5b
            // reads it as a length.
            let last_width = declarations
                .iter()
                .rposition(|(text, _)| text.starts_with("width") && !text.contains("calc"));
            let left: Vec<&str> = declarations
                .iter()
                .enumerate()
                .filter(|(index, (text, planned))| {
                    !planned || (Some(*index) == last_width && NO_KEPT_WIDTH.contains(text))
                })
                .map(|(_, (text, _))| *text)
                .collect();
            let (out, _) = n5b(&format!(r#"<p><img src="{SRC}" alt="a" style="{style}"></p>"#));
            let html = dom::parse(&out);
            let written = dom::elements_named(&html, "img")
                .first()
                .and_then(|&id| dom::node(&html, id).value().as_element()?.attr("style").map(str::to_owned))
                .unwrap_or_default();
            prop_assert!(written.starts_with(LOCKED), "{:?}", written);
            if left.is_empty() {
                // The canonical declarations alone: the locked ones and a
                // width.
                let rest = written.strip_prefix(LOCKED).unwrap_or_default();
                prop_assert!(
                    rest.is_empty() || (rest.starts_with("; width: ") && !rest[1..].contains(';')),
                    "{:?} from {:?}", written, style
                );
            } else {
                let tail = format!("; {}", left.join("; "));
                prop_assert!(written.ends_with(&tail), "{:?} from {:?}, wanted {:?}", written, style, tail);
            }
            let (again, report) = n5b(&out);
            prop_assert_eq!(&again, &out);
            prop_assert!(!report.has_hits(), "{:?}", report);
        }

        // A lone image's paragraph loses only its `text-align`
        // declarations; the rest of its `style` stays, byte for byte.
        #[test]
        fn only_the_alignment_is_cut_from_a_paragraph(
            declarations in prop::collection::vec(prop::sample::select(PARAGRAPH_PIECES), 0..6),
        ) {
            let style = declarations.join("; ");
            let mut paragraph = dom::new_element(local_name!("p"), &[("style", style.as_str())]);
            let aligned = declarations.iter().any(|text| text.starts_with("text-align"));
            prop_assert_eq!(cut_alignment(&mut paragraph), aligned);
            let rest: Vec<&str> = declarations
                .iter()
                .copied()
                .filter(|text| !text.starts_with("text-align"))
                .collect();
            let expected = blocks::join_declarations(
                &blocks::declaration_texts(&rest.join("; ")),
            );
            if !aligned {
                prop_assert_eq!(paragraph.attr("style"), Some(style.as_str()));
            } else if expected.is_empty() {
                prop_assert_eq!(paragraph.attr("style"), None);
            } else {
                prop_assert_eq!(paragraph.attr("style"), Some(expected.as_str()));
            }
        }
    }

    /// Declarations of an image's legacy `style`, each with whether an
    /// image rule plans for it. `width: 20em` and `width: auto` are planned
    /// only when another `width` follows them ([`NO_KEPT_WIDTH`]), which the
    /// property reads.
    const STYLE_PIECES: &[(&str, bool)] = &[
        ("width: 300px", true),
        ("width: 20em", true),
        ("width: auto", true),
        ("height: 200px", true),
        ("height:auto", true),
        ("float: left", true),
        ("margin: 5px 10px", true),
        ("max-width: 50%", true),
        ("display: inline", true),
        ("display: none", false),
        ("border: 1px solid #000000", false),
        ("padding:2px", false),
        ("vertical-align: middle", false),
        ("width: calc(100% - 2px)", false),
        ("/* c */ color: red", false),
    ];

    /// The `width` declarations of [`STYLE_PIECES`] that are no width N5b
    /// keeps: as the last `width`, each stays for N7's review (D168).
    const NO_KEPT_WIDTH: &[&str] = &["width: 20em", "width: auto"];

    /// Declarations of a paragraph's `style`.
    const PARAGRAPH_PIECES: &[&str] = &[
        "text-align: center",
        "text-align:left",
        "color: #c00000",
        "margin-left: 40px",
        "/* c */ text-align: right",
        "font-family: Arial",
    ];
}
