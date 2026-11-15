//! Whether a declaration that N7's sanitiser drops was invalid CSS anyway,
//! so that a browser ignored it too (data-migration.md §7.1 N7: "invalid
//! values such as `100%px` are dropped").
//!
//! N7 plans the drop of an invalid value, because it changes nothing a
//! reader saw. It does not plan the drop of a valid value that content-v1
//! does not keep (`width: 50%` on a cell, `background: none`, `inherit`, a
//! border wider than 20 px): a browser applied that one, so dropping it can
//! change what the page shows, and the body goes to the editors for review
//! (`N7-accept`). The sanitiser's own report names both
//! `style_declaration_rejected`, so this module keeps them apart (the
//! register's question D145 asked for exactly that).
//!
//! **Only what is certainly invalid.** [`is_invalid`] answers `true` only
//! for a value that no browser accepts for the property: one that has the
//! wrong number of parts, a part of a type the property never takes (a
//! unitless number other than 0 for a length, a percentage for a border
//! width, a negative width), a unit CSS does not define, or a keyword the
//! property does not know. Anything it is not sure about counts as valid,
//! and is reviewed: a keyword with a vendor prefix (`-webkit-center`), a
//! value of a property whose full grammar this module does not model
//! (`display`), and the multi-part values that newer CSS gives some
//! properties (`vertical-align`, `white-space`). A wrong "valid" costs an
//! editor's look at the row; a wrong "invalid" would change a page without
//! one.
//!
//! The value comes as the sanitiser's declaration reader read it
//! ([`declarations::read`]): its identifiers and units are lower-case, and a
//! value with a function other than `rgb()` and `rgba()`, a string, a
//! comment or a bad token never gets here, because the reader cannot read
//! that declaration at all (N7 does not plan such a drop).
//!
//! [`declarations::read`]: crate::domain::rich_text::css::declarations::read

use crate::domain::rich_text::css::declarations::Component;
use crate::domain::rich_text::css::values;

/// CSS's wide keywords (CSS Cascade 5 §7.3), valid alone on every property
/// and nowhere else.
const CSS_WIDE: &[&str] = &["inherit", "initial", "unset", "revert", "revert-layer"];

/// Every length unit of CSS Values 4 and 5 and CSS Containment 3: font
/// relative, viewport (with the small, large and dynamic forms), container
/// and absolute units. A dimension in any other unit is no length.
#[rustfmt::skip]
const LENGTH_UNITS: &[&str] = &[
    "em", "rem", "ex", "rex", "cap", "rcap", "ch", "rch", "ic", "ric", "lh", "rlh",
    "vw", "vh", "vi", "vb", "vmin", "vmax",
    "svw", "svh", "svi", "svb", "svmin", "svmax",
    "lvw", "lvh", "lvi", "lvb", "lvmin", "lvmax",
    "dvw", "dvh", "dvi", "dvb", "dvmin", "dvmax",
    "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax",
    "px", "cm", "mm", "q", "in", "pt", "pc",
];

/// The system colours of CSS Color 4 §6.2, current and deprecated, which a
/// browser accepts wherever a colour goes. `values::colour` reads only
/// `windowtext`, which Office writes for black.
#[rustfmt::skip]
const SYSTEM_COLOURS: &[&str] = &[
    "accentcolor", "accentcolortext", "activetext", "buttonborder", "buttonface", "buttontext",
    "canvas", "canvastext", "field", "fieldtext", "graytext", "highlight", "highlighttext",
    "linktext", "mark", "marktext", "selecteditem", "selecteditemtext", "visitedtext",
    "activeborder", "activecaption", "appworkspace", "background", "buttonhighlight",
    "buttonshadow", "captiontext", "inactiveborder", "inactivecaption", "inactivecaptiontext",
    "infobackground", "infotext", "menu", "menutext", "scrollbar", "threeddarkshadow",
    "threedface", "threedhighlight", "threedlightshadow", "threedshadow", "window",
    "windowframe", "windowtext",
];

