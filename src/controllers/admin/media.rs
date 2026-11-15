//! What every admin upload endpoint shares (ADR 0015 §6 and §7, ADR 0021
//! §4): the per-process upload handles, the reading of the one file part of
//! a multipart request, and the answer to every upload error.
//!
//! An upload endpoint is a handler of `controllers::admin::routes()`, mounted
//! after the JSON body-limit layer so that the 128 MiB request limit of
//! `server.middlewares.limit_payload` applies to it. It takes
//! `Result<Multipart, MultipartRejection>` last among its arguments and runs:
//!
//! ```text
//! let uploads = media_uploads(&ctx)?;
//! let received = match receive_image_part(multipart, &uploads.media, &uploads.images).await {
//!     Ok(received) => received,
//!     Err(error) => return upload_error_response(&error),
//! };
//! let stored = models::media::store_entity_image(
//!     received, EntityImageFolder::AuthorImages, models::media::random_name_bytes()?,
//!     &uploads.media, &uploads.images, &uploads.gate,
//! ).await;
//! // on error: return upload_error_response(&error)
//! ```
//!
//! and then stores `stored.stored_value` on its entity in its own
//! transaction: the value in the shape of the legacy rows (`<32 hex>.<ext>`,
//! or `index/<32 hex>.<ext>` for a post's index image), not the path below
//! the root. The admin's URL of the image is `/storage/` and `stored.path`
//! (ADR 0015 §3). The first endpoint is the author image; this module has
//! no route of its own.
//!
//! The request is one multipart part named `file` ([`FILE_PART`]), with the
//! client's file name, which gives the extension. A request that is not
//! multipart, has another first part, or has a second part is refused as
//! `invalid_body`, and the staged upload is removed. The client's content
//! type of the part is never read: the magic bytes decide.

use axum::{
    Json,
    extract::{
        Multipart,
        multipart::{MultipartError, MultipartRejection},
    },
    http::StatusCode,
};
use futures_util::StreamExt as _;
use loco_rs::prelude::*;

use crate::{
    domain::{
        image_optimise::{ImageError, MAX_ANIMATION_FRAMES, MAX_ANIMATION_PIXELS},
        media_rules::MediaRuleError,
    },
    dtos::admin::error::{ErrorCode, ErrorEnvelope, ErrorParam},
    models::media::{self, BodyError, ReceivedImage, UploadError, UploadGate, UploadRefusal},
    settings::{ImageOptimisationSettings, MediaSettings, Settings},
};

/// The name of the multipart part that carries the uploaded file. The image
/// refusals name it as their `field`, so the admin shows the message at the
/// file input (ADR 0021 §4).
pub const FILE_PART: &str = "file";

/// The start of the English description of an upload's `invalid_body`;
/// the reason follows it, for example `: the request has more than one
/// part`.
pub const INVALID_UPLOAD_BODY: &str =
    "The request body is not one multipart/form-data part named `file`";

/// The per-process handles of uploads: the typed settings that the upload
/// model takes and the one optimiser permit (ADR 0015 §7).
///
/// `App::after_context` builds it once and puts it into the shared store;
/// handlers read it with [`media_uploads`] and pass its parts to
/// `models::media` (ADR 0008 rules 2 and 3).
#[derive(Debug, Clone)]
pub struct MediaUploads {
    /// `settings.media`.
    pub media: MediaSettings,
    /// `settings.image_optimisation`.
    pub images: ImageOptimisationSettings,
    /// The process's optimiser permit.
    pub gate: UploadGate,
}

impl MediaUploads {
    /// The handles for `settings`, with a new optimiser permit.
    ///
    /// - `settings`: the validated settings of the process.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            media: settings.media.clone(),
            images: settings.image_optimisation,
            gate: UploadGate::new(),
        }
    }
}

/// The upload handles of this app context.
///
/// - `ctx`: the request's app context.
///
/// # Errors
///
/// [`Error::Message`] when the context carries none: every context Loco
/// builds has them (`App::after_context`), one built by hand does not.
pub fn media_uploads(ctx: &AppContext) -> Result<MediaUploads> {
    ctx.shared_store.get::<MediaUploads>().ok_or_else(|| {
        Error::string(
            "the upload handles are missing from the shared store; App::after_context \
             registers them, so this context was not built by App::boot",
        )
    })
}

