//! The per-element CSS grammar of content-v1 as data (editor.md §3.3 and
//! §3.4): the properties each element may carry, how each value is read,
//! the locked declarations, and the rows that are still provisional.
//!
//! The table is the allow-list: a property that is not listed for an
//! element is rejected on it, which is how every property of editor.md §3.4
//! is refused (`font-family`, `padding`, `float`, `position`, `--*`,
//! `-webkit-*`, `mso-*`, ...), together with everything else a browser would
//! accept. The value grammars are deliberately narrower than CSS: they accept
//! what the editor, the paste filter and the import write, and the spellings
//! of the same values that Word, Excel and browsers emit.
//!
//! Like the tables of [`super::super::policy`], this is policy data: a
//! change to it is a policy change (editor.md §3), and the manifest of
//! editor.md §3.9 will export it as written here.

use std::fmt;

use super::declarations::{self, Component};
use super::values::{self, Colour, Hundredths};

/// How the value of one property is read (editor.md §3.3, "Accepted input").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grammar {
    /// One of `values`, matched regardless of ASCII case. `omitted` is the
    /// CSS default, accepted and then left out (`list-style-type: disc`).
    Keyword {
        /// The keywords that are stored.
        values: &'static [&'static str],
        /// The default keyword, which is accepted but not stored.
        omitted: Option<&'static str>,
    },
    /// A length from `min` to `max` px, after conversion to px and rounding.
    Px {
        /// The smallest accepted length.
        min: Hundredths,
        /// The largest accepted length.
        max: Hundredths,
    },
    /// `auto`, or a length as for [`Grammar::Px`].
    AutoOrPx {
        /// The smallest accepted length.
        min: Hundredths,
        /// The largest accepted length.
        max: Hundredths,
    },
    /// A size from `min` to `max` em.
    Em {
        /// The smallest accepted size.
        min: Hundredths,
        /// The largest accepted size.
        max: Hundredths,
    },
    /// A colour; with `opaque_only`, `transparent`, `currentcolor` and every
    /// alpha below 1 are rejected (span `color`).
    Colour {
        /// Whether only opaque colours are accepted.
        opaque_only: bool,
    },
    /// One border side: at most one width (0 to [`BORDER_WIDTH_MAX`], or a
    /// keyword of [`BORDER_WIDTH_KEYWORDS`]), at most one of the
    /// [`BORDER_STYLES`] and at most one colour, in any order, at least one
    /// of them. `none` and `hidden` alone are styles.
    BorderSide,
    /// One to four border widths (`border-width`).
    BorderWidths,
    /// One to four border styles (`border-style`).
    BorderStyles,
    /// One to four colours (`border-color`).
    BorderColours,
    /// Exactly this value (editor.md §3.3, "Locked"), compared after each
    /// component is normalised as the values module reads it, so `0px auto`
    /// matches `0 auto`. It is always written as given here.
    Locked(&'static str),
}

/// When a declaration is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// When the input has it.
    Optional,
    /// Always: a locked declaration that the sanitiser adds when the input
    /// lacks it (editor.md §3.3, "always present").
    Always,
    /// Exactly when the element's stored style has a `width` or a `height`:
    /// a locked declaration that the canonical writer adds then, whether the
    /// input had it or not, and removes otherwise (the cell `box-sizing`,
    /// editor.md §3.3). Without a size it changes nothing on either public
    /// site, so a dropped one is not counted as rejected.
    WhenSized,
}

/// What happens to an accepted declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// It is stored.
    Stored,
    /// It is input only: a shorthand that the canonical writer expands into
    /// the stored properties (editor.md §3.1 rule 1, §3.3 "input only"):
    /// `border`, `border-width`, `border-style` and `border-color` into the
    /// four cell sides, and a single-colour `background` into
    /// `background-color` ([`super::canonical::Cascade`]).
    InputOnly,
}

/// Why a row is provisional: the Windows desktop captures of editor.md
/// §13.2 decide it, and the paste slice that reads them confirms or changes
/// the row (editor.md §16 item 1). Confirming or changing a row is a policy
/// change like any other change to this table: it bumps
/// [`POLICY_VERSION`](super::super::policy::POLICY_VERSION) and ships with a
/// `content_normalize` run (editor.md §3). If the change lands before the
/// first body is stored, that run finds no row to rewrite. The in-place
/// amendment that the maintainer confirmed on 2026-10-25 ("Confirm all four
/// as built", item D87, research: DECISIONS R15) covers only the R11
/// hardening of editor.md §3.5 and §3.6, not these rows (editor.md §3,
/// §3.3).
///
/// Only the rows whose accepted values the captures can change are marked.
/// `border-style` and `border-color` are not: the Excel border-weight map
/// turns weights into widths and leaves the styles and colours as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provisional {
    /// The cell rows that carry a border width (the four sides, `border`
    /// and `border-width`): the accepted range and the widths of `thin`,
    /// `medium` and `thick`, which the Excel border-weight map of editor.md
    /// §5.2 step 8 has to agree with.
    ExcelBorderWeights,
    /// The cell `text-align` row, which Excel's General-alignment rule of
    /// editor.md §5.2 step 8 writes. That rule is about horizontal
    /// alignment only.
    ExcelGeneralAlignment,
    /// The cell `vertical-align` row. Excel writes a vertical alignment on
    /// every cell (editor.md §5.2 step 8), and the paste slice that applies
    /// the General rule confirms the vertical values from the same captures.
    ExcelVerticalAlignment,
}

