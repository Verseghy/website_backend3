//! The canonical style string of editor.md §3.1: the writer that turns the
//! declarations the grammar accepted into the one spelling content-v1
//! stores.
//!
//! There is one spelling for each visual result, so that the editor, the
//! paste filter, the import and the sanitiser cannot store the same style
//! two ways, and so that `S(S(x)) == S(x)` holds byte for byte.
//! [`super::sanitize_style`] reads a `style` attribute with the grammar and
//! hands every accepted declaration, in source order, to a [`Cascade`],
//! which resolves them as CSS's cascade does (rule 1):
//!
//! - Each declaration sets the longhands it names, and the last one wins. A
//!   declaration that the grammar rejected sets nothing, so an earlier valid
//!   one stays in force, as in a browser. A default (`list-style-type:
//!   disc`) unsets the property.
//! - The cell border properties set twelve longhands: a width, a style and
//!   a colour for each of the four [`SIDES`]. `border` sets all twelve,
//!   `border-<side>` the three of its side, and `border-width`,
//!   `border-style` and `border-color` one longhand of every side with
//!   CSS's one-to-four value rule (top, right, bottom, left).
//! - `border` and `border-<side>` reset every longhand they leave out to its
//!   initial value ([`Side::INITIAL`]): the width of `medium`, 3 px
//!   ([`BORDER_WIDTH_INITIAL`]), the style `none` and the colour
//!   `currentcolor` (CSS Backgrounds 3 §3.4: "omitted values are set to
//!   their initial values"). So `border-top: solid` draws a 3 px border, as
//!   in a browser.
//! - A longhand that no declaration set keeps what the page gives it
//!   ([`Side::UNSET`]): width 0, style `none` and colour `currentcolor`,
//!   from the `border: 0` reset that both public sites and the editor apply
//!   to every element (editor.md §3.2, §11.2). A stored style is rendered
//!   against that reset, so `border-style: solid` alone, which draws no
//!   border there, is written as four `0px solid currentcolor` sides and
//!   still draws none. Taking CSS's initial width here instead would add a
//!   3 px border on every side that the input never drew. The sites' own
//!   cell lines (§11.3) are page styling, not content, so no stored side
//!   copies them.
//! - A single-colour `background` sets `background-color`.
//!
//! [`Cascade::write`] then writes the stored declarations:
//!
//! - Once any border declaration was accepted, all four sides are written,
//!   because they are "four per-side triples" (rule 1). A side whose style
//!   is `none` (also when no declaration gave it one) is written `none`,
//!   and one whose style is `hidden` is written `hidden`: both render no
//!   border whatever the width and colour, and `hidden` differs from `none`
//!   only in how collapsed borders win against their neighbours. Every
//!   other side is written as its full triple, `<width>px <style>
//!   <colour>` (rule 2), so `medium` and a width that a shorthand leaves
//!   out, which render the same, are stored the same.
//! - The locked declarations that are always present are added, and a
//!   size-dependent one (the cell `box-sizing`) is added exactly when the
//!   style has a `width` or a `height`, and removed otherwise (editor.md
//!   §3.3, [`Presence`]).
//! - The declarations are sorted by property name and written `prop: value`,
//!   joined with `; `, with no trailing `;` (rule 5). No shorthand survives,
//!   so the order cannot change the cascade. The names are ASCII, so their
//!   byte order is their alphabetical order, and it is also the order of
//!   JavaScript's default `Array.prototype.sort`, which the admin's
//!   `canonicalStyle` uses.
//! - Nothing left means no attribute: content-v1 never stores `style=""`.
//!
//! [`in_alphabet`] is the alphabet guard of rule 6, which
//! [`super::sanitize_style`] runs on the written string.
//!
//! [`declares`] and [`without`] read and edit a string this writer wrote, for
//! the structural pass, which drops the `font-size` of a `span` inside a
//! sized `span` (editor.md §7.5).

use std::collections::BTreeMap;
use std::fmt;

use super::grammar::{BORDER_WIDTH_INITIAL, BorderSide, ElementStyle, Grammar, Presence, Value};
use super::values::{Colour, Hundredths};

/// The four cell sides, in the order of CSS's one-to-four value
/// shorthands: top, right, bottom, left.
pub const SIDES: [&str; 4] = ["border-top", "border-right", "border-bottom", "border-left"];

/// One cell side after the cascade: its three longhands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Side {
    /// The width in px.
    pub width: Hundredths,
    /// The style keyword, one of the grammar's border styles.
    pub style: &'static str,
    /// The colour.
    pub colour: Colour,
}

impl Side {
    /// CSS's initial values, which `border` and `border-<side>` give every
    /// part they leave out: the width of `medium`, the style `none` and the
    /// colour `currentcolor` (CSS Backgrounds 3 §3.4).
    pub const INITIAL: Self = Self {
        width: BORDER_WIDTH_INITIAL,
        style: "none",
        colour: Colour::CurrentColor,
    };

