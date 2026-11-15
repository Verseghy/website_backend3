//! The images of a normalised body that have no usable alt text
//! (data-migration.md §7.1, "Missing alt (no change)"; editor.md §9).
//!
//! The importer never invents alt text: an image without one is imported
//! as it is. ADR 0013 §2 makes alt text mandatory for the next save, and
//! treats an empty or white-space-only `alt` as missing, so both kinds are
//! listed for editors. "Blank" is decided by
//! [`validate::is_blank`](crate::domain::rich_text::validate::is_blank), the
//! rule the admin write path refuses with `image_alt_missing`, so the list
//! names exactly the images that will block a save.

use scraper::Html;

use super::dom;
use super::report::{AltState, ImageWithoutAlt};
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