/// One property allowed on one element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyRule {
    /// The property name, lower-case.
    pub name: &'static str,
    /// How its value is read.
    pub grammar: Grammar,
    /// Whether it is stored or only read.
    pub role: Role,
    /// Whether it is always present.
    pub presence: Presence,
    /// Whether the row waits for the Windows captures, and why.
    pub provisional: Option<Provisional>,
}

impl PropertyRule {
    const fn input_only(self) -> Self {
        Self {
            role: Role::InputOnly,
            ..self
        }
    }

    const fn always(self) -> Self {
        Self {
            presence: Presence::Always,
            ..self
        }
    }

    const fn when_sized(self) -> Self {
        Self {
            presence: Presence::WhenSized,
            ..self
        }
    }

    const fn provisional(self, reason: Provisional) -> Self {
        Self {
            provisional: Some(reason),
            ..self
        }
    }
}

/// The properties one element may carry in its `style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementStyle {
    /// The tag name.
    pub element: &'static str,
    /// Its properties, in the order of editor.md §3.3.
    pub properties: &'static [PropertyRule],
}

impl ElementStyle {
    /// The rule for `name` on this element, if it is allowed.
    ///
    /// - `name`: a property name, lower-case as the declaration reader
    ///   writes it.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&'static PropertyRule> {
        self.properties
            .iter()
            .find(|property| property.name == name)
    }
}

/// The border styles of a cell side (editor.md §3.3).
pub const BORDER_STYLES: &[&str] = &[
    "solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset", "none", "hidden",
];

/// CSS's initial border width, which `border` and `border-<side>` give
/// every side they set without naming a width: the width of the CSS
/// keyword `medium`, 3 px (CSS Backgrounds 3 §3.3 and §3.4), which is what
/// a browser draws for `border: solid`.
///
/// It is a constant of its own, not the `medium` row of
/// [`BORDER_WIDTH_KEYWORDS`]. That row belongs to the provisional Excel
/// border-weight map, which the Windows captures may change (editor.md §3.3,
/// §16 item 1). A browser draws an omitted width the same whatever that map
/// says, so changing the keyword's width and changing this default are two
/// separate decisions. Only the provisional `border` and `border-<side>`
/// rows use this constant. A side that no declaration set gets the page's
/// width of 0 instead ([`Side::UNSET`](super::canonical::Side::UNSET)).
pub const BORDER_WIDTH_INITIAL: Hundredths = Hundredths::whole(3);

/// The border-width keywords and their widths (editor.md §3.1 rule 2).
/// Provisional: [`Provisional::ExcelBorderWeights`].
pub const BORDER_WIDTH_KEYWORDS: &[(&str, Hundredths)] = &[
    ("thin", Hundredths::whole(1)),
    ("medium", Hundredths::whole(3)),
    ("thick", Hundredths::whole(5)),
];

/// The widest border side (editor.md §3.3). Provisional:
/// [`Provisional::ExcelBorderWeights`].
pub const BORDER_WIDTH_MAX: Hundredths = Hundredths::whole(20);

const ALIGNMENTS: &[&str] = &["left", "center", "right", "justify"];
const VERTICAL_ALIGNMENTS: &[&str] = &["top", "middle", "bottom", "baseline"];
const COLOUR: Grammar = Grammar::Colour { opaque_only: false };

const fn rule(name: &'static str, grammar: Grammar) -> PropertyRule {
    PropertyRule {
        name,
        grammar,
        role: Role::Stored,
        presence: Presence::Optional,
        provisional: None,
    }
}

const fn locked(name: &'static str, value: &'static str) -> PropertyRule {
    rule(name, Grammar::Locked(value))
}

const fn keywords(values: &'static [&'static str]) -> Grammar {
    Grammar::Keyword {
        values,
        omitted: None,
    }
}

const fn px(min: u32, max: u32) -> Grammar {
    Grammar::Px {
        min: Hundredths::whole(min),
        max: Hundredths::whole(max),
    }
}

const fn style(element: &'static str, properties: &'static [PropertyRule]) -> ElementStyle {
    ElementStyle {
        element,
        properties,
    }
}

const BORDERS: Provisional = Provisional::ExcelBorderWeights;
const GENERAL: Provisional = Provisional::ExcelGeneralAlignment;
const VERTICAL: Provisional = Provisional::ExcelVerticalAlignment;

const TEXT_ALIGN: &[PropertyRule] = &[rule("text-align", keywords(ALIGNMENTS))];