    /// A side that no declaration set: what the `border: 0` reset of both
    /// public sites and the editor gives every element (editor.md §3.2,
    /// §11.2), width 0, style `none` and colour `currentcolor`.
    /// `border-width`, `border-style` and `border-color` each change one
    /// part of every side and leave the other two as the page has them, so
    /// `border-style: solid` alone draws no border (see the module
    /// documentation).
    pub const UNSET: Self = Self {
        width: Hundredths::ZERO,
        style: "none",
        colour: Colour::CurrentColor,
    };

    /// The side that a border shorthand writes: its parts, and the initial
    /// value for each part it leaves out ([`Self::INITIAL`]).
    ///
    /// - `side`: the shorthand's value, as the grammar read it.
    #[must_use]
    pub fn from_shorthand(side: BorderSide) -> Self {
        Self {
            width: side.width.unwrap_or(Self::INITIAL.width),
            style: side.style.unwrap_or(Self::INITIAL.style),
            colour: side.colour.unwrap_or(Self::INITIAL.colour),
        }
    }
}

/// `none` or `hidden` alone, otherwise `<width>px <style> <colour>`
/// (editor.md §3.1 rule 2).
impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.style {
            "none" | "hidden" => f.write_str(self.style),
            style => write!(f, "{}px {style} {}", self.width, self.colour),
        }
    }
}

/// The accepted declarations of one `style` attribute, resolved with CSS's
/// cascade (see the module documentation).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cascade {
    /// The stored properties other than the border sides, by name.
    values: BTreeMap<&'static str, Value>,
    /// The four sides, once any border declaration was accepted.
    sides: Option<[Side; 4]>,
}

impl Cascade {
    /// Applies one accepted declaration, after every earlier one.
    ///
    /// - `property`: the property's name as the grammar lists it.
    /// - `value`: its value, as the grammar read it.
    pub fn set(&mut self, property: &'static str, value: Value) {
        match value {
            Value::BorderSide(side) => {
                let side = Side::from_shorthand(side);
                // The sides this shorthand does not name stay as the page
                // has them, not at CSS's initial values.
                let sides = self.sides.get_or_insert([Side::UNSET; 4]);
                for &index in sides_named(property) {
                    sides[index] = side;
                }
            }
            Value::Widths(widths) => self.set_each(&widths, |side, width| side.width = width),
            Value::Styles(styles) => self.set_each(&styles, |side, style| side.style = style),
            Value::Colours(colours) => {
                self.set_each(&colours, |side, colour| side.colour = colour);
            }
            value => {
                self.values.insert(stored_name(property), value);
            }
        }
    }

    /// Unsets `property`: its default came last, so the property is left
    /// out (`list-style-type: disc`).
    ///
    /// - `property`: the property's name as the grammar lists it.
    pub fn unset(&mut self, property: &'static str) {
        self.values.remove(stored_name(property));
    }

    /// Sets one longhand of every side from a one-to-four value list. The
    /// other two longhands keep what an earlier declaration gave them, or
    /// else what the page gives them ([`Side::UNSET`]): these properties
    /// reset nothing.
    fn set_each<T: Copy>(&mut self, values: &[T], mut set: impl FnMut(&mut Side, T)) {
        // The grammar accepts one to four values, so `four` always answers.
        if let Some(values) = four(values) {
            let sides = self.sides.get_or_insert([Side::UNSET; 4]);
            for (side, value) in sides.iter_mut().zip(values) {
                set(side, value);
            }
        }
    }

