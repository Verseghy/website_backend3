//! content-v1 as data: the tags, the attributes in canonical order with their
//! value rules, the removed and forbidden elements, the URL constants, and
//! the deployment-specific [`SanitizePolicy`] (editor.md §3.2, §3.5, §3.6,
//! §7.2).
//!
//! The tables are `const` data rather than code so that the ammonia builder,
//! the attribute filter, the removal report and, later, the policy manifest
//! (editor.md §3.9) read one source. Any change here is a policy change: it
//! bumps [`POLICY_VERSION`] and ships with a `content_normalize` run
//! (editor.md §3).
//!
//! [`SanitizePolicy`] holds what differs between deployments: the embed
//! providers, the image path prefix, backend3's own hosts and the input size
//! limit. It is built once at boot from the typed settings and passed in
//! explicitly (ADR 0008); [`SanitizePolicy::validate`] names the bad key.

use url::{Host, Url};

/// The stored HTML dialect this policy defines. Rows record it in
/// `content_policy` (ADR 0013 §1, §6).
pub const POLICY_VERSION: &str = "content-v1";

/// How the value of one allowed attribute is checked (editor.md §3.2, §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueRule {
    /// `style`: the per-element CSS grammar of editor.md §3.3. Until that
    /// grammar exists, every `style` attribute is dropped.
    Style,
    /// Free text kept as written (`img[alt]`, `iframe[title]`).
    Text,
    /// An embed marker whose value is always the empty string.
    Marker,
    /// A decimal integer from `min` to `max`. `default` is the value that
    /// HTML assumes when the attribute is absent, so it is omitted.
    Integer {
        /// The smallest accepted value.
        min: u32,
        /// The largest accepted value.
        max: u32,
        /// The value that is omitted instead of stored.
        default: Option<u32>,
    },
    /// `iframe[width]`: an integer from `min` to `max`, or exactly `100%`.
    IntegerOrFull {
        /// The smallest accepted integer.
        min: u32,
        /// The largest accepted integer.
        max: u32,
    },
    /// One of `values`. ammonia admits it only through `tag_attribute_values`
    /// (editor.md §7.2); `default` is omitted.
    Enumerated {
        /// Every accepted spelling.
        values: &'static [&'static str],
        /// The value that is omitted instead of stored.
        default: Option<&'static str>,
    },
    /// Exactly this value. ammonia admits the attribute only with it and adds
    /// it when it is missing (`set_tag_attribute_values`, editor.md §7.4).
    Forced(&'static str),
    /// `a[rel]`: never on the allow-list; ammonia's `link_rel` writes it.
    LinkRel,
    /// `img[src]`: [`super::url_rules::image_src`].
    ImageSrc,
    /// `iframe[src]`: [`super::url_rules::embed_src`].
    EmbedSrc,
    /// `a[href]`: the scheme list, the same-origin rule and
    /// [`super::url_rules::link_href`].
    LinkHref,
}

impl ValueRule {
    /// Whether ammonia treats the attribute as a URL: it checks the scheme
    /// at its allow-list step and the relative-URL rule after the filter.
    #[must_use]
    pub const fn is_url(self) -> bool {
        matches!(self, Self::ImageSrc | Self::EmbedSrc | Self::LinkHref)
    }
}

/// One allowed attribute of one element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeRule {
    /// The attribute name, lower-case as the HTML parser reports it.
    pub name: &'static str,
    /// How its value is checked.
    pub rule: ValueRule,
}

/// One content-v1 element with its attributes in canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementRule {
    /// The tag name.
    pub name: &'static str,
    /// The allowed attributes, in the order the stored HTML writes them.
    pub attributes: &'static [AttributeRule],
}

impl ElementRule {
    /// The rule for the attribute `name` on this element, if it is allowed.
    ///
    /// - `name`: the attribute name as the HTML parser reports it.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&'static AttributeRule> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
    }
}

const fn attribute(name: &'static str, rule: ValueRule) -> AttributeRule {
    AttributeRule { name, rule }
}

const fn element(name: &'static str, attributes: &'static [AttributeRule]) -> ElementRule {
    ElementRule { name, attributes }
}

