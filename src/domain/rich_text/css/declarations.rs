//! Reads a `style` attribute into declarations with the cssparser 0.38
//! tokenizer that ammonia already depends on (editor.md §7.3).
//!
//! This is the token-level half of the grammar: it splits the attribute into
//! declarations, reads each value component by component, and refuses every
//! token that no content-v1 property can use. The property-level half, which
//! decides what a component means for one property of one element, is
//! [`super::grammar`].
//!
//! The rules, in the order they apply:
//!
//! - **Recovery is per declaration.** A declaration that cannot be read is
//!   skipped up to and including the next top-level `;`, as a browser does;
//!   a `;` inside a block or a function does not end it.
//! - **Names** are identifiers, ASCII-lower-cased, followed by `:`.
//! - **Values** are identifiers, numbers, dimensions, percentages, hashes,
//!   `/`, and the functions `rgb()` and `rgba()` with numeric arguments.
//!   Every other function (`url()`, `expression()`, `var()`, `calc()`,
//!   `attr()`, `env()`, `hsl()`, ...), every block, string, unquoted URL, bad
//!   token, at-keyword, comma and other delimiter makes the declaration
//!   unreadable.
//! - **Comments** make the declaration they are in unreadable. A browser
//!   skips a comment between two tokens, but no content-v1 writer emits one,
//!   and refusing it keeps the rule simple to mirror in TypeScript and free
//!   of the cases where a comment separates tokens that would otherwise run
//!   together (`1px/**/solid`).
//! - **`!important`** at the end of a value is stripped. A `!` anywhere else
//!   makes the declaration unreadable.
//! - **Limits.** At most [`MAX_DECLARATIONS`] non-empty declarations are
//!   read; the rest are counted and dropped. A value of more than
//!   [`MAX_COMPONENTS`] components is unreadable, so one long declaration
//!   costs no more than tokenizing it.
//! - **Numbers** are taken from their source text, not from cssparser's
//!   `f32`, and parsed as `f64` with Rust's correctly rounded parser. The
//!   admin's TypeScript canonicaliser is to parse the same text with
//!   `Number()`, which rounds the same way, so both sides start from the
//!   same value (editor.md §3.1, "the same rules written twice").

use cssparser::{Delimiter, ParseError, Parser, SourcePosition, Token};

/// The most declarations one `style` attribute may hold; later ones are
/// dropped (editor.md §7.3).
pub const MAX_DECLARATIONS: usize = 32;

/// The most components one value may hold. No content-v1 grammar needs more
/// than four (`border-width: 1px 2px 3px 4px`), so a longer value is refused
/// before it is interpreted.
pub const MAX_COMPONENTS: usize = 8;

/// The most arguments one `rgb()` or `rgba()` call may hold: three channels,
/// an alpha value and their three separators.
const MAX_ARGUMENTS: usize = 7;

/// One argument of an `rgb()` or `rgba()` call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Argument {
    /// A number, such as `255` or `0.5`.
    Number(f64),
    /// A percentage, as the number before the `%`.
    Percentage(f64),
    /// `,`, the separator of the legacy syntax.
    Comma,
    /// `/`, the separator before the alpha value of the modern syntax.
    Slash,
}

/// One component of a declaration value.
#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    /// An identifier, ASCII-lower-cased: CSS keywords match regardless of
    /// ASCII case.
    Ident(String),
    /// A number without a unit.
    Number(f64),
    /// A number with a unit; the unit is ASCII-lower-cased.
    Dimension {
        /// The number.
        value: f64,
        /// The unit, such as `px` or `em`.
        unit: String,
    },
    /// A percentage, as the number before the `%`.
    Percentage(f64),
    /// A hash token such as `#c00000`, without the `#`, as written.
    Hash(String),
    /// An `rgb()` or `rgba()` call with its arguments.
    Rgb(Vec<Argument>),
    /// `/`, as in `aspect-ratio: 16 / 9`.
    Slash,
}

