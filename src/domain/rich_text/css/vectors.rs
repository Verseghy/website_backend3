//! The cases of `css-vectors.jsonl`, the file that the server's grammar and
//! the admin's `canonicalStyle` must agree on (editor.md §3.1, §3.9 and
//! §13.6, "Differential CSS").
//!
//! The two canonicalisers are the same rules written twice, in Rust and in
//! TypeScript. Rust is the reference: [`css_vectors`] runs every case of
//! [`CASES`] through [`super::sanitize_style`] and records what it wrote,
//! and the TypeScript side must write the same for every line. The
//! `rich_text_manifest` task writes the file from [`css_vectors_jsonl`]
//! (editor.md §3.9), and `tests/rich_text/css_vectors.rs` keeps a snapshot
//! of it, so a change to any output shows up in review.
//!
//! The cases cover every property of every element of editor.md §3.3 with an
//! accepted and a rejected value, each rule of §3.1, each row of §3.4, the
//! reader rules of §7.3 that the TypeScript side has to mirror (comments,
//! `!important`, the component and declaration limits, numbers read from
//! their source text, the `rgb()` ranges), and the CSS pool of §13.6. They
//! are invented, like every fixture (`CLAUDE.md`, public-safety rules).
//!
//! The file is deterministic: the cases are a constant list in a fixed
//! order, the grammar reads no hash map, and each line is written with the
//! keys in one order.

use serde::Serialize;

use super::sanitize_style;

/// One line of `css-vectors.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Vector {
    /// The element whose `style` attribute this is.
    pub element: &'static str,
    /// The attribute value, with character references already decoded.
    pub input: &'static str,
    /// The canonical style string, or `None` (`null` in the file) when
    /// nothing is left and the attribute is removed.
    pub output: Option<String>,
}