const CELL: &[PropertyRule] = &[
    rule("border-top", Grammar::BorderSide).provisional(BORDERS),
    rule("border-right", Grammar::BorderSide).provisional(BORDERS),
    rule("border-bottom", Grammar::BorderSide).provisional(BORDERS),
    rule("border-left", Grammar::BorderSide).provisional(BORDERS),
    rule("border", Grammar::BorderSide)
        .input_only()
        .provisional(BORDERS),
    rule("border-width", Grammar::BorderWidths)
        .input_only()
        .provisional(BORDERS),
    rule("border-style", Grammar::BorderStyles).input_only(),
    rule("border-color", Grammar::BorderColours).input_only(),
    rule("vertical-align", keywords(VERTICAL_ALIGNMENTS)).provisional(VERTICAL),
    rule("white-space", keywords(&["nowrap", "normal"])),
    // No `%`: the two public renderers resolve it differently.
    rule("width", px(1, 2000)),
    rule("height", px(1, 2000)),
    // Present exactly when `width` or `height` is: the canonical writer
    // adds and removes it (editor.md §3.3).
    locked("box-sizing", "content-box").when_sized(),
    rule("text-align", keywords(ALIGNMENTS)).provisional(GENERAL),
    rule("background-color", COLOUR),
    // A single-colour `background` only.
    rule("background", COLOUR).input_only(),
];

/// The `style` grammar of every content-v1 element that has a `style`
/// attribute, in the element order of editor.md §3.2.
pub const ELEMENT_STYLES: &[ElementStyle] = &[
    style("p", TEXT_ALIGN),
    style("h1", TEXT_ALIGN),
    style("h2", TEXT_ALIGN),
    style("h3", TEXT_ALIGN),
    style("h4", TEXT_ALIGN),
    style("tbody", &[locked("display", "table-row-group").always()]),
    style(
        "span",
        &[
            rule("color", Grammar::Colour { opaque_only: true }),
            rule("background-color", COLOUR),
            rule("background", COLOUR).input_only(),
            rule(
                "font-size",
                Grammar::Em {
                    min: Hundredths::from_hundredths(50),
                    max: Hundredths::whole(3),
                },
            ),
        ],
    ),
    style(
        "ul",
        &[rule(
            "list-style-type",
            Grammar::Keyword {
                values: &["circle", "square"],
                omitted: Some("disc"),
            },
        )],
    ),
    style(
        "img",
        &[
            rule("width", px(1, 2000)),
            locked("display", "block").always(),
            locked("height", "auto").always(),
            locked("margin", "0 auto").always(),
            locked("max-width", "100%").always(),
        ],
    ),
    style(
        "table",
        &[
            locked("border-collapse", "collapse").always(),
            locked("max-width", "fit-content").always(),
        ],
    ),
    style("td", CELL),
    style("th", CELL),
    style(
        "iframe",
        &[
            locked("width", "100%"),
            rule("max-width", px(200, 1280)),
            rule(
                "height",
                Grammar::AutoOrPx {
                    min: Hundredths::whole(100),
                    max: Hundredths::whole(2000),
                },
            ),
            locked("aspect-ratio", "16 / 9"),
        ],
    ),
];

/// The `style` grammar of `element`, or `None` when the element takes no
/// `style` attribute.
///
/// - `element`: a tag name as the HTML parser reports it.
#[must_use]
pub fn element_style(element: &str) -> Option<&'static ElementStyle> {
    ELEMENT_STYLES.iter().find(|style| style.element == element)
}

/// One border side as it was read: each part only when the input had it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderSide {
    /// The width in px.
    pub width: Option<Hundredths>,
    /// The style keyword.
    pub style: Option<&'static str>,
    /// The colour.
    pub colour: Option<Colour>,
}

/// The parts that are present, as width, style, colour.
impl fmt::Display for BorderSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts = [
            self.width.map(|width| format!("{width}px")),
            self.style.map(str::to_owned),
            self.colour.map(|colour| colour.to_string()),
        ];
        f.write_str(&parts.into_iter().flatten().collect::<Vec<_>>().join(" "))
    }
}

/// An accepted value, typed. Each has one spelling, its `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A keyword of the grammar's list, lower-case.
    Keyword(&'static str),
    /// A length in px.
    Px(Hundredths),
    /// A size in em.
    Em(Hundredths),
    /// A colour.
    Colour(Colour),
    /// One border side.
    BorderSide(BorderSide),
    /// Border widths in px.
    Widths(Vec<Hundredths>),
    /// Border styles.
    Styles(Vec<&'static str>),
    /// Border colours.
    Colours(Vec<Colour>),
    /// A locked value, in its one spelling.
    Locked(&'static str),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn spaced(f: &mut fmt::Formatter<'_>, parts: impl Iterator<Item = String>) -> fmt::Result {
            f.write_str(&parts.collect::<Vec<_>>().join(" "))
        }
        match self {
            Self::Keyword(text) | Self::Locked(text) => f.write_str(text),
            Self::Px(length) => write!(f, "{length}px"),
            Self::Em(size) => write!(f, "{size}em"),
            Self::Colour(colour) => write!(f, "{colour}"),
            Self::BorderSide(side) => write!(f, "{side}"),
            Self::Widths(widths) => spaced(f, widths.iter().map(|width| format!("{width}px"))),
            Self::Styles(styles) => spaced(f, styles.iter().map(|&style| style.to_owned())),
            Self::Colours(colours) => spaced(f, colours.iter().map(Colour::to_string)),
        }
    }
}

/// What a grammar made of a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// The value is accepted, as this typed value.
    Accepted(Value),
    /// The value is the property's default, which is not stored.
    Default,
    /// The value is outside the grammar.
    Rejected,
}

