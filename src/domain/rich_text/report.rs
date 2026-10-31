//! The removal report: what the sanitiser removed or rewrote, counted per
//! rule (editor.md §7.2).
//!
//! `validate_content` turns some of these counts into admin errors (an image
//! whose source was dropped is `image_src_rejected`), the drift canary
//! returns them when an admin write changed, and the importer and
//! `content_normalize` record them (editor.md §7.6, §7.7).
//!
//! ammonia reports nothing about what it removes, and its own allow-list
//! step runs before any callback of ours, so the report cannot be collected
//! while ammonia runs. [`explain`] derives it instead: it parses the input
//! exactly as ammonia does (the same html5ever, a fragment in a `div`
//! context) and walks the tree with the same tables, the same namespace
//! check, the same scheme check, the same filter and the same relative-URL
//! rule, in ammonia's order. The sanitiser's proptests compare the report
//! with ammonia's real output: the attribute entries of single elements,
//! and, over nested trees with table cells, the removed and unwrapped
//! elements, the comments and the rejected image sources. An ammonia upgrade
//! that changes the order or the namespace lists fails them.
//!
//! [`explain`] covers the ammonia step only. The structural pass (editor.md
//! §7.5) runs on a tree this crate builds, so it records its own removals
//! while it runs ([`RemovalRule::IframeContentRemoved`],
//! [`RemovalRule::ImageWithoutSrcRemoved`],
//! [`RemovalRule::NestedSubSupUnwrapped`],
//! [`RemovalRule::NestedLinkUnwrapped`]), and the fixed-point loop merges the
//! reports of every pass into one.

use std::collections::BTreeMap;

use html5ever::driver::{ParseOpts, parse_fragment};
use html5ever::tendril::TendrilSink;
use html5ever::{QualName, local_name, ns};
use scraper::{Html, HtmlTreeSink, Node};

use super::attrs::{self, Decision};
use super::policy::{
    ElementRule, LINK_REL, REMOVED_WITH_CONTENT, SanitizePolicy, ValueRule, element_rule,
};
use super::url_rules;

/// Why something was removed or rewritten.
///
/// The names of [`Self::as_str`] are provisional: they become stable when
/// the drift canary's DTO fields and the `rich_text_normalized_total{rule}`
/// labels are fixed (editor.md §7.7, §16 item 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RemovalRule {
    /// A comment was removed.
    CommentRemoved,
    /// An element was removed together with its content (editor.md §3.5).
    ElementRemoved,
    /// An element outside content-v1 was removed and its content kept.
    ElementUnwrapped,
    /// An attribute that its element does not allow was removed.
    AttributeNotAllowed,
    /// An allowed attribute had a value outside its rule and was removed.
    AttributeValueRejected,
    /// An attribute that held its default value was omitted.
    DefaultValueOmitted,
    /// An attribute's value was rewritten into its canonical spelling.
    ValueCanonicalised,
    /// A `style` attribute was removed: none of its declarations is allowed
    /// on its element (editor.md §3.3, §3.4).
    StyleRejected,
    /// A `style` attribute was kept, but some of its declarations were
    /// removed.
    StyleDeclarationRejected,
    /// An image's `src` was removed (editor.md §3.6).
    ImageSrcRejected,
    /// An embed's `src` was removed (editor.md §3.6).
    EmbedSrcRejected,
    /// A link's `href` was removed (editor.md §3.6).
    LinkHrefRejected,
    /// The child nodes of an `iframe` were removed; counted once per
    /// `iframe` (the structural pass, editor.md §7.5).
    IframeContentRemoved,
    /// An `img` without `src` was removed (the structural pass, editor.md
    /// §7.5). An `img` whose `src` the filter dropped counts here as well as
    /// under [`Self::ImageSrcRejected`].
    ImageWithoutSrcRemoved,
    /// A `sub` or `sup` inside another `sub` or `sup` was unwrapped (the
    /// structural pass, editor.md §7.5).
    NestedSubSupUnwrapped,
    /// An `a` inside another `a`, with no table cell between them, was
    /// unwrapped: its content stays in the outer link (the structural pass,
    /// editor.md §7.5, ADR 0037).
    NestedLinkUnwrapped,
}

