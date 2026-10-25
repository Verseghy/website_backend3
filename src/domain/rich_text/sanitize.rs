//! The [`Sanitizer`]: ammonia 4.2.4 configured from the content-v1 tables,
//! the [`SanitizedHtml`] type, and the removal report of each run
//! (editor.md §7.2, ADR 0013 §5).
//!
//! `Builder::empty()` clears only the tag list, so every other `Builder`
//! field is set here explicitly; an ammonia default can never decide what is
//! stored. `filter_style_properties` is the one setting never called: its
//! filter re-serialises tokens without a grammar and keeps `https:` `url()`
//! values (ADR 0013 §5), and `style` goes through the attribute filter
//! instead.
//!
//! The byte-exact output relies on ammonia internals that the tests below
//! pin (editor.md §7.4, §13.6, "Pinned ammonia behaviour"):
//!
//! - its allow-list keeps attribute order (`retain`);
//! - a forced attribute that the allow-list admitted with its canonical
//!   value is overwritten in place, while a missing one is appended in the
//!   order of a randomly seeded `HashMap`;
//! - `link_rel` appends `rel` after `retain` removed the original;
//! - an attribute that the filter or the relative-URL rule drops is removed
//!   with `swap_remove`, which moves the last attribute into its place.
//!
//! The `HashMap` seed is the standard library's, not an input this module
//! reads: it only decides where ammonia appends a missing forced attribute.
//!
//! The appended and swapped attributes are put back into canonical order by
//! the structural pass of editor.md §7.5, which, with the fixed-point loop,
//! the CSS grammar and the manifest, is not part of this module yet. Until
//! then `sanitize` is one ammonia pass, and its attribute order is
//! byte-stable only for input that already has every forced attribute.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use ammonia::{Builder, UrlRelative};

use super::attrs;
use super::policy::{
    ELEMENTS, LINK_REL, PolicyError, REMOVED_WITH_CONTENT, SanitizePolicy, URL_SCHEMES, ValueRule,
};
use super::report::{RemovalReport, explain};
use super::url_rules;

/// HTML that [`Sanitizer::sanitize`] produced.
///
/// The field is private and nothing else constructs it, so code that takes a
/// `SanitizedHtml` (the content write service) cannot be handed bytes that
/// skipped the sanitiser without a compile error (ADR 0013 §5):
///
/// ```compile_fail,E0423
/// use backend3::domain::rich_text::SanitizedHtml;
///
/// let forged = SanitizedHtml(String::from("<script>alert(1)</script>"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SanitizedHtml(String);

impl SanitizedHtml {
    /// The HTML.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The HTML as an owned string, for writing it.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for SanitizedHtml {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SanitizedHtml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The result of one sanitiser run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    /// The sanitised HTML.
    pub html: SanitizedHtml,
    /// What was removed or rewritten, per rule.
    pub report: RemovalReport,
}

/// Why the sanitiser refused an input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SanitizeError {
    /// The input is larger than the policy's `max_input_bytes`.
    #[error("the content is {actual} bytes long, more than the limit of {limit} bytes")]
    TooLarge {
        /// The policy's limit.
        limit: usize,
        /// The input's length.
        actual: usize,
    },
}

/// The content-v1 sanitiser. Built once at boot, shared, and passed to
/// every write path as an argument (editor.md §7.2).
#[derive(Debug)]
pub struct Sanitizer {
    policy: Arc<SanitizePolicy>,
    builder: Builder<'static>,
}

impl Sanitizer {
    /// Builds the sanitiser for `policy`.
    ///
    /// - `policy`: the deployment's policy; it is validated first.
    ///
    /// # Errors
    ///
    /// The [`PolicyError`] of [`SanitizePolicy::validate`].
    pub fn new(policy: SanitizePolicy) -> Result<Self, PolicyError> {
        policy.validate()?;
        let policy = Arc::new(policy);
        let builder = builder(Arc::clone(&policy));
        Ok(Self { policy, builder })
    }

    /// The policy this sanitiser was built from.
    #[must_use]
    pub fn policy(&self) -> &SanitizePolicy {
        &self.policy
    }

