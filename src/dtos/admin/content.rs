//! Wire types of the admin API's rich-text bodies (editor.md §7.6, §7.7).
//!
//! An admin write whose body the server refuses answers `422` with a list
//! of [`ContentError`], the `{code, index, detail}` of editor.md §7.6, one
//! per finding of `domain::rich_text::validate_content` (at most
//! `domain::rich_text::validate::MAX_FINDINGS`, the first ones) or the
//! sanitiser's own refusal (`domain::rich_text::refusal`). The admin shows
//! each one next to the editor as the Hungarian `errors.<code>` message,
//! filled with `index` and the `detail` values (ADR 0021 §4). The
//! handlers that answer with it, and how the list sits in the response,
//! come with the post and page writes (the content write service); this
//! module fixes the entry itself, so the admin's messages and their
//! placeholders are checked against it now.

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;

use super::error::{ErrorCode, ErrorParam};
use crate::domain::rich_text::validate;

/// The largest integer that a JSON reader holds exactly, 2^53 - 1
/// (ADR 0005 §3): a `detail` number never goes above it.
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// One reason the admin API refused a rich-text body. A refused body gets
/// at most 100 of them, the first ones: images, then embeds, each in
/// document order (editor.md §7.6).
///
/// - `code`: the finding; the admin shows `errors.<code>`.
/// - `index`: the 1-based position of the image among the body's images,
///   or of the embed among its embeds, in document order; `null` for a
///   finding about the whole body, or when the server could not trace the
///   position back to the submitted body. The second case cannot happen by
///   construction, but a client that gets it shows the finding without a
///   position: the `errors.<code>` messages of the image and embed codes
///   need `index`, and are not filled with a `null` one.
/// - `detail`: the values that explain the finding and fill the message's
///   placeholders beside `index`. `src` (text) comes with
///   `image_alt_missing` (the source as stored), `image_src_rejected` and
///   `embed_rejected` (as submitted, cut after 100 characters and then
///   ended with `…`), and is absent when there was none; `iframe_content`
///   has no detail. `content_too_large` has `limit` and `actual` (numbers,
///   bytes, which the admin shows as sizes, ADR 0021 §5) and `measured`
///   (text): `input` when the submitted body is over its limit, `output`
///   when what the sanitiser wrote from it is. `content_too_deep` has
///   `limit` (levels) and `line`, and `content_not_normalisable` has
///   `passes`.
// The doc comment above is exported into the TypeScript binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct ContentError {
    /// The stable error code.
    pub code: ErrorCode,
    /// The position of the image or embed, from 1, or `null`.
    pub index: Option<u32>,
    /// Named values that explain the finding; `{}` when there are none.
    #[ts(type = "Record<string, string | number>")]
    pub detail: BTreeMap<String, ErrorParam>,
}

impl From<&validate::ContentError> for ContentError {
    fn from(error: &validate::ContentError) -> Self {
        use validate::ContentError as Finding;
        let mut detail = BTreeMap::new();
        let mut text = |name: &str, value: Option<&String>| {
            if let Some(value) = value {
                detail.insert(name.to_owned(), ErrorParam::Text(value.clone()));
            }
        };
        let code = match error {
            Finding::ImageAltMissing { src, .. } => {
                text("src", src.as_ref());
                ErrorCode::ImageAltMissing
            }
            Finding::ImageSrcRejected { src, .. } => {
                text("src", src.as_ref());
                ErrorCode::ImageSrcRejected
            }
            Finding::EmbedRejected { src, .. } => {
                text("src", src.as_ref());
                ErrorCode::EmbedRejected
            }
            Finding::IframeContent { .. } => ErrorCode::IframeContent,
            Finding::ContentTooLarge {
                measured,
                limit,
                actual,
            } => {
                detail.insert("limit".to_owned(), number(*limit));
                detail.insert("actual".to_owned(), number(*actual));
                detail.insert(
                    "measured".to_owned(),
                    ErrorParam::Text(measured.as_str().to_owned()),
                );
                ErrorCode::ContentTooLarge
            }
            Finding::ContentTooDeep { limit, line } => {
                detail.insert("limit".to_owned(), number(*limit));
                detail.insert("line".to_owned(), number(*line));
                ErrorCode::ContentTooDeep
            }
            Finding::ContentNotNormalisable { passes } => {
                detail.insert("passes".to_owned(), number(*passes));
                ErrorCode::ContentNotNormalisable
            }
        };
        Self {
            code,
            // A body within the input limit (at most 16 MiB, editor.md §7.2)
            // holds fewer than 2^32 images or embeds, so this never
            // saturates.
            index: error
                .index()
                .map(|index| u32::try_from(index.get()).unwrap_or(u32::MAX)),
            detail,
        }
    }
}