impl RemovalRule {
    /// The `snake_case` name of the rule.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommentRemoved => "comment_removed",
            Self::ElementRemoved => "element_removed",
            Self::ElementUnwrapped => "element_unwrapped",
            Self::AttributeNotAllowed => "attribute_not_allowed",
            Self::AttributeValueRejected => "attribute_value_rejected",
            Self::DefaultValueOmitted => "default_value_omitted",
            Self::ValueCanonicalised => "value_canonicalised",
            Self::StyleRejected => "style_rejected",
            Self::StyleDeclarationRejected => "style_declaration_rejected",
            Self::ImageSrcRejected => "image_src_rejected",
            Self::EmbedSrcRejected => "embed_src_rejected",
            Self::LinkHrefRejected => "link_href_rejected",
            Self::IframeContentRemoved => "iframe_content_removed",
            Self::ImageWithoutSrcRemoved => "image_without_src_removed",
            Self::NestedSubSupUnwrapped => "nested_sub_sup_unwrapped",
            Self::NestedLinkUnwrapped => "nested_link_unwrapped",
        }
    }
}

/// One kind of removal: the rule, and the element and attribute it hit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Removal {
    /// The rule that removed or rewrote it.
    pub rule: RemovalRule,
    /// The tag name, or `#comment` for a comment.
    pub element: String,
    /// The attribute name, for the attribute rules.
    pub attribute: Option<String>,
}

/// Every removal of one sanitiser run, counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemovalReport {
    entries: BTreeMap<Removal, usize>,
}

impl RemovalReport {
    /// Whether nothing was removed or rewritten.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many removals `rule` made, over every element and attribute.
    ///
    /// - `rule`: the rule to count.
    #[must_use]
    pub fn count(&self, rule: RemovalRule) -> usize {
        self.entries
            .iter()
            .filter(|(removal, _)| removal.rule == rule)
            .map(|(_, count)| count)
            .sum()
    }

    /// The count of every rule that removed anything.
    #[must_use]
    pub fn per_rule(&self) -> BTreeMap<RemovalRule, usize> {
        let mut counts = BTreeMap::new();
        for (removal, count) in &self.entries {
            *counts.entry(removal.rule).or_insert(0) += count;
        }
        counts
    }

    /// Every removal with its count, ordered by rule, element and attribute.
    pub fn entries(&self) -> impl Iterator<Item = (&Removal, usize)> {
        self.entries
            .iter()
            .map(|(removal, &count)| (removal, count))
    }

    /// Counts one removal.
    ///
    /// - `rule`: why it was removed.
    /// - `element`: the tag name, or `#comment`.
    /// - `attribute`: the attribute name, for the attribute rules.
    pub(super) fn record(&mut self, rule: RemovalRule, element: &str, attribute: Option<&str>) {
        let removal = Removal {
            rule,
            element: element.to_owned(),
            attribute: attribute.map(str::to_owned),
        };
        *self.entries.entry(removal).or_insert(0) += 1;
    }

    /// Adds every count of `other` to this report: one report covers all
    /// passes of a sanitiser run.
    ///
    /// - `other`: the removals of one more pass.
    pub(super) fn merge(&mut self, other: Self) {
        for (removal, count) in other.entries {
            *self.entries.entry(removal).or_insert(0) += count;
        }
    }
}

