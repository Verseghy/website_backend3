//! JPEG optimisation (ADR 0015 §7, amended by ADR 0033): a lossless
//! metadata strip, then a re-encode at quality 85 that is kept only when it
//! is at least 5 % smaller than the stripped original and reaches a luma
//! SSIM of 0.98 against it.
//!
//! [`optimise_jpeg`] runs five steps, each refusing what it cannot accept:
//!
//! 1. [`inspect`]: the format must be JPEG and the header within the pixel
//!    budget, before anything is decoded.
//! 2. The strip (`strip`): the marker structure is rewritten without
//!    the metadata ADR 0015 §7 removes; the compressed image data is copied
//!    unchanged. Only the ICC profile, the EXIF Orientation tag and the JFIF
//!    segment (without its thumbnail) are kept, plus the part of the Adobe
//!    segment that the decoder reads. The strip also counts the scans and
//!    refuses an image with more than [`MAX_JPEG_SCANS`], still before any
//!    decode (ADR 0033 §4).
//! 3. The decode check (`reencode`): the stripped JPEG is decoded with
//!    libjpeg, as browsers decode it; an image that does not decode is
//!    refused as corrupt.
//! 4. The re-encode: a greyscale or YCbCr image is encoded again at quality
//!    85 with the stripped original's metadata.
//! 5. The keep rule (ADR 0033 §1): [`reencode_is_kept`] compares the sizes;
//!    only a re-encode that passes is decoded once more, to its luma, and
//!    [`reencode_meets_ssim_floor`] holds its SSIM against the original's
//!    luma to [`SSIM_FLOOR`](super::SSIM_FLOOR). The re-encode is stored
//!    only when both pass; otherwise the stripped original is.
//!
//! The stored pixels are never turned, scaled or cropped, so the format, the
//! width and the height stay those of the upload, and the kept Orientation
//! tag still shows the image the way it showed before. The result is never
//! larger than the upload: the strip only removes or shortens segments, and
//! a re-encode is kept only when it is smaller than the stripped original.

mod reencode;
mod strip;

use super::{
    DecodeDefect, ImageError, ImageFormat, ImageHeader, PixelBudget, inspect, reencode_is_kept,
    reencode_meets_ssim_floor,
};

/// The most scans (SOS segments) a JPEG upload may have; one more is
/// refused before any decode, as [`ImageError::TooManyScans`] (ADR 0033 §4).
///
/// The value is set by measurement, as the maintainer's answer asks
/// (decision D79), and the maintainer kept it when asked to choose between
/// 100, 500 and 64: "Keep 100, as built" (decision D101, 2026-10-26). It is
/// libjpeg-turbo's own limit for the scan scripts that its cjpeg and
/// jpegtran read (`MAX_SCANS`, `libjpeg-turbo@36956793:src/rdswitch.c`), so
/// no progressive file those tools can write is refused. The cap is checked
/// by the strip, before libjpeg runs.
///
/// - **Real files are far below it.** libjpeg's progressive script writes 10
///   scans for a colour image, `MozJPEG` wrote 4 to 11 for every re-encode
///   measured, and a sequential JPEG holds each component in one scan, four
///   at most.
/// - **What it bounds.** libjpeg passes over every block of the image for
///   each scan, whatever data the scan holds, so a scan costs time in
///   proportion to the pixels rather than to its bytes. The costliest scans
///   measured at the 50-megapixel budget, AC refinement scans of EOB runs,
///   take about 27 ms each; at this cap they add at most about 2.8 CPU
///   seconds to an upload.
/// - **Why not 500.** libjpeg-turbo's `TurboJPEG` API refuses more than 500
///   scans when asked to refuse "an unreasonably large number of scans"
///   (`TJFLAG_LIMITSCANS`, `src/turbojpeg.c`). That would allow about 14 CPU
///   seconds here, more on the node's slower cores, while a second upload
///   waits behind the one-at-a-time semaphore within the same 120 s
///   `/api/admin` timeout (`docs/design/image-encoders.md`, "The scan cap").
pub const MAX_JPEG_SCANS: usize = 100;

