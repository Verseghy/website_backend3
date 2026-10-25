//! The [`Sanitizer`]: ammonia 4.2.4 configured from the content-v1 tables,
//! the structural pass, the fixed-point loop around both, the
//! [`SanitizedHtml`] type, and the removal report of each run (editor.md
//! §7.2, §7.5, ADR 0013 §5).
//!
//! [`Sanitizer::sanitize`] first bounds the work (input size, then nesting
//! depth by a linear scan), then repeats one pass, ammonia followed by the
//! structural pass, until a pass returns its own input. That output `z` has
//! `P(z) == z`, so sanitising it again returns it after one pass: this is
//! what makes `S(S(x)) == S(x)` hold byte for byte. At most
//! [`MAX_PASSES`] passes run; an input that is still changing is refused
//! (`content_not_normalisable`), because it means the parser and the
//! serializer disagree. One pass is enough for editor output. Any other
//! input usually settles in its first pass, because the structural pass
//! already parses ammonia's output afresh, as the next reader would (text
//! that an unwrapped `caption` leaves in a table moves in front of it then,
//! editor.md §3.5); the second pass confirms it.
//!
//! The third pass confirms a second change. Each pass parses twice, in
//! ammonia and in the structural pass, and the first pass's structural pass
//! can write a tree that the second pass's ammonia parse closes: a heading
//! directly inside a heading, or an `li` inside an `li`. The structural
//! pass's own unwrapping makes the first: in `<sub><h2><sub><h2>x`,
//! removing the inner `sub` makes the headings parent and child. So does
//! the adoption agency, when the structural pass's parse meets a link that
//! ammonia's parse left inside another link, and moves the heading between
//! them up into the outer heading. ammonia's parse leaves that nesting when
//! it fosters a link out of a table into another link
//! (`tests/fixtures/regressions/link-fostered-into-link`), and also, with
//! no table, when eight or more blocks stand between a link start tag and
//! the open link, because the adoption agency stops after eight rounds and
//! leaves a copy of the outer link open around the new one. And so does
//! foster parenting in the structural pass's parse, of what an unwrapped
//! `caption` left in a table. Nothing changes after that first parse of the
//! second pass: by then no element that ammonia unwraps is left to leave
//! content in a table, the structural pass has unwrapped every link nested
//! in a link, and a parse of such markup only closes elements, which builds
//! a tree that parses back into itself ([`super::structure`], "How the
//! output settles", has the sketch). The worst case is therefore three
//! passes with one parse to spare: the second pass's structural parse never
//! changes anything. Before nested links were unwrapped, the adoption
//! agency's limit of eight rounds made the number of passes grow with the
//! nesting depth, and a link around 24 quotes was refused
//! (`nested-link-deeper-than-eight-blocks`). ADR 0037 adds the unwrapping
//! of nested links to the structural pass of ADR 0013 §5.
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
//! the structural pass ([`super::structure`]), so the output is the same
//! in every process (editor.md §13.6, P5).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use ammonia::{Builder, UrlRelative};

use super::attrs;
use super::depth::exceeds_depth;
use super::policy::{
    ELEMENTS, LINK_REL, PolicyError, REMOVED_WITH_CONTENT, SanitizePolicy, URL_SCHEMES, ValueRule,
};
use super::report::{RemovalReport, explain};
use super::structure::{Violation, restructure};
use super::url_rules;

/// The most passes one run makes before it refuses the input (editor.md
/// §7.2, ADR 0013 §5).
pub const MAX_PASSES: usize = 3;

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

/// Why the sanitiser refused an input. Nothing is written for a refused
/// input; [`Self::code`] gives the admin error code of editor.md §7.6.
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
    /// The input nests elements deeper than the policy's
    /// `max_nesting_depth`, as the depth scan counts them. The count can be
    /// too high (implied end tags, and the simpler count after unusual SVG
    /// or `MathML`); both use this one variant and code, as the maintainer
    /// confirmed (item D87, research: DECISIONS R15).
    #[error("the content nests elements more than {limit} levels deep (line {line})")]
    TooDeep {
        /// The policy's limit.
        limit: usize,
        /// The line of the first start tag that went deeper, or of the SVG
        /// or `MathML` markup after which the depth scan counts every tag as
        /// open ([`super::depth`]).
        line: u64,
    },
    /// A pass produced more than the policy's `max_output_bytes`.
    #[error(
        "the sanitised content would be {actual} bytes long, more than the limit of {limit} bytes"
    )]
    OutputTooLarge {
        /// The policy's limit.
        limit: usize,
        /// The length of the pass's output.
        actual: usize,
    },
    /// The output was still changing after [`MAX_PASSES`] passes. This
    /// means the parser and the serializer disagree about some input, so the
    /// caller also logs it as an error (editor.md §7.2).
    #[error("the content did not reach a fixed point in {passes} sanitiser passes")]
    NotNormalisable {
        /// How many passes ran.
        passes: usize,
    },
    /// The structural pass found an invariant of editor.md §7.5 broken.
    /// This is a bug: an admin write answers `500` and logs it, a read
    /// serves an empty body and raises the alert metric.
    #[error("the sanitiser broke one of its own invariants: {0}")]
    InvariantViolated(Violation),
}

