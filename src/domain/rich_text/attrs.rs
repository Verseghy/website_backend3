//! The attribute filter (editor.md §7.3): for an attribute that passed
//! ammonia's allow-list, the value to store or the rule that drops it.
//!
//! [`decide`] is the whole decision, with the rule kept for the removal
//! report; [`filter`] is its shape for ammonia's `attribute_filter`. Both
//! read only policy data: the tables of [`super::policy`] and, for `style`,
//! the CSS grammar of [`super::css::grammar`]. A change to either is a
//! policy change and the only way to change what the filter does.
//!
//! The removal report classifies a `style` with the same grammar:
//! [`decide_style`] for the styles the filter decides, and
//! [`forced_style_rule`] for the `table` and `tbody` styles, which ammonia
//! forces before the filter sees them.
//!
//! ammonia also calls the filter for the attributes of elements it is about
//! to unwrap or remove; those answers are never written, so the filter drops
//! everything that is not in the tables, which is harmless there and a
//! second line of defence for the elements ammonia keeps.

use std::borrow::Cow;

use super::css;
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
        ValueRule::Style => decide_style(element, value).0,
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

/// Decides a `style` attribute with the CSS grammar, and names the rule the
/// removal report records for it, if any.
///
/// The rule is [`RemovalRule::StyleRejected`] when no declaration is left
/// because some were removed, [`RemovalRule::DefaultValueOmitted`] when none
/// is left because the attribute was empty or held only defaults (or a cell
/// `box-sizing` without a size), [`RemovalRule::StyleDeclarationRejected`]
/// when some declarations were removed and others kept, and
/// [`RemovalRule::ValueCanonicalised`] when every declaration was kept but
/// the value was re-spelled: re-sorted, a shorthand expanded, a value
/// written in its one spelling. [`RemovalRule::StyleOutsideAlphabet`] marks
/// a written string that the alphabet guard dropped, which is a bug. The
/// report and the filter both call this, so they cannot disagree. A forced
/// style never reaches it: see [`forced_style_rule`].
///
/// - `element`: the tag name, as the HTML parser reports it.
/// - `value`: the `style` value, after character references are decoded.
#[must_use]
pub fn decide_style<'a>(element: &str, value: &'a str) -> (Decision<'a>, Option<RemovalRule>) {
    style_decision(css::sanitize_style(element, value), value)
}

/// The decision and the report rule for one filtered `style`, as
/// [`decide_style`] describes them. It is apart from the filtering so that
/// the alphabet guard's case, which no input reaches, is tested too.
///
/// - `style`: what the CSS grammar made of the attribute.
/// - `value`: the attribute value it was made from.
fn style_decision(style: css::Style, value: &str) -> (Decision<'_>, Option<RemovalRule>) {
    if style.outside_alphabet {
        let rule = RemovalRule::StyleOutsideAlphabet;
        return (Decision::Drop(rule), Some(rule));
    }
    match style.css {
        None => {
            let rule = if style.rejected > 0 {
                RemovalRule::StyleRejected
            } else {
                RemovalRule::DefaultValueOmitted
            };
            (Decision::Drop(rule), Some(rule))
        }
        Some(kept) => {
            let rule = if style.rejected > 0 {
                Some(RemovalRule::StyleDeclarationRejected)
            } else {
                (kept != value).then_some(RemovalRule::ValueCanonicalised)
            };
            let kept = if kept == value {
                Cow::Borrowed(value)
            } else {
                Cow::Owned(kept)
            };
            (Decision::Keep(kept), rule)
        }
    }
}