/// Reads the one file part of an upload request into the staging directory
/// (`models::media::receive_image`), and refuses anything around it.
///
/// - `multipart`: the request's multipart extractor, or why axum refused it
///   (not `multipart/form-data`, no boundary).
/// - `media`: `settings.media`.
/// - `images`: `settings.image_optimisation`, whose pixel budget refuses an
///   image from its header while it arrives.
///
/// # Errors
///
/// The errors of `receive_image`, and [`UploadError::InvalidBody`] when the
/// request is not multipart, its first part is not named [`FILE_PART`], or
/// another part follows it; [`UploadError::BodyTooLarge`] when the body
/// passes the request body limit, wherever that happens. The staged upload
/// is removed after every error.
pub async fn receive_image_part(
    multipart: Result<Multipart, MultipartRejection>,
    media: &MediaSettings,
    images: &ImageOptimisationSettings,
) -> Result<ReceivedImage, UploadError> {
    let mut multipart =
        multipart.map_err(|rejection| UploadError::InvalidBody(rejection.body_text()))?;
    let field = multipart
        .next_field()
        .await
        .map_err(|error| multipart_error(&error))?
        .ok_or_else(|| UploadError::InvalidBody("the request has no part".to_owned()))?;
    if field.name() != Some(FILE_PART) {
        return Err(UploadError::InvalidBody(format!(
            "the first part must be the file part `{FILE_PART}`"
        )));
    }
    let file_name = field.file_name().map(str::to_owned);
    let chunks = field.map(|chunk| chunk.map_err(|error| body_error(&error)));
    let received = media::receive_image(media, images, file_name.as_deref(), chunks).await?;
    // One file per request: a second part refuses the whole request, and
    // dropping `received` removes its staging file.
    match multipart.next_field().await {
        Ok(None) => Ok(received),
        Ok(Some(_)) => Err(UploadError::InvalidBody(
            "the request has more than one part".to_owned(),
        )),
        Err(error) => Err(multipart_error(&error)),
    }
}

/// The [`BodyError`] of a multipart error: axum gives the body-limit
/// refusal, and only that one, the status 413 (axum 0.8
/// `status_code_from_multer_error`), so the status is the test, as in
/// `json_rejection_response`.
///
/// - `error`: what the multipart reader reported.
fn body_error(error: &MultipartError) -> BodyError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        BodyError::TooLarge
    } else {
        BodyError::Unreadable(error.body_text())
    }
}

/// The [`UploadError`] of a multipart error between parts.
///
/// - `error`: what the multipart reader reported.
fn multipart_error(error: &MultipartError) -> UploadError {
    body_error(error).into()
}