impl SanitizeError {
    /// The admin error code of editor.md §7.6 that a refused admin write
    /// answers with `422`, or `None` for [`Self::InvariantViolated`], which
    /// is a bug and answers `500`.
    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        match self {
            Self::TooLarge { .. } | Self::OutputTooLarge { .. } => Some("content_too_large"),
            Self::TooDeep { .. } => Some("content_too_deep"),
            Self::NotNormalisable { .. } => Some("content_not_normalisable"),
            Self::InvariantViolated(_) => None,
        }
    }
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

    /// Sanitises `input` into content-v1 and reports what it removed, over
    /// every pass of the fixed-point loop (see the module documentation).
    ///
    /// - `input`: an HTML fragment, for example a post body.
    ///
    /// # Errors
    ///
    /// - [`SanitizeError::TooLarge`] when `input` is longer than the
    ///   policy's `max_input_bytes`, and [`SanitizeError::TooDeep`] when it
    ///   nests deeper than `max_nesting_depth`; both are refused before
    ///   anything is parsed.
    /// - [`SanitizeError::OutputTooLarge`] when a pass writes more than
    ///   `max_output_bytes`.
    /// - [`SanitizeError::NotNormalisable`] when the output is still
    ///   changing after [`MAX_PASSES`] passes.
    /// - [`SanitizeError::InvariantViolated`] when the structural pass finds
    ///   a broken invariant, which is a bug.
    pub fn sanitize(&self, input: &str) -> Result<Sanitized, SanitizeError> {
        let limit = self.policy.max_input_bytes;
        if input.len() > limit {
            return Err(SanitizeError::TooLarge {
                limit,
                actual: input.len(),
            });
        }
        let limit = self.policy.max_nesting_depth;
        if let Some(exceeded) = exceeds_depth(input, limit) {
            return Err(SanitizeError::TooDeep {
                limit,
                line: exceeded.line,
            });
        }
        let mut report = RemovalReport::default();
        let (html, _) = fixed_point(input, MAX_PASSES, |current| self.pass(current, &mut report))?;
        Ok(Sanitized {
            html: SanitizedHtml(html),
            report,
        })
    }

    /// One pass: ammonia, then the structural pass. Their removals are
    /// added to `report`.
    fn pass(&self, input: &str, report: &mut RemovalReport) -> Result<String, SanitizeError> {
        let cleaned = self.builder.clean(input).to_string();
        // Checked before the structural pass parses it again.
        self.check_output(&cleaned)?;
        // Output equal to the input byte for byte means ammonia removed and
        // rewrote nothing, so its part of the report is empty without the
        // second parse. Editor saves, the common write, are such fixed
        // points (editor.md §7.2). The proptest
        // `sanitising_the_output_again_reports_nothing` checks that
        // `explain` agrees on every sanitised output.
        if cleaned != input {
            report.merge(explain(&self.policy, input));
        }
        let structured = restructure(&cleaned, report).map_err(SanitizeError::InvariantViolated)?;
        self.check_output(&structured)?;
        Ok(structured)
    }

    /// Refuses a pass output above the policy's `max_output_bytes`.
    fn check_output(&self, output: &str) -> Result<(), SanitizeError> {
        let limit = self.policy.max_output_bytes;
        if output.len() > limit {
            return Err(SanitizeError::OutputTooLarge {
                limit,
                actual: output.len(),
            });
        }
        Ok(())
    }
}

