//! What the normaliser records about one body for the run report
//! (data-migration.md §7.2, §11.3).
//!
//! Two kinds of record, kept apart on purpose:
//!
//! - **Rule hits** count changes: each [`Rule`] that rewrote something,
//!   with the step of data-migration.md §7.1 it belongs to. They are added
//!   up over every pass of one run, like the sanitiser's removal report. A
//!   body that is already at the normaliser's fixed point has none, which is
//!   what `content_normalize` relies on to report zero changes (V13).
//! - **Observations** describe the normalised body without changing it: the
//!   occurrences of the dead host that rule N6 leaves alone, and the images
//!   without alt text (editor.md §9). Editors review both lists on the
//!   sign-off sheet (data-migration.md §8.2, §13 item 4). They are taken from
//!   the final pass only, so a body that needs two passes is not listed
//!   twice.
//!
//! A third record is neither: the **unplanned removals** of N7, what the
//! sanitiser removed that no step planned ([`UnplannedRemoval`]). They are
//! added up over the passes like rule hits, but they are not rule hits: the
//! import stops on a body that has any, unless an `N7-accept` entry of the
//! resolution file accepts them (data-migration.md §7.1 N7, §8.2;
//! `domain::import_mysql::n7_review`). The **wide images** ([`WideImage`])
//! name one kind of them by image: the images whose width over content-v1's
//! 2000 px bound N5b kept and N7 dropped (editor.md §16 item 5), which the
//! maintainer reads, because a policy change that raises the bound lets
//! such a body in too. They are read from the tree that N7 is given, since
//! its output no longer holds the width, and added up over the passes like
//! the removals; N7 drops the width in the pass that lists it, so an image
//! is listed once.
//!
//! The `snake_case` names of [`Rule::as_str`] are provisional in the same
//! way as the sanitiser's: the run report that the importer slices write
//! fixes them.

use std::collections::BTreeMap;
use std::fmt;

use crate::domain::rich_text::RemovalRule;
use crate::domain::rich_text::css::values::Hundredths;

/// A step of data-migration.md §7.1 that records rule hits. The steps that
/// later slices add (N1, N2 and N4) join this list with their rules, and
/// N5b's table and structure rules join [`Step::N5b`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    /// N3: `<center>` unwrapped into `text-align: center`.
    N3,
    /// N5: legacy presentational markup mapped onto content-v1: its block
    /// rules, then its inline rules, the legacy marks.
    N5,
    /// N5b: the content-v1 layout (decision 15b), so far its image rules:
    /// every image in the canonical image paragraph.
    N5b,
    /// N6: links to the dead host rewritten to root-relative.
    N6,
    /// N7: the sanitiser's planned changes (`sanitise.rs`).
    N7,
}

impl Step {
    /// The step's name in data-migration.md §7.1, such as `N3`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::N3 => "N3",
            Self::N5 => "N5",
            Self::N5b => "N5b",
            Self::N6 => "N6",
            Self::N7 => "N7",
        }
    }
}

