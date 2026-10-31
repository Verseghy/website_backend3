//! libwebp, through webpx: the decode that proves a WebP upload is an image,
//! the lossy re-encode, and the luma that the keep rule compares (ADR 0015
//! §7, amended by ADR 0033 §1).
//!
//! The encoder was chosen by measurement; `docs/design/image-encoders.md`
//! records the candidates, the numbers and the settings. Every call here is
//! computation over bytes in memory: no file, no clock, and no thread, since
//! the decoders are asked for none and the encoder's default uses none, so
//! the module stays in `src/domain/` (ADR 0008).

use webpx::{Decoder, DecoderConfig, Encoder, Limits, Unstoppable};

use super::super::{DecodeDefect, Dimensions, ImageError, ImageFormat, LumaSsim, PixelBudget};

/// The quality of the lossy re-encode. ADR 0015 §7 re-encodes lossy WebP "at
/// the same quality target" as JPEG, quality 85, and libwebp's quality scale
/// is modelled on JPEG's (`cwebp -q`).
pub(super) const QUALITY: f32 = 85.0;

/// libwebp's effort level for the re-encode, 0 (fastest) to 6. Level 4 is
/// libwebp's and `cwebp`'s default; the measurement found that level 6 saves
/// little more for a large cost in CPU time on the node.
pub(super) const METHOD: u8 = 4;

/// The decoder limits for an upload: the pixel budget for one image.
///
/// webpx's other defaults do not apply here. Its 64 MiB input limit would
/// refuse uploads that ADR 0015 §6 allows (up to 128 MiB), and its 16383
/// pixel side limit is libwebp's own bitstream limit, which libwebp checks
/// anyway. Its animation limits are not needed either: each frame is decoded
/// here as a still image, and the frame count and the pixels of all frames
/// are checked before (`MAX_ANIMATION_FRAMES`, `MAX_ANIMATION_PIXELS`).
///
/// - `budget`: the caller's pixel budget.
fn limits(budget: PixelBudget) -> Limits {
    Limits::none().with_max_pixels(budget.max_pixels())
}

/// Decodes a still WebP in full, alpha included, so an image that does not
/// decode is refused.
///
/// libwebp decodes every pixel and scales the result down to one, so no
/// output image is held. A lossy image then needs only libwebp's row buffers
/// and its alpha plane (1 byte per pixel); a lossless one needs libwebp's
/// own copy of its pixels (4 bytes per pixel). `docs/design/image-encoders.md`
/// records the measured peak.
///
/// - `bytes`: a WebP file, or the `ALPH` and bitstream chunks of one frame,
///   which libwebp decodes on their own as its animation decoder does.
/// - `budget`: the pixel budget, which the decoder holds the declared size
///   to before it allocates.
///
/// # Errors
///
/// [`DecodeDefect::WebpData`] when libwebp does not decode the image.
/// libwebp's own reason is not kept: as JPEG's and PNG's decode checks
/// do, the refusal names the decoder that refused the data.
pub(super) fn check_decodes(bytes: &[u8], budget: PixelBudget) -> Result<(), ImageError> {
    let config = DecoderConfig::new().limits(limits(budget)).scale(1, 1);
    Decoder::new(bytes)
        .and_then(|decoder| decoder.config(config).decode_rgba_raw())
        .map(drop)
        .map_err(|_| ImageError::Undecodable {
            format: ImageFormat::WebP,
            defect: DecodeDefect::WebpData,
        })
}

/// The luma plane of a decoded lossy image: the Y plane of its YUV 4:2:0
/// decode, which is the luma that VP8 stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Luma {
    /// The samples, `stride` per row, at least `width` of them used.
    samples: Vec<u8>,
    /// The distance between the starts of two rows.
    stride: usize,
    /// The width and height of the image.
    dimensions: Dimensions,
}

/// Why [`luma_ssim`] gave no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NotMeasured {
    /// A side is below the 11-pixel window of the SSIM, so the similarity
    /// half of the keep rule cannot be shown (ADR 0033 §1).
    TooSmall,
    /// libwebp could not decode its own re-encode, or decoded it to another
    /// size: a failure of the re-encode, for the log.
    Failed(String),
}