    /// Sanitises `input` into content-v1 and reports what it removed. The
    /// report is derived from a second parse of `input`, which is skipped
    /// when the output equals the input.
    ///
    /// - `input`: an HTML fragment, for example a post body.
    ///
    /// # Errors
    ///
    /// [`SanitizeError::TooLarge`] when `input` is longer than the policy's
    /// `max_input_bytes`; it is refused before it is parsed.
    pub fn sanitize(&self, input: &str) -> Result<Sanitized, SanitizeError> {
        let limit = self.policy.max_input_bytes;
        if input.len() > limit {
            return Err(SanitizeError::TooLarge {
                limit,
                actual: input.len(),
            });
        }
        let html = self.builder.clean(input).to_string();
        // Output equal to the input byte for byte means ammonia removed and
        // rewrote nothing, so the report is empty without the second parse.
        // Editor saves, the common write, are such fixed points (editor.md
        // §7.2). The proptest `sanitising_the_output_again_reports_nothing`
        // checks that `explain` agrees on every sanitised output.
        let report = if html == input {
            RemovalReport::default()
        } else {
            explain(&self.policy, input)
        };
        Ok(Sanitized {
            html: SanitizedHtml(html),
            report,
        })
    }
}

/// The ammonia builder for content-v1 (the sketch of editor.md §7.2, pinned
/// against ammonia 4.2.4): [`allow_list`] plus the attribute filter.
fn builder(policy: Arc<SanitizePolicy>) -> Builder<'static> {
    let mut builder = allow_list();
    builder.attribute_filter(move |element, attribute, value| {
        attrs::filter(&policy, element, attribute, value)
    });
    builder
}