/// The line styles of a border side (CSS Backgrounds 3 §3.2).
const LINE_STYLES: &[&str] = &[
    "none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset", "outset",
];

/// The line-width keywords of a border side (CSS Backgrounds 3 §3.3).
const LINE_WIDTHS: &[&str] = &["thin", "medium", "thick"];

/// The keywords of `text-align` (CSS Text 3 §6.1, CSS Text 4).
const TEXT_ALIGN: &[&str] = &[
    "start",
    "end",
    "left",
    "right",
    "center",
    "justify",
    "match-parent",
    "justify-all",
];

/// The keywords of `vertical-align` (CSS 2 §10.8.1).
const VERTICAL_ALIGN: &[&str] = &[
    "baseline",
    "sub",
    "super",
    "text-top",
    "text-bottom",
    "middle",
    "top",
    "bottom",
];

/// The keywords of `white-space` and of the longhands it stands for in CSS
/// Text 4 (`white-space-collapse`, `text-wrap-mode`, `white-space-trim`).
#[rustfmt::skip]
const WHITE_SPACE: &[&str] = &[
    "normal", "pre", "nowrap", "pre-wrap", "pre-line", "break-spaces", "collapse", "discard",
    "preserve", "preserve-breaks", "preserve-spaces", "wrap", "none", "discard-before",
    "discard-after", "discard-inner",
];

/// The keywords of `width` and `height` (CSS Sizing 3 and 4).
const SIZE_KEYWORDS: &[&str] = &[
    "auto",
    "min-content",
    "max-content",
    "fit-content",
    "stretch",
];

/// The keywords of `max-width` (CSS Sizing 3 and 4).
const MAX_SIZE_KEYWORDS: &[&str] = &[
    "none",
    "min-content",
    "max-content",
    "fit-content",
    "stretch",
];

/// The keywords of `font-size` (CSS Fonts 4 §2.5).
#[rustfmt::skip]
const FONT_SIZE_KEYWORDS: &[&str] = &[
    "xx-small", "x-small", "small", "medium", "large", "x-large", "xx-large", "xxx-large",
    "larger", "smaller", "math",
];

/// The keywords that a part of the `background` shorthand may be, besides
/// a colour and a length or percentage: `none` (no image), positions,
/// repeat styles, attachments, boxes and sizes (CSS Backgrounds 3 §2.11).
#[rustfmt::skip]
const BACKGROUND_KEYWORDS: &[&str] = &[
    "none", "left", "right", "top", "bottom", "center", "repeat", "repeat-x", "repeat-y",
    "no-repeat", "space", "round", "scroll", "fixed", "local", "border-box", "padding-box",
    "content-box", "text", "auto", "cover", "contain",
];

