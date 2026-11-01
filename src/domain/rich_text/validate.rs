//! The checks of an admin write's rich-text body, after the sanitiser has
//! run: [`validate_content`] and [`refusal`] (editor.md §7.6, ADR 0013 §5).
//!
//! The sanitiser repairs every input into content-v1, but some repairs lose
//! what the editor meant, so an admin write that needed them is refused
//! instead of stored: an image whose source the sanitiser dropped, an
//! embed from a host outside editor.md §3.6, text inside an embed. An image
//! without alt text is refused as well, because ADR 0013 §2 makes alt text
//! mandatory. The editor blocks all of these before it saves, so for editor
//! output this module finds nothing; it is the server's half of the rule,
//! for a request that skipped the editor or an editor bug. The importer and
//! `content_normalize` never call it: they record the same findings in
//! their reports (data-migration.md §4.3).
//!
//! Each finding is one [`ContentError`], the `{code, index, detail}` of
//! editor.md §7.6: a stable code, the 1-based position of the image or
//! embed in the submitted body, and the values that explain it. The admin
//! shows the Hungarian `errors.<code>` message next to the editor, filled
//! with them (ADR 0021 §4).
//!
//! # Where each finding comes from
//!
//! The sanitised output alone cannot show every case, because the
//! structural pass removes an `img` without `src` and the children of an
//! `iframe` (editor.md §7.5). So:
//!
//! - `image_src_rejected` comes from the removal report: one per `img` the
//!   structural pass removed ([`RemovalRule::ImageWithoutSrcRemoved`]),
//!   whether the filter dropped its `src` or it never had one. An `img`
//!   whose `src` the filter dropped is also counted under
//!   [`RemovalRule::ImageSrcRejected`]; that count is not added, so each
//!   image is refused once.
//! - `iframe_content` comes from the report too:
//!   [`RemovalRule::IframeContentRemoved`], once per `iframe`.
//! - `image_alt_missing` and `embed_rejected` come from the sanitised
//!   output: an `img` whose `alt` is missing or blank, and an `iframe` left
//!   without `src`.
//! - `content_too_large`, `content_too_deep` and `content_not_normalisable`
//!   are the sanitiser's own refusals, mapped by [`refusal`]: nothing was
//!   sanitised then, so there is nothing to validate. `content_too_large`
//!   covers two limits (research: DECISIONS R11): the submitted body's and
//!   that of what a sanitiser pass wrote. [`Measured`] says which, because a
//!   body under the input limit can still expand past the output limit, and
//!   a message that named only the output limit would quote a number the
//!   submitted body never reached.
//!
//! # How many findings
//!
//! At most [`MAX_FINDINGS`], the first ones in the order [`validate_content`]
//! documents. Every finding is small (a source is cut to [`EXCERPT_CHARS`]),
//! but a body within the input limit can hold hundreds of thousands of
//! images (`<img>` is five bytes), and a `422` that listed them all would be
//! many times larger than the request. A body with more findings is refused
//! with the first ones; the editor blocks every one of them before it saves,
//! so only a request that skipped the editor can reach the cap.
//!
//! # How a finding gets its index
//!
//! The original input is read only to explain a finding, and only when
//! there is one. It is parsed as ammonia parses it and walked as the removal
//! report walks it, skipping what ammonia removes with its content, so the
//! images and embeds it lists are the ones the editor showed, in document
//! order. Their sources are decided by the same function the report uses
//! (`report::url_outcome`). The walk is trusted only where it agrees with
//! the authority for that finding: the number of removed images and of
//! emptied embeds must equal the report's counts, and the kept images
//! (source and alt) and the embeds (source) must be the same multisets as
//! the output's. By construction they always agree; the proptests check it
//! over tag soup. Where they do not, the findings still come from the
//! authority, without an index, rather than with an index that could point
//! at the wrong image.
//!
//! Like everything under `src/domain/`, the module is pure (ADR 0008): the
//! policy, the output and the report come in as arguments.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::num::NonZeroUsize;

use scraper::{ElementRef, Node};

use super::policy::SanitizePolicy;
use super::report::{
    RemovalReport, RemovalRule, parse_like_ammonia, removed_with_content, url_outcome,
};
use super::sanitize::{SanitizeError, SanitizedHtml};

/// The most characters of an attribute value that a finding repeats from
/// the input; a longer value is cut there and ends in `…`. A rejected image
/// source can be a `data:` URL of several megabytes.
pub const EXCERPT_CHARS: usize = 100;

/// The most findings [`validate_content`] returns (see the module
/// documentation, "How many findings"). With a source cut to
/// [`EXCERPT_CHARS`], the list stays within a few tens of kilobytes of JSON.
pub const MAX_FINDINGS: usize = 100;

/// Which length a [`ContentError::ContentTooLarge`] measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Measured {
    /// The submitted body, against the policy's `max_input_bytes`.
    Input,
    /// What a sanitiser pass wrote, against the policy's
    /// `max_output_bytes`. The policy keeps that limit at or above the input
    /// limit, and the sanitiser leaves editor output unchanged, so only a
    /// body that the sanitiser expands reaches it.
    Output,
}

impl Measured {
    /// The `snake_case` name the admin API sends in the finding's `detail`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

/// One reason an admin write's rich-text body is refused: the
/// `{code, index, detail}` of editor.md §7.6.
///
/// `index` is the 1-based position of the image among the body's images,
/// or of the embed among its embeds, in document order, as the editor
/// shows them; `None` when it could not be traced back to the input (see
/// the module documentation). The other fields are the detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentError {
    /// `image_alt_missing`: an image has no alt text, or only white space
    /// ([`is_blank`]).
    ImageAltMissing {
        /// The image's position among the images.
        index: Option<NonZeroUsize>,
        /// Its source, as the sanitiser kept it.
        src: Option<String>,
    },
    /// `image_src_rejected`: the sanitiser removed an image, because its
    /// source is not an uploaded image under the policy's image prefix
    /// (editor.md §3.6), or because it had none.
    ImageSrcRejected {
        /// The image's position among the images.
        index: Option<NonZeroUsize>,
        /// Its source in the input, cut to [`EXCERPT_CHARS`]; `None` when it
        /// had none or the index is unknown.
        src: Option<String>,
    },
    /// `embed_rejected`: an embed's source is not a `YouTube` or Padlet embed
    /// URL of editor.md §3.6, or it had none.
    EmbedRejected {
        /// The embed's position among the embeds.
        index: Option<NonZeroUsize>,
        /// Its source in the input, cut to [`EXCERPT_CHARS`]; `None` when it
        /// had none or the index is unknown.
        src: Option<String>,
    },
    /// `iframe_content`: an embed had child nodes, which the structural
    /// pass removed (editor.md §7.5).
    IframeContent {
        /// The embed's position among the embeds.
        index: Option<NonZeroUsize>,
    },
    /// `content_too_large`: the input is longer than the policy's
    /// `max_input_bytes`, or a sanitiser pass wrote more than its
    /// `max_output_bytes`.
    ContentTooLarge {
        /// Which of the two lengths went over its limit.
        measured: Measured,
        /// The limit that was exceeded, in bytes.
        limit: usize,
        /// The length that exceeded it, in bytes.
        actual: usize,
    },
    /// `content_too_deep`: the depth scan counted more nested elements than
    /// the policy's `max_nesting_depth` (editor.md §7.2).
    ContentTooDeep {
        /// The limit, in levels.
        limit: usize,
        /// The input line where the scan went deeper, or where it switched
        /// to its simpler count.
        line: u64,
    },
    /// `content_not_normalisable`: the sanitiser's output was still
    /// changing after its last pass. This is a parser or serializer
    /// mismatch, which the caller also logs as an error (editor.md §7.2).
    ContentNotNormalisable {
        /// How many passes ran.
        passes: usize,
    },
}