/// An integer attribute with no default.
const fn integer(min: u32, max: u32) -> ValueRule {
    ValueRule::Integer {
        min,
        max,
        default: None,
    }
}

/// An integer attribute whose HTML default is `1`, which is omitted.
const fn integer_or_one(min: u32, max: u32) -> ValueRule {
    ValueRule::Integer {
        min,
        max,
        default: Some(1),
    }
}

/// `rel` on every link (ammonia's `link_rel`).
pub const LINK_REL: &str = "noopener noreferrer";
/// The one `target` of a link.
pub const LINK_TARGET: &str = "_blank";
/// The one `allow` of an embed.
pub const IFRAME_ALLOW: &str = "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share";
/// The one `referrerpolicy` of an embed.
pub const IFRAME_REFERRER_POLICY: &str = "strict-origin-when-cross-origin";

const NO_ATTRIBUTES: &[AttributeRule] = &[];
const STYLE_ONLY: &[AttributeRule] = &[attribute("style", ValueRule::Style)];
const CELL: &[AttributeRule] = &[
    attribute("colspan", integer_or_one(2, 1000)),
    attribute("rowspan", integer_or_one(2, 65534)),
    attribute("style", ValueRule::Style),
];

/// The content-v1 elements in the order of editor.md §3.2, each with its
/// attributes in canonical order.
pub const ELEMENTS: &[ElementRule] = &[
    element(
        "p",
        &[
            attribute("style", ValueRule::Style),
            attribute("data-youtube-video", ValueRule::Marker),
            attribute("data-padlet-embed", ValueRule::Marker),
        ],
    ),
    element("h1", STYLE_ONLY),
    element("h2", STYLE_ONLY),
    element("h3", STYLE_ONLY),
    element("h4", STYLE_ONLY),
    element("strong", NO_ATTRIBUTES),
    element("em", NO_ATTRIBUTES),
    element("u", NO_ATTRIBUTES),
    element("s", NO_ATTRIBUTES),
    element("sub", NO_ATTRIBUTES),
    element("sup", NO_ATTRIBUTES),
    element("br", NO_ATTRIBUTES),
    element("hr", NO_ATTRIBUTES),
    element("blockquote", NO_ATTRIBUTES),
    element("li", NO_ATTRIBUTES),
    element("tr", NO_ATTRIBUTES),
    element("tbody", STYLE_ONLY),
    element("span", STYLE_ONLY),
    element("ul", STYLE_ONLY),
    element(
        "ol",
        &[
            // "A non-negative integer of at most 4 digits; 1 is omitted."
            attribute("start", integer_or_one(0, 9999)),
            attribute(
                "type",
                ValueRule::Enumerated {
                    values: OL_TYPES,
                    default: Some("1"),
                },
            ),
        ],
    ),
    element(
        "a",
        &[
            attribute("href", ValueRule::LinkHref),
            attribute("target", ValueRule::Forced(LINK_TARGET)),
            attribute("rel", ValueRule::LinkRel),
        ],
    ),
    element(
        "img",
        &[
            attribute("src", ValueRule::ImageSrc),
            attribute("alt", ValueRule::Text),
            attribute("width", integer(1, 5000)),
            attribute("height", integer(1, 5000)),
            attribute("style", ValueRule::Style),
        ],
    ),
    element("table", STYLE_ONLY),
    element("td", CELL),
    element("th", CELL),
    element(
        "iframe",
        &[
            attribute("src", ValueRule::EmbedSrc),
            attribute("width", ValueRule::IntegerOrFull { min: 1, max: 4000 }),
            attribute("height", integer(1, 4000)),
            attribute("title", ValueRule::Text),
            attribute("style", ValueRule::Style),
            attribute("allow", ValueRule::Forced(IFRAME_ALLOW)),
            attribute("allowfullscreen", ValueRule::Forced("")),
            attribute("referrerpolicy", ValueRule::Forced(IFRAME_REFERRER_POLICY)),
        ],
    ),
];

/// The `ol[type]` values. `1` is the default and is omitted.
const OL_TYPES: &[&str] = &["a", "A", "i", "I", "1"];