/// One declaration that could be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// The property name, ASCII-lower-cased.
    pub name: String,
    /// The value's components, without whitespace and without `!important`.
    pub value: Vec<Component>,
}

/// What reading one `style` attribute gave.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Declarations {
    /// The declarations that could be read, in source order.
    pub read: Vec<Declaration>,
    /// How many non-empty declarations could not be read, or came after the
    /// first [`MAX_DECLARATIONS`].
    pub unreadable: usize,
}

/// Reads a `style` attribute.
///
/// - `style`: the attribute value, with character references already
///   decoded (as ammonia passes it to the attribute filter).
#[must_use]
pub fn read(style: &str) -> Declarations {
    let mut input = Parser::new(style);
    let mut declarations = Declarations::default();
    while !at_end(&mut input) {
        let chunk = input.parse_until_after(Delimiter::Semicolon, declaration);
        let counted = declarations.read.len() + declarations.unreadable;
        match chunk {
            // Whitespace between two `;` is no declaration.
            Ok(None) => {}
            Ok(Some(declaration)) if counted < MAX_DECLARATIONS => {
                declarations.read.push(declaration);
            }
            Ok(Some(_)) | Err(_) => declarations.unreadable += 1,
        }
    }
    declarations
}

/// Reads a bare value, such as the locked value `0 auto`, with the same
/// rules as a declaration's value.
///
/// - `value`: the value text, without a property name.
#[must_use]
pub fn read_value(value: &str) -> Option<Vec<Component>> {
    Parser::new(value).parse_entirely(components).ok()
}

/// Whether nothing but the end of the input is left. Whitespace and comments
/// still count as input, so a trailing comment is an unreadable declaration
/// rather than silently ignored.
fn at_end(input: &mut Parser<'_>) -> bool {
    let state = input.state();
    let end = input.next_including_whitespace_and_comments().is_err();
    input.reset(&state);
    end
}

fn reject() -> ParseError<()> {
    ParseError::custom(())
}

/// The next token that is not whitespace, with its start position, or `None`
/// at the end of the current declaration or block.
fn next_significant<'i>(
    input: &mut Parser<'i>,
) -> Result<Option<(SourcePosition, Token<'i>)>, ParseError<()>> {
    loop {
        let start = input.position();
        match input.next_including_whitespace_and_comments() {
            // Only the end of the delimited input is reported as an error.
            Err(_) => return Ok(None),
            Ok(Token::WhiteSpace(_)) => {}
            Ok(Token::Comment(_)) => return Err(reject()),
            Ok(token) => return Ok(Some((start, token.clone()))),
        }
    }
}

/// One declaration, up to the next top-level `;`: `Ok(None)` when it is
/// empty.
fn declaration(input: &mut Parser<'_>) -> Result<Option<Declaration>, ParseError<()>> {
    let Some((_, token)) = next_significant(input)? else {
        return Ok(None);
    };
    let Token::Ident(name) = token else {
        return Err(reject());
    };
    let Some((_, Token::Colon)) = next_significant(input)? else {
        return Err(reject());
    };
    let value = components(input)?;
    Ok(Some(Declaration {
        name: name.to_ascii_lowercase(),
        value,
    }))
}

/// The components of a value, up to its end or a trailing `!important`.
fn components(input: &mut Parser<'_>) -> Result<Vec<Component>, ParseError<()>> {
    let mut value = Vec::new();
    while let Some((start, token)) = next_significant(input)? {
        let component = match token {
            Token::Ident(ident) => Component::Ident(ident.to_ascii_lowercase()),
            Token::Number { .. } => Component::Number(number(input.slice_from(start))?),
            Token::Percentage { .. } => Component::Percentage(number(input.slice_from(start))?),
            Token::Dimension { ref unit, .. } => Component::Dimension {
                value: number(input.slice_from(start))?,
                unit: unit.to_ascii_lowercase(),
            },
            Token::Hash(ref hash) | Token::IDHash(ref hash) => Component::Hash(hash.to_string()),
            Token::Delim('/') => Component::Slash,
            Token::Delim('!') => {
                important(input)?;
                break;
            }
            Token::Function(ref function)
                if function.eq_ignore_ascii_case("rgb")
                    || function.eq_ignore_ascii_case("rgba") =>
            {
                Component::Rgb(input.parse_nested_block(arguments)?)
            }
            _ => return Err(reject()),
        };
        value.push(component);
        if value.len() > MAX_COMPONENTS {
            return Err(reject());
        }
    }
    Ok(value)
}