/// A JPEG upload after optimisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimisedJpeg {
    /// The bytes to store: a JPEG of the same format and stored dimensions,
    /// never larger than the upload.
    pub bytes: Vec<u8>,
    /// The header of the upload, which is also the header of `bytes`.
    pub header: ImageHeader,
    /// Which of the two candidates `bytes` is.
    pub kept: JpegKept,
}

/// Which bytes [`optimise_jpeg`] kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JpegKept {
    /// The re-encode at quality 85: at least 5 % smaller than the stripped
    /// original, with a luma SSIM of at least 0.98 against it.
    Reencoded,
    /// The stripped original, with its compressed data unchanged.
    Stripped(NotReencoded),
}

/// Why a JPEG keeps its stripped original.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotReencoded {
    /// The re-encode was not at least 5 % smaller (ADR 0015 §7). This is
    /// the usual case for an image that was already compressed hard, at or
    /// below quality 85.
    NotSmallerEnough,
    /// The re-encode was small enough, but its luma SSIM against the
    /// original stayed below 0.98 (ADR 0033 §1): the usual case for a
    /// photograph of fine texture, such as foliage, fur or water, saved at
    /// a high quality. Storing it would lose detail the editor can see.
    BelowSsimFloor,
    /// The image is narrower or shorter than the 11-pixel window of the
    /// SSIM, so the similarity half of the keep rule cannot be shown; such
    /// a tiny image saves next to nothing anyway.
    TooSmallForSsim,
    /// The image is RGB, CMYK or YCCK, which the re-encode does not
    /// reproduce; it was decoded as the check only (ADR 0033 §2).
    ColourModel,
    /// `MozJPEG` reported an error while encoding or while decoding the
    /// re-encode for its SSIM, for example when memory ran out. The stripped
    /// original is still a correct result, so the upload is not refused for
    /// a failure of the encoder.
    EncoderFailed,
}

/// Optimises a JPEG upload: strips its metadata, checks that it decodes,
/// and keeps a re-encode at quality 85 when it is at least 5 % smaller and
/// reaches the SSIM floor.
///
/// - `bytes`: the whole upload.
/// - `budget`: the largest pixel count the caller is prepared to decode
///   (from its settings; [`PixelBudget::DEFAULT`] is ADR 0015 §7's 50
///   megapixels). An image above it is refused before any decode.
///
/// It is CPU-bound, so the caller runs it on a blocking thread, one at a
/// time (ADR 0015 §7); the SSIM pass runs inside it, on the same thread.
/// Besides `bytes`, which the caller holds, it holds the stripped copy (at
/// most the size of `bytes`) and the decoder's and encoder's buffers. These
/// depend on the pixels: about 10 bytes per pixel for a colour re-encode, up
/// to 12.5 for a file of extreme detail, 8 for a progressive CMYK or YCCK
/// image, which is only decoded, and less for the rest. The SSIM pass needs
/// less than the encoder before it, and runs after the encoder has freed
/// its buffers. At the default budget that is up to about 730 MiB beyond
/// `bytes` (`docs/design/image-encoders.md` has the measurements).
///
/// # Errors
///
/// The errors of [`inspect`]; [`ImageError::WrongFormat`] when the upload
/// is another image format; [`ImageError::TooManyScans`] when it has more
/// than [`MAX_JPEG_SCANS`] scans; and [`ImageError::Undecodable`] when its
/// segments are malformed or cut short, or libjpeg cannot decode it.
pub fn optimise_jpeg(bytes: &[u8], budget: PixelBudget) -> Result<OptimisedJpeg, ImageError> {
    let (header, stripped) = stripped_upload(bytes, budget)?;
    let dimensions = header.dimensions;
    let decoded = reencode::decode(&stripped.bytes, dimensions).map_err(undecodable)?;
    let verdict =
        reencode::reencode(&decoded, dimensions, &stripped.metadata).and_then(|reencoded| {
            // The size first: it is free, and the SSIM pass decodes the
            // re-encode again, so it runs only for a re-encode that could be
            // kept.
            if !reencode_is_kept(stripped.bytes.len(), reencoded.len()) {
                return Err(NotReencoded::NotSmallerEnough);
            }
            let ssim = reencode::luma_ssim(&decoded, &reencoded, dimensions)?;
            if reencode_meets_ssim_floor(ssim) {
                Ok(reencoded)
            } else {
                Err(NotReencoded::BelowSsimFloor)
            }
        });
    let (bytes, kept) = match verdict {
        Ok(reencoded) => (reencoded, JpegKept::Reencoded),
        Err(reason) => (stripped.bytes, JpegKept::Stripped(reason)),
    };
    Ok(OptimisedJpeg {
        bytes,
        header,
        kept,
    })
}

