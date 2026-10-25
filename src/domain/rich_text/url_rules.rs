//! The URL rules of content-v1 (editor.md §3.6): which `href` and `src`
//! values the sanitiser keeps, and in which spelling.
//!
//! Every check parses with the `url` crate, which implements the WHATWG URL
//! standard the browsers use, so `java\tscript:`, `/\host` and similar
//! spellings are seen the way a browser sees them. Relative URLs are
//! resolved against [`SAME_ORIGIN_BASE`], a reserved host that stands for
//! "the page's own origin".
//!
//! ammonia's order matters for safety (editor.md §3.6, §7.4): it checks the
//! scheme of a URL attribute first ([`scheme_admitted`]), then runs the
//! attribute filter, and finally applies [`same_origin_relative`] to filter
//! output that is relative; filter output is never checked against the
//! scheme list again. The rules are safe under that order because
//! [`image_src`] returns a root-relative path, [`embed_src`] an `https` URL,
//! and [`link_href`] its input unchanged.

use std::borrow::Cow;
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use url::{ParseError, Url};

use super::policy::{
    EmbedQuery, FORBIDDEN_STORAGE_LINK_EXTENSIONS, IMAGE_EXTENSIONS, STORAGE_PATH_PREFIX,
    SanitizePolicy, URL_SCHEMES,
};

/// The base that relative URLs are resolved against. `.invalid` is reserved
/// (RFC 2606), so it can never be a real origin a URL points to.
pub const SAME_ORIGIN_BASE: &str = "https://same-origin.invalid/";

static BASE: LazyLock<Url> =
    LazyLock::new(|| Url::parse(SAME_ORIGIN_BASE).expect("the same-origin base is a valid URL"));

/// Whether `value` is a relative URL: one that has no scheme, as ammonia
/// decides it.
///
/// - `value`: an attribute value after HTML character references are decoded.
#[must_use]
pub fn is_relative(value: &str) -> bool {
    matches!(Url::parse(value), Err(ParseError::RelativeUrlWithoutBase))
}

/// The scheme check that ammonia 4.2.4 makes at its allow-list step
/// (ammonia@4.2.4:src/lib.rs, `clean_child`): an absolute URL must use one of
/// [`URL_SCHEMES`], a relative URL passes on to the later rules, and a value
/// that does not parse at all is dropped.
///
/// - `value`: the attribute value.
#[must_use]
pub fn scheme_admitted(value: &str) -> bool {
    match Url::parse(value) {
        Ok(url) => URL_SCHEMES.contains(&url.scheme()),
        Err(ParseError::RelativeUrlWithoutBase) => true,
        Err(_) => false,
    }
}

/// ammonia's relative-URL rule (`UrlRelative::Custom`): keeps a relative URL
/// unchanged when it resolves to the page's own origin, and drops it
/// otherwise.
///
/// The `url` crate calls `//host`, `/\host` and their tab- or
/// newline-obfuscated forms relative, though a browser loads them from
/// `host`. They are refused by their shape, whatever the host: resolving
/// them would keep `//same-origin.invalid/x`, which names the base itself.
/// `#fragment`, `?query` and path-relative links stay on the page's origin
/// and are kept. An absolute URL is not relative and returns `None`; ammonia
/// never passes one.
///
/// - `value`: the filtered attribute value.
#[must_use]
pub fn same_origin_relative(value: &str) -> Option<Cow<'_, str>> {
    if !is_relative(value) || leading_slashes(value) > 1 {
        return None;
    }
    let resolved = BASE.join(value).ok()?;
    (resolved.origin() == BASE.origin()).then_some(Cow::Borrowed(value))
}