/// After a `!`: the keyword `important` and then nothing but the end.
fn important(input: &mut Parser<'_>) -> Result<(), ParseError<()>> {
    match next_significant(input)? {
        Some((_, Token::Ident(keyword))) if keyword.eq_ignore_ascii_case("important") => {}
        _ => return Err(reject()),
    }
    match next_significant(input)? {
        None => Ok(()),
        Some(_) => Err(reject()),
    }
}

/// The arguments inside `rgb(...)` or `rgba(...)`.
fn arguments(input: &mut Parser<'_>) -> Result<Vec<Argument>, ParseError<()>> {
    let mut arguments = Vec::new();
    while let Some((start, token)) = next_significant(input)? {
        arguments.push(match token {
            Token::Number { .. } => Argument::Number(number(input.slice_from(start))?),
            Token::Percentage { .. } => Argument::Percentage(number(input.slice_from(start))?),
            Token::Comma => Argument::Comma,
            Token::Delim('/') => Argument::Slash,
            _ => return Err(reject()),
        });
        if arguments.len() > MAX_ARGUMENTS {
            return Err(reject());
        }
    }
    Ok(arguments)
}

/// The value of a numeric token, read from its source text.
///
/// - `token`: the token's text, such as `12.5pt`, `+3`, `.5` or `50%`.
fn number(token: &str) -> Result<f64, ParseError<()>> {
    number_prefix(token)
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(reject)
}