/// Decodes a lossy (`VP8 `) bitstream to YUV 4:2:0, which is what VP8
/// stores, and encodes it again at [`QUALITY`]. Going through YUV adds no
/// colour conversion and holds 1.5 bytes per pixel. An `ALPH` chunk is not
/// re-encoded: the caller keeps it, since it is independent of the colour
/// bitstream and already lossless.
///
/// Returns libwebp's output, a simple file with one `VP8 ` chunk, and the
/// luma of the decoded original for [`luma_ssim`]. Only that plane is kept
/// after the encode, 1 byte per pixel; the chroma planes are freed.
///
/// - `bitstream`: the payload of the `VP8 ` chunk, which libwebp reads as
///   a bitstream with no container.
/// - `budget`: the pixel budget.
///
/// # Errors
///
/// libwebp's reason, for the log, when the bitstream does not decode or the
/// encoder fails. The caller has decoded the same bitstream already, so
/// either is a failure of the re-encode, not of the upload.
pub(super) fn re_encode(bitstream: &[u8], budget: PixelBudget) -> Result<(Vec<u8>, Luma), String> {
    let planes = decode_yuv(bitstream, budget)
        .map_err(|error| format!("decoding to YUV failed: {error}"))?;
    let encoded = Encoder::new_yuv((&planes).into())
        .quality(QUALITY)
        .method(METHOD)
        .encode(Unstoppable)
        .map_err(|error| format!("encoding failed: {error}"))?;
    let dimensions = Dimensions::new(planes.width, planes.height)
        .ok_or_else(|| String::from("the decoded image is empty"))?;
    let luma = Luma {
        samples: planes.y,
        stride: planes.y_stride,
        dimensions,
    };
    Ok((encoded, luma))
}

/// The mean luma SSIM of a re-encode against the decoded original, at full
/// resolution, with the SSIM that JPEG's keep rule uses (ADR 0033 §1).
///
/// The re-encode is decoded to YUV, and the two Y planes go into
/// [`LumaSsim`] row by row, so the pass holds the original's luma, the
/// re-encode's planes (1.5 bytes per pixel) and the SSIM's 11 rows of sums.
/// The caller runs it only for a re-encode that passed the size test.
///
/// - `original`: the luma that [`re_encode`] returned.
/// - `bitstream`: the payload of the re-encode's `VP8 ` chunk.
/// - `budget`: the pixel budget.
///
/// # Errors
///
/// [`NotMeasured::TooSmall`] when a side is below 11 pixels, before anything
/// is decoded, and [`NotMeasured::Failed`] when the re-encode does not
/// decode to the original's size.
pub(super) fn luma_ssim(
    original: &Luma,
    bitstream: &[u8],
    budget: PixelBudget,
) -> Result<f64, NotMeasured> {
    let failed = |why: String| NotMeasured::Failed(why);
    let mut ssim = LumaSsim::new(original.dimensions).map_err(|_| NotMeasured::TooSmall)?;
    let planes = decode_yuv(bitstream, budget)
        .map_err(|error| failed(format!("decoding the re-encode failed: {error}")))?;
    if Dimensions::new(planes.width, planes.height) != Some(original.dimensions) {
        return Err(failed("the re-encode decodes to another size".into()));
    }
    // A side of a decoded image is far below usize::MAX on the 64-bit
    // platforms backend3 runs on.
    let width = usize::try_from(original.dimensions.width()).unwrap_or(usize::MAX);
    let rows = original
        .samples
        .chunks(original.stride.max(1))
        .zip(planes.y.chunks(planes.y_stride.max(1)));
    for (a, b) in rows {
        let (Some(a), Some(b)) = (a.get(..width), b.get(..width)) else {
            return Err(failed("a decoded row is shorter than the image".into()));
        };
        ssim.push_row(a, b)
            .map_err(|error| failed(error.to_string()))?;
    }
    ssim.finish().map_err(|error| failed(error.to_string()))
}