/// HTML-namespace tag names that ammonia 4.2.4 removes together with their
/// content although they are not in `clean_content_tags`: the lower-case
/// names of its `is_svg_tag` and `is_mathml_tag` lists, without `title`,
/// `style`, `font`, `a`, `script` and `span`, which it also accepts as HTML
/// (ammonia@4.2.4:src/lib.rs, `check_expected_namespace` and `is_html_tag`).
/// An HTML parser lower-cases tag names, so the mixed-case names of those
/// lists (`clipPath`, `foreignObject`) never occur on an HTML element and are
/// left out, and so is `image`, which the parser renames to `img`.
#[rustfmt::skip]
pub const AMMONIA_FOREIGN_TAGS: &[&str] = &[
    "abs", "and", "animate", "annotation", "annotation-xml", "apply", "approx", "arccos",
    "arccosh", "arccot", "arccoth", "arccsc", "arccsch", "arcsec", "arcsech", "arcsin", "arcsinh",
    "arctan", "arctanh", "arg", "bind", "bvar", "card", "cartesianproduct", "cbytes", "ceiling",
    "cerror", "ci", "circle", "cn", "codomain", "complexes", "compose", "condition", "conjugate",
    "cos", "cosh", "cot", "coth", "cs", "csc", "csch", "csymbol", "curl", "declare", "defs",
    "degree", "desc", "determinant", "diff", "discard", "divergence", "divide", "domain",
    "domainofapplication", "ellipse", "emptyset", "eq", "equivalent", "eulergamma", "exists",
    "exp", "exponentiale", "factorial", "factorof", "false", "filter", "floor", "fn", "forall",
    "g", "gcd", "geq", "grad", "gt", "ident", "imaginary", "imaginaryi", "implies", "in",
    "infinity", "int", "integers", "intersect", "interval", "inverse", "lambda", "laplacian",
    "lcm", "leq", "limit", "line", "list", "ln", "log", "logbase", "lowlimit", "lt", "maction",
    "maligngroup", "malignmark", "marker", "mask", "math", "matrix", "matrixrow", "max", "mean",
    "median", "menclose", "merror", "metadata", "mfenced", "mfrac", "mglyph", "mi", "min",
    "minus", "mlabeledtr", "mlongdiv", "mmultiscripts", "mn", "mo", "mode", "moment",
    "momentabout", "mover", "mpadded", "mpath", "mphantom", "mprescripts", "mroot", "mrow", "ms",
    "mscarries", "mscarry", "msgroup", "msline", "mspace", "msqrt", "msrow", "mstack", "mstyle",
    "msub", "msubsup", "msup", "mtable", "mtd", "mtext", "mtr", "munder", "munderover",
    "naturalnumbers", "neq", "none", "not", "notanumber", "notin", "notprsubset", "notsubset",
    "or", "otherwise", "outerproduct", "partialdiff", "path", "pattern", "pi", "piece",
    "piecewise", "plus", "polygon", "polyline", "power", "primes", "product", "prsubset",
    "quotient", "rationals", "real", "reals", "rect", "reln", "rem", "root", "scalarproduct",
    "sdev", "sec", "sech", "selector", "semantics", "sep", "set", "setdiff", "share", "sin",
    "sinh", "stop", "subset", "sum", "svg", "switch", "symbol", "tan", "tanh", "tendsto", "text",
    "times", "transpose", "true", "tspan", "union", "uplimit", "use", "variance", "vector",
    "vectorproduct", "view", "xor",
];

/// Parses `input` the way ammonia 4.2.4 does (ammonia@4.2.4:src/lib.rs,
/// `make_parser`): an HTML fragment in a `div` context, default options.
///
/// - `input`: the HTML to parse.
#[must_use]
pub fn parse_like_ammonia(input: &str) -> Html {
    parse_fragment(
        HtmlTreeSink::new(Html::new_fragment()),
        ParseOpts::default(),
        QualName::new(None, ns!(html), local_name!("div")),
        Vec::new(),
        false,
    )
    .one(input)
}

/// The removal report that sanitising `input` with ammonia under `policy`
/// produces.
///
/// - `policy`: the policy the sanitiser was built from.
/// - `input`: the sanitiser's input.
#[must_use]
pub fn explain(policy: &SanitizePolicy, input: &str) -> RemovalReport {
    let html = parse_like_ammonia(input);
    let mut report = RemovalReport::default();
    // An explicit stack, like ammonia's own walk, so a deeply nested input
    // cannot overflow the call stack.
    let mut stack: Vec<_> = html.root_element().children().collect();
    while let Some(node) = stack.pop() {
        match node.value() {
            Node::Comment(_) => report.record(RemovalRule::CommentRemoved, "#comment", None),
            Node::Element(element) => {
                let name = element.name();
                // ammonia removes these after its allow-list step and never
                // visits their content. `svg` and `math` are removed here, so
                // every element the walk reaches is in the HTML namespace,
                // which is the case where ammonia's namespace check reduces
                // to the name list.
                if REMOVED_WITH_CONTENT.contains(&name) || AMMONIA_FOREIGN_TAGS.contains(&name) {
                    report.record(RemovalRule::ElementRemoved, name, None);
                    continue;
                }
                match element_rule(name) {
                    Some(rule) => explain_attributes(policy, rule, element.attrs(), &mut report),
                    None => report.record(RemovalRule::ElementUnwrapped, name, None),
                }
                stack.extend(node.children());
            }
            _ => {}
        }
    }
    report
}