impl ContentError {
    /// The stable `snake_case` code of editor.md §7.6, the key of the
    /// admin's Hungarian `errors.<code>` message.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ImageAltMissing { .. } => "image_alt_missing",
            Self::ImageSrcRejected { .. } => "image_src_rejected",
            Self::EmbedRejected { .. } => "embed_rejected",
            Self::IframeContent { .. } => "iframe_content",
            Self::ContentTooLarge { .. } => "content_too_large",
            Self::ContentTooDeep { .. } => "content_too_deep",
            Self::ContentNotNormalisable { .. } => "content_not_normalisable",
        }
    }

    /// The 1-based position of the image or embed the finding is about, or
    /// `None` for a finding about the whole body or one whose position
    /// could not be traced.
    #[must_use]
    pub const fn index(&self) -> Option<NonZeroUsize> {
        match self {
            Self::ImageAltMissing { index, .. }
            | Self::ImageSrcRejected { index, .. }
            | Self::EmbedRejected { index, .. }
            | Self::IframeContent { index } => *index,
            Self::ContentTooLarge { .. }
            | Self::ContentTooDeep { .. }
            | Self::ContentNotNormalisable { .. } => None,
        }
    }
}

/// English text for logs and script clients; the admin shows the Hungarian
/// message of the code instead.
impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// `" 3"` for a known position, nothing for an unknown one.
        struct Position(Option<NonZeroUsize>);
        impl fmt::Display for Position {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.0 {
                    Some(index) => write!(f, " {index}"),
                    None => Ok(()),
                }
            }
        }
        let at = Position(self.index());
        match self {
            Self::ImageAltMissing { .. } => write!(f, "image{at} has no alt text"),
            Self::ImageSrcRejected { .. } => write!(
                f,
                "image{at} has no source that is an uploaded image under the upload prefix"
            ),
            Self::EmbedRejected { .. } => {
                write!(f, "embed{at} is not a YouTube or Padlet embed")
            }
            Self::IframeContent { .. } => write!(f, "embed{at} holds content"),
            Self::ContentTooLarge {
                measured: Measured::Input,
                limit,
                actual,
            } => write!(
                f,
                "the content is {actual} bytes long, more than the limit of {limit} bytes"
            ),
            Self::ContentTooLarge {
                measured: Measured::Output,
                limit,
                actual,
            } => write!(
                f,
                "the sanitised content would be {actual} bytes long, more than the limit of {limit} bytes"
            ),
            Self::ContentTooDeep { limit, line } => write!(
                f,
                "the content nests elements more than {limit} levels deep (line {line})"
            ),
            Self::ContentNotNormalisable { passes } => write!(
                f,
                "the content did not reach a fixed point in {passes} sanitiser passes"
            ),
        }
    }
}

impl std::error::Error for ContentError {}

/// The finding of a sanitiser refusal, or `None` for
/// [`SanitizeError::InvariantViolated`], which is a bug: the admin write
/// answers `500` and logs it, rather than `422` (editor.md §7.5).
///
/// - `error`: what [`super::Sanitizer::sanitize`] returned.
#[must_use]
pub const fn refusal(error: &SanitizeError) -> Option<ContentError> {
    match *error {
        SanitizeError::TooLarge { limit, actual } => Some(ContentError::ContentTooLarge {
            measured: Measured::Input,
            limit,
            actual,
        }),
        SanitizeError::OutputTooLarge { limit, actual } => Some(ContentError::ContentTooLarge {
            measured: Measured::Output,
            limit,
            actual,
        }),
        SanitizeError::TooDeep { limit, line } => {
            Some(ContentError::ContentTooDeep { limit, line })
        }
        SanitizeError::NotNormalisable { passes } => {
            Some(ContentError::ContentNotNormalisable { passes })
        }
        SanitizeError::InvariantViolated(_) => None,
    }
}