/// The `img[src]` rule: the root-relative path of an uploaded image, or
/// `None` when the value is not one.
///
/// Accepted: a path-absolute URL, or an `http` or `https` URL on one of the
/// policy's own hosts (rewritten to its path), whose path starts with the
/// image prefix and whose last segment ends in one of [`IMAGE_EXTENSIONS`],
/// with no query and no fragment. The result is the `url` crate's
/// serialisation of the path, with dot segments resolved.
///
/// The prefix and the extension are compared with that serialisation, as
/// editor.md §3.6 says, so a percent-encoded spelling of either
/// (`/%73torage/`, `a%2Ejpg`) does not match, and one file has one spelling.
/// No character is restricted beyond what the `url` crate serialises,
/// because legacy image paths contain `(` and `)`.
///
/// - `policy`: supplies the own hosts and the image prefix.
/// - `value`: the attribute value.
#[must_use]
pub fn image_src(policy: &SanitizePolicy, value: &str) -> Option<String> {
    let url = match Url::parse(value) {
        Ok(url) => {
            let plain =
                url.port().is_none() && url.username().is_empty() && url.password().is_none();
            (matches!(url.scheme(), "http" | "https") && is_own_host(policy, &url) && plain)
                .then_some(url)?
        }
        // Path-relative values are refused: they resolve against the page's
        // own path, which the sanitiser cannot know.
        Err(ParseError::RelativeUrlWithoutBase) if leading_slashes(value) == 1 => BASE
            .join(value)
            .ok()
            .filter(|url| url.origin() == BASE.origin())?,
        Err(_) => return None,
    };
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    let path = url.path();
    // An absolute URL such as `https://own//evil.example/x.jpg` has a path
    // that a browser reads as scheme-relative once it stands alone; refusing
    // it keeps the result root-relative even under a policy prefix of `/`.
    (path.starts_with(policy.image_path_prefix.as_str())
        && !path.starts_with("//")
        && ends_with_extension(path, IMAGE_EXTENSIONS))
    .then(|| path.to_owned())
}

/// The `iframe[src]` rule: the canonical spelling of an embed URL, or `None`
/// when the value is not one.
///
/// Accepted: an `https` URL with no user info, port or fragment, whose host
/// is exactly one of a provider's hosts, whose path is the provider's prefix
/// followed by an id of `[A-Za-z0-9_-]+`, and whose query follows the
/// provider's [`EmbedQuery`]. The result is the `url` crate's serialisation,
/// so an upper-case host comes back lower-case.
///
/// - `policy`: supplies the embed providers.
/// - `value`: the attribute value.
#[must_use]
pub fn embed_src(policy: &SanitizePolicy, value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    let plain = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none();
    let host = url.host_str().filter(|_| plain)?;
    let provider = policy
        .embed_providers
        .iter()
        .find(|p| p.hosts.iter().any(|h| h == host))?;
    let id = url.path().strip_prefix(provider.path_prefix.as_str())?;
    let query_ok = match (provider.query, url.query()) {
        (_, None) => true,
        (EmbedQuery::None, Some(_)) => false,
        (EmbedQuery::StartAndList, Some(query)) => is_start_and_list(query),
    };
    (is_embed_id(id) && query_ok).then(|| url.as_str().to_owned())
}

/// The `a[href]` rule beyond the scheme list and [`same_origin_relative`]:
/// the value unchanged, or `None` for a link to a file under `/storage/` on
/// an own host (a relative link counts as one) whose last segment ends in one
/// of [`FORBIDDEN_STORAGE_LINK_EXTENSIONS`].
///
/// A relative link is resolved as from a page at the root, the position from
/// which the most relative forms reach `/storage/`. The rule is about the
/// file the link reaches, so the path is compared the way a static server
/// maps it to a file: percent-decoded, with its dot segments resolved and
/// repeated slashes merged after decoding (nginx normalises the decoded
/// URI), and lower-cased. `/%73torage/x.svg`, `/x/..%2Fstorage/x.svg` and
/// `/STORAGE/x.SVG` are refused with `/storage/x.svg`.
///
/// - `policy`: supplies the own hosts.
/// - `value`: the attribute value.
#[must_use]
pub fn link_href<'a>(policy: &SanitizePolicy, value: &'a str) -> Option<Cow<'a, str>> {
    let target = match Url::parse(value) {
        Ok(url) => {
            (matches!(url.scheme(), "http" | "https") && is_own_host(policy, &url)).then_some(url)
        }
        Err(ParseError::RelativeUrlWithoutBase) => BASE
            .join(value)
            .ok()
            .filter(|url| url.origin() == BASE.origin()),
        Err(_) => None,
    };
    let forbidden = target.is_some_and(|url| {
        let decoded = percent_decode_str(url.path())
            .decode_utf8_lossy()
            .to_lowercase();
        let served = served_path(&decoded);
        served.starts_with(STORAGE_PATH_PREFIX)
            && ends_with_extension(&served, FORBIDDEN_STORAGE_LINK_EXTENSIONS)
    });
    (!forbidden).then_some(Cow::Borrowed(value))
}