/// Elements removed together with their content (ammonia's
/// `clean_content_tags`, editor.md §3.5): executable or raw-text content,
/// Word's `<xml>` metadata, the `svg` and `math` subtrees, form controls, and
/// the paragraph-scope boundaries `button` and `marquee`.
pub const REMOVED_WITH_CONTENT: &[&str] = &[
    "script", "style", "title", "xml", "template", "noscript", "noembed", "noframes", "object",
    "embed", "applet", "svg", "math", "textarea", "select", "button", "marquee", "video", "audio",
    "canvas", "frameset", "frame",
];

/// Tags that must never enter [`ELEMENTS`] (editor.md §3.5): their parsing
/// depends on the scripting flag, raw text or quirks mode, which differ
/// along the public sites' sink chain.
#[rustfmt::skip]
pub const FORBIDDEN_TAGS: &[&str] = &[
    "noscript", "template", "style", "script", "xmp", "noembed", "noframes", "plaintext",
    "textarea", "title", "listing", "select", "option",
];

/// The URL schemes a URL attribute may carry (editor.md §3.6, §7.2).
pub const URL_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

/// The file extensions an image source may end in, compared case-insensitively.
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];

/// The path under which backend3's hosts serve the uploads volume.
pub const STORAGE_PATH_PREFIX: &str = "/storage/";

/// Extensions a link into [`STORAGE_PATH_PREFIX`] on an own host must not end
/// in: a browser could render such a file as a document (editor.md §3.6).
pub const FORBIDDEN_STORAGE_LINK_EXTENSIONS: &[&str] = &[
    "svg", "svgz", "htm", "html", "xhtml", "xht", "xml", "xsl", "xslt",
];

/// backend3's own hosts, including the retired one (editor.md §3.6, ADR 0014).
pub const OWN_HOSTS: &[&str] = &[
    "verseghy-gimnazium.net",
    "backend.microshift.verseghy-gimnazium.net",
    "backend3.microshift.verseghy-gimnazium.net",
    "backend.verseghy-gimnazium.net",
];

/// The prefix every image source must start with.
pub const IMAGE_PATH_PREFIX: &str = "/storage/uploads/";

/// The path prefix of both embed providers.
pub const EMBED_PATH_PREFIX: &str = "/embed/";

/// The default input limit, above the largest legacy body (editor.md §7.2).
pub const DEFAULT_MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;

/// The accepted range of [`SanitizePolicy::max_input_bytes`].
pub const MAX_INPUT_BYTES_RANGE: core::ops::RangeInclusive<usize> = 1024..=16 * 1024 * 1024;

/// The content-v1 rule for `name`, or `None` when `name` is not a content-v1
/// element (it is then unwrapped or removed).
///
/// - `name`: a tag name as the HTML parser reports it.
#[must_use]
pub fn element_rule(name: &str) -> Option<&'static ElementRule> {
    ELEMENTS.iter().find(|element| element.name == name)
}

/// Which query parameters an embed provider's URLs may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedQuery {
    /// No query at all (Padlet).
    None,
    /// `start=<integer>` and `list=<id>`, each optional, in that order
    /// (the `youtube.com` hosts).
    StartAndList,
}

/// One embed provider: the hosts its iframes may load from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedProvider {
    /// Exact host names, lower-case.
    pub hosts: Vec<String>,
    /// The path prefix before the video or board id, such as `/embed/`.
    pub path_prefix: String,
    /// The query parameters the provider's URLs may carry.
    pub query: EmbedQuery,
}

/// The deployment-specific part of the policy (editor.md §7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizePolicy {
    /// The providers an `iframe` may embed.
    pub embed_providers: Vec<EmbedProvider>,
    /// The prefix every image source must start with.
    pub image_path_prefix: String,
    /// backend3's own hosts: absolute image URLs on them become root-relative,
    /// and links to them follow the `/storage` rule.
    pub own_hosts: Vec<String>,
    /// The largest input the sanitiser accepts, in bytes.
    pub max_input_bytes: usize,
}

/// A [`SanitizePolicy`] value that cannot be used, naming the key to fix.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("rich-text policy `{key}`: {problem}")]
pub struct PolicyError {
    /// The key, relative to the policy, for example `own_hosts[1]`. The
    /// caller prefixes it with the settings path it read the policy from.
    pub key: String,
    /// What is wrong and what is expected.
    pub problem: String,
}

