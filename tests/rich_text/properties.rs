//! P1 and P5 of editor.md §13.6 over generated HTML.
//!
//! - **P1**: `S(S(x)) == S(x)` byte for byte, and sanitising an output
//!   reports nothing.
//! - **P0**, over the same inputs: every output passes the independent
//!   oracle of `oracle.rs`, which re-checks it against `policy.json`.
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
//! A second generator draws the shapes of RUSTSEC-2026-0533 (editor.md
//! §3.5): soup weighted to the `MathML` text integration points and SVG's
//! HTML integration points, the elements of the other foreign namespace, and
//! payloads that are a comment or CDATA in one namespace and live HTML in
//! the other. Over it, P1, P0 and P5 hold, and the output holds only
//! content-v1 elements in the HTML namespace, with no comment and no
//! event-handler attribute, as a renderer's parse reads it.
//!
//! P1 runs at least 512 cases; `PROPTEST_CASES` raises that (the nightly
//! run asks for 20,000). Every shrunk failure is added to
//! `tests/fixtures/regressions/` with its expected output, so it is checked
//! on every run from then on.

use backend3::domain::rich_text::Sanitizer;
use backend3::domain::rich_text::policy::element_rule;
use html5ever::ns;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use scraper::{Html, Node};

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
    "svg", "math", "annotation-xml", "foreignObject", "mi", "mo", "mn", "ms", "mtext", "mglyph",
    "desc", "g",
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
    "https://user@www.youtube.com/embed/AbCdEfGhIjK", "https://www.youtube.com:8443/embed/AbCdEfGhIjK",
    "https://www.youtube.com/embed/AbCdEfGhIjK/extra", "https://www.youtube.com.evil.example/embed/x",
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=30", "https://padlet.com/embed/KEY",
    // Queries the embed rule refuses: an unknown parameter, the wrong order,
    // an empty value (editor.md §3.6).
    "https://www.youtube.com/embed/AbCdEfGhIjK?si=x", "https://www.youtube.com/embed/AbCdEfGhIjK?list=a&start=1",
    "https://www.youtube.com/embed/AbCdEfGhIjK?start=",
    // The YouTube id rules (ADR 0043): ids of the wrong length, `videoseries`
    // with and without `list`, and the `listType` form only the editor maps.
    "https://www.youtube.com/embed/x", "https://www.youtube.com/embed/AbCdEfGhIjKl",
    "https://www.youtube.com/embed/videoseries", "https://www.youtube.com/embed/videoseries?list=PL1",
    "https://www.youtube.com/embed?listType=playlist&list=PL1",
    // Links into /storage/ that name a directory, which the link rule keeps.
    "/storage/x.svg/", "/storage/x.svg%2F",
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

/// Tags for the shapes of RUSTSEC-2026-0533: the two foreign roots, the
/// `MathML` text integration points, SVG's HTML integration points and
/// `annotation-xml`, other elements of both foreign namespaces, the elements
/// whose content the tokenizer reads as raw text in HTML, and content-v1 and
/// formatting tags around them (a formatting end tag inside foreign content
/// reaches the adoption agency).
#[rustfmt::skip]
const FOREIGN_SOUP_TAGS: &[&str] = &[
    "svg", "math", "mi", "mo", "mn", "ms", "mtext", "foreignObject", "desc", "title",
    "annotation-xml", "mglyph", "malignmark", "g", "path", "mrow", "semantics", "annotation",
    "iframe", "style", "xmp", "noembed", "noframes", "noscript", "textarea", "script", "img",
    "p", "a", "b", "em", "font", "span", "table", "td", "li", "div",
];

/// Attributes that change how the parser reads foreign content (`encoding`
/// on `annotation-xml`, `color` on `font`) and attributes a payload uses.
#[rustfmt::skip]
const FOREIGN_SOUP_ATTRIBUTES: &[&str] = &[
    r#" encoding="text/html""#, r#" encoding="application/xhtml+xml""#, r#" color="red""#,
    r#" src="/storage/uploads/a.png""#, r#" alt="a""#, " src", " onerror=alert(1)",
    r#" href="/x""#, r#" xlink:href="javascript:alert(1)""#,
];

/// Markup that is a comment, CDATA or the end of an element in one
/// namespace and live HTML in another, and its pieces.
#[rustfmt::skip]
const FOREIGN_SOUP_PAYLOADS: &[&str] = &[
    "<!--</iframe><img src onerror=alert(1)>-->", "<!--</style><img src onerror=alert(1)>-->",
    "</xmp><img src onerror=alert(1)>", "</title><img src onerror=alert(1)>",
    "<![CDATA[</iframe><img src onerror=alert(1)>]]>", "<!--", "-->", "<![CDATA[", "]]>", "x",
];

/// The advisory's openings: each `MathML` text integration point holding the
/// SVG root, and each SVG HTML integration point (and an HTML-encoded
/// `annotation-xml`) holding the `MathML` root, so that the soup reaches them
/// often rather than by chance.
#[rustfmt::skip]
const FOREIGN_SOUP_OPENINGS: &[&str] = &[
    "<math><mi><svg>", "<math><mo><svg>", "<math><mn><svg>", "<math><ms><svg>",
    "<math><mtext><svg>", "<svg><foreignObject><math>", "<svg><desc><math>",
    "<svg><title><math>", r#"<math><annotation-xml encoding="text/html"><svg>"#,
];