/// The leading number of a numeric token, by the CSS Syntax 3 rule
/// (§4.3.12, "consume a number"): a sign, digits, a `.` only when a digit
/// follows, and an exponent only when a digit follows the `e` or its sign.
/// The rest is the unit (`1em` is one `em`, `1e3px` is 1000 `px`).
fn number_prefix(token: &str) -> &str {
    let bytes = token.as_bytes();
    let digit = |index: usize| bytes.get(index).is_some_and(u8::is_ascii_digit);
    let digits_from = |mut index: usize| {
        while digit(index) {
            index += 1;
        }
        index
    };
    let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    end = digits_from(end);
    if bytes.get(end) == Some(&b'.') && digit(end + 1) {
        end = digits_from(end + 1);
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        if digit(end + 1 + sign) {
            end = digits_from(end + 1 + sign);
        }
    }
    &token[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn names(style: &str) -> Vec<String> {
        read(style).read.into_iter().map(|d| d.name).collect()
    }

    fn one(style: &str) -> Option<Vec<Component>> {
        let declarations = read(style);
        match (declarations.read.as_slice(), declarations.unreadable) {
            ([declaration], 0) => Some(declaration.value.clone()),
            _ => None,
        }
    }

    fn ident(name: &str) -> Component {
        Component::Ident(name.into())
    }

    fn px(value: f64) -> Component {
        Component::Dimension {
            value,
            unit: "px".into(),
        }
    }

    #[test]
    fn declarations_are_split_at_top_level_semicolons() {
        let declarations = read("color: red;  ; WIDTH : 10PX;;background-color:#C00000;");
        assert_eq!(declarations.unreadable, 0);
        assert_eq!(
            declarations.read,
            [
                Declaration {
                    name: "color".into(),
                    value: vec![ident("red")]
                },
                Declaration {
                    name: "width".into(),
                    value: vec![px(10.0)]
                },
                Declaration {
                    name: "background-color".into(),
                    value: vec![Component::Hash("C00000".into())]
                },
            ]
        );
        assert_eq!(read(""), Declarations::default());
        assert_eq!(read(" ;; "), Declarations::default());
    }

    #[test]
    fn an_unreadable_declaration_is_skipped_to_the_next_semicolon() {
        let declarations = read("color: url(x); width: 1px; height: (2px; 3px); text-align: left");
        assert_eq!(names("color: url(x); width: 1px"), ["width"]);
        assert_eq!(declarations.unreadable, 2);
        // The `;` inside the parenthesised block does not end the declaration.
        assert_eq!(
            declarations
                .read
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            ["width", "text-align"]
        );
        // An unclosed block swallows the rest of the attribute.
        assert_eq!(names("width: [1px; height: 2px"), Vec::<String>::new());
    }

    #[test]
    fn every_rejected_token_makes_its_declaration_unreadable() {
        for style in [
            "background: url(https://evil.example/x.png)",
            "background: url( 'x' )",
            "background: URL(x)",
            "background: u\\72 l(x)",
            "width: expression(alert(1))",
            "color: var(--x)",
            "width: calc(1px + 2px)",
            "content: attr(title)",
            "width: env(safe-area-inset-left)",
            "color: hsl(0, 100%, 50%)",
            "font-family: \"Calibri\"",
            "font-family: 'Calibri",
            "color: red}",
            "color: red)",
            "color: {red}",
            "@import x",
            "color: @x",
            "font-family: a, b",
            "color: red; x",
            "width: 1px + 2px",
            "color:: red",
            "color red",
            ": red",
            "1px: red",
            "color: <!-- red",
            "color: red -->",
            "color: #",
            "width: 1px.",
            "color: rgb(255, 0, 0, 0, 0, 0, 0, 0)",
            "color: rgb(x)",
            "color: rgb(var(--r), 0, 0)",
            "border: 1px 1px 1px 1px 1px 1px 1px 1px 1px",
        ] {
            let declarations = read(style);
            assert!(
                declarations.unreadable >= 1
                    && declarations.read.iter().all(|d| d.name != "background"
                        && d.name != "width"
                        && d.name != "content"
                        && d.name != "font-family"),
                "{style}: {declarations:?}"
            );
        }
    }

    #[test]
    fn comments_make_their_declaration_unreadable() {
        assert_eq!(read("color: red /* x */").unreadable, 1);
        assert_eq!(read("col/**/or: red").unreadable, 1);
        assert_eq!(read("width: 1px/**/; color: red").read.len(), 1);
        assert_eq!(read("/* x */").unreadable, 1, "a trailing comment counts");
        assert_eq!(read("color: red !/**/important").unreadable, 1);
    }

    #[test]
    fn important_is_stripped_only_at_the_end() {
        assert_eq!(one("color: red !important"), Some(vec![ident("red")]));
        assert_eq!(one("color:red!IMPORTANT  "), Some(vec![ident("red")]));
        assert_eq!(one("color: red ! important"), Some(vec![ident("red")]));
        assert_eq!(one("color: !important"), Some(vec![]));
        for style in [
            "color: red !important x",
            "color: red !imp",
            "color: red !",
            "color: ! red",
            "color: red !important !important",
        ] {
            assert_eq!(read(style).unreadable, 1, "{style}");
        }
    }

    #[test]
    fn components_keep_their_kind() {
        assert_eq!(
            one("border: thin SOLID #ABC"),
            Some(vec![
                ident("thin"),
                ident("solid"),
                Component::Hash("ABC".into())
            ])
        );
        assert_eq!(
            one("aspect-ratio: 16/9"),
            Some(vec![
                Component::Number(16.0),
                Component::Slash,
                Component::Number(9.0)
            ])
        );
        assert_eq!(
            one("x: 12.5PT 100% #1a2 r\\65 d"),
            Some(vec![
                Component::Dimension {
                    value: 12.5,
                    unit: "pt".into()
                },
                Component::Percentage(100.0),
                Component::Hash("1a2".into()),
                ident("red"),
            ])
        );
        assert_eq!(
            one("color: RGBA( 255 , 0,0 ,50% )"),
            Some(vec![Component::Rgb(vec![
                Argument::Number(255.0),
                Argument::Comma,
                Argument::Number(0.0),
                Argument::Comma,
                Argument::Number(0.0),
                Argument::Comma,
                Argument::Percentage(50.0),
            ])])
        );
        assert_eq!(
            one("color: rgb(1 2 3 / .5)"),
            Some(vec![Component::Rgb(vec![
                Argument::Number(1.0),
                Argument::Number(2.0),
                Argument::Number(3.0),
                Argument::Slash,
                Argument::Number(0.5),
            ])])
        );
    }

    #[test]
    fn numbers_come_from_their_source_text() {
        assert_eq!(
            one("x: .5pt"),
            Some(vec![Component::Dimension {
                value: 0.5,
                unit: "pt".into()
            }])
        );
        assert_eq!(one("x: +3"), Some(vec![Component::Number(3.0)]));
        assert_eq!(
            one("x: -0.25em"),
            Some(vec![Component::Dimension {
                value: -0.25,
                unit: "em".into()
            }])
        );
        assert_eq!(one("x: 1e3px"), Some(vec![px(1000.0)]));
        assert_eq!(one("x: 2E-2PX"), Some(vec![px(0.02)]));
        assert_eq!(
            one("x: 1em"),
            Some(vec![Component::Dimension {
                value: 1.0,
                unit: "em".into()
            }])
        );
        assert_eq!(
            one("x: 1e+1em"),
            Some(vec![Component::Dimension {
                value: 10.0,
                unit: "em".into()
            }])
        );
        // 0.67 as f32 would be 0.6700000166893005. Every numeric token is
        // read from its text: dimensions, unitless numbers, percentages and
        // the arguments of `rgb()`.
        assert_eq!(one("x: 0.67px"), Some(vec![px(0.67)]));
        assert_eq!(one("x: 0.67"), Some(vec![Component::Number(0.67)]));
        assert_eq!(one("x: 0.67%"), Some(vec![Component::Percentage(0.67)]));
        assert_eq!(
            one("color: rgba(0.67, 0, 0, 0.7)"),
            Some(vec![Component::Rgb(vec![
                Argument::Number(0.67),
                Argument::Comma,
                Argument::Number(0.0),
                Argument::Comma,
                Argument::Number(0.0),
                Argument::Comma,
                Argument::Number(0.7),
            ])])
        );
        assert_eq!(
            one("color: rgb(0 0 0 / 0.67%)"),
            Some(vec![Component::Rgb(vec![
                Argument::Number(0.0),
                Argument::Number(0.0),
                Argument::Number(0.0),
                Argument::Slash,
                Argument::Percentage(0.67),
            ])])
        );
        // Too large for an f64: the declaration cannot be read.
        assert_eq!(read("x: 1e400px").unreadable, 1);
        assert_eq!(number_prefix("12.5p\\74"), "12.5");
        assert_eq!(number_prefix("1.e3"), "1");
        assert_eq!(number_prefix("-.5e-x"), "-.5");
    }

    #[test]
    fn at_most_thirty_two_declarations_are_read() {
        let style = (0..40)
            .map(|i| format!("p{i}: {i}"))
            .collect::<Vec<_>>()
            .join("; ");
        let declarations = read(&style);
        assert_eq!(declarations.read.len(), MAX_DECLARATIONS);
        assert_eq!(
            declarations.read.last().map(|d| d.name.as_str()),
            Some("p31")
        );
        assert_eq!(declarations.unreadable, 8);
        // Unreadable declarations count towards the limit, empty ones do not.
        let style = format!("{};;;; color: red", "x; ".repeat(32));
        let declarations = read(&style);
        assert_eq!((declarations.read.len(), declarations.unreadable), (0, 33));
    }

    #[test]
    fn a_value_holds_at_most_eight_components() {
        let value = |count: usize| vec!["1px"; count].join(" ");
        assert_eq!(
            one(&format!("border-width: {}", value(MAX_COMPONENTS))),
            Some(vec![px(1.0); MAX_COMPONENTS])
        );
        let longer = read(&format!(
            "border-width: {}; color: red",
            value(MAX_COMPONENTS + 1)
        ));
        assert_eq!(longer.unreadable, 1);
        assert_eq!(
            longer.read,
            [Declaration {
                name: "color".into(),
                value: vec![ident("red")]
            }]
        );
        // `!important` is not a component.
        assert_eq!(
            one(&format!("x: {} !important", value(MAX_COMPONENTS))),
            Some(vec![px(1.0); MAX_COMPONENTS])
        );
        // An `rgb()` call is one component, whatever its arguments.
        assert_eq!(
            one(&format!("x: {} rgb(1, 2, 3)", value(MAX_COMPONENTS - 1)))
                .map(|components| components.len()),
            Some(MAX_COMPONENTS)
        );
        assert_eq!(read_value(&value(MAX_COMPONENTS + 1)), None);
    }

    #[test]
    fn read_value_reads_a_bare_value() {
        assert_eq!(
            read_value("0 auto"),
            Some(vec![Component::Number(0.0), ident("auto")])
        );
        assert_eq!(read_value(""), Some(vec![]));
        assert_eq!(
            read_value("16 / 9"),
            Some(vec![
                Component::Number(16.0),
                Component::Slash,
                Component::Number(9.0)
            ])
        );
        assert_eq!(read_value("url(x)"), None);
        assert_eq!(read_value("a; b"), None);
    }

    proptest! {
        #[test]
        fn reading_never_panics(style in any::<String>()) {
            let declarations = read(&style);
            prop_assert!(declarations.read.len() <= MAX_DECLARATIONS);
            for declaration in declarations.read {
                prop_assert!(declaration.value.len() <= MAX_COMPONENTS);
                prop_assert_eq!(declaration.name.to_ascii_lowercase(), declaration.name);
            }
        }

        #[test]
        fn reading_never_panics_on_css_shaped_input(
            parts in prop::collection::vec(prop::sample::select(&[
                "color", ":", ";", " ", "red", "#fff", "1px", ".5pt", "(", ")", "[", "]", "{", "}",
                "/*", "*/", "!", "important", "rgb(", "url(", "\"", "'", "\\", "/", ",", "%", "e3",
                "-", "+", "\u{0}", "é", "@media", "<!--", "-->",
            ][..]), 0..40),
        ) {
            let _ = read(&parts.concat());
            let _ = read_value(&parts.concat());
        }

        #[test]
        fn declarations_round_trip_in_order(
            declarations in prop::collection::vec(("[a-z][a-z-]{0,12}", "[a-z]{1,8}", 0u32..100_000), 0..40),
        ) {
            let style = declarations
                .iter()
                .map(|(name, keyword, number)| format!("{name}: {keyword} {number}px"))
                .collect::<Vec<_>>()
                .join("; ");
            let read = read(&style);
            let expected = declarations.len().min(MAX_DECLARATIONS);
            prop_assert_eq!(read.read.len(), expected);
            prop_assert_eq!(read.unreadable, declarations.len() - expected);
            for (declaration, (name, keyword, number)) in read.read.iter().zip(&declarations) {
                prop_assert_eq!(&declaration.name, name);
                prop_assert_eq!(&declaration.value, &vec![ident(keyword), px(f64::from(*number))]);
            }
        }

        #[test]
        fn numbers_parse_like_the_standard_library(whole in 0u32..1_000_000, fraction in 0u32..1000, exponent in -3i32..4) {
            let text = format!("{whole}.{fraction:03}e{exponent}");
            let expected: f64 = text.parse().unwrap();
            prop_assert_eq!(read_value(&format!("{text}pt")), Some(vec![Component::Dimension { value: expected, unit: "pt".into() }]));
            prop_assert_eq!(read_value(&format!("{text}%")), Some(vec![Component::Percentage(expected)]));
        }
    }
}