impl SanitizePolicy {
    /// The content-v1 policy with the values editor.md §3.6 and §7.2 list.
    #[must_use]
    pub fn content_v1() -> Self {
        let provider = |hosts: &[&str], query| EmbedProvider {
            hosts: hosts.iter().map(|&host| host.to_owned()).collect(),
            path_prefix: EMBED_PATH_PREFIX.to_owned(),
            query,
        };
        Self {
            embed_providers: vec![
                provider(
                    &["www.youtube.com", "www.youtube-nocookie.com"],
                    EmbedQuery::StartAndList,
                ),
                provider(&["padlet.com"], EmbedQuery::None),
            ],
            image_path_prefix: IMAGE_PATH_PREFIX.to_owned(),
            own_hosts: OWN_HOSTS.iter().map(|&host| host.to_owned()).collect(),
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
        }
    }

    /// Checks every value and returns the first bad one.
    ///
    /// Prefixes must start and end with `/`, contain no `//` or `..`, and be
    /// written as the `url` crate serialises a path (otherwise no parsed URL
    /// could ever match them). Hosts must be lower-case domain names without
    /// a trailing dot. The byte limit must lie in [`MAX_INPUT_BYTES_RANGE`].
    ///
    /// # Errors
    ///
    /// A [`PolicyError`] that names the key and the expected form.
    pub fn validate(&self) -> Result<(), PolicyError> {
        check_prefix("image_path_prefix", &self.image_path_prefix)?;
        for (index, host) in self.own_hosts.iter().enumerate() {
            check_host(&format!("own_hosts[{index}]"), host)?;
        }
        for (index, provider) in self.embed_providers.iter().enumerate() {
            let key = format!("embed_providers[{index}]");
            check_prefix(&format!("{key}.path_prefix"), &provider.path_prefix)?;
            if provider.hosts.is_empty() {
                return Err(error(
                    &format!("{key}.hosts"),
                    "must list at least one host",
                ));
            }
            for (host_index, host) in provider.hosts.iter().enumerate() {
                check_host(&format!("{key}.hosts[{host_index}]"), host)?;
            }
        }
        if !MAX_INPUT_BYTES_RANGE.contains(&self.max_input_bytes) {
            return Err(error(
                "max_input_bytes",
                &format!(
                    "must be from {} to {} bytes, got {}",
                    MAX_INPUT_BYTES_RANGE.start(),
                    MAX_INPUT_BYTES_RANGE.end(),
                    self.max_input_bytes
                ),
            ));
        }
        Ok(())
    }
}

fn error(key: &str, problem: &str) -> PolicyError {
    PolicyError {
        key: key.to_owned(),
        problem: problem.to_owned(),
    }
}

fn check_prefix(key: &str, prefix: &str) -> Result<(), PolicyError> {
    let shape_ok = prefix.starts_with('/')
        && prefix.ends_with('/')
        && !prefix.contains("//")
        && !prefix.contains("..");
    // A prefix the URL parser would rewrite (a space, `%`, `\`, `?`, `#`, a
    // non-ASCII letter) never equals the start of a parsed path.
    let serialised = Url::parse("https://policy.invalid")
        .and_then(|base| base.join(prefix))
        .is_ok_and(|url| url.path() == prefix && url.query().is_none() && url.fragment().is_none());
    if shape_ok && serialised {
        Ok(())
    } else {
        Err(error(
            key,
            &format!(
                "must start and end with `/`, contain no `//` or `..`, and use only characters a URL path keeps as they are, got `{prefix}`"
            ),
        ))
    }
}