/// Every `Builder` setting except the attribute filter: the tag lists, the
/// attribute allow-lists with the forced values, the schemes, the
/// relative-URL rule, `rel`, comments and ids. It is a function of its own
/// so that a test can pin the allow-list, which the filter would hide.
fn allow_list() -> Builder<'static> {
    let mut tag_attributes: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    let mut tag_attribute_values: HashMap<
        &'static str,
        HashMap<&'static str, HashSet<&'static str>>,
    > = HashMap::new();
    let mut forced: HashMap<&'static str, HashMap<&'static str, &'static str>> = HashMap::new();
    for element in ELEMENTS {
        for attribute in element.attributes {
            let (tag, name) = (element.name, attribute.name);
            match attribute.rule {
                // Listed only in `tag_attribute_values`: an attribute listed
                // in both maps is admitted with any value (editor.md §7.2).
                ValueRule::Enumerated { values, .. } => {
                    tag_attribute_values
                        .entry(tag)
                        .or_default()
                        .entry(name)
                        .or_default()
                        .extend(values);
                }
                // Admitted with its one canonical value, so a correct value
                // is overwritten in place instead of re-appended (§7.4).
                ValueRule::Forced(value) => {
                    tag_attribute_values
                        .entry(tag)
                        .or_default()
                        .entry(name)
                        .or_default()
                        .insert(value);
                    forced.entry(tag).or_default().insert(name, value);
                }
                // `link_rel` asserts that `rel` is on no allow-list.
                ValueRule::LinkRel => {}
                _ => {
                    tag_attributes.entry(tag).or_default().insert(name);
                }
            }
        }
    }
    let mut builder = Builder::empty();
    builder
        .tags(ELEMENTS.iter().map(|element| element.name).collect())
        .clean_content_tags(REMOVED_WITH_CONTENT.iter().copied().collect())
        .generic_attributes(HashSet::new())
        .generic_attribute_prefixes(HashSet::new())
        .allowed_classes(HashMap::new())
        .tag_attributes(tag_attributes)
        .tag_attribute_values(tag_attribute_values)
        .set_tag_attribute_values(forced)
        .url_schemes(URL_SCHEMES.iter().copied().collect())
        .url_relative(UrlRelative::Custom(Box::new(
            url_rules::same_origin_relative,
        )))
        .link_rel(Some(LINK_REL))
        .strip_comments(true)
        .id_prefix(None);
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::policy::{IFRAME_ALLOW, IFRAME_REFERRER_POLICY, element_rule};
    use crate::domain::rich_text::report::{AMMONIA_FOREIGN_TAGS, RemovalRule, parse_like_ammonia};
    use proptest::prelude::*;
    use scraper::Node;
    use std::fmt::Write as _;

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is valid")
    }

    fn clean(input: &str) -> String {
        sanitizer()
            .sanitize(input)
            .expect("small input")
            .html
            .into_string()
    }

    const YOUTUBE: &str = r#"<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/VIDEOID?start=30" width="640" height="360" title="Videó" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>"#;
    const LINK: &str = r#"<p><a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a></p>"#;

    // editor.md §13.6, "Pinned ammonia behaviour". An ammonia upgrade that
    // changes any of these fails here.

    #[test]
    fn a_forced_attribute_with_its_canonical_value_stays_in_place() {
        assert_eq!(clean(YOUTUBE), YOUTUBE);
        assert_eq!(clean(LINK), LINK);
        // A case variant matches the allow-list and is overwritten in place.
        assert_eq!(
            clean(r#"<a target="_BLANK" href="/x">x</a>"#),
            r#"<a target="_blank" href="/x" rel="noopener noreferrer">x</a>"#
        );
    }

    #[test]
    fn rel_is_appended_last() {
        assert_eq!(
            clean(r#"<a rel="nofollow" href="https://example.org/" target="_blank">x</a>"#),
            r#"<a href="https://example.org/" target="_blank" rel="noopener noreferrer">x</a>"#
        );
    }

    #[test]
    fn swap_remove_reorders_only_when_the_filter_drops() {
        let foreign = YOUTUBE.replace(
            "https://www.youtube-nocookie.com/embed/VIDEOID?start=30",
            "https://evil.example/embed/x",
        );
        // The filter drops `src` (index 0); `swap_remove` moves the last
        // attribute into its place. The structural pass restores the order.
        assert_eq!(
            clean(&foreign),
            r#"<p data-youtube-video=""><iframe referrerpolicy="strict-origin-when-cross-origin" width="640" height="360" title="Videó" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen=""></iframe></p>"#
        );
    }

    #[test]
    fn scheme_relative_and_backslash_links_are_rejected() {
        for href in [
            "//evil.example/x",
            "/\\evil.example/x",
            "\\\\evil.example",
            "&#9;//evil.example",
            "/&#10;/evil.example",
        ] {
            // The relative-URL step runs after `target` and `rel` were
            // appended and drops `href` with `swap_remove` too, so `rel`
            // moves to the front; the structural pass restores the order.
            assert_eq!(
                clean(&format!(r#"<a href="{href}">x</a>"#)),
                r#"<a rel="noopener noreferrer" target="_blank">x</a>"#,
                "{href}"
            );
        }
    }

    #[test]
    fn data_images_are_dropped() {
        let result = sanitizer()
            .sanitize(r#"<p><img src="data:image/png;base64,iVBORw0KGgo=" alt="x"></p>"#)
            .unwrap();
        // The image without `src` is removed by the structural pass.
        assert_eq!(result.html.as_str(), r#"<p><img alt="x"></p>"#);
        assert_eq!(result.report.count(RemovalRule::ImageSrcRejected), 1);
    }

    #[test]
    fn entity_obfuscated_javascript_is_dropped() {
        for href in [
            "java&#x73;cript:alert(1)",
            "jav&#x09;ascript:alert(1)",
            " JAVASCRIPT:alert(1)",
            "vbscript:x",
        ] {
            assert_eq!(
                clean(&format!(r#"<a href="{href}">x</a>"#)),
                r#"<a target="_blank" rel="noopener noreferrer">x</a>"#,
                "{href}"
            );
        }
    }

    #[test]
    fn every_link_gets_rel() {
        assert_eq!(
            clean("<a>x</a>"),
            r#"<a target="_blank" rel="noopener noreferrer">x</a>"#
        );
        assert_eq!(
            clean(r#"<a href="/x" rel="">x</a>"#),
            r#"<a href="/x" target="_blank" rel="noopener noreferrer">x</a>"#
        );
    }

    #[test]
    fn no_empty_style_is_ever_emitted() {
        assert_eq!(clean(r#"<p style="">x</p>"#), "<p>x</p>");
        assert_eq!(clean(r#"<p style="text-align: center">x</p>"#), "<p>x</p>");
        assert!(!clean(r#"<table style="border-collapse: collapse"><tbody><tr><td style="width: 10px">x</td></tr></tbody></table>"#).contains("style"));
    }

    #[test]
    fn caption_text_stays_inside_the_table() {
        assert_eq!(
            clean("<table><caption>Cím</caption><tbody><tr><td>x</td></tr></tbody></table>"),
            "<table>Cím<tbody><tr><td>x</td></tr></tbody></table>"
        );
    }

    #[test]
    fn foreign_tag_names_are_removed_with_their_content() {
        // ammonia's namespace check removes these HTML elements with their
        // content; `report::explain` mirrors the list.
        for name in AMMONIA_FOREIGN_TAGS {
            let result = sanitizer()
                .sanitize(&format!("<p>a<{name}>titok</{name}>b</p>"))
                .unwrap();
            assert_eq!(result.html.as_str(), "<p>ab</p>", "{name}");
            assert_eq!(
                result.report.count(RemovalRule::ElementRemoved),
                1,
                "{name}"
            );
        }
        for name in ["font", "mark", "o:p", "clippath", "foreignobject", "mrows"] {
            assert_eq!(
                clean(&format!("<p>a<{name}>marad</{name}>b</p>")),
                "<p>amaradb</p>",
                "{name}"
            );
        }
    }

    #[test]
    fn the_allow_list_alone_admits_only_the_listed_attributes() {
        // Without the attribute filter, so the filter cannot hide a wider
        // allow-list: unlisted names go, listed ones stay with any value,
        // and enumerated and forced ones only with an allowed value.
        let bare = allow_list();
        let clean = |input: &str| bare.clean(input).to_string();
        assert_eq!(
            clean(r#"<span style="s" title="t" class="c" id="i" lang="hu" data-x="1">x</span>"#),
            r#"<span style="s">x</span>"#
        );
        assert_eq!(
            clean(
                r#"<p style="s" data-youtube-video="v" data-padlet-embed="p" align="center">x</p>"#
            ),
            r#"<p style="s" data-youtube-video="v" data-padlet-embed="p">x</p>"#
        );
        assert_eq!(
            clean(r#"<img src="/x.png" alt="a" width="w" height="h" style="s" srcset="x">"#),
            r#"<img src="/x.png" alt="a" width="w" height="h" style="s">"#
        );
        assert_eq!(
            clean(
                r#"<ol start="s" type="disc"><li title="t">x</li></ol><ol type="I"><li>y</li></ol>"#
            ),
            r#"<ol start="s"><li>x</li></ol><ol type="I"><li>y</li></ol>"#
        );
        assert_eq!(
            clean(
                r#"<table style="s" width="1"><tbody style="s"><tr style="s"><th colspan="c" rowspan="r" style="s" width="1">x</th></tr></tbody></table>"#
            ),
            r#"<table style="s"><tbody style="s"><tr><th colspan="c" rowspan="r" style="s">x</th></tr></tbody></table>"#
        );
        assert_eq!(
            clean(r#"<a href="/x" target="_self" title="t">x</a>"#),
            r#"<a href="/x" target="_blank" rel="noopener noreferrer">x</a>"#
        );
        let iframe = clean(
            r#"<iframe src="https://padlet.com/embed/k" width="w" height="h" title="t" style="s" frameborder="0" name="n"></iframe>"#,
        );
        assert!(
            iframe.starts_with(r#"<iframe src="https://padlet.com/embed/k" width="w" height="h" title="t" style="s" "#)
                && !iframe.contains("frameborder")
                && !iframe.contains("name="),
            "{iframe}"
        );
    }

    // The rest of the allow-list, element by element.

    #[test]
    fn removed_elements_take_their_content_and_others_are_unwrapped() {
        assert_eq!(clean("<p>a<script>alert(1)</script>b</p>"), "<p>ab</p>");
        assert_eq!(
            clean("<xml><o:DocumentProperties>Szerző</o:DocumentProperties></xml><p>x</p>"),
            "<p>x</p>"
        );
        assert_eq!(clean("<svg><a href='/x'>x</a></svg>y"), "y");
        assert_eq!(
            clean("<div><b>vastag</b> <font face='Calibri'>szöveg</font><o:p></o:p></div>"),
            "vastag szöveg"
        );
        assert_eq!(clean("<p>a<!-- megjegyzés -->b</p>"), "<p>ab</p>");
        assert_eq!(clean("<h5>Cím</h5><center>x</center>"), "Címx");
    }

    #[test]
    fn unlisted_attributes_are_removed() {
        assert_eq!(
            clean(
                r#"<p class="MsoNormal" id="x" lang="hu" dir="ltr" onclick="x" data-x="1" aria-label="x" role="note">x</p>"#
            ),
            "<p>x</p>"
        );
        assert_eq!(
            clean(r#"<img src="/storage/uploads/a.jpg" srcset="/a.jpg 2x" alt="a" title="t">"#),
            r#"<img src="/storage/uploads/a.jpg" alt="a">"#
        );
        let iframe =
            clean(r#"<iframe srcdoc="<script></script>" frameborder="0" onload="x"></iframe>"#);
        assert!(
            !iframe.contains("srcdoc")
                && !iframe.contains("frameborder")
                && !iframe.contains("onload"),
            "{iframe}"
        );
    }

    #[test]
    fn values_are_canonicalised() {
        assert_eq!(
            clean(
                r#"<img src="https://verseghy-gimnazium.net/storage/uploads/a.jpg" alt="a" width="0350" height="5001">"#
            ),
            r#"<img src="/storage/uploads/a.jpg" alt="a" width="350">"#
        );
        assert_eq!(
            clean(r#"<ol start="1" type="1"><li>x</li></ol>"#),
            "<ol><li>x</li></ol>"
        );
        assert_eq!(
            clean(r#"<p data-padlet-embed="yes">x</p>"#),
            r#"<p data-padlet-embed="">x</p>"#
        );
        assert_eq!(
            clean(r#"<td colspan="1">x</td>"#),
            "x",
            "a lone cell is not a table"
        );
    }

    #[test]
    fn missing_forced_iframe_attributes_are_added() {
        let output = clean(r#"<iframe src="https://padlet.com/embed/k" height="608"></iframe>"#);
        let attributes: HashMap<String, String> = attributes_of(&output, "iframe")
            .expect("an iframe")
            .into_iter()
            .collect();
        // Appended in hash order; the structural pass sorts them.
        assert_eq!(attributes.len(), 5);
        assert_eq!(attributes["src"], "https://padlet.com/embed/k");
        assert_eq!(attributes["allow"], IFRAME_ALLOW);
        assert_eq!(attributes["allowfullscreen"], "");
        assert_eq!(attributes["referrerpolicy"], IFRAME_REFERRER_POLICY);
        assert!(
            output.starts_with(r#"<iframe src="https://padlet.com/embed/k" height="608" "#),
            "{output}"
        );
    }

    #[test]
    fn style_free_canonical_shapes_are_fixed_points() {
        for shape in [
            YOUTUBE,
            LINK,
            r#"<p data-padlet-embed=""><iframe src="https://padlet.com/embed/KEY" width="100%" height="608" title="Padlet" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>"#,
            "<h2>Eredmények</h2><p>Szöveg <strong>félkövér</strong>, <em>dőlt</em>, <u>a</u> <s>b</s> H<sub>2</sub>O x<sup>2</sup><br>új sor</p><hr>",
            r#"<ol start="5" type="a"><li>első</li><li><p>két</p><p>bekezdés</p></li></ol><ul><li>pont</li></ul><blockquote>Idézet</blockquote>"#,
            r#"<table><tbody><tr><th colspan="2">Fej</th></tr><tr><td rowspan="3">a</td><td>b &amp; c &lt; d</td></tr></tbody></table>"#,
            r#"<p><img src="/storage/uploads/posts/612/kep.jpg" alt="A díjazottak a színpadon" width="350" height="495"></p>"#,
            "<p>a\u{a0}b</p>",
        ] {
            let result = sanitizer().sanitize(shape).unwrap();
            assert_eq!(result.html.as_str(), shape.replace('\u{a0}', "&nbsp;"));
            assert!(result.report.is_empty(), "{shape}: {:?}", result.report);
        }
    }

    #[test]
    fn oversized_input_is_refused_before_parsing() {
        let policy = SanitizePolicy {
            max_input_bytes: 1024,
            ..SanitizePolicy::content_v1()
        };
        let sanitizer = Sanitizer::new(policy).unwrap();
        assert!(sanitizer.sanitize(&"a".repeat(1024)).is_ok());
        let error = sanitizer.sanitize(&"a".repeat(1025)).unwrap_err();
        assert_eq!(
            error,
            SanitizeError::TooLarge {
                limit: 1024,
                actual: 1025
            }
        );
        assert_eq!(
            error.to_string(),
            "the content is 1025 bytes long, more than the limit of 1024 bytes"
        );
    }

    #[test]
    fn an_invalid_policy_builds_no_sanitizer() {
        let policy = SanitizePolicy {
            own_hosts: vec!["Example.org".into()],
            ..SanitizePolicy::content_v1()
        };
        assert_eq!(Sanitizer::new(policy).unwrap_err().key, "own_hosts[0]");
        assert_eq!(sanitizer().policy(), &SanitizePolicy::content_v1());
    }

    #[test]
    fn the_sanitizer_can_be_shared_between_threads() {
        fn shared<T: Send + Sync + 'static>() {}
        shared::<Sanitizer>();
        let sanitizer = Arc::new(sanitizer());
        let worker = Arc::clone(&sanitizer);
        let output = std::thread::spawn(move || worker.sanitize(LINK).unwrap().html)
            .join()
            .unwrap();
        assert_eq!(output.as_ref(), LINK);
        assert_eq!(output.to_string(), LINK);
    }

    /// The attributes of the first element named `tag`, as (name, value).
    fn attributes_of(html: &str, tag: &str) -> Option<Vec<(String, String)>> {
        let parsed = parse_like_ammonia(html);
        parsed.tree.nodes().find_map(|node| match node.value() {
            Node::Element(element) if element.name() == tag => Some(
                element
                    .attrs()
                    .map(|(n, v)| (n.to_owned(), v.to_owned()))
                    .collect(),
            ),
            _ => None,
        })
    }

    const POOL_NAMES: &[&str] = &[
        "src",
        "href",
        "alt",
        "title",
        "width",
        "height",
        "style",
        "class",
        "id",
        "onclick",
        "target",
        "rel",
        "colspan",
        "rowspan",
        "start",
        "type",
        "allow",
        "allowfullscreen",
        "referrerpolicy",
        "data-youtube-video",
        "data-padlet-embed",
        "data-x",
        "srcset",
        "srcdoc",
        "frameborder",
    ];
    const POOL_VALUES: &[&str] = &[
        "",
        "1",
        "0350",
        "5001",
        "100%",
        "a",
        "_blank",
        "_self",
        "noopener noreferrer",
        "x y",
        "/storage/uploads/a.jpg",
        "https://verseghy-gimnazium.net/storage/uploads/b.png",
        "data:image/png;base64,AA",
        "javascript:alert(1)",
        "//evil.example/x",
        "/storage/x.svg",
        "https://www.youtube-nocookie.com/embed/abc?start=3",
        "https://padlet.com/embed/k",
        "https://evil.example/embed/x",
        "mailto:a@b.example",
        "#top",
        "color: red",
        IFRAME_ALLOW,
        IFRAME_REFERRER_POLICY,
        // Case variants that ammonia admits and then overwrites.
        "_BLANK",
        "STRICT-ORIGIN-WHEN-CROSS-ORIGIN",
        "ACCELEROMETER; AUTOPLAY; CLIPBOARD-WRITE; ENCRYPTED-MEDIA; GYROSCOPE; PICTURE-IN-PICTURE; WEB-SHARE",
        "I",
    ];

    /// A tree that the HTML parser keeps as written: in a `div` context and
    /// inside table cells these tags nest freely. `p`, `li`, `a`, headings,
    /// `button` and the raw-text elements are left out, because they close,
    /// move or swallow what follows them.
    #[derive(Debug, Clone)]
    enum Tree {
        Text,
        Comment,
        /// `<img src="data:…">`: kept, with its `src` rejected.
        Image,
        Element(&'static str, Vec<Tree>),
        /// One row of `td` and `th` cells.
        Table(Vec<Vec<Tree>>),
    }

    #[rustfmt::skip]
    const NESTING_TAGS: &[&str] = &[
        "span", "strong", "em", "sub", "blockquote", // content-v1
        "div", "b", "font", "center", "o:p", // unwrapped
        "object", "video", "xml", // clean_content_tags
        "g", "desc", "sum", "text", "list", // ammonia's namespace check
    ];

    fn arb_tree() -> impl Strategy<Value = Tree> {
        let leaf = prop_oneof![Just(Tree::Text), Just(Tree::Comment), Just(Tree::Image)];
        leaf.prop_recursive(4, 48, 4, |inner| {
            prop_oneof![
                (
                    prop::sample::select(NESTING_TAGS),
                    prop::collection::vec(inner.clone(), 0..4)
                )
                    .prop_map(|(tag, children)| Tree::Element(tag, children)),
                prop::collection::vec(prop::collection::vec(inner, 0..3), 1..3)
                    .prop_map(Tree::Table),
            ]
        })
    }

    /// What the walk of a generated tree predicts, from the tables alone.
    #[derive(Debug, Default)]
    struct Expected {
        kept: usize,
        removed: usize,
        unwrapped: usize,
        comments: usize,
        images: usize,
        /// The text markers outside every removed element.
        texts: Vec<usize>,
    }

    /// Writes `tree` as HTML with numbered text markers, and records what
    /// ammonia should do with it; `live` is false inside a removed element.
    fn render(
        tree: &Tree,
        out: &mut String,
        next: &mut usize,
        live: bool,
        expected: &mut Expected,
    ) {
        match tree {
            Tree::Text => {
                if live {
                    expected.texts.push(*next);
                }
                let _ = write!(out, "[{next}]");
                *next += 1;
            }
            Tree::Comment => {
                expected.comments += usize::from(live);
                out.push_str("<!--c-->");
            }
            Tree::Image => {
                expected.kept += usize::from(live);
                expected.images += usize::from(live);
                out.push_str(r#"<img src="data:x" alt="">"#);
            }
            Tree::Element(tag, children) => {
                let removed =
                    REMOVED_WITH_CONTENT.contains(tag) || AMMONIA_FOREIGN_TAGS.contains(tag);
                let count = if removed {
                    &mut expected.removed
                } else if element_rule(tag).is_some() {
                    &mut expected.kept
                } else {
                    &mut expected.unwrapped
                };
                *count += usize::from(live);
                let _ = write!(out, "<{tag}>");
                for child in children {
                    render(child, out, next, live && !removed, expected);
                }
                let _ = write!(out, "</{tag}>");
            }
            Tree::Table(cells) => {
                expected.kept += if live { 3 + cells.len() } else { 0 };
                out.push_str("<table><tbody><tr>");
                for (index, cell) in cells.iter().enumerate() {
                    let tag = if index % 2 == 0 { "td" } else { "th" };
                    let _ = write!(out, "<{tag}>");
                    for child in cell {
                        render(child, out, next, live, expected);
                    }
                    let _ = write!(out, "</{tag}>");
                }
                out.push_str("</tr></tbody></table>");
            }
        }
    }

    /// Counts the elements of ammonia's output and collects its text. The
    /// output is parsed again (ammonia exposes its tree only under
    /// `--cfg ammonia_unstable`); the tags of [`Tree`] keep their places in
    /// that parse, so it has ammonia's elements and text.
    fn walk_output(output: &str) -> (usize, String) {
        let parsed = parse_like_ammonia(output);
        let (mut elements, mut text) = (0, String::new());
        for node in parsed.root_element().descendants().skip(1) {
            match node.value() {
                Node::Element(_) => elements += 1,
                Node::Text(contents) => text.push_str(contents),
                _ => {}
            }
        }
        (elements, text)
    }

    proptest! {
        #[test]
        fn sanitize_never_panics(input in any::<String>()) {
            let _ = sanitizer().sanitize(&input);
        }

        // The report is checked against ammonia's real output: every input
        // attribute that does not come out byte-identical has exactly one
        // entry, and nothing else has one.
        #[test]
        fn the_report_accounts_for_every_changed_attribute(
            tag in prop::sample::select(&["a", "img", "iframe", "ol", "p", "span", "h2"][..]),
            attributes in prop::collection::vec((prop::sample::select(POOL_NAMES), prop::sample::select(POOL_VALUES)), 0..8),
        ) {
            let rendered = attributes.iter().map(|(n, v)| format!(r#" {n}="{}""#, v.replace('&', "&amp;").replace('"', "&quot;"))).collect::<Vec<_>>().concat();
            let input = if tag == "ol" { format!("<ol{rendered}><li>x</li></ol>") } else { format!("<{tag}{rendered}>x</{tag}>") };
            let result = sanitizer().sanitize(&input).unwrap();
            let before = attributes_of(&input, tag).unwrap();
            let after = attributes_of(result.html.as_str(), tag).unwrap();
            let mut expected: Vec<String> = before.iter().filter(|attribute| !after.contains(attribute)).map(|(n, _)| n.clone()).collect();
            let mut reported = Vec::new();
            for (removal, count) in result.report.entries() {
                prop_assert_eq!(removal.element.as_str(), tag);
                let attribute = removal.attribute.clone();
                prop_assert!(attribute.is_some(), "{:?}", removal);
                reported.extend(std::iter::repeat_n(attribute.unwrap_or_default(), count));
            }
            expected.sort();
            reported.sort();
            prop_assert_eq!(expected, reported, "{} -> {}", input, result.html);
        }

        // The element counts are checked against ammonia's real output: the
        // elements it kept and the text that survived must match the same
        // prediction that the report's counts are compared with.
        #[test]
        fn the_report_matches_ammonia_over_nested_trees(trees in prop::collection::vec(arb_tree(), 0..4)) {
            let (mut input, mut next, mut expected) = (String::new(), 0, Expected::default());
            for tree in &trees {
                render(tree, &mut input, &mut next, true, &mut expected);
            }
            let result = sanitizer().sanitize(&input).unwrap();
            let (elements, text) = walk_output(result.html.as_str());
            let survived: Vec<usize> = (0..next).filter(|id| text.contains(&format!("[{id}]"))).collect();
            prop_assert_eq!(&survived, &expected.texts, "{}", input);
            prop_assert_eq!(elements, expected.kept, "{}", input);
            let report = result.report;
            prop_assert_eq!(report.count(RemovalRule::ElementRemoved), expected.removed, "{}", input);
            prop_assert_eq!(report.count(RemovalRule::ElementUnwrapped), expected.unwrapped, "{}", input);
            prop_assert_eq!(report.count(RemovalRule::CommentRemoved), expected.comments, "{}", input);
            prop_assert_eq!(report.count(RemovalRule::ImageSrcRejected), expected.images, "{}", input);
            let total: usize = report.per_rule().values().sum();
            prop_assert_eq!(total, expected.removed + expected.unwrapped + expected.comments + expected.images);
        }

        #[test]
        fn sanitising_the_output_again_reports_nothing(
            parts in prop::collection::vec(prop::sample::select(&[
                "<p>", "</p>", "<div>", "<b>", "<script>x</script>", "<!--c-->", "<a href=\"/x\">", "</a>",
                "<a href=\"javascript:x\">", "<img src=\"/storage/uploads/a.jpg\" alt=\"\" width=\"0\">",
                "<table>", "<caption>", "<tr>", "<td colspan=\"2\" style=\"x\">", "</td>", "</table>",
                "<ol type=\"I\" start=\"0\">", "<li>", "<span class=\"x\">", "szöveg", "&amp;", "<br>",
                "<svg>", "<iframe src=\"https://padlet.com/embed/k\" width=\"100%\">", "</iframe>",
                "<h1>", "<sub>", "<o:p>", "<textarea>", "<plaintext>", "<g>", "<desc>", "<sum>",
            ][..]), 0..24),
        ) {
            // `explain` itself, not `sanitize`, whose shortcut would skip it here.
            let once = sanitizer().sanitize(&parts.concat()).unwrap().html;
            let again = explain(&SanitizePolicy::content_v1(), once.as_str());
            prop_assert!(again.is_empty(), "{} -> {:?}", once, again);
        }
    }
}
