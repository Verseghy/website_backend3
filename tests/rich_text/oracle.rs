//! P0 of editor.md §13.6: an independent oracle that re-checks the
//! sanitiser's output against `policy.json`.
//!
//! The sanitiser and the oracle must not share the rules they apply, or a
//! wrong rule would pass both. So the oracle never calls
//! `backend3::domain::rich_text` to judge an output. It reads the committed
//! `admin/src/rich-text/policy.json`, the file the admin imports, into
//! types of its own (unknown members are refused, also on a rule kind
//! without parameters, so a new rule kind or parameter cannot pass
//! unchecked), parses the output with scraper 0.29 in a `div` context as
//! ammonia and the public sites' `innerHTML` do, and checks every rule
//! again from that data:
//!
//! - every element is a content-v1 tag in the HTML namespace, never a
//!   forbidden one; no comment or other node kind survives;
//! - every attribute is listed for its element and its value follows its
//!   rule (integers in their range and canonical spelling, enumerated and
//!   forced values, the embed markers); every forced attribute is present;
//! - the URL rules of editor.md §3.6, on the output: `img[src]`,
//!   `iframe[src]` and `a[href]`, including the `/storage` link rule;
//! - every `style` is in the alphabet, sorted, written `prop: value` and
//!   joined with `; `, holds only stored properties of its element in their
//!   canonical spelling, carries every always-present declaration, and
//!   carries the cell `box-sizing` exactly when it has a `width` or a
//!   `height`;
//! - the structural rules of editor.md §7.5: no `table` with a `p` ancestor,
//!   no `iframe` with child nodes, no `img` without `src`, no `sub` or `sup`
//!   inside another, no `a` inside another `a` unless a table cell lies
//!   between them, no `span` with a `font-size` inside a `span` with one,
//!   and the product of the `em` factors along every path between 0.5 and 3;
//! - style rules that hold whatever `policy.json` says, written here so that
//!   a policy change cannot loosen them unnoticed: every `img` carries its
//!   four locked declarations, every `span` colour is an opaque `#rrggbb`,
//!   a cell has `box-sizing: content-box` exactly when it has a `width` or a
//!   `height`, and it writes none or all four of its border sides (editor.md
//!   §3.3, §7.3);
//! - every element's attributes in the canonical order of its list. scraper
//!   0.29 sorts attributes unless its `deterministic` feature is on, which
//!   backend3 leaves off (features add up, so turning it on for tests would
//!   change what the sanitiser itself runs on), so this check reads the
//!   start tags with html5ever's tokenizer instead.
//!
//! The URL checks use the `url` crate, the WHATWG parser that browsers
//! follow, as the sanitiser does: the oracle re-implements the rules, not
//! the URL standard.
//!
//! It runs over the sanitiser's output for every regression fixture
//! (`tests/fixtures/regressions/`) and, in `properties.rs`, over the P1
//! generator's outputs. The tests below also feed it hand-written markup
//! that breaks each rule, so a check that stopped working would fail here.

use std::{cell::RefCell, collections::BTreeMap, sync::LazyLock};

use html5ever::{
    QualName,
    driver::{ParseOpts, parse_fragment},
    local_name, ns,
    tendril::{StrTendril, TendrilSink},
    tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    },
};
use percent_encoding::percent_decode_str;
use scraper::{ElementRef, Html, HtmlTreeSink, Node};
use serde::Deserialize;
use url::{ParseError, Url};

use super::{regressions::fixtures, sanitizer};
use crate::support::repository_root;

// ---------------------------------------------------------------------------
// policy.json, as the oracle reads it

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    #[serde(rename = "$comment")]
    _comment: String,
    #[serde(rename = "policy_version")]
    version: String,
    tags: Vec<String>,
    elements: BTreeMap<String, Element>,
    removed_with_content: Vec<String>,
    removed_by_namespace_check: Vec<String>,
    forbidden_tags: Vec<String>,
    css: Css,
    urls: Urls,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Element {
    attributes: Vec<Attribute>,
    style: Option<Vec<Property>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attribute {
    name: String,
    value: ValueRule,
}

// A rule without parameters is an empty struct variant, not a unit one:
// serde ignores extra members of a unit variant in an internally tagged enum,
// even with `deny_unknown_fields`, so a parameter added to one of these kinds
// would otherwise pass unchecked.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ValueRule {
    Style {},
    Text {},
    Marker {},
    Integer {
        min: u32,
        max: u32,
        omitted: Option<u32>,
    },
    IntegerOrFull {
        min: u32,
        max: u32,
        full: String,
    },
    Enumerated {
        values: Vec<String>,
        omitted: Option<String>,
    },
    Forced {
        value: String,
    },
    LinkRel {
        value: String,
    },
    ImageSrc {},
    EmbedSrc {},
    LinkHref {},
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Property {
    #[serde(rename = "property")]
    name: String,
    grammar: Grammar,
    role: Role,
    presence: Presence,
    #[allow(
        dead_code,
        reason = "read so that deny_unknown_fields accepts it; it changes no check"
    )]
    provisional: Option<String>,
}