/// A `detail` number, held at [`MAX_SAFE_INTEGER`] so a JSON reader gets it
/// exactly. The limits and lines it carries stay far below that.
fn number(value: impl TryInto<i64>) -> ErrorParam {
    ErrorParam::Number(
        value
            .try_into()
            .unwrap_or(MAX_SAFE_INTEGER)
            .min(MAX_SAFE_INTEGER),
    )
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn wire(finding: &validate::ContentError) -> serde_json::Value {
        serde_json::to_value(ContentError::from(finding)).expect("serializes")
    }

    fn at(index: usize) -> Option<NonZeroUsize> {
        NonZeroUsize::new(index)
    }

    #[test]
    fn every_finding_serializes_exactly() {
        use validate::ContentError as Finding;
        let cases = [
            (
                Finding::ImageAltMissing {
                    index: at(3),
                    src: Some("/storage/uploads/a.jpg".into()),
                },
                json!({"code": "image_alt_missing", "index": 3, "detail": {"src": "/storage/uploads/a.jpg"}}),
            ),
            (
                Finding::ImageSrcRejected {
                    index: at(2),
                    src: Some("data:image/png;base64,AA".into()),
                },
                json!({"code": "image_src_rejected", "index": 2, "detail": {"src": "data:image/png;base64,AA"}}),
            ),
            (
                Finding::ImageSrcRejected {
                    index: None,
                    src: None,
                },
                json!({"code": "image_src_rejected", "index": null, "detail": {}}),
            ),
            (
                Finding::EmbedRejected {
                    index: at(1),
                    src: Some("https://evil.example/embed/x".into()),
                },
                json!({"code": "embed_rejected", "index": 1, "detail": {"src": "https://evil.example/embed/x"}}),
            ),
            (
                Finding::IframeContent { index: at(4) },
                json!({"code": "iframe_content", "index": 4, "detail": {}}),
            ),
            (
                Finding::ContentTooLarge {
                    measured: validate::Measured::Input,
                    limit: 2_097_152,
                    actual: 2_097_153,
                },
                json!({"code": "content_too_large", "index": null, "detail": {"actual": 2_097_153, "limit": 2_097_152, "measured": "input"}}),
            ),
            (
                Finding::ContentTooLarge {
                    measured: validate::Measured::Output,
                    limit: 8_388_608,
                    actual: 19_400_000,
                },
                json!({"code": "content_too_large", "index": null, "detail": {"actual": 19_400_000, "limit": 8_388_608, "measured": "output"}}),
            ),
            (
                Finding::ContentTooDeep {
                    limit: 256,
                    line: 12,
                },
                json!({"code": "content_too_deep", "index": null, "detail": {"limit": 256, "line": 12}}),
            ),
            (
                Finding::ContentNotNormalisable { passes: 3 },
                json!({"code": "content_not_normalisable", "index": null, "detail": {"passes": 3}}),
            ),
        ];
        for (finding, expected) in cases {
            assert_eq!(wire(&finding), expected, "{finding:?}");
        }
    }

    #[test]
    fn the_wire_code_is_the_domain_code() {
        use validate::ContentError as Finding;
        for finding in [
            Finding::ImageAltMissing {
                index: None,
                src: None,
            },
            Finding::ImageSrcRejected {
                index: None,
                src: None,
            },
            Finding::EmbedRejected {
                index: None,
                src: None,
            },
            Finding::IframeContent { index: None },
            Finding::ContentTooLarge {
                measured: validate::Measured::Input,
                limit: 1,
                actual: 2,
            },
            Finding::ContentTooLarge {
                measured: validate::Measured::Output,
                limit: 1,
                actual: 2,
            },
            Finding::ContentTooDeep { limit: 1, line: 1 },
            Finding::ContentNotNormalisable { passes: 3 },
        ] {
            let dto = ContentError::from(&finding);
            assert_eq!(wire(&finding)["code"], json!(finding.code()));
            assert_eq!(dto.code.status(), 422, "{finding:?}");
        }
    }

    #[test]
    fn numbers_stay_exact_for_a_json_reader() {
        assert_eq!(number(12_usize), ErrorParam::Number(12));
        assert_eq!(number(u64::MAX), ErrorParam::Number(MAX_SAFE_INTEGER));
        assert_eq!(number(usize::MAX), ErrorParam::Number(MAX_SAFE_INTEGER));
        assert_eq!(
            number(MAX_SAFE_INTEGER.unsigned_abs() + 1),
            ErrorParam::Number(MAX_SAFE_INTEGER)
        );
    }

    #[test]
    fn the_documented_cap_is_the_validators() {
        // The doc comment above, exported into the TypeScript binding, and
        // editor.md §7.6 name the number.
        assert_eq!(validate::MAX_FINDINGS, 100);
    }

    #[test]
    fn the_typescript_declaration() {
        let declaration = ContentError::decl(&ts_rs::Config::default());
        assert!(declaration.contains("code: ErrorCode"), "{declaration}");
        assert!(
            declaration.contains("index: number | null"),
            "{declaration}"
        );
        assert!(
            declaration.contains("detail: Record<string, string | number>"),
            "{declaration}"
        );
    }

    proptest! {
        #[test]
        fn positions_pass_through_unchanged(index in 1u32..=u32::MAX) {
            let position = NonZeroUsize::new(usize::try_from(index).expect("a 64-bit usize"));
            let finding = validate::ContentError::IframeContent { index: position };
            prop_assert_eq!(ContentError::from(&finding).index, Some(index));
        }

        #[test]
        fn limits_pass_through_below_the_safe_bound(limit in 0usize..1 << 40, line in 0u64..1 << 40) {
            let dto = ContentError::from(&validate::ContentError::ContentTooDeep { limit, line });
            prop_assert_eq!(&dto.detail["limit"], &ErrorParam::Number(i64::try_from(limit).expect("small")));
            prop_assert_eq!(&dto.detail["line"], &ErrorParam::Number(i64::try_from(line).expect("small")));
        }

        #[test]
        fn sizes_pass_through_with_what_they_measured(limit in 0usize..1 << 40, actual in 0usize..1 << 40, output in any::<bool>()) {
            let measured = if output { validate::Measured::Output } else { validate::Measured::Input };
            let dto = ContentError::from(&validate::ContentError::ContentTooLarge { measured, limit, actual });
            prop_assert_eq!(dto.detail.len(), 3);
            prop_assert_eq!(&dto.detail["limit"], &ErrorParam::Number(i64::try_from(limit).expect("small")));
            prop_assert_eq!(&dto.detail["actual"], &ErrorParam::Number(i64::try_from(actual).expect("small")));
            prop_assert_eq!(&dto.detail["measured"], &ErrorParam::Text(measured.as_str().to_owned()));
        }

        #[test]
        fn a_source_is_repeated_as_given(src in ".{0,120}") {
            let finding = validate::ContentError::EmbedRejected { index: None, src: Some(src.clone()) };
            prop_assert_eq!(&ContentError::from(&finding).detail["src"], &ErrorParam::Text(src));
        }
    }
}
