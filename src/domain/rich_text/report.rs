//! The removal report: what the sanitiser removed or rewrote, counted per
//! rule (editor.md §7.2).
//!
//! [`super::validate::validate_content`] turns some of these counts into
//! admin errors (every `img` that the structural pass removed is one
//! `image_src_rejected`, every `iframe` whose children it removed one
//! `iframe_content`), the drift canary returns them when an admin write
//! changed, and the importer and `content_normalize` record them (editor.md
//! §7.6, §7.7).
//!
//! ammonia reports nothing about what it removes, and its own allow-list
//! step runs before any callback of ours, so the report cannot be collected
//! while ammonia runs. [`explain`] derives it instead: it parses the input
//! exactly as ammonia does (the same html5ever, a fragment in a `div`
//! context, and a tree that answers the parser's one tree-dependent
//! question as ammonia's does, [`parse_like_ammonia`]) and walks the tree
//! with the same tables, the same namespace check, the same scheme check,
//! the same filter and the same relative-URL rule, in ammonia's order. The
//! sanitiser's proptests compare the report with ammonia's real output: the
//! attribute entries of single elements, and, over nested trees with table
//! cells, the removed and unwrapped elements, the comments and the rejected
//! image sources. An ammonia upgrade that changes the order or the namespace
//! lists fails them.
//!
//! [`explain`] covers the ammonia step only. The structural pass (editor.md
//! §7.5) runs on a tree this crate builds, so it records its own removals
//! while it runs ([`RemovalRule::IframeContentRemoved`],
//! [`RemovalRule::ImageWithoutSrcRemoved`],
//! [`RemovalRule::NestedSubSupUnwrapped`],
//! [`RemovalRule::NestedLinkUnwrapped`],
//! [`RemovalRule::NestedFontSizeRemoved`]), and the fixed-point loop merges
//! the reports of every pass into one. Adding missing locked declarations is
//! not a removal and is not recorded: neither the forced `table` and `tbody`
//! styles that ammonia appends nor the image style that the structural pass
//! gives an `img` without one.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use html5ever::driver::{ParseOpts, parse_fragment};
use html5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, QualName, local_name, ns};
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
    /// A `style` attribute was dropped because the string the grammar wrote
    /// broke the alphabet guard of editor.md §3.1 rule 6. The grammar never
    /// writes such a string, so this is a bug, which the write path logs as
    /// an error.
    StyleOutsideAlphabet,
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
    /// The `font-size` of a `span` inside another `span` with a `font-size`
    /// was removed, so that sizes cannot compound (the structural pass,
    /// editor.md §7.5).
    NestedFontSizeRemoved,
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
            Self::StyleOutsideAlphabet => "style_outside_alphabet",
            Self::ImageSrcRejected => "image_src_rejected",
            Self::EmbedSrcRejected => "embed_src_rejected",
            Self::LinkHrefRejected => "link_href_rejected",
            Self::IframeContentRemoved => "iframe_content_removed",
            Self::ImageWithoutSrcRemoved => "image_without_src_removed",
            Self::NestedSubSupUnwrapped => "nested_sub_sup_unwrapped",
            Self::NestedFontSizeRemoved => "nested_font_size_removed",
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
/// `make_parser`): an HTML fragment in a `div` context, default options,
/// into a tree that treats an HTML-encoded `annotation-xml` as an
/// integration point, as ammonia's own tree does
/// ([`IntegrationPointSink`]).
///
/// - `input`: the HTML to parse.
#[must_use]
pub fn parse_like_ammonia(input: &str) -> Html {
    parse_fragment(
        IntegrationPointSink::new(true),
        ParseOpts::default(),
        QualName::new(None, ns!(html), local_name!("div")),
        Vec::new(),
        false,
    )
    .one(input)
}

