//! The tree one normaliser pass works on: the N0 parse, the helpers the
//! rules share to read and edit it, and the N8 serialisation
//! (data-migration.md §7.1).
//!
//! **N0.** A body is parsed exactly as the sanitiser parses it
//! ([`parse_like_ammonia`]: html5ever 0.40.1, an HTML fragment, scraper's
//! tree). That parse decodes every character reference, lower-cases tag and
//! attribute names (the legacy `<P STYLE=…>`), turns `\r\n` and a lone `\r`
//! into `\n`, and drops the slash of `<img … />`. data-migration.md names a
//! `body` context; the sanitiser's `div` context builds the same tree,
//! because for both the tokenizer starts in the data state and the tree
//! builder resets its insertion mode to "in body" (WHATWG HTML, "reset the
//! insertion mode appropriately", whose last step gives "in body" for an
//! element that is neither a table part nor `select` nor `template`).
//! Sharing the sanitiser's parse means that the tree N0 builds is the tree
//! N7 sees, including the one tree-dependent question of the parser, an
//! `annotation-xml` integration point.
//!
//! **N8.** The tree is written with html5ever's serializer and its default
//! options, as the sanitiser's structural pass writes it: in text only `&`,
//! `<`, `>` and U+00A0 are escaped, in attributes also `"`, so no other
//! character reference is left (editor.md §3.8). scraper's own `html()` and
//! `inner_html()` are not used: they serialise with scripting disabled,
//! which escapes the text of a `noscript` that the parser, with scripting
//! enabled, read as raw text, so the next parse would see the escapes as
//! text and the body would grow with every pass.
//!
//! **Attribute order.** scraper's tree does not keep the source order of
//! attributes: `Element::new` sorts them by name, and `Element::attr` finds
//! one by binary search (scraper@0.29.0:src/node.rs). N8 therefore writes
//! the attributes of a content-v1 element in the canonical order of
//! editor.md §3.2, which the policy tables hold ([`element_rule`]), and any
//! other attribute after them in the tree's order. A content-v1 body then
//! comes out as it went in, so the sanitiser finds it at its fixed point
//! (N9), and the output is the same in every process. [`set_attribute`]
//! keeps the sorted order that `Element::attr` relies on.
//!
//! The parsed fragment hangs below one `html` element, the fragment's root
//! ([`content`]); its children are the body. Every walk here uses the
//! tree's iterators or an explicit list, never recursion, so a deeply
//! nested legacy body cannot overflow the stack.

use std::io;

use ego_tree::iter::Edge;
use ego_tree::{NodeId, NodeRef};
use html5ever::serialize::{HtmlSerializer, SerializeOpts, Serializer};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, LocalName, QualName, ns};
use scraper::node::Element;
use scraper::{Html, Node};

use crate::domain::rich_text::policy::element_rule;
use crate::domain::rich_text::report::parse_like_ammonia;

/// Parses a body (N0, see the module documentation).
///
/// - `html`: the body, as stored or as the previous pass wrote it.
pub(super) fn parse(html: &str) -> Html {
    parse_like_ammonia(html)
}

/// Writes the body that `html` holds in html5ever's dialect (N8).
///
/// - `html`: a parsed body, edited or not.
pub(super) fn serialise(html: &Html) -> String {
    let root = html.root_element();
    let mut out = Vec::new();
    let mut serializer = HtmlSerializer::new(&mut out, SerializeOpts::default());
    for edge in root.traverse() {
        match edge {
            Edge::Open(node) if node.id() != root.id() => {
                written(match node.value() {
                    Node::Text(text) => serializer.write_text(text),
                    Node::Comment(comment) => serializer.write_comment(comment),
                    Node::Element(element) => serializer.start_elem(
                        element.name.clone(),
                        written_attributes(element).into_iter(),
                    ),
                    Node::Doctype(doctype) => serializer.write_doctype(doctype.name()),
                    Node::ProcessingInstruction(instruction) => serializer
                        .write_processing_instruction(&instruction.target, &instruction.data),
                    // A fragment's root is never below its `html` element.
                    Node::Document | Node::Fragment => Ok(()),
                });
            }
            Edge::Close(node) if node.id() != root.id() => {
                if let Node::Element(element) = node.value() {
                    written(serializer.end_elem(element.name.clone()));
                }
            }
            Edge::Open(_) | Edge::Close(_) => {}
        }
    }
    drop(serializer);
    // The serializer writes only `&str` data and ASCII markup.
    String::from_utf8(out).expect("html5ever's serializer writes UTF-8")
}

