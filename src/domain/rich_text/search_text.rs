//! The plain-text copy of a post body that search matches against:
//! [`html_to_search_text`] (ADR 0020 §2).
//!
//! The public header search and the admin search find a term anywhere in a
//! post's title, description or body, ignoring case and accents. A body is
//! HTML, so a plain `LIKE` over it would miss a phrase that a tag splits
//! (`<strong>Ünnepi</strong> műsor`) and would match markup that no reader
//! sees (`width`, `youtube`, the `alt` of an image). Search therefore
//! matches `posts.content_text`, which holds only what the body says, as
//! one line of text. The post model recomputes it whenever `content`
//! changes, the importer fills it after body normalisation (ADR 0017), and a
//! task recomputes every row when this function changes; all three call
//! this one function, so they cannot disagree.
//!
//! The rules, from ADR 0020 §2:
//!
//! - only text nodes are kept, with every character reference decoded by
//!   the HTML parser, so `&uuml;nnep` and `ünnep` give the same text;
//! - tags, comments and attribute values (`href`, `src`, `alt`, `style`) are
//!   dropped, and so is the content of `script`, `style` and `template`,
//!   and of the other elements whose content no reader of the page sees
//!   ([`DROPPED_CONTENT`]);
//! - every start and end of a block element ([`BLOCK_ELEMENTS`]) and every
//!   `br` separates the text around it with a space, so the words of two
//!   paragraphs, list items or table cells never run together, while inline
//!   markup joins its text with its neighbours exactly as written
//!   (`<b>Ün</b>nep` is `Ünnep`);
//! - every white-space character (Unicode `White_Space`, which includes
//!   U+00A0, the no-break space that editor.md §3.8 writes for repeated
//!   spaces) becomes a space, every run of spaces collapses to one, and the
//!   result is trimmed.
//!
//! The body is parsed as the sanitiser parses it (html5ever, a fragment in a
//! `div` context, [`super::report::parse_like_ammonia`]), which is the same
//! HTML5 parser family ADR 0020 §2 names. The function is meant for stored
//! bodies, which are content-v1, but it accepts any HTML, because the task
//! that recomputes every row also reads a row that reached the database
//! outside the write path (editor.md §14, R19). Such input gets the same
//! rules, with HTML's block-level elements separating words too. Two cases
//! need the parser's raw-text rules spelt out, because the parser keeps
//! the content of some elements as one text node however much markup it
//! holds:
//!
//! - where no reader sees that text (an `iframe`'s fallback, `noscript`
//!   with scripting on, as html5ever parses by default, `noembed`,
//!   `noframes` and `title`), the element is in [`DROPPED_CONTENT`], so the
//!   markup in it never reaches the search text;
//! - where the page shows it exactly as written (`xmp`, `plaintext`,
//!   `textarea`), it is what a reader sees, markup characters included, so
//!   it is kept like any other text: `<xmp><b>x</b></xmp>` reads
//!   `<b>x</b>`.
//!
//! Like everything under `src/domain/`, the module is pure (ADR 0008).

use html5ever::ns;
use scraper::{Node, node::Element};

use super::report::parse_like_ammonia;

/// Elements whose content is dropped, whatever namespace they are in,
/// because no reader of the page sees it: script and style text and a
/// template (ADR 0020 §2), an `iframe`'s fallback content, `noscript` (the
/// parser runs with scripting on), `noembed`, `noframes` and the document
/// `title`. Sorted, so the list reads as a set.
///
/// An HTML `template`'s content never reaches the walk in any case: the
/// tree keeps it under a document fragment, which the walk skips (see
/// [`html_to_search_text`]). The entry drops a `template` that is not an
/// HTML element, such as one inside `svg`, whose text is its child.
#[rustfmt::skip]
pub const DROPPED_CONTENT: &[&str] = &[
    "iframe", "noembed", "noframes", "noscript", "script", "style", "template", "title",
];

