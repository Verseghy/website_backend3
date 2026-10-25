//! What optimising a WebP or GIF upload produces, and the two checks that
//! the WebP and GIF optimisers share: the sniffed format, and the totals
//! that bound the decode check of an animation (ADR 0015 §7, amended by
//! ADR 0033 §1 and §4).
//!
//! [`super::optimise_webp`] and [`super::optimise_gif`] return an
//! [`Optimised`] value, so the upload code stores [`Optimised::bytes`] and
//! records the [`Treatment`] in its metrics without knowing which of the two
//! formats it was. They refuse with the [`ImageError`] of every optimiser,
//! with the shared [`ImageError::AnimationTooLarge`] for an animation above
//! [`MAX_ANIMATION_FRAMES`] or [`MAX_ANIMATION_PIXELS`], which also holds an
//! animated PNG. The keep rule of a lossy re-encode is shared with JPEG and
//! lives in `keep.rs`; the reasons a re-encode is not kept are JPEG's
//! [`NotReencoded`], so both formats report them alike.

use super::{
    ImageError, ImageFormat, ImageHeader, MAX_ANIMATION_FRAMES, MAX_ANIMATION_PIXELS, NotReencoded,
    PixelBudget, inspect,
};

/// What an optimiser did to an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Treatment {
    /// The metadata was removed and the image data was copied unchanged, in
    /// a format that is never re-encoded: lossless and animated WebP, and
    /// GIF (ADR 0015 §7, ADR 0033 §3).
    Stripped,
    /// A lossy still WebP was re-encoded, and the re-encode did not pass the
    /// keep rule (ADR 0033 §1), or failed: the stripped original, with its
    /// image data unchanged, is what is stored. [`NotReencoded`] says why.
    ReEncodeDiscarded(NotReencoded),
    /// A lossy still WebP was re-encoded, and the re-encode, at least 5 %
    /// smaller than the stripped original and at a luma SSIM of at least
    /// 0.98 against it, is what is stored.
    ReEncoded,
}

/// An optimised upload, ready to be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Optimised {
    /// The format and the stored dimensions. They are the upload's own, since
    /// optimising never changes either (ADR 0015 §7).
    pub header: ImageHeader,
    /// The bytes to store, written by the parser-based strip or by the
    /// encoder. Of the upload's own bytes, only the image data and the kept
    /// metadata that the strip copies reach them.
    pub bytes: Vec<u8>,
    /// What was done to the upload.
    pub treatment: Treatment,
    /// Why the re-encode failed, for the log: set exactly when
    /// [`Optimised::treatment`] is `ReEncodeDiscarded(EncoderFailed)`, that
    /// is when libwebp could not encode an image that had decoded, or could
    /// not decode its own re-encode for the SSIM. The stripped original is
    /// stored then, as ADR 0015 §7 stores it whenever the re-encode is not
    /// kept. `None` in every other case.
    pub re_encode_failure: Option<String>,
}

/// Whether the frames and pixels of an animation counted so far are within
/// [`MAX_ANIMATION_FRAMES`] and [`MAX_ANIMATION_PIXELS`]. Both limits are
/// inclusive. The WebP and GIF walks call it at every frame they count, so
/// they stop at the first frame that passes a limit (ADR 0033 §4).
///
/// - `format`: the sniffed format, which the refusal names.
/// - `frames`: the frames counted so far, the current one included.
/// - `pixels`: the pixels of those frames, each at its own size.
///
/// # Errors
///
/// [`ImageError::AnimationTooLarge`] when either limit is passed.
pub(super) fn check_animation_size(
    format: ImageFormat,
    frames: u64,
    pixels: u64,
) -> Result<(), ImageError> {
    if frames <= MAX_ANIMATION_FRAMES && pixels <= MAX_ANIMATION_PIXELS {
        Ok(())
    } else {
        Err(ImageError::AnimationTooLarge {
            format,
            frames,
            pixels,
        })
    }
}

/// The header of an upload that must be of `expected` format: [`inspect`],
/// then a check of the sniffed format, as `optimise_jpeg` and `optimise_png`
/// make it.
///
/// - `bytes`: the whole upload.
/// - `budget`: the pixel budget, as [`inspect`] takes it.
/// - `expected`: the format of the optimiser that asks.
///
/// # Errors
///
/// The errors of [`inspect`], and [`ImageError::WrongFormat`] for an image
/// of another format.
pub(super) fn inspect_as(
    bytes: &[u8],
    budget: PixelBudget,
    expected: ImageFormat,
) -> Result<ImageHeader, ImageError> {
    let header = inspect(bytes, budget)?;
    if header.format == expected {
        Ok(header)
    } else {
        Err(ImageError::WrongFormat {
            expected,
            found: header.format,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::header::samples;
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_animation_limits_are_inclusive() {
        let check = |frames, pixels| check_animation_size(ImageFormat::WebP, frames, pixels);
        assert_eq!(check(1, 1), Ok(()));
        assert_eq!(check(0, 0), Ok(()));
        assert_eq!(check(MAX_ANIMATION_FRAMES, MAX_ANIMATION_PIXELS), Ok(()));
        for (frames, pixels) in [(MAX_ANIMATION_FRAMES + 1, 1), (1, MAX_ANIMATION_PIXELS + 1)] {
            for format in [ImageFormat::WebP, ImageFormat::Gif] {
                assert_eq!(
                    check_animation_size(format, frames, pixels),
                    Err(ImageError::AnimationTooLarge {
                        format,
                        frames,
                        pixels
                    })
                );
            }
        }
        // The values are webpx 0.4.1's defaults for untrusted input, which
        // ADR 0033 §4 accepts for WebP, GIF and APNG.
        assert_eq!(MAX_ANIMATION_FRAMES, 4096);
        assert_eq!(MAX_ANIMATION_PIXELS, 268_435_456);
    }

    #[test]
    fn inspect_as_checks_the_sniffed_format() {
        let gif = samples::gif(67, 41);
        assert_eq!(
            inspect_as(&gif, PixelBudget::DEFAULT, ImageFormat::Gif).map(|h| h.format),
            Ok(ImageFormat::Gif)
        );
        assert_eq!(
            inspect_as(&gif, PixelBudget::DEFAULT, ImageFormat::WebP),
            Err(ImageError::WrongFormat {
                expected: ImageFormat::WebP,
                found: ImageFormat::Gif,
            })
        );
        assert_eq!(
            inspect_as(b"<svg/>", PixelBudget::DEFAULT, ImageFormat::Gif),
            Err(ImageError::UnrecognisedFormat)
        );
    }

    proptest! {
        #[test]
        fn an_animation_passes_exactly_when_both_counts_are_within_the_limits(
            frames in 0..2 * MAX_ANIMATION_FRAMES,
            pixels in 0..2 * MAX_ANIMATION_PIXELS,
        ) {
            let within = frames <= MAX_ANIMATION_FRAMES && pixels <= MAX_ANIMATION_PIXELS;
            prop_assert_eq!(
                check_animation_size(ImageFormat::Gif, frames, pixels).is_ok(),
                within
            );
        }
    }
}