/// Applies `pass` until its output equals its input, at most `max_passes`
/// times, and returns that output with the number of passes it took. It is
/// generic over the pass so that the loop's own rules (stop at the first
/// fixed point, refuse after the last pass) are tested without depending on
/// an input that ammonia and html5ever fail to settle.
///
/// - `input`: the first input.
/// - `max_passes`: how many passes may run; the last one may only confirm.
/// - `pass`: one sanitiser pass; its error stops the loop.
fn fixed_point(
    input: &str,
    max_passes: usize,
    mut pass: impl FnMut(&str) -> Result<String, SanitizeError>,
) -> Result<(String, usize), SanitizeError> {
    let mut current = Cow::Borrowed(input);
    for passes in 1..=max_passes {
        let output = pass(&current)?;
        if output == *current {
            return Ok((output, passes));
        }
        current = Cow::Owned(output);
    }
    Err(SanitizeError::NotNormalisable { passes: max_passes })
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

    /// One ammonia pass alone, without the structural pass: the tests that
    /// pin ammonia's own behaviour read this, because the structural pass
    /// would hide it.
    fn ammonia(input: &str) -> String {
        sanitizer().builder.clean(input).to_string()
    }

    /// The output and the number of passes the fixed-point loop takes.
    fn passes(input: &str) -> (String, usize) {
        let sanitizer = sanitizer();
        let mut report = RemovalReport::default();
        fixed_point(input, MAX_PASSES, |current| {
            sanitizer.pass(current, &mut report)
        })
        .expect("converges")
    }

    const YOUTUBE: &str = r#"<p data-youtube-video=""><iframe src="https://www.youtube-nocookie.com/embed/VIDEOID?start=30" width="640" height="360" title="Videó" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>"#;
    const LINK: &str = r#"<p><a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a></p>"#;

    // editor.md §13.6, "Pinned ammonia behaviour". An ammonia upgrade that
    // changes any of these fails here.

    #[test]
    fn a_forced_attribute_with_its_canonical_value_stays_in_place() {
        assert_eq!(clean(YOUTUBE), YOUTUBE);
        assert_eq!(clean(LINK), LINK);
        // A case variant matches the allow-list and is overwritten in place;
        // the structural pass then writes the canonical order.
        let variant = r#"<a target="_BLANK" href="/x">x</a>"#;
        assert_eq!(
            ammonia(variant),
            r#"<a target="_blank" href="/x" rel="noopener noreferrer">x</a>"#
        );
        assert_eq!(
            clean(variant),
            r#"<a href="/x" target="_blank" rel="noopener noreferrer">x</a>"#
        );
    }

    #[test]
    fn rel_is_appended_last() {
        let input = r#"<a rel="nofollow" href="https://example.org/" target="_blank">x</a>"#;
        let expected =
            r#"<a href="https://example.org/" target="_blank" rel="noopener noreferrer">x</a>"#;
        assert_eq!(ammonia(input), expected);
        assert_eq!(clean(input), expected);
    }

    #[test]
    fn swap_remove_reorders_only_when_the_filter_drops() {
        let foreign = YOUTUBE.replace(
            "https://www.youtube-nocookie.com/embed/VIDEOID?start=30",
            "https://evil.example/embed/x",
        );
        // The filter drops `src` (index 0); `swap_remove` moves the last
        // attribute into its place.
        assert_eq!(
            ammonia(&foreign),
            r#"<p data-youtube-video=""><iframe referrerpolicy="strict-origin-when-cross-origin" width="640" height="360" title="Videó" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen=""></iframe></p>"#
        );
        // The structural pass restores the order.
        assert_eq!(
            clean(&foreign),
            r#"<p data-youtube-video=""><iframe width="640" height="360" title="Videó" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin"></iframe></p>"#
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
            let link = format!(r#"<a href="{href}">x</a>"#);
            assert_eq!(
                ammonia(&link),
                r#"<a rel="noopener noreferrer" target="_blank">x</a>"#,
                "{href}"
            );
            assert_eq!(
                clean(&link),
                r#"<a target="_blank" rel="noopener noreferrer">x</a>"#,
                "{href}"
            );
        }
    }

    #[test]
    fn data_images_are_dropped() {
        let input = r#"<p><img src="data:image/png;base64,iVBORw0KGgo=" alt="x"></p>"#;
        // ammonia drops the `src` and keeps the element ...
        assert_eq!(ammonia(input), r#"<p><img alt="x"></p>"#);
        // ... and the structural pass removes the image without `src`.
        let result = sanitizer().sanitize(input).unwrap();
        assert_eq!(result.html.as_str(), "<p></p>");
        assert_eq!(result.report.count(RemovalRule::ImageSrcRejected), 1);
        assert_eq!(result.report.count(RemovalRule::ImageWithoutSrcRemoved), 1);
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
        let input = "<table><caption>Cím</caption><tbody><tr><td>x</td></tr></tbody></table>";
        assert_eq!(
            ammonia(input),
            "<table>Cím<tbody><tr><td>x</td></tr></tbody></table>"
        );
        // The structural pass parses that again, and the parser moves the
        // text in front of the table (editor.md §3.5): the second pass
        // finds the fixed point.
        assert_eq!(
            passes(input),
            (
                "Cím<table><tbody><tr><td>x</td></tr></tbody></table>".to_owned(),
                2
            )
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
        let input = r#"<iframe src="https://padlet.com/embed/k" height="608"></iframe>"#;
        let output = ammonia(input);
        let attributes: HashMap<String, String> = attributes_of(&output, "iframe")
            .expect("an iframe")
            .into_iter()
            .collect();
        // ammonia appends them in hash order ...
        assert_eq!(attributes.len(), 5);
        assert_eq!(attributes["src"], "https://padlet.com/embed/k");
        assert_eq!(attributes["allow"], IFRAME_ALLOW);
        assert_eq!(attributes["allowfullscreen"], "");
        assert_eq!(attributes["referrerpolicy"], IFRAME_REFERRER_POLICY);
        assert!(
            output.starts_with(r#"<iframe src="https://padlet.com/embed/k" height="608" "#),
            "{output}"
        );
        // ... and the structural pass writes them in canonical order.
        assert_eq!(
            clean(input),
            format!(
                r#"<iframe src="https://padlet.com/embed/k" height="608" allow="{IFRAME_ALLOW}" allowfullscreen="" referrerpolicy="{IFRAME_REFERRER_POLICY}"></iframe>"#
            )
        );
    }

    #[test]
    fn ammonia_alone_appends_in_an_order_that_differs_between_instances() {
        // What P5 guards against (editor.md §7.4): every builder has its own
        // hash seeds, so the appended order varies. Sixteen builders all
        // agreeing by chance is about one in 10^12.
        let input = r#"<iframe src="https://padlet.com/embed/k"></iframe>"#;
        let orders: HashSet<String> = (0..16)
            .map(|_| allow_list().clean(input).to_string())
            .collect();
        assert!(orders.len() > 1, "{orders:?}");
        let sanitized: HashSet<String> = (0..16).map(|_| clean(input)).collect();
        assert_eq!(sanitized.len(), 1, "{sanitized:?}");
    }

    #[test]
    fn the_structural_pass_runs_inside_the_sanitizer() {
        let result = sanitizer()
            .sanitize(
                r#"<p data-padlet-embed=""><iframe src="https://padlet.com/embed/k">felirat</iframe></p><p>H<sub>2<sub>x</sub></sub>O <img alt="kép"></p>"#,
            )
            .unwrap();
        assert_eq!(
            result.html.as_str(),
            format!(
                r#"<p data-padlet-embed=""><iframe src="https://padlet.com/embed/k" allow="{IFRAME_ALLOW}" allowfullscreen="" referrerpolicy="{IFRAME_REFERRER_POLICY}"></iframe></p><p>H<sub>2x</sub>O </p>"#
            )
        );
        let per_rule = result.report.per_rule();
        assert_eq!(per_rule[&RemovalRule::IframeContentRemoved], 1);
        assert_eq!(per_rule[&RemovalRule::NestedSubSupUnwrapped], 1);
        assert_eq!(per_rule[&RemovalRule::ImageWithoutSrcRemoved], 1);
        assert_eq!(per_rule.len(), 3, "{per_rule:?}");
    }

    #[test]
    fn the_loop_stops_at_the_first_fixed_point() {
        // Canonical input: one pass. A repair: the second pass confirms it.
        assert_eq!(passes(LINK), (LINK.to_owned(), 1));
        assert_eq!(
            passes(r#"<a target="_blank" href="/x">x</a>"#),
            (
                r#"<a href="/x" target="_blank" rel="noopener noreferrer">x</a>"#.to_owned(),
                2
            )
        );
        // A heading inside a heading, left by an unwrapped `div`, is closed
        // by the fresh parse in the first pass.
        assert_eq!(
            passes("<h2>a<div><h3>b</h3></div></h2>"),
            ("<h2>a</h2><h3>b</h3>".to_owned(), 2)
        );
        // Tag soup that the parser repairs by foster parenting: ammonia
        // writes the repaired tree, and the structural pass's fresh parse
        // turns it into its final shape in the same pass.
        assert_eq!(
            passes("<ul><li><table><li>x</table></ul>"),
            ("<ul><li></li><li>x</li><table></table></ul>".to_owned(), 2)
        );
        // The structural pass's own unwrapping can leave a tree that parses
        // differently: the inner `sub` goes, the two headings become parent
        // and child, and the next parse closes the outer one. The third pass
        // confirms.
        assert_eq!(
            passes("<sub><h2><sub><h2>x"),
            ("<sub><h2></h2><h2>x</h2></sub>".to_owned(), 3)
        );
        // ammonia unwraps the `caption`, and the structural pass's parse
        // fosters the heading it held into the outer heading; the second
        // pass's first parse closes the outer one, and the third confirms.
        assert_eq!(
            passes("<h2><table><caption><h3>x</h3></caption></table></h2>"),
            ("<h2></h2><h3>x</h3><table></table>".to_owned(), 3)
        );
        // ammonia's parse fosters the second link out of the table into the
        // inner heading, inside the first link. The structural pass's parse
        // runs the adoption agency on it, which moves the inner heading up
        // into the outer one; the second pass's first parse closes that,
        // and the third pass confirms.
        let link = r#"href="/x" target="_blank" rel="noopener noreferrer""#;
        assert_eq!(
            passes(r#"<h2><a href="/x"><h3><table><a href="/x">"#),
            (
                format!(
                    "<h2><a {link}></a></h2><h3><a {link}></a><a {link}></a><table></table></h3>"
                ),
                3
            )
        );
        // The same without a table: nine blocks stand between the two links
        // (seven quotes and two headings), and the adoption agency in
        // ammonia's parse moves only eight of them, one per round, so a copy
        // of the first link stays open around the second. The structural
        // pass's parse runs the adoption agency on it again and moves the
        // inner heading into the outer one; the second pass's first parse
        // closes that, and the third pass confirms.
        let other = r#"href="/y" target="_blank" rel="noopener noreferrer""#;
        assert_eq!(
            passes(&format!(
                r#"<a href="/x">{}<h2><span><h3><a href="/y">x"#,
                "<blockquote>".repeat(7)
            )),
            (
                format!(
                    "<a {link}></a>{}<h2><a {link}><span></span></a></h2><h3><a {link}></a><a {other}>x</a></h3>{}",
                    format!("<blockquote><a {link}></a>").repeat(7),
                    "</blockquote>".repeat(7)
                ),
                3
            )
        );
        // Deeper than the adoption agency's eight rounds: the structural
        // pass's parse leaves the last copy of the outer link around the
        // fostered one, and the walk unwraps the fostered one. Before that
        // rule, every parse repaired eight more levels and this input was
        // refused after three passes.
        let (output, passes_taken) = passes(&format!(
            r#"<a href="/x">{}<table><a href="/y">x"#,
            "<blockquote>".repeat(24)
        ));
        assert_eq!(passes_taken, 2);
        assert_eq!(
            output,
            format!(
                "<a {link}></a>{}<blockquote><a {link}>{}x<table></table>{}</a></blockquote>{}",
                format!("<blockquote><a {link}></a>").repeat(7),
                "<blockquote>".repeat(16),
                "</blockquote>".repeat(16),
                "</blockquote>".repeat(7)
            )
        );
    }

    #[test]
    fn the_loop_refuses_what_does_not_settle() {
        let count = |output: &str| -> usize { output.len() };
        // A pass that always changes its input is refused after the last
        // pass, which could only have confirmed.
        let mut calls = 0;
        let refused = fixed_point("x", MAX_PASSES, |current| {
            calls += 1;
            Ok(format!("{current}x"))
        });
        assert_eq!(refused, Err(SanitizeError::NotNormalisable { passes: 3 }));
        assert_eq!(calls, 3);
        // Two changes, then a confirmation: accepted in exactly three.
        let settles = fixed_point("x", MAX_PASSES, |current| {
            Ok(if count(current) < 3 {
                format!("{current}x")
            } else {
                current.to_owned()
            })
        });
        assert_eq!(settles, Ok(("xxx".to_owned(), 3)));
        // An error from a pass stops the loop.
        let failed = fixed_point("x", MAX_PASSES, |_| {
            Err(SanitizeError::OutputTooLarge {
                limit: 1,
                actual: 2,
            })
        });
        assert_eq!(
            failed,
            Err(SanitizeError::OutputTooLarge {
                limit: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn deep_input_is_refused_before_parsing() {
        let deep = |depth: usize| format!("{}x{}", "<div>".repeat(depth), "</div>".repeat(depth));
        assert_eq!(clean(&deep(256)), "x");
        let error = sanitizer().sanitize(&deep(257)).unwrap_err();
        assert_eq!(
            error,
            SanitizeError::TooDeep {
                limit: 256,
                line: 1
            }
        );
        assert_eq!(error.code(), Some("content_too_deep"));
        assert_eq!(
            error.to_string(),
            "the content nests elements more than 256 levels deep (line 1)"
        );
        let shallow = Sanitizer::new(SanitizePolicy {
            max_nesting_depth: 32,
            ..SanitizePolicy::content_v1()
        })
        .unwrap();
        assert!(matches!(
            shallow.sanitize(&deep(33)),
            Err(SanitizeError::TooDeep { limit: 32, .. })
        ));
    }

    #[test]
    fn deep_input_that_leaves_foreign_content_is_refused_too() {
        // Bodies that leave `svg` for HTML content, where a scan that misread
        // foreign content would count too little (editor.md §7.2).
        let divs = "<div>".repeat(2_000);
        for input in [
            format!("<svg>{}", "<div/>".repeat(2_000)),
            format!("<svg><p><![CDATA[>{divs}"),
            format!("<svg><g></svg><![CDATA[>{divs}"),
            format!("<table><tr><td><svg><foreignObject><col><![CDATA[>{divs}"),
        ] {
            assert!(
                matches!(
                    sanitizer().sanitize(&input),
                    Err(SanitizeError::TooDeep {
                        limit: 256,
                        line: 1
                    })
                ),
                "{input:.60}"
            );
        }
        // A body full of well-formed SVG icons is followed to the end, not
        // refused, and the icons are removed with their content.
        let icon =
            r#"<p><svg viewBox="0 0 1 1"><title>Logo</title><path d="M0 0"/></svg> szöveg</p>"#;
        assert_eq!(clean(&icon.repeat(500)), "<p> szöveg</p>".repeat(500));
    }

    #[test]
    fn output_above_the_bound_is_refused() {
        let policy = SanitizePolicy {
            max_input_bytes: 1024,
            max_output_bytes: 1024,
            ..SanitizePolicy::content_v1()
        };
        let sanitizer = Sanitizer::new(policy).unwrap();
        // Every link gains `target` and `rel`: 7 bytes become 50.
        let error = sanitizer.sanitize(&"<a></a>".repeat(140)).unwrap_err();
        assert!(
            matches!(error, SanitizeError::OutputTooLarge { limit: 1024, actual } if actual > 1024),
            "{error:?}"
        );
        assert_eq!(error.code(), Some("content_too_large"));
        // A canonical body as large as the input bound is never refused for
        // its output, which is the same size.
        let body = format!("<p>{}</p>", "a".repeat(1017));
        assert_eq!(body.len(), 1024);
        assert_eq!(sanitizer.sanitize(&body).unwrap().html.as_str(), body);
    }

    #[test]
    fn every_error_has_its_code() {
        let violation = Violation::TableInsideParagraph;
        let cases = [
            (
                SanitizeError::TooLarge {
                    limit: 1,
                    actual: 2,
                },
                Some("content_too_large"),
            ),
            (
                SanitizeError::OutputTooLarge {
                    limit: 1,
                    actual: 2,
                },
                Some("content_too_large"),
            ),
            (
                SanitizeError::TooDeep { limit: 1, line: 2 },
                Some("content_too_deep"),
            ),
            (
                SanitizeError::NotNormalisable { passes: 3 },
                Some("content_not_normalisable"),
            ),
            (SanitizeError::InvariantViolated(violation.clone()), None),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code, "{error}");
        }
        assert_eq!(
            SanitizeError::NotNormalisable { passes: 3 }.to_string(),
            "the content did not reach a fixed point in 3 sanitiser passes"
        );
        assert_eq!(
            SanitizeError::InvariantViolated(violation).to_string(),
            "the sanitiser broke one of its own invariants: a `table` is inside a `p`"
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

        // The code depends on the kind of refusal only, never on the numbers
        // it carries.
        #[test]
        fn the_code_ignores_the_numbers(limit in any::<usize>(), actual in any::<usize>(), line in any::<u64>()) {
            prop_assert_eq!(SanitizeError::TooLarge { limit, actual }.code(), Some("content_too_large"));
            prop_assert_eq!(SanitizeError::OutputTooLarge { limit, actual }.code(), Some("content_too_large"));
            prop_assert_eq!(SanitizeError::TooDeep { limit, line }.code(), Some("content_too_deep"));
            prop_assert_eq!(SanitizeError::NotNormalisable { passes: actual }.code(), Some("content_not_normalisable"));
            let violation = SanitizeError::InvariantViolated(Violation::ElementOutsidePolicy { element: limit.to_string() });
            prop_assert_eq!(violation.code(), None);
        }

        // Every error names its numbers, so the operator sees the limit.
        #[test]
        fn the_messages_name_the_limits(limit in any::<usize>(), actual in any::<usize>(), line in any::<u64>()) {
            let too_large = SanitizeError::TooLarge { limit, actual }.to_string();
            prop_assert!(too_large.contains(&limit.to_string()) && too_large.contains(&actual.to_string()));
            let too_deep = SanitizeError::TooDeep { limit, line }.to_string();
            prop_assert!(too_deep.contains(&limit.to_string()) && too_deep.contains(&line.to_string()));
            let output = SanitizeError::OutputTooLarge { limit, actual }.to_string();
            prop_assert!(output.contains(&limit.to_string()) && output.contains(&actual.to_string()));
        }

        // The ammonia part of the report is checked against ammonia's real
        // output: every input attribute that does not come out of one
        // ammonia pass byte-identical has exactly one entry in `explain`,
        // and nothing else has one.
        #[test]
        fn the_report_accounts_for_every_changed_attribute(
            tag in prop::sample::select(&["a", "img", "iframe", "ol", "p", "span", "h2"][..]),
            attributes in prop::collection::vec((prop::sample::select(POOL_NAMES), prop::sample::select(POOL_VALUES)), 0..8),
        ) {
            let rendered = attributes.iter().map(|(n, v)| format!(r#" {n}="{}""#, v.replace('&', "&amp;").replace('"', "&quot;"))).collect::<Vec<_>>().concat();
            let input = if tag == "ol" { format!("<ol{rendered}><li>x</li></ol>") } else { format!("<{tag}{rendered}>x</{tag}>") };
            let output = ammonia(&input);
            let before = attributes_of(&input, tag).unwrap();
            let after = attributes_of(&output, tag).unwrap();
            let mut expected: Vec<String> = before.iter().filter(|attribute| !after.contains(attribute)).map(|(n, _)| n.clone()).collect();
            let mut reported = Vec::new();
            for (removal, count) in explain(&SanitizePolicy::content_v1(), &input).entries() {
                prop_assert_eq!(removal.element.as_str(), tag);
                let attribute = removal.attribute.clone();
                prop_assert!(attribute.is_some(), "{:?}", removal);
                reported.extend(std::iter::repeat_n(attribute.unwrap_or_default(), count));
            }
            expected.sort();
            reported.sort();
            prop_assert_eq!(expected, reported, "{} -> {}", input, output);
        }

        // The full report holds the ammonia part and the structural part:
        // the image without `src` and the iframe text are each counted once.
        #[test]
        fn the_full_report_adds_the_structural_removals(
            tag in prop::sample::select(&["img", "iframe"][..]),
            attributes in prop::collection::vec((prop::sample::select(POOL_NAMES), prop::sample::select(POOL_VALUES)), 0..8),
        ) {
            let rendered = attributes.iter().map(|(n, v)| format!(r#" {n}="{}""#, v.replace('&', "&amp;").replace('"', "&quot;"))).collect::<Vec<_>>().concat();
            let input = format!("<p><{tag}{rendered}>x</{tag}></p>");
            let result = sanitizer().sanitize(&input).unwrap();
            let mut expected = explain(&SanitizePolicy::content_v1(), &input);
            let single_pass = ammonia(&input);
            if tag == "iframe" {
                expected.record(RemovalRule::IframeContentRemoved, "iframe", None);
            } else if attributes_of(&single_pass, "img").is_some_and(|kept| !kept.iter().any(|(n, _)| n == "src")) {
                expected.record(RemovalRule::ImageWithoutSrcRemoved, "img", None);
            }
            prop_assert_eq!(&result.report, &expected, "{} -> {}", input, result.html);
        }

        // The element counts are checked against ammonia's real output: the
        // elements it kept and the text that survived must match the same
        // prediction that the counts of `explain` are compared with.
        #[test]
        fn the_report_matches_ammonia_over_nested_trees(trees in prop::collection::vec(arb_tree(), 0..4)) {
            let (mut input, mut next, mut expected) = (String::new(), 0, Expected::default());
            for tree in &trees {
                render(tree, &mut input, &mut next, true, &mut expected);
            }
            let (elements, text) = walk_output(&ammonia(&input));
            let survived: Vec<usize> = (0..next).filter(|id| text.contains(&format!("[{id}]"))).collect();
            prop_assert_eq!(&survived, &expected.texts, "{}", input);
            prop_assert_eq!(elements, expected.kept, "{}", input);
            let report = explain(&SanitizePolicy::content_v1(), &input);
            prop_assert_eq!(report.count(RemovalRule::ElementRemoved), expected.removed, "{}", input);
            prop_assert_eq!(report.count(RemovalRule::ElementUnwrapped), expected.unwrapped, "{}", input);
            prop_assert_eq!(report.count(RemovalRule::CommentRemoved), expected.comments, "{}", input);
            prop_assert_eq!(report.count(RemovalRule::ImageSrcRejected), expected.images, "{}", input);
            let total: usize = report.per_rule().values().sum();
            prop_assert_eq!(total, expected.removed + expected.unwrapped + expected.comments + expected.images);
            // The whole sanitiser keeps the same text, removes every one of
            // those images, and reports each of them once more.
            let result = sanitizer().sanitize(&input).unwrap();
            let (_, text) = walk_output(result.html.as_str());
            let survived: Vec<usize> = (0..next).filter(|id| text.contains(&format!("[{id}]"))).collect();
            prop_assert_eq!(&survived, &expected.texts, "{}", input);
            prop_assert!(!result.html.as_str().contains("<img"), "{}", result.html);
            prop_assert_eq!(result.report.count(RemovalRule::ImageWithoutSrcRemoved), expected.images);
        }

        #[test]
        fn sanitising_the_output_again_reports_nothing(
            parts in prop::collection::vec(prop::sample::select(&[
                "<p>", "</p>", "<div>", "<b>", "<script>x</script>", "<!--c-->", "<a href=\"/x\">", "</a>",
                "<a href=\"javascript:x\">", "<img src=\"/storage/uploads/a.jpg\" alt=\"\" width=\"0\">",
                "<table>", "<caption>", "<tr>", "<td colspan=\"2\" style=\"x\">", "</td>", "</table>",
                "<ol type=\"I\" start=\"0\">", "<li>", "<span class=\"x\">", "szöveg", "&amp;", "<br>",
                "<svg>", "<iframe src=\"https://padlet.com/embed/k\" width=\"100%\">", "</iframe>",
                "<h1>", "<sub>", "<sup>", "<o:p>", "<textarea>", "<plaintext>", "<g>", "<desc>", "<sum>",
                "<img alt=\"x\">", "<thead>", "<h2>", "<h3>",
            ][..]), 0..24),
        ) {
            let once = sanitizer().sanitize(&parts.concat()).unwrap().html;
            // `explain` itself, not `sanitize`, whose shortcut would skip it.
            let again = explain(&SanitizePolicy::content_v1(), once.as_str());
            prop_assert!(again.is_empty(), "{} -> {:?}", once, again);
            // And the whole sanitiser takes one pass and changes nothing.
            prop_assert_eq!(passes(once.as_str()), (once.as_str().to_owned(), 1));
            let twice = sanitizer().sanitize(once.as_str()).unwrap();
            prop_assert_eq!(&twice.html, &once);
            prop_assert!(twice.report.is_empty(), "{:?}", twice.report);
        }

        // P5 (editor.md §13.6): independent instances, each with its own
        // hash seeds, write the same bytes and the same report.
        #[test]
        fn independent_instances_agree(
            parts in prop::collection::vec(prop::sample::select(&[
                "<iframe src=\"https://padlet.com/embed/k\">", "<iframe height=\"1\" title=\"t\">", "</iframe>",
                "<a>", "<a href=\"//evil.example\" target=\"_self\" rel=\"x\">", "</a>",
                "<img width=\"1\" alt=\"a\" src=\"data:x\" height=\"2\">", "<img alt=\"a\" src=\"/storage/uploads/a.jpg\" class=\"c\" width=\"9\">",
                "<ol type=\"A\" class=\"x\" start=\"3\">", "<li>", "<td colspan=\"9\" rowspan=\"0\">", "<table>", "x",
            ][..]), 0..16),
        ) {
            let input = parts.concat();
            let first = sanitizer().sanitize(&input).unwrap();
            for _ in 0..3 {
                prop_assert_eq!(&sanitizer().sanitize(&input).unwrap(), &first, "{}", input);
            }
        }

        // A link around nesting deeper than the adoption agency repairs in
        // one parse, with a table at the bottom that fosters a second link
        // into it, either at once or, from a `caption` that ammonia
        // unwraps, in the structural pass's parse. The loop settles within
        // its passes, and the output is a fixed point (module
        // documentation). Before nested links were unwrapped, a link around
        // 24 quotes was refused.
        #[test]
        fn nested_links_settle_at_any_depth(
            levels in prop::collection::vec(prop::sample::select(&[
                "<blockquote>", "<ul><li>", "<li>", "<h2>", "<h3>", "<p>", "<em>", "<sub>",
                "<span>", "<div>", "<b>", "<a href=\"/z\">",
            ][..]), 0..120),
            caption in any::<bool>(),
        ) {
            let inner = if caption {
                r#"<caption><a href="/y">x</a></caption>"#
            } else {
                r#"<a href="/y">x"#
            };
            let input = format!(r#"<a href="/x">{}<table>{inner}"#, levels.concat());
            let result = sanitizer().sanitize(&input);
            prop_assert!(result.is_ok(), "{}: {:?}", input, result);
            let html = result.expect("checked above").html.into_string();
            prop_assert_eq!(clean(&html), html);
        }
    }
}