// Empty struct variants for the parameterless kinds, as for `ValueRule`.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Grammar {
    Keyword {
        values: Vec<String>,
        omitted: Option<String>,
    },
    Px {
        min: f64,
        max: f64,
    },
    AutoOrPx {
        min: f64,
        max: f64,
    },
    Em {
        min: f64,
        max: f64,
    },
    Colour {
        opaque_only: bool,
    },
    BorderSide {},
    BorderWidths {},
    BorderStyles {},
    BorderColours {},
    Locked {
        value: String,
    },
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Role {
    Stored,
    InputOnly,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Presence {
    Optional,
    Always,
    WhenSized,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Css {
    alphabet: String,
    border: Border,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Border {
    styles: Vec<String>,
    #[allow(dead_code, reason = "an input spelling: the output always writes px")]
    width_keywords: BTreeMap<String, f64>,
    /// What `border` and `border-<side>` give a part they leave out.
    initial: SideParts,
    /// What a part that no declaration set keeps.
    unset: SideParts,
    max_width: f64,
}

/// The three parts of a border side.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SideParts {
    width: f64,
    style: String,
    colour: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Urls {
    schemes: Vec<String>,
    own_hosts: Vec<String>,
    storage_path_prefix: String,
    image: Image,
    link: Link,
    embed: Embed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    path_prefix: String,
    extensions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    forbidden_storage_extensions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Embed {
    scheme: String,
    id_alphabet: String,
    providers: Vec<Provider>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Provider {
    hosts: Vec<String>,
    path_prefix: String,
    query: Vec<QueryParameter>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryParameter {
    name: String,
    value: QueryValue,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum QueryValue {
    Digits,
    Id,
}

/// The committed `policy.json`, read once.
static POLICY: LazyLock<Policy> = LazyLock::new(|| {
    let path = repository_root().join("admin/src/rich-text/policy.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "{}: {error}. The oracle reads every member of policy.json: a new member or rule \
             kind needs a check here too",
            path.display()
        )
    })
});

/// Where a relative URL is resolved: a reserved host that stands for the
/// page's own origin (RFC 2606).
static SAME_ORIGIN: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://oracle.invalid/").expect("a valid base"));

// ---------------------------------------------------------------------------
// The checks

/// Every rule that `html` breaks, one line each; empty when it is
/// content-v1.
///
/// - `html`: a sanitiser output.
fn violations(html: &str) -> Vec<String> {
    let mut found = tree_violations(&parse(html));
    check_attribute_order(&POLICY, html, &mut found);
    found
}

/// Parses `html` as ammonia does and as the public sites' `innerHTML` on an
/// article element does: a fragment in a `div` context. The golden-file
/// inventory of `golden.rs` reads its trees with it too.
///
/// - `html`: the fragment.
pub(super) fn parse(html: &str) -> Html {
    parse_fragment(
        HtmlTreeSink::new(Html::new_fragment()),
        ParseOpts::default(),
        QualName::new(None, ns!(html), local_name!("div")),
        Vec::new(),
        false,
    )
    .one(html)
}

/// Every rule that the parsed `tree` breaks, apart from the attribute
/// order, which the tree does not keep.
fn tree_violations(tree: &Html) -> Vec<String> {
    let policy = &*POLICY;
    let mut found = Vec::new();
    for node in tree.root_element().descendants().skip(1) {
        match node.value() {
            Node::Text(_) => {}
            Node::Element(_) => {
                let element = ElementRef::wrap(node).expect("an element node");
                check_element(policy, element, &mut found);
            }
            other => found.push(format!("a node that content-v1 does not store: {other:?}")),
        }
    }
    found
}

fn check_element(policy: &Policy, element: ElementRef<'_>, found: &mut Vec<String>) {
    let name = element.value().name();
    if element.value().name.ns != ns!(html) {
        found.push(format!("<{name}> is not in the HTML namespace"));
        return;
    }
    if policy
        .forbidden_tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(name))
    {
        found.push(format!("<{name}> is a forbidden tag"));
    }
    let Some(rules) = policy
        .elements
        .get(name)
        .filter(|_| policy.tags.iter().any(|t| t == name))
    else {
        found.push(format!("<{name}> is not a content-v1 element"));
        return;
    };

    // The field, not `attrs()`, which drops the namespace.
    for (attribute, value) in &element.value().attrs {
        let attribute_name = &*attribute.local;
        let value = &**value;
        if attribute.ns != ns!() {
            found.push(format!(
                "<{name}> has the namespaced attribute {attribute:?}"
            ));
            continue;
        }
        match rules
            .attributes
            .iter()
            .find(|rule| rule.name == attribute_name)
        {
            None => found.push(format!(
                "<{name}> has the attribute `{attribute_name}`, which it may not carry"
            )),
            Some(rule) => {
                if let Err(problem) = check_value(policy, name, rules, &rule.value, value) {
                    found.push(format!("<{name} {attribute_name}=\"{value}\">: {problem}"));
                }
            }
        }
    }
    for rule in &rules.attributes {
        let missing = element.value().attr(&rule.name).is_none();
        match &rule.value {
            ValueRule::Forced { .. } | ValueRule::LinkRel { .. } if missing => {
                found.push(format!(
                    "<{name}> lacks its forced attribute `{}`",
                    rule.name
                ));
            }
            ValueRule::Style {} if missing => {
                if let Some(always) = rules
                    .style
                    .iter()
                    .flatten()
                    .find(|property| property.presence == Presence::Always)
                {
                    found.push(format!(
                        "<{name}> has no style, so it lacks the always-present `{}`",
                        always.name
                    ));
                }
            }
            _ => {}
        }
    }
    check_structure(element, found);
}

/// The four locked declarations every `img` carries (editor.md §3.3, §7.5).
/// Written here rather than read from `policy.json`, like the other
/// structural rules: a policy change that loosened them would loosen the
/// oracle too.
const IMG_LOCKED: [(&str, &str); 4] = [
    ("display", "block"),
    ("height", "auto"),
    ("margin", "0 auto"),
    ("max-width", "100%"),
];

/// The four cell sides, all of which a cell writes once it has any (editor.md
/// §7.3).
const CELL_SIDES: [&str; 4] = ["border-top", "border-right", "border-bottom", "border-left"];

/// The declarations of an element's `style`, none when it has no style.
fn style_of<'a>(element: &ElementRef<'a>) -> Vec<(&'a str, &'a str)> {
    element
        .value()
        .attr("style")
        .map_or_else(Vec::new, declarations)
}

/// The `font-size` value of a `span`, if its style has one.
fn font_size<'a>(span: &ElementRef<'a>) -> Option<&'a str> {
    style_of(span)
        .into_iter()
        .find(|(property, _)| *property == "font-size")
        .map(|(_, value)| value)
}

/// The structural rules of editor.md §7.5, and the style rules that hold
/// whatever `policy.json` says: they are written into the oracle, so a
/// policy change cannot loosen them unnoticed.
fn check_structure(element: ElementRef<'_>, found: &mut Vec<String>) {
    let name = element.value().name();
    let ancestors: Vec<ElementRef<'_>> = element.ancestors().filter_map(ElementRef::wrap).collect();
    let ancestor_named =
        |names: &[&str]| ancestors.iter().any(|a| names.contains(&a.value().name()));
    let style = style_of(&element);
    let declares = |property: &str| style.iter().any(|(name, _)| *name == property);
    match name {
        "table" if ancestor_named(&["p"]) => found.push("a <table> has a <p> ancestor".to_owned()),
        "iframe" if element.children().next().is_some() => {
            found.push("an <iframe> has child nodes".to_owned());
        }
        "img" => {
            if element.value().attr("src").is_none() {
                found.push("an <img> has no src".to_owned());
            }
            for (property, value) in IMG_LOCKED {
                if !style.contains(&(property, value)) {
                    found.push(format!("an <img> lacks its locked `{property}: {value}`"));
                }
            }
        }
        "sub" | "sup" if ancestor_named(&["sub", "sup"]) => {
            found.push(format!("a <{name}> is inside another <sub> or <sup>"));
        }
        "a" => {
            // The nearest of an enclosing link and an enclosing cell decides:
            // a cell restarts the parser's list of formatting elements.
            let nearest = ancestors
                .iter()
                .map(|a| a.value().name())
                .find(|ancestor| matches!(*ancestor, "a" | "td" | "th"));
            if nearest == Some("a") {
                found.push("an <a> is inside another <a> with no table cell between".to_owned());
            }
        }
        "span" => check_span(&element, &ancestors, &style, found),
        "td" | "th" => {
            let sides = CELL_SIDES.iter().filter(|side| declares(side)).count();
            if sides != 0 && sides != CELL_SIDES.len() {
                found.push(format!(
                    "a <{name}> writes {sides} of its four border sides"
                ));
            }
            let sized = declares("width") || declares("height");
            match style.iter().find(|(property, _)| *property == "box-sizing") {
                Some((_, value)) if *value != "content-box" => {
                    found.push(format!("a <{name}> has `box-sizing: {value}`"));
                }
                box_sizing if box_sizing.is_some() != sized => found.push(format!(
                    "a <{name}> has `box-sizing` without a size, or a size without it"
                )),
                _ => {}
            }
        }
        _ => {}
    }
}

/// The `span` rules: an opaque `#rrggbb` colour only, no `font-size` inside
/// a `span` that has one (the sanitiser drops it, so sizes cannot
/// compound), and the product of the `em` factors along the path between
/// 0.5 and 3. The nested-size rule already implies the product rule; the
/// product is checked as well, so it still holds if the nested-size rule
/// is ever relaxed.
fn check_span(
    element: &ElementRef<'_>,
    ancestors: &[ElementRef<'_>],
    style: &[(&str, &str)],
    found: &mut Vec<String>,
) {
    if let Some((_, colour)) = style.iter().find(|(property, _)| *property == "color")
        && !canonical_colour(colour, true)
    {
        found.push(format!("a <span> colour is not opaque: `{colour}`"));
    }
    let Some(own) = font_size(element) else {
        return;
    };
    let sized_ancestors: Vec<&str> = ancestors
        .iter()
        .filter(|a| a.value().name() == "span")
        .filter_map(font_size)
        .collect();
    if !sized_ancestors.is_empty() {
        found.push(format!(
            "a <span> with `font-size: {own}` is inside a <span> with a font-size"
        ));
    }
    let factor = |value: &str| canonical_number(value.strip_suffix("em")?);
    let product: f64 = sized_ancestors
        .iter()
        .filter_map(|value| factor(value))
        .fold(factor(own).unwrap_or(1.0), |product, factor| {
            product * factor
        });
    if !(0.5..=3.0).contains(&product) {
        found.push(format!("the em factors along a path multiply to {product}"));
    }
}

/// Checks one attribute value against its rule.
fn check_value(
    policy: &Policy,
    element: &str,
    rules: &Element,
    rule: &ValueRule,
    value: &str,
) -> Result<(), String> {
    match rule {
        // No `?` here: it would return before the element is named below.
        ValueRule::Style {} => rules
            .style
            .as_deref()
            .ok_or_else(|| "a style rule without a grammar".to_owned())
            .and_then(|properties| check_style(policy, properties, value)),
        ValueRule::Text {} => Ok(()),
        ValueRule::Marker {} => expect(value.is_empty(), "a marker is always empty"),
        ValueRule::Integer { min, max, omitted } => check_integer(value, *min, *max, *omitted),
        ValueRule::IntegerOrFull { full, .. } if value == full => Ok(()),
        ValueRule::IntegerOrFull { min, max, .. } => check_integer(value, *min, *max, None),
        ValueRule::Enumerated { values, omitted } => expect(
            values.iter().any(|v| v == value) && omitted.as_deref() != Some(value),
            "not one of the stored values",
        ),
        ValueRule::Forced { value: forced } | ValueRule::LinkRel { value: forced } => {
            expect(value == forced, &format!("must be exactly `{forced}`"))
        }
        ValueRule::ImageSrc {} => check_image_src(&policy.urls, value),
        ValueRule::EmbedSrc {} => check_embed_src(&policy.urls.embed, value),
        ValueRule::LinkHref {} => check_link_href(&policy.urls, value),
    }
    .map_err(|problem| format!("{problem} (on <{element}>)"))
}

fn expect(holds: bool, problem: &str) -> Result<(), String> {
    if holds {
        Ok(())
    } else {
        Err(problem.to_owned())
    }
}

/// A decimal integer as the sanitiser writes it: digits only, no leading
/// zero, inside the range, and never the omitted default.
fn check_integer(value: &str, min: u32, max: u32, omitted: Option<u32>) -> Result<(), String> {
    let canonical = !value.is_empty()
        && value.len() <= 10
        && value.bytes().all(|b| b.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'));
    let number: u64 = value
        .parse()
        .ok()
        .filter(|_| canonical)
        .ok_or("not a canonical integer")?;
    expect(
        (u64::from(min)..=u64::from(max)).contains(&number),
        &format!("outside {min}..={max}"),
    )?;
    expect(
        omitted.map(u64::from) != Some(number),
        "the default, which is omitted",
    )
}

/// `prop: value` pairs of a style string, split at `; ` and `: `.
fn declarations(style: &str) -> Vec<(&str, &str)> {
    style
        .split("; ")
        .map(|declaration| declaration.split_once(": ").unwrap_or((declaration, "")))
        .collect()
}

fn check_style(policy: &Policy, properties: &[Property], style: &str) -> Result<(), String> {
    expect(!style.is_empty(), "an empty style is never stored")?;
    if let Some(c) = style.chars().find(|&c| !policy.css.alphabet.contains(c)) {
        return Err(format!("`{c}` is outside the style alphabet"));
    }
    let pairs = declarations(style);
    let names: Vec<&str> = pairs.iter().map(|(name, _)| *name).collect();
    expect(
        names.windows(2).all(|pair| pair[0] < pair[1]),
        "the declarations are not sorted, or one repeats",
    )?;
    for (name, value) in &pairs {
        let rule = properties
            .iter()
            .find(|property| property.name == *name)
            .ok_or_else(|| format!("`{name}` is not allowed here"))?;
        expect(
            rule.role == Role::Stored,
            &format!("`{name}` is input only and never stored"),
        )?;
        expect(
            value_matches(&policy.css.border, &rule.grammar, value),
            &format!("`{name}: {value}` is not a canonical value of its grammar"),
        )?;
    }
    let sized = names.contains(&"width") || names.contains(&"height");
    for property in properties {
        let present = names.contains(&property.name.as_str());
        match property.presence {
            Presence::Optional => {}
            Presence::Always => expect(
                present,
                &format!("lacks the always-present `{}`", property.name),
            )?,
            Presence::WhenSized => expect(
                present == sized,
                &format!(
                    "`{}` must be present exactly when width or height is",
                    property.name
                ),
            )?,
        }
    }
    Ok(())
}

/// A number as the canonical writer spells it: no sign, no leading zero
/// before another digit, at most two decimals, no trailing zero or dot.
fn canonical_number(text: &str) -> Option<f64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let whole_ok = !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && (whole == "0" || !whole.starts_with('0'));
    let fraction_ok = (text.contains('.')
        && !fraction.is_empty()
        && fraction.len() <= 2
        && !fraction.ends_with('0')
        && fraction.bytes().all(|b| b.is_ascii_digit()))
        || !text.contains('.');
    (whole_ok && fraction_ok)
        .then(|| text.parse().ok())
        .flatten()
}

fn in_range(number: Option<f64>, min: f64, max: f64) -> bool {
    number.is_some_and(|n| (min..=max).contains(&n))
}

/// A colour as the canonical writer spells it: lower-case `#rrggbb`, or
/// `#rrggbbaa` with an alpha below 1, or `transparent` or `currentcolor`.
fn canonical_colour(value: &str, opaque_only: bool) -> bool {
    let hex = |digits: &str| {
        digits
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    match value.strip_prefix('#') {
        Some(digits) if digits.len() == 6 => hex(digits),
        Some(digits) if digits.len() == 8 => !opaque_only && hex(digits) && !digits.ends_with("ff"),
        Some(_) => false,
        None => !opaque_only && matches!(value, "transparent" | "currentcolor"),
    }
}

fn value_matches(border: &Border, grammar: &Grammar, value: &str) -> bool {
    let px = |min: f64, max: f64| {
        in_range(
            value.strip_suffix("px").and_then(canonical_number),
            min,
            max,
        )
    };
    match grammar {
        Grammar::Keyword { values, omitted } => {
            values.iter().any(|v| v == value) && omitted.as_deref() != Some(value)
        }
        Grammar::Px { min, max } => px(*min, *max),
        Grammar::AutoOrPx { min, max } => value == "auto" || px(*min, *max),
        Grammar::Em { min, max } => in_range(
            value.strip_suffix("em").and_then(canonical_number),
            *min,
            *max,
        ),
        Grammar::Colour { opaque_only } => canonical_colour(value, *opaque_only),
        Grammar::BorderSide {} => match value.split(' ').collect::<Vec<_>>()[..] {
            ["none" | "hidden"] => true,
            [width, style, colour] => {
                in_range(
                    width.strip_suffix("px").and_then(canonical_number),
                    0.0,
                    border.max_width,
                ) && border.styles.iter().any(|s| s == style)
                    && !matches!(style, "none" | "hidden")
                    && canonical_colour(colour, false)
            }
            _ => false,
        },
        // Input-only shorthands: the canonical writer expands them, so no
        // stored value can follow them.
        Grammar::BorderWidths {} | Grammar::BorderStyles {} | Grammar::BorderColours {} => false,
        Grammar::Locked { value: locked } => value == locked,
    }
}

/// `img[src]` (editor.md §3.6): a root-relative path under the image prefix,
/// as the URL parser serialises it, ending in an image extension, with no
/// query or fragment and no segment that decodes to `.`, `..`, a separator,
/// a NUL or bytes that are not UTF-8.
fn check_image_src(urls: &Urls, value: &str) -> Result<(), String> {
    expect(
        value.starts_with('/') && !value.starts_with("//"),
        "not a root-relative path",
    )?;
    let url = SAME_ORIGIN
        .join(value)
        .map_err(|error| format!("does not parse: {error}"))?;
    expect(
        url.origin() == SAME_ORIGIN.origin()
            && url.path() == value
            && url.query().is_none()
            && url.fragment().is_none(),
        "not a path as the URL parser writes it, or it has a query or a fragment",
    )?;
    expect(
        value.starts_with(&urls.image.path_prefix),
        "outside the image prefix",
    )?;
    let file = value
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    expect(
        urls.image
            .extensions
            .iter()
            .any(|extension| file.ends_with(&format!(".{extension}"))),
        "not an image extension",
    )?;
    for segment in value.split('/') {
        let decoded = percent_decode_str(segment)
            .decode_utf8()
            .map_err(|_| "a segment is not UTF-8")?;
        expect(
            !matches!(&*decoded, "." | "..") && !decoded.contains(['/', '\\', '\0']),
            "a segment decodes to a dot segment, a separator or a NUL",
        )?;
    }
    Ok(())
}

/// `iframe[src]` (editor.md §3.6): an embed URL of a provider, as the URL
/// parser writes it.
fn check_embed_src(embed: &Embed, value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|error| format!("does not parse: {error}"))?;
    expect(
        url.as_str() == value,
        "not written as the URL parser writes it",
    )?;
    expect(
        url.scheme() == embed.scheme
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.fragment().is_none(),
        "not a plain https URL",
    )?;
    let host = url.host_str().unwrap_or_default();
    let provider = embed
        .providers
        .iter()
        .find(|provider| provider.hosts.iter().any(|h| h == host))
        .ok_or_else(|| format!("`{host}` is not an embed host"))?;
    let id = url
        .path()
        .strip_prefix(&provider.path_prefix)
        .ok_or("outside the embed path")?;
    let is_id =
        |text: &str| !text.is_empty() && text.chars().all(|c| embed.id_alphabet.contains(c));
    expect(is_id(id), "not an embed id")?;
    let Some(query) = url.query() else {
        return Ok(());
    };
    let mut expected = provider.query.iter();
    for parameter in query.split('&') {
        let (name, argument) = parameter
            .split_once('=')
            .ok_or("a query parameter without a value")?;
        let rule = expected
            .by_ref()
            .find(|rule| rule.name == name)
            .ok_or_else(|| {
                format!("the query parameter `{name}` is not allowed here, or out of order")
            })?;
        let valid = match rule.value {
            QueryValue::Digits => {
                !argument.is_empty() && argument.bytes().all(|b| b.is_ascii_digit())
            }
            QueryValue::Id => is_id(argument),
        };
        expect(valid, &format!("`{name}={argument}` is not a valid value"))?;
    }
    Ok(())
}

/// `a[href]` (editor.md §3.6): an allowed scheme, or a relative URL on the
/// page's own origin, and never a link to a forbidden file under `/storage/`
/// on an own host.
fn check_link_href(urls: &Urls, value: &str) -> Result<(), String> {
    let target = match Url::parse(value) {
        Ok(url) => {
            expect(
                urls.schemes.iter().any(|s| s == url.scheme()),
                "a scheme that is not allowed",
            )?;
            let own = matches!(url.scheme(), "http" | "https")
                && url
                    .domain()
                    .map(|domain| domain.strip_suffix('.').unwrap_or(domain))
                    .is_some_and(|domain| urls.own_hosts.iter().any(|h| h == domain));
            own.then_some(url)
        }
        Err(ParseError::RelativeUrlWithoutBase) => {
            let url = SAME_ORIGIN
                .join(value)
                .map_err(|error| format!("does not resolve: {error}"))?;
            expect(
                url.origin() == SAME_ORIGIN.origin(),
                "resolves to another origin",
            )?;
            // A scheme-relative URL that names the page's own host stays on
            // the origin, but §3.6 refuses every one: after the parser's own
            // clean-up, two leading slashes or backslashes make it one.
            let cleaned: String = value
                .trim_matches(|c: char| c <= ' ')
                .chars()
                .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
                .take(2)
                .collect();
            let scheme_relative =
                cleaned.chars().count() == 2 && cleaned.chars().all(|c| matches!(c, '/' | '\\'));
            expect(!scheme_relative, "a scheme-relative URL")?;
            Some(url)
        }
        Err(error) => return Err(format!("does not parse: {error}")),
    };
    if let Some(url) = target {
        let decoded = percent_decode_str(url.path())
            .decode_utf8_lossy()
            .to_lowercase();
        let served = served_path(&decoded);
        let file = served.rsplit('/').next().unwrap_or_default();
        let forbidden = served.starts_with(&urls.storage_path_prefix)
            && urls
                .link
                .forbidden_storage_extensions
                .iter()
                .any(|extension| file.ends_with(&format!(".{extension}")));
        expect(
            !forbidden,
            "a link to a file under /storage/ that a browser renders as a document",
        )?;
    }
    Ok(())
}

/// The path a static server serves for a decoded path: dot segments
/// resolved and empty segments merged. A path that ends in `/`, `/.` or
/// `/..` names a directory rather than a file, so it keeps (or gains) a
/// trailing slash, and no file extension can match it.
fn served_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    let mut served = format!("/{}", segments.join("/"));
    let directory = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..");
    if directory && !served.ends_with('/') {
        served.push('/');
    }
    served
}

/// Reads every start tag with html5ever's tokenizer, which keeps the
/// attributes in their written order, and checks that order against the
/// element's list. Unknown attributes are reported by the tree checks.
fn check_attribute_order(policy: &Policy, html: &str, found: &mut Vec<String>) {
    struct StartTags(RefCell<Vec<(String, Vec<String>)>>);
    impl TokenSink for StartTags {
        type Handle = ();
        fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
            if let Token::TagToken(tag) = token
                && tag.kind == TagKind::StartTag
            {
                let names = tag.attrs.iter().map(|a| a.name.local.to_string()).collect();
                self.0.borrow_mut().push((tag.name.to_string(), names));
            }
            TokenSinkResult::Continue
        }
    }
    let tokenizer = Tokenizer::new(StartTags(RefCell::default()), TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(html));
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    for (name, attributes) in tokenizer.sink.0.into_inner() {
        let Some(rules) = policy.elements.get(&name) else {
            continue;
        };
        let positions: Vec<usize> = attributes
            .iter()
            .filter_map(|attribute| {
                rules
                    .attributes
                    .iter()
                    .position(|rule| rule.name == *attribute)
            })
            .collect();
        if positions.windows(2).any(|pair| pair[0] >= pair[1]) {
            found.push(format!(
                "<{name}> writes its attributes as {attributes:?}, not in canonical order"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// The tests

/// The oracle's verdict on one sanitiser output, for the property tests.
///
/// - `output`: what the sanitiser wrote.
pub(super) fn assert_content_v1(output: &str) -> Result<(), String> {
    let found = violations(output);
    if found.is_empty() {
        Ok(())
    } else {
        Err(format!("{found:#?}\nin: {output}"))
    }
}

#[test]
fn every_regression_output_passes_the_oracle() {
    let fixtures = fixtures();
    let mut failures = Vec::new();
    for (name, (input, _)) in &fixtures {
        let output = sanitizer()
            .sanitize(input)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let found = violations(output.html.as_str());
        if !found.is_empty() {
            failures.push(format!("{name}: {found:#?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "outputs that break content-v1:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_oracle_reads_the_whole_policy() {
    let policy = &*POLICY;
    assert_eq!(policy.version, "content-v1");
    assert_eq!(policy.tags.len(), policy.elements.len());
    assert!(
        policy
            .tags
            .iter()
            .all(|tag| policy.elements.contains_key(tag))
    );
    // Every forced value and every always-present lock is itself canonical,
    // or every table would break the oracle's own rules.
    for (name, element) in &policy.elements {
        for attribute in &element.attributes {
            if let ValueRule::Forced { value } = &attribute.value
                && attribute.name == "style"
            {
                let properties = element
                    .style
                    .as_deref()
                    .expect("a forced style has a grammar");
                assert_eq!(check_style(policy, properties, value), Ok(()), "{name}");
            }
        }
    }
    assert!(
        policy
            .removed_with_content
            .iter()
            .any(|tag| tag == "script")
    );
    // No element that is removed with its content is a content-v1 tag.
    for removed in policy
        .removed_with_content
        .iter()
        .chain(&policy.removed_by_namespace_check)
    {
        assert!(!policy.tags.contains(removed), "{removed}");
    }
    assert!(
        policy
            .removed_by_namespace_check
            .iter()
            .any(|tag| tag == "g")
    );
    // A side that a shorthand or the page reset leaves without a drawing
    // style is written `none`; with one, its parts make a stored side that
    // the cell grammar accepts (editor.md §7.3).
    let border = &policy.css.border;
    for (which, side) in [("initial", &border.initial), ("unset", &border.unset)] {
        assert_eq!(side.style, "none", "{which}");
        let drawn = format!("{}px solid {}", side.width, side.colour);
        assert!(
            value_matches(border, &Grammar::BorderSide {}, &drawn),
            "{which}: {drawn}"
        );
    }
    assert!(border.unset.width < border.initial.width);
}

/// An element whose `style` attribute has no grammar is a broken policy,
/// not broken markup, so it is reported for any style value rather than
/// passed.
#[test]
fn a_style_without_a_grammar_is_reported() {
    let element = Element {
        attributes: Vec::new(),
        style: None,
    };
    assert_eq!(
        check_value(
            &POLICY,
            "p",
            &element,
            &ValueRule::Style {},
            "text-align: center"
        ),
        Err("a style rule without a grammar (on <p>)".to_owned())
    );
}

/// The oracle's types refuse a member they do not know anywhere in the file,
/// also on a rule kind without parameters, so a rule that gains a parameter
/// cannot pass unchecked.
#[test]
fn the_oracle_refuses_a_member_it_does_not_know() {
    let path = repository_root().join("admin/src/rich-text/policy.json");
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("policy.json")).expect("JSON");
    assert!(serde_json::from_value::<Policy>(committed.clone()).is_ok());
    let rule_index = |element: &str, attribute: &str| {
        committed["elements"][element]["attributes"]
            .as_array()
            .expect("attributes")
            .iter()
            .position(|rule| rule["name"] == attribute)
            .expect("the attribute")
    };
    let src = rule_index("img", "src");
    let cell_side = committed["elements"]["td"]["style"]
        .as_array()
        .expect("a cell grammar")
        .iter()
        .position(|rule| rule["property"] == "border-top")
        .expect("border-top");
    let pointers = [
        String::new(),
        format!("/elements/img/attributes/{src}/value"),
        format!("/elements/p/attributes/{}/value", rule_index("p", "style")),
        format!("/elements/td/style/{cell_side}/grammar"),
        "/css/border/unset".to_owned(),
        "/urls/embed".to_owned(),
    ];
    for pointer in pointers {
        let mut changed = committed.clone();
        changed
            .pointer_mut(&pointer)
            .and_then(serde_json::Value::as_object_mut)
            .unwrap_or_else(|| panic!("{pointer}: an object"))
            .insert("max".to_owned(), 5.into());
        let error = serde_json::from_value::<Policy>(changed)
            .expect_err(&format!("{pointer}: an extra member is refused"));
        assert!(
            error.to_string().contains("unknown field `max`"),
            "{pointer}: {error}"
        );
    }
}

/// Pieces of valid markup that the broken cases below are built from.
const IMG: &str = r#"src="/storage/uploads/a.jpg" alt="a""#;
const IMG_STYLE: &str = r#"style="display: block; height: auto; margin: 0 auto; max-width: 100%""#;
const TABLE: &str = r#"<table style="border-collapse: collapse; max-width: fit-content"><tbody style="display: table-row-group">"#;
const REL: &str = r#"target="_blank" rel="noopener noreferrer""#;
const IFRAME_ATTRS: &str = r#"allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen="" referrerpolicy="strict-origin-when-cross-origin""#;

/// Markup that breaks each rule once, written by hand, with the words of
/// the violation the oracle must report for it.
fn broken_markup() -> Vec<(String, &'static str)> {
    let (img, img_style, table, rel, iframe_attrs) = (IMG, IMG_STYLE, TABLE, REL, IFRAME_ATTRS);
    vec![
        ("<div>x</div>".into(), "<div> is not a content-v1 element"),
        ("<p>a<!-- c --></p>".into(), "a node that content-v1 does not store"),
        ("<p class=\"x\">a</p>".into(), "attribute `class`"),
        ("<p data-youtube-video=\"1\">a</p>".into(), "a marker is always empty"),
        (format!("<p><img {img} width=\"0\" {img_style}></p>"), "outside 1..=5000"),
        (format!("<p><img {img} width=\"035\" {img_style}></p>"), "not a canonical integer"),
        ("<ol start=\"1\"><li>a</li></ol>".into(), "the default, which is omitted"),
        ("<ol type=\"1\"><li>a</li></ol>".into(), "not one of the stored values"),
        ("<a href=\"/x\" target=\"_self\" rel=\"noopener noreferrer\">a</a>".into(), "must be exactly `_blank`"),
        ("<a href=\"/x\" target=\"_blank\">a</a>".into(), "lacks its forced attribute `rel`"),
        ("<a href=\"/x\" target=\"_blank\" rel=\"noopener\">a</a>".into(), "must be exactly `noopener noreferrer`"),
        (format!("<a href=\"javascript:alert(1)\" {rel}>a</a>"), "a scheme that is not allowed"),
        (format!("<a href=\"//evil.example/\" {rel}>a</a>"), "resolves to another origin"),
        (format!("<a href=\"/\\evil.example/\" {rel}>a</a>"), "resolves to another origin"),
        (format!("<a href=\"//oracle.invalid/x\" {rel}>a</a>"), "a scheme-relative URL"),
        (format!("<a href=\"/\\oracle.invalid/x\" {rel}>a</a>"), "a scheme-relative URL"),
        (format!("<a href=\"//exa mple/\" {rel}>a</a>"), "does not resolve"),
        (format!("<a href=\"https://exa mple/\" {rel}>a</a>"), "does not parse"),
        (format!("<a href=\"/%73torage/x.SVG\" {rel}>a</a>"), "under /storage/"),
        (format!("<a href=\"https://verseghy-gimnazium.net/storage/a/../b.html\" {rel}>a</a>"), "under /storage/"),
        (format!("<p><img src=\"/storage/other/a.jpg\" alt=\"a\" {img_style}></p>"), "outside the image prefix"),
        (format!("<p><img src=\"/storage/uploads/a.svg\" alt=\"a\" {img_style}></p>"), "not an image extension"),
        (format!("<p><img src=\"https://x.example/storage/uploads/a.jpg\" alt=\"a\" {img_style}></p>"), "not a root-relative path"),
        (format!("<p><img src=\"/storage/uploads/a%2F..%2Fb.jpg\" alt=\"a\" {img_style}></p>"), "a separator"),
        (format!("<p><img src=\"/storage/uploads/a.jpg?x=1\" alt=\"a\" {img_style}></p>"), "query or a fragment"),
        (format!("<p><img src=\"/\\exa mple/a.jpg\" alt=\"a\" {img_style}></p>"), "does not parse"),
        (format!("<p><img src=\"/storage/uploads/%FF.jpg\" alt=\"a\" {img_style}></p>"), "a segment is not UTF-8"),
        (format!("<p><img {img} style=\"display: block; height: auto; margin: 0 auto; max-width: 50%\"></p>"), "lacks its locked `max-width: 100%`"),
        (format!("<p><img alt=\"a\" {img_style}></p>"), "an <img> has no src"),
        (format!("<p><img {img}></p>"), "lacks the always-present `display`"),
        (format!("<p><img {img} style=\"display: block; height: auto; margin: 0 auto\"></p>"), "lacks the always-present `max-width`"),
        (format!("<p><img alt=\"a\" src=\"/storage/uploads/a.jpg\" {img_style}></p>"), "not in canonical order"),
        (format!("<p><iframe src=\"https://evil.example/embed/x\" {iframe_attrs}></iframe></p>"), "not an embed host"),
        (format!("<p><iframe src=\"https://padlet.com/embed/x?start=1\" {iframe_attrs}></iframe></p>"), "not allowed here, or out of order"),
        (format!("<p><iframe src=\"https://www.youtube.com/embed/x?list=a&amp;start=1\" {iframe_attrs}></iframe></p>"), "out of order"),
        (format!("<p><iframe src=\"https://www.youtube.com/embed/a.b\" {iframe_attrs}></iframe></p>"), "not an embed id"),
        (format!("<p><iframe src=\"http://www.youtube.com/embed/x\" {iframe_attrs}></iframe></p>"), "not a plain https URL"),
        (format!("<p><iframe src=\"https://u@www.youtube.com/embed/x\" {iframe_attrs}></iframe></p>"), "not a plain https URL"),
        (format!("<p><iframe src=\"https://www.youtube.com:8443/embed/x\" {iframe_attrs}></iframe></p>"), "not a plain https URL"),
        (format!("<p><iframe src=\"https://www.youtube.com/embed/x#t\" {iframe_attrs}></iframe></p>"), "not a plain https URL"),
        (format!("<p><iframe src=\"https://www.youtube.com/watch/x\" {iframe_attrs}></iframe></p>"), "outside the embed path"),
        (format!("<p><iframe src=\"https://www.youtube.com/embed/x?start\" {iframe_attrs}></iframe></p>"), "a query parameter without a value"),
        (format!("<p><iframe src=\"https://www.youtube.com/embed/x?start=abc\" {iframe_attrs}></iframe></p>"), "`start=abc` is not a valid value"),
        (format!("<p><iframe src=\"https://www.youtube.com/embed/x?list=a.b\" {iframe_attrs}></iframe></p>"), "`list=a.b` is not a valid value"),
        (format!("<p><iframe src=\"https://exa mple/embed/x\" {iframe_attrs}></iframe></p>"), "does not parse"),
        (format!("<p><iframe src=\"https://WWW.youtube.com/embed/x\" {iframe_attrs}></iframe></p>"), "not written as the URL parser writes it"),
        ("<p><iframe src=\"https://padlet.com/embed/x\" allow=\"autoplay\" allowfullscreen=\"\" referrerpolicy=\"strict-origin-when-cross-origin\"></iframe></p>".into(), "must be exactly"),
        ("<p><span style=\"color: red\">a</span></p>".into(), "not a canonical value"),
        ("<p><span style=\"color: #c0000080\">a</span></p>".into(), "not a canonical value"),
        ("<p><span style=\"font-size: 2em; color: #c00000\">a</span></p>".into(), "not sorted"),
        ("<p><span style=\"font-size: 1.50em\">a</span></p>".into(), "not a canonical value"),
        ("<p style=\"text-align: Center\">a</p>".into(), "`C` is outside the style alphabet"),
        ("<p style=\"text-align: center !important\">a</p>".into(), "`!` is outside the style alphabet"),
        ("<p style=\"\">a</p>".into(), "an empty style is never stored"),
        ("<p style=\"color: #000000\">a</p>".into(), "`color` is not allowed here"),
        (format!("{table}<tr><td style=\"border: 1px solid #000000\">a</td></tr></tbody></table>"), "input only"),
        (format!("{table}<tr><td style=\"box-sizing: content-box\">a</td></tr></tbody></table>"), "exactly when width or height"),
        (format!("{table}<tr><td style=\"width: 20px\">a</td></tr></tbody></table>"), "exactly when width or height"),
        (format!("{table}<tr><td style=\"box-sizing: content-box\">a</td></tr></tbody></table>"), "`box-sizing` without a size"),
        (format!("{table}<tr><td style=\"width: 20px\">a</td></tr></tbody></table>"), "or a size without it"),
        (format!("{table}<tr><th style=\"box-sizing: border-box; width: 20px\">a</th></tr></tbody></table>"), "a <th> has `box-sizing: border-box`"),
        (format!("{table}<tr><td style=\"border-top: 1px solid #000000\">a</td></tr></tbody></table>"), "a <td> writes 1 of its four border sides"),
        (format!("{table}<tr><th style=\"border-bottom: none; border-left: none; border-top: none\">a</th></tr></tbody></table>"), "a <th> writes 3 of its four border sides"),
        (format!("{table}<tr><td style=\"border-top: 21px solid #000000\">a</td></tr></tbody></table>"), "not a canonical value"),
        (format!("{table}<tr><td style=\"border-top: 1px none #000000\">a</td></tr></tbody></table>"), "not a canonical value"),
        ("<table><tbody style=\"display: table-row-group\"><tr><td>a</td></tr></tbody></table>".into(), "lacks its forced attribute `style`"),
        (format!("<p><iframe src=\"https://padlet.com/embed/x\" {iframe_attrs}>text</iframe></p>"), "an <iframe> has child nodes"),
        ("<p><sub><sup>a</sup></sub></p>".into(), "inside another <sub> or <sup>"),
        ("<p><span style=\"font-size: 3em\"><span style=\"font-size: 2em\">a</span></span></p>".into(), "multiply to 6"),
        ("<p><span style=\"font-size: 0.5em\"><span style=\"font-size: 0.5em\">a</span></span></p>".into(), "multiply to 0.25"),
        ("<p><span style=\"font-size: 1.5em\"><span style=\"font-size: 2em\">a</span></span></p>".into(), "`font-size: 2em` is inside a <span> with a font-size"),
        ("<p><span style=\"font-size: 2em\"><strong><span style=\"font-size: 0.5em\">a</span></strong></span></p>".into(), "`font-size: 0.5em` is inside a <span> with a font-size"),
        ("<p><span style=\"color: transparent\">a</span></p>".into(), "a <span> colour is not opaque: `transparent`"),
        ("<p><span style=\"color: #c0000080\">a</span></p>".into(), "a <span> colour is not opaque: `#c0000080`"),
        ("<p><svg><g></g></svg></p>".into(), "is not in the HTML namespace"),
        ("<noscript>a</noscript>".into(), "<noscript> is a forbidden tag"),
    ]
}

/// Each rule the oracle checks reports on markup that breaks it, so no
/// check can stop working unnoticed.
#[test]
fn the_oracle_reports_every_rule_it_checks() {
    for (html, needle) in &broken_markup() {
        let found = violations(html);
        assert!(
            found.iter().any(|v| v.contains(needle)),
            "{html}\nexpected `{needle}`, got {found:#?}"
        );
    }
}

/// The rules that hold for every tree a parser builds from text are
/// checked on trees built by hand.
#[test]
fn the_oracle_checks_shapes_that_no_parse_builds() {
    let (table, rel) = (TABLE, REL);
    // No parser builds a table inside a paragraph from text (in no-quirks
    // mode, a table start tag closes an open `p`), so the tree is built by
    // hand: the table of a valid body is moved into its paragraph.
    let mut tree = parse(&format!(
        "<p>a</p>{table}<tr><td>b</td></tr></tbody></table>"
    ));
    let ids: Vec<_> = tree
        .root_element()
        .children()
        .map(|child| child.id())
        .collect();
    assert_eq!(tree_violations(&tree), Vec::<String>::new());
    tree.tree
        .get_mut(ids[0])
        .expect("the paragraph")
        .append_id(ids[1]);
    assert_eq!(tree_violations(&tree), ["a <table> has a <p> ancestor"]);
    // Nor does `<a><a>` parse as a link inside a link (the second start tag
    // closes the first link), so the second link is moved into the first.
    let mut tree = parse(&format!(
        "<a href=\"/a\" {rel}>x</a><a href=\"/b\" {rel}>y</a>"
    ));
    let ids: Vec<_> = tree
        .root_element()
        .children()
        .map(|child| child.id())
        .collect();
    assert_eq!(tree_violations(&tree), Vec::<String>::new());
    tree.tree
        .get_mut(ids[0])
        .expect("the first link")
        .append_id(ids[1]);
    assert_eq!(
        tree_violations(&tree),
        ["an <a> is inside another <a> with no table cell between"]
    );
    // A link nested in a link with a cell between them is allowed (ADR 0037).
    let nested_in_cell = format!(
        "<a href=\"/a\" {rel}>{table}<tr><td><a href=\"/b\" {rel}>x</a></td></tr></tbody></table></a>"
    );
    let found = violations(&nested_in_cell);
    assert!(
        !found.iter().any(|v| v.contains("inside another <a>")),
        "{found:#?}"
    );
    // A parse in the HTML namespace never gives an attribute a namespace
    // (`xlink:href` on an HTML element is a plain name), so one is added
    // to a parsed paragraph by hand.
    let mut tree = parse("<p>a</p>");
    let id = tree
        .root_element()
        .first_child()
        .expect("the paragraph")
        .id();
    assert_eq!(tree_violations(&tree), Vec::<String>::new());
    let mut paragraph = tree.tree.get_mut(id).expect("the paragraph");
    let Node::Element(element) = paragraph.value() else {
        panic!("an element");
    };
    element.attrs.push((
        QualName::new(None, ns!(xlink), local_name!("href")),
        StrTendril::from("#x"),
    ));
    let found = tree_violations(&tree);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].starts_with("<p> has the namespaced attribute"),
        "{found:#?}"
    );
}

#[test]
fn canonical_shapes_pass_the_oracle() {
    let fixtures = fixtures();
    let (_, expected) = fixtures
        .get("canonical-shapes")
        .expect("the canonical-shapes fixture");
    assert_eq!(violations(expected), Vec::<String>::new());
    // Text, the empty output and plain marks pass.
    assert_eq!(violations(""), Vec::<String>::new());
    assert_eq!(violations("plain text &amp; more"), Vec::<String>::new());
    assert_eq!(
        violations("<p><strong>a</strong> <em>b</em> <u>c</u> <s>d</s></p>"),
        Vec::<String>::new()
    );
    // A path that ends in `/`, `/.` or `/..` names a directory, which no
    // static server renders as a document, so the sanitiser keeps such a
    // link into /storage/ (editor.md §3.6; research: DECISIONS R15, D87),
    // and the oracle agrees.
    for href in [
        "/storage/x.svg/",
        "/storage/x.svg%2F",
        "/storage/x.svg/.",
        "/storage/a.svg%2f%2e",
        "/storage/a.svg/..",
        "https://verseghy-gimnazium.net/storage/x.html/",
    ] {
        let html = format!("<p><a href=\"{href}\" {REL}>a</a></p>");
        let output = sanitizer().sanitize(&html).expect("a small input");
        assert_eq!(output.html.as_str(), html, "the sanitiser keeps {href}");
        assert_eq!(violations(&html), Vec::<String>::new(), "{href}");
    }
}

#[test]
fn numbers_and_colours_are_read_in_their_canonical_spelling() {
    for (text, expected) in [
        ("0", Some(0.0)),
        ("20", Some(20.0)),
        ("0.5", Some(0.5)),
        ("37.8", Some(37.8)),
        ("1.25", Some(1.25)),
    ] {
        assert_eq!(canonical_number(text), expected, "{text}");
    }
    for text in [
        "", "01", "1.", ".5", "1.50", "1.234", "-1", "1e3", "1.0", "+1", "0.0",
    ] {
        assert_eq!(canonical_number(text), None, "{text}");
    }
    assert!(canonical_colour("#c00000", true));
    assert!(canonical_colour("#c0000080", false));
    for (colour, opaque_only) in [
        ("#c0000080", true),
        ("#C00000", false),
        ("#c00", false),
        ("#c00000ff", false),
        ("transparent", true),
        ("red", false),
    ] {
        assert!(!canonical_colour(colour, opaque_only), "{colour}");
    }
    assert!(canonical_colour("currentcolor", false));
    assert_eq!(served_path("/a/./b/../c//d"), "/a/c/d");
    assert_eq!(served_path("/x/../storage/a.svg"), "/storage/a.svg");
    assert_eq!(served_path("/storage/x.svg/"), "/storage/x.svg/");
    assert_eq!(served_path("/storage/x.svg/."), "/storage/x.svg/");
    assert_eq!(served_path("/storage/a.svg/.."), "/storage/");
    assert_eq!(served_path("/storage/a.svg//"), "/storage/a.svg/");
    assert_eq!(served_path("/"), "/");
}
