//! N6: links to the dead host's `/storage/` become root-relative
//! (data-migration.md §7.1, editor.md §9, ADR 0017 §2 step 7).
//!
//! `backend.verseghy-gimnazium.net` was the legacy admin's host. It has no
//! DNS record, so the images and links that name it are broken on the live
//! site, while the files themselves are on the uploads volume that backend3
//! serves at `/storage/` (ADR 0015). N6 rewrites, in every `src` and `href`
//! attribute (in no namespace, on any element), a value that starts with
//! `https://backend.verseghy-gimnazium.net/storage/` or with its `http:`
//! form so that it starts with `/storage/` instead; the rest of the value
//! is kept byte for byte.
//!
//! The scheme and the host are compared ASCII case-insensitively, because a
//! URL's scheme and host are (`HTTPS://Backend.…` names the same dead
//! host); `/storage/` is compared exactly, because a path is not, and the
//! file server would not find `/STORAGE/`. Nothing else is rewritten: a
//! value with white space before the URL, a port, user information, another
//! path, a protocol-relative `//backend.…` URL, or the host in any other
//! attribute (`srcset`, `data-src`, `style`) or in text. N6 reports those
//! instead ([`occurrences`]), so that editors can review them
//! (data-migration.md §13 item 4). For images, the sanitiser's own rule
//! also turns an absolute URL on any of backend3's hosts, the retired one
//! included, into a root-relative path (editor.md §3.6); links need N6.
//!
//! The report scans attribute values and text nodes, not comments, which
//! no reader sees and N7 removes.

use html5ever::ns;
use html5ever::tendril::StrTendril;
use scraper::{Html, Node};

use super::dom;
use super::report::{DeadHostOccurrence, NormaliseReport, Place, Rule};

/// The legacy admin host, which has no DNS record.
pub(super) const DEAD_HOST: &str = "backend.verseghy-gimnazium.net";

/// The two origins whose `/storage/` N6 rewrites.
const DEAD_ORIGINS: [&str; 2] = [
    "https://backend.verseghy-gimnazium.net",
    "http://backend.verseghy-gimnazium.net",
];

/// The path prefix that both the dead host and backend3 serve uploads under.
const STORAGE: &str = "/storage/";

/// How many characters an excerpt keeps on each side of the host.
const EXCERPT_CHARS: usize = 40;

/// The root-relative form of `value`, if it is a URL on the dead host's
/// `/storage/` (see the module documentation), or `None`.
///
/// - `value`: an attribute value, with character references decoded.
pub(super) fn rewritten(value: &str) -> Option<&str> {
    DEAD_ORIGINS.iter().find_map(|origin| {
        let head = value.as_bytes().get(..origin.len())?;
        if !head.eq_ignore_ascii_case(origin.as_bytes()) {
            return None;
        }
        // The matched bytes are ASCII, so `origin.len()` is a character
        // boundary of `value`.
        let rest = &value[origin.len()..];
        rest.starts_with(STORAGE).then_some(rest)
    })
}

/// Applies N6 to every `src` and `href` of `html` and counts each rewrite
/// in `report`.
///
/// - `html`: a parsed body.
/// - `report`: the report of the current run.
pub(super) fn rewrite_links(html: &mut Html, report: &mut NormaliseReport) {
    let elements: Vec<_> = html
        .root_element()
        .descendants()
        .filter(|node| node.value().is_element())
        .map(|node| node.id())
        .collect();
    for id in elements {
        let rewrites = dom::edit_element(html, id, |element| {
            let mut rewrites = 0;
            for (name, value) in &mut element.attrs {
                if name.ns != ns!() || !matches!(&*name.local, "src" | "href") {
                    continue;
                }
                if let Some(relative) = rewritten(value).map(StrTendril::from) {
                    *value = relative;
                    rewrites += 1;
                }
            }
            rewrites
        })
        .unwrap_or(0);
        for _ in 0..rewrites {
            report.record(Rule::DeadHostLinkRewritten);
        }
    }
}

