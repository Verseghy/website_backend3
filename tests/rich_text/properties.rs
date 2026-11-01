//! P1 and P5 of editor.md §13.6 over generated HTML.
//!
//! - **P1**: `S(S(x)) == S(x)` byte for byte, and sanitising an output
//!   reports nothing.
//! - **P5**: independent `Sanitizer` instances, each with its own hash
//!   seeds, produce the same bytes and the same report.
//!
//! The generator draws tag soup, not trees: start and end tags in any
//! order, unclosed and misnested, from the pools of §13.6 (allowed,
//! forbidden and tricky tags in mixed case, dangerous attributes, the URL
//! and CSS pools), plus text with references, NUL and markup-like
//! characters, comments, CDATA and doctypes. Soup is what reaches the
//! parser's repair rules (foster parenting, the adoption agency, raw-text
//! states), which is where a serialised tree can parse differently the
//! next time.
//!
//! P1 runs at least 512 cases; `PROPTEST_CASES` raises that (the nightly
//! run asks for 20,000). Every shrunk failure is added to
//! `tests/fixtures/regressions/` with its expected output, so it is checked
//! on every run from then on.

use backend3::domain::rich_text::Sanitizer;
use proptest::prelude::*;
use proptest::strategy::ValueTree;

use super::sanitizer;

/// Tags from §13.6: the content-v1 tags, the forbidden and raw-text ones,
/// the `svg` and `math` surface, and the elements whose unwrapping or
/// parsing moves content (`caption`, `thead`, `button`, `select`, `form`).
#[rustfmt::skip]
const TAGS: &[&str] = &[
    // content-v1
    "p", "h1", "h2", "h3", "h4", "strong", "em", "u", "s", "sub", "sup", "br", "hr",
    "blockquote", "ul", "ol", "li", "a", "span", "img", "table", "tbody", "tr", "td", "th",
    "iframe",
    // removed with their content, raw text and parser-state tags
    "svg", "math", "annotation-xml", "foreignObject", "mi", "mtext", "mglyph", "desc", "g",
    "noscript", "template", "xmp", "plaintext", "textarea", "title", "style", "script",
    "button", "marquee", "select", "option", "object", "embed", "xml", "frameset",
    // unwrapped, or restructured by the parser
    "caption", "thead", "tfoot", "colgroup", "col", "form", "div", "center", "font", "b", "i",
    "o:p", "h5", "image", "listing", "nobr",
];

/// Attribute names from §13.6 plus every content-v1 attribute, and the two
/// that change how the parser reads foreign content: `color` makes a `font`
/// break out of `svg` or `math`, and `encoding` can make an `annotation-xml`
/// an integration point.
#[rustfmt::skip]
const ATTRIBUTES: &[&str] = &[
    "onclick", "onerror", "onload", "srcdoc", "srcset", "xlink:href", "formaction", "style",
    "class", "id", "data-x", "data-youtube-video", "data-padlet-embed", "src", "href", "alt",
    "title", "width", "height", "target", "rel", "colspan", "rowspan", "start", "type", "allow",
    "allowfullscreen", "referrerpolicy", "name", "xmlns", "color", "encoding",
];

/// The URL pool of §13.6.
#[rustfmt::skip]
const URLS: &[&str] = &[
    "javascript:alert(1)", "JaVaScRiPt:alert(1)", "java&#x73;cript:alert(1)",
    "java\tscript:alert(1)", "&#106;avascript:alert(1)", " javascript:alert(1)",
    "javascript&colon;alert(1)", "data:image/png;base64,AA", "data:text/html,<script>x</script>",
    "vbscript:x", "blob:https://verseghy-gimnazium.net/1", "file:///etc/passwd",
    "//evil.example/x", "/\\evil.example", "/%2F/evil", "\\\\evil.example",
    "https://user@www.youtube.com/embed/x", "https://www.youtube.com:8443/embed/x",
    "https://www.youtube.com/embed/x/extra", "https://www.youtube.com.evil.example/embed/x",
    "https://www.youtube-nocookie.com/embed/VIDEOID?start=30", "https://padlet.com/embed/KEY",
    "/storage/uploads/../../x.jpg", "/storage/uploads/a.jpg", "/storage/uploads/a%2F..%2Fb.jpg",
    "/storage/uploads/%FF.png", "https://verseghy-gimnazium.net/storage/uploads/a.png",
    "https://backend.verseghy-gimnazium.net/storage/x.svg", "/storage/x.html", "mailto:a@example.org",
    "tel:+3612345678", "#top", "",
];

