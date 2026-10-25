//! The decode check and the re-encode of a stripped JPEG (ADR 0015 §7).
//!
//! Both run in `MozJPEG`, through the `mozjpeg` crate;
//! `docs/design/image-encoders.md` records the measurement that chose it.
//! `MozJPEG` is a fork of libjpeg-turbo, the JPEG library that browsers decode
//! with, so the check reads an upload as the editors' and the visitors'
//! browsers read it. An image that it cannot decode is refused as corrupt,
//! and the colour model (greyscale, YCbCr, RGB, CMYK or YCCK) is the one
//! libjpeg infers from the JFIF and Adobe segments and the component
//! identifiers, not one this module guesses.
//!
//! - **Greyscale and YCbCr** images, which is what cameras, phones and nearly
//!   every encoder write, are re-encoded. They are decoded into their own
//!   colour model, so the samples never pass through RGB and back, and
//!   encoded with `MozJPEG`'s defaults at [`QUALITY`]: progressive, trellis
//!   quantisation, optimised Huffman tables and scans, and 4:2:0 chroma
//!   subsampling for YCbCr, also for a 4:4:4 original, as the maintainer
//!   accepted (ADR 0033 §2).
//! - **RGB, CMYK and YCCK** images are decoded as the check and keep their
//!   stripped original. A re-encode would have to carry the Adobe segment
//!   that tells a decoder how to read them, or convert an RGB image to
//!   YCbCr; these images are rare, mostly from print workflows. ADR 0033 §2
//!   amends ADR 0015 §7, which re-encodes every JPEG, to say so.
//!
//! A re-encode that passes the size half of the keep rule is decoded once
//! more, to its luma only, and compared with the original's luma
//! ([`luma_ssim`]); below the SSIM floor, the stripped original is kept
//! (ADR 0033 §1).
//!
//! The decoded samples of a greyscale or YCbCr image (one or three bytes per
//! pixel) are held whole from the decode check, which reads the whole image
//! before anything is encoded, to the re-encode and its SSIM pass. Those of
//! an image that is not re-encoded are decoded one row at a time and never
//! held. The peak memory of each case is measured in
//! `docs/design/image-encoders.md`.
//!
//! The library works in memory only here (`Decompress::new_mem` and a `Vec`
//! destination) and is deterministic: it reads no file, clock or random
//! source, so this module stays within ADR 0008's rule 1 although the code
//! it calls is C.
//!
//! The re-encode carries the stripped original's metadata segments (JFIF
//! without its thumbnail, Orientation, ICC profile) in place of the ones the
//! encoder writes, so it holds exactly the metadata the strip kept.
//!
//! libjpeg reports a fatal error through a callback that must not return,
//! and the crate leaves the C code by unwinding (`resume_unwind`, which skips
//! the panic hook, so nothing is printed). Every call into the library is
//! therefore wrapped in `catch_unwind`, and a caught unwind becomes an error
//! value: a corrupt upload never panics the request. libjpeg's warnings,
//! for data that is damaged but still decodes, are not errors here, as they
//! are not in browsers.

use std::{
    io,
    panic::{self, AssertUnwindSafe},
};

// With `panic = "abort"`, libjpeg's error unwind would end the process on
// the first corrupt upload, so a build that sets it stops here.
#[cfg(panic = "abort")]
compile_error!("the JPEG re-encode needs panic = \"unwind\": libjpeg reports errors by unwinding");

use mozjpeg::{
    ColorSpace, ColorSpaceExt, Compress, Decompress, Format, decompress::DecompressStarted,
};

use super::{
    super::{DecodeDefect, Dimensions, LumaSsim},
    NotReencoded,
};

/// The quality of the re-encode: ADR 0015 §7's visually lossless target, on
/// libjpeg's 1-100 scale.
pub(super) const QUALITY: u8 = 85;

/// A decoded JPEG, in the colour model the re-encode works in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Decoded {
    /// One 8-bit sample per pixel, row by row.
    Grey(Vec<u8>),
    /// Three 8-bit samples per pixel, Y, Cb and Cr, upsampled to full
    /// resolution, row by row.
    YCbCr(Vec<u8>),
    /// RGB, CMYK or YCCK: decoded as the check only.
    Other,
}