/// Every attribute value and text node of `html` that names the dead host,
/// in document order.
///
/// - `html`: a parsed body, after N6.
pub(super) fn occurrences(html: &Html) -> Vec<DeadHostOccurrence> {
    let root = dom::content(html);
    let mut found = Vec::new();
    for node in html.root_element().descendants() {
        match node.value() {
            Node::Element(element) => {
                for (name, value) in &element.attrs {
                    if let Some((count, excerpt)) = find(value) {
                        found.push(DeadHostOccurrence {
                            place: Place::Attribute {
                                element: element.name.local.to_string(),
                                attribute: match &name.prefix {
                                    Some(prefix) => format!("{prefix}:{}", name.local),
                                    None => name.local.to_string(),
                                },
                            },
                            count,
                            excerpt,
                        });
                    }
                }
            }
            Node::Text(text) => {
                if let Some((count, excerpt)) = find(text) {
                    let parent = node.parent().expect("text below the root has a parent");
                    found.push(DeadHostOccurrence {
                        place: Place::Text {
                            element: (parent.id() != root)
                                .then(|| parent.value().as_element())
                                .flatten()
                                .map(|element| element.name.local.to_string()),
                        },
                        count,
                        excerpt,
                    });
                }
            }
            _ => {}
        }
    }
    found
}

/// How many times `value` names the dead host (ASCII case-insensitively,
/// not overlapping), and the excerpt around the first time; `None` when it
/// does not.
///
/// - `value`: an attribute value or the content of a text node.
fn find(value: &str) -> Option<(usize, String)> {
    // ASCII lower-casing keeps every byte offset.
    let lower = value.to_ascii_lowercase();
    let mut matches = lower.match_indices(DEAD_HOST);
    let (first, _) = matches.next()?;
    Some((1 + matches.count(), excerpt(value, first, DEAD_HOST.len())))
}