/// A rule that changed a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// N3: a `center` element was removed and its content kept in its
    /// place; counted once per `center`.
    CenterUnwrapped,
    /// N3: a paragraph, heading or table cell inside a `center` (directly,
    /// or in a list, quote, table or other container there), or the one
    /// whose only visible content is the `center`, got `text-align: center`.
    CenterBlockAligned,
    /// N3: text and inline elements inside a `center` (directly, or in a
    /// list item, quote, cell or other container there) were wrapped in a
    /// new `<p style="text-align: center">`; counted once per run of inline
    /// content.
    CenterRunWrapped,
    /// N3: content of a `center` whose centring content-v1 cannot keep, such
    /// as inline content in a heading that also holds other text, or a
    /// `pre`. The content is kept but no longer centred, so the report lists
    /// the body for review.
    CenterAlignmentDropped,
    /// N3: a block inside a `center` whose `style` holds a `text-align`
    /// that content-v1 does not keep (such as `inherit` or `start`, or one
    /// the sanitiser cannot read). N3 treated it as inheriting the centring,
    /// which a reader may not have seen, so the report lists the body for
    /// review.
    CenterAlignmentUnclear,
    /// N5: a paragraph's `align` attribute, whose value a browser maps to
    /// `text-align` (`left`, `right`, `center` or `justify`), became that
    /// `text-align` declaration in front of its `style`.
    ParagraphAlignMapped,
    /// N5: a paragraph's `align` attribute was removed without changing
    /// what the paragraph shows: a browser maps its value to nothing, or
    /// the paragraph's `style` sets a `text-align` that content-v1 keeps,
    /// which wins over the attribute.
    ParagraphAlignDropped,
    /// N5: a paragraph whose `align` became `text-align` also has a
    /// `text-align` in its `style` that content-v1 does not keep (such as
    /// `inherit` or `start`, or one the sanitiser cannot read). The
    /// attribute's alignment is kept, but a reader may have seen the other
    /// one, so the report lists the body for review.
    ParagraphAlignUnclear,
    /// N5: a table's `align` attribute was removed; both public sites
    /// centre every content-v1 table.
    TableAlignDropped,
    /// N5: an `h5` or `h6` became an `h4`, the smallest heading content-v1
    /// has.
    HeadingToH4,
    /// N5: an `h2` whose `style` makes it italic lost that `font-style`,
    /// and its content was wrapped in `em`.
    ItalicHeadingToEm,
    /// N5: a paragraph's `lang` attribute was removed.
    ParagraphLangStripped,
    /// N5: a `b` was renamed to `strong`; counted once per element.
    BToStrong,
    /// N5: an `i` was renamed to `em`.
    IToEm,
    /// N5: an `ins` was renamed to `u`.
    InsToU,
    /// N5: a `strike` was renamed to `s`.
    StrikeToS,
    /// N5: a `del` was renamed to `s`.
    DelToS,
    /// N5: a `big` was removed and its content kept in its place.
    BigUnwrapped,
    /// N5: a `span` with the class `marker` lost that class and got
    /// `background-color: #ffff00` in front of its `style`.
    MarkerHighlighted,
    /// N5: a `span` with a `dir` attribute and no `style`, or a blank one
    /// (only ASCII white space and `;`), was removed and its content kept in
    /// its place (editor.md §9).
    SpanDirUnwrapped,
    /// N5: a `span` with a `dir` attribute and a `style` that is not blank
    /// was removed and its content kept in its place, as editor.md §9 says,
    /// and its `style` went with it. The style may have shown a colour, a
    /// highlight or a size that content-v1 keeps on a span, so the report
    /// lists the body for review.
    SpanDirStyleDropped,
    /// N5: a marker `span` (see [`Self::MarkerHighlighted`]) with a `dir`
    /// attribute lost the `dir` and stayed, with its new highlight.
    SpanDirRemoved,
    /// N5b: an image that stood at the top level outside a paragraph, or in
    /// a top-level paragraph beside nothing a reader sees (white space, a
    /// comment, another image) or with an alignment, now stands alone in a
    /// `<p>` of its own: a new one without attributes at the top level, and
    /// otherwise the old paragraph without its alignment, which the centred
    /// image shows anyway. The old paragraph's other attributes (a class,
    /// an `id`, another declaration) stay on it for N7, which removes them
    /// as removals that no step planned, so the row goes to review
    /// (data-migration.md §7.4, reading (f), the maintainer's answer of
    /// 2026-11-15; research: DECISIONS R39 (D168)).
    ImageWrapped,
    /// N5b: an image in the running text of a top-level paragraph split the
    /// paragraph: the text before it and the text after it each keep the
    /// paragraph's attributes, and the image paragraph stands between them
    /// (editor.md §5.2 step 11a).
    ImageParagraphSplit,
    /// N5b: an image inside a heading, a link, inline formatting, a cell, a
    /// list item, a quote or any other element moved into its own paragraph
    /// in front of the top-level block that held it, where content-v1 keeps
    /// every image (editor.md §3.7, §9).
    ImageMovedBeforeBlock,
    /// N5b: a link held an image that moved out of it, so the image no
    /// longer links anywhere. The link keeps its other content, and goes
    /// when it held nothing else. Counted for review: a reader could follow
    /// the image before.
    ImageLinkDropped,
    /// N5b: an image lost its float (a `float` declaration, the class `left`
    /// or `right`, or an `align` of `left` or `right`) and is centred.
    ImageFloatRemoved,
    /// N5b: an image's `height` declaration became `height: auto`; the
    /// ratio it showed is kept in the `width` and `height` attributes.
    ImageHeightAuto,
    /// N5b: an image's `width`, `height` or `style` attribute was rewritten
    /// into the canonical image shape (editor.md §3.7).
    ImageShaped,
    /// N5b: an image's legacy markup fixed a height that cannot become a
    /// ratio, a height without a width or one whose ratio gives a `height`
    /// attribute outside the attributes' range, and it went with the rest
    /// of the legacy height, since content-v1 locks `height: auto`
    /// (data-migration.md §7.4, reading (g), which the maintainer confirmed
    /// as built on 2026-11-15: research: DECISIONS R39 (D169)). The count
    /// lists the body in the dry run; N7 never meets the height, so it does
    /// not stop `apply`. Everything else in an image's legacy sizing that
    /// the canonical shape does not keep (a border, a padding, a `display:
    /// none`, a last `width` that is no length N5b keeps, `width: auto`
    /// included, an unreadable declaration or `width` attribute) N5b leaves
    /// in place for N7's review instead (reading (f), D168).
    ImageSizingDropped,
    /// N6: a `src` or `href` on the dead host's `/storage/` was rewritten to
    /// the root-relative `/storage/` path.
    DeadHostLinkRewritten,
    /// N7: a named colour (`black`, `whitesmoke`, or `windowtext`, which
    /// Office writes for black) that the canonical style writes, as hex
    /// (`#000000`): one in the declaration in force for its property or
    /// border side, on a side that draws a border; counted per colour.
    NamedColourToHex,
    /// N7: a cell border side that the input gave a width or a colour but
    /// no visible style (`medium`, `1px #000000`, `medium none
    /// currentcolor`) was written as `none`, which draws the same: no
    /// border; counted per side.
    BorderSideToNone,
    /// N7: a length in `pt`, `cm` or another absolute unit (`pc`, `in`,
    /// `mm`, `q`) that the canonical style writes, in px: one in the
    /// declaration in force for its property or border side, on a side
    /// that draws a border; counted per length.
    LengthToPx,
    /// N7: a `style` was written in its canonical spelling and order
    /// (editor.md §3.1): its case, order, `!important` markers, overridden
    /// declarations or left-out defaults changed, or the locked
    /// declarations that a size or the element needs were added, and every
    /// declaration a browser applied is kept; counted per attribute in
    /// which none of the three re-spellings that §7.1 names (above) is
    /// counted, so that every re-spelled `style` counts under one of them.
    StyleRespelled,
    /// N7: an attribute other than `style` and a link's `target` was
    /// written in its canonical form: an enumerated value's case, an own
    /// host's address made root-relative, a default left out
    /// (`ol[start="1"]`), or a forced value that replaced another (a link's
    /// `rel`, an embed's `allow`); counted per attribute.
    AttributeRespelled,
    /// N7: a link's `target` held another value than `_blank` (`_self`,
    /// `_top`) and was replaced by `_blank`, which content-v1 forces: the
    /// link now opens in a new tab; counted per link.
    LinkTargetReplaced,
    /// N7: an element got the attributes or declarations that content-v1
    /// locks on it and it lacked: a link its `target` and `rel`, a table
    /// and its `tbody` their locked `style`, an embed its `allow`,
    /// `allowfullscreen` and `referrerpolicy`, an image its locked `style`
    /// (editor.md §3.2, §3.3); counted per element.
    LockedMarkupAdded,
    /// N7: a declaration of a property that content-v1 keeps on the element
    /// was dropped because its value is not valid CSS for that property
    /// (`width: 100%px`), so a browser ignored it too; counted per
    /// declaration.
    InvalidStyleValueDropped,
    /// N7: a `text-align` that content-v1 does not keep was dropped after
    /// the alignment that N3 or N5 wrote in front of the element's `style`,
    /// as their unclear readings planned (`center_alignment_unclear`,
    /// `paragraph_align_unclear`); counted per declaration.
    UnclearAlignmentDropped,
    /// N7: a table's `border` attribute was removed; both public sites
    /// already suppress the grid it draws.
    TableBorderDropped,
    /// N7: a table's `cellspacing` attribute was removed.
    TableCellspacingDropped,
    /// N7: a table's `cellpadding` attribute was removed; cells fall back
    /// to the sites' 5 px.
    TableCellpaddingDropped,
    /// N7: an iframe's `tabindex` attribute was removed.
    IframeTabindexDropped,
    /// N7: an iframe's `scrolling` attribute was removed.
    IframeScrollingDropped,
    /// N7: an iframe's `style` lost declarations, or was removed; counted
    /// per attribute.
    IframeStyleDropped,
    /// N7: a comment was removed.
    CommentRemoved,
    /// N7: a `script` element left after N1 was removed with its content.
    ScriptRemoved,
}