/// The removals among the attributes of one kept element, in ammonia's
/// order: the allow-list with its scheme check, then the filter, then the
/// relative-URL rule.
fn explain_attributes<'a>(
    policy: &SanitizePolicy,
    element: &ElementRule,
    attributes: impl Iterator<Item = (&'a str, &'a str)>,
    report: &mut RemovalReport,
) {
    let tag = element.name;
    for (name, value) in attributes {
        let mut record = |rule| report.record(rule, tag, Some(name));
        let Some(attribute) = element.attribute(name) else {
            record(RemovalRule::AttributeNotAllowed);
            continue;
        };
        let rejected_url = match attribute.rule {
            ValueRule::ImageSrc => RemovalRule::ImageSrcRejected,
            ValueRule::EmbedSrc => RemovalRule::EmbedSrcRejected,
            _ => RemovalRule::LinkHrefRejected,
        };
        // A style is classified by the CSS grammar. That includes the forced
        // styles of `table` and `tbody`: ammonia writes their one value, but a
        // declaration the grammar rejects there (a legacy `width: 100%`) is a
        // removal, not a re-spelling.
        let style = match attribute.rule {
            ValueRule::Style => Some(attrs::decide_style(tag, value).1),
            ValueRule::Forced(forced) if name == "style" => {
                Some(attrs::forced_style_rule(tag, value, forced))
            }
            _ => None,
        };
        if let Some(rule) = style {
            if let Some(rule) = rule {
                record(rule);
            }
            continue;
        }
        // ammonia removes `rel` and appends its own, and a forced attribute
        // ends up with its canonical value however it was written.
        let fixed_value = match attribute.rule {
            ValueRule::Forced(canonical) => Some(canonical),
            ValueRule::LinkRel => Some(LINK_REL),
            _ => None,
        };
        if let Some(canonical) = fixed_value {
            if value != canonical {
                record(RemovalRule::ValueCanonicalised);
            }
            continue;
        }
        match attribute.rule {
            // ammonia compares enumerated values with `to_lowercase`.
            ValueRule::Enumerated { values, .. }
                if !values
                    .iter()
                    .any(|allowed| allowed.to_lowercase() == value.to_lowercase()) =>
            {
                record(RemovalRule::AttributeValueRejected);
                continue;
            }
            rule if rule.is_url() && !url_rules::scheme_admitted(value) => {
                record(rejected_url);
                continue;
            }
            _ => {}
        }
        match attrs::decide(policy, tag, name, value) {
            Decision::Drop(rule) => record(rule),
            Decision::Keep(kept)
                if attribute.rule.is_url()
                    && url_rules::is_relative(&kept)
                    && url_rules::same_origin_relative(&kept).is_none() =>
            {
                record(rejected_url);
            }
            Decision::Keep(kept) if kept != value => record(RemovalRule::ValueCanonicalised),
            Decision::Keep(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::policy::ELEMENTS;
    use proptest::prelude::*;

    fn report(input: &str) -> RemovalReport {
        explain(&SanitizePolicy::content_v1(), input)
    }

    fn entry(rule: RemovalRule, element: &str, attribute: Option<&str>) -> Removal {
        Removal {
            rule,
            element: element.into(),
            attribute: attribute.map(Into::into),
        }
    }

    #[test]
    fn rule_names_are_snake_case() {
        use RemovalRule::*;
        let names = [
            (CommentRemoved, "comment_removed"),
            (ElementRemoved, "element_removed"),
            (ElementUnwrapped, "element_unwrapped"),
            (AttributeNotAllowed, "attribute_not_allowed"),
            (AttributeValueRejected, "attribute_value_rejected"),
            (DefaultValueOmitted, "default_value_omitted"),
            (ValueCanonicalised, "value_canonicalised"),
            (StyleRejected, "style_rejected"),
            (StyleDeclarationRejected, "style_declaration_rejected"),
            (ImageSrcRejected, "image_src_rejected"),
            (EmbedSrcRejected, "embed_src_rejected"),
            (LinkHrefRejected, "link_href_rejected"),
            (IframeContentRemoved, "iframe_content_removed"),
            (ImageWithoutSrcRemoved, "image_without_src_removed"),
            (NestedSubSupUnwrapped, "nested_sub_sup_unwrapped"),
            (NestedLinkUnwrapped, "nested_link_unwrapped"),
        ];
        for (rule, name) in names {
            assert_eq!(rule.as_str(), name);
        }
    }

    #[test]
    fn the_parse_uses_a_div_fragment_context() {
        let text = |input: &str| -> Vec<String> {
            let html = parse_like_ammonia(input);
            let root = html.root_element();
            assert_eq!(root.value().name(), "html", "html5ever's fragment root");
            root.children()
                .map(|child| match child.value() {
                    Node::Text(text) => format!("text:{}", &**text),
                    Node::Element(element) => format!("element:{}", element.name()),
                    Node::Comment(comment) => format!("comment:{}", &**comment),
                    other => format!("{other:?}"),
                })
                .collect()
        };
        // Outside a table, cell and row tags are dropped and their text kept.
        assert_eq!(text("<td>x</td>"), ["text:x"]);
        assert_eq!(text("<tr><th>x</th></tr>"), ["text:x"]);
        // In a `div` context, a `body` tag is ignored too.
        assert_eq!(text("<body><p>a</p>"), ["element:p"]);
        assert_eq!(text("a<!--c-->"), ["text:a", "comment:c"]);
    }

    #[test]
    fn foreign_names_lose_their_content() {
        let report = report(r#"<p>a<g>secret</g><desc><img src="data:x" alt="a"></desc>b</p>"#);
        assert_eq!(
            report
                .entries()
                .map(|(r, n)| (r.clone(), n))
                .collect::<Vec<_>>(),
            [
                (entry(RemovalRule::ElementRemoved, "desc", None), 1),
                (entry(RemovalRule::ElementRemoved, "g", None), 1),
            ],
            "the image inside `desc` is never reached"
        );
        // Mixed case names exist only in the SVG namespace, inside `svg`.
        assert_eq!(
            super::explain(&SanitizePolicy::content_v1(), "<clipPath>x</clipPath>")
                .count(RemovalRule::ElementUnwrapped),
            1
        );
    }

    #[test]
    fn the_foreign_list_spares_content_v1() {
        for element in ELEMENTS {
            assert!(
                !AMMONIA_FOREIGN_TAGS.contains(&element.name),
                "{}",
                element.name
            );
        }
        for name in AMMONIA_FOREIGN_TAGS {
            assert_eq!(*name, name.to_ascii_lowercase(), "{name}");
        }
        assert!(
            AMMONIA_FOREIGN_TAGS.is_sorted(),
            "sorted, so a diff with ammonia's lists reads"
        );
    }

    #[test]
    fn forced_values_are_compared_exactly() {
        // ammonia admits case variants and writes the canonical value in
        // their place, which changes the bytes.
        let report = report(
            r#"<a href="/x" target="_BLANK">x</a><iframe referrerpolicy="STRICT-ORIGIN-WHEN-CROSS-ORIGIN" allowfullscreen=""></iframe>"#,
        );
        assert_eq!(
            report
                .entries()
                .map(|(r, n)| (r.clone(), n))
                .collect::<Vec<_>>(),
            [
                (
                    entry(RemovalRule::ValueCanonicalised, "a", Some("target")),
                    1
                ),
                (
                    entry(
                        RemovalRule::ValueCanonicalised,
                        "iframe",
                        Some("referrerpolicy")
                    ),
                    1
                ),
            ]
        );
    }

    #[test]
    fn elements_and_comments_are_counted() {
        let report = report(
            "<!-- x --><p>a<script>b<i>c</i></script><div><b>d</b><o:p></o:p></div></p>\
             <svg><p>inside</p></svg><xml><w:x>e</w:x></xml>",
        );
        let counted: Vec<_> = report
            .entries()
            .map(|(removal, count)| (removal.clone(), count))
            .collect();
        assert_eq!(
            counted,
            [
                (entry(RemovalRule::CommentRemoved, "#comment", None), 1),
                (entry(RemovalRule::ElementRemoved, "script", None), 1),
                (entry(RemovalRule::ElementRemoved, "svg", None), 1),
                (entry(RemovalRule::ElementRemoved, "xml", None), 1),
                (entry(RemovalRule::ElementUnwrapped, "b", None), 1),
                (entry(RemovalRule::ElementUnwrapped, "div", None), 1),
                (entry(RemovalRule::ElementUnwrapped, "o:p", None), 1),
            ]
        );
        assert_eq!(report.count(RemovalRule::ElementRemoved), 3);
        assert_eq!(
            report.per_rule().get(&RemovalRule::ElementUnwrapped),
            Some(&3)
        );
    }

    #[test]
    fn attributes_are_counted_per_rule() {
        let report = report(
            r#"<p class="x" style="color: red" data-youtube-video="1"><img src="data:image/png;base64,AA" alt="a" width="0350" height="0">
               <a href="//evil.example/" target="_self" rel="nofollow" onclick="x">l</a><a href="/storage/x.svg">s</a>
               <a href="javascript:alert(1)">j</a><iframe src="https://evil.example/embed/x"></iframe></p>
               <ol start="1" type="disc"><li>x</li></ol><table><tbody><tr><td colspan="1">c</td></tr></tbody></table>"#,
        );
        let per_rule = report.per_rule();
        assert_eq!(
            per_rule[&RemovalRule::AttributeNotAllowed],
            2,
            "class, onclick"
        );
        assert_eq!(per_rule[&RemovalRule::StyleRejected], 1);
        assert_eq!(
            per_rule[&RemovalRule::ValueCanonicalised],
            4,
            "marker, width, target, rel"
        );
        assert_eq!(per_rule[&RemovalRule::ImageSrcRejected], 1);
        assert_eq!(
            per_rule[&RemovalRule::AttributeValueRejected],
            2,
            "height, type"
        );
        assert_eq!(
            per_rule[&RemovalRule::LinkHrefRejected],
            3,
            "//evil, /storage/x.svg, javascript:"
        );
        assert_eq!(per_rule[&RemovalRule::EmbedSrcRejected], 1);
        assert_eq!(
            per_rule[&RemovalRule::DefaultValueOmitted],
            2,
            "start, colspan"
        );
        assert_eq!(report.count(RemovalRule::CommentRemoved), 0);
    }

    #[test]
    fn styles_are_counted_by_what_happened() {
        // The forced table and tbody styles are classified by the grammar too:
        // a rejected declaration there is a removal, another spelling of the
        // forced value a re-spelling, and the forced value itself nothing.
        let report = report(
            r#"<p style="color: red">a</p><span style="color: red; font-family: x">b</span><span style="COLOR: red">c</span>
               <span style="color: #ff0000">d</span><ul style="list-style-type: disc"><li>e</li></ul>
               <table style="width: 100%"><tbody style="display: table"><tr><td style="width: 10px">f</td></tr></tbody></table>
               <table style="max-width: fit-content; border-collapse: collapse"><tbody style="display: table-row-group"><tr><td>g</td></tr></tbody></table>
               <table style="border-collapse: collapse; max-width: fit-content"><tbody><tr><td>h</td></tr></tbody></table>"#,
        );
        assert_eq!(
            report
                .entries()
                .map(|(r, n)| (r.clone(), n))
                .collect::<Vec<_>>(),
            [
                (
                    entry(RemovalRule::DefaultValueOmitted, "ul", Some("style")),
                    1
                ),
                (
                    entry(RemovalRule::ValueCanonicalised, "span", Some("style")),
                    1
                ),
                (
                    entry(RemovalRule::ValueCanonicalised, "table", Some("style")),
                    1
                ),
                (entry(RemovalRule::StyleRejected, "p", Some("style")), 1),
                (
                    entry(RemovalRule::StyleDeclarationRejected, "span", Some("style")),
                    1
                ),
                (
                    entry(
                        RemovalRule::StyleDeclarationRejected,
                        "table",
                        Some("style")
                    ),
                    1
                ),
                (
                    entry(
                        RemovalRule::StyleDeclarationRejected,
                        "tbody",
                        Some("style")
                    ),
                    1
                ),
            ]
        );
    }

    #[test]
    fn canonical_content_reports_nothing() {
        let report = report(
            r#"<h2>Cím</h2><p>Szöveg <strong>félkövér</strong> <a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a></p><ol start="5" type="a"><li>első</li></ol><p><img src="/storage/uploads/a.jpg" alt="Kép" width="350" height="495"></p>"#,
        );
        assert!(report.is_empty(), "{report:?}");
        assert_eq!(report.count(RemovalRule::ImageSrcRejected), 0);
        assert!(RemovalReport::default().per_rule().is_empty());
    }

    #[test]
    fn merge_adds_the_counts_of_another_pass() {
        let mut first = RemovalReport::default();
        first.record(RemovalRule::CommentRemoved, "#comment", None);
        first.record(RemovalRule::ImageSrcRejected, "img", Some("src"));
        let mut second = RemovalReport::default();
        second.record(RemovalRule::ImageSrcRejected, "img", Some("src"));
        second.record(RemovalRule::ImageWithoutSrcRemoved, "img", None);
        first.merge(second);
        assert_eq!(
            first
                .entries()
                .map(|(r, n)| (r.clone(), n))
                .collect::<Vec<_>>(),
            [
                (entry(RemovalRule::CommentRemoved, "#comment", None), 1),
                (entry(RemovalRule::ImageSrcRejected, "img", Some("src")), 2),
                (entry(RemovalRule::ImageWithoutSrcRemoved, "img", None), 1),
            ]
        );
        first.merge(RemovalReport::default());
        assert_eq!(first.per_rule().values().sum::<usize>(), 4);
    }

    #[test]
    fn the_walk_survives_deep_nesting() {
        let deep = "<div>".repeat(5_000);
        assert_eq!(report(&deep).count(RemovalRule::ElementUnwrapped), 5_000);
    }

    proptest! {
        #[test]
        fn explain_never_panics(input in any::<String>()) {
            let _ = report(&input);
        }

        // Text without markup, references, CR or NUL is one text node, kept
        // byte for byte; the parser rewrites only those.
        #[test]
        fn plain_text_parses_to_itself(text in "[a-zA-Z0-9 .,;:!?'\"/\\n\\t-]{1,40}") {
            let html = parse_like_ammonia(&text);
            let children: Vec<_> = html.root_element().children().collect();
            prop_assert_eq!(children.len(), 1);
            let parsed = match children[0].value() { Node::Text(t) => t.to_string(), other => format!("{other:?}") };
            prop_assert_eq!(parsed, text);
        }

        #[test]
        fn merging_reports_adds_every_count(
            first in prop::collection::vec(prop::sample::select(&["div", "b", "script", "p", "font"][..]), 0..10),
            second in prop::collection::vec(prop::sample::select(&["div", "b", "script", "p", "font"][..]), 0..10),
        ) {
            let render = |tags: &[&str]| tags.iter().map(|tag| format!("<{tag}>x</{tag}>")).collect::<Vec<_>>().concat();
            let (a, b) = (report(&render(&first)), report(&render(&second)));
            let mut merged = a.clone();
            merged.merge(b.clone());
            for rule in [RemovalRule::ElementRemoved, RemovalRule::ElementUnwrapped] {
                prop_assert_eq!(merged.count(rule), a.count(rule) + b.count(rule));
            }
            let both = [first, second].concat();
            prop_assert_eq!(merged, report(&render(&both)));
        }

        #[test]
        fn counts_add_up(tags in prop::collection::vec(prop::sample::select(&["div", "b", "font", "script", "p", "em"][..]), 0..20)) {
            let input = tags.iter().map(|tag| format!("<{tag}>x</{tag}>")).collect::<Vec<_>>().concat();
            let report = report(&input);
            let total: usize = report.entries().map(|(_, count)| count).sum();
            prop_assert_eq!(total, report.per_rule().values().sum::<usize>());
            prop_assert_eq!(report.count(RemovalRule::ElementRemoved), tags.iter().filter(|t| **t == "script").count());
            prop_assert_eq!(report.count(RemovalRule::ElementUnwrapped), tags.iter().filter(|t| ["div", "b", "font"].contains(t)).count());
        }
    }
}
