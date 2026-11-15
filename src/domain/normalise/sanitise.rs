//! N7: the body goes through the sanitiser that every write uses, and what
//! the sanitiser removed is compared with the planned N7 rules
//! (data-migration.md §7.1 N7 and §7.4, editor.md §9, ADR 0013 §5, ADR 0017
//! §2-§3).
//!
//! The sanitiser keeps content-v1 and removes everything else. Some of what
//! it removes or rewrites is planned: §7.1 names it, or content-v1 writes it
//! that way for every body, and N7 counts it per [`Rule`]. Every other
//! removal is **unplanned**: it can change what a page shows, and no earlier
//! step decided it, so the body is reported with each [`UnplannedRemoval`]
//! and the import stops on it unless an `N7-accept` entry of the resolution
//! file accepts it (§8.2; `domain::import_mysql::n7_review`). The earlier
//! steps plan most removals by rewriting the legacy markup into content-v1
//! before N7 sees it (a bare `YouTube` iframe becomes the canonical embed in
//! N2, a table loses its width in N5b), so a removal that is unplanned today
//! may become planned when the step for it lands.
//!
//! **Every change is recorded.** data-migration.md §10 V5(b) checks that a
//! row without a rule hit kept its DOM, so whatever N7 changes in a body
//! leaves a rule hit or an unplanned removal: the planned removals and
//! re-spellings below, and the locked markup that the sanitiser adds without
//! reporting it (a link's `target` and `rel`, a table's locked `style`),
//! which N7 finds itself ([`Rule::LockedMarkupAdded`]).
//!
//! # What N7 plans
//!
//! The planned removals outside `style` are data ([`PLANNED_REMOVALS`]): the
//! table attributes content-v1 drops (`border`, `cellspacing`,
//! `cellpadding`), an iframe's `tabindex`, `scrolling` and `style`,
//! comments, and a `script` left after N1. The sanitiser's re-spellings of
//! other attributes are counted too: a link's `target` replaced by `_blank`
//! ([`Rule::LinkTargetReplaced`]), and any other value it re-spells or
//! default it leaves out ([`Rule::AttributeRespelled`]).
//!
//! The rest of N7's plan is about `style` values, which the sanitiser
//! reports per attribute, not per declaration, so this module reads every
//! `style` again with the sanitiser's own CSS reader, grammar and canonical
//! writer ([`css`]), one declaration as written at a time (the pieces that
//! N5's italic-heading rule cuts, `blocks::declaration_texts`), and judges
//! each declaration:
//!
//! - **Re-spellings** are not removals, as long as every declaration that a
//!   browser applied is kept: editor.md §3.1 writes one spelling per visual
//!   result. N7 counts the three re-spellings that §7.1 names, in the
//!   declarations in force, where the canonical style writes them: a named colour written as hex ([`Rule::NamedColourToHex`]),
//!   a cell border side with a width or a colour but no visible style
//!   written as `none` ([`Rule::BorderSideToNone`]), and a length in `pt`,
//!   `cm` or another absolute unit written in px ([`Rule::LengthToPx`]). An
//!   overridden declaration, and the colour or width of a side that draws no
//!   border, are not written, so they are not counted. An attribute that is
//!   only re-spelled in other ways (case, order, a left-out default, a
//!   locked declaration added) counts once as [`Rule::StyleRespelled`].
//! - **`!important`.** The sanitiser's reader drops the marker, and its
//!   canonical cascade lets the last accepted declaration win, where a
//!   browser let the last `!important` one win. N7 writes the style a
//!   second time with every `!important` declaration moved behind the
//!   others, which is the order in which a browser's cascade and the
//!   canonical one agree; a property whose written value differs between
//!   the two kept another value than the one a reader saw, and is
//!   unplanned (`value_canonicalised span[style] color`).
//! - **An invalid value** of a property that content-v1 keeps on the
//!   element, such as `width: 100%px`, is dropped as planned
//!   ([`Rule::InvalidStyleValueDropped`]): a browser ignored it too. Only a
//!   value that is certainly invalid CSS counts (`invalid_css.rs`); a valid
//!   value that content-v1 does not keep (`width: 50%`, `background: none`,
//!   `inherit`) is unplanned.
//! - **An unclear alignment** that N3 or N5 left behind is dropped as
//!   planned ([`Rule::UnclearAlignmentDropped`]): when they write a
//!   `text-align` in front of a `style` whose own `text-align` content-v1
//!   does not keep, they count the block as unclear and leave that
//!   declaration for N7 to drop, so that theirs is in force (D139 reading 4,
//!   D142 reading 2). N7 recognises the bytes they write in front
//!   (`text-align: center; `, as `dom::prepend_declaration` writes them),
//!   and plans the drop only within the number of blocks that N3 and N5
//!   counted as unclear in the same pass, one for each alignment written in
//!   front of the style. A hand-written `style` of the same shape beyond
//!   that number is unplanned, so the review errs towards an editor's look.
//! - **Everything else** a `style` loses is unplanned, with its property: a
//!   property that content-v1 does not allow on the element
//!   (`font-family`), a valid value outside the grammar, and a declaration
//!   the reader cannot read (`url()`, `calc()`, a comment), named by its
//!   property where the text names one.
//!
//! Everything the sanitiser reports outside a `style` and outside the
//! planned list is unplanned too: an element unwrapped or removed, an
//! attribute that is not allowed (`class`, an attribute on a mark that N5
//! renamed, D146), a value outside its rule, a URL that is refused, and the
//! structural pass's removals.
//!
//! If the reading of the `style` attributes ever disagrees with the
//! sanitiser's report about how many attributes of an element lost
//! declarations or were re-spelled, every one of them counts as unplanned
//! (with no property): the review then errs towards an editor's look, never
//! towards a silent change.

use std::collections::{BTreeMap, BTreeSet};

use cssparser::Parser;
use scraper::Node;

use super::NormaliseError;
use super::blocks::{declaration_texts, is_important};
use super::dom;
use super::invalid_css;
use super::report::{NormaliseReport, Rule, UnplannedRemoval};
use crate::domain::rich_text::attrs;
use crate::domain::rich_text::css::canonical::{SIDES, four};
use crate::domain::rich_text::css::declarations::{self, Component, Declaration, MAX_DECLARATIONS};
use crate::domain::rich_text::css::grammar::{self, ElementStyle, Grammar, Reading, Value};
use crate::domain::rich_text::css::{self, values};
use crate::domain::rich_text::policy::{
    ElementRule, REMOVED_WITH_CONTENT, ValueRule, element_rule,
};
use crate::domain::rich_text::report::{AMMONIA_FOREIGN_TAGS, url_outcome};
use crate::domain::rich_text::{Removal, RemovalReport, RemovalRule, SanitizePolicy, Sanitizer};

/// One removal that N7 plans: the sanitiser's rule, the element and the
/// attribute it hits, and the [`Rule`] N7 counts it under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlannedRemoval {
    /// The sanitiser's rule.
    pub(super) removal: RemovalRule,
    /// The tag name, or `#comment`.
    pub(super) element: &'static str,
    /// The attribute, for the attribute rules.
    pub(super) attribute: Option<&'static str>,
    /// What N7 counts it as.
    pub(super) rule: Rule,
}

const fn planned(
    removal: RemovalRule,
    element: &'static str,
    attribute: Option<&'static str>,
    rule: Rule,
) -> PlannedRemoval {
    PlannedRemoval {
        removal,
        element,
        attribute,
        rule,
    }
}

/// The removals that data-migration.md §7.1 N7 plans outside the `style`
/// declarations: "Comments and everything outside content-v1 are removed:
/// any `script` element left after N1, iframe `tabindex`, `scrolling` and
/// `style`, and the table attributes that content-v1 drops (`border`,
/// `cellspacing`, `cellpadding`)". The sanitiser reports a whole iframe
/// `style` that goes as `style_rejected` and one that loses some
/// declarations as `style_declaration_rejected`; both are the planned
/// iframe `style` removal.
pub(super) const PLANNED_REMOVALS: &[PlannedRemoval] = &[
    planned(
        RemovalRule::AttributeNotAllowed,
        "table",
        Some("border"),
        Rule::TableBorderDropped,
    ),
    planned(
        RemovalRule::AttributeNotAllowed,
        "table",
        Some("cellspacing"),
        Rule::TableCellspacingDropped,
    ),
    planned(
        RemovalRule::AttributeNotAllowed,
        "table",
        Some("cellpadding"),
        Rule::TableCellpaddingDropped,
    ),
    planned(
        RemovalRule::AttributeNotAllowed,
        "iframe",
        Some("tabindex"),
        Rule::IframeTabindexDropped,
    ),
    planned(
        RemovalRule::AttributeNotAllowed,
        "iframe",
        Some("scrolling"),
        Rule::IframeScrollingDropped,
    ),
    planned(
        RemovalRule::StyleRejected,
        "iframe",
        Some("style"),
        Rule::IframeStyleDropped,
    ),
    planned(
        RemovalRule::StyleDeclarationRejected,
        "iframe",
        Some("style"),
        Rule::IframeStyleDropped,
    ),
    planned(
        RemovalRule::CommentRemoved,
        "#comment",
        None,
        Rule::CommentRemoved,
    ),
    planned(
        RemovalRule::ElementRemoved,
        "script",
        None,
        Rule::ScriptRemoved,
    ),
];