/// Decodes a `VP8 ` bitstream to YUV 4:2:0 planes within the budget.
///
/// - `bitstream`: the payload of a `VP8 ` chunk.
/// - `budget`: the pixel budget.
fn decode_yuv(bitstream: &[u8], budget: PixelBudget) -> webpx::Result<webpx::YuvPlanes> {
    Decoder::new(bitstream).and_then(|decoder| {
        decoder
            .config(DecoderConfig::new().limits(limits(budget)))
            .decode_yuv()
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::mean_ssim;
    use super::super::samples::{SIZE, card_of, encode, still, textured};
    use super::*;
    use proptest::{collection::vec, prelude::*};

    /// The `VP8 ` payload of a simple lossy file: everything after the RIFF
    /// header (12 bytes) and the chunk header (8 bytes).
    fn vp8_payload(file: &[u8]) -> &[u8] {
        &file[20..]
    }

    fn dimensions(width: u32, height: u32) -> Dimensions {
        Dimensions::new(width, height).unwrap()
    }

    #[test]
    fn re_encodes_a_lossy_bitstream_at_its_size_and_keeps_its_luma() {
        let original = still(255, false, 75.0);
        let (re_encoded, luma) = re_encode(vp8_payload(&original), PixelBudget::DEFAULT).unwrap();
        let info = Decoder::new(&re_encoded).unwrap().info().clone();
        assert_eq!((info.width, info.height), SIZE);
        assert!(!info.has_alpha);
        assert_eq!(&re_encoded[12..16], b"VP8 ");
        check_decodes(&re_encoded, PixelBudget::DEFAULT).unwrap();
        // The luma is the original's Y plane, one byte per pixel.
        let planes = webpx::decode_yuv(&original).unwrap();
        assert_eq!(luma.dimensions, dimensions(SIZE.0, SIZE.1));
        assert_eq!((luma.samples, luma.stride), (planes.y, planes.y_stride));
        assert!(re_encode(b"not a bitstream", PixelBudget::DEFAULT).is_err());
    }

    #[test]
    fn the_re_encode_is_libwebps_encode_of_the_decoded_planes_at_quality_85_method_4() {
        // ADR 0015 §7 re-encodes lossy WebP "at the same quality target" as
        // JPEG, 85; method 4 is libwebp's default, which the measurement in
        // docs/design/image-encoders.md kept over 6. (Compared as bits, as
        // clippy's float_cmp asks; 85.0 is exact in f32.)
        assert_eq!(QUALITY.to_bits(), 85.0_f32.to_bits());
        assert_eq!(METHOD, 4);
        // Those settings reach the encoder: the output is, byte for byte,
        // what libwebp writes for the decoded planes at them. A textured
        // 64×64 picture, on which another quality or method writes other
        // bytes, so the comparison tells them apart.
        let original = encode(&textured(64), (64, 64), false, 100.0);
        let (re_encoded, _) = re_encode(vp8_payload(&original), PixelBudget::DEFAULT).unwrap();
        let planes = webpx::decode_yuv(&original).unwrap();
        let at = |quality: f32, method: u8| {
            Encoder::new_yuv((&planes).into())
                .quality(quality)
                .method(method)
                .encode(Unstoppable)
                .unwrap()
        };
        assert_eq!(re_encoded, at(85.0, 4));
        for (quality, method) in [(80.0, 4), (90.0, 4), (85.0, 3), (85.0, 6)] {
            assert_ne!(re_encoded, at(quality, method), "q{quality} m{method}");
        }
    }

    #[test]
    fn the_luma_ssim_is_the_shared_mean_ssim_of_the_two_y_planes() {
        let original = still(255, false, 100.0);
        let (re_encoded, luma) = re_encode(vp8_payload(&original), PixelBudget::DEFAULT).unwrap();
        let measured = luma_ssim(&luma, vp8_payload(&re_encoded), PixelBudget::DEFAULT).unwrap();
        // The same value, bit for bit, as the shared function over the two
        // whole planes, which the corpus tests use.
        let plane = |file: &[u8]| {
            let planes = webpx::decode_yuv(file).unwrap();
            let width = usize::try_from(planes.width).unwrap();
            let rows = planes.y.chunks(planes.y_stride).map(|row| &row[..width]);
            rows.flatten().copied().collect::<Vec<u8>>()
        };
        let expected = mean_ssim(&plane(&original), &plane(&re_encoded), luma.dimensions).unwrap();
        assert_eq!(measured.to_bits(), expected.to_bits());
        assert!(measured > 0.9 && measured < 1.0, "{measured}");
        // The original against itself is 1.
        let same = luma_ssim(&luma, vp8_payload(&original), PixelBudget::DEFAULT).unwrap();
        assert!((same - 1.0).abs() < 1e-12, "{same}");
    }

    #[test]
    fn an_image_below_the_window_is_not_measured_before_any_decode() {
        let short = encode(&card_of(16, 10, 255, 0), (16, 10), false, 90.0);
        let (_, luma) = re_encode(vp8_payload(&short), PixelBudget::DEFAULT).unwrap();
        // Not even a bitstream is read: the size alone decides.
        assert_eq!(
            luma_ssim(&luma, b"not a bitstream", PixelBudget::DEFAULT),
            Err(NotMeasured::TooSmall)
        );
    }

    #[test]
    fn a_re_encode_that_does_not_decode_to_the_original_size_fails() {
        let original = still(255, false, 90.0);
        let (_, luma) = re_encode(vp8_payload(&original), PixelBudget::DEFAULT).unwrap();
        let other = encode(&card_of(16, 12, 255, 0), (16, 12), false, 90.0);
        assert_eq!(
            luma_ssim(&luma, vp8_payload(&other), PixelBudget::DEFAULT),
            Err(NotMeasured::Failed(
                "the re-encode decodes to another size".into()
            ))
        );
        assert!(matches!(
            luma_ssim(&luma, b"not a bitstream", PixelBudget::DEFAULT),
            Err(NotMeasured::Failed(why)) if why.starts_with("decoding the re-encode failed")
        ));
    }

    #[test]
    fn the_budget_is_held_by_the_decoder_too() {
        let tight = PixelBudget::new(u64::from(SIZE.0 * SIZE.1) - 1).unwrap();
        let lossless = still(255, true, 75.0);
        assert!(matches!(
            check_decodes(&lossless, tight),
            Err(ImageError::Undecodable {
                format: ImageFormat::WebP,
                defect: DecodeDefect::WebpData
            })
        ));
        let lossy = still(255, false, 75.0);
        assert!(re_encode(vp8_payload(&lossy), tight).is_err());
        let (_, luma) = re_encode(vp8_payload(&lossy), PixelBudget::DEFAULT).unwrap();
        assert!(matches!(
            luma_ssim(&luma, vp8_payload(&lossy), tight),
            Err(NotMeasured::Failed(_))
        ));
        assert_eq!(check_decodes(&lossless, PixelBudget::DEFAULT), Ok(()));
    }

    #[test]
    fn every_kind_of_still_image_decodes_and_garbage_does_not() {
        for (alpha, lossless) in [(255, false), (128, false), (255, true), (128, true)] {
            assert_eq!(
                check_decodes(&still(alpha, lossless, 75.0), PixelBudget::DEFAULT),
                Ok(()),
                "alpha {alpha}, lossless {lossless}"
            );
        }
        assert!(check_decodes(b"RIFF\x04\0\0\0WEBP", PixelBudget::DEFAULT).is_err());
        // The chunks of a frame decode without a container, as libwebp's
        // animation decoder passes them.
        let transparent = still(128, false, 75.0);
        assert_eq!(&transparent[30..34], b"ALPH");
        assert_eq!(
            check_decodes(&transparent[30..], PixelBudget::DEFAULT),
            Ok(())
        );
    }

    proptest! {
        #[test]
        fn damaged_bitstreams_never_panic(
            damage in vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
            lossless in any::<bool>(),
        ) {
            let mut bytes = still(200, lossless, 75.0);
            for (at, byte) in damage {
                let at = at.index(bytes.len());
                bytes[at] = byte;
            }
            let _ = check_decodes(&bytes, PixelBudget::DEFAULT);
            if let Ok((re_encoded, luma)) = re_encode(&bytes, PixelBudget::DEFAULT) {
                prop_assert_eq!(check_decodes(&re_encoded, PixelBudget::DEFAULT), Ok(()));
                if let Ok(ssim) = luma_ssim(&luma, vp8_payload(&re_encoded), PixelBudget::DEFAULT) {
                    prop_assert!((-1.0..=1.0).contains(&ssim));
                }
            }
        }

        #[test]
        fn the_ssim_of_a_re_encode_is_at_most_1_and_symmetric_in_its_planes(
            quality in 50.0..=100.0_f32,
            shift in any::<u8>(),
        ) {
            let original = encode(&super::super::samples::card(255, shift), SIZE, false, quality);
            let (re_encoded, luma) = re_encode(vp8_payload(&original), PixelBudget::DEFAULT).unwrap();
            let ssim = luma_ssim(&luma, vp8_payload(&re_encoded), PixelBudget::DEFAULT).unwrap();
            prop_assert!(ssim <= 1.0 + 1e-12 && ssim > 0.5, "{}", ssim);
        }
    }
}