/// Reads `value` with `grammar`.
///
/// - `grammar`: the property's grammar.
/// - `value`: the declaration's components, as the declaration reader read
///   them.
#[must_use]
pub fn parse(grammar: Grammar, value: &[Component]) -> Reading {
    let accepted = match (grammar, value) {
        (
            Grammar::Keyword {
                omitted: Some(default),
                ..
            },
            [Component::Ident(ident)],
        ) if ident == default => return Reading::Default,
        (Grammar::Keyword { values, .. }, [single]) => keyword(single, values).map(Value::Keyword),
        (Grammar::Px { min, max }, [single]) => px_within(single, min, max).map(Value::Px),
        (Grammar::AutoOrPx { min, max }, [single]) => keyword(single, &["auto"])
            .map(Value::Keyword)
            .or_else(|| px_within(single, min, max).map(Value::Px)),
        (Grammar::Em { min, max }, [single]) => values::em(single)
            .filter(|size| (min..=max).contains(size))
            .map(Value::Em),
        (Grammar::Colour { opaque_only }, [single]) => values::colour(single)
            .filter(|colour| !opaque_only || colour.is_opaque())
            .map(Value::Colour),
        (Grammar::BorderSide, _) => border_side(value).map(Value::BorderSide),
        (Grammar::BorderWidths, _) => one_to_four(value, border_width).map(Value::Widths),
        (Grammar::BorderStyles, _) => {
            one_to_four(value, |component| keyword(component, BORDER_STYLES)).map(Value::Styles)
        }
        (Grammar::BorderColours, _) => one_to_four(value, values::colour).map(Value::Colours),
        (Grammar::Locked(locked), _) => {
            matches_locked(locked, value).then_some(Value::Locked(locked))
        }
        _ => None,
    };
    accepted.map_or(Reading::Rejected, Reading::Accepted)
}

/// The keyword of `values` that `component` is.
fn keyword(component: &Component, values: &'static [&'static str]) -> Option<&'static str> {
    match component {
        Component::Ident(ident) => values.iter().copied().find(|value| value == ident),
        _ => None,
    }
}

fn px_within(component: &Component, min: Hundredths, max: Hundredths) -> Option<Hundredths> {
    values::length_px(component).filter(|length| (min..=max).contains(length))
}

/// A border width: a keyword, or a length from 0 to [`BORDER_WIDTH_MAX`].
fn border_width(component: &Component) -> Option<Hundredths> {
    match component {
        Component::Ident(ident) => BORDER_WIDTH_KEYWORDS
            .iter()
            .find(|(name, _)| name == ident)
            .map(|&(_, width)| width),
        other => values::length_px(other).filter(|width| *width <= BORDER_WIDTH_MAX),
    }
}

fn border_side(value: &[Component]) -> Option<BorderSide> {
    if value.is_empty() || value.len() > 3 {
        return None;
    }
    let mut side = BorderSide {
        width: None,
        style: None,
        colour: None,
    };
    // Each component fills the first empty part it can be; a second width,
    // style or colour rejects the side.
    for component in value {
        if side.width.is_none()
            && let Some(width) = border_width(component)
        {
            side.width = Some(width);
        } else if side.style.is_none()
            && let Some(style) = keyword(component, BORDER_STYLES)
        {
            side.style = Some(style);
        } else if side.colour.is_none()
            && let Some(colour) = values::colour(component)
        {
            side.colour = Some(colour);
        } else {
            return None;
        }
    }
    Some(side)
}

fn one_to_four<T>(value: &[Component], read: impl Fn(&Component) -> Option<T>) -> Option<Vec<T>> {
    if (1..=4).contains(&value.len()) {
        value.iter().map(read).collect()
    } else {
        None
    }
}

/// One component reduced to what a locked comparison looks at.
#[derive(Debug, PartialEq, Eq)]
enum Normal<'a> {
    Ident(&'a str),
    Px(Hundredths),
    Percentage(Hundredths),
    Number(Hundredths),
    Slash,
}

fn normal(component: &Component) -> Option<Normal<'_>> {
    match component {
        Component::Ident(ident) => Some(Normal::Ident(ident)),
        Component::Percentage(_) => values::percentage(component).map(Normal::Percentage),
        Component::Slash => Some(Normal::Slash),
        // A unitless zero is a length (`0 auto`); other numbers stay
        // numbers (`16 / 9`).
        Component::Number(_) => values::length_px(component)
            .map(Normal::Px)
            .or_else(|| values::number(component).map(Normal::Number)),
        Component::Dimension { .. } => values::length_px(component).map(Normal::Px),
        Component::Hash(_) | Component::Rgb(_) => None,
    }
}