impl Rule {
    /// Every rule, in the order of [`Ord`].
    pub const ALL: [Self; 48] = [
        Self::CenterUnwrapped,
        Self::CenterBlockAligned,
        Self::CenterRunWrapped,
        Self::CenterAlignmentDropped,
        Self::CenterAlignmentUnclear,
        Self::ParagraphAlignMapped,
        Self::ParagraphAlignDropped,
        Self::ParagraphAlignUnclear,
        Self::TableAlignDropped,
        Self::HeadingToH4,
        Self::ItalicHeadingToEm,
        Self::ParagraphLangStripped,
        Self::BToStrong,
        Self::IToEm,
        Self::InsToU,
        Self::StrikeToS,
        Self::DelToS,
        Self::BigUnwrapped,
        Self::MarkerHighlighted,
        Self::SpanDirUnwrapped,
        Self::SpanDirStyleDropped,
        Self::SpanDirRemoved,
        Self::ImageWrapped,
        Self::ImageParagraphSplit,
        Self::ImageMovedBeforeBlock,
        Self::ImageLinkDropped,
        Self::ImageFloatRemoved,
        Self::ImageHeightAuto,
        Self::ImageShaped,
        Self::ImageSizingDropped,
        Self::DeadHostLinkRewritten,
        Self::NamedColourToHex,
        Self::BorderSideToNone,
        Self::LengthToPx,
        Self::StyleRespelled,
        Self::AttributeRespelled,
        Self::LinkTargetReplaced,
        Self::LockedMarkupAdded,
        Self::InvalidStyleValueDropped,
        Self::UnclearAlignmentDropped,
        Self::TableBorderDropped,
        Self::TableCellspacingDropped,
        Self::TableCellpaddingDropped,
        Self::IframeTabindexDropped,
        Self::IframeScrollingDropped,
        Self::IframeStyleDropped,
        Self::CommentRemoved,
        Self::ScriptRemoved,
    ];

