//! The attribute filter (editor.md §7.3): for an attribute that passed
//! ammonia's allow-list, the value to store or the rule that drops it.
//!
//! [`decide`] is the whole decision, with the rule kept for the removal
//! report; [`filter`] is its shape for ammonia's `attribute_filter`. Both
//! read only the tables of [`super::policy`], so a table change is the only
//! way to change what the filter does.
//!
//! ammonia also calls the filter for the attributes of elements it is about
//! to unwrap or remove; those answers are never written, so the filter drops
//! everything that is not in the tables, which is harmless there and a
//! second line of defence for the elements ammonia keeps.

use std::borrow::Cow;

use super::policy::{SanitizePolicy, ValueRule, element_rule};
use super::report::RemovalRule;
use super::url_rules;

/// What the filter does with one attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision<'a> {
    /// Store the attribute with this value.
    Keep(Cow<'a, str>),
    /// Remove the attribute, for this reason.
    Drop(RemovalRule),
}

/// Decides one attribute of one element.
///
/// - `policy`: supplies the hosts and prefixes of the URL rules.
/// - `element`: the tag name, as the HTML parser reports it.
/// - `attribute`: the attribute name, as the HTML parser reports it.
/// - `value`: the value after character references are decoded, and after
///   ammonia has written the canonical value of a forced attribute.
#[must_use]
pub fn decide<'a>(
    policy: &SanitizePolicy,
    element: &str,
    attribute: &str,
    value: &'a str,
) -> Decision<'a> {
    let Some(rule) = element_rule(element).and_then(|rule| rule.attribute(attribute)) else {
        return Decision::Drop(RemovalRule::AttributeNotAllowed);
    };
    let keep = |kept: &'a str| Decision::Keep(Cow::Borrowed(kept));
    match rule.rule {
        // The CSS grammar of editor.md §3.3 is not part of this policy yet,
        // so no style is stored.
        ValueRule::Style => Decision::Drop(RemovalRule::StyleRejected),
        // `rel` passes through: ammonia's `link_rel` wrote it.
        ValueRule::Text | ValueRule::LinkRel => keep(value),
        ValueRule::Marker => Decision::Keep(Cow::Borrowed("")),
        ValueRule::Forced(canonical) => Decision::Keep(Cow::Borrowed(canonical)),
        ValueRule::Integer { min, max, default } => integer(value, min, max, default),
        ValueRule::IntegerOrFull { .. } if value == "100%" => keep(value),
        ValueRule::IntegerOrFull { min, max } => integer(value, min, max, None),
        ValueRule::Enumerated { default, .. } if Some(value) == default => {
            Decision::Drop(RemovalRule::DefaultValueOmitted)
        }
        ValueRule::Enumerated { values, .. } if values.contains(&value) => keep(value),
        ValueRule::Enumerated { .. } => Decision::Drop(RemovalRule::AttributeValueRejected),
        ValueRule::ImageSrc => url_rules::image_src(policy, value)
            .map_or(Decision::Drop(RemovalRule::ImageSrcRejected), |src| {
                Decision::Keep(Cow::Owned(src))
            }),
        ValueRule::EmbedSrc => url_rules::embed_src(policy, value)
            .map_or(Decision::Drop(RemovalRule::EmbedSrcRejected), |src| {
                Decision::Keep(Cow::Owned(src))
            }),
        ValueRule::LinkHref => url_rules::link_href(policy, value).map_or(
            Decision::Drop(RemovalRule::LinkHrefRejected),
            Decision::Keep,
        ),
    }
}

/// [`decide`] in the shape of ammonia's `attribute_filter` callback: `None`
/// removes the attribute.
///
/// - `policy`, `element`, `attribute`, `value`: as for [`decide`].
#[must_use]
pub fn filter<'a>(
    policy: &SanitizePolicy,
    element: &str,
    attribute: &str,
    value: &'a str,
) -> Option<Cow<'a, str>> {
    match decide(policy, element, attribute, value) {
        Decision::Keep(kept) => Some(kept),
        Decision::Drop(_) => None,
    }
}