/// scraper's tree sink, which can also answer the one question of the tree
/// builder where scraper's tree and ammonia's differ: whether an
/// `annotation-xml` element is an HTML integration point.
///
/// The tree builder asks the sink, because the answer depends on how the
/// element was created: an `annotation-xml` whose `encoding` is `text/html`
/// or `application/xhtml+xml` is one, and html5ever passes that in the
/// creation flags. ammonia's tree keeps the flag and answers with it
/// (ammonia@4.2.4:src/rcdom.rs, `is_mathml_annotation_xml_integration_point`);
/// scraper's always answers no (`scraper@0.29.0:src/html/tree_sink.rs`). The
/// trees then differ: `<math><annotation-xml encoding="text/html"><img>`
/// keeps the `img` inside the `math` in ammonia's tree, which removes it
/// with the `math`, while scraper's tree lets the `img` break out of the
/// `math` and keeps it. A report from scraper's tree counted an
/// `image_src_rejected` there for an image that ammonia never kept.
///
/// - With `like_ammonia` set, the sink answers as ammonia's tree does. Every
///   parse outside the tests uses that, through [`parse_like_ammonia`]: the
///   sanitiser's passes, the removal report, the structural pass, the
///   validator and the search text.
/// - Without it, the sink answers as scraper's own tree does. Only tests
///   use that: the depth scan's oracle and its `tree_depth` check the scan
///   against both trees, so the scan stays correct whichever tree a parse
///   builds, and the test of this sink shows the two trees differ.
///
/// The created integration points are kept in a `BTreeSet`, so a body with
/// many `annotation-xml` elements costs a logarithmic lookup per question,
/// and no hash seed is read (ADR 0008 rule 1).
pub(super) struct IntegrationPointSink {
    inner: HtmlTreeSink,
    /// Answer like ammonia's tree (`true`) or like scraper's (`false`).
    like_ammonia: bool,
    /// The elements created as HTML integration points.
    points: RefCell<BTreeSet<<HtmlTreeSink as TreeSink>::Handle>>,
}

impl IntegrationPointSink {
    /// A sink over a new scraper fragment.
    ///
    /// - `like_ammonia`: answer the integration-point question as ammonia's
    ///   tree does (`true`) or as scraper's does (`false`).
    pub(super) fn new(like_ammonia: bool) -> Self {
        Self {
            inner: HtmlTreeSink::new(Html::new_fragment()),
            like_ammonia,
            points: RefCell::new(BTreeSet::new()),
        }
    }
}

// Every method but `create_element` and
// `is_mathml_annotation_xml_integration_point` forwards to scraper's sink.
impl TreeSink for IntegrationPointSink {
    type Handle = <HtmlTreeSink as TreeSink>::Handle;
    type Output = Html;
    type ElemName<'a>
        = <HtmlTreeSink as TreeSink>::ElemName<'a>
    where
        Self: 'a;

    fn finish(self) -> Html {
        self.inner.finish()
    }

    fn parse_error(&self, message: Cow<'static, str>) {
        self.inner.parse_error(message);
    }