    /// The step of data-migration.md §7.1 that the rule belongs to.
    #[must_use]
    pub const fn step(self) -> Step {
        match self {
            Self::CenterUnwrapped
            | Self::CenterBlockAligned
            | Self::CenterRunWrapped
            | Self::CenterAlignmentDropped
            | Self::CenterAlignmentUnclear => Step::N3,
            Self::ParagraphAlignMapped
            | Self::ParagraphAlignDropped
            | Self::ParagraphAlignUnclear
            | Self::TableAlignDropped
            | Self::HeadingToH4
            | Self::ItalicHeadingToEm
            | Self::ParagraphLangStripped
            | Self::BToStrong
            | Self::IToEm
            | Self::InsToU
            | Self::StrikeToS
            | Self::DelToS
            | Self::BigUnwrapped
            | Self::MarkerHighlighted
            | Self::SpanDirUnwrapped
            | Self::SpanDirStyleDropped
            | Self::SpanDirRemoved => Step::N5,
            Self::ImageWrapped
            | Self::ImageParagraphSplit
            | Self::ImageMovedBeforeBlock
            | Self::ImageLinkDropped
            | Self::ImageFloatRemoved
            | Self::ImageHeightAuto
            | Self::ImageShaped
            | Self::ImageSizingDropped => Step::N5b,
            Self::DeadHostLinkRewritten => Step::N6,
            Self::NamedColourToHex
            | Self::BorderSideToNone
            | Self::LengthToPx
            | Self::StyleRespelled
            | Self::AttributeRespelled
            | Self::LinkTargetReplaced
            | Self::LockedMarkupAdded
            | Self::InvalidStyleValueDropped
            | Self::UnclearAlignmentDropped
            | Self::TableBorderDropped
            | Self::TableCellspacingDropped
            | Self::TableCellpaddingDropped
            | Self::IframeTabindexDropped
            | Self::IframeScrollingDropped
            | Self::IframeStyleDropped
            | Self::CommentRemoved
            | Self::ScriptRemoved => Step::N7,
        }
    }

    /// The `snake_case` name of the rule.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CenterUnwrapped => "center_unwrapped",
            Self::CenterBlockAligned => "center_block_aligned",
            Self::CenterRunWrapped => "center_run_wrapped",
            Self::CenterAlignmentDropped => "center_alignment_dropped",
            Self::CenterAlignmentUnclear => "center_alignment_unclear",
            Self::ParagraphAlignMapped => "paragraph_align_mapped",
            Self::ParagraphAlignDropped => "paragraph_align_dropped",
            Self::ParagraphAlignUnclear => "paragraph_align_unclear",
            Self::TableAlignDropped => "table_align_dropped",
            Self::HeadingToH4 => "heading_to_h4",
            Self::ItalicHeadingToEm => "italic_heading_to_em",
            Self::ParagraphLangStripped => "paragraph_lang_stripped",
            Self::BToStrong => "b_to_strong",
            Self::IToEm => "i_to_em",
            Self::InsToU => "ins_to_u",
            Self::StrikeToS => "strike_to_s",
            Self::DelToS => "del_to_s",
            Self::BigUnwrapped => "big_unwrapped",
            Self::MarkerHighlighted => "marker_highlighted",
            Self::SpanDirUnwrapped => "span_dir_unwrapped",
            Self::SpanDirStyleDropped => "span_dir_style_dropped",
            Self::SpanDirRemoved => "span_dir_removed",
            Self::ImageWrapped => "image_wrapped",
            Self::ImageParagraphSplit => "image_paragraph_split",
            Self::ImageMovedBeforeBlock => "image_moved_before_block",
            Self::ImageLinkDropped => "image_link_dropped",
            Self::ImageFloatRemoved => "image_float_removed",
            Self::ImageHeightAuto => "image_height_auto",
            Self::ImageShaped => "image_shaped",
            Self::ImageSizingDropped => "image_sizing_dropped",
            Self::DeadHostLinkRewritten => "dead_host_link_rewritten",
            Self::NamedColourToHex => "named_colour_to_hex",
            Self::BorderSideToNone => "border_side_to_none",
            Self::LengthToPx => "length_to_px",
            Self::StyleRespelled => "style_respelled",
            Self::AttributeRespelled => "attribute_respelled",
            Self::LinkTargetReplaced => "link_target_replaced",
            Self::LockedMarkupAdded => "locked_markup_added",
            Self::InvalidStyleValueDropped => "invalid_style_value_dropped",
            Self::UnclearAlignmentDropped => "unclear_alignment_dropped",
            Self::TableBorderDropped => "table_border_dropped",
            Self::TableCellspacingDropped => "table_cellspacing_dropped",
            Self::TableCellpaddingDropped => "table_cellpadding_dropped",
            Self::IframeTabindexDropped => "iframe_tabindex_dropped",
            Self::IframeScrollingDropped => "iframe_scrolling_dropped",
            Self::IframeStyleDropped => "iframe_style_dropped",
            Self::CommentRemoved => "comment_removed",
            Self::ScriptRemoved => "script_removed",
        }
    }
}