/// The CSS pool of §13.6, and inputs that reach the canonical writer's
/// rules (editor.md §3.1) and the nested-size rule of §7.5.
#[rustfmt::skip]
const STYLES: &[&str] = &[
    "background: url(https://evil.example/x)", "background-image: URL(\"x\")", "u\\rl(x)",
    "width: expression(alert(1))", "width: var(--x)", "width: calc(1px + 2px)", "content: attr(x)",
    "width: env(x)", "color: #ff0000 !important", "/* c */ color: red", "content: \"x\"",
    "color: red; }", "@import \"x\"", "width: 1e30px", "width: -1px", "font-size: .5pt",
    "width: 15.0pt", "width: 100%px", "color: windowtext", "color: rgb(1, 2, 3)",
    "color: hsl(1, 2%, 3%)", "color: whitesmoke", "font-family: Árvíztűrő", "color: re\0d",
    "text-align: center", "border-collapse: collapse; max-width: fit-content",
    // The canonical writer: shorthands, sizes that nest, and `box-sizing`.
    "border: thin solid; border-top: none; width: 96pt", "border-width: 1px 2px; border-style: solid",
    "background: yellow; font-size: 2em", "font-size: 3em", "box-sizing: content-box",
];

/// Other attribute values: the forced and enumerated values, their case
/// variants, integers out of range, quotes, and the `encoding` that makes an
/// `annotation-xml` an HTML integration point.
#[rustfmt::skip]
const VALUES: &[&str] = &[
    "1", "0350", "5001", "100%", "_blank", "_BLANK", "noopener noreferrer", "a", "I", "disc",
    "strict-origin-when-cross-origin",
    "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share",
    "x\"y", "a'b", "<b>", "&amp;", "kép", "text/html",
];

/// Text from §13.6: references, NUL, markup-like characters, non-ASCII.
#[rustfmt::skip]
const TEXTS: &[&str] = &[
    "x", "szöveg", " ", "\n", "\u{a0}", "&nbsp;", "&amp;", "&lt;", "&gt;", "&#0;", "&#x1F600;",
    "\0", "<", ">", "&", "\"", "'", "</iframe", "</", "<!--", "-->", "<![CDATA[x]]>",
    "<!DOCTYPE html>", "<?php x ?>", "<!-- c -->", "</p>", "</a>", "<![CDATA[", "]]>",
];

/// Writes a tag name in one of three cases.
fn cased(name: &str, case: u8) -> String {
    match case {
        0 => name.to_owned(),
        1 => name.to_ascii_uppercase(),
        _ => name
            .chars()
            .enumerate()
            .map(|(i, c)| {
                if i % 2 == 0 {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            })
            .collect(),
    }
}

/// One attribute: a name and a value from the pool that suits it, written
/// double-quoted, single-quoted or bare.
fn arb_attribute() -> impl Strategy<Value = String> {
    (
        prop::sample::select(ATTRIBUTES),
        prop_oneof![
            prop::sample::select(URLS),
            prop::sample::select(STYLES),
            prop::sample::select(VALUES),
        ],
        0u8..4,
    )
        .prop_map(|(name, value, quoting)| match quoting {
            0 => format!(r#" {name}="{}""#, value.replace('"', "&quot;")),
            1 => format!(" {name}='{}'", value.replace('\'', "&#39;")),
            2 if !value.contains([' ', '>', '"', '\'', '=', '<', '`']) && !value.is_empty() => {
                format!(" {name}={value}")
            }
            _ => format!(" {name}"),
        })
}

/// One piece of tag soup.
fn arb_part() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => (
            prop::sample::select(TAGS),
            0u8..3,
            prop::collection::vec(arb_attribute(), 0..4),
            prop::bool::weighted(0.1),
        )
            .prop_map(|(tag, case, attributes, closing)| {
                format!(
                    "<{}{}{}>",
                    cased(tag, case),
                    attributes.concat(),
                    if closing { "/" } else { "" }
                )
            }),
        3 => (prop::sample::select(TAGS), 0u8..3)
            .prop_map(|(tag, case)| format!("</{}>", cased(tag, case))),
        3 => prop::sample::select(TEXTS).prop_map(str::to_owned),
    ]
}