/// Decodes a stripped JPEG with libjpeg, as the check that it decodes and as
/// the input of the re-encode.
///
/// - `stripped`: the output of the strip.
/// - `dimensions`: the stored size its header declares, already held to the
///   pixel budget.
///
/// # Errors
///
/// [`DecodeDefect::JpegData`] when libjpeg refuses the data, or when the
/// size it decodes differs from `dimensions`.
pub(super) fn decode(stripped: &[u8], dimensions: Dimensions) -> Result<Decoded, DecodeDefect> {
    let (width, height) = sides(dimensions);
    let decoded = panic::catch_unwind(AssertUnwindSafe(|| -> io::Result<Option<Decoded>> {
        let decompress = Decompress::new_mem(stripped)?;
        if decompress.size() != (width, height) {
            return Ok(None);
        }
        Ok(Some(match decompress.color_space() {
            ColorSpace::JCS_GRAYSCALE => Decoded::Grey(read(decompress.grayscale()?)?),
            ColorSpace::JCS_YCbCr => {
                Decoded::YCbCr(read(decompress.to_colorspace(ColorSpace::JCS_YCbCr)?)?)
            }
            // libjpeg's default output: RGB for RGB, CMYK for CMYK and YCCK.
            _ => match decompress.image()? {
                Format::RGB(started) | Format::Gray(started) | Format::CMYK(started) => {
                    check(started)?;
                    Decoded::Other
                }
            },
        }))
    }));
    let samples = |decoded: &Decoded| match decoded {
        Decoded::Grey(samples) => samples.len() == width * height,
        Decoded::YCbCr(samples) => samples.len() == 3 * width * height,
        Decoded::Other => true,
    };
    match decoded {
        Ok(Ok(Some(decoded))) if samples(&decoded) => Ok(decoded),
        _ => Err(DecodeDefect::JpegData),
    }
}

/// Re-encodes a decoded JPEG at [`QUALITY`] and gives it `metadata`.
///
/// - `decoded`: the output of [`decode`].
/// - `dimensions`: its stored size.
/// - `metadata`: the metadata segments the strip kept, with their markers.
///
/// # Errors
///
/// [`NotReencoded::ColourModel`] for an RGB, CMYK or YCCK image, and
/// [`NotReencoded::EncoderFailed`] when `MozJPEG` reports an error.
pub(super) fn reencode(
    decoded: &Decoded,
    dimensions: Dimensions,
    metadata: &[u8],
) -> Result<Vec<u8>, NotReencoded> {
    let (colour_space, samples) = match decoded {
        Decoded::Grey(samples) => (ColorSpace::JCS_GRAYSCALE, samples),
        Decoded::YCbCr(samples) => (ColorSpace::JCS_YCbCr, samples),
        Decoded::Other => return Err(NotReencoded::ColourModel),
    };
    let encoded = encode(colour_space, samples, dimensions).ok_or(NotReencoded::EncoderFailed)?;
    with_metadata(&encoded, metadata).ok_or(NotReencoded::EncoderFailed)
}

