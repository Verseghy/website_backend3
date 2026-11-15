//! The images of a body that the report lists without changing them:
//! those of the normalised body that have no usable alt text
//! (data-migration.md §7.1, "Missing alt (no change)"; editor.md §9), and
//! those whose width over content-v1's bound N7 drops (editor.md §16 item
//! 5).
//!
//! The importer never invents alt text: an image without one is imported
//! as it is. ADR 0013 §2 makes alt text mandatory for the next save, and
//! treats an empty or white-space-only `alt` as missing, so both kinds are
//! listed for editors. "Blank" is decided by
//! [`validate::is_blank`](crate::domain::rich_text::validate::is_blank), the
//! rule the admin write path refuses with `image_alt_missing`, so the list
//! names exactly the images that will block a save.
//!
//! N5b keeps a legacy image's width also above content-v1's bound of
//! 2000 px (`image_layout.rs`), because a value over a limit is reported,
//! never truncated (data-migration.md §5). N7's sanitiser drops such a
//! width, a removal that no step planned, so the import stops on the body
//! until a policy change raises the bound or an `N7-accept` entry lets the
//! width go (data-migration.md §7.1, N7; §7.4, reading (e)). The list,
//! read from the tree that N7 is given, tells the maintainer which images
//! a policy change that raises the bound must cover before the import. It
//! names only the images that N5b places, the ones N7 keeps, and counts
//! their positions among those, which are the positions in N7's output:
//! an image that N7 removes (one without `src`, with a `src` the sanitiser
//! refuses, or inside an element removed with its content) blocks nothing,
//! so it says nothing about the bound.

use scraper::Html;

use super::dom;
use super::image_layout::{Bounds, is_placeable};
use super::report::{AltState, ImageWithoutAlt, WideImage};
use crate::domain::rich_text::SanitizePolicy;
use crate::domain::rich_text::validate::is_blank;

/// Every image of `html` without alt text or with a blank one, in document
/// order.
///
/// - `html`: a parsed body, after every rule of the pass.
pub(super) fn without_alt(html: &Html) -> Vec<ImageWithoutAlt> {
    dom::elements_named(html, "img")
        .into_iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let element = dom::node(html, id).value().as_element()?;
            let alt = match element.attr("alt") {
                None => AltState::Missing,
                Some(alt) if is_blank(alt) => AltState::Blank,
                Some(_) => return None,
            };
            Some(ImageWithoutAlt {
                position: index + 1,
                src: element.attr("src").map(shown_src),
                alt,
            })
        })
        .collect()
}

/// Every image of `html` that N5b places and whose `style` declares a width
/// above content-v1's bound, in document order, positioned among the
/// images that N5b places, as [`without_alt`] positions the images of N7's
/// output. The width is read as N5b reads it ([`Bounds::styled_width`]), so
/// after N5b, which writes every kept width into the `style`, an image is
/// listed exactly when its legacy width was over the bound. An image N5b
/// leaves for N7 to remove is neither listed nor counted (see the module
/// documentation).
///
/// - `html`: a parsed body, after every rule of the pass and before N7.
pub(super) fn wider_than_bound(html: &Html) -> Vec<WideImage> {
    let bounds = Bounds::of_policy();
    let policy = SanitizePolicy::content_v1();
    dom::elements_named(html, "img")
        .into_iter()
        .map(|id| dom::node(html, id))
        .filter(|&node| is_placeable(node, &policy))
        .enumerate()
        .filter_map(|(index, node)| {
            let element = node.value().as_element()?;
            let width = bounds.styled_width(element.attr("style")?)?;
            let src = element.attr("src")?;
            (width > bounds.max_width).then(|| WideImage {
                position: index + 1,
                src: shown_src(src),
                width,
            })
        })
        .collect()
}