/// A removal that N7's sanitiser made and no step planned
/// (data-migration.md §7.1 N7): the sanitiser's rule, and what it hit. The
/// import stops on a body with one, unless an `N7-accept` entry of the
/// resolution file accepts the body (§8.2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnplannedRemoval {
    /// The sanitiser's rule, as its removal report names it
    /// ([`RemovalRule::as_str`]).
    pub rule: RemovalRule,
    /// The tag name, or `#comment` for a comment.
    pub element: String,
    /// The attribute, for the attribute rules.
    pub attribute: Option<String>,
    /// For a `style` declaration: its property, lower-case. A declaration
    /// that a `style` lost (the rules `style_rejected` and
    /// `style_declaration_rejected`) names it, or has `None` when the
    /// sanitiser's CSS reader could not read the declaration, or, in the
    /// rare case where N7 could not tell the attribute's declarations
    /// apart, for the whole attribute. A `style` that the sanitiser only
    /// re-spelled (`value_canonicalised`) names the property whose
    /// declaration a browser applied and the canonical spelling does not
    /// keep: an `!important` one that a later declaration overrides there.
    /// `None` for every other removal.
    pub property: Option<String>,
}

/// `attribute_not_allowed p[class]`, `style_declaration_rejected
/// span[style] font-family`, `value_canonicalised span[style] color`,
/// `element_unwrapped div`: the rule, the element with the attribute in
/// brackets, and the property when there is one; a lost `style`
/// declaration without one shows `(unreadable)`.
impl fmt::Display for UnplannedRemoval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.rule.as_str(), self.element)?;
        if let Some(attribute) = &self.attribute {
            write!(f, "[{attribute}]")?;
        }
        match &self.property {
            Some(property) => write!(f, " {property}")?,
            None if matches!(
                self.rule,
                RemovalRule::StyleRejected | RemovalRule::StyleDeclarationRejected
            ) =>
            {
                f.write_str(" (unreadable)")?;
            }
            None => {}
        }
        Ok(())
    }
}

/// Where the dead host appears in a normalised body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// In the value of an attribute.
    Attribute {
        /// The element's tag name.
        element: String,
        /// The attribute's name, with its namespace prefix if it has one
        /// (`xlink:href`).
        attribute: String,
    },
    /// In a text node.
    Text {
        /// The tag name of the element that holds the text, or `None` for
        /// text at the top level of the body.
        element: Option<String>,
    },
}

/// An attribute value or a text node that names the dead host
/// `backend.verseghy-gimnazium.net` after rule N6 ran: anything other than
/// a `src` or `href` that starts with one of its two `/storage/` prefixes
/// (data-migration.md §7.1 N6: "Other occurrences of the host are reported,
/// not changed").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadHostOccurrence {
    /// Where it is.
    pub place: Place,
    /// How many times the host name appears in that value or text, at least
    /// one.
    pub count: usize,
    /// The text around its first appearance, at most 40 characters on each
    /// side, with `…` where it was cut, so that an editor can find it.
    pub excerpt: String,
}

/// Why an image counts as having no alt text (editor.md §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AltState {
    /// The `img` has no `alt` attribute.
    Missing,
    /// The `alt` is empty or holds only white space, which ADR 0013 §2
    /// treats as missing: the editor asks for alt text before the next save.
    Blank,
}

/// An image of a normalised body that needs alt text before its next save.
/// The importer never invents alt text (data-migration.md §7.1, "Missing
/// alt (no change)").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageWithoutAlt {
    /// The image's place among all `img` elements of the body, from 1, in
    /// document order.
    pub position: usize,
    /// Its `src` as an editor needs it to find the picture: a `data:` URL is
    /// cut after its header (`data:image/jpeg;base64,…`), because the
    /// encoded picture can be hundreds of kilobytes and N4 replaces it with
    /// a file path. `None` when the image has no `src`.
    pub src: Option<String>,
    /// Whether the `alt` is missing or blank.
    pub alt: AltState,
}

/// An image of a legacy body that was wider than content-v1's bound on an
/// image's `width` declaration, 2000 px (editor.md §3.3). N5b keeps the
/// width in the `style`, as every legacy value over a limit is reported and
/// never truncated (data-migration.md §5), and N7's sanitiser drops it, a
/// removal that no step planned, so the import stops on the body until a
/// policy change raises the bound before the import (editor.md §16 item 5)
/// or an `N7-accept` entry lets the width go (data-migration.md §7.1, N7;
/// §7.4, reading (e)). Only the images that N5b places, the ones N7 keeps,
/// are listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WideImage {
    /// The image's place among the `img` elements of the normalised body,
    /// from 1, in document order, counted as for
    /// [`ImageWithoutAlt::position`]: among the images that N7 keeps.
    pub position: usize,
    /// Its `src`, shown as for [`ImageWithoutAlt::src`]. A listed image
    /// always has one: N5b places only images whose `src` the sanitiser
    /// keeps.
    pub src: String,
    /// Its width in px, as N5b read it and wrote it into the `style` for N7.
    pub width: Hundredths,
}