/// A body of up to 40 pieces of soup.
pub(super) fn arb_html() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_part(), 0..40).prop_map(|parts| parts.concat())
}

/// At least 512 cases (editor.md §13.6), or more when `PROPTEST_CASES`
/// asks for more. proptest itself reads the variable in its default
/// configuration.
pub(super) fn config() -> ProptestConfig {
    let default = ProptestConfig::default();
    ProptestConfig {
        cases: default.cases.max(512),
        ..default
    }
}

/// Checks P1 for `input` with `sanitizer`.
fn assert_idempotent(sanitizer: &Sanitizer, input: &str) -> Result<(), TestCaseError> {
    let once = sanitizer
        .sanitize(input)
        .map_err(|error| TestCaseError::fail(format!("{input:?} was refused: {error}")))?;
    let twice = sanitizer.sanitize(once.html.as_str()).map_err(|error| {
        TestCaseError::fail(format!("the output {:?} was refused: {error}", once.html))
    })?;
    prop_assert_eq!(&twice.html, &once.html, "input {:?}", input);
    prop_assert!(
        twice.report.is_empty(),
        "{:?}: {:?}",
        once.html,
        twice.report
    );
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    // P1.
    #[test]
    fn sanitising_twice_equals_sanitising_once(input in arb_html()) {
        assert_idempotent(&sanitizer(), &input)?;
    }

    // P5: instances built one after another get different hash seeds, so
    // ammonia appends missing forced attributes in a different order in
    // each; the output and the report must not show it.
    #[test]
    fn independent_instances_write_the_same_bytes(input in arb_html()) {
        let first = sanitizer().sanitize(&input);
        for _ in 0..3 {
            prop_assert_eq!(&sanitizer().sanitize(&input), &first, "{:?}", input);
        }
    }
}

#[test]
fn the_generator_reaches_the_tricky_parts() {
    // A sample of the generator's output contains what §13.6 asks P1 to
    // draw from, so the property is not passing on a narrow input space.
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = arb_html();
    let mut sample = String::new();
    for _ in 0..2_000 {
        let tree = strategy
            .new_tree(&mut runner)
            .expect("the generator always produces a value");
        sample.push_str(&tree.current());
    }
    let lower = sample.to_ascii_lowercase();
    for needle in [
        "<svg",
        "<math",
        "<iframe",
        "<table",
        "<caption",
        "<noscript",
        "<template",
        "<plaintext",
        "javascript:",
        "srcdoc",
        "url(",
        "expression(",
        "</iframe",
        "<![cdata[",
        "\0",
    ] {
        assert!(lower.contains(needle), "{needle}");
    }
    assert!(sample.contains("<IFRAME") || sample.contains("<IfRaMe"));
}

#[test]
fn the_case_writer_covers_three_spellings() {
    assert_eq!(cased("iframe", 0), "iframe");
    assert_eq!(cased("iframe", 1), "IFRAME");
    assert_eq!(cased("iframe", 2), "IfRaMe");
    assert_eq!(cased("o:p", 2), "O:P");
}
