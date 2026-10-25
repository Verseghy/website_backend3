//! The sanitiser's own CSS grammar for `style` attributes (ADR 0013 §5,
//! editor.md §3.3, §3.4 and §7.3).
//!
//! Inline styles reach readers untouched on both public sites, so this
//! grammar, not the sites' sanitisers, decides what readers see (editor.md
//! §3.3, "Why this list"). ammonia's own `filter_style_properties` is never
//! enabled: it re-serialises tokens without a grammar, uses one property set
//! for every element and keeps `https:` `url()` values. `style` goes through
//! ammonia's attribute filter instead, which calls [`sanitize_style`].
//!
//! The work is split in three:
//!
//! - [`declarations`] reads the attribute into declarations with the
//!   cssparser tokenizer and refuses every token no grammar can use
//!   (`url()`, `expression()`, `var()`, `calc()`, `attr()`, `env()`,
//!   strings, blocks, comments, ...);
//! - [`grammar`] holds the per-element table of editor.md §3.3 as data and
//!   reads each value into a typed [`grammar::Value`];
//! - [`values`] reads and writes the value types: lengths in px, sizes in
//!   em, percentages and colours, each in one spelling.
//!
//! [`sanitize_style`] puts them together: it keeps the declarations the
//! element allows, lets the last valid occurrence of a property win, adds
//! the locked declarations that are always present, and writes `prop: value`
//! pairs joined with `; `.
//!
//! ammonia calls the filter only for attributes that exist. The wholly
//! locked styles of `table` and `tbody` are therefore forced attributes in
//! the policy ([`super::policy::TABLE_STYLE`]), which ammonia adds when they
//! are missing. An `img` without a `style` attribute never reaches this
//! module and keeps no style. Its locked declarations are added by a rule of
//! the structural pass ([`super::structure`]), which sees whole elements
//! (editor.md §7.5, "Give an `img` without a `style` its locked
//! declarations"). The pass does not have that rule yet; the sanitiser test
//! `an_image_without_a_style_keeps_none_until_its_structural_rule` pins the
//! gap until it does. The other tree-level style rule of §7.5, which drops
//! `font-size` from a `span` inside a sized `span`, is not there yet either.
//!
//! The canonical style string of editor.md §3.1 is completed by the writer
//! that follows this grammar: it expands the input-only shorthands into the
//! four cell sides and `background-color`, writes every side as its full
//! triple, sorts the declarations, adds or removes the cell `box-sizing`,
//! and runs the alphabet guard. Until then the declarations are written in
//! source order, because sorting before the shorthands are expanded could
//! change which declaration wins the cascade.

pub mod declarations;
pub mod grammar;
pub mod values;

use grammar::{Grammar, Presence, Reading, Value};

/// The result of filtering one `style` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    /// The declarations to store, or `None` when none is left and the
    /// attribute is removed (content-v1 never stores `style=""`).
    pub css: Option<String>,
    /// How many declarations were removed: unreadable, not allowed on the
    /// element, outside their grammar, or after the first
    /// [`declarations::MAX_DECLARATIONS`]. A default that is left out
    /// (`list-style-type: disc`) and an occurrence that a later one replaces
    /// are not counted.
    pub rejected: usize,
}