/// The luma SSIM of a re-encode against the decoded stripped original: the
/// similarity half of the keep rule (ADR 0033 §1).
///
/// - `original`: the stripped original as [`decode`] gave it, whose luma is
///   the grey samples or the Y of each YCbCr pixel.
/// - `reencoded`: the output of [`reencode`].
/// - `dimensions`: the stored size of both.
///
/// The re-encode is decoded to its luma only (libjpeg then skips the
/// colour conversion and the chroma upsampling) one row at a time, and
/// each row goes into [`LumaSsim`] with the original's row, so the pass
/// holds one row of each image and the SSIM's 11 rows of sums, never a
/// second decoded image. It runs after the size test, and only for a
/// re-encode that passed it.
///
/// # Errors
///
/// [`NotReencoded::TooSmallForSsim`] when a side is below the 11-pixel
/// SSIM window; [`NotReencoded::ColourModel`] for an original that was not
/// re-encoded; [`NotReencoded::EncoderFailed`] when libjpeg cannot decode
/// the re-encode, or decodes it to another size. The stripped original is
/// kept in every case.
pub(super) fn luma_ssim(
    original: &Decoded,
    reencoded: &[u8],
    dimensions: Dimensions,
) -> Result<f64, NotReencoded> {
    let ssim = LumaSsim::new(dimensions).map_err(|_| NotReencoded::TooSmallForSsim)?;
    let (samples, stride) = match original {
        Decoded::Grey(samples) => (samples, 1),
        Decoded::YCbCr(samples) => (samples, 3),
        Decoded::Other => return Err(NotReencoded::ColourModel),
    };
    let (width, height) = sides(dimensions);
    if samples.len() != stride * width * height {
        return Err(NotReencoded::EncoderFailed);
    }
    let measured = panic::catch_unwind(AssertUnwindSafe(move || -> io::Result<Option<f64>> {
        let mut ssim = ssim;
        let decompress = Decompress::new_mem(reencoded)?;
        if decompress.size() != (width, height) {
            return Ok(None);
        }
        let mut started = decompress.grayscale()?;
        let mut decoded_row = vec![0_u8; width];
        let mut original_row = vec![0_u8; width];
        for pixels in samples.chunks_exact(stride * width) {
            // One row per call: `decoded_row` holds exactly one scanline.
            started.read_scanlines_into(&mut decoded_row)?;
            for (luma, pixel) in original_row.iter_mut().zip(pixels.chunks_exact(stride)) {
                *luma = pixel[0];
            }
            if ssim.push_row(&original_row, &decoded_row).is_err() {
                return Ok(None);
            }
        }
        started.finish()?;
        Ok(ssim.finish().ok())
    }));
    match measured {
        Ok(Ok(Some(value))) => Ok(value),
        _ => Err(NotReencoded::EncoderFailed),
    }
}

/// Encodes `samples` of `colour_space` with `MozJPEG`'s defaults at
/// [`QUALITY`], or `None` when `MozJPEG` reports an error (for example when
/// `samples` holds fewer rows than `dimensions`).
fn encode(colour_space: ColorSpace, samples: &[u8], dimensions: Dimensions) -> Option<Vec<u8>> {
    let (width, height) = sides(dimensions);
    let encoded = panic::catch_unwind(AssertUnwindSafe(|| -> io::Result<Vec<u8>> {
        // `Compress::new` applies the defaults for the colour space: for
        // YCbCr input, a YCbCr JPEG with 4:2:0 subsampling. The quality is
        // set after them, as libjpeg requires.
        let mut compress = Compress::new(colour_space);
        compress.set_size(width, height);
        compress.set_quality(f32::from(QUALITY));
        let mut started = compress.start_compress(Vec::new())?;
        started.write_scanlines(samples)?;
        started.finish()
    }));
    encoded.ok()?.ok()
}

/// `encoded` with `metadata` in place of the segments the encoder wrote:
/// its SOI, then `metadata`, then everything after its leading `APPn` and
/// COM segments. `None` when `encoded` is not shaped like a JPEG.
fn with_metadata(encoded: &[u8], metadata: &[u8]) -> Option<Vec<u8>> {
    if !encoded.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while let [0xFF, 0xE0..=0xEF | 0xFE, high, low, ..] = *encoded.get(at..)? {
        at += 2 + usize::from(u16::from_be_bytes([high, low]));
    }
    Some([&encoded[..2], metadata, encoded.get(at..)?].concat())
}

/// Reads every scanline of a started decompression and finishes it.
fn read<R>(mut started: DecompressStarted<R>) -> io::Result<Vec<u8>> {
    let samples = started.read_scanlines::<u8>()?;
    started.finish()?;
    Ok(samples)
}

/// Decodes every scanline of a started decompression, one row at a time
/// into the same buffer, and finishes it: the decode check of an image that
/// is not re-encoded. Its decoded pixels are never needed whole, and at the
/// pixel budget they would be the largest allocation of the request (200 MB
/// for a 50-megapixel CMYK image, on top of the decoder's own buffers).
/// libjpeg fails a row it cannot decode, as it does in [`read`].
fn check<R>(mut started: DecompressStarted<R>) -> io::Result<()> {
    let mut row = vec![0_u8; started.width() * started.color_space().num_components()];
    for _ in 0..started.height() {
        // One row per call: `row` holds exactly one scanline.
        started.read_scanlines_into(&mut row)?;
    }
    started.finish()
}

/// The width and height as `usize`. Both are JPEG sides, at most 65,535, so
/// the conversion cannot fail on any platform backend3 runs on.
fn sides(dimensions: Dimensions) -> (usize, usize) {
    let side = |side: u32| usize::try_from(side).unwrap_or(usize::MAX);
    (side(dimensions.width()), side(dimensions.height()))
}