/// The absolute length units that N7 counts when it writes them in px
/// ([`Rule::LengthToPx`]): every unit of the sanitiser's reader but `px`.
const ABSOLUTE_UNITS: &[&str] = &["pt", "pc", "in", "cm", "mm", "q"];

/// The alignments N3 and N5 write in front of a `style`: the `text-align`
/// values that content-v1 keeps.
const WRITTEN_ALIGNMENTS: &[&str] = &["left", "center", "right", "justify"];

/// N7 over one pass's body: sanitises `input`, records the planned changes
/// and the unplanned removals in `report`, and returns the sanitised body.
///
/// - `input`: the body after N6, as the pass serialised it.
/// - `sanitizer`: the deployment's sanitiser, the one every write uses.
/// - `unclear_blocks`: how many blocks N3 and N5 counted as unclear in this
///   pass (`center_alignment_unclear`, `paragraph_align_unclear`): the most
///   alignments written in front that N7 plans to keep over a dropped
///   `text-align`.
/// - `report`: the report of the current normaliser run.
///
/// # Errors
///
/// [`NormaliseError::SanitiseRefused`] when the sanitiser refuses the body:
/// above its size or depth limit, still changing after its passes, or a
/// broken invariant.
pub(super) fn sanitise(
    input: &str,
    sanitizer: &Sanitizer,
    unclear_blocks: usize,
    report: &mut NormaliseReport,
) -> Result<String, NormaliseError> {
    let sanitised = sanitizer
        .sanitize(input)
        .map_err(NormaliseError::SanitiseRefused)?;
    let output = sanitised.html.into_string();
    // A body the sanitiser left as it was, with an empty report, is
    // content-v1 already, so the second reading is skipped. The report
    // alone does not tell: the locked markup it adds is not in it.
    if output != input || !sanitised.report.is_empty() {
        review(
            input,
            sanitizer.policy(),
            unclear_blocks,
            &sanitised.report,
            report,
        );
    }
    Ok(output)
}

/// Compares the sanitiser's removal report for `input` with the planned N7
/// rules, and records the planned changes and the unplanned removals in
/// `report` (see the module documentation).
///
/// - `input`: what the sanitiser was given.
/// - `policy`: the sanitiser's policy, for the image sources it keeps.
/// - `unclear_blocks`: the unclear blocks of N3 and N5 in this pass (see
///   [`sanitise`]).
/// - `removals`: the removal report of that sanitiser run.
/// - `report`: the report of the current normaliser run.
fn review(
    input: &str,
    policy: &SanitizePolicy,
    unclear_blocks: usize,
    removals: &RemovalReport,
    report: &mut NormaliseReport,
) {
    let styles = StyleReview::of(input, policy, unclear_blocks);
    for (&rule, &count) in &styles.counted {
        report.record_many(rule, count);
    }
    for (removal, count) in removals.entries() {
        if let Some(planned) = planned_rule(removal) {
            report.record_many(planned, count);
            continue;
        }
        let style = removal.attribute.as_deref() == Some("style");
        match removal.rule {
            RemovalRule::StyleRejected
            | RemovalRule::StyleDeclarationRejected
            | RemovalRule::ValueCanonicalised
            | RemovalRule::DefaultValueOmitted
                if style =>
            {
                match styles.groups.get(&(removal.rule, removal.element.clone())) {
                    Some(group) if group.attributes == count => {
                        for (&rule, &planned) in &group.planned {
                            report.record_many(rule, planned);
                        }
                        for (unplanned, &times) in &group.unplanned {
                            report.record_unplanned(unplanned.clone(), times);
                        }
                    }
                    // The reading disagrees with the report: err towards
                    // review.
                    _ => report.record_unplanned(unplanned(removal, None), count),
                }
            }
            // content-v1 forces `_blank`, which opens the link in a new tab:
            // a change a reader notices, so it has a rule of its own.
            RemovalRule::ValueCanonicalised
                if removal.element == "a" && removal.attribute.as_deref() == Some("target") =>
            {
                report.record_many(Rule::LinkTargetReplaced, count);
            }
            RemovalRule::ValueCanonicalised | RemovalRule::DefaultValueOmitted => {
                report.record_many(Rule::AttributeRespelled, count);
            }
            _ => report.record_unplanned(unplanned(removal, None), count),
        }
    }
}

/// The planned rule of a removal outside the `style` declarations, from
/// [`PLANNED_REMOVALS`].
///
/// - `removal`: one entry of the sanitiser's removal report.
fn planned_rule(removal: &Removal) -> Option<Rule> {
    PLANNED_REMOVALS
        .iter()
        .find(|planned| {
            planned.removal == removal.rule
                && planned.element == removal.element
                && planned.attribute == removal.attribute.as_deref()
        })
        .map(|planned| planned.rule)
}

/// The unplanned removal that `removal` is, for one declaration
/// `property` of a `style` or for the whole entry.
///
/// - `removal`: one entry of the sanitiser's removal report, or one built
///   for a `style` attribute.
/// - `property`: the declaration's property, when there is one.
fn unplanned(removal: &Removal, property: Option<&str>) -> UnplannedRemoval {
    UnplannedRemoval {
        rule: removal.rule,
        element: removal.element.clone(),
        attribute: removal.attribute.clone(),
        property: property.map(str::to_owned),
    }
}

/// What the `style` attributes of one element name lost or had re-spelled,
/// among those the sanitiser reports under one rule.
#[derive(Debug, Default, PartialEq, Eq)]
struct StyleGroup {
    /// How many such attributes there are.
    attributes: usize,
    /// The planned changes, per rule.
    planned: BTreeMap<Rule, usize>,
    /// The unplanned drops, per declaration property.
    unplanned: BTreeMap<UnplannedRemoval, usize>,
}

/// The second reading of every element and `style` the sanitiser keeps.
#[derive(Debug)]
struct StyleReview<'p> {
    /// The sanitiser's policy, for the image sources it keeps.
    policy: &'p SanitizePolicy,
    /// How many more alignments written in front N7 may still plan to keep
    /// over a dropped `text-align` (see [`sanitise`]).
    unclear_budget: usize,
    /// Per sanitiser rule and element name, the attributes that lost
    /// declarations or were re-spelled.
    groups: BTreeMap<(RemovalRule, String), StyleGroup>,
    /// The planned changes that are counted whatever the report says: the
    /// re-spellings that §7.1 names and the locked markup added, per rule.
    counted: BTreeMap<Rule, usize>,
}

impl<'p> StyleReview<'p> {
    /// Reads every element and `style` of `input` that the sanitiser keeps,
    /// in the tree the sanitiser's ammonia step walks: the same parse, the
    /// same subtrees skipped (the elements removed with their content), and
    /// only the elements and attributes the policy keeps. An explicit stack,
    /// so that a deep body cannot overflow the call stack.
    ///
    /// - `input`: what the sanitiser was given.
    /// - `policy`: the sanitiser's policy.
    /// - `unclear_blocks`: the unclear blocks of N3 and N5 in this pass.
    fn of(input: &str, policy: &'p SanitizePolicy, unclear_blocks: usize) -> Self {
        let html = dom::parse(input);
        let mut review = Self {
            policy,
            unclear_budget: unclear_blocks,
            groups: BTreeMap::new(),
            counted: BTreeMap::new(),
        };
        let mut stack: Vec<_> = html.root_element().children().collect();
        while let Some(node) = stack.pop() {
            let Node::Element(element) = node.value() else {
                continue;
            };
            let name = element.name();
            if REMOVED_WITH_CONTENT.contains(&name) || AMMONIA_FOREIGN_TAGS.contains(&name) {
                continue;
            }
            if let Some(rule) = element_rule(name) {
                if review.lacks_locked_markup(element, rule) {
                    *review.counted.entry(Rule::LockedMarkupAdded).or_insert(0) += 1;
                }
                if let (Some(style_rule), Some(style)) =
                    (rule.attribute("style"), element.attr("style"))
                {
                    review.style(name, style, style_rule.rule);
                }
            }
            stack.extend(node.children());
        }
        review
    }