/// A decimal integer from `min` to `max`, written without leading zeros;
/// `default` is omitted. Surrounding ASCII whitespace is ignored, anything
/// else that is not a digit (a sign, `px`, a decimal point) rejects it.
fn integer(value: &str, min: u32, max: u32, default: Option<u32>) -> Decision<'_> {
    let digits = value.trim_ascii();
    // Ten digits always fit in a u64, so the parse below cannot overflow.
    let parsed =
        (!digits.is_empty() && digits.len() <= 10 && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse::<u64>().ok())
            .flatten();
    match parsed {
        Some(number) if default.is_some_and(|d| u64::from(d) == number) => {
            Decision::Drop(RemovalRule::DefaultValueOmitted)
        }
        Some(number) if (u64::from(min)..=u64::from(max)).contains(&number) => {
            let canonical = number.to_string();
            Decision::Keep(if canonical == value {
                Cow::Borrowed(value)
            } else {
                Cow::Owned(canonical)
            })
        }
        _ => Decision::Drop(RemovalRule::AttributeValueRejected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::policy::{ELEMENTS, IFRAME_ALLOW, IFRAME_REFERRER_POLICY};
    use proptest::prelude::*;

    fn run<'a>(element: &str, attribute: &str, value: &'a str) -> Decision<'a> {
        decide(&SanitizePolicy::content_v1(), element, attribute, value)
    }

    fn kept(value: &str) -> Decision<'_> {
        Decision::Keep(Cow::Borrowed(value))
    }

    #[test]
    fn integers_are_bounded_and_canonical() {
        assert_eq!(run("img", "width", "350"), kept("350"));
        assert_eq!(
            run("img", "width", " 0350 "),
            Decision::Keep(Cow::Owned("350".into()))
        );
        assert_eq!(run("img", "height", "5000"), kept("5000"));
        for bad in [
            "0",
            "5001",
            "-1",
            "+3",
            "35px",
            "3.5",
            "",
            " ",
            "99999999999",
            "١٢",
        ] {
            assert_eq!(
                run("img", "width", bad),
                Decision::Drop(RemovalRule::AttributeValueRejected),
                "{bad:?}"
            );
        }
        assert_eq!(run("iframe", "width", "100%"), kept("100%"));
        assert_eq!(run("iframe", "width", "4000"), kept("4000"));
        assert_eq!(
            run("iframe", "width", "50%"),
            Decision::Drop(RemovalRule::AttributeValueRejected)
        );
        assert_eq!(
            run("iframe", "height", "100%"),
            Decision::Drop(RemovalRule::AttributeValueRejected)
        );
        assert_eq!(
            run("iframe", "height", "4001"),
            Decision::Drop(RemovalRule::AttributeValueRejected)
        );
    }

    #[test]
    fn spans_and_list_starts_omit_their_default() {
        assert_eq!(
            run("td", "colspan", "1"),
            Decision::Drop(RemovalRule::DefaultValueOmitted)
        );
        assert_eq!(
            run("th", "rowspan", "01"),
            Decision::Drop(RemovalRule::DefaultValueOmitted)
        );
        assert_eq!(run("td", "colspan", "2"), kept("2"));
        assert_eq!(run("td", "colspan", "1000"), kept("1000"));
        assert_eq!(
            run("td", "colspan", "1001"),
            Decision::Drop(RemovalRule::AttributeValueRejected)
        );
        assert_eq!(run("td", "rowspan", "65534"), kept("65534"));
        assert_eq!(
            run("td", "rowspan", "0"),
            Decision::Drop(RemovalRule::AttributeValueRejected)
        );
        assert_eq!(run("ol", "start", "0"), kept("0"));
        assert_eq!(run("ol", "start", "9999"), kept("9999"));
        assert_eq!(
            run("ol", "start", "1"),
            Decision::Drop(RemovalRule::DefaultValueOmitted)
        );
        assert_eq!(
            run("ol", "start", "10000"),
            Decision::Drop(RemovalRule::AttributeValueRejected)
        );
    }

    #[test]
    fn enumerated_forced_and_marker_values() {
        for value in ["a", "A", "i", "I"] {
            assert_eq!(run("ol", "type", value), kept(value));
        }
        assert_eq!(
            run("ol", "type", "1"),
            Decision::Drop(RemovalRule::DefaultValueOmitted)
        );
        assert_eq!(
            run("ol", "type", "disc"),
            Decision::Drop(RemovalRule::AttributeValueRejected)
        );
        assert_eq!(run("a", "target", "_BLANK"), kept("_blank"));
        assert_eq!(run("iframe", "allow", "autoplay"), kept(IFRAME_ALLOW));
        assert_eq!(run("iframe", "allowfullscreen", "true"), kept(""));
        assert_eq!(
            run("iframe", "referrerpolicy", "x"),
            kept(IFRAME_REFERRER_POLICY)
        );
        assert_eq!(run("p", "data-youtube-video", "x"), kept(""));
        assert_eq!(run("p", "data-padlet-embed", ""), kept(""));
        assert_eq!(
            run("a", "rel", "noopener noreferrer"),
            kept("noopener noreferrer")
        );
        assert_eq!(
            run("img", "alt", "  A díjazottak  "),
            kept("  A díjazottak  ")
        );
        assert_eq!(run("iframe", "title", "Videó"), kept("Videó"));
    }

    #[test]
    fn style_and_unlisted_attributes_are_dropped() {
        assert_eq!(
            run("td", "style", "width: 10px"),
            Decision::Drop(RemovalRule::StyleRejected)
        );
        assert_eq!(
            run("p", "style", ""),
            Decision::Drop(RemovalRule::StyleRejected)
        );
        for (element, attribute) in [
            ("p", "class"),
            ("a", "onclick"),
            ("img", "srcset"),
            ("iframe", "srcdoc"),
            ("iframe", "frameborder"),
            ("p", "data-x"),
            ("div", "style"),
            ("span", "title"),
        ] {
            assert_eq!(
                run(element, attribute, "x"),
                Decision::Drop(RemovalRule::AttributeNotAllowed),
                "{element}[{attribute}]"
            );
        }
    }

    #[test]
    fn urls_use_their_rules() {
        assert_eq!(
            run(
                "img",
                "src",
                "https://verseghy-gimnazium.net/storage/uploads/a.jpg"
            ),
            Decision::Keep(Cow::Owned("/storage/uploads/a.jpg".into()))
        );
        assert_eq!(
            run("img", "src", "/x.jpg"),
            Decision::Drop(RemovalRule::ImageSrcRejected)
        );
        assert_eq!(
            run("iframe", "src", "https://padlet.com/embed/k"),
            Decision::Keep(Cow::Owned("https://padlet.com/embed/k".into()))
        );
        assert_eq!(
            run("iframe", "src", "https://evil.example/embed/k"),
            Decision::Drop(RemovalRule::EmbedSrcRejected)
        );
        assert_eq!(run("a", "href", "/hirek"), kept("/hirek"));
        assert_eq!(
            run("a", "href", "/storage/x.svg"),
            Decision::Drop(RemovalRule::LinkHrefRejected)
        );
        assert_eq!(
            filter(&SanitizePolicy::content_v1(), "a", "href", "/storage/x.svg"),
            None
        );
        assert_eq!(
            filter(&SanitizePolicy::content_v1(), "a", "href", "/x"),
            Some(Cow::Borrowed("/x"))
        );
    }

    proptest! {
        #[test]
        fn every_kept_value_is_kept_again_unchanged(index in 0..ELEMENTS.len(), attribute in "[a-z-]{1,16}", value in any::<String>()) {
            let element = ELEMENTS[index];
            for name in element.attributes.iter().map(|a| a.name).chain([attribute.as_str()]) {
                if let Decision::Keep(first) = run(element.name, name, &value) {
                    prop_assert_eq!(run(element.name, name, &first), Decision::Keep(Cow::Borrowed(first.as_ref())));
                }
            }
        }

        #[test]
        fn integers_inside_the_range_are_kept(number in 1u32..=5000, zeros in 0usize..3) {
            let value = format!("{}{number}", "0".repeat(zeros));
            let expected = number.to_string();
            let kept = filter(&SanitizePolicy::content_v1(), "img", "width", &value);
            prop_assert_eq!(kept.as_deref(), Some(expected.as_str()));
        }

        #[test]
        fn integers_outside_the_range_are_rejected(number in 5001u64..10_000_000_000) {
            let value = number.to_string();
            prop_assert_eq!(run("img", "height", &value), Decision::Drop(RemovalRule::AttributeValueRejected));
        }

        #[test]
        fn unlisted_attributes_never_pass(element in "[a-z]{1,10}", attribute in "(on|data-x|aria-)[a-z]{0,8}", value in any::<String>()) {
            prop_assert_eq!(filter(&SanitizePolicy::content_v1(), &element, &attribute, &value), None);
        }
    }
}