/// Whether the declaration `property: value` is certainly invalid CSS, so
/// that a browser ignores it (see the module documentation). `false` means
/// "possibly valid": the property's grammar is not modelled here, or the
/// value may be one a browser applies.
///
/// - `property`: the property name, lower-case, as the declaration reader
///   writes it.
/// - `value`: its components, as the reader read them.
#[must_use]
pub(super) fn is_invalid(property: &str, value: &[Component]) -> bool {
    // A declaration with nothing after its colon is a parse error.
    let [first, ..] = value else {
        return true;
    };
    if value.len() == 1 && is_ident_in(first, CSS_WIDE) {
        return false;
    }
    match property {
        "text-align" => single(value, |c| keyword(c, TEXT_ALIGN)),
        "vertical-align" => {
            // CSS Inline 3 makes `vertical-align` a shorthand of more parts.
            value.len() == 1 && !(keyword(first, VERTICAL_ALIGN) || length(first, Sign::Any, true))
        }
        "white-space" => {
            // CSS Text 4 makes `white-space` a shorthand: a value of several
            // keywords may be valid, one with another part never is.
            value.iter().any(|c| !matches!(c, Component::Ident(_)))
                || (value.len() == 1 && !keyword(first, WHITE_SPACE))
        }
        "width" | "height" => single(value, |c| {
            keyword(c, SIZE_KEYWORDS) || length(c, Sign::NonNegative, true)
        }),
        "max-width" => single(value, |c| {
            keyword(c, MAX_SIZE_KEYWORDS) || length(c, Sign::NonNegative, true)
        }),
        "font-size" => single(value, |c| {
            keyword(c, FONT_SIZE_KEYWORDS) || length(c, Sign::NonNegative, true)
        }),
        "color" | "background-color" => single(value, colour),
        "background" => !value.iter().all(|c| {
            keyword(c, BACKGROUND_KEYWORDS)
                || colour(c)
                || length(c, Sign::Any, true)
                || matches!(c, Component::Slash)
        }),
        // Any identifier names a counter style; a missing one falls back
        // to `decimal`, but the declaration is valid.
        "list-style-type" => single(value, |c| matches!(c, Component::Ident(_))),
        "box-sizing" => single(value, |c| keyword(c, &["content-box", "border-box"])),
        "border-collapse" => single(value, |c| keyword(c, &["collapse", "separate"])),
        "margin" => {
            value.len() > 4
                || !value
                    .iter()
                    .all(|c| keyword(c, &["auto"]) || length(c, Sign::Any, true))
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            border_side_is_invalid(value)
        }
        "border-width" => value.len() > 4 || !value.iter().all(line_width),
        "border-style" => value.len() > 4 || !value.iter().all(|c| keyword(c, LINE_STYLES)),
        "border-color" => value.len() > 4 || !value.iter().all(colour),
        // `display`, `aspect-ratio` and every property not listed: not
        // modelled, so never certainly invalid.
        _ => false,
    }
}

/// Whether a value that must be one part is invalid: it has more than one,
/// or its one part is not `accepted`.
fn single(value: &[Component], accepted: impl Fn(&Component) -> bool) -> bool {
    match value {
        [only] => !accepted(only),
        _ => true,
    }
}

/// Which signs a length may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sign {
    /// Zero or more: sizes and border widths.
    NonNegative,
    /// Any: margins, positions, `vertical-align`.
    Any,
}

/// Whether `component` may be a length (or, with `percentage`, a
/// percentage): a dimension in a CSS length unit, or a unitless zero. A
/// unitless number other than zero is a length only in quirks mode, and
/// both public sites render in standards mode.
fn length(component: &Component, sign: Sign, percentage: bool) -> bool {
    let signed = |value: f64| sign == Sign::Any || value >= 0.0;
    match component {
        Component::Number(value) => *value == 0.0,
        Component::Dimension { value, unit } => {
            LENGTH_UNITS.contains(&unit.as_str()) && signed(*value)
        }
        Component::Percentage(value) => percentage && signed(*value),
        _ => false,
    }
}

/// Whether `component` may be a border line width: a non-negative length
/// or one of the three keywords.
fn line_width(component: &Component) -> bool {
    keyword(component, LINE_WIDTHS) || length(component, Sign::NonNegative, false)
}

/// Whether `component` is one of `keywords`, or an identifier with a
/// vendor prefix, which this module cannot judge.
fn keyword(component: &Component, keywords: &[&str]) -> bool {
    is_ident_in(component, keywords) || vendor_prefixed(component)
}

/// Whether `component` is one of `keywords`.
fn is_ident_in(component: &Component, keywords: &[&str]) -> bool {
    matches!(component, Component::Ident(ident) if keywords.contains(&ident.as_str()))
}

/// Whether `component` is an identifier with a vendor prefix (`-webkit-`,
/// `-moz-`, ...), which a browser may know.
fn vendor_prefixed(component: &Component) -> bool {
    matches!(component, Component::Ident(ident) if ident.starts_with('-'))
}