/// The answer to an upload error: the admin error envelope with its status
/// for everything the editor or the operator can act on, and Loco's `500`
/// for a failure inside backend3.
///
/// - `error`: why the upload did not end with a stored file.
///
/// Every refusal is logged with its English reason, so a refused file can be
/// traced in the log (`the GIF has no image`, `a WebP animation sets reserved
/// VP8X flags`, ...); the editor sees the Hungarian message of the code.
///
/// # Errors
///
/// [`Error::Message`] for [`UploadError::Storage`], [`UploadError::Optimiser`]
/// and [`UploadError::Random`]: Loco logs it at `ERROR` level with the
/// message, which names the setting and the path, and answers `500` without
/// it.
pub fn upload_error_response(error: &UploadError) -> Result<Response> {
    let Some(envelope) = upload_error_envelope(error) else {
        return Err(Error::Message(error.to_string()));
    };
    match error {
        UploadError::Refused(refusal) => {
            tracing::info!(code = ?envelope.error, reason = %refusal, "upload refused");
        }
        UploadError::InsufficientSpace { .. } => {
            tracing::warn!(reason = %error, "upload refused: the media volume is nearly full");
        }
        _ => tracing::debug!(reason = %error, "upload request refused"),
    }
    // Every status `ErrorCode::status` returns is a valid status code; the
    // fallback only guards against a mistake in that table.
    let status =
        StatusCode::from_u16(envelope.error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    Ok((status, Json(envelope)).into_response())
}

/// The admin error envelope of an upload error, or `None` for a failure
/// inside backend3 (ADR 0021 §4). The mapping of [`upload_error_response`].
///
/// - `error`: why the upload did not end with a stored file.
///
/// | Error | Code | `params` |
/// |---|---|---|
/// | extension not allowed, content no image | `image_not_allowed` | |
/// | content of another format | `image_type_mismatch` | `extension`, `format` |
/// | header or data that does not decode, cut short | `image_corrupt` | `format` |
/// | above the pixel budget | `image_too_many_pixels` | `width`, `height`, `max_megapixels` |
/// | animation above the totals of ADR 0033 §4 | `image_animation_too_large` | `max_frames`, `max_megapixels` |
/// | JPEG above the scan cap of ADR 0033 §4 | `image_too_many_scans` | `max_scans` |
/// | free space below the reserve | `insufficient_storage` | |
/// | body above the request body limit | `body_too_large` | |
/// | not one readable file part | `invalid_body` | |
///
/// The image refusals carry [`FILE_PART`] as their `field` and the English
/// reason as their `description`. An `invalid_body` carries
/// [`INVALID_UPLOAD_BODY`] and the reason as its description, because the
/// code's default description speaks of JSON, and script clients and logs
/// read it (ADR 0021 §4). The others carry their code's default
/// description. None of these three has a field, like the JSON body
/// refusals: they are errors of the whole request.
#[must_use]
pub fn upload_error_envelope(error: &UploadError) -> Option<ErrorEnvelope> {
    match error {
        UploadError::Refused(refusal) => {
            let (code, params) = refusal_code(*refusal);
            let mut envelope = ErrorEnvelope::new(code);
            envelope.description = refusal.to_string();
            envelope.field = Some(FILE_PART.to_owned());
            envelope.params = params
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect();
            Some(envelope)
        }
        UploadError::InsufficientSpace { .. } => {
            Some(ErrorEnvelope::new(ErrorCode::InsufficientStorage))
        }
        UploadError::BodyTooLarge => Some(ErrorEnvelope::new(ErrorCode::BodyTooLarge)),
        UploadError::InvalidBody(reason) => {
            let mut envelope = ErrorEnvelope::new(ErrorCode::InvalidBody);
            envelope.description = format!("{INVALID_UPLOAD_BODY}: {reason}");
            Some(envelope)
        }
        UploadError::Storage { .. } | UploadError::Optimiser(_) | UploadError::Random(_) => None,
    }
}

/// The code and the message parameters of a refusal.
///
/// - `refusal`: why the file is refused.
fn refusal_code(refusal: UploadRefusal) -> (ErrorCode, Vec<(&'static str, ErrorParam)>) {
    let text = |value: &str| ErrorParam::Text(value.to_owned());
    match refusal {
        UploadRefusal::Rule(
            MediaRuleError::ExtensionNotAllowed | MediaRuleError::UnrecognisedContent,
        )
        | UploadRefusal::Image {
            error: ImageError::UnrecognisedFormat,
            ..
        } => (ErrorCode::ImageNotAllowed, Vec::new()),
        UploadRefusal::Rule(MediaRuleError::TypeMismatch { extension, found })
        | UploadRefusal::Image {
            extension,
            error: ImageError::WrongFormat { found, .. },
        } => (
            ErrorCode::ImageTypeMismatch,
            vec![
                ("extension", text(extension.as_str())),
                ("format", text(found.name())),
            ],
        ),
        UploadRefusal::Image {
            error:
                ImageError::Truncated { format }
                | ImageError::Corrupt { format, .. }
                | ImageError::Undecodable { format, .. },
            ..
        } => (
            ErrorCode::ImageCorrupt,
            vec![("format", text(format.name()))],
        ),
        UploadRefusal::Image {
            error:
                ImageError::OverPixelBudget {
                    dimensions, budget, ..
                },
            ..
        } => (
            ErrorCode::ImageTooManyPixels,
            vec![
                ("width", ErrorParam::Number(i64::from(dimensions.width()))),
                ("height", ErrorParam::Number(i64::from(dimensions.height()))),
                ("max_megapixels", megapixels(budget.max_pixels())),
            ],
        ),
        UploadRefusal::Image {
            error: ImageError::AnimationTooLarge { .. },
            ..
        } => (
            ErrorCode::ImageAnimationTooLarge,
            vec![
                ("max_frames", count(MAX_ANIMATION_FRAMES)),
                ("max_megapixels", megapixels(MAX_ANIMATION_PIXELS)),
            ],
        ),
        UploadRefusal::Image {
            error: ImageError::TooManyScans { limit },
            ..
        } => (
            ErrorCode::ImageTooManyScans,
            vec![("max_scans", count(u64::try_from(limit).unwrap_or(u64::MAX)))],
        ),
    }
}

/// A pixel count as whole megapixels, rounded down, so the message never
/// promises more than the limit allows: 50,000,000 is 50, and the
/// 268,435,456 pixels of an animation are 268.
///
/// - `pixels`: the limit in pixels.
fn megapixels(pixels: u64) -> ErrorParam {
    count(pixels / 1_000_000)
}

/// A count as a message parameter. Every count here is far below 2^53
/// (ADR 0005 §3); a larger one would show as the largest `i64`.
///
/// - `value`: the count.
fn count(value: u64) -> ErrorParam {
    ErrorParam::Number(i64::try_from(value).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use axum::body::to_bytes;
    use serde_json::{Value, json};

    use super::*;
    use crate::domain::{
        image_optimise::{
            DecodeDefect, Dimensions, HeaderDefect, ImageFormat, MAX_JPEG_SCANS, PixelBudget,
        },
        media_rules::ImageExtension,
    };

    /// The envelope of `error` as JSON.
    fn envelope_json(error: &UploadError) -> Option<Value> {
        upload_error_envelope(error).map(|envelope| serde_json::to_value(envelope).expect("JSON"))
    }

    fn refused(refusal: UploadRefusal) -> UploadError {
        UploadError::Refused(refusal)
    }

    fn image(extension: ImageExtension, error: ImageError) -> UploadError {
        refused(UploadRefusal::Image { extension, error })
    }

    #[test]
    fn content_that_is_no_image_is_not_allowed() {
        for error in [
            refused(UploadRefusal::Rule(MediaRuleError::ExtensionNotAllowed)),
            refused(UploadRefusal::Rule(MediaRuleError::UnrecognisedContent)),
            image(ImageExtension::Png, ImageError::UnrecognisedFormat),
        ] {
            let envelope = upload_error_envelope(&error).expect("an envelope");
            assert_eq!(envelope.error, ErrorCode::ImageNotAllowed, "{error}");
            assert_eq!(envelope.field.as_deref(), Some(FILE_PART));
            assert!(envelope.params.is_empty());
        }
        assert_eq!(
            envelope_json(&refused(UploadRefusal::Rule(
                MediaRuleError::UnrecognisedContent
            ))),
            Some(json!({
                "error": "image_not_allowed",
                "description": "the file content is not a JPEG, PNG, GIF or WebP image",
                "field": "file",
                "params": {}
            }))
        );
    }

    #[test]
    fn a_type_mismatch_names_the_extension_and_the_format() {
        let expected = json!({ "extension": "png", "format": "JPEG" });
        for error in [
            refused(UploadRefusal::Rule(MediaRuleError::TypeMismatch {
                extension: ImageExtension::Png,
                found: ImageFormat::Jpeg,
            })),
            image(
                ImageExtension::Png,
                ImageError::WrongFormat {
                    expected: ImageFormat::Png,
                    found: ImageFormat::Jpeg,
                },
            ),
        ] {
            let json = envelope_json(&error).expect("an envelope");
            assert_eq!(json["error"], "image_type_mismatch", "{error}");
            assert_eq!(json["params"], expected, "{error}");
        }
    }

    #[test]
    fn every_decode_refusal_is_corrupt_with_its_reason() {
        for (error, format) in [
            (
                ImageError::Truncated {
                    format: ImageFormat::Gif,
                },
                "GIF",
            ),
            (
                ImageError::Corrupt {
                    format: ImageFormat::Jpeg,
                    defect: HeaderDefect::ZeroDimension,
                },
                "JPEG",
            ),
            (
                ImageError::Undecodable {
                    format: ImageFormat::Gif,
                    defect: DecodeDefect::GifNoImage,
                },
                "GIF",
            ),
            (
                ImageError::Undecodable {
                    format: ImageFormat::WebP,
                    defect: DecodeDefect::WebpReservedFlags,
                },
                "WebP",
            ),
            // An APNG cut short inside its animation (D111: refused as corrupt).
            (
                ImageError::Undecodable {
                    format: ImageFormat::Png,
                    defect: DecodeDefect::PngTruncated,
                },
                "PNG",
            ),
        ] {
            let json = envelope_json(&image(ImageExtension::Gif, error)).expect("an envelope");
            assert_eq!(json["error"], "image_corrupt", "{error}");
            assert_eq!(json["params"], json!({ "format": format }), "{error}");
            // The English reason reaches the description and the log.
            assert_eq!(json["description"], error.to_string());
        }
    }

    #[test]
    fn the_size_and_cpu_limits_have_codes_of_their_own() {
        let over_budget = image(
            ImageExtension::Jpg,
            ImageError::OverPixelBudget {
                format: ImageFormat::Jpeg,
                dimensions: Dimensions::new(8_688, 5_792).expect("non-zero"),
                budget: PixelBudget::DEFAULT,
            },
        );
        let json = envelope_json(&over_budget).expect("an envelope");
        assert_eq!(json["error"], "image_too_many_pixels");
        assert_eq!(
            json["params"],
            json!({ "width": 8_688, "height": 5_792, "max_megapixels": 50 })
        );

        for format in [ImageFormat::WebP, ImageFormat::Gif, ImageFormat::Png] {
            let animation = image(
                ImageExtension::Webp,
                ImageError::AnimationTooLarge {
                    format,
                    frames: 4_097,
                    pixels: 1,
                },
            );
            let json = envelope_json(&animation).expect("an envelope");
            assert_eq!(json["error"], "image_animation_too_large", "{format}");
            assert_eq!(
                json["params"],
                json!({ "max_frames": 4_096, "max_megapixels": 268 })
            );
        }

        let scans = image(
            ImageExtension::Jpeg,
            ImageError::TooManyScans {
                limit: MAX_JPEG_SCANS,
            },
        );
        let json = envelope_json(&scans).expect("an envelope");
        assert_eq!(json["error"], "image_too_many_scans");
        assert_eq!(json["params"], json!({ "max_scans": 100 }));
    }

    #[test]
    fn the_request_errors_carry_no_field() {
        for (error, code) in [
            (
                UploadError::InsufficientSpace {
                    available: 1,
                    needed: 2,
                    min_free: 3,
                },
                ErrorCode::InsufficientStorage,
            ),
            (UploadError::BodyTooLarge, ErrorCode::BodyTooLarge),
        ] {
            assert_eq!(
                upload_error_envelope(&error),
                Some(ErrorEnvelope::new(code))
            );
        }
    }

    #[test]
    fn an_invalid_upload_body_says_why_without_speaking_of_json() {
        let json = envelope_json(&UploadError::InvalidBody(
            "the request has more than one part".to_owned(),
        ));
        assert_eq!(
            json,
            Some(json!({
                "error": "invalid_body",
                "description": "The request body is not one multipart/form-data part named \
                                `file`: the request has more than one part",
                "field": null,
                "params": {}
            }))
        );
        assert!(INVALID_UPLOAD_BODY.contains(&format!("`{FILE_PART}`")));
        assert!(!INVALID_UPLOAD_BODY.contains("JSON"));
    }

    #[test]
    fn a_failure_inside_backend3_is_no_envelope() {
        for error in [
            UploadError::Storage {
                action: "cannot open the media root",
                key: "settings.media.root",
                path: PathBuf::from("/data/app/public"),
                source: io::Error::from(io::ErrorKind::NotFound),
            },
            UploadError::Optimiser("panicked".to_owned()),
            UploadError::Random("no entropy".to_owned()),
        ] {
            assert_eq!(upload_error_envelope(&error), None, "{error}");
            let response = upload_error_response(&error).expect_err("an internal error");
            assert!(matches!(response, Error::Message(message) if message == error.to_string()));
        }
    }

    #[tokio::test]
    async fn a_response_has_the_code_s_status_and_the_envelope() {
        for (error, status) in [
            (
                refused(UploadRefusal::Rule(MediaRuleError::ExtensionNotAllowed)),
                422,
            ),
            (
                UploadError::InsufficientSpace {
                    available: 0,
                    needed: 1,
                    min_free: 1,
                },
                507,
            ),
            (UploadError::BodyTooLarge, 413),
            (UploadError::InvalidBody("x".to_owned()), 422),
        ] {
            let expected = serde_json::to_vec(&upload_error_envelope(&error).expect("an envelope"))
                .expect("JSON");
            let response = upload_error_response(&error).expect("a response");
            assert_eq!(response.status().as_u16(), status, "{error}");
            let body = to_bytes(response.into_body(), 4096).await.expect("body");
            assert_eq!(body, expected);
        }
    }

    #[test]
    fn megapixels_round_down() {
        assert_eq!(megapixels(50_000_000), ErrorParam::Number(50));
        assert_eq!(megapixels(49_999_999), ErrorParam::Number(49));
        assert_eq!(megapixels(999_999), ErrorParam::Number(0));
        assert_eq!(count(u64::MAX), ErrorParam::Number(i64::MAX));
    }
}