/// The attributes of `element` in the order N8 writes them: for an HTML
/// element of content-v1, the attributes its policy rule lists, in the
/// rule's order (editor.md §3.2), then every other attribute in the tree's
/// order (see "Attribute order" in the module documentation).
///
/// - `element`: an element of a parsed body.
fn written_attributes(element: &Element) -> Vec<(&QualName, &str)> {
    let rule = if element.name.ns == ns!(html) {
        element_rule(&element.name.local)
    } else {
        None
    };
    let listed = |name: &QualName| {
        rule.is_some_and(|rule| name.ns == ns!() && rule.attribute(&name.local).is_some())
    };
    let mut ordered = Vec::with_capacity(element.attrs.len());
    for allowed in rule.map_or(&[][..], |rule| rule.attributes) {
        ordered.extend(
            element
                .attrs
                .iter()
                .filter(|(name, _)| name.ns == ns!() && &*name.local == allowed.name)
                .map(|(name, value)| (name, &**value)),
        );
    }
    ordered.extend(
        element
            .attrs
            .iter()
            .filter(|(name, _)| !listed(name))
            .map(|(name, value)| (name, &**value)),
    );
    ordered
}

/// The result of one serializer call. The serializer writes into a
/// `Vec<u8>`, whose `io::Write` never fails.
fn written(result: io::Result<()>) {
    result.expect("writing into a Vec<u8> cannot fail");
}

/// The id of the `html` element that holds the parsed body.
///
/// - `html`: a parsed body.
pub(super) fn content(html: &Html) -> NodeId {
    html.root_element().id()
}

/// The ids of every element below the body's root whose HTML tag name is
/// `name`, in document order.
///
/// - `html`: a parsed body.
/// - `name`: a lower-case tag name.
pub(super) fn elements_named(html: &Html, name: &str) -> Vec<NodeId> {
    html.root_element()
        .descendants()
        .filter(|node| html_name(*node) == Some(name))
        .map(|node| node.id())
        .collect()
}

/// The tag name of `node` if it is an element in the HTML namespace, the
/// only namespace whose elements the rules touch (a `p` inside `svg` is an
/// SVG element).
///
/// - `node`: any node of a parsed body.
pub(super) fn html_name(node: NodeRef<'_, Node>) -> Option<&str> {
    match node.value() {
        Node::Element(element) if element.name.ns == ns!(html) => Some(&element.name.local),
        _ => None,
    }
}

/// Whether `text` holds only ASCII white space (space, tab, LF, FF, CR),
/// the characters that HTML collapses and that a renderer does not show
/// between blocks. U+00A0 is not white space here: it renders.
///
/// - `text`: the content of a text node.
pub(super) fn is_white_space(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii_whitespace())
}

/// The ids of the children of `id`, in order.
///
/// - `html`: a parsed body.
/// - `id`: a node of it.
pub(super) fn children(html: &Html, id: NodeId) -> Vec<NodeId> {
    node(html, id).children().map(|child| child.id()).collect()
}

/// The node `id` of `html`.
///
/// - `html`: a parsed body.
/// - `id`: an id that a walk of this tree returned.
pub(super) fn node(html: &Html, id: NodeId) -> NodeRef<'_, Node> {
    html.tree.get(id).expect("an id of this tree")
}

/// Runs `edit` on the element `id` of `html` and returns its result, or
/// `None` when the node is not an element. It takes a closure because
/// ego-tree lends a node's value only for as long as the node handle
/// lives.
///
/// - `html`: a parsed body.
/// - `id`: an id that a walk of this tree returned.
/// - `edit`: what to do with the element, for example set an attribute.
pub(super) fn edit_element<R>(
    html: &mut Html,
    id: NodeId,
    edit: impl FnOnce(&mut Element) -> R,
) -> Option<R> {
    match html.tree.get_mut(id).expect("an id of this tree").value() {
        Node::Element(element) => Some(edit(element)),
        _ => None,
    }
}

/// Replaces the node `id` by its children, in their order.
///
/// - `html`: a parsed body.
/// - `id`: a node below the body's root.
pub(super) fn unwrap(html: &mut Html, id: NodeId) {
    let children = children(html, id);
    let mut node = html.tree.get_mut(id).expect("an id of this tree");
    for child in children {
        // `insert_id_before` detaches the child from `id` first.
        node.insert_id_before(child);
    }
    node.detach();
}