/// Whether `component` may be a colour: everything the sanitiser reads as
/// one, a system colour, an `rgb()` or `rgba()` call whose numbers a
/// browser would clamp rather than refuse, or a vendor-prefixed name.
fn colour(component: &Component) -> bool {
    values::colour(component).is_some()
        || is_ident_in(component, SYSTEM_COLOURS)
        || vendor_prefixed(component)
        || matches!(component, Component::Rgb(_))
}

/// Whether the value of a border side (`border`, `border-top`, ...) is
/// invalid: CSS takes at most one line width, one line style and one
/// colour, in any order (CSS Backgrounds 3 §3.4). A vendor-prefixed name
/// may be any of the three, so a value with one is not judged.
fn border_side_is_invalid(value: &[Component]) -> bool {
    if value.len() > 3 {
        return true;
    }
    let (mut widths, mut styles, mut colours) = (0, 0, 0);
    for component in value {
        if vendor_prefixed(component) {
            return false;
        }
        if line_width(component) {
            widths += 1;
        } else if is_ident_in(component, LINE_STYLES) {
            styles += 1;
        } else if colour(component) {
            colours += 1;
        } else {
            return true;
        }
    }
    widths > 1 || styles > 1 || colours > 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::css::declarations::read_value;
    use proptest::prelude::*;

    /// Whether `property: value` is certainly invalid, from the value's text.
    fn invalid(property: &str, value: &str) -> bool {
        let components = read_value(value).unwrap_or_else(|| panic!("{value:?} is unreadable"));
        is_invalid(property, &components)
    }

    #[test]
    fn the_legacy_invalid_values_are_invalid() {
        // The corpus's 123 `100%px` widths (data-migration.md §7.1 N7).
        assert!(invalid("width", "100%px"));
        assert!(invalid("width", "100 px"));
        assert!(
            invalid("height", "20"),
            "a unitless length is quirks mode only"
        );
        assert!(invalid("width", "-5px"), "a size is never negative");
        assert!(invalid("width", "5pxx"), "no such unit");
        assert!(invalid("width", "#fff"));
        assert!(invalid("width", "nope"));
        assert!(is_invalid("width", &[]), "an empty value");
    }

    #[test]
    fn valid_values_that_content_v1_does_not_keep_are_not_invalid() {
        for (property, value) in [
            ("width", "50%"),
            ("width", "auto"),
            ("width", "3000px"),
            ("width", "0"),
            ("width", "10em"),
            ("width", "12cqw"),
            ("width", "inherit"),
            ("height", "initial"),
            ("max-width", "none"),
            ("text-align", "start"),
            ("text-align", "inherit"),
            ("text-align", "-webkit-center"),
            ("vertical-align", "sub"),
            ("vertical-align", "-2px"),
            ("vertical-align", "10%"),
            ("vertical-align", "baseline 10px"),
            ("white-space", "pre"),
            ("white-space", "collapse wrap"),
            ("font-size", "12px"),
            ("font-size", "larger"),
            ("font-size", "150%"),
            ("color", "transparent"),
            ("color", "rgba(0, 0, 0, 0.5)"),
            ("color", "canvastext"),
            ("color", "rgb(300, 0, 0)"),
            ("background-color", "buttonface"),
            ("background", "none"),
            ("background", "#fff no-repeat left top"),
            ("background", "0 0 / cover"),
            ("list-style-type", "lower-roman"),
            ("list-style-type", "my-counter"),
            ("box-sizing", "border-box"),
            ("border-collapse", "separate"),
            ("margin", "-10px auto"),
            ("border-top", "25px solid"),
            ("border-top", "1em dashed red"),
            ("border", "medium none currentcolor"),
            ("border-width", "thin 2px 0 thick"),
            ("border-style", "solid none"),
            ("border-color", "red green blue windowtext"),
            ("display", "block flow"),
            ("aspect-ratio", "4 / 3"),
            ("font-family", "x y z"),
        ] {
            assert!(!invalid(property, value), "{property}: {value}");
        }
    }

    #[test]
    fn invalid_values_of_each_modelled_property_are_invalid() {
        for (property, value) in [
            ("text-align", "middle"),
            ("text-align", "center left"),
            ("text-align", "5px"),
            ("vertical-align", "centre"),
            ("vertical-align", "5"),
            ("white-space", "nowraps"),
            ("white-space", "pre 5px"),
            ("max-width", "-1px"),
            ("max-width", "auto"),
            ("font-size", "big"),
            ("font-size", "-1em"),
            ("color", "nope"),
            ("color", "#ff"),
            ("color", "red blue"),
            ("color", "5px"),
            ("background-color", "none"),
            ("background", "nope"),
            ("background", "5"),
            ("list-style-type", "5"),
            ("list-style-type", "disc circle"),
            ("box-sizing", "padding-box"),
            ("border-collapse", "none"),
            ("margin", "1px 2px 3px 4px 5px"),
            ("margin", "nope"),
            ("border-top", "1px solid red blue"),
            ("border-top", "1px 2px"),
            ("border-top", "solid dashed"),
            ("border-top", "10%"),
            ("border-top", "1px solid red 2px"),
            ("border", "nope"),
            ("border-width", "1px 2px 3px 4px 5px"),
            ("border-width", "10%"),
            ("border-width", "-1px"),
            ("border-style", "wavy"),
            ("border-color", "red nope"),
            ("width", "inherit 5px"),
        ] {
            assert!(invalid(property, value), "{property}: {value}");
        }
    }

    #[test]
    fn vendor_prefixes_are_never_judged_invalid() {
        assert!(!invalid("text-align", "-moz-center"));
        assert!(!invalid("border-top", "-moz-something 1px"));
        assert!(!invalid("color", "-moz-buttondefault"));
        assert!(!invalid("width", "-webkit-fill-available"));
    }

    /// Any one component, of every kind the reader produces.
    fn arb_component() -> impl Strategy<Value = Component> {
        prop_oneof![
            "[a-z-]{1,12}".prop_map(Component::Ident),
            (-1000.0_f64..1000.0).prop_map(Component::Number),
            ((-1000.0_f64..1000.0), "[a-z]{1,5}")
                .prop_map(|(value, unit)| Component::Dimension { value, unit }),
            (-200.0_f64..200.0).prop_map(Component::Percentage),
            "[0-9a-f]{1,8}".prop_map(Component::Hash),
            Just(Component::Slash),
        ]
    }

    proptest! {
        // A CSS-wide keyword alone is valid on every property.
        #[test]
        fn a_wide_keyword_alone_is_never_invalid(
            property in "[a-z-]{1,20}",
            keyword in prop::sample::select(CSS_WIDE),
        ) {
            prop_assert!(!is_invalid(&property, &[Component::Ident(keyword.to_owned())]));
        }

        // A property this module does not model is never judged.
        #[test]
        fn an_unmodelled_property_is_never_invalid(
            property in "x-[a-z-]{1,20}",
            value in prop::collection::vec(arb_component(), 1..6),
        ) {
            prop_assert!(!is_invalid(&property, &value));
        }

        // A value that a single-part property cannot hold is invalid,
        // whatever its parts.
        #[test]
        fn single_part_properties_refuse_several_parts(
            property in prop::sample::select(&[
                "text-align", "width", "height", "max-width", "font-size", "color",
                "background-color", "list-style-type", "box-sizing", "border-collapse",
            ][..]),
            value in prop::collection::vec(arb_component(), 2..6),
        ) {
            prop_assert!(is_invalid(property, &value));
        }

        // Every length the sanitiser itself reads as px is valid as a
        // width, so N7 never plans the drop of a width it could have kept.
        #[test]
        fn every_length_the_sanitiser_reads_is_valid(
            value in 0.0_f64..5000.0,
            unit in prop::sample::select(&["px", "pt", "pc", "in", "cm", "mm", "q"][..]),
        ) {
            let component = Component::Dimension { value, unit: unit.to_owned() };
            prop_assert!(values::length_px(&component).is_some());
            prop_assert!(!is_invalid("width", &[component]));
        }
    }
}