    /// Whether the sanitiser adds to the kept element `node` markup that
    /// content-v1 locks on it and that it lacks, without reporting it: a
    /// forced attribute that ammonia appends (a link's `target`, the locked
    /// `style` of a table and its `tbody`, an embed's `allow`,
    /// `allowfullscreen` and `referrerpolicy`), the `rel` that ammonia
    /// appends to every link, and the locked `style` that the structural
    /// pass gives an image without one. An image that the structural pass
    /// removes (its `src` is refused or missing) gets nothing, and is not
    /// counted; the removal is reported. A link is counted whether or not
    /// the structural pass then unwraps it inside another link (ADR 0037):
    /// which ones it unwraps depends on how its own parse of ammonia's
    /// output nests them, which N7 does not repeat, and every such unwrap
    /// is reported as `nested_link_unwrapped` anyway.
    ///
    /// - `element`: an element of the input that the sanitiser keeps.
    /// - `rule`: its content-v1 rule.
    fn lacks_locked_markup(&self, element: &scraper::node::Element, rule: &ElementRule) -> bool {
        let lacks = |attribute: &str| element.attr(attribute).is_none();
        match rule.name {
            "a" => lacks("target") || lacks("rel"),
            "img" => {
                lacks("style")
                    && element
                        .attr("src")
                        .is_some_and(|src| url_outcome(self.policy, "img", "src", src).is_some())
            }
            _ => rule.attributes.iter().any(|attribute| {
                matches!(attribute.rule, ValueRule::Forced(_)) && lacks(attribute.name)
            }),
        }
    }

    /// Reads one `style` attribute.
    ///
    /// - `element`: its element's tag name.
    /// - `value`: the attribute value, references decoded.
    /// - `rule`: the policy's rule for it: [`ValueRule::Style`], or
    ///   [`ValueRule::Forced`] for the wholly locked `table` and `tbody`
    ///   styles.
    fn style(&mut self, element: &str, value: &str, rule: ValueRule) {
        // The rule the sanitiser's report records for this attribute,
        // decided by the same functions.
        let outcome = match rule {
            ValueRule::Style => attrs::decide_style(element, value).1,
            ValueRule::Forced(forced) => attrs::forced_style_rule(element, value, forced),
            _ => return,
        };
        let Some(grammar) = grammar::element_style(element) else {
            return;
        };
        let pieces = pieces(value);
        let written = css::sanitize_style(element, value).css;
        let mut named_respellings = 0;
        for (rule, count) in respellings(&pieces, grammar, written.as_deref()) {
            if count > 0 {
                *self.counted.entry(rule).or_insert(0) += count;
                named_respellings += count;
            }
        }
        let Some(outcome) = outcome else {
            return;
        };
        let removal = Removal {
            rule: outcome,
            element: element.to_owned(),
            attribute: Some("style".to_owned()),
        };
        let overridden = overridden_by_importance(element, &pieces, written.as_deref());
        let group = self
            .groups
            .entry((outcome, element.to_owned()))
            .or_default();
        group.attributes += 1;
        match outcome {
            RemovalRule::StyleRejected | RemovalRule::StyleDeclarationRejected => {
                judge_drops(&removal, &pieces, grammar, &mut self.unclear_budget, group);
            }
            // Only re-spelled: planned, unless a reader saw another value.
            // The three re-spellings that §7.1 names already count the
            // attribute when they occur in it; `style_respelled` counts the
            // rest, so that every re-spelled attribute counts, once.
            _ if overridden.is_empty() && named_respellings == 0 => {
                *group.planned.entry(Rule::StyleRespelled).or_insert(0) += 1;
            }
            _ => {}
        }
        for property in overridden {
            *group
                .unplanned
                .entry(unplanned(&removal, Some(&property)))
                .or_insert(0) += 1;
        }
    }
}

/// One declaration of a `style` as written: its text, as
/// `blocks::declaration_texts` cuts it, and what the sanitiser's reader
/// makes of it.
#[derive(Debug, Clone, PartialEq)]
struct Piece<'a> {
    /// The text, with the `;` that ends it, if one does.
    text: &'a str,
    /// Whether a top-level `;` ends it.
    ended: bool,
    /// What the reader makes of it.
    kind: PieceKind,
}

/// What the sanitiser's reader makes of one [`Piece`].
#[derive(Debug, Clone, PartialEq)]
enum PieceKind {
    /// White space, comments or nothing between two `;`: no declaration.
    Empty,
    /// A declaration the reader reads.
    Read {
        /// The declaration, without its `!important`.
        declaration: Declaration,
        /// Whether it is marked `!important`, which the reader drops.
        important: bool,
    },
    /// A declaration the reader does not read, or one after its first
    /// [`MAX_DECLARATIONS`].
    Unreadable {
        /// The property the text names, lower-case, when it starts with a
        /// name and a colon and lies within the reader's limit.
        name: Option<String>,
    },
}

/// `value` cut into its declarations as written, each read on its own.
/// The reader reads a `style` one top-level `;` at a time, so the pieces
/// read as the whole does (a test checks it), counting
/// [`MAX_DECLARATIONS`] the same way.
///
/// - `value`: a `style` attribute value, references decoded.
fn pieces(value: &str) -> Vec<Piece<'_>> {
    let mut counted = 0;
    declaration_texts(value)
        .into_iter()
        .map(|(text, ended)| {
            let read = declarations::read(text);
            let kind = if read.read.is_empty() && read.unreadable == 0 {
                PieceKind::Empty
            } else {
                counted += 1;
                match (read.read.into_iter().next(), read.unreadable) {
                    (Some(declaration), 0) if counted <= MAX_DECLARATIONS => PieceKind::Read {
                        important: is_important(text),
                        declaration,
                    },
                    _ => PieceKind::Unreadable {
                        name: (counted <= MAX_DECLARATIONS)
                            .then(|| declared_name(text))
                            .flatten(),
                    },
                }
            };
            Piece { text, ended, kind }
        })
        .collect()
}

/// The property that the declaration `text` names: its first token, when
/// it is an identifier followed by a colon, ASCII-lower-cased. Comments
/// and white space before them are skipped, as a browser skips them.
///
/// - `text`: one declaration as written.
fn declared_name(text: &str) -> Option<String> {
    let mut input = Parser::new(text);
    let name = input.expect_ident().ok()?.to_ascii_lowercase();
    input.expect_colon().ok()?;
    Some(name)
}

/// The properties of the canonical style `written` whose value is not the
/// one a browser applied, because the input marked a declaration
/// `!important` and a later one of the same property overrides it in the
/// canonical cascade (see the module documentation). Empty when nothing is
/// marked, or the marks change nothing.
///
/// - `element`: the tag name.
/// - `pieces`: the style's declarations as written.
/// - `written`: what the canonical writer writes for the style as written.
fn overridden_by_importance(
    element: &str,
    pieces: &[Piece<'_>],
    written: Option<&str>,
) -> Vec<String> {
    let important = |piece: &&Piece<'_>| {
        matches!(
            piece.kind,
            PieceKind::Read {
                important: true,
                ..
            }
        )
    };
    if !pieces.iter().any(|piece| important(&piece)) {
        return Vec::new();
    }
    // The readable declarations only, each ended with its own `;`: one the
    // reader cannot read sets nothing in either cascade, and an unended
    // last one would swallow what follows it.
    let readable = pieces
        .iter()
        .filter(|piece| matches!(piece.kind, PieceKind::Read { .. }));
    let (marked, plain): (Vec<_>, Vec<_>) = readable.partition(important);
    let shown: String = plain
        .iter()
        .chain(&marked)
        .map(|piece| {
            if piece.ended {
                format!("{} ", piece.text)
            } else {
                format!("{}; ", piece.text)
            }
        })
        .collect();
    let shown = css::sanitize_style(element, &shown).css;
    differing_properties(written, shown.as_deref())
}