/// `src` as the report shows it: a `data:` URL (ASCII case-insensitive
/// scheme) cut after its first comma, the end of its header, with `…`
/// appended; any other value whole.
///
/// - `src`: an image's `src`, with character references decoded.
pub(super) fn shown_src(src: &str) -> String {
    let is_data = src
        .as_bytes()
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(b"data:"));
    match src.find(',') {
        Some(comma) if is_data && comma + 1 < src.len() => format!("{}…", &src[..=comma]),
        _ => src.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn images(body: &str) -> Vec<ImageWithoutAlt> {
        without_alt(&dom::parse(body))
    }

    #[test]
    fn missing_and_blank_alts_are_listed_by_position() {
        let listed = images(concat!(
            r#"<p><img src="/storage/uploads/a.jpg" alt="Kép"></p>"#,
            r#"<p><img src="/storage/uploads/b.jpg"></p>"#,
            r#"<p><img src="/storage/uploads/c.jpg" alt=""></p>"#,
            r#"<p><img src="/storage/uploads/d.jpg" alt=" &nbsp;"></p>"#,
            r#"<table><tr><td><IMG ALT=x></td></tr></table><img>"#,
        ));
        assert_eq!(
            listed,
            [
                ImageWithoutAlt {
                    position: 2,
                    src: Some("/storage/uploads/b.jpg".to_owned()),
                    alt: AltState::Missing,
                },
                ImageWithoutAlt {
                    position: 3,
                    src: Some("/storage/uploads/c.jpg".to_owned()),
                    alt: AltState::Blank,
                },
                ImageWithoutAlt {
                    position: 4,
                    src: Some("/storage/uploads/d.jpg".to_owned()),
                    alt: AltState::Blank,
                },
                ImageWithoutAlt {
                    position: 6,
                    src: None,
                    alt: AltState::Missing,
                },
            ]
        );
    }

    #[test]
    fn only_html_images_count() {
        // An SVG `image` is not an `img`; a legacy `<image>` start tag is
        // one, because the parser renames it.
        let listed = images(r#"<svg><image href="x"></image></svg><image src="y">"#);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].position, 1);
        assert_eq!(listed[0].src.as_deref(), Some("y"));
    }

    #[test]
    fn images_wider_than_the_bound_are_listed_by_position() {
        use crate::domain::rich_text::css::values::Hundredths;
        let listed = wider_than_bound(&dom::parse(concat!(
            r#"<p><img src="/storage/uploads/a.jpg" style="width: 2000px"></p>"#,
            r#"<p><img src="/storage/uploads/b.jpg" style="display: block; width: 2000.01px"></p>"#,
            r#"<p><img src="/storage/uploads/c.jpg" width="2400"></p>"#,
            r#"<p><img src="/storage/uploads/e.png" style="width: 2400px; width: 1600pt"></p>"#,
            r#"<p><img src="/storage/uploads/d.jpg" style="width: 3000px; width: 50%"></p>"#,
        )));
        assert_eq!(
            listed,
            [
                WideImage {
                    position: 2,
                    src: "/storage/uploads/b.jpg".to_owned(),
                    width: Hundredths::from_hundredths(200_001),
                },
                // 1600 pt is 2133.33 px; the last declaration counts.
                WideImage {
                    position: 4,
                    src: "/storage/uploads/e.png".to_owned(),
                    width: Hundredths::from_hundredths(213_333),
                },
            ],
            "a width at the bound, one only in the attribute and one that a later declaration replaces are not listed"
        );
    }

    #[test]
    fn images_that_n7_removes_are_not_listed_as_wide() {
        // They block nothing, so they say nothing about the bound: no
        // `src`, a `src` the sanitiser refuses, and an image inside an
        // element it removes with its content. N7 removes them, so they do
        // not count for the position of the next image either: the
        // positions are those of N7's output.
        let listed = wider_than_bound(&dom::parse(concat!(
            r#"<p><img style="width: 2400px"></p>"#,
            r#"<p><img src="https://example.com/a.jpg" style="width: 2400px"></p>"#,
            r#"<p><img src="" style="width: 2400px"></p>"#,
            r#"<p><img src="data:image/png;base64,iVBO" style="width: 2400px"></p>"#,
            r#"<object><img src="/storage/uploads/a.jpg" style="width: 2400px"></object>"#,
            r#"<template><img src="/storage/uploads/a.jpg" style="width: 2400px"></template>"#,
            r#"<p><img src="https://backend.verseghy-gimnazium.net/storage/uploads/f.jpg" style="width: 2400px"></p>"#,
        )));
        // The dead host's image is N6's to rewrite, and N7 keeps it then.
        assert_eq!(
            listed
                .iter()
                .map(|image| (image.position, image.src.as_str()))
                .collect::<Vec<_>>(),
            [(
                1,
                "https://backend.verseghy-gimnazium.net/storage/uploads/f.jpg"
            )]
        );
    }

    #[test]
    fn data_urls_are_cut_after_their_header() {
        assert_eq!(
            shown_src("data:image/jpeg;base64,/9j/4AAQSkZJRg"),
            "data:image/jpeg;base64,…"
        );
        assert_eq!(shown_src("DATA:image/png,AA"), "DATA:image/png,…");
        assert_eq!(
            shown_src("data:image/png;base64,"),
            "data:image/png;base64,"
        );
        assert_eq!(shown_src("data:image/png"), "data:image/png");
        assert_eq!(shown_src("/storage/a,b.jpg"), "/storage/a,b.jpg");
        assert_eq!(shown_src(""), "");
    }

    proptest! {
        // An image is listed exactly when its alt is missing or blank, and
        // the positions count every image.
        #[test]
        fn exactly_the_images_without_usable_alt_are_listed(alts in prop::collection::vec(proptest::option::of(prop::sample::select(&["", " ", "\t", "Kép", "a b", "&nbsp;"][..])), 0..8)) {
            let body: String = alts
                .iter()
                .map(|alt| match alt {
                    Some(alt) => format!(r#"<img src="/storage/uploads/a.jpg" alt="{alt}">"#),
                    None => r#"<img src="/storage/uploads/a.jpg">"#.to_owned(),
                })
                .collect();
            let expected: Vec<_> = alts
                .iter()
                .enumerate()
                .filter_map(|(index, alt)| {
                    let state = match alt {
                        None => AltState::Missing,
                        Some(alt) if is_blank(&alt.replace("&nbsp;", "\u{a0}")) => AltState::Blank,
                        Some(_) => return None,
                    };
                    Some((index + 1, state))
                })
                .collect();
            let listed: Vec<_> = images(&body).into_iter().map(|image| (image.position, image.alt)).collect();
            prop_assert_eq!(listed, expected);
        }

        // A shown source is never longer than the source plus the ellipsis,
        // and starts like it.
        #[test]
        fn a_shown_source_is_a_prefix(src in "(data:|DATA:|/storage/)?\\PC{0,40}") {
            let shown = shown_src(&src);
            let kept = shown.strip_suffix('…').unwrap_or(&shown);
            prop_assert!(src.starts_with(kept), "{:?} {:?}", src, shown);
            prop_assert!(shown == src || shown.ends_with(",…"));
        }
    }
}