/// The part of `value` around the `len` bytes at `at`: at most
/// [`EXCERPT_CHARS`] characters on each side, with `…` where it was cut.
///
/// - `value`: the whole text.
/// - `at`: the byte offset of the part, on a character boundary.
/// - `len`: its length in bytes, ending on a character boundary.
pub(super) fn excerpt(value: &str, at: usize, len: usize) -> String {
    let start = value[..at]
        .char_indices()
        .rev()
        .nth(EXCERPT_CHARS - 1)
        .map_or(0, |(i, _)| i);
    let end = value[at + len..]
        .char_indices()
        .nth(EXCERPT_CHARS)
        .map_or(value.len(), |(i, _)| at + len + i);
    let mut out = String::with_capacity(end - start + 6);
    if start > 0 {
        out.push('…');
    }
    out.push_str(&value[start..end]);
    if end < value.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// N6 over `body`: the output, the report and the occurrences left.
    fn n6(body: &str) -> (String, NormaliseReport, Vec<DeadHostOccurrence>) {
        let mut html = dom::parse(body);
        let mut report = NormaliseReport::default();
        rewrite_links(&mut html, &mut report);
        let left = occurrences(&html);
        (dom::serialise(&html), report, left)
    }

    #[test]
    fn both_storage_prefixes_become_root_relative() {
        assert_eq!(
            rewritten(
                "https://backend.verseghy-gimnazium.net/storage/uploads/posts/12/kép (1).jpg"
            ),
            Some("/storage/uploads/posts/12/kép (1).jpg")
        );
        assert_eq!(
            rewritten("http://backend.verseghy-gimnazium.net/storage/a.pdf?x=1#y"),
            Some("/storage/a.pdf?x=1#y")
        );
        assert_eq!(
            rewritten("HTTPS://Backend.Verseghy-Gimnazium.NET/storage/a.png"),
            Some("/storage/a.png")
        );
    }

    #[test]
    fn nothing_else_is_rewritten() {
        for value in [
            "https://backend.verseghy-gimnazium.net/",
            "https://backend.verseghy-gimnazium.net/storage",
            "https://backend.verseghy-gimnazium.net/STORAGE/a.png",
            "https://backend.verseghy-gimnazium.net/admin/storage/a.png",
            "https://backend.verseghy-gimnazium.net:443/storage/a.png",
            "https://user@backend.verseghy-gimnazium.net/storage/a.png",
            "//backend.verseghy-gimnazium.net/storage/a.png",
            " https://backend.verseghy-gimnazium.net/storage/a.png",
            "ftp://backend.verseghy-gimnazium.net/storage/a.png",
            "https://backend.verseghy-gimnazium.network/storage/a.png",
            "https://verseghy-gimnazium.net/storage/a.png",
            "/storage/a.png",
            "https://backend.verseghy-gimnázium.net/storage/a.png",
            "",
            "h",
        ] {
            assert_eq!(rewritten(value), None, "{value:?}");
        }
    }

    #[test]
    fn src_and_href_are_rewritten_and_counted() {
        let (out, report, left) = n6(concat!(
            r#"<p><img src="https://backend.verseghy-gimnazium.net/storage/uploads/posts/12/a.jpg" alt="Kép">"#,
            r#"<a href="http://backend.verseghy-gimnazium.net/storage/uploads/posts/12/b.pdf">PDF</a>"#,
            r#"<iframe src="https://backend.verseghy-gimnazium.net/storage/c.html"></iframe></p>"#,
        ));
        assert_eq!(
            out,
            concat!(
                r#"<p><img src="/storage/uploads/posts/12/a.jpg" alt="Kép">"#,
                r#"<a href="/storage/uploads/posts/12/b.pdf">PDF</a>"#,
                r#"<iframe src="/storage/c.html"></iframe></p>"#,
            )
        );
        assert_eq!(report.count(Rule::DeadHostLinkRewritten), 3);
        assert_eq!(left, []);
    }

    #[test]
    fn other_occurrences_are_reported_unchanged() {
        let body = concat!(
            r#"<p><a href="https://backend.verseghy-gimnazium.net/admin">régi admin</a> "#,
            r#"<img data-src="https://backend.verseghy-gimnazium.net/storage/x.jpg"></p>"#,
            "BACKEND.verseghy-gimnazium.net és backend.verseghy-gimnazium.net",
            r#"<svg><a xlink:href="http://backend.verseghy-gimnazium.net/storage/y"></a></svg>"#,
            "<!-- backend.verseghy-gimnazium.net -->",
        );
        let (out, report, left) = n6(body);
        assert_eq!(out, dom::serialise(&dom::parse(body)), "nothing changed");
        assert!(!report.has_hits());
        let host = || "backend.verseghy-gimnazium.net".to_owned();
        assert_eq!(
            left,
            [
                DeadHostOccurrence {
                    place: Place::Attribute {
                        element: "a".to_owned(),
                        attribute: "href".to_owned()
                    },
                    count: 1,
                    excerpt: "https://backend.verseghy-gimnazium.net/admin".to_owned(),
                },
                DeadHostOccurrence {
                    place: Place::Attribute {
                        element: "img".to_owned(),
                        attribute: "data-src".to_owned()
                    },
                    count: 1,
                    excerpt: "https://backend.verseghy-gimnazium.net/storage/x.jpg".to_owned(),
                },
                DeadHostOccurrence {
                    place: Place::Text { element: None },
                    count: 2,
                    excerpt: "BACKEND.verseghy-gimnazium.net és backend.verseghy-gimnazium.net"
                        .to_owned(),
                },
                DeadHostOccurrence {
                    place: Place::Attribute {
                        element: "a".to_owned(),
                        attribute: "xlink:href".to_owned()
                    },
                    count: 1,
                    excerpt: format!("http://{}/storage/y", host()),
                },
            ]
        );
    }

    #[test]
    fn text_occurrences_name_their_element() {
        let (_, _, left) =
            n6("<p>Lásd: <strong>https://backend.verseghy-gimnazium.net/</strong></p>");
        assert_eq!(
            left.iter().map(|o| o.place.clone()).collect::<Vec<_>>(),
            [Place::Text {
                element: Some("strong".to_owned())
            }]
        );
    }

    #[test]
    fn a_rewritten_value_that_still_names_the_host_is_reported() {
        let (out, report, left) = n6(
            r#"<a href="https://backend.verseghy-gimnazium.net/storage/a.pdf?from=backend.verseghy-gimnazium.net">x</a>"#,
        );
        assert_eq!(
            out,
            r#"<a href="/storage/a.pdf?from=backend.verseghy-gimnazium.net">x</a>"#
        );
        assert_eq!(report.count(Rule::DeadHostLinkRewritten), 1);
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn long_text_is_cut_around_the_host() {
        let before = "á".repeat(50);
        let after = "ő".repeat(45);
        let text = format!("{before}{DEAD_HOST}{after}");
        let at = before.len();
        let cut = excerpt(&text, at, DEAD_HOST.len());
        assert_eq!(
            cut,
            format!("…{}{DEAD_HOST}{}…", "á".repeat(40), "ő".repeat(40))
        );
        assert_eq!(excerpt(DEAD_HOST, 0, DEAD_HOST.len()), DEAD_HOST);
        let short = format!("ab{DEAD_HOST}c");
        assert_eq!(excerpt(&short, 2, DEAD_HOST.len()), short);
        let exactly = format!("{}{DEAD_HOST}{}", "x".repeat(40), "y".repeat(40));
        assert_eq!(excerpt(&exactly, 40, DEAD_HOST.len()), exactly);
    }

    /// Attribute values around the dead host's prefixes.
    fn arb_value() -> impl Strategy<Value = String> {
        (
            prop::sample::select(
                &[
                    "https://backend.verseghy-gimnazium.net",
                    "http://backend.verseghy-gimnazium.net",
                    "HTTP://BACKEND.VERSEGHY-GIMNAZIUM.NET",
                    "https://backend.verseghy-gimnazium.net:443",
                    "//backend.verseghy-gimnazium.net",
                    "https://verseghy-gimnazium.net",
                    " https://backend.verseghy-gimnazium.net",
                    "",
                ][..],
            ),
            prop::sample::select(
                &["/storage/", "/STORAGE/", "/", "/storage", "/x/storage/", ""][..],
            ),
            "[a-zé0-9/._ ()-]{0,12}",
        )
            .prop_map(|(origin, path, rest)| format!("{origin}{path}{rest}"))
    }

    proptest! {
        // A rewrite drops exactly one of the two origins in front of
        // `/storage/`, and anything that is not such a URL stays.
        #[test]
        fn a_rewrite_only_drops_the_origin(value in arb_value()) {
            match rewritten(&value) {
                Some(relative) => {
                    prop_assert!(relative.starts_with(STORAGE));
                    let origin = &value[..value.len() - relative.len()];
                    prop_assert!(DEAD_ORIGINS.iter().any(|o| o.eq_ignore_ascii_case(origin)), "{:?}", value);
                }
                None => prop_assert!(!DEAD_ORIGINS.iter().any(|origin| {
                    let bytes = value.as_bytes();
                    bytes.get(..origin.len()).is_some_and(|head| head.eq_ignore_ascii_case(origin.as_bytes()))
                        && bytes[origin.len()..].starts_with(STORAGE.as_bytes())
                }), "{:?}", value),
            }
        }

        #[test]
        fn any_text_is_safe_to_check(value in any::<String>()) {
            if let Some(relative) = rewritten(&value) {
                prop_assert!(value.ends_with(relative));
            }
        }

        // After N6 no `src` or `href` starts with a dead `/storage/` URL,
        // and every rewrite is counted.
        #[test]
        fn no_dead_storage_link_survives(values in prop::collection::vec((arb_value(), prop::sample::select(&["src", "href", "data-src"][..])), 0..6)) {
            let body = values
                .iter()
                .map(|(value, name)| format!(r#"<a {name}="{value}">x</a>"#))
                .collect::<Vec<_>>()
                .concat();
            let expected = values.iter().filter(|(value, name)| *name != "data-src" && rewritten(value).is_some()).count();
            let (out, report, _) = n6(&body);
            prop_assert_eq!(report.count(Rule::DeadHostLinkRewritten), expected);
            let html = dom::parse(&out);
            for node in html.root_element().descendants() {
                if let Some(element) = node.value().as_element() {
                    for name in ["src", "href"] {
                        prop_assert!(element.attr(name).and_then(rewritten).is_none(), "{:?}", out);
                    }
                }
            }
        }

        // The excerpt always holds the host and stays bounded.
        #[test]
        fn an_excerpt_holds_the_host(before in "\\PC{0,60}", after in "\\PC{0,60}") {
            let text = format!("{before}{DEAD_HOST}{after}");
            let cut = excerpt(&text, before.len(), DEAD_HOST.len());
            prop_assert!(cut.contains(DEAD_HOST));
            prop_assert!(cut.chars().count() <= DEAD_HOST.len() + 2 * EXCERPT_CHARS + 2);
            prop_assert_eq!(find(&text).map(|(count, _)| count >= 1), Some(true));
        }
    }
}