/// Filters the `style` attribute of one element.
///
/// - `element`: the tag name, as the HTML parser reports it. An element
///   without a grammar keeps no declaration.
/// - `style`: the attribute value, with character references decoded.
#[must_use]
pub fn sanitize_style(element: &str, style: &str) -> Style {
    let read = declarations::read(style);
    let mut rejected = read.unreadable;
    let Some(rules) = grammar::element_style(element) else {
        return Style {
            css: None,
            rejected: rejected + read.read.len(),
        };
    };
    let mut kept: Vec<(&'static str, Value)> = Vec::new();
    for declaration in &read.read {
        let Some(rule) = rules.property(&declaration.name) else {
            rejected += 1;
            continue;
        };
        match grammar::parse(rule.grammar, &declaration.value) {
            // As in a browser, a declaration that is not valid leaves an
            // earlier valid one of the same property in force.
            Reading::Rejected => rejected += 1,
            reading => {
                // The last occurrence wins and keeps its own position, so the
                // order of shorthands and longhands, and with it the cascade,
                // stays as written.
                kept.retain(|(name, _)| *name != rule.name);
                if let Reading::Accepted(value) = reading {
                    kept.push((rule.name, value));
                }
            }
        }
    }
    for rule in rules.properties {
        if let (Presence::Always, Grammar::Locked(value)) = (rule.presence, rule.grammar)
            && !kept.iter().any(|(name, _)| *name == rule.name)
        {
            kept.push((rule.name, Value::Locked(value)));
        }
    }
    let css = (!kept.is_empty()).then(|| {
        kept.iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("; ")
    });
    Style { css, rejected }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::policy::{
        AttributeRule, ELEMENTS, TABLE_STYLE, TBODY_STYLE, ValueRule,
    };
    use proptest::prelude::*;

    fn css(element: &str, style: &str) -> Option<String> {
        sanitize_style(element, style).css
    }

    /// The canonical shapes of editor.md §3.7 are kept byte for byte.
    #[test]
    fn the_canonical_shapes_are_fixed_points() {
        for (element, style) in [
            ("h2", "text-align: center"),
            ("span", "color: #c00000; font-size: 1.25em"),
            ("ul", "list-style-type: circle"),
            ("table", TABLE_STYLE),
            ("tbody", TBODY_STYLE),
            (
                "td",
                "background-color: #d9d9d9; border-bottom: 1px solid #000000; border-left: none; \
                 border-right: 1px solid #000000; border-top: 1px solid #000000; box-sizing: content-box; \
                 height: 20px; text-align: center; vertical-align: middle; white-space: nowrap; width: 128px",
            ),
            (
                "img",
                "display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px",
            ),
            (
                "iframe",
                "aspect-ratio: 16 / 9; height: auto; max-width: 640px; width: 100%",
            ),
            ("iframe", "height: 608px; width: 100%"),
        ] {
            assert_eq!(
                sanitize_style(element, style),
                Style {
                    css: Some(style.to_owned()),
                    rejected: 0
                },
                "{element}"
            );
        }
    }

    /// A `style` that the policy forces is exactly what the grammar writes
    /// for the element, and the grammar locks every declaration of it, so
    /// forcing the attribute loses nothing the grammar would keep.
    #[test]
    fn the_forced_styles_are_the_grammars_locked_declarations() {
        let mut forced = Vec::new();
        for element in ELEMENTS {
            if let Some(AttributeRule {
                rule: ValueRule::Forced(value),
                ..
            }) = element.attribute("style")
            {
                let rules = grammar::element_style(element.name).expect("a grammar");
                assert!(
                    rules
                        .properties
                        .iter()
                        .all(|property| property.presence == Presence::Always
                            && matches!(property.grammar, Grammar::Locked(_))),
                    "{}",
                    element.name
                );
                assert_eq!(css(element.name, "").as_deref(), Some(*value));
                assert_eq!(sanitize_style(element.name, value).rejected, 0);
                forced.push(element.name);
            }
        }
        assert_eq!(forced, ["tbody", "table"]);
    }

    /// Every row of editor.md §3.4 is rejected wherever it appears.
    #[test]
    fn the_rejected_properties_are_removed() {
        for (element, declaration) in [
            ("span", "font-family: Calibri"),
            ("span", "font: 11pt Calibri"),
            ("td", "font-weight: bold"),
            ("td", "font-style: italic"),
            ("h2", "font-style: italic"),
            ("td", "text-decoration: underline"),
            ("td", "color: red"),
            ("p", "color: red"),
            ("td", "font-size: 1em"),
            ("p", "font-size: 1em"),
            ("td", "padding: 1px"),
            ("td", "padding-left: 1px"),
            ("p", "margin: 0 auto"),
            ("p", "margin-bottom: 0"),
            ("p", "text-indent: 1em"),
            ("p", "line-height: 1.5"),
            ("img", "float: left"),
            ("p", "position: absolute"),
            ("p", "top: 0"),
            ("p", "left: 0"),
            ("p", "padding-bottom: 56.25%"),
            ("iframe", "border: 0"),
            ("table", "width: 100%"),
            ("table", "min-width: 100%"),
            ("td", "display: none"),
            ("td", "visibility: hidden"),
            ("td", "background-image: url(x.png)"),
            ("ul", "list-style: circle"),
            ("ul", "list-style-image: url(x.png)"),
            ("td", "border-image: url(x.png) 30"),
            ("td", "background: url(https://evil.example/x.png)"),
            ("td", "background-color: var(--x)"),
            ("td", "width: calc(10px + 1px)"),
            ("td", "width: expression(alert(1))"),
            ("span", "color: attr(data-x)"),
            ("td", "width: env(x)"),
            ("p", "behavior: url(x.htc)"),
            ("p", "-moz-binding: url(x.xml#y)"),
            ("td", "mso-number-format: General"),
            ("td", "-webkit-text-stroke: 1px"),
            ("td", "--x: red"),
            ("span", "color: transparent"),
            ("span", "color: rgba(255, 0, 0, 0.5)"),
            ("span", "color: currentcolor"),
        ] {
            // What is left is what the element has without any input: the
            // always-present locked declarations, if it has any.
            assert_eq!(
                sanitize_style(element, declaration),
                Style {
                    css: css(element, ""),
                    rejected: 1
                },
                "{element} {{ {declaration} }}"
            );
        }
    }

    #[test]
    fn allowed_declarations_survive_beside_rejected_ones() {
        let style = sanitize_style(
            "span",
            "font-family: Calibri; COLOR: Red !important; mso-ansi-language: HU; font-size: 1.5EM",
        );
        assert_eq!(
            style,
            Style {
                css: Some("color: #ff0000; font-size: 1.5em".into()),
                rejected: 2
            }
        );
        assert_eq!(
            css(
                "td",
                "width: 96pt; color: red; /* x */; vertical-align: bottom"
            ),
            Some("width: 128px; vertical-align: bottom".into())
        );
    }

    #[test]
    fn the_last_valid_occurrence_wins_in_its_own_place() {
        assert_eq!(
            css("span", "color: red; background-color: blue; color: green"),
            Some("background-color: #0000ff; color: #008000".into())
        );
        // An invalid later occurrence leaves the earlier one in force.
        assert_eq!(
            sanitize_style("span", "color: red; color: hsl(0, 100%, 50%)"),
            Style {
                css: Some("color: #ff0000".into()),
                rejected: 1
            }
        );
        // Shorthands and sides keep their written order.
        assert_eq!(
            css(
                "td",
                "border-top: 1px solid; border: none; border-top: 2px dashed"
            ),
            Some("border: none; border-top: 2px dashed".into())
        );
        // A default that comes last removes the earlier value.
        assert_eq!(
            sanitize_style("ul", "list-style-type: circle; list-style-type: disc"),
            Style {
                css: None,
                rejected: 0
            }
        );
    }

    #[test]
    fn locked_declarations_are_always_present() {
        let image = "display: block; height: auto; margin: 0 auto; max-width: 100%";
        assert_eq!(css("img", ""), Some(image.into()));
        assert_eq!(
            css("img", "width: 350px"),
            Some(format!("width: 350px; {image}"))
        );
        // A legacy fixed height becomes the locked `auto`.
        assert_eq!(
            sanitize_style("img", "height: 495px; width: 350px; float: left"),
            Style {
                css: Some(format!("width: 350px; {image}")),
                rejected: 2
            }
        );
        assert_eq!(
            css("img", "MARGIN: 0PX AUTO; display: BLOCK"),
            Some("margin: 0 auto; display: block; height: auto; max-width: 100%".into())
        );
        assert_eq!(css("table", "width: 100%"), Some(TABLE_STYLE.into()));
        assert_eq!(css("table", ""), Some(TABLE_STYLE.into()));
        assert_eq!(css("tbody", "display: table"), Some(TBODY_STYLE.into()));
        // Locked declarations that are not always present stay optional.
        assert_eq!(css("iframe", "height: 608px"), Some("height: 608px".into()));
        assert_eq!(css("td", "width: 10px"), Some("width: 10px".into()));
    }

    #[test]
    fn nothing_left_means_no_attribute() {
        assert_eq!(
            sanitize_style("p", ""),
            Style {
                css: None,
                rejected: 0
            }
        );
        assert_eq!(
            sanitize_style("p", " ; ;"),
            Style {
                css: None,
                rejected: 0
            }
        );
        assert_eq!(
            sanitize_style("p", "color: red; x"),
            Style {
                css: None,
                rejected: 2
            }
        );
        assert_eq!(
            sanitize_style("ul", "list-style-type: disc"),
            Style {
                css: None,
                rejected: 0
            }
        );
        // Elements without a grammar keep nothing.
        assert_eq!(
            sanitize_style("li", "text-align: center; color: red"),
            Style {
                css: None,
                rejected: 2
            }
        );
        assert_eq!(
            sanitize_style("div", "x"),
            Style {
                css: None,
                rejected: 1
            }
        );
    }

    #[test]
    fn the_declaration_limit_applies() {
        let many = (0..40)
            .map(|_| "text-align: left")
            .collect::<Vec<_>>()
            .join("; ");
        assert_eq!(
            sanitize_style("p", &many),
            Style {
                css: Some("text-align: left".into()),
                rejected: 8
            }
        );
    }

    /// The characters editor.md §3.1 rule 6 allows in a stored style.
    fn in_alphabet(css: &str) -> bool {
        css.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b" #.%:;/-".contains(&b))
    }

    const ELEMENT_POOL: &[&str] = &[
        "p", "h2", "span", "ul", "img", "table", "tbody", "td", "th", "iframe", "li",
    ];

    #[rustfmt::skip]
    const PROPERTY_POOL: &[&str] = &[
        "color", "COLOR", "background", "background-color", "font-size", "font-family", "text-align",
        "vertical-align", "white-space", "width", "height", "max-width", "box-sizing", "border",
        "border-top", "border-left", "border-width", "border-style", "border-color", "display",
        "margin", "aspect-ratio", "list-style-type", "border-collapse", "padding", "--x", "-webkit-x",
    ];

    /// The CSS pool of editor.md §13.6 (P1), plus the spellings of the
    /// accepted values.
    #[rustfmt::skip]
    const VALUE_POOL: &[&str] = &[
        "red", "#C00000", "#c008", "windowtext", "transparent", "currentcolor", "rgb(1, 2, 3)",
        "rgba(1, 2, 3, .5)", "hsl(0, 100%, 50%)", "1px", ".5pt", "15.0pt", "100%", "100%px", "1e30px",
        "-1px", "0", "1.25em", "3em", "auto", "none", "solid", "thin", "medium", "center", "nowrap",
        "block", "collapse", "fit-content", "table-row-group", "content-box", "circle", "disc", "16 / 9",
        "0 auto", "url(x)", "url(\"https://evil.example/x\")", "u\\72l(x)", "expression(alert(1))",
        "var(--x)", "calc(1px + 1px)", "attr(x)", "env(x)", "!important", "/* c */", "\"x\"", "'x'",
        "(", ")", "[", "}", "@import", "é", "\u{0}",
    ];

    fn arb_style() -> impl Strategy<Value = String> {
        prop::collection::vec(
            (
                prop::sample::select(PROPERTY_POOL),
                prop::collection::vec(prop::sample::select(VALUE_POOL), 0..4),
            ),
            0..12,
        )
        .prop_map(|declarations| {
            declarations
                .into_iter()
                .map(|(name, value)| format!("{name}: {}", value.join(" ")))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    proptest! {
        #[test]
        fn filtering_twice_changes_nothing(element in prop::sample::select(ELEMENT_POOL), style in arb_style()) {
            let once = sanitize_style(element, &style);
            if let Some(css) = &once.css {
                prop_assert_eq!(sanitize_style(element, css), Style { css: Some(css.clone()), rejected: 0 }, "{}", style);
            }
        }

        #[test]
        fn the_output_stays_in_the_alphabet(element in prop::sample::select(ELEMENT_POOL), style in arb_style()) {
            if let Some(css) = sanitize_style(element, &style).css {
                prop_assert!(in_alphabet(&css), "{} -> {}", style, css);
                prop_assert!(!css.is_empty() && !css.ends_with(';'));
            }
        }

        #[test]
        fn arbitrary_input_is_filtered_safely(element in prop::sample::select(ELEMENT_POOL), style in any::<String>()) {
            let once = sanitize_style(element, &style);
            if let Some(css) = once.css {
                prop_assert!(in_alphabet(&css), "{:?} -> {}", style, css);
                prop_assert_eq!(sanitize_style(element, &css).css, Some(css.clone()));
            }
        }

        #[test]
        fn every_declaration_is_kept_or_counted(element in prop::sample::select(ELEMENT_POOL), style in arb_style()) {
            let read = declarations::read(&style);
            let total = read.read.len() + read.unreadable;
            let filtered = sanitize_style(element, &style);
            let kept = filtered.css.as_deref().map_or(0, |css| css.split("; ").count());
            prop_assert!(filtered.rejected <= total);
            // Each kept declaration came from the input or is a locked one
            // that is always present.
            let always = grammar::element_style(element).map_or(0, |rules| rules.properties.iter().filter(|p| p.presence == Presence::Always).count());
            prop_assert!(kept <= total - filtered.rejected + always, "{} -> {:?}", style, filtered);
        }
    }
}