fn matches_locked(locked: &str, value: &[Component]) -> bool {
    let Some(expected) = declarations::read_value(locked) else {
        return false;
    };
    expected.len() == value.len()
        && expected.iter().zip(value).all(|(expected, actual)| {
            let expected = normal(expected);
            expected.is_some() && expected == normal(actual)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::policy::{ELEMENTS, element_rule};
    use proptest::prelude::*;

    /// What `property` on `element` makes of `value`: its spelling, the
    /// marker `default`, or `None` when it is rejected or unreadable.
    fn check(element: &str, property: &str, value: &str) -> Option<String> {
        let rule = element_style(element)
            .and_then(|style| style.property(property))
            .unwrap_or_else(|| panic!("{element} {property} is not listed"));
        match parse(rule.grammar, &declarations::read_value(value)?) {
            Reading::Accepted(value) => Some(value.to_string()),
            Reading::Default => Some("default".into()),
            Reading::Rejected => None,
        }
    }

    /// The per-property table: element, property, input, and the stored
    /// spelling or `None` for a rejected value.
    #[rustfmt::skip]
    const CASES: &[(&str, &str, &str, Option<&str>)] = &[
        // td and th: border sides.
        ("td", "border-top", "1px solid #000000", Some("1px solid #000000")),
        ("td", "border-top", "solid 1px black", Some("1px solid #000000")),
        ("td", "border-top", "#000 SOLID .75pt", Some("1px solid #000000")),
        ("td", "border-top", "thin solid", Some("1px solid")),
        ("td", "border-top", "medium none", Some("3px none")),
        ("td", "border-top", "thick double windowtext", Some("5px double #000000")),
        ("td", "border-top", ".5pt solid black", Some("0.67px solid #000000")),
        ("td", "border-top", "none", Some("none")),
        ("td", "border-top", "hidden", Some("hidden")),
        ("td", "border-top", "0 none", Some("0px none")),
        ("td", "border-top", "20px dashed red", Some("20px dashed #ff0000")),
        ("td", "border-top", "15pt dotted", Some("20px dotted")),
        ("td", "border-top", "1px solid transparent", Some("1px solid transparent")),
        ("td", "border-top", "1px solid rgba(0, 0, 0, .5)", Some("1px solid #00000080")),
        ("td", "border-top", "1px currentcolor", Some("1px currentcolor")),
        ("td", "border-top", "groove", Some("groove")),
        ("td", "border-top", "21px solid", None),
        ("td", "border-top", "-1px solid", None),
        ("td", "border-top", "1px solid red blue", None),
        ("td", "border-top", "1px 2px", None),
        ("td", "border-top", "solid dashed", None),
        ("td", "border-top", "1px solid red 1px", None),
        ("td", "border-top", "1% solid", None),
        ("td", "border-top", "1em solid", None),
        ("td", "border-top", "wavy", None),
        ("td", "border-top", "1px solid url(x)", None),
        ("td", "border-top", "", None),
        ("td", "border-top", "inherit", None),
        ("th", "border-right", "1px solid #000000", Some("1px solid #000000")),
        ("td", "border-right", "1px 1px", None),
        ("td", "border-bottom", "none", Some("none")),
        ("th", "border-bottom", "wavy", None),
        ("th", "border-left", "outset 2px", Some("2px outset")),
        ("td", "border-left", "21px", None),
        // td and th: the input-only border shorthands.
        ("td", "border", "1px solid #000", Some("1px solid #000000")),
        ("td", "border", "1px 1px", None),
        ("td", "border-width", "1px 2px 3px 4px", Some("1px 2px 3px 4px")),
        ("td", "border-width", "thin medium", Some("1px 3px")),
        ("td", "border-width", "1px 2px 3px 4px 5px", None),
        ("td", "border-width", "21px", None),
        ("td", "border-width", "solid", None),
        ("td", "border-style", "solid none", Some("solid none")),
        ("td", "border-style", "1px", None),
        ("td", "border-style", "solid dashed dotted double groove", None),
        ("td", "border-color", "#000 red", Some("#000000 #ff0000")),
        ("td", "border-color", "1px", None),
        // td and th: the other cell properties.
        ("td", "vertical-align", "top", Some("top")),
        ("td", "vertical-align", "MIDDLE", Some("middle")),
        ("th", "vertical-align", "bottom", Some("bottom")),
        ("td", "vertical-align", "baseline", Some("baseline")),
        ("td", "vertical-align", "center", None),
        ("td", "vertical-align", "sub", None),
        ("td", "vertical-align", "10px", None),
        ("td", "white-space", "nowrap", Some("nowrap")),
        ("td", "white-space", "normal", Some("normal")),
        ("td", "white-space", "pre", None),
        ("td", "white-space", "pre-wrap", None),
        ("td", "width", "128px", Some("128px")),
        ("td", "width", "96pt", Some("128px")),
        ("td", "width", "1px", Some("1px")),
        ("td", "width", "2000px", Some("2000px")),
        ("td", "width", "2000.004px", Some("2000px")),
        ("td", "width", "0.99px", None),
        ("td", "width", "2000.01px", None),
        ("td", "width", "100%", None),
        ("td", "width", "100%px", None),
        ("td", "width", "auto", None),
        ("td", "width", "0", None),
        ("td", "width", "1e30px", None),
        ("th", "height", "20px", Some("20px")),
        ("th", "height", "2001px", None),
        ("td", "box-sizing", "content-box", Some("content-box")),
        ("td", "box-sizing", "CONTENT-BOX", Some("content-box")),
        ("td", "box-sizing", "border-box", None),
        ("td", "text-align", "left", Some("left")),
        ("td", "text-align", "center", Some("center")),
        ("td", "text-align", "right", Some("right")),
        ("th", "text-align", "justify", Some("justify")),
        ("td", "text-align", "start", None),
        ("td", "text-align", "-webkit-center", None),
        ("td", "background-color", "#D9D9D9", Some("#d9d9d9")),
        ("td", "background-color", "silver", Some("#c0c0c0")),
        ("td", "background-color", "transparent", Some("transparent")),
        ("td", "background-color", "#d9d9d9 #000", None),
        ("td", "background", "#d9d9d9", Some("#d9d9d9")),
        ("td", "background", "#d9d9d9 no-repeat", None),
        ("td", "background", "none", None),
        // table and tbody: locked.
        ("table", "border-collapse", "collapse", Some("collapse")),
        ("table", "border-collapse", "separate", None),
        ("table", "max-width", "fit-content", Some("fit-content")),
        ("table", "max-width", "100%", None),
        ("table", "max-width", "700px", None),
        ("tbody", "display", "table-row-group", Some("table-row-group")),
        ("tbody", "display", "table", None),
        ("tbody", "display", "none", None),
        // Blocks.
        ("p", "text-align", "center", Some("center")),
        ("p", "text-align", "middle", None),
        ("h1", "text-align", "right", Some("right")),
        ("h2", "text-align", "justify", Some("justify")),
        ("h3", "text-align", "left", Some("left")),
        ("h4", "text-align", "center", Some("center")),
        ("ul", "list-style-type", "circle", Some("circle")),
        ("ul", "list-style-type", "SQUARE", Some("square")),
        ("ul", "list-style-type", "disc", Some("default")),
        ("ul", "list-style-type", "decimal", None),
        ("ul", "list-style-type", "none", None),
        // span.
        ("span", "color", "#C00000", Some("#c00000")),
        ("span", "color", "red", Some("#ff0000")),
        ("span", "color", "rgb(192, 0, 0)", Some("#c00000")),
        ("span", "color", "windowtext", Some("#000000")),
        ("span", "color", "#c00000ff", Some("#c00000")),
        ("span", "color", "transparent", None),
        ("span", "color", "currentcolor", None),
        ("span", "color", "rgba(192, 0, 0, 0.5)", None),
        ("span", "color", "#c0000080", None),
        ("span", "color", "#c008", None),
        ("span", "background-color", "yellow", Some("#ffff00")),
        ("span", "background-color", "transparent", Some("transparent")),
        ("span", "background-color", "rgba(255, 255, 0, .5)", Some("#ffff0080")),
        ("span", "background-color", "1px", None),
        ("span", "background", "yellow", Some("#ffff00")),
        ("span", "background", "yellow no-repeat", None),
        ("span", "font-size", "1.25em", Some("1.25em")),
        ("span", "font-size", "0.5em", Some("0.5em")),
        ("span", "font-size", "3EM", Some("3em")),
        ("span", "font-size", "0.49em", None),
        ("span", "font-size", "3.01em", None),
        ("span", "font-size", "12pt", None),
        ("span", "font-size", "120%", None),
        ("span", "font-size", "large", None),
        ("span", "font-size", "0", None),
        // img.
        ("img", "width", "350px", Some("350px")),
        ("img", "width", "2000px", Some("2000px")),
        ("img", "width", "2001px", None),
        ("img", "width", "50%", None),
        ("img", "display", "block", Some("block")),
        ("img", "display", "inline", None),
        ("img", "height", "auto", Some("auto")),
        ("img", "height", "495px", None),
        ("img", "margin", "0 auto", Some("0 auto")),
        ("img", "margin", "0px auto", Some("0 auto")),
        ("img", "margin", "0PT AUTO", Some("0 auto")),
        ("img", "margin", "auto", None),
        ("img", "margin", "0 auto 0", None),
        ("img", "margin", "10px auto", None),
        ("img", "margin", "auto 0", None),
        ("img", "max-width", "100%", Some("100%")),
        ("img", "max-width", "100.00%", Some("100%")),
        ("img", "max-width", "700px", None),
        ("img", "max-width", "50%", None),
        // iframe.
        ("iframe", "width", "100%", Some("100%")),
        ("iframe", "width", "640px", None),
        ("iframe", "max-width", "640px", Some("640px")),
        ("iframe", "max-width", "200px", Some("200px")),
        ("iframe", "max-width", "1280px", Some("1280px")),
        ("iframe", "max-width", "199px", None),
        ("iframe", "max-width", "1281px", None),
        ("iframe", "max-width", "100%", None),
        ("iframe", "height", "auto", Some("auto")),
        ("iframe", "height", "608px", Some("608px")),
        ("iframe", "height", "100px", Some("100px")),
        ("iframe", "height", "2000px", Some("2000px")),
        ("iframe", "height", "99px", None),
        ("iframe", "height", "2001px", None),
        ("iframe", "height", "50%", None),
        ("iframe", "aspect-ratio", "16 / 9", Some("16 / 9")),
        ("iframe", "aspect-ratio", "16/9", Some("16 / 9")),
        ("iframe", "aspect-ratio", "4 / 3", None),
        ("iframe", "aspect-ratio", "16", None),
        ("iframe", "aspect-ratio", "1.7778", None),
        ("iframe", "aspect-ratio", "16 / 9 / 1", None),
    ];

    #[test]
    fn every_property_reads_its_table() {
        for &(element, property, input, expected) in CASES {
            assert_eq!(
                check(element, property, input).as_deref(),
                expected,
                "{element} {{ {property}: {input} }}"
            );
        }
    }

    #[test]
    fn every_listed_property_has_cases() {
        for style in ELEMENT_STYLES {
            for property in style.properties {
                let tested = |accepted: bool| {
                    CASES.iter().any(|(element, name, _, expected)| {
                        // td and th share one list, as do p and h1-h4.
                        let same = *element == style.element
                            || (style.properties == CELL && ["td", "th"].contains(element))
                            || (style.properties == TEXT_ALIGN
                                && ["p", "h1", "h2", "h3", "h4"].contains(element));
                        same && *name == property.name && expected.is_some() == accepted
                    })
                };
                assert!(
                    tested(true),
                    "{} {}: no accepted case",
                    style.element,
                    property.name
                );
                assert!(
                    tested(false),
                    "{} {}: no rejected case",
                    style.element,
                    property.name
                );
            }
        }
    }

    #[test]
    fn the_table_matches_the_elements_with_a_style_attribute() {
        let with_style: Vec<&str> = ELEMENTS
            .iter()
            .filter(|element| element.attribute("style").is_some())
            .map(|element| element.name)
            .collect();
        let listed: Vec<&str> = ELEMENT_STYLES.iter().map(|style| style.element).collect();
        assert_eq!(listed, with_style);
        for style in ELEMENT_STYLES {
            assert!(element_rule(style.element).is_some());
            for property in style.properties {
                assert_eq!(property.name, property.name.to_ascii_lowercase());
                assert_eq!(
                    style.property(property.name),
                    Some(property),
                    "{} is listed twice",
                    property.name
                );
                assert!(
                    property.presence == Presence::Optional
                        || matches!(property.grammar, Grammar::Locked(_)),
                    "only a locked declaration can always be present: {}",
                    property.name
                );
            }
        }
        assert_eq!(element_style("div"), None);
        assert_eq!(element_style("li"), None);
        assert_eq!(
            element_style("TD"),
            None,
            "the parser lower-cases tag names"
        );
    }

    #[test]
    fn every_locked_value_is_read_back_as_itself() {
        for style in ELEMENT_STYLES {
            for property in style.properties {
                if let Grammar::Locked(value) = property.grammar {
                    let components =
                        declarations::read_value(value).expect("a readable locked value");
                    assert_eq!(
                        parse(property.grammar, &components),
                        Reading::Accepted(Value::Locked(value))
                    );
                    assert_eq!(Value::Locked(value).to_string(), value);
                }
            }
        }
    }

    #[test]
    fn the_provisional_and_input_only_rows_are_exactly_these() {
        let rows = |pick: fn(&PropertyRule) -> bool| -> Vec<String> {
            ELEMENT_STYLES
                .iter()
                .flat_map(|style| {
                    style
                        .properties
                        .iter()
                        .filter(|p| pick(p))
                        .map(move |p| format!("{} {}", style.element, p.name))
                })
                .collect()
        };
        let cell = |names: &[&str]| -> Vec<String> {
            ["td", "th"]
                .iter()
                .flat_map(|cell| names.iter().map(move |name| format!("{cell} {name}")))
                .collect()
        };
        assert_eq!(
            rows(|p| p.provisional == Some(Provisional::ExcelBorderWeights)),
            cell(&[
                "border-top",
                "border-right",
                "border-bottom",
                "border-left",
                "border",
                "border-width"
            ])
        );
        assert_eq!(
            rows(|p| p.provisional == Some(Provisional::ExcelGeneralAlignment)),
            cell(&["text-align"])
        );
        assert_eq!(
            rows(|p| p.provisional == Some(Provisional::ExcelVerticalAlignment)),
            cell(&["vertical-align"])
        );
        // Every provisional row is one of the three lists above, so editor.md
        // §3.3 can name them exactly.
        assert_eq!(rows(|p| p.provisional.is_some()).len(), 2 * (6 + 1 + 1));
        let mut input_only = cell(&[
            "border",
            "border-width",
            "border-style",
            "border-color",
            "background",
        ]);
        input_only.insert(0, "span background".into());
        assert_eq!(rows(|p| p.role == Role::InputOnly), input_only);
        assert_eq!(
            rows(|p| p.presence == Presence::Always),
            [
                "tbody display",
                "img display",
                "img height",
                "img margin",
                "img max-width",
                "table border-collapse",
                "table max-width"
            ]
        );
        assert_eq!(
            rows(|p| p.presence == Presence::WhenSized),
            cell(&["box-sizing"])
        );
        // The writer looks for these two names to decide a size-dependent
        // declaration; every element with one stores both.
        for style in ELEMENT_STYLES {
            if style
                .properties
                .iter()
                .any(|p| p.presence == Presence::WhenSized)
            {
                assert!(style.property("width").is_some() && style.property("height").is_some());
            }
        }
    }

    /// The omitted width is CSS's 3 px, pinned on its own: a change to the
    /// provisional keyword map must not move it silently (see
    /// [`BORDER_WIDTH_INITIAL`]).
    #[test]
    fn the_initial_border_width_is_css_medium() {
        assert_eq!(BORDER_WIDTH_INITIAL, Hundredths::whole(3));
        assert_eq!(BORDER_WIDTH_INITIAL.to_string(), "3");
    }

    #[test]
    fn values_are_written_in_one_spelling() {
        let side = BorderSide {
            width: Some(Hundredths::whole(1)),
            style: None,
            colour: Some(Colour::opaque(0, 0, 0)),
        };
        assert_eq!(side.to_string(), "1px #000000");
        assert_eq!(
            Value::Widths(vec![Hundredths::whole(1), Hundredths::from_hundredths(67)]).to_string(),
            "1px 0.67px"
        );
        assert_eq!(
            Value::Styles(vec!["solid", "none"]).to_string(),
            "solid none"
        );
        assert_eq!(
            Value::Colours(vec![Colour::Transparent, Colour::opaque(255, 0, 0)]).to_string(),
            "transparent #ff0000"
        );
        assert_eq!(
            Value::Em(Hundredths::from_hundredths(125)).to_string(),
            "1.25em"
        );
        assert_eq!(Value::Keyword("nowrap").to_string(), "nowrap");
        assert_eq!(Value::Px(Hundredths::whole(2000)).to_string(), "2000px");
        assert_eq!(
            Value::Colour(Colour::CurrentColor).to_string(),
            "currentcolor"
        );
    }

    /// Components of every kind the grammars read, accepted and not.
    #[rustfmt::skip]
    const POOL: &[&str] = &[
        "1px", "0", ".5pt", "15.0pt", "20px", "21px", "-1px", "1em", "1.25em", "3em", "100%", "50%",
        "16", "/", "9", "solid", "SOLID", "none", "hidden", "double", "thin", "medium", "thick", "auto",
        "block", "collapse", "fit-content", "table-row-group", "content-box", "center", "middle",
        "nowrap", "circle", "disc", "red", "#c00000", "#C008", "transparent", "currentcolor",
        "windowtext", "rgb(1, 2, 3)", "rgba(1, 2, 3, .5)", "rgb(1 2 3 / 50%)",
    ];

    fn every_grammar() -> Vec<Grammar> {
        let mut grammars: Vec<Grammar> = ELEMENT_STYLES
            .iter()
            .flat_map(|style| style.properties.iter().map(|p| p.grammar))
            .collect();
        grammars.dedup();
        grammars
    }

    proptest! {
        #[test]
        fn accepted_values_are_read_back_unchanged(parts in prop::collection::vec(prop::sample::select(POOL), 0..5)) {
            let input = parts.join(" ");
            let components = declarations::read_value(&input).unwrap();
            for grammar in every_grammar() {
                if let Reading::Accepted(value) = parse(grammar, &components) {
                    let written = value.to_string();
                    let again = declarations::read_value(&written).unwrap();
                    prop_assert_eq!(parse(grammar, &again), Reading::Accepted(value), "{} -> {}", input, written);
                }
            }
        }

        #[test]
        fn parsing_never_panics(value in any::<String>()) {
            if let Some(components) = declarations::read_value(&value) {
                for grammar in every_grammar() {
                    let _ = parse(grammar, &components);
                }
            }
        }

        #[test]
        fn px_bounds_are_exact(hundredths in 0u32..300_000) {
            let length = Hundredths::from_hundredths(hundredths);
            let accepted = check("td", "width", &format!("{length}px"));
            prop_assert_eq!(accepted.is_some(), (100..=200_000).contains(&hundredths));
        }

        #[test]
        fn em_bounds_are_exact(hundredths in 0u32..1000) {
            let size = Hundredths::from_hundredths(hundredths);
            let accepted = check("span", "font-size", &format!("{size}em"));
            prop_assert_eq!(accepted, (50..=300).contains(&hundredths).then(|| format!("{size}em")));
        }

        #[test]
        fn border_side_parts_may_come_in_any_order(
            width in 0u32..=2000,
            style in prop::sample::select(BORDER_STYLES),
            colour in prop::sample::select(&["red", "#123456", "transparent", "rgb(1, 2, 3)"][..]),
            order in Just(vec![0usize, 1, 2]).prop_shuffle(),
        ) {
            let parts = [format!("{}px", Hundredths::from_hundredths(width)), style.to_owned(), colour.to_owned()];
            let canonical = check("td", "border-top", &parts.join(" "));
            prop_assert!(canonical.is_some());
            let shuffled = order.iter().map(|&index| parts[index].clone()).collect::<Vec<_>>().join(" ");
            prop_assert_eq!(check("td", "border-left", &shuffled), canonical);
        }

        #[test]
        fn keywords_match_in_any_ascii_case(index in 0usize..4, upper in prop::collection::vec(any::<bool>(), 8)) {
            let keyword = VERTICAL_ALIGNMENTS[index];
            let mixed: String = keyword.chars().zip(upper.iter().cycle()).map(|(c, &up)| if up { c.to_ascii_uppercase() } else { c }).collect();
            let read = check("td", "vertical-align", &mixed);
            prop_assert_eq!(read.as_deref(), Some(keyword));
        }

        #[test]
        fn property_finds_only_listed_names(name in "[a-z-]{1,20}") {
            for style in ELEMENT_STYLES {
                prop_assert_eq!(style.property(&name).is_some(), style.properties.iter().any(|p| p.name == name));
            }
            prop_assert_eq!(element_style(&name).is_some(), ELEMENT_STYLES.iter().any(|s| s.element == name));
        }
    }
}