/// The file path a static server serves for the decoded, `/`-led `path`:
/// `.` and `..` segments resolved and empty segments merged. A trailing
/// slash is kept, because it names a directory rather than a file.
fn served_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    let mut parts = path.split('/').skip(1).peekable();
    while let Some(segment) = parts.next() {
        let last = parts.peek().is_none();
        match segment {
            "." => {}
            ".." => {
                segments.pop();
            }
            "" if !last => {}
            _ => segments.push(segment),
        }
        if last && matches!(segment, "." | "..") {
            segments.push("");
        }
    }
    format!("/{}", segments.join("/"))
}

/// Whether the URL's host is one of the policy's own hosts. A trailing dot
/// names the same host to DNS, so it is ignored.
fn is_own_host(policy: &SanitizePolicy, url: &Url) -> bool {
    url.domain()
        .map(|domain| domain.strip_suffix('.').unwrap_or(domain))
        .is_some_and(|domain| policy.own_hosts.iter().any(|own| own == domain))
}

/// How many of the first two characters of a relative URL are `/` or `\`
/// (which WHATWG parsing treats as `/`), after the parser's own clean-up:
/// leading and trailing C0 controls and spaces are trimmed, tabs and
/// newlines removed. One means path-absolute; two means scheme-relative.
fn leading_slashes(value: &str) -> usize {
    value
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .take(2)
        .take_while(|c| matches!(c, '/' | '\\'))
        .count()
}

/// Whether the last `/`-separated segment of `path` ends in `.` and one of
/// `extensions`, ignoring ASCII case.
fn ends_with_extension(path: &str, extensions: &[&str]) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    extensions.iter().any(|extension| {
        name.strip_suffix(extension)
            .is_some_and(|stem| stem.ends_with('.'))
    })
}