    fn get_document(&self) -> Self::Handle {
        self.inner.get_document()
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        self.inner.elem_name(target)
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        let point = flags.mathml_annotation_xml_integration_point;
        let handle = self.inner.create_element(name, attrs, flags);
        if point {
            self.points.borrow_mut().insert(handle);
        }
        handle
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.inner.create_comment(text)
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.inner.create_pi(target, data)
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        self.inner.append(parent, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        self.inner
            .append_based_on_parent_node(element, prev_element, child);
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.inner
            .append_doctype_to_document(name, public_id, system_id);
    }

    fn mark_script_already_started(&self, node: &Self::Handle) {
        self.inner.mark_script_already_started(node);
    }

    fn pop(&self, node: &Self::Handle) {
        self.inner.pop(node);
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        self.inner.get_template_contents(target)
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        self.inner.same_node(x, y)
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.inner.set_quirks_mode(mode);
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        self.inner.append_before_sibling(sibling, new_node);
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        self.inner.add_attrs_if_missing(target, attrs);
    }

    fn associate_with_form(
        &self,
        target: &Self::Handle,
        form: &Self::Handle,
        nodes: (&Self::Handle, Option<&Self::Handle>),
    ) {
        self.inner.associate_with_form(target, form, nodes);
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.inner.remove_from_parent(target);
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        self.inner.reparent_children(node, new_parent);
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &Self::Handle) -> bool {
        self.like_ammonia && self.points.borrow().contains(handle)
    }

    fn set_current_line(&self, line_number: u64) {
        self.inner.set_current_line(line_number);
    }

    fn allow_declarative_shadow_roots(&self, intended_parent: &Self::Handle) -> bool {
        self.inner.allow_declarative_shadow_roots(intended_parent)
    }

    fn attach_declarative_shadow(
        &self,
        location: &Self::Handle,
        template: &Self::Handle,
        attrs: &[Attribute],
    ) -> bool {
        self.inner
            .attach_declarative_shadow(location, template, attrs)
    }

    fn maybe_clone_an_option_into_selectedcontent(&self, option: &Self::Handle) {
        self.inner
            .maybe_clone_an_option_into_selectedcontent(option);
    }
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
                if removed_with_content(name) {
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

/// Whether ammonia removes an HTML element named `name` together with its
/// content: it is in `clean_content_tags` ([`REMOVED_WITH_CONTENT`]), or on
/// ammonia's SVG and `MathML` name lists ([`AMMONIA_FOREIGN_TAGS`], editor.md
/// §3.5). The removal report and the validator's walk of the input both skip
/// such an element's subtree with this one test.
///
/// - `name`: the local name, as the HTML parser reports it.
#[must_use]
pub(super) fn removed_with_content(name: &str) -> bool {
    REMOVED_WITH_CONTENT.contains(&name) || AMMONIA_FOREIGN_TAGS.contains(&name)
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
            }
            rule if rule.is_url() => match url_outcome(policy, tag, name, value) {
                None => record(rejected_url),
                Some(kept) if kept != value => record(RemovalRule::ValueCanonicalised),
                Some(_) => {}
            },
            _ => match attrs::decide(policy, tag, name, value) {
                Decision::Drop(rule) => record(rule),
                Decision::Keep(kept) if kept != value => record(RemovalRule::ValueCanonicalised),
                Decision::Keep(_) => {}
            },
        }
    }
}