/// Strips the metadata of a JPEG upload losslessly: the header check, the
/// scan cap and the strip of [`optimise_jpeg`], without its decode check
/// and re-encode.
///
/// Uploads go through [`optimise_jpeg`], which refuses an image that does
/// not decode; this function alone does not decode anything. It is public
/// for the corpus tests, which compare a re-encode with the stripped
/// original, as ADR 0015 §7 asks, and check that the strip leaves every
/// decoded pixel as it was.
///
/// - `bytes`: the whole upload.
/// - `budget`: the pixel budget, as for [`optimise_jpeg`].
///
/// # Errors
///
/// The errors of [`optimise_jpeg`], except [`ImageError::Undecodable`] for
/// data that libjpeg cannot decode: that is found only by the decode.
pub fn strip_jpeg(bytes: &[u8], budget: PixelBudget) -> Result<Vec<u8>, ImageError> {
    stripped_upload(bytes, budget).map(|(_, stripped)| stripped.bytes)
}

/// The header check and the strip that [`optimise_jpeg`] and [`strip_jpeg`]
/// share, with the parameters of [`optimise_jpeg`].
fn stripped_upload(
    bytes: &[u8],
    budget: PixelBudget,
) -> Result<(ImageHeader, strip::Stripped), ImageError> {
    let header = inspect(bytes, budget)?;
    if header.format != ImageFormat::Jpeg {
        return Err(ImageError::WrongFormat {
            expected: ImageFormat::Jpeg,
            found: header.format,
        });
    }
    let stripped = strip::strip(bytes, MAX_JPEG_SCANS).map_err(|refusal| match refusal {
        strip::Refusal::Defect(defect) => undecodable(defect),
        strip::Refusal::TooManyScans => ImageError::TooManyScans {
            limit: MAX_JPEG_SCANS,
        },
    })?;
    Ok((header, stripped))
}