/// The `(element, input)` pairs of the vector file, in its order.
#[rustfmt::skip]
pub const CASES: &[(&str, &str)] = &[
    // editor.md §3.7: the canonical shapes are fixed points.
    ("h2", "text-align: center"),
    ("span", "color: #c00000; font-size: 1.25em"),
    ("ul", "list-style-type: circle"),
    ("table", "border-collapse: collapse; max-width: fit-content"),
    ("tbody", "display: table-row-group"),
    ("td", "background-color: #d9d9d9; border-bottom: 1px solid #000000; border-left: none; border-right: 1px solid #000000; border-top: 1px solid #000000; box-sizing: content-box; height: 20px; text-align: center; vertical-align: middle; white-space: nowrap; width: 128px"),
    ("img", "display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px"),
    ("iframe", "aspect-ratio: 16 / 9; height: auto; max-width: 640px; width: 100%"),
    ("iframe", "height: 608px; width: 100%"),
    // §3.1 rule 1: shorthands expanded in source order with the cascade.
    ("td", "border: 1px solid #000"),
    ("td", "border: solid"),
    ("td", "border: none"),
    ("td", "border: 0"),
    ("td", "border: solid windowtext 1.0pt"),
    ("th", "border: thin dashed red"),
    ("td", "border-top: 1px solid black"),
    ("td", "border-top: 1px solid; border: none; border-top: 2px dashed"),
    ("td", "border-bottom: 1px solid red; border: 2px dotted"),
    ("td", "border: 1px solid; border-right: none; border-left: hidden"),
    ("td", "border-width: 1px 2px; border-style: solid; border-color: red green blue"),
    ("td", "border-style: solid dashed dotted double; border-width: thin"),
    ("td", "border: 1px solid #000; border-width: 0 2px"),
    ("td", "border: 1px solid #000; border-color: #fff #000 #fff #000"),
    ("td", "border-width: 1px"),
    ("td", "border-style: solid"),
    ("td", "border-color: red"),
    ("td", "border-style: hidden none"),
    // A part that no declaration set is the page's (width 0); a later
    // `border-<side>` resets every part it leaves out, an earlier width too.
    ("td", "border-style: solid; border-color: red"),
    ("td", "border-top: 1px solid; border-style: dashed"),
    ("td", "border: 1px solid red; border-top: dashed"),
    ("td", "border-width: 1px; border-top: solid"),
    ("td", "border: 1px solid; border: 1px wavy"),
    ("td", "border: 1px solid; border-top: 21px solid"),
    ("td", "border-top: none; border-right: none; border-bottom: none; border-left: none"),
    ("td", "background: #D9D9D9"),
    ("td", "background-color: red; background: blue"),
    ("td", "background: blue; background-color: red"),
    ("td", "background: blue; background: url(x)"),
    ("span", "background: yellow; color: red"),
    // §3.1 rule 2: border sides.
    ("td", "border-top: 1px none red"),
    ("td", "border-top: 2px hidden red"),
    ("td", "border-top: medium"),
    ("td", "border-top: medium none currentcolor"),
    ("td", "border-top: solid"),
    ("td", "border-top: thick double"),
    ("td", "border-top: .5pt solid"),
    ("td", "border-top: 0 solid"),
    ("td", "border-top: #000 SOLID .75pt"),
    ("td", "border-top: 20px dashed red"),
    ("td", "border-top: 15pt dotted"),
    ("td", "border-top: 1px solid transparent"),
    ("td", "border-top: 1px solid rgba(0, 0, 0, .5)"),
    ("td", "border-top: groove"),
    ("td", "border-right: 1px ridge #808080"),
    ("td", "border-right: 1px 1px"),
    ("th", "border-bottom: wavy"),
    ("td", "border-left: 21px"),
    ("th", "border-bottom: inset 2px"),
    ("th", "border-left: outset 2px"),
    ("td", "border-top: 21px solid"),
    ("td", "border-top: -1px solid"),
    ("td", "border-top: 1px solid red blue"),
    ("td", "border-top: 1px 2px"),
    ("td", "border-top: solid dashed"),
    ("td", "border-top: 1% solid"),
    ("td", "border-top: 1em solid"),
    ("td", "border-top: wavy"),
    ("td", "border-top: inherit"),
    ("td", "border-width: 1px 2px 3px 4px 5px"),
    ("td", "border-style: solid dashed dotted double groove"),
    ("td", "border-color: 1px"),
    // §3.1 rule 3: colours.
    ("span", "color: red"),
    ("span", "color: RED"),
    ("span", "color: whitesmoke"),
    ("span", "color: rebeccapurple"),
    ("span", "color: #C00"),
    ("span", "color: #c00000ff"),
    ("span", "color: windowtext"),
    ("span", "color: rgb(192, 0, 0)"),
    ("span", "color: rgb(100%, 0%, 0%)"),
    ("span", "color: rgb(192 0 0)"),
    ("span", "color: rgb(192 0 0 / 100%)"),
    ("span", "color: RGBA(192, 0, 0, 1)"),
    ("span", "color: rgb(50.5%, 0%, 0%)"),
    ("span", "color: transparent"),
    ("span", "color: currentcolor"),
    ("span", "color: rgba(192, 0, 0, 0.5)"),
    ("span", "color: #c0000080"),
    ("span", "color: #c008"),
    ("span", "color: rgb(256, 0, 0)"),
    ("span", "color: rgb(-1, 0, 0)"),
    ("span", "color: rgb(101%, 0%, 0%)"),
    ("span", "color: rgb(1, 2%, 3)"),
    ("span", "color: rgb(1 2 3 / 2)"),
    ("span", "color: hsl(0, 100%, 50%)"),
    ("span", "color: buttontext"),
    ("span", "color: #12345"),
    ("span", "background-color: rgba(0, 0, 0, 0.7)"),
    ("span", "background-color: rgba(255, 255, 0, .5)"),
    ("span", "background-color: transparent"),
    ("span", "background-color: rgb(0 0 0 / 50%)"),
    ("span", "background-color: rgb(0 0 0 / 0)"),
    ("span", "background-color: 1px"),
    ("span", "background: yellow no-repeat"),
    ("td", "background-color: silver"),
    ("td", "background-color: currentcolor"),
    ("td", "background-color: #d9d9d9 #000"),
    ("td", "background: #d9d9d9 no-repeat"),
    ("td", "background: none"),
    // §3.1 rule 4: lengths.
    ("td", "width: 128px"),
    ("td", "width: 96pt"),
    ("td", "width: 1.0cm"),
    ("td", "width: 10mm"),
    ("td", "width: 1in"),
    ("td", "width: 6pc"),
    ("td", "width: 40q"),
    ("td", "width: 10.004px"),
    ("td", "width: 10.005px"),
    ("td", "width: 0.015in"),
    ("td", "width: 20.00px"),
    ("td", "width: 1px"),
    ("td", "width: 2000px"),
    ("td", "width: 0.99px"),
    ("td", "width: 2000.01px"),
    ("td", "width: 0"),
    ("td", "width: 100%"),
    ("td", "width: 100%px"),
    ("td", "width: auto"),
    ("td", "width: -1px"),
    ("td", "width: 1e30px"),
    ("td", "width: 1e3px"),
    ("td", "width: 10PX"),
    ("td", "width: 10"),
    ("th", "height: 20px"),
    ("th", "height: 2001px"),
    ("img", "width: 262.5pt"),
    ("img", "width: 2000px"),
    ("img", "width: 2001px"),
    ("img", "width: 50%"),
    ("iframe", "max-width: 200px"),
    ("iframe", "max-width: 1280px"),
    ("iframe", "max-width: 199px"),
    ("iframe", "max-width: 1281px"),
    ("iframe", "max-width: 100%"),
    ("iframe", "height: auto"),
    ("iframe", "height: 100px"),
    ("iframe", "height: 2000px"),
    ("iframe", "height: 99px"),
    ("iframe", "height: 50%"),
    ("span", "font-size: 0.5em"),
    ("span", "font-size: 3EM"),
    ("span", "font-size: 1.255em"),
    ("span", "font-size: 0.49em"),
    ("span", "font-size: 3.01em"),
    ("span", "font-size: 12pt"),
    ("span", "font-size: 120%"),
    ("span", "font-size: large"),
    ("span", "font-size: -1em"),
    // §3.1 rule 4: locked values compared after normalisation.
    ("img", "margin: 0px auto"),
    ("img", "margin: 0PT AUTO"),
    ("img", "margin: auto"),
    ("img", "margin: 10px auto"),
    ("img", "max-width: 100.00%"),
    ("img", "max-width: 700px"),
    ("img", "height: 495px"),
    ("img", "display: inline"),
    ("iframe", "width: 100%"),
    ("iframe", "width: 640px"),
    ("iframe", "aspect-ratio: 16/9"),
    ("iframe", "aspect-ratio: 4 / 3"),
    ("iframe", "aspect-ratio: 16 / 9 / 1"),
    ("table", "border-collapse: separate"),
    ("table", "max-width: 100%"),
    ("table", ""),
    ("tbody", "display: table"),
    ("tbody", ""),
    // §3.1 rule 5: order, last occurrence, `!important`, empty results.
    ("td", "width: 10px; vertical-align: top; text-align: left; white-space: nowrap; height: 2px; background-color: red; border-left: none"),
    ("span", "font-size: 1.5em; color: red; background-color: yellow"),
    ("img", "width: 350px; max-width: 100%; margin: 0 auto; height: auto; display: block"),
    ("iframe", "width: 100%; max-width: 640px; height: auto; aspect-ratio: 16 / 9"),
    ("table", "max-width: fit-content; border-collapse: collapse"),
    ("span", "color: red; background-color: blue; color: green"),
    ("span", "color: red; color: hsl(0, 100%, 50%)"),
    ("span", "color: red !important; color: blue"),
    ("span", "color: red !IMPORTANT"),
    ("ul", "list-style-type: square; list-style-type: disc"),
    ("ul", "list-style-type: disc"),
    ("p", ""),
    ("p", " ; ;"),
    ("p", "text-align: center;"),
    ("p", "TEXT-ALIGN: CENTER"),
    // §3.3: the remaining properties of each element.
    ("p", "text-align: justify"),
    ("p", "text-align: middle"),
    ("h1", "text-align: right"),
    ("h3", "text-align: left"),
    ("h4", "text-align: start"),
    ("ul", "list-style-type: decimal"),
    ("ul", "list-style-type: none"),
    ("td", "vertical-align: baseline"),
    ("td", "vertical-align: center"),
    ("td", "white-space: normal"),
    ("td", "white-space: pre"),
    ("th", "text-align: justify"),
    ("td", "text-align: -webkit-center"),
    // §3.3: `box-sizing` exactly when `width` or `height` is present.
    ("td", "box-sizing: content-box"),
    ("td", "box-sizing: content-box; width: 10px"),
    ("td", "box-sizing: CONTENT-BOX; height: 10px"),
    ("td", "box-sizing: border-box; width: 10px"),
    ("td", "box-sizing: content-box; text-align: center"),
    ("td", "width: 100%; box-sizing: content-box"),
    ("th", "height: 1px"),
    // §3.4: rejected properties.
    ("span", "font-family: Calibri"),
    ("span", "font: 11pt Calibri"),
    ("td", "font-weight: bold"),
    ("td", "font-style: italic"),
    ("h2", "font-style: italic"),
    ("td", "text-decoration: underline"),
    ("td", "color: red"),
    ("p", "color: red"),
    ("td", "font-size: 1em"),
    ("td", "padding: 1px"),
    ("td", "padding-left: 1px"),
    ("p", "margin: 0 auto"),
    ("p", "text-indent: 1em"),
    ("p", "line-height: 1.5"),
    ("img", "float: left"),
    ("p", "position: absolute; top: 0; left: 0"),
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
    ("p", "behavior: url(x.htc)"),
    ("p", "-moz-binding: url(x.xml#y)"),
    ("td", "mso-number-format: General"),
    ("td", "-webkit-text-stroke: 1px"),
    ("td", "--x: red"),
    // §7.3: the reader rules.
    ("span", "font-family: Calibri; COLOR: Red !important; mso-ansi-language: HU; font-size: 1.5EM"),
    ("td", "width: 96pt; color: red; /* x */; vertical-align: bottom"),
    ("span", "color: red /* x */"),
    ("span", "/* x */ color: red"),
    ("span", "color: /* x */ red"),
    ("span", "color: red; background: u\\72l(x)"),
    ("span", "color: \"red\""),
    ("span", "color: 'red'"),
    ("span", "color: red; x"),
    ("span", "color red; background-color: blue"),
    ("span", "color: red;; background-color: blue"),
    ("span", "color:red;background-color:blue"),
    ("td", "border-width: 1px 1px 1px 1px 1px 1px 1px 1px"),
    ("td", "border-width: 1px 1px 1px 1px 1px 1px 1px 1px 1px"),
    ("td", "border-color: rgb(1, 2, 3), red"),
    ("td", "width: 0.015px"),
    ("td", "width: 1.005px"),
    ("span", "color: rgb(127.5, 0, 0)"),
    ("span", "background-color: rgba(0, 0, 0, 0.502)"),
    ("p", concat!(
        "text-align: left; text-align: left; text-align: left; text-align: left; ",
        "text-align: left; text-align: left; text-align: left; text-align: left; ",
        "text-align: left; text-align: left; text-align: left; text-align: left; ",
        "text-align: left; text-align: left; text-align: left; text-align: left; ",
        "text-align: left; text-align: left; text-align: left; text-align: left; ",
        "text-align: left; text-align: left; text-align: left; text-align: left; ",
        "text-align: left; text-align: left; text-align: left; text-align: left; ",
        "text-align: left; text-align: left; text-align: left; text-align: center; ",
        "text-align: right",
    )),
    // editor.md §13.6: the CSS pool.
    ("td", "background: url(https://evil.example/x)"),
    ("td", "background-image: URL(\"x\")"),
    ("td", "background: url( x )"),
    ("td", "width: expression(alert(1))"),
    ("td", "width: var(--x)"),
    ("td", "width: calc(1px + 2px)"),
    ("span", "color: attr(x)"),
    ("td", "width: env(x)"),
    ("span", "color: #ff0000 !important"),
    ("span", "content: \"x\""),
    ("span", "color: red; }"),
    ("span", "color: (red"),
    ("span", "color: [red]"),
    ("span", "color: red); background-color: blue"),
    ("span", "@import url(x); color: red"),
    ("span", "color: réd"),
    ("span", "color: red\u{0}"),
    ("td", "width: .5pt"),
    ("td", "width: 15.0pt"),
    // Elements without a style grammar keep nothing.
    ("li", "text-align: center"),
    ("div", "color: red"),
];