/// `jpeg` with its first scan naming Huffman tables that no DHT segment
/// defines (table 3, for the first component): the marker structure stays
/// sound, so the strip passes it, but libjpeg refuses to decode it. For the
/// tests of this module and of `optimise_jpeg`.
#[cfg(test)]
pub(super) fn undefined_huffman_table(jpeg: &[u8]) -> Vec<u8> {
    let sos = jpeg
        .windows(2)
        .position(|pair| pair == [0xFF, 0xDA])
        .expect("a JPEG with a scan");
    let mut broken = jpeg.to_vec();
    // FF DA, the length (2), the component count (1), the first component's
    // identifier (1), then its DC and AC table numbers in one byte.
    broken[sos + 6] = 0x33;
    broken
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*};

    fn dimensions(width: u32, height: u32) -> Dimensions {
        Dimensions::new(width, height).unwrap()
    }

    /// A 24 × 16 YCbCr image: smooth gradients, Y and Cb along the rows and
    /// Cr down the columns, so the three channels all differ from each other
    /// and a swapped or flattened channel shows.
    fn ycbcr() -> Vec<u8> {
        (0..16_u8)
            .flat_map(|y| (0..24_u8).flat_map(move |x| [x * 8 + y * 2, 64 + x * 4, 200 - y * 6]))
            .collect()
    }

    fn encoded(colour_space: ColorSpace, samples: &[u8]) -> Vec<u8> {
        encode(colour_space, samples, dimensions(24, 16)).unwrap()
    }

    /// The largest difference between `out` and `input` in each of the three
    /// channels of their pixels.
    fn worst_per_channel(out: &[u8], input: &[u8]) -> [u8; 3] {
        out.chunks(3)
            .zip(input.chunks(3))
            .fold([0; 3], |worst, (out, input)| {
                [0, 1, 2].map(|channel| worst[channel].max(out[channel].abs_diff(input[channel])))
            })
    }

    #[test]
    fn ycbcr_round_trips_through_the_encoder() {
        let jpeg = encoded(ColorSpace::JCS_YCbCr, &ycbcr());
        let Ok(Decoded::YCbCr(samples)) = decode(&jpeg, dimensions(24, 16)) else {
            panic!("not decoded as YCbCr");
        };
        assert_eq!(samples.len(), ycbcr().len());
        // Quality 85 with 4:2:0 subsampling keeps every sample of smooth
        // gradients close, in each channel and in its own channel: a Cb and
        // Cr swap, or chroma lost to a flat 128, is off by 30 or more.
        let [luma, cb, cr] = worst_per_channel(&samples, &ycbcr());
        assert!(luma <= 6 && cb <= 8 && cr <= 8, "{luma} {cb} {cr}");
    }

    #[test]
    fn greyscale_stays_greyscale() {
        let grey: Vec<u8> = (0..24 * 16)
            .map(|i| u8::try_from(i % 256).unwrap())
            .collect();
        let jpeg = encoded(ColorSpace::JCS_GRAYSCALE, &grey);
        assert!(
            matches!(decode(&jpeg, dimensions(24, 16)), Ok(Decoded::Grey(samples)) if samples.len() == 24 * 16)
        );
    }

    /// A 24 × 16 JPEG that keeps `colour_space` (RGB or CMYK): libjpeg
    /// writes one, with an Adobe segment, when it is asked to keep the
    /// input's colour space.
    fn kept_colour_space(colour_space: ColorSpace) -> Vec<u8> {
        let samples = vec![90; 24 * 16 * colour_space.num_components()];
        panic::catch_unwind(|| {
            let mut compress = Compress::new(colour_space);
            compress.set_size(24, 16);
            compress.set_color_space(colour_space);
            let mut started = compress.start_compress(Vec::new()).unwrap();
            started.write_scanlines(&samples).unwrap();
            started.finish().unwrap()
        })
        .unwrap()
    }

    #[test]
    fn rgb_and_cmyk_are_decoded_but_not_reencoded() {
        for colour_space in [ColorSpace::JCS_RGB, ColorSpace::JCS_CMYK] {
            let jpeg = kept_colour_space(colour_space);
            let decoded = decode(&jpeg, dimensions(24, 16));
            assert_eq!(decoded, Ok(Decoded::Other), "{colour_space:?}");
            assert_eq!(
                reencode(&Decoded::Other, dimensions(24, 16), b""),
                Err(NotReencoded::ColourModel)
            );
            // The check, which decodes without keeping the rows, still
            // refuses data that libjpeg cannot decode.
            assert_eq!(
                decode(&undefined_huffman_table(&jpeg), dimensions(24, 16)),
                Err(DecodeDefect::JpegData),
                "{colour_space:?}"
            );
        }
    }

    #[test]
    fn a_size_that_differs_from_the_header_is_refused() {
        // For greyscale and YCbCr the sample count would differ too; for
        // RGB and CMYK, whose samples are not kept, the size is the only
        // check, so each colour model is tried.
        let jpegs = [
            encoded(ColorSpace::JCS_YCbCr, &ycbcr()),
            encoded(ColorSpace::JCS_GRAYSCALE, &[50; 24 * 16]),
            kept_colour_space(ColorSpace::JCS_RGB),
            kept_colour_space(ColorSpace::JCS_CMYK),
        ];
        for jpeg in jpegs {
            for other in [dimensions(24, 17), dimensions(23, 16)] {
                assert_eq!(decode(&jpeg, other), Err(DecodeDefect::JpegData));
            }
        }
    }

    #[test]
    fn data_libjpeg_refuses_is_corrupt() {
        let jpeg = encoded(ColorSpace::JCS_YCbCr, &ycbcr());
        assert_eq!(
            decode(&undefined_huffman_table(&jpeg), dimensions(24, 16)),
            Err(DecodeDefect::JpegData)
        );
        assert_eq!(decode(b"", dimensions(24, 16)), Err(DecodeDefect::JpegData));
        assert_eq!(
            decode(&jpeg[..40], dimensions(24, 16)),
            Err(DecodeDefect::JpegData)
        );
    }

    /// The SSIM of two whole planes, through the same streaming measure.
    fn plane_ssim(a: &[u8], b: &[u8], size: Dimensions) -> f64 {
        let width = usize::try_from(size.width()).unwrap();
        let mut ssim = LumaSsim::new(size).unwrap();
        for (a, b) in a.chunks(width).zip(b.chunks(width)) {
            ssim.push_row(a, b).unwrap();
        }
        ssim.finish().unwrap()
    }

    #[test]
    fn the_ssim_compares_the_two_luma_planes() {
        let size = dimensions(24, 16);
        let original = Decoded::YCbCr(ycbcr());
        let reencoded = reencode(&original, size, b"").unwrap();
        let measured = luma_ssim(&original, &reencoded, size).unwrap();
        // The same value from the two Y planes taken apart by hand: the
        // original's from its samples, the re-encode's from a full decode.
        let Ok(Decoded::YCbCr(decoded)) = decode(&reencoded, size) else {
            panic!("the re-encode decodes as YCbCr");
        };
        let luma = |samples: &[u8]| samples.iter().step_by(3).copied().collect::<Vec<u8>>();
        let expected = plane_ssim(&luma(&ycbcr()), &luma(&decoded), size);
        assert_eq!(
            measured.to_bits(),
            expected.to_bits(),
            "{measured} {expected}"
        );
        assert!(measured > 0.9 && measured < 1.0, "{measured}");
        // Against itself, a re-encode scores 1.
        let Ok(again) = decode(&reencoded, size) else {
            panic!("the re-encode decodes");
        };
        assert!((luma_ssim(&again, &reencoded, size).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_ssim_of_a_greyscale_image_uses_its_one_channel() {
        let size = dimensions(24, 16);
        let grey: Vec<u8> = (0..24 * 16_u16)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let original = Decoded::Grey(grey.clone());
        let reencoded = reencode(&original, size, b"").unwrap();
        let Ok(Decoded::Grey(decoded)) = decode(&reencoded, size) else {
            panic!("the re-encode decodes as greyscale");
        };
        assert_eq!(
            luma_ssim(&original, &reencoded, size).map(f64::to_bits),
            Ok(plane_ssim(&grey, &decoded, size).to_bits())
        );
    }

    #[test]
    fn an_image_the_ssim_cannot_measure_keeps_its_stripped_original() {
        // Below the 11-pixel window in either direction.
        let narrow = dimensions(10, 16);
        let samples = vec![9; 3 * 10 * 16];
        let reencoded = encode(ColorSpace::JCS_YCbCr, &samples, narrow).unwrap();
        assert_eq!(
            luma_ssim(&Decoded::YCbCr(samples), &reencoded, narrow),
            Err(NotReencoded::TooSmallForSsim)
        );
        let size = dimensions(24, 16);
        let original = Decoded::YCbCr(ycbcr());
        let reencoded = reencode(&original, size, b"").unwrap();
        assert_eq!(
            luma_ssim(&Decoded::Other, &reencoded, size),
            Err(NotReencoded::ColourModel)
        );
        // A re-encode that does not decode, or decodes to another size, or
        // an original whose samples do not fit the size.
        assert_eq!(
            luma_ssim(&original, &undefined_huffman_table(&reencoded), size),
            Err(NotReencoded::EncoderFailed)
        );
        assert_eq!(
            luma_ssim(&original, b"not a JPEG", size),
            Err(NotReencoded::EncoderFailed)
        );
        let taller = dimensions(24, 17);
        assert_eq!(
            luma_ssim(&Decoded::Grey(vec![0; 24 * 17]), &reencoded, taller),
            Err(NotReencoded::EncoderFailed)
        );
        assert_eq!(
            luma_ssim(&Decoded::YCbCr(vec![0; 10]), &reencoded, size),
            Err(NotReencoded::EncoderFailed)
        );
    }

    #[test]
    fn an_encoder_error_is_not_reencoded() {
        // Fewer rows than the size: libjpeg refuses to finish the image.
        assert_eq!(
            encode(ColorSpace::JCS_GRAYSCALE, &[7; 24], dimensions(24, 16)),
            None
        );
        assert_eq!(
            reencode(&Decoded::Grey(vec![7; 24]), dimensions(24, 16), b""),
            Err(NotReencoded::EncoderFailed)
        );
    }

    #[test]
    fn the_metadata_replaces_the_encoders_segments() {
        let metadata = b"\xFF\xE2\0\x08ICC_\0\0";
        let reencoded = reencode(&Decoded::YCbCr(ycbcr()), dimensions(24, 16), metadata).unwrap();
        let plain = encoded(ColorSpace::JCS_YCbCr, &ycbcr());
        // MozJPEG writes a JFIF segment of 16 bytes after its SOI.
        assert_eq!(&plain[2..6], b"\xFF\xE0\0\x10");
        assert_eq!(reencoded, [&plain[..2], metadata, &plain[20..]].concat());
        assert_eq!(
            with_metadata(b"\xFF\xD8", b"meta"),
            Some(b"\xFF\xD8meta".to_vec())
        );
        assert_eq!(with_metadata(b"\xFF\xD9", b""), None);
        assert_eq!(with_metadata(b"\xFF\xD8\xFF\xE0\0\x10", b""), None);
    }

    proptest! {
        #[test]
        fn arbitrary_data_never_panics(bytes in vec(any::<u8>(), 0..400), width in 1..64_u32, height in 1..64_u32) {
            let jpeg = [&[0xFF, 0xD8, 0xFF][..], &bytes].concat();
            if let Ok(decoded) = decode(&jpeg, dimensions(width, height)) {
                let _ = reencode(&decoded, dimensions(width, height), b"");
            }
        }

        #[test]
        fn damaged_images_never_panic(
            damage in vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
        ) {
            let mut jpeg = encoded(ColorSpace::JCS_YCbCr, &ycbcr());
            for (at, byte) in damage {
                let at = at.index(jpeg.len());
                jpeg[at] = byte;
            }
            if let Ok(decoded) = decode(&jpeg, dimensions(24, 16)) {
                prop_assert!(reencode(&decoded, dimensions(24, 16), b"").is_ok() || decoded == Decoded::Other);
            }
        }

        #[test]
        fn with_metadata_never_panics(encoded in vec(any::<u8>(), 0..64), metadata in vec(any::<u8>(), 0..16)) {
            if let Some(out) = with_metadata(&encoded, &metadata) {
                prop_assert!(out.starts_with(&[0xFF, 0xD8]));
                prop_assert_eq!(&out[2..2 + metadata.len()], metadata.as_slice());
            }
        }
    }
}