/// What ammonia, configured from `policy`, stores for the URL attribute
/// `name` of the element `tag`: the value it writes, or `None` when one of
/// its three steps removes the attribute. The steps run in ammonia's order:
/// the scheme check of the allow-list, then the attribute filter
/// ([`attrs::decide`], with the URL rules of editor.md §3.6), then the
/// same-origin rule for a relative value ([`url_rules::same_origin_relative`]),
/// which ammonia applies to the filter's output.
///
/// The removal report and [`super::validate::validate_content`] both decide
/// image and embed sources here, so the index the validator gives a removed
/// image cannot disagree with the report's count of removed images.
///
/// - `policy`: supplies the hosts and prefixes of the URL rules.
/// - `tag`: the element, as the HTML parser reports it; its rule must list
///   `name` as a URL attribute (`img[src]`, `iframe[src]`, `a[href]`).
/// - `name`: the attribute name.
/// - `value`: the value after character references are decoded.
#[must_use]
pub(super) fn url_outcome<'a>(
    policy: &SanitizePolicy,
    tag: &str,
    name: &str,
    value: &'a str,
) -> Option<Cow<'a, str>> {
    if !url_rules::scheme_admitted(value) {
        return None;
    }
    match attrs::decide(policy, tag, name, value) {
        Decision::Drop(_) => None,
        Decision::Keep(kept)
            if url_rules::is_relative(&kept)
                && url_rules::same_origin_relative(&kept).is_none() =>
        {
            None
        }
        Decision::Keep(kept) => Some(kept),
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
            (StyleOutsideAlphabet, "style_outside_alphabet"),
            (ImageSrcRejected, "image_src_rejected"),
            (EmbedSrcRejected, "embed_src_rejected"),
            (LinkHrefRejected, "link_href_rejected"),
            (IframeContentRemoved, "iframe_content_removed"),
            (ImageWithoutSrcRemoved, "image_without_src_removed"),
            (NestedSubSupUnwrapped, "nested_sub_sup_unwrapped"),
            (NestedFontSizeRemoved, "nested_font_size_removed"),
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
        // A sized cell gains its `box-sizing`, which is a re-spelling too.
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
                (
                    entry(RemovalRule::ValueCanonicalised, "td", Some("style")),
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

    /// The names of the root's children and grandchildren, as `parent/child`.
    fn shape(html: &Html) -> Vec<String> {
        html.root_element()
            .descendants()
            .filter_map(|node| {
                let element = node.value().as_element()?;
                let parent = node.parent()?.value().as_element()?;
                Some(format!("{}/{}", parent.name(), element.name()))
            })
            .collect()
    }

    #[test]
    fn an_html_annotation_is_an_integration_point_as_in_ammonia() {
        let input = r#"<math><annotation-xml encoding="text/html"><img src="data:x" alt="a"><p>szöveg</p></annotation-xml></math>"#;
        // ammonia's tree keeps the HTML inside the annotation ...
        assert_eq!(
            shape(&parse_like_ammonia(input)),
            [
                "html/math",
                "math/annotation-xml",
                "annotation-xml/img",
                "annotation-xml/p"
            ]
        );
        // ... where scraper's own tree lets it break out of the `math`.
        let plain = parse_fragment(
            IntegrationPointSink::new(false),
            ParseOpts::default(),
            QualName::new(None, ns!(html), local_name!("div")),
            Vec::new(),
            false,
        )
        .one(input);
        assert_eq!(
            shape(&plain),
            ["html/math", "math/annotation-xml", "html/img", "html/p"]
        );
        // So the report names only the `math` that ammonia removed.
        assert_eq!(
            report(input)
                .entries()
                .map(|(r, n)| (r.clone(), n))
                .collect::<Vec<_>>(),
            [(entry(RemovalRule::ElementRemoved, "math", None), 1)]
        );
        // Without an HTML encoding the annotation is no integration point in
        // either tree.
        let xml = r#"<math><annotation-xml encoding="application/mathml+xml"><img src="data:x"></annotation-xml></math>"#;
        assert_eq!(report(xml).count(RemovalRule::ImageSrcRejected), 1);
    }

    #[test]
    fn removed_with_content_covers_both_lists() {
        for name in REMOVED_WITH_CONTENT.iter().chain(AMMONIA_FOREIGN_TAGS) {
            assert!(removed_with_content(name), "{name}");
        }
        for element in ELEMENTS {
            assert!(!removed_with_content(element.name), "{}", element.name);
        }
        assert!(!removed_with_content("div"));
        assert!(!removed_with_content("clipPath"));
    }

    #[test]
    fn url_outcomes_follow_ammonias_three_steps() {
        let policy = SanitizePolicy::content_v1();
        let outcome =
            |tag, value| url_outcome(&policy, tag, if tag == "a" { "href" } else { "src" }, value);
        // The scheme check of the allow-list.
        assert_eq!(outcome("img", "data:image/png;base64,AA"), None);
        assert_eq!(outcome("a", "javascript:alert(1)"), None);
        // The filter, which also rewrites own-host image URLs.
        assert_eq!(outcome("img", "https://evil.example/a.jpg"), None);
        assert_eq!(
            outcome(
                "img",
                "https://verseghy-gimnazium.net/storage/uploads/a.png"
            )
            .as_deref(),
            Some("/storage/uploads/a.png")
        );
        assert_eq!(outcome("iframe", "https://evil.example/embed/x"), None);
        assert_eq!(
            outcome("iframe", "https://padlet.com/embed/k").as_deref(),
            Some("https://padlet.com/embed/k")
        );
        // The same-origin rule for what stays relative.
        assert_eq!(outcome("a", "//evil.example/x"), None);
        assert_eq!(outcome("a", "#top").as_deref(), Some("#top"));
        assert_eq!(
            outcome("img", "/storage/uploads/a.jpg").as_deref(),
            Some("/storage/uploads/a.jpg")
        );
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

        // `url_outcome` itself is checked against ammonia's real output, in
        // the sanitiser's tests (`url_outcome_matches_ammonia`): the report
        // decides URL attributes with it, so comparing the two here would
        // compare the function with itself.

        #[test]
        fn removed_with_content_is_the_union_of_the_lists(name in "[a-z-]{1,14}") {
            prop_assert_eq!(
                removed_with_content(&name),
                REMOVED_WITH_CONTENT.contains(&name.as_str()) || AMMONIA_FOREIGN_TAGS.contains(&name.as_str())
            );
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