/// Moves the nodes `run`, siblings in document order, into a new element
/// that takes the place of the first of them, and returns the new
/// element's id.
///
/// - `html`: a parsed body.
/// - `run`: consecutive siblings, at least one.
/// - `element`: the new element, without children.
pub(super) fn wrap(html: &mut Html, run: &[NodeId], element: Element) -> NodeId {
    let first = *run.first().expect("a run holds at least one node");
    let wrapper = html
        .tree
        .get_mut(first)
        .expect("an id of this tree")
        .insert_before(Node::Element(element))
        .id();
    let mut node = html.tree.get_mut(wrapper).expect("the new element");
    for &id in run {
        node.append_id(id);
    }
    wrapper
}

/// A new HTML element with attributes in no namespace, in the given order.
///
/// - `name`: the tag name.
/// - `attributes`: the attributes, as name and value.
pub(super) fn new_element(name: LocalName, attributes: &[(&str, &str)]) -> Element {
    Element::new(
        QualName::new(None, ns!(html), name),
        attributes
            .iter()
            .map(|&(name, value)| Attribute {
                name: QualName::new(None, ns!(), LocalName::from(name)),
                value: StrTendril::from(value),
            })
            .collect(),
    )
}

/// Sets the attribute `name`, in no namespace, of `element` to `value`: in
/// place if the element has it, otherwise inserted where the sorted order
/// of scraper's tree puts it, so that `Element::attr` still finds every
/// attribute (see "Attribute order" in the module documentation).
///
/// - `element`: the element to change.
/// - `name`: the attribute's name.
/// - `value`: its new value.
pub(super) fn set_attribute(element: &mut Element, name: &str, value: &str) {
    let name = QualName::new(None, ns!(), LocalName::from(name));
    match element
        .attrs
        .binary_search_by(|(attribute, _)| attribute.cmp(&name))
    {
        Ok(at) => element.attrs[at].1 = StrTendril::from(value),
        Err(at) => element.attrs.insert(at, (name, StrTendril::from(value))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use html5ever::local_name;
    use proptest::prelude::*;
    use std::fmt::Write as _;

    /// `words`, each in a `<tag>` element.
    fn tagged(tag: &str, words: &[String]) -> String {
        let mut out = String::new();
        for word in words {
            let _ = write!(out, "<{tag}>{word}</{tag}>");
        }
        out
    }

    /// `body` parsed and written back.
    fn round_trip(body: &str) -> String {
        serialise(&parse(body))
    }

    #[test]
    fn the_parse_decodes_references_and_the_writer_keeps_only_five() {
        assert_eq!(
            round_trip(
                "<p>&eacute;v &ndash; &bdquo;x&rdquo; &amp; &lt;b&gt; &nbsp;&hellip; &#39;&quot;</p>"
            ),
            "<p>év – „x” &amp; &lt;b&gt; &nbsp;… '\"</p>"
        );
        assert_eq!(
            round_trip(r#"<a href="/x?a=1&amp;b=&quot;2&quot;" title="&eacute;&#60;&gt;">l</a>"#),
            r#"<a href="/x?a=1&amp;b=&quot;2&quot;" title="é&lt;&gt;">l</a>"#
        );
    }

    #[test]
    fn the_parse_lower_cases_names_and_normalises_line_ends_and_void_tags() {
        assert_eq!(
            round_trip(
                "<P STYLE=\"text-align: justify\">a<BR />\r\nb\rc</P>\r\n<IMG SRC=\"/x.jpg\" />"
            ),
            "<p style=\"text-align: justify\">a<br>\nb\nc</p>\n<img src=\"/x.jpg\">"
        );
    }

    #[test]
    fn comments_stay_for_the_sanitiser() {
        assert_eq!(
            round_trip("<!-- Iframely --><p>a</p>"),
            "<!-- Iframely --><p>a</p>"
        );
    }

    #[test]
    fn noscript_text_is_written_raw_as_the_parser_read_it() {
        // scraper's own serialiser would write `&lt;b&gt;` here, and the next
        // parse would keep the escapes as text.
        let body = "<noscript><b>&amp;</b></noscript>";
        assert_eq!(round_trip(body), body);
        assert_eq!(round_trip(&round_trip(body)), body);
    }

    #[test]
    fn the_root_holds_the_body() {
        let html = parse("<p>a</p>b");
        let root = content(&html);
        assert_eq!(html_name(node(&html, root)), Some("html"));
        assert_eq!(children(&html, root).len(), 2);
    }

    #[test]
    fn elements_are_found_by_name_in_document_order_and_only_in_html() {
        let html = parse("<div><p id=1><span><p id=2></span></div><svg><p></svg><p id=3>");
        // The `p` start tag breaks out of `svg`, so the third `p` is HTML.
        let ids: Vec<_> = elements_named(&html, "p")
            .into_iter()
            .map(|id| {
                node(&html, id)
                    .value()
                    .as_element()
                    .and_then(|e| e.attr("id"))
            })
            .collect();
        assert_eq!(ids, [Some("1"), Some("2"), None, Some("3")]);
        let html = parse("<svg><title>x</title></svg>");
        assert!(elements_named(&html, "title").is_empty(), "an SVG title");
    }

    #[test]
    fn white_space_is_the_ascii_set_only() {
        assert!(is_white_space(""));
        assert!(is_white_space(" \t\n\r\u{c}"));
        assert!(!is_white_space("\u{a0}"));
        assert!(!is_white_space("\u{b}"), "U+000B is not HTML white space");
        assert!(!is_white_space(" x "));
    }

    #[test]
    fn unwrapping_keeps_the_children_in_place_and_order() {
        let mut html = parse("<p>a<center>b<em>c</em>d</center>e</p>");
        let centre = elements_named(&html, "center")[0];
        unwrap(&mut html, centre);
        assert_eq!(serialise(&html), "<p>a</p>b<em>c</em>de<p></p>");
        let mut html = parse("<div>x<span></span>y</div>");
        let span = elements_named(&html, "span")[0];
        unwrap(&mut html, span);
        assert_eq!(serialise(&html), "<div>xy</div>");
    }

    #[test]
    fn wrapping_moves_a_run_into_a_new_element() {
        let mut html = parse("<div>a<b>b</b>c<hr></div>");
        let div = elements_named(&html, "div")[0];
        let run: Vec<_> = children(&html, div).into_iter().take(3).collect();
        let p = wrap(
            &mut html,
            &run,
            new_element(local_name!("p"), &[("style", "text-align: center")]),
        );
        assert_eq!(
            serialise(&html),
            r#"<div><p style="text-align: center">a<b>b</b>c</p><hr></div>"#
        );
        assert_eq!(html_name(node(&html, p)), Some("p"));
    }

    #[test]
    fn attributes_are_set_in_place_or_inserted_and_stay_findable() {
        let mut html = parse(r#"<p title="t" style="color: red">x</p>"#);
        let p = elements_named(&html, "p")[0];
        edit_element(&mut html, p, |element| {
            set_attribute(element, "style", "text-align: center");
            set_attribute(element, "class", "a");
            set_attribute(element, "zz", "z");
        })
        .expect("an element");
        let element = node(&html, p).value().as_element().expect("an element");
        for (name, value) in [
            ("style", "text-align: center"),
            ("class", "a"),
            ("title", "t"),
            ("zz", "z"),
        ] {
            assert_eq!(element.attr(name), Some(value), "{name}");
        }
        // `style` is content-v1's attribute of `p`; the others follow in
        // name order.
        assert_eq!(
            serialise(&html),
            r#"<p style="text-align: center" class="a" title="t" zz="z">x</p>"#
        );
        let text = node(&html, p).first_child().expect("a text node").id();
        assert!(edit_element(&mut html, text, |_| ()).is_none());
    }

    #[test]
    fn content_v1_attributes_are_written_in_their_canonical_order() {
        let image = r#"<img src="/storage/uploads/a.jpg" alt="Kép" width="350" height="495" style="display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px">"#;
        assert_eq!(round_trip(image), image);
        let link =
            r#"<a href="https://example.org/" target="_blank" rel="noopener noreferrer">l</a>"#;
        assert_eq!(round_trip(link), link);
        // Outside content-v1 the tree's order, which is by name.
        assert_eq!(
            round_trip(r#"<font size="2" color="red" face="x">a</font>"#),
            r#"<font color="red" face="x" size="2">a</font>"#
        );
        // An SVG `a` is not the content-v1 `a`.
        assert_eq!(
            round_trip(r#"<svg><a target="t" href="h"></a></svg>"#),
            r#"<svg><a href="h" target="t"></a></svg>"#
        );
    }

    /// Text with the references and characters that N0 and N8 change.
    fn arb_text() -> impl Strategy<Value = String> {
        let pieces = prop::sample::select(
            &[
                "a",
                "Á",
                "ő",
                " ",
                "\n",
                "\r\n",
                "&eacute;",
                "&Ouml;",
                "&#337;",
                "&#x171;",
                "&ndash;",
                "&bdquo;",
                "&rdquo;",
                "&hellip;",
                "&nbsp;",
                "\u{a0}",
                "&amp;",
                "&lt;",
                "&gt;",
                "&quot;",
                "&#39;",
                "\"",
                "'",
                "&",
                "&copy",
                "&unknown;",
            ][..],
        );
        prop::collection::vec(pieces, 0..12).prop_map(|pieces| pieces.concat())
    }

    /// Elements without raw text, so every text is escaped when written.
    fn arb_body() -> impl Strategy<Value = String> {
        let tags = prop::sample::select(
            &[
                "p", "P", "h2", "strong", "a", "span", "td", "center", "div", "li",
            ][..],
        );
        prop::collection::vec((tags, arb_text(), arb_text(), any::<bool>()), 0..8).prop_map(
            |parts| {
                parts
                    .into_iter()
                    .map(|(tag, attribute, text, close)| -> String {
                        let end = if close {
                            format!("</{tag}>")
                        } else {
                            String::new()
                        };
                        format!(
                            r#"<{tag} title="{}">{text}{end}"#,
                            attribute.replace('"', "&quot;")
                        )
                    })
                    .collect::<Vec<_>>()
                    .concat()
            },
        )
    }

    /// Whether every `&` in `html` starts one of the five references N8
    /// writes.
    fn only_five_references(html: &str) -> bool {
        html.match_indices('&').all(|(at, _)| {
            ["&amp;", "&lt;", "&gt;", "&nbsp;", "&quot;"]
                .iter()
                .any(|reference| html[at..].starts_with(reference))
        })
    }

    proptest! {
        // N8: no character reference other than the five survives, and N0
        // leaves no carriage return. (A round trip need not settle at once:
        // `<h2><a><h2>` nests a heading in a heading, which the next parse
        // closes. The normaliser's fixed-point loop handles that.)
        #[test]
        fn a_round_trip_leaves_only_five_references(body in arb_body()) {
            let once = round_trip(&body);
            prop_assert!(only_five_references(&once), "{:?}", once);
            prop_assert!(!once.contains('\r'), "{:?}", once);
            let twice = round_trip(&once);
            prop_assert!(only_five_references(&twice), "{:?}", twice);
        }

        #[test]
        fn white_space_is_ascii_white_space(text in any::<String>()) {
            prop_assert_eq!(
                is_white_space(&text),
                text.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
            );
        }

        // Unwrapping keeps every node, in order: the text is unchanged.
        #[test]
        fn unwrapping_keeps_the_text(words in prop::collection::vec("[a-zé]{1,4}", 0..6)) {
            let inner = tagged("em", &words);
            let body = format!("<div>x<span>{inner}</span>y</div>");
            let mut html = parse(&body);
            let span = elements_named(&html, "span")[0];
            unwrap(&mut html, span);
            prop_assert_eq!(serialise(&html), format!("<div>x{inner}y</div>"));
        }

        // A run of any length moves into the wrapper in order.
        #[test]
        fn wrapping_keeps_the_run_in_order(words in prop::collection::vec("[a-zé]{1,4}", 1..6), take in 1usize..6) {
            let inner = tagged("b", &words);
            let mut html = parse(&format!("<div>{inner}</div>"));
            let div = elements_named(&html, "div")[0];
            let all = children(&html, div);
            let take = take.min(all.len());
            wrap(&mut html, &all[..take], new_element(local_name!("p"), &[]));
            let (wrapped, rest) = words.split_at(take);
            let wrapped = tagged("b", wrapped);
            let rest = tagged("b", rest);
            prop_assert_eq!(serialise(&html), format!("<div><p>{wrapped}</p>{rest}</div>"));
        }

        // Every attribute stays findable, whatever is set and in which order.
        #[test]
        fn set_attributes_read_back(
            existing in prop::collection::btree_set("[a-z]{1,3}", 0..5),
            set in prop::collection::vec(("[a-z]{1,3}", any::<String>()), 0..5),
        ) {
            let mut attributes = String::new();
            for name in &existing {
                let _ = write!(attributes, r#" {name}="old""#);
            }
            let mut html = parse(&format!("<span{attributes}>a</span>"));
            let span = elements_named(&html, "span")[0];
            let mut expected: std::collections::BTreeMap<String, String> =
                existing.iter().map(|name| (name.clone(), "old".to_owned())).collect();
            for (name, value) in &set {
                edit_element(&mut html, span, |element| set_attribute(element, name, value));
                expected.insert(name.clone(), value.clone());
            }
            let element = node(&html, span).value().as_element().expect("an element");
            prop_assert_eq!(element.attrs.len(), expected.len());
            for (name, value) in &expected {
                prop_assert_eq!(element.attr(name), Some(value.as_str()), "{}", name);
            }
        }
    }
}