    /// Writes the canonical style string, or `None` when nothing is left.
    ///
    /// - `rules`: the grammar of the element whose style this is; it gives
    ///   the locked declarations that are always present or depend on a
    ///   size.
    #[must_use]
    pub fn write(self, rules: &ElementStyle) -> Option<String> {
        let Self { mut values, sides } = self;
        let sized = values.contains_key("width") || values.contains_key("height");
        for rule in rules.properties {
            let Grammar::Locked(locked) = rule.grammar else {
                continue;
            };
            match rule.presence {
                Presence::Always => {
                    values.entry(rule.name).or_insert(Value::Locked(locked));
                }
                Presence::WhenSized if sized => {
                    values.insert(rule.name, Value::Locked(locked));
                }
                Presence::WhenSized => {
                    values.remove(rule.name);
                }
                Presence::Optional => {}
            }
        }
        let mut declarations: Vec<(&str, String)> = values
            .iter()
            .map(|(&name, value)| (name, value.to_string()))
            .collect();
        if let Some(sides) = sides {
            declarations.extend(
                SIDES
                    .iter()
                    .zip(sides)
                    .map(|(&name, side)| (name, side.to_string())),
            );
        }
        // The names are unique: the sides are not in `values`, which holds
        // each other name once.
        declarations.sort_unstable_by_key(|&(name, _)| name);
        (!declarations.is_empty()).then(|| {
            declarations
                .iter()
                .map(|(name, value)| format!("{name}: {value}"))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
}

/// The indexes into [`SIDES`] that a border-side shorthand sets: all four
/// for `border`, its own for `border-<side>`. The grammar gives a border
/// side value to no other property (a test checks it).
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

/// The stored property that `property` sets: `background-color` for the
/// input-only `background`, the property itself otherwise.
fn stored_name(property: &'static str) -> &'static str {
    match property {
        "background" => "background-color",
        other => other,
    }
}

/// CSS's one-to-four value rule: the values for top, right, bottom and
/// left, or `None` for a list of another length.
///
/// - `values`: one to four values, as written.
#[must_use]
pub fn four<T: Copy>(values: &[T]) -> Option<[T; 4]> {
    match *values {
        [all] => Some([all; 4]),
        [vertical, horizontal] => Some([vertical, horizontal, vertical, horizontal]),
        [top, horizontal, bottom] => Some([top, horizontal, bottom, horizontal]),
        [top, right, bottom, left] => Some([top, right, bottom, left]),
        _ => None,
    }
}

/// Whether `css` uses only the characters a stored style may contain:
/// `a-z`, `0-9`, space and `# . % : ; / -` (editor.md §3.1 rule 6). `/`
/// exists only for `aspect-ratio: 16 / 9`. A CSS comment needs `*` and
/// `url(` needs `(`, so neither can pass.
///
/// - `css`: a written style string.
#[must_use]
pub fn in_alphabet(css: &str) -> bool {
    css.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b" #.%:;/-".contains(&b))
}

/// The declarations of `css` as `(name, value)` pairs.
fn pairs(css: &str) -> impl Iterator<Item = (&str, &str)> {
    css.split("; ")
        .map(|declaration| declaration.split_once(": ").unwrap_or((declaration, "")))
}

/// Whether the canonical style string `css` declares `property`.
///
/// - `css`: a string that [`Cascade::write`] wrote; its declarations are
///   joined with `; ` and written `prop: value`, and no value holds `;`.
/// - `property`: a lower-case property name.
#[must_use]
pub fn declares(css: &str, property: &str) -> bool {
    pairs(css).any(|(name, _)| name == property)
}

/// The canonical style string `css` without its `property` declaration, or
/// `None` when nothing is left. The declarations that stay keep their
/// sorted order, so the result is canonical too, as long as the element
/// has no locked declaration that depends on the one removed (the
/// structural pass removes only a `span` `font-size`).
///
/// - `css`: a string that [`Cascade::write`] wrote, as for [`declares`].
/// - `property`: the lower-case name of the property to remove.
#[must_use]
pub fn without(css: &str, property: &str) -> Option<String> {
    let kept: Vec<&str> = css
        .split("; ")
        .filter(|declaration| {
            declaration
                .split_once(": ")
                .is_none_or(|(name, _)| name != property)
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::css::grammar::{
        BORDER_STYLES, ELEMENT_STYLES, Role, element_style,
    };
    use crate::domain::rich_text::css::sanitize_style;
    use proptest::prelude::*;

    fn css(element: &str, style: &str) -> Option<String> {
        sanitize_style(element, style).css
    }

    const NONE_SIDES: &str =
        "border-bottom: none; border-left: none; border-right: none; border-top: none";

    /// The canonicalisation table of editor.md §3.1: input, element and the
    /// one spelling that is stored. Every row is checked to be a fixed
    /// point as well.
    #[rustfmt::skip]
    const TABLE: &[(&str, &str, Option<&str>)] = &[
        // Rule 1: `border` sets all four sides; its missing parts are the
        // initial values.
        ("td", "border: 1px solid #000", Some("border-bottom: 1px solid #000000; border-left: 1px solid #000000; border-right: 1px solid #000000; border-top: 1px solid #000000")),
        ("td", "border: solid", Some("border-bottom: 3px solid currentcolor; border-left: 3px solid currentcolor; border-right: 3px solid currentcolor; border-top: 3px solid currentcolor")),
        ("td", "border: medium solid", Some("border-bottom: 3px solid currentcolor; border-left: 3px solid currentcolor; border-right: 3px solid currentcolor; border-top: 3px solid currentcolor")),
        ("td", "border: none", Some(NONE_SIDES)),
        ("td", "border: 1px", Some(NONE_SIDES)),
        ("td", "border: red", Some(NONE_SIDES)),
        ("td", "border: solid windowtext 1.0pt", Some("border-bottom: 1.33px solid #000000; border-left: 1.33px solid #000000; border-right: 1.33px solid #000000; border-top: 1.33px solid #000000")),
        // Rule 1: one side; the sides no declaration set are `none`.
        ("td", "border-top: 1px solid black", Some("border-bottom: none; border-left: none; border-right: none; border-top: 1px solid #000000")),
        ("th", "border-left: thick double", Some("border-bottom: none; border-left: 5px double currentcolor; border-right: none; border-top: none")),
        // Rule 1: the cascade in source order, a later shorthand resetting
        // an earlier side and a later side overriding the shorthand.
        ("td", "border-top: 1px solid; border: none; border-top: 2px dashed", Some("border-bottom: none; border-left: none; border-right: none; border-top: 2px dashed currentcolor")),
        ("td", "border-bottom: 1px solid red; border: 2px dotted", Some("border-bottom: 2px dotted currentcolor; border-left: 2px dotted currentcolor; border-right: 2px dotted currentcolor; border-top: 2px dotted currentcolor")),
        ("td", "border: 1px solid; border-right: none; border-left: hidden", Some("border-bottom: 1px solid currentcolor; border-left: hidden; border-right: none; border-top: 1px solid currentcolor")),
        // Rule 1: the one-to-four value rule of the longhand shorthands.
        ("td", "border-style: solid; border-width: 1px 2px; border-color: red green blue", Some("border-bottom: 1px solid #0000ff; border-left: 2px solid #008000; border-right: 2px solid #008000; border-top: 1px solid #ff0000")),
        ("td", "border-style: solid dashed dotted double; border-width: thin", Some("border-bottom: 1px dotted currentcolor; border-left: 1px double currentcolor; border-right: 1px dashed currentcolor; border-top: 1px solid currentcolor")),
        ("td", "border: 1px solid #000; border-width: 0 2px", Some("border-bottom: 0px solid #000000; border-left: 2px solid #000000; border-right: 2px solid #000000; border-top: 0px solid #000000")),
        ("td", "border-width: 1px", Some(NONE_SIDES)),
        ("td", "border-color: red", Some(NONE_SIDES)),
        ("td", "border-style: hidden none", Some("border-bottom: hidden; border-left: none; border-right: none; border-top: hidden")),
        // Rule 1: a part that no declaration set is the page's, from the
        // `border: 0` reset (editor.md §3.2): a width of 0, so a style alone
        // draws no border. The longhand shorthands reset nothing.
        ("td", "border-style: solid", Some("border-bottom: 0px solid currentcolor; border-left: 0px solid currentcolor; border-right: 0px solid currentcolor; border-top: 0px solid currentcolor")),
        ("td", "border-style: solid; border-color: red", Some("border-bottom: 0px solid #ff0000; border-left: 0px solid #ff0000; border-right: 0px solid #ff0000; border-top: 0px solid #ff0000")),
        ("td", "border-color: red; border-style: dashed none", Some("border-bottom: 0px dashed #ff0000; border-left: none; border-right: none; border-top: 0px dashed #ff0000")),
        ("td", "border-top: 1px solid; border-style: dashed", Some("border-bottom: 0px dashed currentcolor; border-left: 0px dashed currentcolor; border-right: 0px dashed currentcolor; border-top: 1px dashed currentcolor")),
        ("td", "border-style: solid; border-width: 2px", Some("border-bottom: 2px solid currentcolor; border-left: 2px solid currentcolor; border-right: 2px solid currentcolor; border-top: 2px solid currentcolor")),
        // Rule 1: a later `border` or `border-<side>` resets every part it
        // leaves out to its initial value, an earlier width included.
        ("td", "border: 1px solid red; border-top: dashed", Some("border-bottom: 1px solid #ff0000; border-left: 1px solid #ff0000; border-right: 1px solid #ff0000; border-top: 3px dashed currentcolor")),
        ("td", "border-width: 1px; border-top: solid", Some("border-bottom: none; border-left: none; border-right: none; border-top: 3px solid currentcolor")),
        ("td", "border-width: 2px; border-top: solid; border-style: solid", Some("border-bottom: 2px solid currentcolor; border-left: 2px solid currentcolor; border-right: 2px solid currentcolor; border-top: 3px solid currentcolor")),
        // Rule 1: an invalid declaration leaves the earlier one in force.
        ("td", "border: 1px solid; border: 1px wavy", Some("border-bottom: 1px solid currentcolor; border-left: 1px solid currentcolor; border-right: 1px solid currentcolor; border-top: 1px solid currentcolor")),
        // Rule 1: a single-colour `background` is `background-color`.
        ("td", "background: #D9D9D9", Some("background-color: #d9d9d9")),
        ("td", "background-color: red; background: blue", Some("background-color: #0000ff")),
        ("td", "background: blue; background-color: red", Some("background-color: #ff0000")),
        ("td", "background: blue; background: url(x)", Some("background-color: #0000ff")),
        ("span", "background: yellow; color: red", Some("background-color: #ffff00; color: #ff0000")),
        // Rule 2: `none` and `hidden` alone, every other side a triple.
        ("td", "border-top: 1px none red", Some("border-bottom: none; border-left: none; border-right: none; border-top: none")),
        ("td", "border-top: 2px hidden red", Some("border-bottom: none; border-left: none; border-right: none; border-top: hidden")),
        ("td", "border-top: medium; border-right: medium none; border-bottom: medium none currentcolor", Some(NONE_SIDES)),
        ("td", "border-top: 0 solid", Some("border-bottom: none; border-left: none; border-right: none; border-top: 0px solid currentcolor")),
        ("td", "border-top: .5pt solid transparent", Some("border-bottom: none; border-left: none; border-right: none; border-top: 0.67px solid transparent")),
        // §3.3: `box-sizing` exactly when `width` or `height` is present.
        ("td", "width: 96pt", Some("box-sizing: content-box; width: 128px")),
        ("th", "height: 20px", Some("box-sizing: content-box; height: 20px")),
        ("td", "box-sizing: content-box; width: 10px", Some("box-sizing: content-box; width: 10px")),
        ("td", "box-sizing: content-box", None),
        ("td", "box-sizing: content-box; text-align: center", Some("text-align: center")),
        ("td", "width: 10px; width: 100%", Some("box-sizing: content-box; width: 10px")),
        ("td", "width: 100%; box-sizing: content-box", None),
        ("td", "box-sizing: border-box; width: 10px", Some("box-sizing: content-box; width: 10px")),
        // Rule 5: sorted by name, whatever the input order.
        ("td", "width: 10px; vertical-align: top; text-align: left; white-space: nowrap; height: 2px; background-color: red; border-left: none",
            Some("background-color: #ff0000; border-bottom: none; border-left: none; border-right: none; border-top: none; box-sizing: content-box; height: 2px; text-align: left; vertical-align: top; white-space: nowrap; width: 10px")),
        ("span", "font-size: 1.5em; color: red; background-color: yellow", Some("background-color: #ffff00; color: #ff0000; font-size: 1.5em")),
        ("img", "width: 350px; max-width: 100%; margin: 0 auto; height: auto; display: block", Some("display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px")),
        ("img", "width: 262.5pt", Some("display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px")),
        ("iframe", "width: 100%; max-width: 640px; height: auto; aspect-ratio: 16/9", Some("aspect-ratio: 16 / 9; height: auto; max-width: 640px; width: 100%")),
        ("table", "max-width: fit-content; border-collapse: collapse", Some("border-collapse: collapse; max-width: fit-content")),
        // Rule 5: the last occurrence wins, `!important` is stripped, and
        // nothing left is no attribute.
        ("span", "color: red !important; color: blue", Some("color: #0000ff")),
        ("ul", "list-style-type: square; list-style-type: disc", None),
        ("p", "", None),
        ("p", "color: red", None),
    ];

    #[test]
    fn the_canonicalisation_table() {
        for &(element, input, expected) in TABLE {
            let output = css(element, input);
            assert_eq!(output.as_deref(), expected, "{element} {{ {input} }}");
            if let Some(output) = &output {
                assert_eq!(
                    css(element, output).as_deref(),
                    Some(output.as_str()),
                    "fixed point"
                );
                assert!(in_alphabet(output), "{output}");
            }
        }
    }

    #[test]
    fn sides_read_like_css_initial_values() {
        let side = |width, style, colour| BorderSide {
            width,
            style,
            colour,
        };
        assert_eq!(Side::from_shorthand(side(None, None, None)), Side::INITIAL);
        assert_eq!(Side::INITIAL.to_string(), "none");
        let solid = Side::from_shorthand(side(None, Some("solid"), None));
        assert_eq!(solid.to_string(), "3px solid currentcolor");
        // What the page gives a side differs from CSS's initial values only
        // in its width.
        assert_eq!(Side::UNSET.to_string(), "none");
        assert_eq!(
            Side::UNSET,
            Side {
                width: Hundredths::ZERO,
                ..Side::INITIAL
            }
        );
        let styled = Side {
            style: "solid",
            ..Side::UNSET
        };
        assert_eq!(styled.to_string(), "0px solid currentcolor");
        let red = Side::from_shorthand(side(
            Some(Hundredths::from_hundredths(67)),
            Some("double"),
            Some(Colour::opaque(255, 0, 0)),
        ));
        assert_eq!(red.to_string(), "0.67px double #ff0000");
        let hidden = Side {
            style: "hidden",
            ..red
        };
        assert_eq!(hidden.to_string(), "hidden");
    }

    #[test]
    fn four_follows_the_one_to_four_rule() {
        assert_eq!(four(&[1]), Some([1, 1, 1, 1]));
        assert_eq!(four(&[1, 2]), Some([1, 2, 1, 2]));
        assert_eq!(four(&[1, 2, 3]), Some([1, 2, 3, 2]));
        assert_eq!(four(&[1, 2, 3, 4]), Some([1, 2, 3, 4]));
        assert_eq!(four::<u8>(&[]), None);
        assert_eq!(four(&[1, 2, 3, 4, 5]), None);
    }

    #[test]
    fn the_cascade_writes_nothing_for_nothing() {
        for style in ELEMENT_STYLES {
            let written = Cascade::default().write(style);
            // Only the always-present locked declarations are left.
            assert_eq!(written, css(style.element, ""), "{}", style.element);
        }
        let td = element_style("td").expect("a grammar");
        assert_eq!(Cascade::default().write(td), None);
        let mut cascade = Cascade::default();
        cascade.set("list-style-type", Value::Keyword("circle"));
        cascade.unset("list-style-type");
        assert_eq!(cascade.write(element_style("ul").expect("a grammar")), None);
    }

    /// Every border-side shorthand of the grammar names sides, so no
    /// accepted border value is lost.
    #[test]
    fn every_border_side_property_names_sides() {
        for style in ELEMENT_STYLES {
            for property in style.properties {
                if property.grammar == Grammar::BorderSide {
                    assert!(!sides_named(property.name).is_empty(), "{}", property.name);
                }
            }
        }
        assert_eq!(sides_named("width"), &[] as &[usize]);
    }

    #[test]
    fn the_alphabet_is_rule_six() {
        assert!(in_alphabet(
            "aspect-ratio: 16 / 9; color: #c00000; max-width: 100%; width: 0.5px"
        ));
        assert!(in_alphabet(""));
        for outside in [
            "COLOR", "url(x)", "/* x */", "\"x\"", "a_b", "é", "\0", "!", "a\tb", ",", "+",
        ] {
            assert!(!in_alphabet(outside), "{outside:?}");
        }
    }

    #[test]
    fn declarations_are_found_and_removed() {
        let span = "background-color: #ffff00; color: #c00000; font-size: 1.25em";
        assert!(declares(span, "font-size"));
        assert!(declares(span, "color"));
        assert!(!declares(span, "size"));
        assert!(!declares(span, "font"));
        assert_eq!(
            without(span, "font-size").as_deref(),
            Some("background-color: #ffff00; color: #c00000")
        );
        assert_eq!(
            without(span, "color").as_deref(),
            Some("background-color: #ffff00; font-size: 1.25em")
        );
        assert_eq!(without(span, "width").as_deref(), Some(span));
        assert_eq!(without("font-size: 3em", "font-size"), None);
    }

    /// Properties and values for the proptests below, from the §13.6 CSS
    /// pool and the spellings of accepted values.
    #[rustfmt::skip]
    const CELL_PROPERTIES: &[&str] = &[
        "border", "border-top", "border-right", "border-bottom", "border-left", "border-width",
        "border-style", "border-color", "background", "background-color", "width", "height",
        "box-sizing", "text-align", "vertical-align", "white-space", "BORDER-TOP", "padding",
    ];

    #[rustfmt::skip]
    const CELL_VALUES: &[&str] = &[
        "1px", "0", ".5pt", "15.0pt", "21px", "thin", "medium", "thick", "solid", "dashed", "double",
        "none", "hidden", "wavy", "red", "#000", "windowtext", "transparent", "currentcolor",
        "rgb(1, 2, 3)", "content-box", "border-box", "10px", "100%", "center", "top", "nowrap",
        "url(x)", "!important", "/* c */",
    ];

    fn arb_cell_style() -> impl Strategy<Value = String> {
        prop::collection::vec(
            (
                prop::sample::select(CELL_PROPERTIES),
                prop::collection::vec(prop::sample::select(CELL_VALUES), 1..5),
            ),
            0..10,
        )
        .prop_map(|declarations| {
            declarations
                .into_iter()
                .map(|(name, value)| format!("{name}: {}", value.join(" ")))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    /// The cell properties the [`Op::Unset`] calls name.
    const UNSET_PROPERTIES: &[&str] = &[
        "width",
        "height",
        "text-align",
        "background-color",
        "box-sizing",
    ];

    /// The border-side shorthands, in [`sides_named`]'s terms.
    const SIDE_SHORTHANDS: &[&str] = &[
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
    ];

    fn arb_colour() -> impl Strategy<Value = Colour> {
        prop_oneof![
            (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(
                |(red, green, blue, alpha)| Colour::Rgba {
                    red,
                    green,
                    blue,
                    alpha,
                }
            ),
            Just(Colour::Transparent),
            Just(Colour::CurrentColor),
        ]
    }

    /// A border width the grammar accepts: 0 to 20 px.
    fn arb_border_width() -> impl Strategy<Value = Hundredths> {
        (0..=2000_u32).prop_map(Hundredths::from_hundredths)
    }

    fn arb_border_side() -> impl Strategy<Value = BorderSide> {
        (
            prop::option::of(arb_border_width()),
            prop::option::of(prop::sample::select(BORDER_STYLES)),
            prop::option::of(arb_colour()),
        )
            .prop_map(|(width, style, colour)| BorderSide {
                width,
                style,
                colour,
            })
    }

    /// One call on a [`Cascade`], with a value the cell grammar could have
    /// accepted, so that the properties below drive the cascade directly
    /// rather than through the reader.
    #[derive(Debug, Clone)]
    enum Op {
        Set(&'static str, Value),
        Unset(&'static str),
    }

    impl Op {
        fn apply(&self, cascade: &mut Cascade) {
            match self {
                Self::Set(property, value) => cascade.set(property, value.clone()),
                Self::Unset(property) => cascade.unset(property),
            }
        }

        /// Whether this is a border declaration, after which all four sides
        /// are written.
        fn is_border(&self) -> bool {
            matches!(
                self,
                Self::Set(
                    _,
                    Value::BorderSide(_) | Value::Widths(_) | Value::Styles(_) | Value::Colours(_)
                )
            )
        }

        /// The stored property this call sets or unsets, `None` for a border
        /// declaration.
        fn stored(&self) -> Option<&'static str> {
            match self {
                _ if self.is_border() => None,
                Self::Set(property, _) | Self::Unset(property) => Some(stored_name(property)),
            }
        }
    }

    fn arb_border_op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (prop::sample::select(SIDE_SHORTHANDS), arb_border_side())
                .prop_map(|(property, side)| Op::Set(property, Value::BorderSide(side))),
            prop::collection::vec(arb_border_width(), 1..=4)
                .prop_map(|widths| Op::Set("border-width", Value::Widths(widths))),
            prop::collection::vec(prop::sample::select(BORDER_STYLES), 1..=4)
                .prop_map(|styles| Op::Set("border-style", Value::Styles(styles))),
            prop::collection::vec(arb_colour(), 1..=4)
                .prop_map(|colours| Op::Set("border-color", Value::Colours(colours))),
        ]
    }

    fn arb_cell_op() -> impl Strategy<Value = Op> {
        prop_oneof![
            arb_border_op(),
            (prop::sample::select(&["width", "height"][..]), 1..=2000_u32)
                .prop_map(|(property, px)| Op::Set(property, Value::Px(Hundredths::whole(px)))),
            prop::sample::select(&["left", "center", "justify"][..])
                .prop_map(|keyword| Op::Set("text-align", Value::Keyword(keyword))),
            (
                prop::sample::select(&["background", "background-color"][..]),
                arb_colour()
            )
                .prop_map(|(property, colour)| Op::Set(property, Value::Colour(colour))),
            Just(Op::Set("box-sizing", Value::Locked("content-box"))),
            prop::sample::select(UNSET_PROPERTIES).prop_map(Op::Unset),
        ]
    }

    fn cascade_of(ops: &[Op]) -> Cascade {
        let mut cascade = Cascade::default();
        for op in ops {
            op.apply(&mut cascade);
        }
        cascade
    }

    fn td() -> &'static ElementStyle {
        element_style("td").expect("a grammar")
    }

    proptest! {
        // `from_shorthand` keeps every part the shorthand gave and fills each
        // part it left out with CSS's initial value.
        #[test]
        fn a_shorthand_side_keeps_its_parts_and_fills_the_rest(side in arb_border_side()) {
            let written = Side::from_shorthand(side);
            prop_assert_eq!(written.width, side.width.unwrap_or(BORDER_WIDTH_INITIAL));
            prop_assert_eq!(written.style, side.style.unwrap_or("none"));
            prop_assert_eq!(written.colour, side.colour.unwrap_or(Colour::CurrentColor));
        }

        // Whatever sequence of `set` and `unset` calls came first, `write`
        // gives a sorted string with each name once, four sides exactly when
        // a border declaration came, `box-sizing` exactly when the cell is
        // sized, every side `none`, `hidden` or a triple, inside the
        // alphabet, and a fixed point of the whole filter.
        #[test]
        fn a_cascade_writes_a_canonical_cell_style(ops in prop::collection::vec(arb_cell_op(), 0..12)) {
            let borders = ops.iter().any(Op::is_border);
            let Some(css) = cascade_of(&ops).write(td()) else {
                prop_assert!(!borders);
                return Ok(());
            };
            let names: Vec<&str> = pairs(&css).map(|(name, _)| name).collect();
            prop_assert!(names.is_sorted(), "{}", css);
            prop_assert!(names.windows(2).all(|pair| pair[0] != pair[1]), "{}", css);
            let sides = names.iter().filter(|name| SIDES.contains(name)).count();
            prop_assert_eq!(sides, if borders { 4 } else { 0 }, "{}", css);
            let sized = names.contains(&"width") || names.contains(&"height");
            prop_assert_eq!(names.contains(&"box-sizing"), sized, "{}", css);
            for (name, value) in pairs(&css) {
                if SIDES.contains(&name) {
                    let parts: Vec<&str> = value.split(' ').collect();
                    prop_assert!(matches!(parts.as_slice(), ["none" | "hidden"] | [_, _, _]), "{}", value);
                }
            }
            prop_assert!(in_alphabet(&css), "{}", css);
            prop_assert_eq!(sanitize_style("td", &css).css, Some(css.clone()));
        }

        // `unset` leaves the property out, and the result is the one the
        // calls give without any declaration of that property.
        #[test]
        fn unset_forgets_every_earlier_declaration_of_its_property(
            ops in prop::collection::vec(arb_cell_op(), 0..10),
            property in prop::sample::select(&["width", "text-align", "background-color"][..]),
        ) {
            let mut cascade = cascade_of(&ops);
            cascade.unset(property);
            let written = cascade.write(td());
            prop_assert!(!written.as_deref().is_some_and(|css| declares(css, property)));
            let others: Vec<Op> = ops.into_iter().filter(|op| op.stored() != Some(property)).collect();
            prop_assert_eq!(written, cascade_of(&others).write(td()));
        }

        // A later `border` or `border-<side>` replaces every part of the
        // sides it names, whatever came before: its own parts, and CSS's
        // initial value for each one it leaves out (a width reset to 3 px).
        #[test]
        fn a_later_side_shorthand_replaces_its_sides_whole(
            ops in prop::collection::vec(arb_cell_op(), 0..8),
            property in prop::sample::select(SIDE_SHORTHANDS),
            side in arb_border_side(),
        ) {
            let mut cascade = cascade_of(&ops);
            cascade.set(property, Value::BorderSide(side));
            let css = cascade.write(td()).expect("a border declaration writes four sides");
            let expected = Side::from_shorthand(side).to_string();
            for &index in sides_named(property) {
                let value = pairs(&css).find(|&(name, _)| name == SIDES[index]).map(|(_, value)| value);
                prop_assert_eq!(value, Some(expected.as_str()), "{}", css);
            }
        }

        // `border-width`, `border-style` and `border-color` set one part of
        // each side; a width that none of them set is the page's 0 px, never
        // CSS's initial 3 px, so a style alone draws no border.
        #[test]
        fn a_width_that_no_declaration_set_is_zero(
            ops in prop::collection::vec(prop_oneof![
                prop::collection::vec(prop::sample::select(BORDER_STYLES), 1..=4)
                    .prop_map(|styles| Op::Set("border-style", Value::Styles(styles))),
                prop::collection::vec(arb_colour(), 1..=4)
                    .prop_map(|colours| Op::Set("border-color", Value::Colours(colours))),
            ], 1..6),
        ) {
            let css = cascade_of(&ops).write(td()).expect("a border declaration writes four sides");
            for (name, value) in pairs(&css) {
                prop_assert!(SIDES.contains(&name), "{}", css);
                prop_assert!(matches!(value, "none" | "hidden") || value.starts_with("0px "), "{}", css);
            }
        }
        // The written string is sorted, names each property once, keeps no
        // shorthand, and has `box-sizing` exactly when it has a size.
        #[test]
        fn cell_styles_come_out_canonical(element in prop::sample::select(&["td", "th"][..]), style in arb_cell_style()) {
            let rules = element_style(element).expect("a grammar");
            if let Some(css) = css(element, &style) {
                let names: Vec<&str> = pairs(&css).map(|(name, _)| name).collect();
                prop_assert!(names.is_sorted(), "{}", css);
                prop_assert!(names.windows(2).all(|pair| pair[0] != pair[1]), "{}", css);
                for name in &names {
                    let rule = rules.property(name);
                    prop_assert!(rule.is_some_and(|rule| rule.role == Role::Stored), "{}", css);
                }
                let sides = names.iter().filter(|name| SIDES.contains(name)).count();
                prop_assert!(sides == 0 || sides == 4, "{}", css);
                let sized = names.contains(&"width") || names.contains(&"height");
                prop_assert_eq!(names.contains(&"box-sizing"), sized, "{}", css);
                for (name, value) in pairs(&css) {
                    if SIDES.contains(&name) {
                        let parts: Vec<&str> = value.split(' ').collect();
                        prop_assert!(matches!(parts.as_slice(), ["none" | "hidden"] | [_, _, _]), "{}", value);
                    }
                }
            }
        }

        // A side's spelling depends only on what it renders: the same three
        // longhands, reached through any shorthand, give the same string.
        #[test]
        fn a_side_is_spelled_the_same_through_every_shorthand(
            width in prop::sample::select(&["1px", "0.75pt", "thin", "medium", "3px", "0"][..]),
            style in prop::sample::select(&["solid", "dashed", "none", "hidden", "double"][..]),
            colour in prop::sample::select(&["red", "#ff0000", "rgb(255, 0, 0)", "currentcolor"][..]),
        ) {
            let side = css("td", &format!("border: {width} {style} {colour}"));
            let longhands = css("td", &format!("border-width: {width}; border-style: {style}; border-color: {colour}"));
            let each = css("td", &SIDES.iter().map(|side| format!("{side}: {colour} {style} {width}")).collect::<Vec<_>>().join("; "));
            prop_assert_eq!(&side, &longhands);
            prop_assert_eq!(&side, &each);
        }

        #[test]
        fn removing_a_declaration_keeps_the_string_canonical(
            parts in prop::collection::vec(prop::sample::select(&[
                "color: red", "background: yellow", "font-size: 1.5em", "background-color: #000", "font-size: 3em",
            ][..]), 1..6),
            property in prop::sample::select(&["font-size", "color", "background-color"][..]),
        ) {
            let Some(written) = css("span", &parts.join("; ")) else { return Ok(()) };
            let removed = without(&written, property);
            prop_assert!(!removed.as_deref().is_some_and(|css| declares(css, property)));
            prop_assert_eq!(declares(&written, property), removed.as_deref() != Some(written.as_str()));
            if let Some(removed) = removed {
                prop_assert_eq!(css("span", &removed), Some(removed.clone()));
            }
        }

        #[test]
        fn declares_finds_exactly_the_written_names(style in arb_cell_style(), name in prop::sample::select(CELL_PROPERTIES)) {
            if let Some(css) = css("td", &style) {
                prop_assert_eq!(declares(&css, name), css.split("; ").any(|d| d.starts_with(&format!("{name}: "))));
            }
        }

        #[test]
        fn the_alphabet_guard_matches_its_definition(text in any::<String>()) {
            let allowed = "abcdefghijklmnopqrstuvwxyz0123456789 #.%:;/-";
            prop_assert_eq!(in_alphabet(&text), text.chars().all(|c| allowed.contains(c)));
        }

        #[test]
        fn four_repeats_what_it_is_given(values in prop::collection::vec(any::<u8>(), 0..6)) {
            let expanded = four(&values);
            prop_assert_eq!(expanded.is_some(), (1..=4).contains(&values.len()));
            if let Some(expanded) = expanded {
                prop_assert!(expanded.iter().all(|value| values.contains(value)));
                prop_assert_eq!(expanded[0], values[0]);
            }
        }
    }
}