/// The rule the removal report records for a `style` that the policy
/// forces (`table` and `tbody`, editor.md §7.3), or `None` when the input
/// already held the forced value.
///
/// ammonia writes `forced` whatever the input held, so the filter never
/// decides this attribute. The grammar still classifies what the input
/// lost: [`RemovalRule::StyleDeclarationRejected`] when it rejects a
/// declaration (a legacy `width: 100%` on a table), and otherwise
/// [`RemovalRule::ValueCanonicalised`] when the value is not `forced` byte
/// for byte (an empty style, another order, another case). The attribute is
/// never removed, so [`RemovalRule::StyleRejected`] cannot occur here.
///
/// - `element`: the tag name, as the HTML parser reports it.
/// - `value`: the `style` value in the input, after character references
///   are decoded.
/// - `forced`: the value the policy forces on the element
///   ([`super::policy::TABLE_STYLE`] or [`super::policy::TBODY_STYLE`]).
#[must_use]
pub fn forced_style_rule(element: &str, value: &str, forced: &str) -> Option<RemovalRule> {
    if css::sanitize_style(element, value).rejected > 0 {
        Some(RemovalRule::StyleDeclarationRejected)
    } else {
        // Compared with the forced value, which ammonia writes whatever the
        // input held: an input that is not that value byte for byte (an
        // empty style, another order, another case) was re-spelled.
        (value != forced).then_some(RemovalRule::ValueCanonicalised)
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
    use crate::domain::rich_text::policy::{
        ELEMENTS, IFRAME_ALLOW, IFRAME_REFERRER_POLICY, TABLE_STYLE, TBODY_STYLE,
    };
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
    fn styles_go_through_the_css_grammar() {
        assert_eq!(
            run("td", "style", "box-sizing: content-box; width: 10px"),
            kept("box-sizing: content-box; width: 10px")
        );
        assert_eq!(
            run("td", "style", "width: 10px"),
            Decision::Keep(Cow::Owned("box-sizing: content-box; width: 10px".into()))
        );
        assert_eq!(
            run("span", "style", "COLOR: red"),
            Decision::Keep(Cow::Owned("color: #ff0000".into()))
        );
        assert_eq!(
            run("p", "style", "color: red"),
            Decision::Drop(RemovalRule::StyleRejected)
        );
        assert_eq!(
            run("p", "style", ""),
            Decision::Drop(RemovalRule::DefaultValueOmitted)
        );
        // The table styles are forced: ammonia writes the one value.
        assert_eq!(run("table", "style", "width: 100%"), kept(TABLE_STYLE));
        assert_eq!(run("tbody", "style", ""), kept(TBODY_STYLE));
    }

    #[test]
    fn the_style_rule_says_what_happened() {
        let rule = |element, value| decide_style(element, value).1;
        assert_eq!(rule("span", "color: #ff0000"), None);
        assert_eq!(
            rule("span", "color: red"),
            Some(RemovalRule::ValueCanonicalised)
        );
        assert_eq!(
            rule("span", "color: #ff0000; font-family: Calibri"),
            Some(RemovalRule::StyleDeclarationRejected)
        );
        assert_eq!(
            rule("span", "font-family: Calibri"),
            Some(RemovalRule::StyleRejected)
        );
        assert_eq!(
            rule("ul", "list-style-type: disc"),
            Some(RemovalRule::DefaultValueOmitted)
        );
        assert_eq!(
            rule("img", "width: 350px"),
            Some(RemovalRule::ValueCanonicalised),
            "the locked declarations are added"
        );
        assert_eq!(
            rule("td", "border: 1px solid #000000"),
            Some(RemovalRule::ValueCanonicalised),
            "a shorthand is expanded, not removed"
        );
        assert_eq!(
            rule("td", "width: 10px; box-sizing: content-box"),
            Some(RemovalRule::ValueCanonicalised),
            "re-sorted"
        );
        assert_eq!(
            rule("td", "box-sizing: content-box"),
            Some(RemovalRule::DefaultValueOmitted),
            "a box-sizing without a size changes nothing"
        );
        assert_eq!(
            decide_style("span", "color: #ff0000; font-family: Calibri").0,
            Decision::Keep(Cow::Owned("color: #ff0000".into()))
        );
    }

    /// The alphabet guard's flag drops the attribute under its own rule,
    /// whatever else the grammar counted. No input reaches it (the
    /// grammar's proptests check that), so the decision is fed by hand.
    #[test]
    fn a_style_outside_the_alphabet_is_dropped_under_its_own_rule() {
        let flagged = |rejected| css::Style {
            css: None,
            rejected,
            outside_alphabet: true,
        };
        for rejected in [0, 2] {
            assert_eq!(
                style_decision(flagged(rejected), "color: red"),
                (
                    Decision::Drop(RemovalRule::StyleOutsideAlphabet),
                    Some(RemovalRule::StyleOutsideAlphabet)
                )
            );
        }
        let fine = css::Style {
            css: Some("color: #ff0000".into()),
            rejected: 0,
            outside_alphabet: false,
        };
        assert_eq!(
            style_decision(fine, "color: #ff0000"),
            (Decision::Keep(Cow::Borrowed("color: #ff0000")), None)
        );
    }

    #[test]
    fn a_forced_style_names_what_it_lost() {
        let table = |value| forced_style_rule("table", value, TABLE_STYLE);
        assert_eq!(table(TABLE_STYLE), None);
        // The legacy table width of editor.md §3.4 is a removal.
        assert_eq!(
            table("width: 100%"),
            Some(RemovalRule::StyleDeclarationRejected)
        );
        assert_eq!(
            table("width: 100%; border-collapse: collapse"),
            Some(RemovalRule::StyleDeclarationRejected)
        );
        assert_eq!(
            table("border-collapse: separate; max-width: fit-content"),
            Some(RemovalRule::StyleDeclarationRejected)
        );
        // Nothing lost, but not the one spelling.
        assert_eq!(table(""), Some(RemovalRule::ValueCanonicalised));
        assert_eq!(
            table("border-collapse: collapse"),
            Some(RemovalRule::ValueCanonicalised)
        );
        assert_eq!(
            table("max-width: fit-content; border-collapse: collapse"),
            Some(RemovalRule::ValueCanonicalised),
            "the grammar keeps this order, ammonia does not"
        );
        assert_eq!(
            table(&TABLE_STYLE.to_uppercase()),
            Some(RemovalRule::ValueCanonicalised)
        );
        let tbody = |value| forced_style_rule("tbody", value, TBODY_STYLE);
        assert_eq!(tbody(TBODY_STYLE), None);
        assert_eq!(
            tbody("display: table"),
            Some(RemovalRule::StyleDeclarationRejected)
        );
        assert_eq!(
            tbody("display: TABLE-ROW-GROUP"),
            Some(RemovalRule::ValueCanonicalised)
        );
    }

    #[test]
    fn unlisted_attributes_are_dropped() {
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
        fn the_style_rule_is_none_exactly_when_nothing_changed(
            element in prop::sample::select(&["p", "span", "td", "img", "iframe", "ul", "li"][..]),
            value in "[a-z0-9:;#. %()/!-]{0,40}",
        ) {
            let (decision, rule) = decide_style(element, &value);
            let unchanged = decision == Decision::Keep(Cow::Borrowed(value.as_str()));
            prop_assert_eq!(rule.is_none(), unchanged, "{} {:?}", value, decision);
            if let Decision::Drop(dropped) = decision {
                prop_assert_eq!(rule, Some(dropped));
            }
        }

        #[test]
        fn a_forced_style_rule_is_none_exactly_for_the_forced_value(
            forced in prop::sample::select(&[("table", TABLE_STYLE), ("tbody", TBODY_STYLE)][..]),
            parts in prop::collection::vec(prop::sample::select(&[
                "border-collapse: collapse", "max-width: fit-content", "display: table-row-group",
                "BORDER-COLLAPSE: COLLAPSE", "width: 100%", "display: table", "color: red", "", " ",
            ][..]), 0..4),
        ) {
            let (element, forced) = forced;
            let value = parts.join("; ");
            let rule = forced_style_rule(element, &value, forced);
            prop_assert_eq!(rule.is_none(), value == forced, "{}", value);
            let lost = css::sanitize_style(element, &value).rejected > 0;
            prop_assert_eq!(rule == Some(RemovalRule::StyleDeclarationRejected), lost, "{}", value);
            prop_assert_ne!(rule, Some(RemovalRule::StyleRejected));
        }

        #[test]
        fn unlisted_attributes_never_pass(element in "[a-z]{1,10}", attribute in "(on|data-x|aria-)[a-z]{0,8}", value in any::<String>()) {
            prop_assert_eq!(filter(&SanitizePolicy::content_v1(), &element, &attribute, &value), None);
        }
    }
}