/// Runs every case of [`CASES`] through the grammar.
#[must_use]
pub fn css_vectors() -> Vec<Vector> {
    CASES
        .iter()
        .map(|&(element, input)| Vector {
            element,
            input,
            output: sanitize_style(element, input).css,
        })
        .collect()
}

/// `css-vectors.jsonl`: one JSON object per line, `{"element", "input",
/// "output"}` in that key order, each line ending in `\n`.
///
/// # Panics
///
/// Never: a [`Vector`] holds only strings, which always serialise.
#[must_use]
pub fn css_vectors_jsonl() -> String {
    css_vectors()
        .iter()
        .map(|vector| {
            // A struct of strings always serialises.
            let mut line = serde_json::to_string(vector).expect("a vector serialises");
            line.push('\n');
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::css::canonical::in_alphabet;
    use crate::domain::rich_text::css::declarations;
    use crate::domain::rich_text::css::grammar::{ELEMENT_STYLES, Reading, element_style, parse};
    use std::collections::BTreeSet;

    #[test]
    fn every_case_is_listed_once() {
        let mut seen = BTreeSet::new();
        for case in CASES {
            assert!(seen.insert(case), "{case:?} is listed twice");
        }
    }

    /// Every property of every element has an input that the grammar
    /// accepts and one it rejects, so the TypeScript side is checked on
    /// both sides of each row of editor.md §3.3.
    #[test]
    fn every_property_is_covered_both_ways() {
        for style in ELEMENT_STYLES {
            for property in style.properties {
                let readings: Vec<bool> = CASES
                    .iter()
                    .filter(|(element, _)| {
                        element_style(element)
                            .is_some_and(|other| other.properties == style.properties)
                    })
                    .flat_map(|(_, input)| declarations::read(input).read)
                    .filter(|declaration| declaration.name == property.name)
                    .map(|declaration| {
                        matches!(
                            parse(property.grammar, &declaration.value),
                            Reading::Accepted(_) | Reading::Default
                        )
                    })
                    .collect();
                assert!(
                    readings.contains(&true),
                    "{} {}: no accepted case",
                    style.element,
                    property.name
                );
                assert!(
                    readings.contains(&false),
                    "{} {}: no rejected case",
                    style.element,
                    property.name
                );
            }
        }
    }

    #[test]
    fn every_element_with_a_style_and_one_without_appear() {
        for style in ELEMENT_STYLES {
            assert!(
                CASES.iter().any(|(element, _)| *element == style.element),
                "{}",
                style.element
            );
        }
        assert!(
            CASES
                .iter()
                .any(|(element, _)| element_style(element).is_none())
        );
    }

    /// Every output is canonical: a fixed point (P1c) inside the alphabet.
    #[test]
    fn every_output_is_a_canonical_fixed_point() {
        for vector in css_vectors() {
            if let Some(output) = &vector.output {
                assert!(in_alphabet(output), "{vector:?}");
                let again = sanitize_style(vector.element, output);
                assert_eq!(again.css.as_deref(), Some(output.as_str()), "{vector:?}");
                assert_eq!(again.rejected, 0, "{vector:?}");
            }
        }
    }

    #[test]
    fn the_file_has_one_line_per_case_in_order() {
        let file = css_vectors_jsonl();
        assert!(file.ends_with('\n'));
        let lines: Vec<&str> = file.lines().collect();
        assert_eq!(lines.len(), CASES.len());
        for (line, vector) in lines.iter().zip(css_vectors()) {
            let parsed: serde_json::Value = serde_json::from_str(line).expect("a JSON line");
            assert_eq!(
                parsed,
                serde_json::json!({
                    "element": vector.element,
                    "input": vector.input,
                    "output": vector.output,
                })
            );
            assert!(line.starts_with(r#"{"element":"#), "{line}");
            assert!(!line.contains('\n'));
        }
        // A removed attribute is `null`, not an empty string.
        assert!(file.contains(r#""input":"","output":null}"#));
    }

    #[test]
    fn the_file_is_deterministic() {
        assert_eq!(css_vectors_jsonl(), css_vectors_jsonl());
        assert_eq!(css_vectors(), css_vectors());
    }
}