fn check_host(key: &str, host: &str) -> Result<(), PolicyError> {
    let is_domain = matches!(Host::parse(host), Ok(Host::Domain(parsed)) if parsed == host);
    if is_domain && !host.ends_with('.') {
        Ok(())
    } else {
        Err(error(
            key,
            &format!("must be a lower-case domain name without a trailing dot, got `{host}`"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_element_list_is_exactly_content_v1() {
        let names: Vec<&str> = ELEMENTS.iter().map(|element| element.name).collect();
        assert_eq!(
            names,
            [
                "p",
                "h1",
                "h2",
                "h3",
                "h4",
                "strong",
                "em",
                "u",
                "s",
                "sub",
                "sup",
                "br",
                "hr",
                "blockquote",
                "li",
                "tr",
                "tbody",
                "span",
                "ul",
                "ol",
                "a",
                "img",
                "table",
                "td",
                "th",
                "iframe",
            ]
        );
    }

    #[test]
    fn the_attribute_order_is_the_canonical_one() {
        let order = |name: &str| -> Vec<&str> {
            element_rule(name)
                .unwrap()
                .attributes
                .iter()
                .map(|a| a.name)
                .collect()
        };
        assert_eq!(
            order("p"),
            ["style", "data-youtube-video", "data-padlet-embed"]
        );
        assert_eq!(order("ol"), ["start", "type"]);
        assert_eq!(order("a"), ["href", "target", "rel"]);
        assert_eq!(order("img"), ["src", "alt", "width", "height", "style"]);
        assert_eq!(order("td"), ["colspan", "rowspan", "style"]);
        assert_eq!(
            order("iframe"),
            [
                "src",
                "width",
                "height",
                "title",
                "style",
                "allow",
                "allowfullscreen",
                "referrerpolicy"
            ]
        );
        for name in [
            "strong",
            "em",
            "u",
            "s",
            "sub",
            "sup",
            "br",
            "hr",
            "blockquote",
            "li",
            "tr",
        ] {
            assert!(order(name).is_empty(), "{name}");
        }
        for name in ["h1", "h4", "tbody", "span", "ul", "table"] {
            assert_eq!(order(name), ["style"], "{name}");
        }
    }

    #[test]
    fn the_tables_keep_their_invariants() {
        for element in ELEMENTS {
            assert!(
                !FORBIDDEN_TAGS.contains(&element.name),
                "{} is forbidden",
                element.name
            );
            // ammonia panics when a tag is both kept and removed with content.
            assert!(
                !REMOVED_WITH_CONTENT.contains(&element.name),
                "{}",
                element.name
            );
            for (index, attribute) in element.attributes.iter().enumerate() {
                assert_eq!(
                    element.attribute(attribute.name),
                    Some(attribute),
                    "duplicate"
                );
                // ammonia decides by name which attributes are URLs.
                let named_as_url = matches!(attribute.name, "href" | "src");
                assert_eq!(attribute.rule.is_url(), named_as_url, "{}", attribute.name);
                if attribute.rule == ValueRule::LinkRel {
                    // `link_rel` pushes `rel` after everything else.
                    assert_eq!((element.name, index + 1), ("a", element.attributes.len()));
                }
            }
        }
        assert_eq!(element_rule("div"), None);
        assert_eq!(element_rule("P"), None, "the parser lower-cases tag names");
    }

    #[test]
    fn content_v1_is_valid() {
        assert_eq!(SanitizePolicy::content_v1().validate(), Ok(()));
    }

    #[test]
    fn validate_names_the_bad_key() {
        let check = |change: fn(&mut SanitizePolicy)| {
            let mut policy = SanitizePolicy::content_v1();
            change(&mut policy);
            policy.validate().unwrap_err().key
        };
        assert_eq!(
            check(|p| p.image_path_prefix = "storage/".into()),
            "image_path_prefix"
        );
        assert_eq!(
            check(|p| p.image_path_prefix = "/storage".into()),
            "image_path_prefix"
        );
        assert_eq!(
            check(|p| p.image_path_prefix = "/a//b/".into()),
            "image_path_prefix"
        );
        assert_eq!(
            check(|p| p.image_path_prefix = "/a/../b/".into()),
            "image_path_prefix"
        );
        assert_eq!(
            check(|p| p.image_path_prefix = "/a b/".into()),
            "image_path_prefix"
        );
        assert_eq!(
            check(|p| p.image_path_prefix = "/képek/".into()),
            "image_path_prefix"
        );
        assert_eq!(
            check(|p| p.own_hosts[1] = "Example.org".into()),
            "own_hosts[1]"
        );
        assert_eq!(
            check(|p| p.own_hosts[0] = "127.0.0.1".into()),
            "own_hosts[0]"
        );
        assert_eq!(
            check(|p| p.own_hosts[0] = "example.0x".into()),
            "own_hosts[0]"
        );
        assert_eq!(check(|p| p.own_hosts[0] = String::new()), "own_hosts[0]");
        assert_eq!(
            check(|p| p.own_hosts[0] = "example.org.".into()),
            "own_hosts[0]"
        );
        assert_eq!(
            check(|p| p.own_hosts[0] = "exa mple.org".into()),
            "own_hosts[0]"
        );
        assert_eq!(
            check(|p| p.embed_providers[1].path_prefix = "/embed".into()),
            "embed_providers[1].path_prefix"
        );
        assert_eq!(
            check(|p| p.embed_providers[0].hosts[1] = "WWW.youtube.com".into()),
            "embed_providers[0].hosts[1]"
        );
        assert_eq!(
            check(|p| p.embed_providers[1].hosts.clear()),
            "embed_providers[1].hosts"
        );
        assert_eq!(check(|p| p.max_input_bytes = 1023), "max_input_bytes");
        assert_eq!(
            check(|p| p.max_input_bytes = 16 * 1024 * 1024 + 1),
            "max_input_bytes"
        );
    }

    #[test]
    fn the_message_says_what_to_fix() {
        let mut policy = SanitizePolicy::content_v1();
        policy.own_hosts[2] = "Backend3.example".into();
        assert_eq!(
            policy.validate().unwrap_err().to_string(),
            "rich-text policy `own_hosts[2]`: must be a lower-case domain name without a \
             trailing dot, got `Backend3.example`"
        );
    }

    proptest! {
        #[test]
        fn the_byte_limit_is_valid_exactly_inside_its_range(limit in 0usize..20 * 1024 * 1024) {
            let policy = SanitizePolicy { max_input_bytes: limit, ..SanitizePolicy::content_v1() };
            prop_assert_eq!(policy.validate().is_ok(), MAX_INPUT_BYTES_RANGE.contains(&limit));
        }

        #[test]
        fn lower_case_domains_are_valid_hosts(labels in prop::collection::vec("[a-z][a-z0-9]{0,9}", 1..4)) {
            // Labels start with a letter: a last label that looks like a
            // number (`123`, `0x`) makes the host an IPv4 address, and no
            // hyphen, which could form an invalid `xn--` label.
            let host = labels.join(".");
            let policy = SanitizePolicy { own_hosts: vec![host.clone()], ..SanitizePolicy::content_v1() };
            prop_assert!(policy.validate().is_ok(), "{}", host);
            let upper = SanitizePolicy { own_hosts: vec![format!("X{host}")], ..SanitizePolicy::content_v1() };
            prop_assert!(upper.validate().is_err());
        }

        #[test]
        fn prefixes_of_plain_segments_are_valid(segments in prop::collection::vec("[a-z0-9_-]{1,8}", 0..4)) {
            let prefix = if segments.is_empty() { "/".to_owned() } else { format!("/{}/", segments.join("/")) };
            let policy = SanitizePolicy { image_path_prefix: prefix.clone(), ..SanitizePolicy::content_v1() };
            prop_assert_eq!(policy.validate(), Ok(()));
            let open = SanitizePolicy { image_path_prefix: prefix.trim_end_matches('/').to_owned() + "x", ..SanitizePolicy::content_v1() };
            prop_assert!(open.validate().is_err());
        }

        #[test]
        fn element_rule_finds_only_content_v1_tags(name in "[a-z0-9:-]{1,12}") {
            let found = element_rule(&name);
            prop_assert_eq!(found.is_some(), ELEMENTS.iter().any(|e| e.name == name));
            if let Some(rule) = found {
                prop_assert_eq!(rule.name, name.as_str());
            }
        }

        #[test]
        fn attribute_finds_only_listed_attributes(name in "[a-z-]{1,20}") {
            for element in ELEMENTS {
                let found = element.attribute(&name);
                prop_assert_eq!(found.is_some(), element.attributes.iter().any(|a| a.name == name));
            }
        }
    }
}