/// The properties whose values differ between two canonical style strings,
/// or that only one of them declares, in name order.
///
/// - `left`, `right`: strings that the canonical writer wrote, or `None`
///   for no style.
fn differing_properties(left: Option<&str>, right: Option<&str>) -> Vec<String> {
    let declared = |css: Option<&str>| -> BTreeMap<String, String> {
        css.into_iter()
            .flat_map(|css| css.split("; "))
            .filter_map(|declaration| declaration.split_once(": "))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    };
    let (left, right) = (declared(left), declared(right));
    left.keys()
        .chain(right.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|name| left.get(*name) != right.get(*name))
        .cloned()
        .collect()
}

/// Where a counted part of the canonical style comes from: the index of its
/// declaration among the style's pieces, and of the component within that
/// declaration's value. One part written on several border sides is
/// counted once.
type Origin = (usize, usize);

/// The counted parts in force for one cell border side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SideParts {
    /// The absolute length that sets its width.
    width: Option<Origin>,
    /// The named colour that sets its colour.
    colour: Option<Origin>,
    /// Whether a declaration in force gave it a width or a colour, which
    /// [`Rule::BorderSideToNone`] counts on a side written as `none`.
    given: bool,
}

/// The counted re-spellings of one `style` (see the module documentation):
/// the named colours and the absolute lengths in the declarations in force
/// that the canonical style writes, and the cell border sides written as
/// `none` that the input gave a width or a colour. The cascade mirrors the
/// canonical writer's (`css::canonical::Cascade`): the last accepted
/// declaration of a property wins, a default unsets it, `border` and
/// `border-<side>` replace a side, and `border-width` and `border-color`
/// change one part of every side.
///
/// - `pieces`: the style's declarations as written.
/// - `grammar`: the element's grammar.
/// - `written`: what the canonical writer writes for the style, or `None`.
fn respellings(
    pieces: &[Piece<'_>],
    grammar: &ElementStyle,
    written: Option<&str>,
) -> [(Rule, usize); 3] {
    let mut slots: BTreeMap<&'static str, (Vec<Origin>, Vec<Origin>)> = BTreeMap::new();
    let mut sides: Option<[SideParts; 4]> = None;
    for (index, piece) in pieces.iter().enumerate() {
        let PieceKind::Read { declaration, .. } = &piece.kind else {
            continue;
        };
        let Some(property) = grammar.property(&declaration.name) else {
            continue;
        };
        let at = |test: fn(&Component) -> bool| -> Vec<Origin> {
            declaration
                .value
                .iter()
                .enumerate()
                .filter(|(_, component)| test(component))
                .map(|(component, _)| (index, component))
                .collect()
        };
        match grammar::parse(property.grammar, &declaration.value) {
            Reading::Rejected => {}
            Reading::Default => {
                slots.remove(slot(property.name));
            }
            Reading::Accepted(Value::BorderSide(side)) => {
                let parts = SideParts {
                    width: at(is_absolute_length).first().copied(),
                    colour: at(is_named_colour).first().copied(),
                    given: side.width.is_some() || side.colour.is_some(),
                };
                let sides = sides.get_or_insert_default();
                for &named in sides_named(property.name) {
                    sides[named] = parts;
                }
            }
            Reading::Accepted(Value::Widths(_) | Value::Colours(_) | Value::Styles(_)) => {
                let sides = sides.get_or_insert_default();
                let components: Vec<usize> = (0..declaration.value.len()).collect();
                let Some(per_side) = four(&components) else {
                    continue;
                };
                for (side, component) in sides.iter_mut().zip(per_side) {
                    let part = &declaration.value[component];
                    let origin = Some((index, component));
                    match declaration.name.as_str() {
                        "border-width" => {
                            side.width = origin.filter(|_| is_absolute_length(part));
                            side.given = true;
                        }
                        "border-color" => {
                            side.colour = origin.filter(|_| is_named_colour(part));
                            side.given = true;
                        }
                        _ => {}
                    }
                }
            }
            Reading::Accepted(_) => {
                let colours = if takes_colour(property.grammar) {
                    at(is_named_colour)
                } else {
                    Vec::new()
                };
                slots.insert(slot(property.name), (colours, at(is_absolute_length)));
            }
        }
    }
    let written_side = |side: &str| {
        written
            .into_iter()
            .flat_map(|css| css.split("; "))
            .find_map(|declaration| declaration.strip_prefix(side)?.strip_prefix(": "))
    };
    let mut colours: BTreeSet<Origin> = BTreeSet::new();
    let mut lengths: BTreeSet<Origin> = BTreeSet::new();
    for (in_force_colours, in_force_lengths) in slots.values() {
        colours.extend(in_force_colours);
        lengths.extend(in_force_lengths);
    }
    let mut to_none = 0;
    for (name, parts) in SIDES.iter().zip(sides.unwrap_or_default()) {
        match written_side(name) {
            Some("none") => to_none += usize::from(parts.given),
            Some("hidden") | None => {}
            Some(_) => {
                colours.extend(parts.colour);
                lengths.extend(parts.width);
            }
        }
    }
    [
        (Rule::NamedColourToHex, colours.len()),
        (Rule::BorderSideToNone, to_none),
        (Rule::LengthToPx, lengths.len()),
    ]
}

/// The canonical cascade's slot for `property`: `background` sets
/// `background-color`, as the canonical writer stores it; every other
/// property is its own.
///
/// - `property`: a property name as the grammar lists it.
fn slot(property: &'static str) -> &'static str {
    match property {
        "background" => "background-color",
        other => other,
    }
}

/// The indexes into [`SIDES`] that a border-side shorthand sets: all four
/// for `border`, its own for `border-<side>`.
///
/// - `property`: a property whose grammar reads one border side.
fn sides_named(property: &str) -> &'static [usize] {
    match property {
        "border" => &[0, 1, 2, 3],
        "border-top" => &[0],
        "border-right" => &[1],
        "border-bottom" => &[2],
        "border-left" => &[3],
        _ => &[],
    }
}

/// Whether a property of grammar `grammar` takes colours, so that a named
/// colour in it is written as hex.
///
/// - `grammar`: a property's grammar.
const fn takes_colour(grammar: Grammar) -> bool {
    matches!(
        grammar,
        Grammar::Colour { .. } | Grammar::BorderSide | Grammar::BorderColours
    )
}

/// Whether `component` is a named colour that the canonical writer writes
/// as hex: a colour keyword, not `transparent` or `currentcolor`, which it
/// keeps as keywords.
///
/// - `component`: one component of an accepted value.
fn is_named_colour(component: &Component) -> bool {
    matches!(component, Component::Ident(name)
        if name != "transparent" && name != "currentcolor" && values::colour(component).is_some())
}

/// Whether `component` is a length in an absolute unit other than px.
///
/// - `component`: one component of an accepted value.
fn is_absolute_length(component: &Component) -> bool {
    matches!(component, Component::Dimension { unit, .. } if ABSOLUTE_UNITS.contains(&unit.as_str()))
}

/// Judges every declaration that one `style` loses (see the module
/// documentation) and adds it to `group`.
///
/// - `removal`: the sanitiser's rule, element and attribute for the style.
/// - `pieces`: the style's declarations as written.
/// - `grammar`: the element's grammar.
/// - `unclear_budget`: how many more alignments written in front may be
///   kept over a dropped `text-align`; the alignments of this style are
///   taken from it.
/// - `group`: where the drops are counted.
fn judge_drops(
    removal: &Removal,
    pieces: &[Piece<'_>],
    grammar: &ElementStyle,
    unclear_budget: &mut usize,
    group: &mut StyleGroup,
) {
    let mut planned: BTreeMap<Rule, usize> = BTreeMap::new();
    // The dropped declarations that no rule plans, by property.
    let mut dropped: Vec<Option<&str>> = Vec::new();
    // The dropped `text-align` declarations, read or unreadable.
    let mut alignments: Vec<Option<&str>> = Vec::new();
    for piece in pieces {
        match &piece.kind {
            PieceKind::Empty => {}
            PieceKind::Read { declaration, .. } => {
                let name = declaration.name.as_str();
                let Some(property) = grammar.property(name) else {
                    dropped.push(Some(name));
                    continue;
                };
                if grammar::parse(property.grammar, &declaration.value) != Reading::Rejected {
                    continue;
                }
                if invalid_css::is_invalid(name, &declaration.value) {
                    *planned.entry(Rule::InvalidStyleValueDropped).or_insert(0) += 1;
                } else if name == "text-align" {
                    alignments.push(Some(name));
                } else {
                    dropped.push(Some(name));
                }
            }
            PieceKind::Unreadable { name } if name.as_deref() == Some("text-align") => {
                // Unreadable, so it names no property in the report.
                alignments.push(None);
            }
            PieceKind::Unreadable { .. } => dropped.push(None),
        }
    }
    if !alignments.is_empty() {
        let written = alignments_written_in_front(pieces);
        if written > 0 && written <= *unclear_budget {
            *unclear_budget -= written;
            *planned.entry(Rule::UnclearAlignmentDropped).or_insert(0) += alignments.len();
        } else {
            dropped.extend(alignments);
        }
    }
    for (rule, count) in planned {
        *group.planned.entry(rule).or_insert(0) += count;
    }
    for property in dropped {
        *group
            .unplanned
            .entry(unplanned(removal, property))
            .or_insert(0) += 1;
    }
}

/// How many alignments N3 and N5 wrote in front of the style: the leading
/// pieces that are exactly what `dom::prepend_declaration` writes for an
/// alignment content-v1 keeps, `text-align: center;`, each followed by one
/// space and the rest of the old value, which it starts with no white
/// space. A hand-written `TEXT-ALIGN: center;`, `text-align:center;` or
/// `text-align: center;text-align: inherit` is not that shape.
///
/// - `pieces`: the style's declarations as written.
fn alignments_written_in_front(pieces: &[Piece<'_>]) -> usize {
    // The first piece starts the attribute value; each later one starts
    // with the one space that follows the `;`.
    let after_separator = |index: usize, text: &'_ str| -> Option<String> {
        if index == 0 {
            return Some(text.to_owned());
        }
        text.strip_prefix(' ')
            .filter(|rest| !rest.starts_with(|c: char| c.is_ascii_whitespace()))
            .map(str::to_owned)
    };
    let written = pieces
        .iter()
        .enumerate()
        .take_while(|&(index, piece)| {
            piece.ended
                && after_separator(index, piece.text).is_some_and(|text| {
                    WRITTEN_ALIGNMENTS
                        .iter()
                        .any(|alignment| text == format!("text-align: {alignment};"))
                })
        })
        .count();
    match pieces.get(written) {
        Some(next) if written > 0 && after_separator(written, next.text).is_none() => 0,
        _ => written,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::normalise::Step;
    use crate::domain::rich_text::policy::{TABLE_STYLE, TBODY_STYLE};
    use proptest::prelude::*;
    use std::sync::LazyLock;

    /// The content-v1 sanitiser, built once: the proptests run N7 over
    /// thousands of bodies.
    static SANITIZER: LazyLock<Sanitizer> = LazyLock::new(sanitizer);

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
    }

    /// N7 over `body` in a pass where N3 and N5 counted no unclear block:
    /// the output and the report.
    fn n7(body: &str) -> (String, NormaliseReport) {
        n7_after(body, 0)
    }

    /// N7 over `body` in a pass where N3 and N5 counted `unclear` blocks as
    /// unclear: the output and the report.
    fn n7_after(body: &str, unclear: usize) -> (String, NormaliseReport) {
        let mut report = NormaliseReport::default();
        let out =
            sanitise(body, &SANITIZER, unclear, &mut report).expect("a body within the limits");
        (out, report)
    }

    /// The N7 hits of `report`.
    fn hits(report: &NormaliseReport) -> Vec<(Rule, usize)> {
        report.hits().collect()
    }

    /// The unplanned removals of `report`, shown.
    fn unplanned(report: &NormaliseReport) -> Vec<(String, usize)> {
        report
            .unplanned_removals()
            .map(|(removal, count)| (removal.to_string(), count))
            .collect()
    }

    /// `cells` in a content-v1 table, so that only the cells' own markup
    /// is under test.
    fn table(cells: &str) -> String {
        format!(
            r#"<table style="{TABLE_STYLE}"><tbody style="{TBODY_STYLE}"><tr>{cells}</tr></tbody></table>"#
        )
    }

    #[test]
    fn the_planned_table_attributes_are_counted() {
        let (out, report) = n7(&table("<td>x</td>").replace(
            "<table ",
            r#"<table border="1" cellspacing="0" cellpadding="2" "#,
        ));
        assert_eq!(out, table("<td>x</td>"));
        assert_eq!(
            hits(&report),
            [
                (Rule::TableBorderDropped, 1),
                (Rule::TableCellspacingDropped, 1),
                (Rule::TableCellpaddingDropped, 1),
            ]
        );
        assert!(!report.has_unplanned_removals());
    }

    #[test]
    fn the_planned_iframe_attributes_are_counted() {
        let canonical = concat!(
            r#"<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/AbCdEfGhIjK" width="640" height="360" title="Videó" "#,
            r#"style="aspect-ratio: 16 / 9; height: auto; max-width: 640px; width: 100%" "#,
            r#"allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>"#,
        );
        let legacy = canonical
            .replace("style=\"", "style=\"border: none; overflow: hidden; ")
            .replace("></iframe>", r#" tabindex="-1" scrolling="no"></iframe>"#);
        let (out, report) = n7(&legacy);
        assert_eq!(out, canonical);
        assert_eq!(
            hits(&report),
            [
                (Rule::IframeTabindexDropped, 1),
                (Rule::IframeScrollingDropped, 1),
                (Rule::IframeStyleDropped, 1),
            ]
        );
        assert!(!report.has_unplanned_removals());
        let (_, report) = n7(&canonical.replace(
            r#"style="aspect-ratio: 16 / 9; height: auto; max-width: 640px; width: 100%""#,
            r#"style="border: 0""#,
        ));
        assert_eq!(
            report.count(Rule::IframeStyleDropped),
            1,
            "a whole style that goes"
        );
    }

    #[test]
    fn comments_and_scripts_are_planned() {
        let (out, report) = n7("<p>Szöveg<!-- megjegyzés --></p><script>alert(1)</script>");
        assert_eq!(out, "<p>Szöveg</p>");
        assert_eq!(
            hits(&report),
            [(Rule::CommentRemoved, 1), (Rule::ScriptRemoved, 1)]
        );
        assert!(!report.has_unplanned_removals());
    }

    #[test]
    fn named_colours_absolute_lengths_and_border_sides_are_counted_respellings() {
        let (out, report) = n7(&table(concat!(
            r#"<td style="border-top: 1px solid black; background-color: WhiteSmoke; border-left: 1pt solid windowtext">a</td>"#,
            r#"<td style="width: 3cm; height: 12pt; border-bottom: medium; border-right: medium none currentcolor; border-left: none">b</td>"#,
        )));
        assert_eq!(
            out,
            table(concat!(
                r#"<td style="background-color: #f5f5f5; border-bottom: none; border-left: 1.33px solid #000000; border-right: none; border-top: 1px solid #000000">a</td>"#,
                r#"<td style="border-bottom: none; border-left: none; border-right: none; border-top: none; box-sizing: content-box; height: 16px; width: 113.39px">b</td>"#,
            ))
        );
        assert_eq!(
            hits(&report),
            [
                (Rule::NamedColourToHex, 3),
                (Rule::BorderSideToNone, 2),
                (Rule::LengthToPx, 3),
            ],
            "the plain `none` and the sides no declaration set are not counted, and the \
             re-spelled attributes are counted by these rules alone"
        );
        assert!(
            !report.has_unplanned_removals(),
            "re-spellings remove nothing"
        );
    }

    #[test]
    fn an_invalid_value_is_a_planned_drop_and_a_valid_one_is_not() {
        let (out, report) = n7(&table(r#"<td style="width: 100%px; height: 20px">x</td>"#));
        assert_eq!(
            out,
            table(r#"<td style="box-sizing: content-box; height: 20px">x</td>"#)
        );
        assert_eq!(hits(&report), [(Rule::InvalidStyleValueDropped, 1)]);
        assert!(!report.has_unplanned_removals());

        // A percentage is valid CSS that content-v1 does not keep on a cell.
        let (_, report) = n7(&table(r#"<td style="width: 50%">x</td>"#));
        assert!(!report.has_hits());
        assert_eq!(
            unplanned(&report),
            [("style_rejected td[style] width".to_owned(), 1)]
        );
    }

    #[test]
    fn a_dropped_background_on_a_marker_is_reviewed() {
        // D145: N5 writes the yellow in front; `background: none` was valid
        // CSS that hid it, so its drop goes to review, while an invalid
        // `background` hid nothing.
        let (out, report) =
            n7(r#"<p><span style="background-color: #ffff00; background: none">a</span></p>"#);
        assert_eq!(
            out,
            r#"<p><span style="background-color: #ffff00">a</span></p>"#
        );
        assert_eq!(
            unplanned(&report),
            [(
                "style_declaration_rejected span[style] background".to_owned(),
                1
            )]
        );
        let (_, report) =
            n7(r#"<p><span style="background-color: #ffff00; background: nope">a</span></p>"#);
        assert_eq!(hits(&report), [(Rule::InvalidStyleValueDropped, 1)]);
        assert!(!report.has_unplanned_removals());
    }

    #[test]
    fn the_unclear_alignment_behind_n3_and_n5_is_a_planned_drop() {
        for style in [
            "text-align: center; text-align: inherit",
            "text-align: right; text-align: start",
            "text-align: center; text-align: var(--x)",
        ] {
            let (out, report) = n7_after(&format!(r#"<p style="{style}">x</p>"#), 1);
            let written = style.split(';').next().expect("a first declaration");
            assert_eq!(out, format!(r#"<p style="{written}">x</p>"#), "{style}");
            assert_eq!(
                hits(&report),
                [(Rule::UnclearAlignmentDropped, 1)],
                "{style}"
            );
            assert!(!report.has_unplanned_removals(), "{style}");
        }
        // Without an alignment in front, the drop changes what a reader
        // saw, whatever N3 and N5 counted.
        for (style, rule) in [
            ("text-align: inherit", "style_rejected"),
            (
                "text-align: start; text-align: center",
                "style_declaration_rejected",
            ),
        ] {
            let (_, report) = n7_after(&format!(r#"<p style="{style}">x</p>"#), 1);
            assert_eq!(
                unplanned(&report),
                [(format!("{rule} p[style] text-align"), 1)],
                "{style}"
            );
        }
        // Another unreadable declaration after the alignment stays
        // unplanned.
        let (_, report) = n7_after(r#"<p style="text-align: center; color: var(--x)">x</p>"#, 1);
        assert_eq!(
            unplanned(&report),
            [(
                "style_declaration_rejected p[style] (unreadable)".to_owned(),
                1
            )]
        );
    }

    #[test]
    fn properties_outside_content_v1_are_unplanned() {
        let (out, report) = n7(&table(
            r#"<td style="font-family: Arial; padding: 2px; mso-number-format: x; vertical-align: top">x</td>"#,
        ));
        assert_eq!(out, table(r#"<td style="vertical-align: top">x</td>"#));
        assert_eq!(
            unplanned(&report),
            [
                (
                    "style_declaration_rejected td[style] font-family".to_owned(),
                    1
                ),
                (
                    "style_declaration_rejected td[style] mso-number-format".to_owned(),
                    1
                ),
                ("style_declaration_rejected td[style] padding".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn removals_outside_the_plan_are_unplanned() {
        let (out, report) = n7(concat!(
            r#"<div class="x"><p class="lead">a</p></div>"#,
            r#"<p><strong style="font-weight: normal">b</strong></p>"#,
            r#"<p><img src="data:image/png;base64,AA" alt="kép"></p>"#,
        ));
        assert_eq!(out, "<p>a</p><p><strong>b</strong></p><p></p>");
        assert_eq!(
            unplanned(&report),
            [
                ("element_unwrapped div".to_owned(), 1),
                ("attribute_not_allowed p[class]".to_owned(), 1),
                ("attribute_not_allowed strong[style]".to_owned(), 1),
                ("image_src_rejected img[src]".to_owned(), 1),
                ("image_without_src_removed img".to_owned(), 1),
            ],
            "D146: the style of a renamed mark is reviewed"
        );
        assert!(!report.has_hits());
    }

    #[test]
    fn a_content_v1_body_records_nothing() {
        let body = table(
            r#"<td style="border-bottom: none; border-left: none; border-right: none; border-top: 1px solid #000000">x</td>"#,
        );
        let (out, report) = n7(&body);
        assert_eq!(out, body);
        assert_eq!(report, NormaliseReport::default());
    }

    #[test]
    fn a_refused_body_is_passed_on() {
        let mut report = NormaliseReport::default();
        let result = sanitise(&"<div>".repeat(300), &SANITIZER, 0, &mut report);
        let Err(NormaliseError::SanitiseRefused(error)) = result else {
            panic!("{result:?}");
        };
        assert_eq!(error.code(), Some("content_too_deep"));
    }

    #[test]
    fn a_reading_that_disagrees_with_the_report_errs_towards_review() {
        // The report names an invalid cell width that the reading of this
        // input does not find: the attribute is unplanned, although the
        // drop it names would have been planned.
        let removals = sanitizer()
            .sanitize(&table(r#"<td style="width: 100%px">x</td>"#))
            .expect("a small body")
            .report;
        let mut report = NormaliseReport::default();
        review(
            &table("<td>x</td>"),
            SANITIZER.policy(),
            0,
            &removals,
            &mut report,
        );
        assert_eq!(
            unplanned(&report),
            [("style_rejected td[style] (unreadable)".to_owned(), 1)]
        );
        assert!(!report.has_hits());
    }

    #[test]
    fn a_group_whose_count_differs_from_the_report_errs_towards_review() {
        // The report names two re-spelled cell styles; the reading of this
        // input finds one. However plain each re-spelling is, the count
        // does not match, so both are unplanned.
        let removals = sanitizer()
            .sanitize(&table(
                r#"<td style="background-color: #FF0000">x</td><td style="background-color: #00FF00">y</td>"#,
            ))
            .expect("a small body")
            .report;
        let mut report = NormaliseReport::default();
        review(
            &table(r#"<td style="background-color: #FF0000">x</td><td>y</td>"#),
            SANITIZER.policy(),
            0,
            &removals,
            &mut report,
        );
        assert_eq!(
            unplanned(&report),
            [("value_canonicalised td[style]".to_owned(), 2)]
        );
        assert!(!report.has_hits());
    }

    #[test]
    fn an_important_declaration_that_the_canonical_style_overrides_is_reviewed() {
        // A browser applied the `!important` declaration; the canonical
        // cascade keeps the later one. The reader saw red, the stored style
        // says blue: the sanitiser calls it a re-spelling, N7 does not.
        for (body, removal) in [
            (
                r#"<p><span style="color: red !important; color: blue">x</span></p>"#,
                "value_canonicalised span[style] color",
            ),
            (
                r#"<p style="text-align: right !important; text-align: center">x</p>"#,
                "value_canonicalised p[style] text-align",
            ),
            (
                r#"<p><span style="color: red ! IMPORTANT; color: blue; font-family: Arial">x</span></p>"#,
                "style_declaration_rejected span[style] color",
            ),
        ] {
            let (_, report) = n7(body);
            assert!(
                unplanned(&report).contains(&(removal.to_owned(), 1)),
                "{body}: {report:?}"
            );
            assert_eq!(report.count(Rule::StyleRespelled), 0, "{body}");
        }
        // The cell side's colour comes from the important shorthand, which
        // the later longhand overrides in the canonical cascade.
        let (_, report) = n7(&table(
            r#"<td style="border-top: 1px solid red !important; border-color: blue">x</td>"#,
        ));
        assert_eq!(
            unplanned(&report),
            [("value_canonicalised td[style] border-top".to_owned(), 1)]
        );
        // Marks that change nothing: the important declaration is the last
        // one, or the only one, or another property's.
        for body in [
            r#"<p><span style="color: #ff0000; color: #0000FF !important">x</span></p>"#,
            r#"<p><span style="color: #FF0000 !important">x</span></p>"#,
            r#"<p><span style="color: #ff0000 !important; background-color: #0000FF">x</span></p>"#,
        ] {
            let (_, report) = n7(body);
            assert!(!report.has_unplanned_removals(), "{body}: {report:?}");
            assert_eq!(report.count(Rule::StyleRespelled), 1, "{body}");
        }
    }

    #[test]
    fn only_the_parts_the_canonical_style_writes_are_counted() {
        for (style, expected) in [
            // An overridden named colour is not written.
            (
                "background-color: black; background-color: #ff0000",
                [0, 0, 0],
            ),
            ("background-color: #ff0000; background: black", [1, 0, 0]),
            // `transparent` and `currentcolor` stay keywords.
            ("background-color: transparent", [0, 0, 0]),
            // A side that draws no border writes neither colour nor width,
            // but counts as written as `none`.
            ("border-top: 1pt none black", [0, 1, 0]),
            // One colour or width written on four sides counts once.
            ("border: 1pt solid black", [1, 0, 1]),
            (
                "border-style: solid; border-color: black; border-width: 2pt",
                [1, 0, 1],
            ),
            // `border` replaces what an earlier side declaration gave.
            ("border-top: medium; border: none", [0, 0, 0]),
            (
                "border-top: 1pt solid black; border-top-color: #ff0000",
                [1, 0, 1],
            ),
            // `border-color` gives the three sides without a style a colour,
            // and they are written as `none`.
            (
                "border-top: 1pt solid black; border-color: #ff0000",
                [0, 3, 1],
            ),
            // An overridden length is not written.
            ("width: 2cm; width: 10px", [0, 0, 0]),
            ("width: 10px; width: 2cm; height: 12pt", [0, 0, 2]),
        ] {
            let (_, report) = n7(&table(&format!(r#"<td style="{style}">x</td>"#)));
            assert_eq!(
                [
                    report.count(Rule::NamedColourToHex),
                    report.count(Rule::BorderSideToNone),
                    report.count(Rule::LengthToPx),
                ],
                expected,
                "{style}"
            );
        }
    }

    #[test]
    fn the_locked_markup_the_sanitiser_adds_is_counted() {
        for (body, added) in [
            // A link gets `target` and `rel`, whatever its `href`.
            (r#"<p><a href="/x">x</a></p>"#, 1),
            ("<p><a>x</a></p>", 1),
            (r#"<p><a href="/x" target="_blank">x</a></p>"#, 1),
            // A table and its `tbody` get their locked styles.
            ("<table><tbody><tr><td>x</td></tr></tbody></table>", 2),
            // An embed gets its forced attributes.
            (
                r#"<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/AbCdEfGhIjK"></iframe></p>"#,
                1,
            ),
            // An image the structural pass keeps gets its locked style; one
            // it removes gets nothing.
            (
                r#"<p><img src="/storage/uploads/posts/1/a.jpg" alt="kép"></p>"#,
                1,
            ),
            (
                r#"<p><img src="https://example.org/a.png" alt="kép"></p>"#,
                0,
            ),
            (r#"<p><img alt="kép"></p>"#, 0),
            // Content-v1 already.
            (
                r#"<p><a href="/x" target="_blank" rel="noopener noreferrer">x</a></p>"#,
                0,
            ),
        ] {
            let (_, report) = n7(body);
            assert_eq!(report.count(Rule::LockedMarkupAdded), added, "{body}");
        }
        // A link fostered into another one: N7's parse nests them, the
        // sanitiser's parse of ammonia's output takes them apart again, and
        // both links get their markup, as do the table and its `tbody`.
        let (_, report) =
            n7(r#"<a href="/1"><table><tr><td>x</td></tr><a href="/2">y</table></a>"#);
        assert_eq!(report.count(Rule::LockedMarkupAdded), 4, "{report:?}");
    }

    #[test]
    fn a_replaced_target_and_other_re_spelled_attributes_are_counted() {
        let (out, report) =
            n7(r#"<p><a href="/x" target="_self" rel="noopener noreferrer">x</a></p>"#);
        assert_eq!(
            out,
            r#"<p><a href="/x" target="_blank" rel="noopener noreferrer">x</a></p>"#
        );
        assert_eq!(hits(&report), [(Rule::LinkTargetReplaced, 1)]);
        let (_, report) = n7(r#"<p><a href="/x" target="_blank" rel="nofollow">x</a></p>"#);
        assert_eq!(hits(&report), [(Rule::AttributeRespelled, 1)]);
        let (_, report) = n7(r#"<ol start="1"><li>x</li></ol>"#);
        assert_eq!(hits(&report), [(Rule::AttributeRespelled, 1)]);
        let (_, report) = n7(r#"<ul style="list-style-type: disc"><li>x</li></ul>"#);
        assert_eq!(
            hits(&report),
            [(Rule::StyleRespelled, 1)],
            "a default left out"
        );
        assert!(!report.has_unplanned_removals());
    }

    #[test]
    fn the_unclear_alignment_is_planned_only_within_the_unclear_blocks() {
        let style = |style: &str| format!(r#"<p style="{style}">x</p>"#);
        // N3 or N5 counted no unclear block in the pass: a hand-written
        // style of their shape is reviewed.
        let (_, report) = n7_after(&style("text-align: center; text-align: inherit"), 0);
        assert_eq!(
            unplanned(&report),
            [(
                "style_declaration_rejected p[style] text-align".to_owned(),
                1
            )]
        );
        // One unclear block plans one style; the second is reviewed.
        let (_, report) = n7_after(
            &format!(
                "{}{}",
                style("text-align: center; text-align: inherit"),
                style("text-align: left; text-align: start")
            ),
            1,
        );
        assert_eq!(report.count(Rule::UnclearAlignmentDropped), 1);
        assert_eq!(
            unplanned(&report),
            [(
                "style_declaration_rejected p[style] text-align".to_owned(),
                1
            )]
        );
        // N3 and N5 both wrote in front of the same paragraph and both
        // counted it.
        let both = style("text-align: right; text-align: center; text-align: inherit");
        assert_eq!(n7_after(&both, 2).1.count(Rule::UnclearAlignmentDropped), 1);
        assert!(n7_after(&both, 1).1.has_unplanned_removals());
        // Only the exact bytes N3 and N5 write are their shape.
        for hand_written in [
            "TEXT-ALIGN: center; text-align: inherit",
            "text-align:center; text-align: inherit",
            "text-align: center;text-align: inherit",
            " text-align: center; text-align: inherit",
            "text-align: center ; text-align: inherit",
        ] {
            let (_, report) = n7_after(&style(hand_written), 1);
            assert!(report.has_unplanned_removals(), "{hand_written}");
            assert_eq!(
                report.count(Rule::UnclearAlignmentDropped),
                0,
                "{hand_written}"
            );
        }
        // An unreadable `text-align` behind it is planned, named by its
        // text; an unreadable declaration of another property whose
        // comment mentions `text-align` is not.
        let (_, report) = n7_after(
            &style("text-align: center; /* régi */ text-align: inherit"),
            1,
        );
        assert_eq!(hits(&report), [(Rule::UnclearAlignmentDropped, 1)]);
        let (_, report) = n7_after(
            &style("text-align: center; color: red /* text-align: left */"),
            1,
        );
        assert_eq!(report.count(Rule::UnclearAlignmentDropped), 0);
        assert_eq!(
            unplanned(&report),
            [(
                "style_declaration_rejected p[style] (unreadable)".to_owned(),
                1
            )]
        );
    }

    #[test]
    fn the_pieces_name_their_properties() {
        assert_eq!(
            declared_name("text-align: x;"),
            Some("text-align".to_owned())
        );
        assert_eq!(
            declared_name(" /* c */ TEXT-Align : x"),
            Some("text-align".to_owned())
        );
        assert_eq!(declared_name("color red"), None);
        assert_eq!(declared_name(": x"), None);
        assert_eq!(declared_name(""), None);
        let kinds: Vec<_> = pieces("color: red !important; ; x: url(y); width: 1px")
            .into_iter()
            .map(|piece| piece.kind)
            .collect();
        assert!(
            matches!(
                &kinds[..],
                [
                    PieceKind::Read { important: true, .. },
                    PieceKind::Empty,
                    PieceKind::Unreadable { name: Some(name) },
                    PieceKind::Read { important: false, .. },
                ] if name == "x"
            ),
            "{kinds:?}"
        );
        // Beyond the reader's limit, a declaration is unreadable and names
        // nothing.
        let many = vec!["color: red"; MAX_DECLARATIONS + 1].join("; ");
        let last = pieces(&many).pop().expect("a piece");
        assert_eq!(last.kind, PieceKind::Unreadable { name: None });
    }

    #[test]
    fn the_properties_that_differ_are_listed_in_name_order() {
        assert_eq!(differing_properties(None, None), Vec::<String>::new());
        assert_eq!(
            differing_properties(
                Some("color: #ff0000; width: 1px"),
                Some("color: #0000ff; width: 1px")
            ),
            ["color"]
        );
        assert_eq!(
            differing_properties(Some("width: 1px"), Some("height: 1px")),
            ["height", "width"]
        );
        assert_eq!(differing_properties(Some("width: 1px"), None), ["width"]);
    }

    #[test]
    fn the_planned_list_names_every_planned_removal_once() {
        for (index, planned) in PLANNED_REMOVALS.iter().enumerate() {
            assert_eq!(planned.rule.step(), Step::N7, "{planned:?}");
            assert!(
                PLANNED_REMOVALS[..index].iter().all(|earlier| {
                    (earlier.removal, earlier.element, earlier.attribute)
                        != (planned.removal, planned.element, planned.attribute)
                }),
                "{planned:?} is listed twice"
            );
        }
    }

    /// Declarations of every kind N7 judges: kept, re-spelled, invalid,
    /// valid outside content-v1, unreadable, and the alignments.
    #[rustfmt::skip]
    const DECLARATIONS: &[&str] = &[
        "width: 100px", "width: 100%px", "width: 50%", "width: 2cm", "height: 12pt", "height: 0",
        "border-top: 1px solid black", "border-top: medium", "border: medium none currentcolor",
        "border-width: 1px", "border-color: red", "border-style: solid", "border-left: 25px solid",
        "background-color: WhiteSmoke", "background: none", "background: nope", "color: red",
        "color: transparent", "font-size: 1.5em", "font-size: 12px", "text-align: center",
        "text-align: inherit", "text-align: var(--x)", "vertical-align: top", "white-space: pre",
        "font-family: Arial", "list-style-type: disc", "box-sizing: border-box", "padding: 1px",
        "x: url(y)", "", "display: block", "color: blue !important", "width: 2cm !important",
        "border-top: 1pt solid black !important", "text-align: right !important", "color: black",
        "border-color: black", "border-width: 1pt",
    ];

    fn arb_style() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(DECLARATIONS), 0..6)
            .prop_map(|declarations| declarations.join("; "))
    }

    proptest! {
        // The second reading always agrees with the sanitiser's report
        // about which attributes lost declarations, so the review never
        // falls back to its unplanned-without-property line.
        #[test]
        fn the_reading_explains_every_style_removal(
            style in arb_style(),
            element in prop::sample::select(&["p", "h2", "span", "td", "th", "ul", "img", "table", "tbody", "iframe", "li", "a"][..]),
        ) {
            let inner = format!(r#"<{element} style="{style}">x</{element}>"#);
            let body = match element {
                "td" | "th" => table(&inner),
                "tbody" => format!(r#"<table style="{TABLE_STYLE}">{inner}</table>"#),
                "li" => format!("<ul>{inner}</ul>"),
                _ => inner,
            };
            let (_, report) = n7(&body);
            for (removal, _) in report.unplanned_removals() {
                let style_rule = matches!(
                    removal.rule,
                    RemovalRule::StyleRejected | RemovalRule::StyleDeclarationRejected
                );
                prop_assert!(
                    !style_rule
                        || removal.property.is_some()
                        || style.contains("var(")
                        || style.contains("url("),
                    "{:?}: {}", body, removal
                );
            }
        }

        // Planned or not, N7 only counts: running it again over its own
        // output records nothing.
        #[test]
        fn n7_over_its_own_output_records_nothing(style in arb_style()) {
            let (out, _) = n7(&table(&format!(r#"<td style="{style}">x</td>"#)));
            let (again, report) = n7(&out);
            prop_assert_eq!(&again, &out);
            prop_assert_eq!(report, NormaliseReport::default());
        }
    }

    /// Legacy markup around what N7 changes: links, tables, images and
    /// embeds with and without their locked markup, re-spelled and
    /// overridden styles, and what no step plans.
    #[rustfmt::skip]
    const MARKUP: &[&str] = &[
        "<p>", "</p>", "<a href=\"/x\">", "<a href=\"/x\" target=\"_self\">", "<a>",
        "<a href=\"/x\" target=\"_blank\" rel=\"noopener noreferrer\">", "</a>", "<table>",
        "<table border=\"1\" cellpadding=\"2\">", "<tr><td>", "<td style=\"color: black\">",
        "</td></tr>", "</table>", "<img src=\"/storage/uploads/posts/1/a.jpg\" alt=\"kép\">",
        "<img src=\"https://example.org/a.png\">", "<img alt=\"x\">",
        "<iframe src=\"https://www.youtube-nocookie.com/embed/AbCdEfGhIjK\" tabindex=\"-1\"></iframe>",
        "<span style=\"color: red !important; color: blue\">", "<span style=\"color: #FF0000\">",
        "</span>", "<ol start=\"1\"><li>", "</li></ol>", "<div class=\"x\">", "</div>",
        "<!-- c -->", "<script>x</script>", "<p style=\"text-align: center; text-align: inherit\">",
        "<ul style=\"list-style-type: disc\"><li>", "</li></ul>", "x", " ",
    ];

    fn arb_body() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(MARKUP), 0..16)
            .prop_map(|pieces| pieces.concat())
    }

    /// Pieces of `style` text, including what makes a declaration
    /// unreadable or ends it early.
    #[rustfmt::skip]
    const STYLE_TEXT: &[&str] = &[
        "color", "text-align", ": ", ":", "red", "center", " !important", "!", "; ", ";", " ",
        "/* c */", "url(x)", "\"s;t\"", "(a;b)", r"\;", "x", "1px", "#ff0000", "rgb(0, 0, 0)",
    ];

    fn arb_style_text() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(STYLE_TEXT), 0..24)
            .prop_map(|pieces| pieces.concat())
    }

    proptest! {
        // data-migration.md §10 V5(b): a row without a rule hit kept its
        // DOM, so N7 changes no body without recording a planned change or
        // an unplanned removal. The input is what a pass gives N7, a body
        // the normaliser's parse and writer left.
        #[test]
        fn n7_records_every_change(body in arb_body(), unclear in 0_usize..3) {
            let input = dom::serialise(&dom::parse(&body));
            let (out, report) = n7_after(&input, unclear);
            if out != input {
                prop_assert!(
                    report.has_hits() || report.has_unplanned_removals(),
                    "{:?} became {:?} with nothing recorded", input, out
                );
            }
        }

        // The pieces read as the reader reads the whole value: the same
        // declarations in the same order, and as many unreadable ones.
        #[test]
        fn the_pieces_read_as_the_whole_style(style in arb_style_text()) {
            let whole = declarations::read(&style);
            let pieces = pieces(&style);
            let read: Vec<_> = pieces
                .iter()
                .filter_map(|piece| match &piece.kind {
                    PieceKind::Read { declaration, .. } => Some(declaration.clone()),
                    _ => None,
                })
                .collect();
            let unreadable = pieces
                .iter()
                .filter(|piece| matches!(piece.kind, PieceKind::Unreadable { .. }))
                .count();
            prop_assert_eq!(read, whole.read);
            prop_assert_eq!(unreadable, whole.unreadable);
            let text: String = pieces.iter().map(|piece| piece.text).collect();
            prop_assert_eq!(text, style);
        }

        // Every counted colour and length is one the canonical style
        // writes: there are never more named colours than hex colours in
        // it, nor more absolute lengths than px lengths.
        #[test]
        fn only_written_parts_are_counted(style in arb_style()) {
            let (out, report) = n7(&table(&format!(r#"<td style="{style}">x</td>"#)));
            let written = out.split("<td").nth(1).unwrap_or_default();
            prop_assert!(report.count(Rule::NamedColourToHex) <= written.matches('#').count(), "{}: {}", style, written);
            prop_assert!(report.count(Rule::LengthToPx) <= written.matches("px").count(), "{}: {}", style, written);
        }

        // An `!important` declaration never changes what N7 records about
        // a style whose properties it does not share with another one.
        #[test]
        fn a_lone_important_mark_changes_nothing(style in arb_style()) {
            let marked = style.split("; ").map(|declaration| {
                if declaration.is_empty() || declaration.contains("!important") {
                    declaration.to_owned()
                } else {
                    format!("{declaration} !important")
                }
            }).collect::<Vec<_>>().join("; ");
            // Every declaration marked: the canonical order and a
            // browser's agree again, so nothing is reviewed for the marks.
            let (_, report) = n7(&table(&format!(r#"<td style="{marked}">x</td>"#)));
            for (removal, _) in report.unplanned_removals() {
                prop_assert!(removal.rule != RemovalRule::ValueCanonicalised, "{}: {}", marked, removal);
            }
        }
    }
}