/// The HTML elements whose start and end separate words: the block
/// elements of content-v1 (editor.md §3.2), and the other elements that
/// HTML's rendering rules display as blocks, list items, table parts or
/// table cells (WHATWG HTML, "Rendering", the user-agent style sheet), for
/// input that is not content-v1. Inline elements, `img` and `iframe` join
/// their neighbours. Sorted, so the list reads as a set.
#[rustfmt::skip]
pub const BLOCK_ELEMENTS: &[&str] = &[
    "address", "article", "aside", "blockquote", "caption", "center", "col", "colgroup", "dd",
    "details", "dialog", "dir", "div", "dl", "dt", "fieldset", "figcaption", "figure", "footer",
    "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr", "legend", "li",
    "listing", "main", "menu", "nav", "ol", "p", "plaintext", "pre", "search", "section",
    "summary", "table", "tbody", "td", "tfoot", "th", "thead", "tr", "ul", "xmp",
];

/// The searchable plain text of an HTML body (ADR 0020 §2; the rules are in
/// the module documentation).
///
/// The walk uses an explicit stack, so a deeply nested body cannot overflow
/// the call stack, and the work is linear in the size of the parsed tree.
/// The result holds no U+0000, which a PostgreSQL `text` column refuses.
///
/// - `html`: an HTML fragment, normally a stored post body.
///
/// # Examples
///
/// ```
/// use backend3::domain::rich_text::html_to_search_text;
///
/// assert_eq!(
///     html_to_search_text(r#"<h2>&Uuml;nnepi</h2><p><strong>Ün</strong>nepi&nbsp;műsor<img src="/storage/uploads/a.jpg" alt="kép"></p>"#),
///     "Ünnepi Ünnepi műsor"
/// );
/// ```
#[must_use]
pub fn html_to_search_text(html: &str) -> String {
    let parsed = parse_like_ammonia(html);
    let mut text = Collapsed::with_capacity(html.len());
    let mut stack: Vec<Step<_>> = parsed
        .root_element()
        .children()
        .rev()
        .map(Step::Enter)
        .collect();
    while let Some(step) = stack.pop() {
        let node = match step {
            Step::Leave => {
                text.separate();
                continue;
            }
            Step::Enter(node) => node,
        };
        match node.value() {
            Node::Text(contents) => text.push(contents),
            Node::Element(element) => {
                if DROPPED_CONTENT.contains(&element.name()) {
                    continue;
                }
                if is_html(element, "br") {
                    text.separate();
                    continue;
                }
                if BLOCK_ELEMENTS.contains(&element.name()) && element.name.ns == ns!(html) {
                    text.separate();
                    stack.push(Step::Leave);
                }
                stack.extend(node.children().rev().map(Step::Enter));
            }
            // Comments, doctypes and processing instructions hold no text a
            // reader sees. Nor does a document fragment: in scraper's tree
            // the only one below the root is an HTML `template`'s content,
            // its first child (`get_template_contents`), so skipping it drops
            // that content whatever `DROPPED_CONTENT` says.
            _ => {}
        }
    }
    text.finish()
}

/// Whether `element` is the HTML element `name`.
fn is_html(element: &Element, name: &str) -> bool {
    element.name() == name && element.name.ns == ns!(html)
}

/// One step of the walk: enter a node, or leave a block element.
enum Step<N> {
    Enter(N),
    Leave,
}

/// The text being collected, with white space collapsed as it comes in.
struct Collapsed {
    out: String,
    /// Whether a separator is due before the next visible character.
    space: bool,
}