/// One piece of the soup of RUSTSEC-2026-0533's shapes.
fn arb_foreign_part() -> impl Strategy<Value = String> {
    prop_oneof![
        2 => prop::sample::select(FOREIGN_SOUP_OPENINGS).prop_map(str::to_owned),
        5 => (
            prop::sample::select(FOREIGN_SOUP_TAGS),
            0u8..3,
            prop::collection::vec(prop::sample::select(FOREIGN_SOUP_ATTRIBUTES), 0..3),
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
        2 => (prop::sample::select(FOREIGN_SOUP_TAGS), 0u8..3)
            .prop_map(|(tag, case)| format!("</{}>", cased(tag, case))),
        3 => prop::sample::select(FOREIGN_SOUP_PAYLOADS).prop_map(str::to_owned),
    ]
}

/// A body of up to 30 pieces of that soup.
fn arb_foreign_soup() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_foreign_part(), 0..30).prop_map(|parts| parts.concat())
}

/// Checks that `output`, parsed as a renderer parses a body (an HTML
/// fragment), holds only content-v1 elements in the HTML namespace, no
/// comment and no event-handler attribute.
fn assert_html_only(output: &str) -> Result<(), TestCaseError> {
    let parsed = Html::parse_fragment(output);
    // The body's nodes are the descendants of the fragment's `html` root.
    // The parser's context element also lives in the tree, unattached, so
    // the walk starts from the root rather than from every node.
    for node in parsed.root_element().descendants().skip(1) {
        match node.value() {
            Node::Element(element) => {
                prop_assert!(
                    element.name.ns == ns!(html),
                    "{:?}: {:?}",
                    output,
                    element.name
                );
                let name = element.name();
                prop_assert!(element_rule(name).is_some(), "{:?}: {}", output, name);
                for (attribute, _) in element.attrs() {
                    prop_assert!(
                        !attribute.to_ascii_lowercase().starts_with("on"),
                        "{:?}: {}",
                        output,
                        attribute
                    );
                }
            }
            Node::Comment(_) => prop_assert!(false, "{:?} holds a comment", output),
            _ => {}
        }
    }
    Ok(())
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

    // P0 over generated input: the independent oracle (`oracle.rs`) finds
    // every output inside content-v1 as `policy.json` describes it. An input
    // the sanitiser refuses has no output to check.
    #[test]
    fn every_output_passes_the_oracle(input in arb_html()) {
        if let Ok(output) = sanitizer().sanitize(&input) {
            super::oracle::assert_content_v1(output.html.as_str()).map_err(TestCaseError::fail)?;
        }
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

    // The shapes of RUSTSEC-2026-0533: P1, P0, P5, and nothing foreign left.
    // `svg` and `math` go with their content before ammonia's namespace
    // check, so the output is HTML only, whichever ammonia check it has.
    // The oracle checks the output against `policy.json`, as it does every
    // P1 output; `assert_html_only` adds what the advisory is about, the
    // namespace and the comments a renderer's parse finds.
    #[test]
    fn foreign_content_in_integration_points_settles_and_leaves_nothing(
        input in arb_foreign_soup()
    ) {
        let sanitizer = sanitizer();
        assert_idempotent(&sanitizer, &input)?;
        let first = sanitizer
            .sanitize(&input)
            .map_err(|error| TestCaseError::fail(format!("{input:?} was refused: {error}")))?;
        super::oracle::assert_content_v1(first.html.as_str()).map_err(TestCaseError::fail)?;
        assert_html_only(first.html.as_str())?;
        for _ in 0..3 {
            let again = super::sanitizer().sanitize(&input);
            prop_assert_eq!(again.as_ref(), Ok(&first), "{:?}", input);
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
        // The integration points of foreign content (RUSTSEC-2026-0533).
        "<mi",
        "<mo",
        "<mn",
        "<ms",
        "<mtext",
        "<foreignobject",
        "<annotation-xml",
    ] {
        assert!(lower.contains(needle), "{needle}");
    }
    assert!(sample.contains("<IFRAME") || sample.contains("<IfRaMe"));
}

#[test]
fn the_foreign_generator_puts_the_other_namespace_in_every_integration_point() {
    // A sample of the second generator holds every opening of the
    // advisory, the payloads, and the tags in more than one case.
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = arb_foreign_soup();
    let mut sample = String::new();
    for _ in 0..2_000 {
        let tree = strategy
            .new_tree(&mut runner)
            .expect("the generator always produces a value");
        sample.push_str(&tree.current());
    }
    for needle in FOREIGN_SOUP_OPENINGS.iter().chain(FOREIGN_SOUP_PAYLOADS) {
        assert!(sample.contains(needle), "{needle}");
    }
    assert!(sample.contains("<MTEXT"));
    assert!(sample.contains("<FOREIGNOBJECT"));
}

#[test]
fn the_case_writer_covers_three_spellings() {
    assert_eq!(cased("iframe", 0), "iframe");
    assert_eq!(cased("iframe", 1), "IFRAME");
    assert_eq!(cased("iframe", 2), "IfRaMe");
    assert_eq!(cased("o:p", 2), "O:P");
}
