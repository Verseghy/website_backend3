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
//! The `snake_case` names of [`Rule::as_str`] are provisional in the same
//! way as the sanitiser's: the run report that the importer slices write
//! fixes them.

use std::collections::BTreeMap;

/// A step of data-migration.md §7.1 that records rule hits. The steps that
/// later slices add (N1, N2, N4, N5b and N7, and N5's inline rules) join
/// this list with their rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    /// N3: `<center>` unwrapped into `text-align: center`.
    N3,
    /// N5: legacy presentational markup mapped onto content-v1; so far its
    /// block rules.
    N5,
    /// N6: links to the dead host rewritten to root-relative.
    N6,
}

impl Step {
    /// The step's name in data-migration.md §7.1, such as `N3`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::N3 => "N3",
            Self::N5 => "N5",
            Self::N6 => "N6",
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
    /// N6: a `src` or `href` on the dead host's `/storage/` was rewritten to
    /// the root-relative `/storage/` path.
    DeadHostLinkRewritten,
}

impl Rule {
    /// Every rule, in the order of [`Ord`].
    pub const ALL: [Self; 13] = [
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
        Self::DeadHostLinkRewritten,
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
            | Self::ParagraphLangStripped => Step::N5,
            Self::DeadHostLinkRewritten => Step::N6,
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
            Self::DeadHostLinkRewritten => "dead_host_link_rewritten",
        }
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

/// What one normaliser run recorded about one body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormaliseReport {
    hits: BTreeMap<Rule, usize>,
    dead_host_kept: Vec<DeadHostOccurrence>,
    images_without_alt: Vec<ImageWithoutAlt>,
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

    /// Counts one hit of `rule`.
    pub(super) fn record(&mut self, rule: Rule) {
        *self.hits.entry(rule).or_insert(0) += 1;
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
                ("dead_host_link_rewritten", "N6"),
            ]
        );
        assert_eq!(Step::N3.as_str(), "N3");
        assert_eq!(Step::N5.as_str(), "N5");
        assert_eq!(Step::N6.as_str(), "N6");
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
    fn a_later_pass_replaces_the_observations_and_keeps_the_hits() {
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
        let mut report = NormaliseReport::default();
        report.record(Rule::CenterUnwrapped);
        report.observe(Observations {
            dead_host_kept: vec![occurrence.clone()],
            images_without_alt: vec![image(1), image(2)],
        });
        report.observe(Observations {
            dead_host_kept: vec![occurrence.clone()],
            images_without_alt: vec![image(1)],
        });
        assert_eq!(report.images_without_alt(), [image(1)]);
        assert_eq!(report.dead_host_kept(), [occurrence]);
        assert_eq!(report.count(Rule::CenterUnwrapped), 1);
    }
}