/// The error for a JPEG whose data is wrong in the way `defect` says.
fn undecodable(defect: DecodeDefect) -> ImageError {
    ImageError::Undecodable {
        format: ImageFormat::Jpeg,
        defect,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Dimensions, header::samples};
    use super::*;
    use mozjpeg::{ColorSpace, Compress};
    use proptest::{collection::vec, prelude::*, sample::Index};

    /// A `width × height` YCbCr JPEG from libjpeg at `quality`, with
    /// `segments` after its SOI, of a picture with fine texture: detailed
    /// enough that a re-encode of a high-quality original is smaller, and
    /// the reverse for a low one, while a quality-85 re-encode stays below
    /// the SSIM floor, as one of a photograph of foliage does.
    fn jpeg(width: u16, height: u16, quality: f32, segments: &[u8]) -> Vec<u8> {
        encoded(width, height, quality, segments, |x, y| {
            x * 7 + y * 3 + (x * y) % 23
        })
    }

    /// As [`jpeg`], of a smooth picture whose quality-85 re-encode is
    /// smaller and reaches the SSIM floor: shaded soft shapes, as in a
    /// photograph of a sky or a wall.
    fn smooth_jpeg(width: u16, height: u16, quality: f32, segments: &[u8]) -> Vec<u8> {
        encoded(width, height, quality, segments, |x, y| {
            40 + x + y + (x / 16 + y / 16) % 2 * 30
        })
    }

    /// The JPEG of [`jpeg`] and [`smooth_jpeg`]: `luma(x, y)` (modulo 256)
    /// as the luma of each pixel, with a fixed colour.
    fn encoded(
        width: u16,
        height: u16,
        quality: f32,
        segments: &[u8],
        luma: impl Fn(usize, usize) -> usize,
    ) -> Vec<u8> {
        let (w, h) = (usize::from(width), usize::from(height));
        let samples: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [u8::try_from(luma(x, y) % 256).unwrap(), 110, 150]
            })
            .collect();
        let encoded = std::panic::catch_unwind(|| {
            let mut compress = Compress::new(ColorSpace::JCS_YCbCr);
            compress.set_fastest_defaults();
            compress.set_size(w, h);
            compress.set_quality(quality);
            let mut started = compress.start_compress(Vec::new()).unwrap();
            started.write_scanlines(&samples).unwrap();
            started.finish().unwrap()
        })
        .unwrap();
        [&encoded[..2], segments, &encoded[2..]].concat()
    }

    /// An EXIF segment with orientation 6 and an invented GPS latitude.
    fn camera_exif() -> Vec<u8> {
        let mut tiff = b"MM\0\x2A\0\0\0\x08\0\x02".to_vec();
        tiff.extend(b"\x01\x12\0\x03\0\0\0\x01\0\x06\0\0");
        tiff.extend(b"\x88\x25\0\x04\0\0\0\x01\0\0\0\x26");
        tiff.extend([0; 4]);
        tiff.extend(b"\0\x01\0\x02\0\x02\0\0\0\x02N\0\0\0\0\0");
        let payload = [b"Exif\0\0".as_slice(), &tiff].concat();
        [
            &[0xFF, 0xE1],
            &u16::try_from(payload.len() + 2).unwrap().to_be_bytes()[..],
            &payload,
        ]
        .concat()
    }

    fn optimise(bytes: &[u8]) -> Result<OptimisedJpeg, ImageError> {
        optimise_jpeg(bytes, PixelBudget::DEFAULT)
    }

    #[test]
    fn a_high_quality_original_is_reencoded_and_keeps_its_orientation() {
        let upload = smooth_jpeg(96, 64, 98.0, &camera_exif());
        let result = optimise(&upload).unwrap();
        assert_eq!(result.kept, JpegKept::Reencoded);
        assert!(result.bytes.len() * 20 <= upload.len() * 19);
        assert_eq!(
            inspect(&result.bytes, PixelBudget::DEFAULT),
            Ok(result.header)
        );
        assert_eq!(result.header.dimensions, Dimensions::new(96, 64).unwrap());
        // The re-encode carries the orientation and nothing else of the EXIF.
        let exif = b"\xFF\xE1\0\x22Exif\0\0II\x2A\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        assert_eq!(&result.bytes[2..38], exif);
    }

    /// The re-encode that [`optimise_jpeg`] judges for `upload`, its size
    /// verdict and its SSIM, through the module's own steps.
    fn candidate(upload: &[u8]) -> (bool, f64) {
        let stripped = strip::strip(upload, MAX_JPEG_SCANS).unwrap();
        let size = inspect(upload, PixelBudget::DEFAULT).unwrap().dimensions;
        let decoded = reencode::decode(&stripped.bytes, size).unwrap();
        let reencoded = reencode::reencode(&decoded, size, &stripped.metadata).unwrap();
        let ssim = reencode::luma_ssim(&decoded, &reencoded, size).unwrap();
        (
            reencode_is_kept(stripped.bytes.len(), reencoded.len()),
            ssim,
        )
    }

    #[test]
    fn a_reencode_below_the_ssim_floor_keeps_the_stripped_original() {
        // Fine texture at quality 98: the re-encode saves more than 5 %, so
        // the SSIM alone decides, and it stays below 0.98.
        let upload = jpeg(96, 64, 98.0, &camera_exif());
        let (smaller, ssim) = candidate(&upload);
        assert!(smaller && ssim < 0.98, "{smaller} {ssim}");
        let result = optimise(&upload).unwrap();
        assert_eq!(
            result.kept,
            JpegKept::Stripped(NotReencoded::BelowSsimFloor)
        );
        assert_eq!(Ok(result.bytes), strip_jpeg(&upload, PixelBudget::DEFAULT));
        // The smooth picture's re-encode passes both halves.
        let (smaller, ssim) = candidate(&smooth_jpeg(96, 64, 98.0, &camera_exif()));
        assert!(smaller && ssim >= 0.98, "{smaller} {ssim}");
    }

    #[test]
    fn an_image_narrower_than_the_ssim_window_keeps_the_stripped_original() {
        // 10 pixels wide: one column short of the window. Its re-encode is
        // small enough, so only the missing SSIM keeps it from being stored.
        let upload = jpeg(10, 64, 100.0, &camera_exif());
        let result = optimise(&upload).unwrap();
        assert_eq!(
            result.kept,
            JpegKept::Stripped(NotReencoded::TooSmallForSsim)
        );
        assert_eq!(Ok(result.bytes), strip_jpeg(&upload, PixelBudget::DEFAULT));
        let at_window = jpeg(11, 64, 100.0, &camera_exif());
        assert_ne!(
            optimise(&at_window).unwrap().kept,
            JpegKept::Stripped(NotReencoded::TooSmallForSsim)
        );
    }

    /// `progressive` with its last scan repeated until it has `scans` scans,
    /// as a crafted file repeats scans. libjpeg warns about the repeated
    /// refinement but decodes it, as it decodes such a file in a browser.
    fn with_scans(progressive: &[u8], scans: usize) -> Vec<u8> {
        let markers: Vec<usize> = progressive
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| *pair == [0xFF, 0xDA])
            .map(|(at, _)| at)
            .collect();
        let last = *markers.last().unwrap();
        let eoi = progressive.len() - 2;
        assert!(scans >= markers.len());
        let extra = progressive[last..eoi].repeat(scans - markers.len());
        [&progressive[..eoi], &extra, &progressive[eoi..]].concat()
    }

    /// A small progressive JPEG, as `MozJPEG` writes one by default.
    fn progressive() -> Vec<u8> {
        let samples = vec![120_u8; 3 * 32 * 24];
        std::panic::catch_unwind(|| {
            let mut compress = Compress::new(ColorSpace::JCS_YCbCr);
            compress.set_size(32, 24);
            compress.set_quality(90.0);
            let mut started = compress.start_compress(Vec::new()).unwrap();
            started.write_scanlines(&samples).unwrap();
            started.finish().unwrap()
        })
        .unwrap()
    }

    #[test]
    fn the_scan_cap_is_the_value_adr_0033_sets() {
        // ADR 0033 §4 fixes the value; changing it is a rule change, which
        // the ADR records first.
        assert_eq!(MAX_JPEG_SCANS, 100);
    }

    #[test]
    fn scans_up_to_the_cap_are_optimised_and_one_more_is_refused() {
        let at_cap = with_scans(&progressive(), MAX_JPEG_SCANS);
        assert!(optimise(&at_cap).is_ok());
        let above = with_scans(&progressive(), MAX_JPEG_SCANS + 1);
        let refused = ImageError::TooManyScans {
            limit: MAX_JPEG_SCANS,
        };
        assert_eq!(optimise(&above), Err(refused));
        assert_eq!(strip_jpeg(&above, PixelBudget::DEFAULT), Err(refused));
        assert_eq!(
            refused.to_string(),
            format!(
                "the JPEG image has more than {MAX_JPEG_SCANS} scans, the most the optimiser decodes"
            )
        );
    }

    #[test]
    fn a_fill_byte_in_the_scan_data_passes_the_strip_and_keeps_the_pixels() {
        // libjpeg reads `FF FF 00` as one stuffed `FF` (T.81 B.1.1.2 allows
        // fill bytes before a marker, and its bit reader skips them), so a
        // fill byte before a stuffed zero is part of the scan: the strip
        // keeps it in place, and the image decodes to the same pixels.
        let upload = jpeg(96, 64, 90.0, &[]);
        let scan = upload
            .windows(2)
            .position(|pair| pair == [0xFF, 0xDA])
            .unwrap();
        let stuffed = scan
            + upload[scan..]
                .windows(2)
                .position(|pair| pair == [0xFF, 0x00])
                .expect("a stuffed zero in the scan data");
        let filled = [&upload[..stuffed], &[0xFF], &upload[stuffed..]].concat();
        let original = strip::strip(&upload, MAX_JPEG_SCANS).unwrap().bytes;
        let stripped = strip::strip(&filled, MAX_JPEG_SCANS).unwrap().bytes;
        // The strip changes only what comes before the scan, so the fill
        // byte sits as far from the end as it did in the upload.
        let at = original.len() - (upload.len() - stuffed);
        assert_eq!(
            stripped,
            [&original[..at], &[0xFF], &original[at..]].concat()
        );
        let size = inspect(&upload, PixelBudget::DEFAULT).unwrap().dimensions;
        let pixels = reencode::decode(&original, size);
        assert!(matches!(pixels, Ok(reencode::Decoded::YCbCr(_))));
        assert_eq!(reencode::decode(&stripped, size), pixels);
    }

    #[test]
    fn a_low_quality_original_keeps_its_stripped_bytes() {
        let upload = jpeg(96, 64, 20.0, &camera_exif());
        let result = optimise(&upload).unwrap();
        assert_eq!(
            result.kept,
            JpegKept::Stripped(NotReencoded::NotSmallerEnough)
        );
        assert!(result.bytes.len() < upload.len());
        // The compressed data is the upload's, byte for byte.
        let data = |jpeg: &[u8]| {
            jpeg[jpeg
                .windows(2)
                .position(|pair| pair == [0xFF, 0xDB])
                .unwrap()..]
                .to_vec()
        };
        assert_eq!(data(&result.bytes), data(&upload));
    }

    #[test]
    fn other_formats_and_bad_headers_are_refused() {
        assert_eq!(
            optimise(&samples::png(4, 4)),
            Err(ImageError::WrongFormat {
                expected: ImageFormat::Jpeg,
                found: ImageFormat::Png
            })
        );
        assert_eq!(optimise(b"<svg/>"), Err(ImageError::UnrecognisedFormat));
        let over = optimise_jpeg(&samples::jpeg(10_000, 5_001), PixelBudget::DEFAULT);
        assert!(matches!(over, Err(ImageError::OverPixelBudget { .. })));
    }

    #[test]
    fn a_header_without_data_does_not_decode() {
        // The header check passes a header that stops at the first scan.
        assert_eq!(
            optimise(&samples::jpeg(67, 41)),
            Err(ImageError::Undecodable {
                format: ImageFormat::Jpeg,
                defect: DecodeDefect::JpegCutShort
            })
        );
        let mut garbage = samples::jpeg(67, 41);
        garbage.extend([0, 0, 0xFF, 0xD9]);
        assert!(matches!(
            optimise(&garbage),
            Err(ImageError::Undecodable {
                format: ImageFormat::Jpeg,
                ..
            })
        ));
    }

    #[test]
    fn an_image_that_libjpeg_cannot_decode_is_refused_after_the_strip() {
        // Sound segments, so the strip passes it, but a scan that names
        // Huffman tables no DHT segment defines: only the decode finds it.
        let upload = reencode::undefined_huffman_table(&jpeg(96, 64, 90.0, &camera_exif()));
        assert!(strip_jpeg(&upload, PixelBudget::DEFAULT).is_ok());
        assert_eq!(
            optimise(&upload),
            Err(ImageError::Undecodable {
                format: ImageFormat::Jpeg,
                defect: DecodeDefect::JpegData
            })
        );
    }

    #[test]
    fn strip_jpeg_gives_the_stripped_candidate_and_the_same_refusals() {
        let upload = jpeg(96, 64, 20.0, &camera_exif());
        let result = optimise(&upload).unwrap();
        assert_eq!(
            result.kept,
            JpegKept::Stripped(NotReencoded::NotSmallerEnough)
        );
        assert_eq!(strip_jpeg(&upload, PixelBudget::DEFAULT), Ok(result.bytes));
        // A re-encoded upload: its stripped original is larger than what is
        // stored, and holds the same metadata.
        let high = smooth_jpeg(96, 64, 98.0, &camera_exif());
        let reencoded = optimise(&high).unwrap();
        assert_eq!(reencoded.kept, JpegKept::Reencoded);
        let stripped = strip_jpeg(&high, PixelBudget::DEFAULT).unwrap();
        assert!(reencoded.bytes.len() < stripped.len());
        assert_eq!(stripped[..38], reencoded.bytes[..38]);
        for refused in [
            samples::png(4, 4),
            samples::jpeg(67, 41),
            samples::jpeg(10_000, 5_001),
            b"<svg/>".to_vec(),
        ] {
            assert_eq!(
                strip_jpeg(&refused, PixelBudget::DEFAULT),
                optimise(&refused).map(|result| result.bytes)
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn the_result_is_never_larger_and_keeps_the_header(
            width in 8..80_u16,
            height in 8..80_u16,
            quality in 30..=100_u8,
        ) {
            let upload = jpeg(width, height, f32::from(quality), &camera_exif());
            let result = optimise(&upload).unwrap();
            prop_assert!(result.bytes.len() <= upload.len());
            prop_assert_eq!(inspect(&result.bytes, PixelBudget::DEFAULT), Ok(result.header));
            prop_assert_eq!(result.header.dimensions, Dimensions::new(width.into(), height.into()).unwrap());
            // Optimising again strips nothing more.
            let again = optimise(&result.bytes).unwrap();
            prop_assert!(again.bytes.len() <= result.bytes.len());
        }

        #[test]
        fn damaged_uploads_never_panic(
            damage in vec((any::<Index>(), any::<u8>()), 1..6),
            cut in any::<Index>(),
        ) {
            let mut upload = jpeg(40, 24, 90.0, &camera_exif());
            for (at, byte) in damage {
                let at = at.index(upload.len());
                upload[at] = byte;
            }
            upload.truncate(cut.index(upload.len() + 1).max(upload.len() / 2));
            let stripped = strip_jpeg(&upload, PixelBudget::DEFAULT);
            match (optimise(&upload), stripped) {
                (Ok(result), Ok(stripped)) => {
                    prop_assert!(result.bytes.len() <= stripped.len());
                    prop_assert!(stripped.len() <= upload.len());
                    prop_assert_eq!(inspect(&result.bytes, PixelBudget::DEFAULT).map(|header| header.format), Ok(ImageFormat::Jpeg));
                    if let JpegKept::Stripped(_) = result.kept {
                        prop_assert_eq!(result.bytes, stripped);
                    }
                }
                (Ok(_), Err(error)) => prop_assert!(false, "stored although the strip refuses it: {error}"),
                // The same refusal before the decode, or the decode's own.
                (Err(error), Ok(_)) => prop_assert_eq!(error, undecodable(DecodeDefect::JpegData)),
                (Err(error), Err(strip_error)) => prop_assert_eq!(error, strip_error),
            }
        }
    }
}