/// What one normaliser run recorded about one body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormaliseReport {
    hits: BTreeMap<Rule, usize>,
    unplanned: BTreeMap<UnplannedRemoval, usize>,
    dead_host_kept: Vec<DeadHostOccurrence>,
    images_without_alt: Vec<ImageWithoutAlt>,
    wide_images: Vec<WideImage>,
}

/// The observations of one pass, which replace those of the pass before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Observations {
    /// The dead-host occurrences that N6 left alone, in document order.
    pub(super) dead_host_kept: Vec<DeadHostOccurrence>,
    /// The images without alt text, in document order.
    pub(super) images_without_alt: Vec<ImageWithoutAlt>,
}

impl NormaliseReport {
    /// How many times `rule` changed the body.
    ///
    /// - `rule`: the rule to count.
    #[must_use]
    pub fn count(&self, rule: Rule) -> usize {
        self.hits.get(&rule).copied().unwrap_or(0)
    }

    /// Every rule that changed the body, with its count, in the order of
    /// [`Rule`].
    pub fn hits(&self) -> impl Iterator<Item = (Rule, usize)> + '_ {
        self.hits.iter().map(|(&rule, &count)| (rule, count))
    }

    /// Whether any rule changed the body. Character references, tag case,
    /// line ends and the serialisation (N0 and N8) are not rule hits
    /// (data-migration.md §7.2 records N1-N7), so a body that only those
    /// changed has none.
    #[must_use]
    pub fn has_hits(&self) -> bool {
        !self.hits.is_empty()
    }

    /// The occurrences of the dead host that N6 left unchanged, in document
    /// order.
    #[must_use]
    pub fn dead_host_kept(&self) -> &[DeadHostOccurrence] {
        &self.dead_host_kept
    }

    /// The images without alt text, or with a blank one, in document order.
    #[must_use]
    pub fn images_without_alt(&self) -> &[ImageWithoutAlt] {
        &self.images_without_alt
    }

    /// The removals of N7 that no step planned, each with how many times
    /// the sanitiser made it, in the order of [`UnplannedRemoval`]. The
    /// import stops on a body with any, unless an `N7-accept` entry accepts
    /// it (data-migration.md §7.1 N7).
    pub fn unplanned_removals(&self) -> impl Iterator<Item = (&UnplannedRemoval, usize)> + '_ {
        self.unplanned
            .iter()
            .map(|(removal, &count)| (removal, count))
    }

    /// Whether N7 made any removal that no step planned.
    #[must_use]
    pub fn has_unplanned_removals(&self) -> bool {
        !self.unplanned.is_empty()
    }

    /// The images whose width over content-v1's 2000 px bound N7 dropped,
    /// in document order (editor.md §16 item 5).
    #[must_use]
    pub fn wide_images(&self) -> &[WideImage] {
        &self.wide_images
    }

    /// Counts one hit of `rule`.
    pub(super) fn record(&mut self, rule: Rule) {
        self.record_many(rule, 1);
    }

    /// Counts `count` hits of `rule`; zero records nothing, so a rule with
    /// no hit never appears in [`Self::hits`].
    ///
    /// - `rule`: the rule that changed the body.
    /// - `count`: how many changes it made.
    pub(super) fn record_many(&mut self, rule: Rule, count: usize) {
        if count > 0 {
            *self.hits.entry(rule).or_insert(0) += count;
        }
    }

    /// Counts `count` occurrences of an unplanned removal; zero records
    /// nothing.
    ///
    /// - `removal`: what N7 removed that no step planned.
    /// - `count`: how many times.
    pub(super) fn record_unplanned(&mut self, removal: UnplannedRemoval, count: usize) {
        if count > 0 {
            *self.unplanned.entry(removal).or_insert(0) += count;
        }
    }

    /// Adds the wide images that one pass found before N7 dropped their
    /// width, after those of the earlier passes.
    ///
    /// - `images`: the pass's wide images, in document order.
    pub(super) fn record_wide_images(&mut self, images: Vec<WideImage>) {
        self.wide_images.extend(images);
    }

    /// Replaces the observations with those of a later pass.
    pub(super) fn observe(&mut self, observations: Observations) {
        self.dead_host_kept = observations.dead_host_kept;
        self.images_without_alt = observations.images_without_alt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_rule_is_listed_once_in_order_with_a_unique_name() {
        let mut sorted = Rule::ALL;
        sorted.sort();
        assert_eq!(sorted, Rule::ALL, "ALL is in the order of Ord");
        let names: BTreeSet<_> = Rule::ALL.iter().map(|rule| rule.as_str()).collect();
        assert_eq!(names.len(), Rule::ALL.len(), "names are unique");
        for rule in Rule::ALL {
            let name = rule.as_str();
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{name} is snake_case"
            );
        }
    }

    #[test]
    fn each_rule_belongs_to_its_step() {
        let steps: Vec<_> = Rule::ALL
            .iter()
            .map(|rule| (rule.as_str(), rule.step().as_str()))
            .collect();
        assert_eq!(
            steps,
            [
                ("center_unwrapped", "N3"),
                ("center_block_aligned", "N3"),
                ("center_run_wrapped", "N3"),
                ("center_alignment_dropped", "N3"),
                ("center_alignment_unclear", "N3"),
                ("paragraph_align_mapped", "N5"),
                ("paragraph_align_dropped", "N5"),
                ("paragraph_align_unclear", "N5"),
                ("table_align_dropped", "N5"),
                ("heading_to_h4", "N5"),
                ("italic_heading_to_em", "N5"),
                ("paragraph_lang_stripped", "N5"),
                ("b_to_strong", "N5"),
                ("i_to_em", "N5"),
                ("ins_to_u", "N5"),
                ("strike_to_s", "N5"),
                ("del_to_s", "N5"),
                ("big_unwrapped", "N5"),
                ("marker_highlighted", "N5"),
                ("span_dir_unwrapped", "N5"),
                ("span_dir_style_dropped", "N5"),
                ("span_dir_removed", "N5"),
                ("image_wrapped", "N5b"),
                ("image_paragraph_split", "N5b"),
                ("image_moved_before_block", "N5b"),
                ("image_link_dropped", "N5b"),
                ("image_float_removed", "N5b"),
                ("image_height_auto", "N5b"),
                ("image_shaped", "N5b"),
                ("image_sizing_dropped", "N5b"),
                ("dead_host_link_rewritten", "N6"),
                ("named_colour_to_hex", "N7"),
                ("border_side_to_none", "N7"),
                ("length_to_px", "N7"),
                ("style_respelled", "N7"),
                ("attribute_respelled", "N7"),
                ("link_target_replaced", "N7"),
                ("locked_markup_added", "N7"),
                ("invalid_style_value_dropped", "N7"),
                ("unclear_alignment_dropped", "N7"),
                ("table_border_dropped", "N7"),
                ("table_cellspacing_dropped", "N7"),
                ("table_cellpadding_dropped", "N7"),
                ("iframe_tabindex_dropped", "N7"),
                ("iframe_scrolling_dropped", "N7"),
                ("iframe_style_dropped", "N7"),
                ("comment_removed", "N7"),
                ("script_removed", "N7"),
            ]
        );
        assert_eq!(Step::N3.as_str(), "N3");
        assert_eq!(Step::N5.as_str(), "N5");
        assert_eq!(Step::N5b.as_str(), "N5b");
        assert_eq!(Step::N6.as_str(), "N6");
        assert_eq!(Step::N7.as_str(), "N7");
        // The steps are ordered as data-migration.md §7.1 runs them.
        assert!(
            Step::N3 < Step::N5
                && Step::N5 < Step::N5b
                && Step::N5b < Step::N6
                && Step::N6 < Step::N7
        );
    }

    fn removal(
        rule: RemovalRule,
        element: &str,
        attribute: Option<&str>,
        property: Option<&str>,
    ) -> UnplannedRemoval {
        UnplannedRemoval {
            rule,
            element: element.to_owned(),
            attribute: attribute.map(str::to_owned),
            property: property.map(str::to_owned),
        }
    }

    #[test]
    fn an_unplanned_removal_names_its_rule_element_attribute_and_property() {
        let shown = [
            removal(RemovalRule::ElementUnwrapped, "div", None, None),
            removal(RemovalRule::AttributeNotAllowed, "p", Some("class"), None),
            removal(
                RemovalRule::StyleDeclarationRejected,
                "span",
                Some("style"),
                Some("font-family"),
            ),
            removal(RemovalRule::StyleRejected, "td", Some("style"), None),
            removal(
                RemovalRule::ValueCanonicalised,
                "span",
                Some("style"),
                Some("color"),
            ),
            removal(RemovalRule::CommentRemoved, "#comment", None, None),
        ]
        .map(|removal| removal.to_string());
        assert_eq!(
            shown,
            [
                "element_unwrapped div",
                "attribute_not_allowed p[class]",
                "style_declaration_rejected span[style] font-family",
                "style_rejected td[style] (unreadable)",
                "value_canonicalised span[style] color",
                "comment_removed #comment",
            ]
        );
    }

    #[test]
    fn unplanned_removals_are_counted_apart_from_the_hits() {
        let div = removal(RemovalRule::ElementUnwrapped, "div", None, None);
        let class = removal(RemovalRule::AttributeNotAllowed, "p", Some("class"), None);
        let mut report = NormaliseReport::default();
        assert!(!report.has_unplanned_removals());
        report.record_unplanned(class.clone(), 0);
        assert!(!report.has_unplanned_removals(), "zero records nothing");
        report.record_unplanned(div.clone(), 2);
        report.record_unplanned(class.clone(), 1);
        report.record_unplanned(div.clone(), 1);
        assert!(report.has_unplanned_removals());
        assert!(!report.has_hits(), "an unplanned removal is no rule hit");
        assert_eq!(
            report.unplanned_removals().collect::<Vec<_>>(),
            [(&div, 3), (&class, 1)],
            "in the order of UnplannedRemoval: the rule first"
        );
        report.record_many(Rule::TableBorderDropped, 0);
        assert!(!report.has_hits(), "zero records nothing");
        report.record_many(Rule::TableBorderDropped, 2);
        assert_eq!(report.count(Rule::TableBorderDropped), 2);
    }

    proptest::proptest! {
        // An unplanned removal is shown as its rule and element first, the
        // attribute in brackets, and the property when there is one; a lost
        // `style` declaration without one shows `(unreadable)`.
        #[test]
        fn an_unplanned_removal_is_shown_whole(
            rule in proptest::sample::select(&[
                RemovalRule::ElementUnwrapped,
                RemovalRule::AttributeNotAllowed,
                RemovalRule::StyleRejected,
                RemovalRule::StyleDeclarationRejected,
                RemovalRule::ValueCanonicalised,
            ][..]),
            element in "[a-z][a-z0-9]{0,8}",
            attribute in proptest::option::of("[a-z][a-z-]{0,10}"),
            property in proptest::option::of("[a-z][a-z-]{0,12}"),
        ) {
            let shown = removal(rule, &element, attribute.as_deref(), property.as_deref()).to_string();
            let mut expected = format!("{} {element}", rule.as_str());
            if let Some(attribute) = &attribute {
                expected = format!("{expected}[{attribute}]");
            }
            let lost = matches!(rule, RemovalRule::StyleRejected | RemovalRule::StyleDeclarationRejected);
            match property.as_deref() {
                Some(property) => expected = format!("{expected} {property}"),
                None if lost => expected.push_str(" (unreadable)"),
                None => {}
            }
            proptest::prop_assert_eq!(shown, expected);
        }

        // Recording counts in any split adds them up, and a zero never
        // shows: the report of a run is the sum of its passes.
        #[test]
        fn counts_add_up_over_any_split(counts in proptest::collection::vec(0_usize..5, 0..8)) {
            let mut report = NormaliseReport::default();
            let removal = removal(RemovalRule::ElementUnwrapped, "font", None, None);
            for &count in &counts {
                report.record_many(Rule::LengthToPx, count);
                report.record_unplanned(removal.clone(), count);
            }
            let total: usize = counts.iter().sum();
            proptest::prop_assert_eq!(report.count(Rule::LengthToPx), total);
            proptest::prop_assert_eq!(report.has_hits(), total > 0);
            proptest::prop_assert_eq!(
                report.unplanned_removals().map(|(_, count)| count).sum::<usize>(),
                total
            );
        }
    }

    #[test]
    fn hits_are_counted_per_rule() {
        let mut report = NormaliseReport::default();
        assert!(!report.has_hits());
        assert_eq!(report.count(Rule::CenterUnwrapped), 0);
        report.record(Rule::DeadHostLinkRewritten);
        report.record(Rule::CenterUnwrapped);
        report.record(Rule::CenterUnwrapped);
        assert!(report.has_hits());
        assert_eq!(report.count(Rule::CenterUnwrapped), 2);
        assert_eq!(report.count(Rule::DeadHostLinkRewritten), 1);
        assert_eq!(report.count(Rule::CenterRunWrapped), 0);
        assert_eq!(
            report.hits().collect::<Vec<_>>(),
            [(Rule::CenterUnwrapped, 2), (Rule::DeadHostLinkRewritten, 1)],
            "in the order of Rule"
        );
    }

    #[test]
    fn a_later_pass_replaces_the_observations_and_keeps_the_hits_and_wide_images() {
        let image = |position| ImageWithoutAlt {
            position,
            src: None,
            alt: AltState::Missing,
        };
        let occurrence = DeadHostOccurrence {
            place: Place::Text { element: None },
            count: 1,
            excerpt: "backend.verseghy-gimnazium.net".to_owned(),
        };
        let wide = |position| WideImage {
            position,
            src: "/storage/uploads/a.jpg".to_owned(),
            width: Hundredths::whole(2400),
        };
        let mut report = NormaliseReport::default();
        report.record(Rule::CenterUnwrapped);
        report.record_wide_images(vec![wide(1), wide(3)]);
        report.observe(Observations {
            dead_host_kept: vec![occurrence.clone()],
            images_without_alt: vec![image(1), image(2)],
        });
        report.record_wide_images(Vec::new());
        report.observe(Observations {
            dead_host_kept: vec![occurrence.clone()],
            images_without_alt: vec![image(1)],
        });
        assert_eq!(report.images_without_alt(), [image(1)]);
        assert_eq!(report.dead_host_kept(), [occurrence]);
        // N7 drops a wide width in the pass that lists it, so the passes
        // after it list nothing: the wide images are kept, like the hits.
        assert_eq!(report.wide_images(), [wide(1), wide(3)]);
        assert_eq!(report.count(Rule::CenterUnwrapped), 1);
    }
}