impl Collapsed {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            out: String::with_capacity(capacity),
            space: false,
        }
    }

    /// Adds the characters of one text node.
    fn push(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_whitespace() {
                self.space = true;
            } else if c != '\0' {
                // html5ever never puts a U+0000 into a text node (the HTML
                // parser drops it or writes U+FFFD); this keeps the column
                // writable even if a parser change ever let one through.
                if self.space && !self.out.is_empty() {
                    self.out.push(' ');
                }
                self.space = false;
                self.out.push(c);
            }
        }
    }

    /// Marks a word boundary: a block start or end, or a `br`.
    fn separate(&mut self) {
        self.space = true;
    }

    /// The text, without the separator still due at its end, so it is
    /// trimmed on both sides.
    fn finish(self) -> String {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::policy::ELEMENTS;
    use proptest::prelude::*;
    use std::fmt::Write as _;

    fn text(html: &str) -> String {
        html_to_search_text(html)
    }

    #[test]
    fn a_tag_inside_a_phrase_does_not_split_it() {
        // The rows of ADR 0020 §3 that this function decides.
        assert_eq!(text("<p><strong>Ünnepi</strong> műsor</p>"), "Ünnepi műsor");
        assert_eq!(text("<p><b>Ün</b>nep</p>"), "Ünnep");
        assert_eq!(
            text("<p><em>Ered</em><span style=\"color: #c00000\">mények</span></p>"),
            "Eredmények"
        );
    }

    #[test]
    fn character_references_are_decoded() {
        assert_eq!(
            text("<p>&Uuml;nnepi &eacute;s &#337;szi &#x171;r</p>"),
            "Ünnepi és őszi űr"
        );
        assert_eq!(text("<p>a &amp; b &lt;c&gt;</p>"), "a & b <c>");
    }

    #[test]
    fn markup_and_attribute_values_are_not_text() {
        let body = r#"<p style="width: 100px; text-align: center"><a href="https://www.youtube.com/eacute" target="_blank" rel="noopener noreferrer">link</a></p><p><img src="/storage/uploads/width.jpg" alt="youtube kép" width="350"></p>"#;
        let plain = text(body);
        assert_eq!(plain, "link");
        for markup in [
            "eacute", "width", "youtube", "storage", "center", "noopener",
        ] {
            assert!(!plain.contains(markup), "{markup}");
        }
    }

    #[test]
    fn script_style_and_template_content_is_dropped() {
        assert_eq!(text("a<script>var b = 1;</script>c"), "ac");
        assert_eq!(
            text("<p>a</p><style>p { color: red }</style><p>c</p>"),
            "a c"
        );
        // An HTML template's content sits under a document fragment, which
        // the walk skips; a template in SVG is a plain element with a text
        // child, which only the `DROPPED_CONTENT` entry drops.
        assert_eq!(text("a<template><p>b</p></template>c"), "ac");
        assert_eq!(text("<svg><template>rejtett</template></svg>z"), "z");
        assert_eq!(text("<svg><style>x</style><script>y</script></svg>z"), "z");
        assert_eq!(text("a<!-- megjegyzés -->c"), "ac");
    }

    #[test]
    fn raw_text_that_no_reader_sees_is_dropped() {
        // The parser keeps each element's content as one text node, markup
        // and all; none of it is shown on the page.
        assert_eq!(text("<noscript><p>x</p></noscript>y"), "y");
        assert_eq!(text("a<iframe>rejtett</iframe>b"), "ab");
        assert_eq!(text("a<noembed><b>x</b></noembed>b"), "ab");
        assert_eq!(text("a<noframes><b>x</b></noframes>b"), "ab");
        assert_eq!(text("a<title>Cím</title>b"), "ab");
        for name in DROPPED_CONTENT {
            assert_eq!(
                text(&format!("első <{name}><p>rejtett</p></{name}> második")),
                "első második",
                "{name}"
            );
        }
    }

    #[test]
    fn raw_text_shown_as_written_is_kept_as_written() {
        // `xmp`, `plaintext` and `textarea` show their content exactly as
        // written, so a reader sees the markup characters, and search
        // matches what the reader sees.
        assert_eq!(text("<xmp><b>x</b></xmp>"), "<b>x</b>");
        assert_eq!(text("<plaintext><p>x</p>"), "<p>x</p>");
        assert_eq!(text("<textarea><b>x</b></textarea>"), "<b>x</b>");
    }

    #[test]
    fn the_dropped_list_is_sorted_and_lower_case() {
        assert!(DROPPED_CONTENT.is_sorted());
        for name in DROPPED_CONTENT {
            assert_eq!(*name, name.to_ascii_lowercase());
        }
    }

    #[test]
    fn blocks_and_breaks_separate_words() {
        assert_eq!(text("<p>első</p><p>második</p>"), "első második");
        assert_eq!(text("<h2>Cím</h2>Szöveg"), "Cím Szöveg");
        assert_eq!(text("a<br>b<br><br>c"), "a b c");
        assert_eq!(
            text("<ul><li>egy</li><li>kettő</li></ul><ol><li>három</li></ol>"),
            "egy kettő három"
        );
        assert_eq!(
            text("<table><tbody><tr><td>a</td><th>b</th></tr><tr><td>c</td></tr></tbody></table>"),
            "a b c"
        );
        assert_eq!(text("<blockquote>idézet</blockquote>után"), "idézet után");
        assert_eq!(text("a<hr>b"), "a b");
        assert_eq!(text("<div>a</div><section>b</section>"), "a b");
    }

    #[test]
    fn every_content_v1_element_is_block_or_inline_as_it_renders() {
        // Blocks separate their text from the neighbours; inline elements,
        // images and embeds join it.
        let blocks = [
            "p",
            "h1",
            "h2",
            "h3",
            "h4",
            "blockquote",
            "ul",
            "ol",
            "li",
            "hr",
            "table",
            "tbody",
            "tr",
            "td",
            "th",
        ];
        for element in ELEMENTS {
            let name = element.name;
            let expected_block = blocks.contains(&name);
            assert_eq!(BLOCK_ELEMENTS.contains(&name), expected_block, "{name}");
        }
        for inline in ["strong", "em", "u", "s", "sub", "sup", "a", "span"] {
            assert_eq!(
                text(&format!("x<{inline}>y</{inline}>z")),
                "xyz",
                "{inline}"
            );
        }
        assert_eq!(
            text(r#"x<img src="/storage/uploads/a.jpg" alt="kép">z"#),
            "xz"
        );
        assert_eq!(
            text(r#"x<iframe src="https://padlet.com/embed/k"></iframe>z"#),
            "xz"
        );
    }

    #[test]
    fn the_block_list_is_sorted_html() {
        assert!(BLOCK_ELEMENTS.is_sorted());
        for name in BLOCK_ELEMENTS {
            assert_eq!(*name, name.to_ascii_lowercase());
            assert!(!DROPPED_CONTENT.contains(name), "{name}");
        }
    }

    #[test]
    fn white_space_collapses_and_is_trimmed() {
        assert_eq!(text(""), "");
        assert_eq!(text("   \n\t "), "");
        assert_eq!(text("<p> </p><p>\u{a0}</p>"), "");
        assert_eq!(text("  a \n\t b  "), "a b");
        assert_eq!(text("a&nbsp;b\u{a0}\u{a0} c"), "a b c");
        // Every Unicode white-space character, not only ASCII.
        assert_eq!(text("a\u{2003}\u{3000}b\u{2028}c\u{85}d"), "a b c d");
        // A zero-width space is not white space; it stays.
        assert_eq!(text("a\u{200b}b"), "a\u{200b}b");
    }

    #[test]
    fn a_body_in_editor_form_reads_as_its_words() {
        let body = concat!(
            r#"<h2 style="text-align: center">Eredmények</h2>"#,
            r#"<p>Szöveg <strong>félkövér</strong>, <span style="color: #c00000; font-size: 1.25em">piros és nagy</span>.</p>"#,
            r#"<ol start="5" type="a"><li>első</li><li><p>két</p><p>bekezdés</p></li></ol>"#,
            r#"<p><img src="/storage/uploads/posts/612/kep.jpg" alt="A díjazottak a színpadon" width="350" height="495"></p>"#,
            r#"<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/VIDEOID?start=30" width="640" height="360" title="Videó"></iframe></p>"#,
            "<p>két\u{a0} szóköz</p>",
        );
        assert_eq!(
            text(body),
            "Eredmények Szöveg félkövér, piros és nagy. első két bekezdés két szóköz"
        );
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        let deep = format!("{}x{}", "<span>".repeat(20_000), "</span>".repeat(20_000));
        assert_eq!(text(&deep), "x");
        let blocks = format!("{}y", "<blockquote>".repeat(5_000));
        assert_eq!(text(&blocks), "y");
    }

    #[test]
    fn a_nul_never_reaches_the_text() {
        assert_eq!(text("a\0b"), "ab");
        assert_eq!(text("<svg><![CDATA[a\0b]]></svg>"), "a\u{fffd}b");
    }

    /// Characters a body says, with the Hungarian letters and those that
    /// need escaping in HTML text.
    const LETTERS: &str = "aáeéiíoóöőuúüűAÁEÉIÍOÓÖŐUÚÜŰbcdz019 .,;:!?-%_&<>\"'\u{a0}\n\t";

    fn arb_text() -> impl Strategy<Value = String> {
        let letters: Vec<char> = LETTERS.chars().collect();
        prop::collection::vec(prop::sample::select(letters), 0..40)
            .prop_map(|chars| chars.into_iter().collect())
    }

    /// `text` as HTML text, with only `&`, `<` and `>` escaped.
    fn literal(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// `text` as HTML text with every character written as a reference:
    /// named where HTML has a name, decimal or hexadecimal by `seed`.
    fn encoded(text: &str, seed: &[u8]) -> String {
        let mut out = String::new();
        for (i, c) in text.chars().enumerate() {
            let named = match c {
                'á' => Some("aacute"),
                'é' => Some("eacute"),
                'ö' => Some("ouml"),
                'ő' => Some("odblac"),
                'ü' => Some("uuml"),
                'ű' => Some("udblac"),
                'Á' => Some("Aacute"),
                'Ü' => Some("Uuml"),
                '&' => Some("amp"),
                '<' => Some("lt"),
                '>' => Some("gt"),
                '"' => Some("quot"),
                '\u{a0}' => Some("nbsp"),
                _ => None,
            };
            let _ = match (
                seed.get(i % seed.len().max(1)).copied().unwrap_or(0) % 3,
                named,
            ) {
                (0, Some(name)) => write!(out, "&{name};"),
                (1, _) => write!(out, "&#{};", u32::from(c)),
                _ => write!(out, "&#x{:X};", u32::from(c)),
            };
        }
        out
    }

    /// What the rules make of plain text: white space collapsed, trimmed.
    fn collapsed(text: &str) -> String {
        text.split(char::is_whitespace)
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[rustfmt::skip]
    const INLINE: &[&str] = &["strong", "em", "u", "s", "sub", "sup", "span", "a", "b", "i", "font", "code", "mark"];

    proptest! {
        #[test]
        fn the_output_has_no_stray_white_space(input in any::<String>()) {
            let plain = text(&input);
            prop_assert_eq!(plain.trim(), plain.as_str());
            prop_assert!(!plain.contains("  "), "{:?}", plain);
            prop_assert!(plain.chars().all(|c| c == ' ' || !c.is_whitespace()), "{:?}", plain);
            prop_assert!(!plain.contains('\0'));
        }

        // ADR 0020: the entity-encoded and the literal form of a body give
        // the same text.
        #[test]
        fn encoded_and_literal_bodies_agree(body in arb_text(), seed in prop::collection::vec(any::<u8>(), 1..8)) {
            let wrapped = |inner: String| format!("<p>{inner}</p>");
            prop_assert_eq!(text(&wrapped(literal(&body))), text(&wrapped(encoded(&body, &seed))));
            prop_assert_eq!(text(&wrapped(literal(&body))), collapsed(&body));
        }

        // ADR 0020: text wrapped in inline tags comes back unchanged apart
        // from white space.
        #[test]
        fn inline_markup_leaves_the_text(pieces in prop::collection::vec((arb_text(), prop::sample::select(INLINE)), 0..8)) {
            let mut html = String::new();
            let mut plain = String::new();
            for (piece, tag) in &pieces {
                let _ = write!(html, "<{tag}>{}</{tag}>", literal(piece));
                plain.push_str(piece);
            }
            prop_assert_eq!(text(&html), collapsed(&plain));
        }

        #[test]
        fn blocks_put_one_space_between_words(words in prop::collection::vec("[a-zőű]{1,8}", 1..8), tags in prop::collection::vec(prop::sample::select(&["p", "h2", "li", "blockquote", "div", "section"][..]), 8)) {
            // No `td`: outside a table the parser drops cell tags (the unit test
            // above checks cells inside one).
            let mut html = String::new();
            for (word, tag) in words.iter().zip(&tags) {
                let _ = write!(html, "<{tag}>{word}</{tag}>");
            }
            prop_assert_eq!(text(&html), words.join(" "));
        }

        #[test]
        fn attribute_values_never_reach_the_text(word in "[a-z]{1,10}", value in "[A-Z]{1,10}", name in prop::sample::select(&["href", "src", "alt", "style", "title", "data-x"][..])) {
            let html = format!(r#"<p><a {name}="{value}">{word}</a><img {name}="{value}"></p>"#);
            prop_assert_eq!(text(&html), word);
        }

        // The text of a search text is itself.
        #[test]
        fn the_text_is_a_fixed_point(input in any::<String>()) {
            let plain = text(&input);
            prop_assert_eq!(text(&literal(&plain)), plain);
        }
    }
}