fn is_embed_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `start=<digits>`, `list=<id>`, or both in that order.
fn is_start_and_list(query: &str) -> bool {
    let start_ok = |param: &str| {
        param
            .strip_prefix("start=")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    let list_ok = |param: &str| param.strip_prefix("list=").is_some_and(is_embed_id);
    match query.split_once('&') {
        None => start_ok(query) || list_ok(query),
        Some((start, list)) => start_ok(start) && list_ok(list),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::policy::{IMAGE_PATH_PREFIX, OWN_HOSTS};
    use proptest::prelude::*;

    fn policy() -> SanitizePolicy {
        SanitizePolicy::content_v1()
    }

    #[test]
    fn scheme_admitted_follows_the_scheme_list() {
        for value in [
            "https://x.example/",
            "http://x",
            "mailto:a@b.example",
            "tel:+3612345",
            "/x",
            "x",
            "#f",
            "",
        ] {
            assert!(scheme_admitted(value), "{value}");
        }
        for value in [
            "javascript:alert(1)",
            "java\tscript:x",
            " javascript:x",
            "data:image/png;base64,AA",
            "vbscript:x",
            "blob:https://x/1",
            "file:///etc/passwd",
            "cid:abc",
            "http://[::1",
            "https://",
        ] {
            assert!(!scheme_admitted(value), "{value}");
        }
    }

    #[test]
    fn is_relative_means_no_scheme() {
        for value in ["/x", "x", "#f", "?q=1", "", "//host/x", "\\x", "%41:x"] {
            assert!(is_relative(value), "{value}");
        }
        for value in [
            "https://x.example/",
            "mailto:a@b.example",
            "javascript:x",
            "data:,x",
            // Unparsable absolute URLs are errors of another kind.
            "http://[::1",
            "https://",
        ] {
            assert!(!is_relative(value), "{value}");
        }
    }

    #[test]
    fn same_origin_relative_rejects_other_hosts() {
        for value in ["/x", "x/y", "../x", "#top", "?q=1", "", "/%2F/evil", ":x"] {
            assert_eq!(
                same_origin_relative(value),
                Some(Cow::Borrowed(value)),
                "{value}"
            );
        }
        for value in [
            "//evil.example/x",
            "/\\evil.example/x",
            "\\\\evil.example",
            "\t//evil.example",
            "/\t/evil.example",
            " //evil.example",
            "\\/evil.example",
            "//same-origin.invalid/x",
            "\\\\same-origin.invalid/x",
            "https://evil.example/",
        ] {
            assert_eq!(same_origin_relative(value), None, "{value:?}");
        }
    }

    #[test]
    fn image_src_accepts_uploads_and_rewrites_own_hosts() {
        let accept = |value: &str, expected: &str| {
            assert_eq!(
                image_src(&policy(), value).as_deref(),
                Some(expected),
                "{value}"
            );
        };
        accept(
            "/storage/uploads/posts/612/kep.jpg",
            "/storage/uploads/posts/612/kep.jpg",
        );
        for extension in ["jpg", "JPEG", "png", "gif", "WebP"] {
            let path = format!("/storage/uploads/a.{extension}");
            accept(&path, &path);
        }
        for host in OWN_HOSTS {
            accept(
                &format!("https://{host}/storage/uploads/a.png"),
                "/storage/uploads/a.png",
            );
            accept(
                &format!("http://{host}/storage/uploads/a.png"),
                "/storage/uploads/a.png",
            );
        }
        accept(
            "https://verseghy-gimnazium.net./storage/uploads/a.png",
            "/storage/uploads/a.png",
        );
        accept(
            "https://verseghy-gimnazium.net:443/storage/uploads/a.png",
            "/storage/uploads/a.png",
        );
        accept(
            "/storage/uploads/kép (1).jpg",
            "/storage/uploads/k%C3%A9p%20(1).jpg",
        );
        accept("/storage/uploads/a/../b.gif", "/storage/uploads/b.gif");
        accept("\\storage\\uploads\\c.jpg", "/storage/uploads/c.jpg");
        accept(" /storage/uploads/d.jpg ", "/storage/uploads/d.jpg");
        // No character is restricted beyond the `url` crate's serialisation.
        accept("/storage/uploads/%FF.jpg", "/storage/uploads/%FF.jpg");
    }

    #[test]
    fn image_src_stays_root_relative_under_any_prefix() {
        let root = SanitizePolicy {
            image_path_prefix: "/".into(),
            ..policy()
        };
        assert_eq!(image_src(&root, "/x.jpg").as_deref(), Some("/x.jpg"));
        for value in [
            "https://verseghy-gimnazium.net//evil.example/x.jpg",
            "/\\evil.example/x.jpg",
        ] {
            assert_eq!(image_src(&root, value), None, "{value}");
        }
    }

    #[test]
    fn image_src_rejects_everything_else() {
        for value in [
            "data:image/png;base64,iVBORw0KGgo=",
            "blob:https://verseghy-gimnazium.net/1",
            "file:///C:/kep.jpg",
            "cid:image001.png@01D",
            "javascript:alert(1)",
            "https://evil.example/storage/uploads/a.jpg",
            "https://verseghy-gimnazium.net.evil.example/storage/uploads/a.jpg",
            "https://verseghy-gimnazium.net:8443/storage/uploads/a.jpg",
            "https://user@verseghy-gimnazium.net/storage/uploads/a.jpg",
            "ftp://verseghy-gimnazium.net/storage/uploads/a.jpg",
            "//evil.example/storage/uploads/a.jpg",
            "/\\evil.example/storage/uploads/a.jpg",
            "storage/uploads/a.jpg",
            "../storage/uploads/a.jpg",
            "/storage/uploads/a.jpg?x=1",
            "/storage/uploads/a.jpg?",
            "/storage/uploads/a.jpg#f",
            "/storage/a.jpg",
            "/storage/uploadsx/a.jpg",
            "/storage/uploads/a.svg",
            "/storage/uploads/a.jpg.html",
            "/storage/uploads/jpg",
            "/storage/uploads/",
            "/storage/uploads/../../x.jpg",
            "/storage/uploads/%2e%2e/x.jpg",
            "/storage/uploads/a.svg%00.jpg/..",
            // The prefix and the extension are compared as serialised.
            "/%73torage/uploads/a.jpg",
            "/storage/%75ploads/a.jpg",
            "/storage%2Fuploads/a.jpg",
            "/storage/uploads/a%2Ejpg",
            "/storage/uploads/a.%6Apg",
            "https://verseghy-gimnazium.net//storage/uploads/a.jpg",
            "",
        ] {
            assert_eq!(image_src(&policy(), value), None, "{value:?}");
        }
    }

    #[test]
    fn embed_src_accepts_the_providers() {
        let accept = |value: &str, expected: &str| {
            assert_eq!(
                embed_src(&policy(), value).as_deref(),
                Some(expected),
                "{value}"
            );
        };
        for url in [
            "https://www.youtube-nocookie.com/embed/VIDEOID",
            "https://www.youtube-nocookie.com/embed/VIDEOID?start=30",
            "https://www.youtube-nocookie.com/embed/VIDEOID?list=PLa_b-1",
            "https://www.youtube-nocookie.com/embed/VIDEOID?start=0&list=PL1",
            "https://www.youtube.com/embed/a-b_C9",
            "https://padlet.com/embed/KEY123",
        ] {
            accept(url, url);
        }
        accept(
            "https://WWW.YouTube-NoCookie.com/embed/x",
            "https://www.youtube-nocookie.com/embed/x",
        );
        accept(
            "https://padlet.com:443/embed/k",
            "https://padlet.com/embed/k",
        );
    }

    #[test]
    fn embed_src_rejects_everything_else() {
        for value in [
            "http://www.youtube-nocookie.com/embed/x",
            "https://youtube.com/embed/x",
            "https://m.youtube.com/embed/x",
            "https://www.youtube.com.evil.example/embed/x",
            "https://www.youtube.com./embed/x",
            "https://www.youtube.com@evil.example/embed/x",
            "https://evil@www.youtube.com/embed/x",
            "https://www.youtube.com:8443/embed/x",
            "https://www.youtube.com/embed/",
            "https://www.youtube.com/embed/a/b",
            "https://www.youtube.com/embed/a%20b",
            "https://www.youtube.com/watch?v=x",
            "https://www.youtube.com/embed/x?si=abc",
            "https://www.youtube.com/embed/x?list=a&start=1",
            "https://www.youtube.com/embed/x?start=",
            "https://www.youtube.com/embed/x?start=-1",
            "https://www.youtube.com/embed/x?start=1&list=a&rel=0",
            "https://www.youtube.com/embed/x?",
            "https://www.youtube.com/embed/x#t=1",
            "https://padlet.com/embed/k?x=1",
            "https://evil.example/embed/x",
            "/embed/x",
            "javascript:alert(1)",
        ] {
            assert_eq!(embed_src(&policy(), value), None, "{value}");
        }
    }

    #[test]
    fn link_href_refuses_documents_under_storage_on_own_hosts() {
        for value in [
            "/storage/x.svg",
            "/storage/uploads/a/b.HTML",
            "storage/x.xml",
            "../storage/x.xsl",
            "/storage/x.svg?download=1",
            "/storage/x%2Esvg",
            "/%73torage/x.svg",
            "/STORAGE/x.SVGZ",
            // Dot segments and slashes that only appear after decoding.
            "/x/..%2Fstorage/x.svg",
            "/a%2F..%2Fstorage%2Fx.svg",
            "/%2Fstorage/x.svg",
            "/storage/uploads/%2e%2e/x.html",
            "https://backend3.microshift.verseghy-gimnazium.net/storage/x.xhtml",
            "http://verseghy-gimnazium.net./storage/x.xht",
            "https://verseghy-gimnazium.net:8443/storage/x.htm",
        ] {
            assert_eq!(link_href(&policy(), value), None, "{value}");
        }
        for value in [
            "/storage/uploads/doc.pdf",
            "/storage/uploads/kep.jpg",
            "/storage/x.svg/",
            "/storage/..%2Fx.svg",
            "https://example.org/storage/x.svg",
            "https://example.org/",
            "mailto:titkarsag@example.org",
            "#top",
            "/hirek/12",
        ] {
            assert_eq!(
                link_href(&policy(), value),
                Some(Cow::Borrowed(value)),
                "{value}"
            );
        }
    }

    proptest! {
        #[test]
        fn the_rules_never_panic(value in any::<String>()) {
            let _ = (scheme_admitted(&value), same_origin_relative(&value), is_relative(&value));
            let _ = (image_src(&policy(), &value), embed_src(&policy(), &value), link_href(&policy(), &value));
        }

        #[test]
        fn accepted_image_sources_are_fixed_points(value in "(https://verseghy-gimnazium\\.net)?/storage/uploads/[a-zA-Z0-9 ()%._/-]{0,20}\\.(jpg|png|webp|txt)") {
            if let Some(path) = image_src(&policy(), &value) {
                prop_assert!(path.starts_with(IMAGE_PATH_PREFIX));
                prop_assert_eq!(image_src(&policy(), &path), Some(path.clone()));
                prop_assert_eq!(same_origin_relative(&path), Some(Cow::Borrowed(path.as_str())));
            }
        }

        #[test]
        fn image_sources_are_root_relative(value in any::<String>(), root in any::<bool>()) {
            let policy = if root { SanitizePolicy { image_path_prefix: "/".into(), ..policy() } } else { policy() };
            if let Some(path) = image_src(&policy, &value) {
                prop_assert!(path.starts_with('/') && !path.starts_with("//"), "{}", path);
            }
        }

        #[test]
        fn foreign_hosts_never_give_an_image(host in "[a-z]{1,10}\\.example", scheme in "https?|//") {
            let value = if scheme == "//" { format!("//{host}/storage/uploads/a.jpg") } else { format!("{scheme}://{host}/storage/uploads/a.jpg") };
            prop_assert_eq!(image_src(&policy(), &value), None);
        }

        #[test]
        fn accepted_embeds_are_https_fixed_points(id in "[A-Za-z0-9_-]{1,20}", start in proptest::option::of(0u32..100_000), list in proptest::option::of("[A-Za-z0-9_-]{1,12}")) {
            let query = match (start, &list) {
                (None, None) => String::new(),
                (Some(s), None) => format!("?start={s}"),
                (None, Some(l)) => format!("?list={l}"),
                (Some(s), Some(l)) => format!("?start={s}&list={l}"),
            };
            let url = format!("https://www.youtube-nocookie.com/embed/{id}{query}");
            prop_assert_eq!(embed_src(&policy(), &url), Some(url.clone()));
            let padlet = format!("https://padlet.com/embed/{id}{query}");
            prop_assert_eq!(embed_src(&policy(), &padlet).is_some(), query.is_empty());
        }

        #[test]
        fn root_relative_links_stay_and_scheme_relative_ones_go(path in "[a-z0-9._~-]{0,20}", host in "[a-z]{1,10}\\.example") {
            let local = format!("/{path}");
            prop_assert_eq!(same_origin_relative(&local), Some(Cow::Borrowed(local.as_str())));
            for value in [format!("//{host}/{path}"), format!("/\\{host}/{path}"), format!("\\\\{host}"), format!("//same-origin.invalid/{path}")] {
                prop_assert_eq!(same_origin_relative(&value), None);
            }
        }

        #[test]
        fn link_href_returns_its_input_or_refuses(value in any::<String>(), name in "[a-z0-9]{1,10}", extension in proptest::sample::select(FORBIDDEN_STORAGE_LINK_EXTENSIONS)) {
            if let Some(kept) = link_href(&policy(), &value) {
                prop_assert_eq!(kept, Cow::Borrowed(value.as_str()));
            }
            for document in [format!("/storage/{name}.{extension}"), format!("/{name}/..%2Fstorage/{name}.{extension}"), format!("/%2Fstorage%2F{name}.{extension}")] {
                prop_assert_eq!(link_href(&policy(), &document), None, "{}", document);
            }
        }
    }
}