/// Checks a sanitised admin write (editor.md §7.6): every image kept its
/// source and has alt text, and every embed is an allowed one without
/// content. Returns the findings, images first and then embeds, each group
/// in document order (an embed with content and a rejected source gets
/// `iframe_content` first), and at most [`MAX_FINDINGS`] of them: the first
/// ones in that order.
///
/// - `original`: the body as the admin sent it; read only to give a finding
///   its index and to quote a rejected source.
/// - `clean`: what [`super::Sanitizer::sanitize`] made of `original`.
/// - `report`: the removal report of that same run.
/// - `policy`: the policy the sanitiser was built from, for deciding the
///   input's sources as the sanitiser did.
///
/// # Errors
///
/// The findings, never an empty list and never longer than
/// [`MAX_FINDINGS`], when the body must be refused with `422`.
///
/// # Examples
///
/// ```
/// use std::num::NonZeroUsize;
///
/// use backend3::domain::rich_text::{ContentError, SanitizePolicy, Sanitizer, validate_content};
///
/// let sanitizer = Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy");
/// let body = r#"<p><img src="/storage/uploads/a.jpg" alt="Kép"></p><p><img src="data:image/png;base64,AA" alt="Beillesztett"></p>"#;
/// let result = sanitizer.sanitize(body).expect("a small input");
/// assert_eq!(
///     validate_content(body, &result.html, &result.report, sanitizer.policy()),
///     Err(vec![ContentError::ImageSrcRejected {
///         index: NonZeroUsize::new(2),
///         src: Some("data:image/png;base64,AA".to_owned()),
///     }])
/// );
/// ```
pub fn validate_content(
    original: &str,
    clean: &SanitizedHtml,
    report: &RemovalReport,
    policy: &SanitizePolicy,
) -> Result<(), Vec<ContentError>> {
    let output = Elements::of_output(clean.as_str());
    let removed_images = report.count(RemovalRule::ImageWithoutSrcRemoved);
    let emptied_embeds = report.count(RemovalRule::IframeContentRemoved);
    let unnamed_images = output
        .images
        .iter()
        .filter(|image| is_missing(image.alt.as_deref()))
        .count();
    let rejected_embeds = output
        .embeds
        .iter()
        .filter(|embed| embed.src.is_none())
        .count();
    if removed_images + emptied_embeds + unnamed_images + rejected_embeds == 0 {
        return Ok(());
    }
    let input = Elements::of_input(policy, original);
    let mut errors = image_errors(&input, &output, removed_images);
    errors.extend(embed_errors(&input, &output, emptied_embeds));
    // Every finding is computed before the cut, so the ones kept are the
    // first in the documented order whatever the cap; the work stays linear
    // in the body.
    errors.truncate(MAX_FINDINGS);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Whether ECMAScript's `String.prototype.trim` removes `c`: its
/// `WhiteSpace` and `LineTerminator` code points (ECMA-262, "White Space"
/// and "Line Terminators"; the `Zs` category as of Unicode 16).
///
/// The editor checks alt text in TypeScript (ADR 0013 §2), where trimming
/// is that `trim`, so the server trims the same code points. Rust's
/// `char::is_whitespace` differs by two: U+0085, which it trims and `trim`
/// does not, and U+FEFF, the other way round. With Rust's set, an alt text
/// the editor accepted could be refused here, or one it blocked accepted.
///
/// - `c`: the character.
#[must_use]
pub const fn is_trimmed(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// Whether an alt text is blank: empty, or only characters that
/// [`is_trimmed`] removes.
///
/// - `alt`: the `alt` value, after character references are decoded.
#[must_use]
pub fn is_blank(alt: &str) -> bool {
    alt.chars().all(is_trimmed)
}

/// `value` cut to at most [`EXCERPT_CHARS`] characters, with `…` after a
/// cut, so a finding never repeats megabytes of input.
///
/// - `value`: an attribute value from the input.
#[must_use]
pub fn excerpt(value: &str) -> String {
    match value.char_indices().nth(EXCERPT_CHARS) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value.to_owned(),
    }
}

/// Whether an image has no usable alt text: none at all, or a blank one.
fn is_missing(alt: Option<&str>) -> bool {
    alt.is_none_or(is_blank)
}

/// One `img` as the walk found it.
#[derive(Debug)]
struct Image {
    /// The position among the images, from 1.
    index: NonZeroUsize,
    /// The `src` value of the input, or of the output.
    src: Option<String>,
    /// The `src` that the sanitiser keeps; `None` when it removes the image.
    /// In the output, the same as `src`.
    kept: Option<String>,
    /// The `alt` value.
    alt: Option<String>,
}

/// One `iframe` as the walk found it.
#[derive(Debug)]
struct Embed {
    /// The position among the embeds, from 1.
    index: NonZeroUsize,
    /// The `src` value of the input, or of the output.
    src: Option<String>,
    /// The `src` that the sanitiser keeps, if any. In the output, the same
    /// as `src`.
    kept: Option<String>,
    /// Whether the element has child nodes.
    has_content: bool,
}

/// The images and embeds of one body, in document order.
#[derive(Debug, Default)]
struct Elements {
    images: Vec<Image>,
    embeds: Vec<Embed>,
}

impl Elements {
    /// The images and embeds of a sanitised output. It is content-v1, so
    /// nothing in it is removed by a later pass.
    fn of_output(clean: &str) -> Self {
        let parsed = parse_like_ammonia(clean);
        let mut elements = Self::default();
        for node in parsed.root_element().descendants() {
            if let Some(element) = ElementRef::wrap(node) {
                let src = element.attr("src").map(str::to_owned);
                elements.push(element, src.clone(), src);
            }
        }
        elements
    }

    /// The images and embeds of an input as the sanitiser sees it: parsed
    /// as ammonia parses it, walked in document order, and skipping what
    /// ammonia removes with its content, which the editor never showed
    /// either. The walk uses an explicit stack, so a deeply nested input
    /// cannot overflow the call stack.
    fn of_input(policy: &SanitizePolicy, original: &str) -> Self {
        let parsed = parse_like_ammonia(original);
        let mut elements = Self::default();
        let mut stack: Vec<_> = parsed.root_element().children().rev().collect();
        while let Some(node) = stack.pop() {
            let Node::Element(element) = node.value() else {
                continue;
            };
            let name = element.name();
            if removed_with_content(name) {
                continue;
            }
            if let Some(element) =
                ElementRef::wrap(node).filter(|_| matches!(name, "img" | "iframe"))
            {
                let src = element.attr("src");
                let kept = src
                    .and_then(|src| url_outcome(policy, name, "src", src))
                    .map(Cow::into_owned);
                elements.push(element, src.map(str::to_owned), kept);
            }
            stack.extend(node.children().rev());
        }
        elements
    }

    /// Records `element` if it is an image or an embed; anything else is
    /// ignored.
    fn push(&mut self, element: ElementRef<'_>, src: Option<String>, kept: Option<String>) {
        match element.value().name() {
            "img" => self.images.push(Image {
                index: position(self.images.len()),
                src,
                kept,
                alt: element.attr("alt").map(str::to_owned),
            }),
            "iframe" => self.embeds.push(Embed {
                index: position(self.embeds.len()),
                src,
                kept,
                has_content: element.has_children(),
            }),
            _ => {}
        }
    }
}

/// The 1-based position after `count` earlier elements.
fn position(count: usize) -> NonZeroUsize {
    NonZeroUsize::MIN.saturating_add(count)
}

/// Whether two lists hold the same values, in any order.
fn same_multiset<T: Eq + std::hash::Hash>(
    first: impl IntoIterator<Item = T>,
    second: impl IntoIterator<Item = T>,
) -> bool {
    let mut counts: HashMap<T, isize> = HashMap::new();
    for value in first {
        *counts.entry(value).or_default() += 1;
    }
    for value in second {
        *counts.entry(value).or_default() -= 1;
    }
    counts.values().all(|&count| count == 0)
}

/// `image_src_rejected` and `image_alt_missing`, by image position.
///
/// - `removed`: the report's count of images the structural pass removed.
fn image_errors(input: &Elements, output: &Elements, removed: usize) -> Vec<ContentError> {
    let mut errors = Vec::new();
    let lost: Vec<_> = input
        .images
        .iter()
        .filter(|image| image.kept.is_none())
        .collect();
    if lost.len() == removed {
        errors.extend(lost.iter().map(|image| ContentError::ImageSrcRejected {
            index: Some(image.index),
            src: image.src.as_deref().map(excerpt),
        }));
    } else {
        errors.extend((0..removed).map(|_| ContentError::ImageSrcRejected {
            index: None,
            src: None,
        }));
    }
    let kept = input.images.iter().filter(|image| image.kept.is_some());
    let key = |image: &Image| (image.kept.clone(), image.alt.clone());
    if same_multiset(kept.clone().map(key), output.images.iter().map(key)) {
        // The kept images are the output's, so checking them is checking
        // the output; the input adds only their positions.
        errors.extend(
            kept.filter(|image| is_missing(image.alt.as_deref()))
                .map(|image| ContentError::ImageAltMissing {
                    index: Some(image.index),
                    src: image.kept.clone(),
                }),
        );
    } else {
        errors.extend(
            output
                .images
                .iter()
                .filter(|image| is_missing(image.alt.as_deref()))
                .map(|image| ContentError::ImageAltMissing {
                    index: None,
                    src: image.kept.clone(),
                }),
        );
    }
    // Stable, so findings without an index keep their order at the end.
    errors.sort_by_key(|error| error.index().map_or(usize::MAX, NonZeroUsize::get));
    errors
}

/// `iframe_content` and `embed_rejected`, by embed position.
///
/// - `emptied`: the report's count of embeds whose children the structural
///   pass removed.
fn embed_errors(input: &Elements, output: &Elements, emptied: usize) -> Vec<ContentError> {
    let mut errors = Vec::new();
    let with_content: Vec<_> = input
        .embeds
        .iter()
        .filter(|embed| embed.has_content)
        .collect();
    if with_content.len() == emptied {
        errors.extend(
            with_content
                .iter()
                .map(|embed| ContentError::IframeContent {
                    index: Some(embed.index),
                }),
        );
    } else {
        errors.extend((0..emptied).map(|_| ContentError::IframeContent { index: None }));
    }
    let key = |embed: &Embed| embed.kept.clone();
    if same_multiset(input.embeds.iter().map(key), output.embeds.iter().map(key)) {
        errors.extend(
            input
                .embeds
                .iter()
                .filter(|embed| embed.kept.is_none())
                .map(|embed| ContentError::EmbedRejected {
                    index: Some(embed.index),
                    src: embed.src.as_deref().map(excerpt),
                }),
        );
    } else {
        errors.extend(
            output
                .embeds
                .iter()
                .filter(|embed| embed.src.is_none())
                .map(|_| ContentError::EmbedRejected {
                    index: None,
                    src: None,
                }),
        );
    }
    // Stable: for one embed, `iframe_content` stays before `embed_rejected`.
    errors.sort_by_key(|error| error.index().map_or(usize::MAX, NonZeroUsize::get));
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::{Sanitized, Sanitizer};
    use proptest::prelude::*;
    use std::fmt::Write as _;

    const GOOD_IMAGE: &str = "/storage/uploads/posts/1/kep.jpg";
    const YOUTUBE: &str = "https://www.youtube-nocookie.com/embed/VIDEOID";

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
    }

    fn sanitized(input: &str) -> Sanitized {
        sanitizer().sanitize(input).expect("a small input")
    }

    /// Sanitises `input` and validates the result, as an admin write does.
    fn validate(input: &str) -> Result<(), Vec<ContentError>> {
        let result = sanitized(input);
        validate_content(
            input,
            &result.html,
            &result.report,
            &SanitizePolicy::content_v1(),
        )
    }

    fn at(index: usize) -> Option<NonZeroUsize> {
        NonZeroUsize::new(index)
    }

    fn image(src: &str, alt: &str) -> String {
        format!(r#"<p><img src="{src}" alt="{alt}" width="350" height="495"></p>"#)
    }

    fn embed(src: &str) -> String {
        format!(
            r#"<p data-youtube-video=""><iframe src="{src}" width="640" height="360" title="Videó"></iframe></p>"#
        )
    }

    #[test]
    fn editor_output_passes() {
        let body = format!(
            "<h2>Cím</h2><p>Szöveg</p>{}{}{}",
            image(GOOD_IMAGE, "A díjazottak"),
            embed(YOUTUBE),
            embed("https://padlet.com/embed/KEY")
        );
        assert_eq!(validate(&body), Ok(()));
        assert_eq!(validate(""), Ok(()));
        assert_eq!(validate("<p>Csak szöveg</p>"), Ok(()));
    }

    #[test]
    fn a_missing_or_blank_alt_is_refused_at_its_position() {
        for alt in ["", "   ", "\u{a0}", "\t\n", "\u{feff}", "\u{3000}"] {
            let body = format!("{}{}", image(GOOD_IMAGE, "Első"), image(GOOD_IMAGE, alt));
            assert_eq!(
                validate(&body),
                Err(vec![ContentError::ImageAltMissing {
                    index: at(2),
                    src: Some(GOOD_IMAGE.into()),
                }]),
                "{alt:?}"
            );
        }
        let without = format!(r#"<p><img src="{GOOD_IMAGE}"></p>"#);
        assert_eq!(
            validate(&without),
            Err(vec![ContentError::ImageAltMissing {
                index: at(1),
                src: Some(GOOD_IMAGE.into()),
            }])
        );
    }

    #[test]
    fn alt_text_is_trimmed_as_the_editor_trims_it() {
        // U+0085 is white space to Rust but not to `String.prototype.trim`,
        // so an alt of only that character is text, as in the editor.
        assert_eq!(validate(&image(GOOD_IMAGE, "\u{85}")), Ok(()));
        assert_eq!(validate(&image(GOOD_IMAGE, " kép ")), Ok(()));
    }

    #[test]
    fn a_removed_image_is_refused_once_at_its_position() {
        let body = format!(
            r#"{}<p><img src="data:image/png;base64,AA" alt="beillesztett"></p><p><img alt="forrás nélkül"></p>{}"#,
            image(GOOD_IMAGE, "Első"),
            image("https://evil.example/kep.jpg", "Idegen"),
        );
        let result = sanitized(&body);
        // The filter dropped two sources and the structural pass removed
        // three images; each image is refused once.
        assert_eq!(result.report.count(RemovalRule::ImageSrcRejected), 2);
        assert_eq!(result.report.count(RemovalRule::ImageWithoutSrcRemoved), 3);
        assert_eq!(
            validate_content(
                &body,
                &result.html,
                &result.report,
                &SanitizePolicy::content_v1()
            ),
            Err(vec![
                ContentError::ImageSrcRejected {
                    index: at(2),
                    src: Some("data:image/png;base64,AA".into()),
                },
                ContentError::ImageSrcRejected {
                    index: at(3),
                    src: None,
                },
                ContentError::ImageSrcRejected {
                    index: at(4),
                    src: Some("https://evil.example/kep.jpg".into()),
                },
            ])
        );
    }

    #[test]
    fn image_findings_follow_the_document_order() {
        let body = format!(
            "{}{}{}{}",
            image(GOOD_IMAGE, ""),
            image("blob:https://verseghy-gimnazium.net/1", "x"),
            image(GOOD_IMAGE, "jó"),
            // An own-host absolute source is kept, rewritten to its path.
            image("https://verseghy-gimnazium.net/storage/uploads/b.png", " "),
        );
        assert_eq!(
            validate(&body),
            Err(vec![
                ContentError::ImageAltMissing {
                    index: at(1),
                    src: Some(GOOD_IMAGE.into()),
                },
                ContentError::ImageSrcRejected {
                    index: at(2),
                    src: Some("blob:https://verseghy-gimnazium.net/1".into()),
                },
                ContentError::ImageAltMissing {
                    index: at(4),
                    src: Some("/storage/uploads/b.png".into()),
                },
            ])
        );
    }

    #[test]
    fn a_long_rejected_source_is_cut() {
        let src = format!("data:image/png;base64,{}", "A".repeat(10_000));
        let Err(errors) = validate(&image(&src, "kép")) else {
            panic!("the image must be refused");
        };
        let [
            ContentError::ImageSrcRejected {
                src: Some(quoted), ..
            },
        ] = errors.as_slice()
        else {
            panic!("{errors:?}");
        };
        assert_eq!(quoted.chars().count(), EXCERPT_CHARS + 1);
        assert!(quoted.ends_with('…'));
        assert!(src.starts_with(quoted.trim_end_matches('…')));
    }

    #[test]
    fn what_ammonia_removes_with_its_content_is_not_counted() {
        // The image inside `object` goes with it, unseen in the editor too,
        // so it neither fails nor takes a position.
        let body = format!(
            r#"<object><img src="data:x" alt=""></object><desc><img alt=""></desc>{}"#,
            image(GOOD_IMAGE, "")
        );
        assert_eq!(
            validate(&body),
            Err(vec![ContentError::ImageAltMissing {
                index: at(1),
                src: Some(GOOD_IMAGE.into()),
            }])
        );
    }

    #[test]
    fn an_image_inside_an_html_annotation_stays_inside_the_math() {
        // ammonia's tree keeps the `img` inside the `annotation-xml`, so it
        // is removed with the `math`: no finding, and no report entry.
        let body = r#"<math><annotation-xml encoding="text/html"><img src="data:x" alt=""><p>szöveg</p></annotation-xml></math>"#;
        let result = sanitized(body);
        assert_eq!(result.html.as_str(), "");
        assert_eq!(result.report.count(RemovalRule::ImageSrcRejected), 0);
        assert_eq!(validate(body), Ok(()));
    }

    #[test]
    fn embeds_are_refused_for_their_source_and_their_content() {
        let body = format!(
            r#"{}{}<p><iframe width="640" height="360"></iframe></p><p><iframe src="{YOUTUBE}">szöveg</iframe></p><p><iframe src="javascript:alert(1)">x</iframe></p>"#,
            embed(YOUTUBE),
            embed("https://evil.example/embed/x"),
        );
        assert_eq!(
            validate(&body),
            Err(vec![
                ContentError::EmbedRejected {
                    index: at(2),
                    src: Some("https://evil.example/embed/x".into()),
                },
                ContentError::EmbedRejected {
                    index: at(3),
                    src: None,
                },
                ContentError::IframeContent { index: at(4) },
                // One embed with both faults: its content first.
                ContentError::IframeContent { index: at(5) },
                ContentError::EmbedRejected {
                    index: at(5),
                    src: Some("javascript:alert(1)".into()),
                },
            ])
        );
    }

    #[test]
    fn images_come_before_embeds() {
        let body = format!(
            "{}{}",
            embed("https://evil.example/embed/x"),
            image(GOOD_IMAGE, "")
        );
        let codes: Vec<_> = validate(&body)
            .expect_err("both are refused")
            .iter()
            .map(ContentError::code)
            .collect();
        assert_eq!(codes, ["image_alt_missing", "embed_rejected"]);
    }

    #[test]
    fn an_image_moved_by_the_parser_keeps_its_position() {
        // An unwrapped `caption` after the rows leaves its image in the
        // table, and the next parse fosters it in front of the table, so the
        // output's order is not the input's. The positions are the input's.
        let body = format!(
            r#"<table><tbody><tr><td><img src="{GOOD_IMAGE}" alt="cella"></td></tr></tbody><caption><img src="/storage/uploads/b.png" alt=""></caption></table>"#
        );
        let result = sanitized(&body);
        let output = Elements::of_output(result.html.as_str());
        assert_eq!(
            output
                .images
                .iter()
                .map(|image| image.alt.as_deref())
                .collect::<Vec<_>>(),
            [Some(""), Some("cella")],
            "{}",
            result.html
        );
        assert_eq!(
            validate(&body),
            Err(vec![ContentError::ImageAltMissing {
                index: at(2),
                src: Some("/storage/uploads/b.png".into()),
            }])
        );
    }

    #[test]
    fn findings_without_a_trace_keep_no_index() {
        let policy = SanitizePolicy::content_v1();
        // A report that counts more removed images and emptied embeds than
        // the input has: the findings follow the report, without positions.
        let result = sanitized("<p>szöveg</p>");
        let mut report = result.report.clone();
        report.record(RemovalRule::ImageWithoutSrcRemoved, "img", None);
        report.record(RemovalRule::IframeContentRemoved, "iframe", None);
        assert_eq!(
            validate_content("<p>szöveg</p>", &result.html, &report, &policy),
            Err(vec![
                ContentError::ImageSrcRejected {
                    index: None,
                    src: None,
                },
                ContentError::IframeContent { index: None },
            ])
        );
        // An output whose images and embeds are not the input's: the
        // output's findings stand, without positions.
        let output = sanitized(&format!(
            "{}{}",
            image(GOOD_IMAGE, ""),
            embed("https://evil.example/embed/x")
        ));
        assert_eq!(
            validate_content("<p>más</p>", &output.html, &output.report, &policy),
            Err(vec![
                ContentError::ImageAltMissing {
                    index: None,
                    src: Some(GOOD_IMAGE.into()),
                },
                ContentError::EmbedRejected {
                    index: None,
                    src: None,
                },
            ])
        );
    }

    #[test]
    fn a_clean_body_is_not_parsed_again() {
        // Nothing to explain: the input is never read, so even an input
        // unrelated to the output passes when the output and report are
        // clean.
        let result = sanitized(&image(GOOD_IMAGE, "kép"));
        assert_eq!(
            validate_content(
                "<img>",
                &result.html,
                &result.report,
                &SanitizePolicy::content_v1()
            ),
            Ok(())
        );
    }

    #[test]
    fn refusals_map_to_their_findings() {
        use crate::domain::rich_text::Violation;
        let cases = [
            (
                SanitizeError::TooLarge {
                    limit: 10,
                    actual: 11,
                },
                Some(ContentError::ContentTooLarge {
                    measured: Measured::Input,
                    limit: 10,
                    actual: 11,
                }),
            ),
            (
                SanitizeError::OutputTooLarge {
                    limit: 20,
                    actual: 21,
                },
                Some(ContentError::ContentTooLarge {
                    measured: Measured::Output,
                    limit: 20,
                    actual: 21,
                }),
            ),
            (
                SanitizeError::TooDeep {
                    limit: 256,
                    line: 7,
                },
                Some(ContentError::ContentTooDeep {
                    limit: 256,
                    line: 7,
                }),
            ),
            (
                SanitizeError::NotNormalisable { passes: 3 },
                Some(ContentError::ContentNotNormalisable { passes: 3 }),
            ),
            (
                SanitizeError::InvariantViolated(Violation::TableInsideParagraph),
                None,
            ),
        ];
        for (error, finding) in cases {
            assert_eq!(refusal(&error), finding, "{error:?}");
            assert_eq!(refusal(&error).map(|f| f.code()), error.code(), "{error:?}");
        }
    }

    #[test]
    fn a_body_the_sanitiser_expands_says_which_limit_it_passed() {
        // 980 bytes, under the input limit; every link gains `target` and
        // `rel`, so the output passes the output limit. The finding says it
        // measured the output, so no message quotes a limit the submitted
        // body never reached as if it had.
        let policy = SanitizePolicy {
            max_input_bytes: 1024,
            max_output_bytes: 1024,
            ..SanitizePolicy::content_v1()
        };
        let small = Sanitizer::new(policy).expect("a valid policy");
        let error = small
            .sanitize(&"<a></a>".repeat(140))
            .expect_err("the output is too large");
        let Some(ContentError::ContentTooLarge {
            measured,
            limit,
            actual,
        }) = refusal(&error)
        else {
            panic!("{error:?}");
        };
        assert_eq!((measured, limit), (Measured::Output, 1024));
        assert!(actual > 1024, "{actual}");
        let error = small
            .sanitize(&"a".repeat(1025))
            .expect_err("the input is too large");
        assert_eq!(
            refusal(&error),
            Some(ContentError::ContentTooLarge {
                measured: Measured::Input,
                limit: 1024,
                actual: 1025,
            })
        );
    }

    #[test]
    fn measured_names_are_the_wire_names() {
        assert_eq!(Measured::Input.as_str(), "input");
        assert_eq!(Measured::Output.as_str(), "output");
    }

    #[test]
    fn findings_stop_at_the_cap() {
        // `<img>` is five bytes, so a body within the input limit could
        // otherwise give hundreds of thousands of findings.
        let body = "<img>".repeat(MAX_FINDINGS * 3);
        let errors = validate(&body).expect_err("every image is refused");
        assert_eq!(errors.len(), MAX_FINDINGS);
        for (position, error) in (1..).zip(&errors) {
            assert_eq!(
                *error,
                ContentError::ImageSrcRejected {
                    index: at(position),
                    src: None,
                }
            );
        }
        // Images come first, so embed findings past the cap are left out.
        let mixed = format!(
            "{}{}",
            "<img>".repeat(MAX_FINDINGS),
            embed("https://evil.example/embed/x")
        );
        let errors = validate(&mixed).expect_err("refused");
        assert_eq!(errors.len(), MAX_FINDINGS);
        assert!(
            errors
                .iter()
                .all(|error| error.code() == "image_src_rejected")
        );
        // One under the cap leaves room for the embed.
        let room = format!(
            "{}{}",
            "<img>".repeat(MAX_FINDINGS - 1),
            embed("https://evil.example/embed/x")
        );
        let errors = validate(&room).expect_err("refused");
        assert_eq!(errors.len(), MAX_FINDINGS);
        assert_eq!(
            errors.last(),
            Some(&ContentError::EmbedRejected {
                index: at(1),
                src: Some("https://evil.example/embed/x".into()),
            })
        );
    }

    #[test]
    fn a_too_deep_body_is_refused_by_the_sanitiser() {
        let deep = "<blockquote>".repeat(300);
        let error = sanitizer().sanitize(&deep).expect_err("deeper than 256");
        assert_eq!(
            refusal(&error),
            Some(ContentError::ContentTooDeep {
                limit: 256,
                line: 1
            })
        );
    }

    #[test]
    fn codes_indices_and_messages() {
        let cases = [
            (
                ContentError::ImageAltMissing {
                    index: at(3),
                    src: None,
                },
                "image_alt_missing",
                at(3),
                "image 3 has no alt text",
            ),
            (
                ContentError::ImageSrcRejected {
                    index: None,
                    src: None,
                },
                "image_src_rejected",
                None,
                "image has no source that is an uploaded image under the upload prefix",
            ),
            (
                ContentError::EmbedRejected {
                    index: at(1),
                    src: None,
                },
                "embed_rejected",
                at(1),
                "embed 1 is not a YouTube or Padlet embed",
            ),
            (
                ContentError::IframeContent { index: at(2) },
                "iframe_content",
                at(2),
                "embed 2 holds content",
            ),
            (
                ContentError::ContentTooLarge {
                    measured: Measured::Input,
                    limit: 1,
                    actual: 2,
                },
                "content_too_large",
                None,
                "the content is 2 bytes long, more than the limit of 1 bytes",
            ),
            (
                ContentError::ContentTooLarge {
                    measured: Measured::Output,
                    limit: 8,
                    actual: 9,
                },
                "content_too_large",
                None,
                "the sanitised content would be 9 bytes long, more than the limit of 8 bytes",
            ),
            (
                ContentError::ContentTooDeep {
                    limit: 256,
                    line: 4,
                },
                "content_too_deep",
                None,
                "the content nests elements more than 256 levels deep (line 4)",
            ),
            (
                ContentError::ContentNotNormalisable { passes: 3 },
                "content_not_normalisable",
                None,
                "the content did not reach a fixed point in 3 sanitiser passes",
            ),
        ];
        for (error, code, index, message) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.index(), index, "{code}");
            assert_eq!(error.to_string(), message, "{code}");
        }
    }

    #[test]
    fn trimmed_characters_are_ecmascripts() {
        for c in [
            '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{200a}',
            '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}', '\u{feff}',
        ] {
            assert!(is_trimmed(c), "{c:?}");
        }
        for c in ['\u{85}', '\u{200b}', '\u{180e}', 'a', '\0'] {
            assert!(!is_trimmed(c), "{c:?}");
        }
        assert!(is_blank(""));
        assert!(is_blank(" \u{feff}\u{a0}"));
        assert!(!is_blank(" a "));
    }

    #[test]
    fn short_values_are_quoted_whole() {
        assert_eq!(excerpt(""), "");
        assert_eq!(excerpt("data:x"), "data:x");
        let exact = "é".repeat(EXCERPT_CHARS);
        assert_eq!(excerpt(&exact), exact);
        assert_eq!(excerpt(&format!("{exact}x")), format!("{exact}…"));
    }

    /// How a generated image or embed is written.
    #[derive(Debug, Clone, Copy)]
    enum Source {
        /// Kept, with the canonical spelling in the output.
        Good(&'static str, &'static str),
        /// Dropped by the URL rules.
        Bad(&'static str),
        /// No `src` at all.
        Missing,
    }

    const IMAGE_SOURCES: &[Source] = &[
        Source::Good(GOOD_IMAGE, GOOD_IMAGE),
        Source::Good(
            "https://verseghy-gimnazium.net/storage/uploads/b.png",
            "/storage/uploads/b.png",
        ),
        Source::Bad("data:image/png;base64,AA"),
        Source::Bad("https://evil.example/a.jpg"),
        Source::Bad("/storage/x.svg"),
        Source::Bad("/storage/uploads/a.txt"),
        Source::Missing,
    ];

    const EMBED_SOURCES: &[Source] = &[
        Source::Good(YOUTUBE, YOUTUBE),
        Source::Good("https://padlet.com/embed/k", "https://padlet.com/embed/k"),
        Source::Bad("https://evil.example/embed/x"),
        Source::Bad("http://www.youtube.com/embed/x"),
        Source::Missing,
    ];

    const ALTS: &[Option<&str>] = &[Some("kép"), Some(""), Some("  "), Some("\u{a0}"), None];

    /// One generated piece of a body.
    #[derive(Debug, Clone)]
    enum Piece {
        Text,
        Image(Source, Option<&'static str>),
        Embed(Source, bool),
        /// Pieces inside an element: kept, unwrapped, or removed with them.
        Wrapped(&'static str, Vec<Piece>),
        /// Pieces inside a table cell.
        Cell(Vec<Piece>),
    }

    /// Wrappers that the parser keeps as written around these pieces and
    /// table cells: kept (`strong`, `blockquote`), unwrapped (`div`, `font`)
    /// and removed with their content (`object`, `xml`, `g`). `p` is left
    /// out: a `table` start tag closes an open `p` and everything above it,
    /// so a wrapper around the `p` would not hold the table. The unit tests
    /// cover pieces inside paragraphs.
    #[rustfmt::skip]
    const WRAPPERS: &[&str] = &["strong", "blockquote", "div", "font", "object", "xml", "g"];

    fn arb_piece() -> impl Strategy<Value = Piece> {
        let leaf = prop_oneof![
            Just(Piece::Text),
            (
                prop::sample::select(IMAGE_SOURCES),
                prop::sample::select(ALTS)
            )
                .prop_map(|(src, alt)| Piece::Image(src, alt)),
            (prop::sample::select(EMBED_SOURCES), any::<bool>())
                .prop_map(|(src, content)| Piece::Embed(src, content)),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                (
                    prop::sample::select(WRAPPERS),
                    prop::collection::vec(inner.clone(), 0..4)
                )
                    .prop_map(|(tag, pieces)| Piece::Wrapped(tag, pieces)),
                prop::collection::vec(inner, 0..3).prop_map(Piece::Cell),
            ]
        })
    }

    /// What a walk of the generated pieces predicts.
    #[derive(Default)]
    struct Model {
        images: Vec<ContentError>,
        embeds: Vec<ContentError>,
        image_count: usize,
        embed_count: usize,
    }

    fn src_attribute(src: Source) -> String {
        match src {
            Source::Good(value, _) | Source::Bad(value) => format!(r#" src="{value}""#),
            Source::Missing => String::new(),
        }
    }

    /// Writes `piece` and records the findings it should give; `live` is
    /// false inside an element that ammonia removes with its content.
    fn render(piece: &Piece, out: &mut String, live: bool, model: &mut Model) {
        match piece {
            Piece::Text => out.push_str("szöveg "),
            Piece::Image(src, alt) => {
                let alt_attribute = alt
                    .map(|alt| format!(r#" alt="{alt}""#))
                    .unwrap_or_default();
                let _ = write!(out, "<img{}{alt_attribute}>", src_attribute(*src));
                if live {
                    model.image_count += 1;
                    let index = at(model.image_count);
                    match src {
                        Source::Good(_, kept) if alt.is_none_or(is_blank) => {
                            model.images.push(ContentError::ImageAltMissing {
                                index,
                                src: Some((*kept).into()),
                            });
                        }
                        Source::Good(..) => {}
                        Source::Bad(value) => {
                            model.images.push(ContentError::ImageSrcRejected {
                                index,
                                src: Some((*value).into()),
                            });
                        }
                        Source::Missing => model
                            .images
                            .push(ContentError::ImageSrcRejected { index, src: None }),
                    }
                }
            }
            Piece::Embed(src, content) => {
                let _ = write!(
                    out,
                    "<iframe{}>{}</iframe>",
                    src_attribute(*src),
                    if *content { "x" } else { "" }
                );
                if live {
                    model.embed_count += 1;
                    let index = at(model.embed_count);
                    if *content {
                        model.embeds.push(ContentError::IframeContent { index });
                    }
                    match src {
                        Source::Good(..) => {}
                        Source::Bad(value) => {
                            model.embeds.push(ContentError::EmbedRejected {
                                index,
                                src: Some((*value).into()),
                            });
                        }
                        Source::Missing => model
                            .embeds
                            .push(ContentError::EmbedRejected { index, src: None }),
                    }
                }
            }
            Piece::Wrapped(tag, pieces) => {
                let removed = removed_with_content(tag);
                let _ = write!(out, "<{tag}>");
                for piece in pieces {
                    render(piece, out, live && !removed, model);
                }
                let _ = write!(out, "</{tag}>");
            }
            Piece::Cell(pieces) => {
                out.push_str("<table><tbody><tr><td>");
                for piece in pieces {
                    render(piece, out, live, model);
                }
                out.push_str("</td></tr></tbody></table>");
            }
        }
    }

    proptest! {
        // The findings, their order, their positions and their sources are
        // exactly what the generated body predicts.
        #[test]
        fn findings_match_the_body(pieces in prop::collection::vec(arb_piece(), 0..6)) {
            let (mut body, mut model) = (String::new(), Model::default());
            for piece in &pieces {
                render(piece, &mut body, true, &mut model);
            }
            let expected: Vec<_> = model.images.into_iter().chain(model.embeds).collect();
            let result = validate(&body);
            if expected.is_empty() {
                prop_assert_eq!(result, Ok(()), "{}", body);
            } else {
                prop_assert_eq!(result, Err(expected), "{}", body);
            }
        }

        // Past the cap, the findings are the first ones the uncapped list
        // would hold, in its order.
        #[test]
        fn the_cap_keeps_the_first_findings(alts in prop::collection::vec(prop::sample::select(ALTS), 0..MAX_FINDINGS * 5 / 2)) {
            let body: String = alts
                .iter()
                .map(|alt| match alt {
                    Some(alt) => format!(r#"<img src="{GOOD_IMAGE}" alt="{alt}">"#),
                    None => format!(r#"<img src="{GOOD_IMAGE}">"#),
                })
                .collect();
            let expected: Vec<_> = (1..)
                .zip(&alts)
                .filter(|(_, alt)| alt.is_none_or(is_blank))
                .map(|(position, _)| ContentError::ImageAltMissing { index: at(position), src: Some(GOOD_IMAGE.into()) })
                .take(MAX_FINDINGS)
                .collect();
            match validate(&body) {
                Ok(()) => prop_assert!(expected.is_empty()),
                Err(errors) => prop_assert_eq!(errors, expected),
            }
        }

        #[test]
        fn validation_never_panics(input in any::<String>()) {
            if let Ok(result) = sanitizer().sanitize(&input) {
                let outcome = validate_content(&input, &result.html, &result.report, &SanitizePolicy::content_v1());
                if let Err(errors) = outcome {
                    prop_assert!((1..=MAX_FINDINGS).contains(&errors.len()));
                }
            }
        }

        // The code always matches the sanitiser's, whatever the numbers.
        #[test]
        fn refusal_codes_follow_the_sanitiser(limit in any::<usize>(), actual in any::<usize>(), line in any::<u64>(), passes in any::<usize>()) {
            for error in [
                SanitizeError::TooLarge { limit, actual },
                SanitizeError::OutputTooLarge { limit, actual },
                SanitizeError::TooDeep { limit, line },
                SanitizeError::NotNormalisable { passes },
            ] {
                let finding = refusal(&error);
                prop_assert_eq!(finding.as_ref().map(ContentError::code), error.code());
                prop_assert_eq!(finding.and_then(|f| f.index()), None);
            }
            // The two size refusals keep their numbers and say which length
            // they measured.
            prop_assert_eq!(
                refusal(&SanitizeError::TooLarge { limit, actual }),
                Some(ContentError::ContentTooLarge { measured: Measured::Input, limit, actual })
            );
            prop_assert_eq!(
                refusal(&SanitizeError::OutputTooLarge { limit, actual }),
                Some(ContentError::ContentTooLarge { measured: Measured::Output, limit, actual })
            );
        }

        // ECMAScript's trim set is Rust's white space without U+0085, plus
        // U+FEFF.
        #[test]
        fn the_trim_set_differs_from_rusts_by_two(c in any::<char>()) {
            let expected = (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}';
            prop_assert_eq!(is_trimmed(c), expected, "{:?}", c);
        }

        #[test]
        fn an_alt_with_text_is_never_blank(padding in "[ \t\u{a0}\u{feff}]{0,4}", word in "[a-zé0-9]{1,8}") {
            let alt = format!("{padding}{word}{padding}");
            prop_assert!(!is_blank(&alt));
            prop_assert!(is_blank(&padding));
        }

        #[test]
        fn an_excerpt_is_a_short_prefix(value in ".{0,300}") {
            let quoted = excerpt(&value);
            let length = value.chars().count();
            if length <= EXCERPT_CHARS {
                prop_assert_eq!(&quoted, &value);
            } else {
                prop_assert_eq!(quoted.chars().count(), EXCERPT_CHARS + 1);
                let kept = quoted.strip_suffix('…').expect("a cut excerpt ends in an ellipsis");
                prop_assert!(value.starts_with(kept));
            }
        }

        #[test]
        fn positions_start_at_one(count in 0usize..1_000_000) {
            prop_assert_eq!(position(count).get(), count + 1);
        }

        #[test]
        fn multisets_ignore_order(mut values in prop::collection::vec(0u8..5, 0..12), extra in 0u8..5) {
            let shuffled: Vec<_> = values.iter().rev().copied().collect();
            prop_assert!(same_multiset(values.clone(), shuffled));
            let fewer = values.split_off(values.len() / 2);
            prop_assert_eq!(same_multiset(values.clone(), fewer.clone()), {
                let (mut a, mut b) = (values.clone(), fewer.clone());
                a.sort_unstable();
                b.sort_unstable();
                a == b
            });
            let mut more = values.clone();
            more.push(extra);
            prop_assert!(!same_multiset(values, more));
        }
    }
}
