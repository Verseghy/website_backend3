//! PNG uploads: lossless recompression that keeps only the colour, HDR and
//! animation chunks and the EXIF Orientation (ADR 0015 §7, as ADR 0033 §5
//! and §6 amend it).
//!
//! [`optimise_png`] runs in two steps:
//!
//! 1. **The chunk walk** ([`chunks`]) rewrites the upload from the parts that
//!    ADR 0015 §7 and ADR 0033 §6 keep: the image (`IHDR`, `PLTE`, `tRNS`
//!    and the image data), the colour chunks `iCCP`, `sRGB`, `gAMA` and
//!    `cHRM`, the HDR colour chunks `cICP`, `mDCV` and `cLLI`, the
//!    animation up to its first defect (`acTL`, `fcTL`, `fdAT`, [`apng`]),
//!    and an `eXIf` reduced to Orientation. Everything else is gone,
//!    whatever it is: text and time chunks, private chunks, the frames from
//!    an animation's first defect on, and every byte after `IEND` or after
//!    the end of a zlib stream. The walk also refuses a file
//!    that libpng would refuse, and an animation above the totals of
//!    [`MAX_ANIMATION_FRAMES`] and [`MAX_ANIMATION_PIXELS`]; its module
//!    documentation lists the rules and the few places where it is
//!    stricter.
//! 2. **The encoder**, oxipng with libdeflate (docs/design/image-encoders.md
//!    records why), re-encodes that rewrite losslessly at the preset that
//!    [`PngEffort`] gives for the image's pixel count (ADR 0033 §5). It may
//!    change the colour type, bit depth, palette order, filters and
//!    compression of a still image, and it removes Adam7 interlacing, but
//!    every decoded pixel stays the same. It keeps the rewrite's ancillary
//!    chunks, recompressing only the ICC profile and the frames of an
//!    animation, and adds none; for an animation it changes nothing but the
//!    filters and the compression of every frame.
//!
//! The stored bytes are the encoder's output whenever that is no larger
//! than the upload, which is almost always. Only when the encoder's output
//! is larger than the upload, because the upload was compressed harder than
//! oxipng's preset compresses, is the rewrite stored instead, so that the
//! result is never larger than the upload. The rewrite is written chunk by
//! chunk, but its image data is the upload's own zlib stream, checked to
//! decode to exactly the declared image and cut where the stream ends, and
//! so is each kept frame's data.
//!
//! The encoder runs on the calling thread, with no time limit, so the
//! result depends on the input and the effort alone. oxipng writes
//! diagnostics through the `log` facade only; nothing here reads or writes
//! anything else.

mod apng;
mod chunks;
mod effort;
mod zlib;

use oxipng::{Options, StripChunks};

pub use effort::{LARGE_PNG_PRESET_MAX, PNG_PRESET, PngEffort, PngEffortError};

use super::{ImageError, ImageFormat, PixelBudget, inspect};
#[cfg(doc)]
use super::{MAX_ANIMATION_FRAMES, MAX_ANIMATION_PIXELS};

/// Optimises a PNG upload losslessly and strips its metadata (ADR 0015 §7,
/// ADR 0033 §5 and §6).
///
/// - `bytes`: the whole upload.
/// - `budget`: the largest pixel count the caller is prepared to decode;
///   [`PixelBudget::DEFAULT`] is the 50 megapixels of ADR 0015 §7. The image
///   is refused from its header before anything is decoded; every frame of
///   an animation lies inside that canvas.
/// - `effort`: the oxipng presets and the pixel count where the cheaper one
///   starts, from the settings ([`PngEffort::DEFAULT`] until the production
///   node is measured).
///
/// Returns the bytes to store: a PNG with the same width, height, decoded
/// pixels and, for an animation, the same frames up to its first defect
/// (all of them when it has none) with `acTL` announcing how many, at most
/// as large as `bytes`, with no chunk outside the allow-list of the module
/// documentation.
///
/// # Errors
///
/// - [`ImageError::UnrecognisedFormat`], [`ImageError::Truncated`],
///   [`ImageError::Corrupt`] or [`ImageError::OverPixelBudget`] from
///   [`inspect`], which runs first;
/// - [`ImageError::WrongFormat`] when the upload is a JPEG, GIF or WebP;
/// - [`ImageError::Undecodable`] when the body of the PNG does not decode;
/// - [`ImageError::AnimationTooLarge`] when the frames that the animation's
///   chunks keep are more, or hold more pixels, than its totals allow. No
///   image data has been inflated then, not even the default image's (only
///   the first ICC profile may have been), so this refusal comes before
///   [`ImageError::Undecodable`] for a default image that does not decode.
pub fn optimise_png(
    bytes: &[u8],
    budget: PixelBudget,
    effort: PngEffort,
) -> Result<Vec<u8>, ImageError> {
    let header = inspect(bytes, budget)?;
    if header.format != ImageFormat::Png {
        return Err(ImageError::WrongFormat {
            expected: ImageFormat::Png,
            found: header.format,
        });
    }
    let undecodable = |defect| ImageError::Undecodable {
        format: ImageFormat::Png,
        defect,
    };
    let rewrite = chunks::canonical(bytes).map_err(|refusal| match refusal {
        chunks::Refusal::Defect(defect) => undecodable(defect),
        chunks::Refusal::AnimationTooLarge(apng::TooLarge { frames, pixels }) => {
            ImageError::AnimationTooLarge {
                format: ImageFormat::Png,
                frames,
                pixels,
            }
        }
    })?;
    // The chunk walk has already checked everything oxipng reads: the
    // chunks, each kept zlib stream to its exact rows and every filter-type
    // byte, of the image and of each frame. An error here is not expected;
    // if one comes, the upload is refused as undecodable rather than stored.
    let encoded = oxipng::optimize_from_memory(&rewrite.png, &options(&rewrite, effort))
        .map_err(|_| undecodable(super::DecodeDefect::PngImageData))?;
    Ok(choose(bytes.len(), encoded, rewrite.png))
}

/// The bytes to store: the encoder's output when it is no larger than the
/// upload, and the rewrite otherwise.
///
/// - `upload`: the length of the upload.
/// - `encoded`: what oxipng wrote.
/// - `rewrite`: the chunk walk's rewrite, never larger than the upload.
fn choose(upload: usize, encoded: Vec<u8>, rewrite: Vec<u8>) -> Vec<u8> {
    if encoded.len() <= upload {
        encoded
    } else {
        rewrite
    }
}

/// The oxipng options for `rewrite`. Every field that the guarantees of
/// [`optimise_png`] rest on is set here, so a change of oxipng's defaults
/// cannot change them.
///
/// - `rewrite`: the chunk walk's rewrite, which says how many pixels the
///   encoder works on and whether the bit depth or the colour must stay.
/// - `effort`: the presets and threshold of ADR 0033 §5.
fn options(rewrite: &chunks::Rewrite, effort: PngEffort) -> Options {
    let mut options = Options::from_preset(effort.preset_for(rewrite.pixels));
    // Keep every chunk of the rewrite, which holds only what ADR 0015 §7
    // keeps. Any other setting would let oxipng replace an sRGB-like iCCP by
    // sRGB, or drop iCCP when sRGB is present: PNG gives iCCP precedence, so
    // that could change colours. It also keeps the animation: with acTL
    // present oxipng recompresses each frame and turns off every reduction
    // and interlacing change itself, as the frames share IHDR's format.
    options.strip = StripChunks::None;
    // Always write oxipng's own encoding. Without `force`, oxipng returns
    // its input whenever it cannot beat the input's image data, which would
    // store the upload's zlib stream far more often than the "never larger"
    // rule needs; `choose` applies that rule against the upload instead.
    options.force = true;
    // Never change a pixel: fully transparent pixels keep their colour, and
    // 16-bit samples are reduced only when the low byte repeats the high one.
    options.optimize_alpha = false;
    options.scale_16 = false;
    // oxipng 10.2.1 keeps a 16-bit tRNS value when it reduces an image to 8
    // bits, so a transparent colour that matched no pixel would match some
    // (chunks::Rewrite::wide_transparency). Keeping the bit depth then is
    // lossless and costs only that reduction.
    if rewrite.wide_transparency {
        options.bit_depth_reduction = false;
    }
    // oxipng keeps an RGB image in colour for iCCP and sRGB, but it does not
    // know cICP, whose code points describe RGB samples: a greyscale image
    // of the same pixels could be shown with other colours by a decoder that
    // applies them. Keeping the colour type costs only that reduction.
    if rewrite.coded_colour {
        options.grayscale_reduction = false;
    }
    // Refuse damage instead of repairing it; the rewrite has valid CRCs.
    options.fix_errors = false;
    // No time limit: a deadline would read the clock and make the result
    // depend on the machine's speed. The preset bounds the time instead.
    options.timeout = None;
    // No size limit of oxipng's own: the pixel budget, the animation totals
    // and the chunk walk already bound the image data. Left unset, oxipng
    // inflates an ICC profile into at most 1 MB and keeps a larger one as
    // it is.
    options.max_decompressed_size = None;
    options
}

#[cfg(test)]
pub(super) mod samples {
    //! PNG files built by hand, with stored zlib streams.

    use super::chunks::SIGNATURE;
    use super::zlib::samples::stored;

    /// A chunk of `kind` with `data`, its length and its CRC.
    pub(in crate::domain::image_optimise) fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        assert_eq!(kind.len(), 4, "a chunk type is four bytes");
        let len = u32::try_from(data.len()).unwrap().to_be_bytes();
        let crc = crc32fast::hash(&[kind, data].concat()).to_be_bytes();
        [&len[..], kind, data, &crc].concat()
    }

    /// The `IHDR` chunk of a `width × height` image of bit `depth`, colour
    /// type `colour` and `interlace` method.
    pub(in crate::domain::image_optimise) fn ihdr(
        width: u32,
        height: u32,
        depth: u8,
        colour: u8,
        interlace: u8,
    ) -> Vec<u8> {
        let data = [
            &width.to_be_bytes()[..],
            &height.to_be_bytes(),
            &[depth, colour, 0, 0, interlace],
        ]
        .concat();
        chunk(b"IHDR", &data)
    }

    /// Non-interlaced 8-bit rows of `channels` samples, each with filter
    /// type 0, holding a pattern that differs from pixel to pixel.
    pub(in crate::domain::image_optimise) fn pixels(
        width: u32,
        height: u32,
        channels: u32,
    ) -> Vec<u8> {
        let mut rows = Vec::new();
        for y in 0..height {
            rows.push(0);
            for x in 0..width * channels {
                rows.push(u8::try_from((x * 37 + y * 101) % 251).unwrap());
            }
        }
        rows
    }

    /// A PNG of `ihdr`, the chunks `before`, one `IDAT` chunk per piece of
    /// `data`, the chunks `after` and `IEND`.
    pub(in crate::domain::image_optimise) fn image(
        ihdr: &[u8],
        before: &[Vec<u8>],
        data: &[Vec<u8>],
        after: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut png = [&SIGNATURE[..], ihdr].concat();
        for c in before {
            png.extend_from_slice(c);
        }
        for piece in data {
            png.extend(chunk(b"IDAT", piece));
        }
        for c in after {
            png.extend_from_slice(c);
        }
        png.extend(chunk(b"IEND", &[]));
        png
    }

    /// A minimal ICC profile that libpng accepts, of colour `space` (`RGB `
    /// or `GRAY`): a display profile header, version 2.1, and one tag of 12
    /// bytes. Invented; it describes no real device.
    pub(in crate::domain::image_optimise) fn profile(space: [u8; 4]) -> Vec<u8> {
        let mut profile = vec![0; 144];
        profile[0..4].copy_from_slice(&144_u32.to_be_bytes());
        profile[8] = 2;
        profile[9] = 0x10;
        profile[12..16].copy_from_slice(b"mntr");
        profile[16..20].copy_from_slice(&space);
        profile[20..24].copy_from_slice(b"XYZ ");
        profile[36..40].copy_from_slice(b"acsp");
        // The D50 illuminant of the profile connection space.
        profile[68..80].copy_from_slice(&[0, 0, 0xF6, 0xD6, 0, 1, 0, 0, 0, 0, 0xD3, 0x2D]);
        profile[128..132].copy_from_slice(&1_u32.to_be_bytes());
        profile[132..136].copy_from_slice(b"wtpt");
        profile[136..140].copy_from_slice(&132_u32.to_be_bytes());
        profile[140..144].copy_from_slice(&12_u32.to_be_bytes());
        profile
    }

    /// The data of an `iCCP` chunk: `name`, a NUL, method 0 and `profile` as
    /// a stored zlib stream.
    pub(in crate::domain::image_optimise) fn icc(name: &[u8], profile: &[u8]) -> Vec<u8> {
        [name, &[0, 0], &stored(profile)].concat()
    }

    /// A little-endian EXIF structure with these IFD0 entries, each
    /// `(tag, type, count, value)`, after the `ifd`-byte offset of IFD0.
    pub(in crate::domain::image_optimise) fn exif_little_endian(
        ifd: u32,
        entries: &[(u16, u16, u32, u32)],
    ) -> Vec<u8> {
        let mut tiff = b"II*\0".to_vec();
        tiff.extend_from_slice(&ifd.to_le_bytes());
        tiff.resize(usize::try_from(ifd).unwrap(), 0);
        tiff.extend_from_slice(&u16::try_from(entries.len()).unwrap().to_le_bytes());
        for &(tag, kind, count, value) in entries {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&kind.to_le_bytes());
            tiff.extend_from_slice(&count.to_le_bytes());
            tiff.extend_from_slice(&value.to_le_bytes());
        }
        tiff.extend_from_slice(&0_u32.to_le_bytes());
        tiff
    }

    /// What a camera writes, in short: Make, Orientation 6 and a pointer to
    /// a GPS IFD that holds an invented latitude reference and serial
    /// number. Little-endian, IFD0 at offset 8, 80 bytes.
    pub(in crate::domain::image_optimise) fn camera() -> Vec<u8> {
        let mut tiff = exif_little_endian(
            8,
            &[
                // Make, ASCII, 4 bytes inline: "Cam\0".
                (0x010F, 2, 4, u32::from_le_bytes(*b"Cam\0")),
                (0x0112, 3, 1, 6),
                // GPS IFD pointer, to the bytes appended below.
                (0x8825, 4, 1, 50),
            ],
        );
        // GPS IFD: GPSLatitudeRef "N", and a body serial number tag (0xA431)
        // with an invented value.
        tiff.extend_from_slice(&2_u16.to_le_bytes());
        tiff.extend_from_slice(&[0x01, 0x00, 0x02, 0x00, 0x02, 0, 0, 0, b'N', 0, 0, 0]);
        tiff.extend_from_slice(&[0x31, 0xA4, 0x02, 0x00, 0x04, 0, 0, 0, b'0', b'0', b'7', 0]);
        tiff.extend_from_slice(&0_u32.to_le_bytes());
        tiff
    }

    /// A 4×3 8-bit RGB PNG with the chunks `before` and `after` its data.
    pub(in crate::domain::image_optimise) fn rgb(before: &[Vec<u8>], after: &[Vec<u8>]) -> Vec<u8> {
        image(
            &ihdr(4, 3, 8, 2, 0),
            before,
            &[stored(&pixels(4, 3, 3))],
            after,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::apng::samples::{actl, fctl, fdat};
    use super::samples::camera;
    use super::samples::{self, chunk, icc, ihdr, image, pixels, rgb};
    use super::zlib::samples::stored;
    use super::*;
    use crate::domain::image_optimise::{DecodeDefect, Dimensions, header};
    use proptest::{collection::vec, prelude::*};

    fn unlimited() -> PixelBudget {
        PixelBudget::new(u64::MAX).unwrap()
    }

    #[test]
    fn a_stored_png_is_recompressed_smaller_with_its_chunks() {
        // A larger image than the 4x3 sample, so that compression pays.
        let input = image(
            &ihdr(64, 48, 8, 2, 0),
            &[
                chunk(b"sRGB", &[0]),
                chunk(b"eXIf", &camera()),
                chunk(b"tEXt", b"a\0b"),
            ],
            &[stored(&pixels(64, 48, 3))],
            &[],
        );
        let out = optimise_png(&input, PixelBudget::DEFAULT, PngEffort::DEFAULT).unwrap();
        assert!(
            out.len() < input.len() / 2,
            "{} of {}",
            out.len(),
            input.len()
        );
        assert_eq!(
            inspect(&out, PixelBudget::DEFAULT).map(|h| h.dimensions),
            Ok(Dimensions::new(64, 48).unwrap())
        );
        let has = |kind: &[u8]| out.windows(4).any(|w| w == kind);
        assert!(has(b"sRGB") && has(b"eXIf") && !has(b"tEXt"));
        // What the encoder wrote passes the chunk walk unchanged.
        assert_eq!(
            chunks::canonical(&out).map(|rewrite| rewrite.png),
            Ok(out.clone())
        );
    }

    #[test]
    fn the_result_is_never_larger_than_the_upload() {
        // oxipng's own output, optimised again, cannot be beaten: the second
        // run keeps the first one's bytes.
        let once = optimise_png(&rgb(&[], &[]), unlimited(), PngEffort::DEFAULT).unwrap();
        let twice = optimise_png(&once, unlimited(), PngEffort::DEFAULT).unwrap();
        assert!(twice.len() <= once.len());
        let with_tail = [&once[..], b"<script>payload</script>"].concat();
        let cleaned = optimise_png(&with_tail, unlimited(), PngEffort::DEFAULT).unwrap();
        assert!(cleaned.len() <= once.len());
        assert!(!cleaned.windows(6).any(|w| w == b"script"));
    }

    #[test]
    fn other_formats_and_broken_pngs_are_refused() {
        assert_eq!(
            optimise_png(&header::samples::gif(1, 1), unlimited(), PngEffort::DEFAULT),
            Err(ImageError::WrongFormat {
                expected: ImageFormat::Png,
                found: ImageFormat::Gif
            })
        );
        assert_eq!(
            optimise_png(b"<svg/>", unlimited(), PngEffort::DEFAULT),
            Err(ImageError::UnrecognisedFormat)
        );
        assert_eq!(
            optimise_png(&header::samples::png(4, 3), unlimited(), PngEffort::DEFAULT),
            Err(ImageError::Undecodable {
                format: ImageFormat::Png,
                defect: DecodeDefect::PngTruncated
            })
        );
        // Filter type 5 does not exist; only the encoder's decode sees it.
        let mut rows = pixels(4, 3, 3);
        rows[0] = 5;
        let broken = image(&ihdr(4, 3, 8, 2, 0), &[], &[stored(&rows)], &[]);
        assert_eq!(
            optimise_png(&broken, unlimited(), PngEffort::DEFAULT),
            Err(ImageError::Undecodable {
                format: ImageFormat::Png,
                defect: DecodeDefect::PngImageData
            })
        );
    }

    #[test]
    fn the_budget_is_held_before_anything_is_decoded() {
        let budget = PixelBudget::new(11).unwrap();
        assert_eq!(
            optimise_png(&rgb(&[], &[]), budget, PngEffort::DEFAULT),
            Err(ImageError::OverPixelBudget {
                format: ImageFormat::Png,
                dimensions: Dimensions::new(4, 3).unwrap(),
                budget,
            })
        );
    }

    #[test]
    fn colour_chunks_survive_and_keep_the_image_in_colour() {
        // Grey pixels in an RGB image would be reduced to greyscale, which an
        // RGB profile cannot describe; with a colour chunk, oxipng keeps the
        // image in colour, and keeps iCCP and sRGB side by side.
        let rows: Vec<u8> = (0..32_u8)
            .flat_map(|y| {
                let mut row = vec![0];
                for x in 0..32_u8 {
                    let v = x.wrapping_mul(7) ^ y.wrapping_mul(13);
                    row.extend([v, v, v]);
                }
                row
            })
            .collect();
        let profile = icc(b"p", &samples::profile(*b"RGB "));
        for colour in [
            vec![chunk(b"iCCP", &profile)],
            vec![chunk(b"sRGB", &[0])],
            vec![chunk(b"sRGB", &[0]), chunk(b"iCCP", &profile)],
            vec![chunk(b"cICP", &[1, 13, 0, 0])],
        ] {
            let input = image(&ihdr(32, 32, 8, 2, 0), &colour, &[stored(&rows)], &[]);
            let out = optimise_png(&input, unlimited(), PngEffort::DEFAULT).unwrap();
            let has = |kind: &[u8]| out.windows(4).any(|w| w == kind);
            for kept in &colour {
                assert!(has(&kept[4..8]), "{}", String::from_utf8_lossy(&kept[4..8]));
            }
            // IHDR's colour type: truecolour or indexed, never greyscale.
            assert!(matches!(out[25], 2 | 3), "colour type {}", out[25]);
        }
    }

    #[test]
    fn the_options_keep_every_guarantee() {
        let mut rewrite = chunks::canonical(&rgb(&[], &[])).unwrap();
        let set = options(&rewrite, PngEffort::DEFAULT);
        assert_eq!(set.strip, StripChunks::None);
        assert!(set.force && !set.optimize_alpha && !set.scale_16);
        assert!(!set.fix_errors && set.timeout.is_none());
        assert!(set.bit_depth_reduction);
        assert!(set.grayscale_reduction);
        rewrite.wide_transparency = true;
        assert!(!options(&rewrite, PngEffort::DEFAULT).bit_depth_reduction);
        rewrite.coded_colour = true;
        assert!(!options(&rewrite, PngEffort::DEFAULT).grayscale_reduction);
    }

    /// Whether `set` holds the filters and the compressor of oxipng's preset
    /// `preset`, which is all a preset sets for these images.
    fn is_preset(set: &Options, preset: u8) -> bool {
        let expected = Options::from_preset(preset);
        set.filters == expected.filters && set.deflater == expected.deflater
    }

    #[test]
    fn the_preset_changes_just_above_the_threshold() {
        // 12 pixels: preset 2 at a threshold of 12 or 13, preset 0 at 11.
        let rewrite = chunks::canonical(&rgb(&[], &[])).unwrap();
        assert_eq!(rewrite.pixels, 12);
        for (threshold, large_preset, preset) in [(11, 0, 0), (11, 1, 1), (12, 0, 2), (13, 1, 2)] {
            let effort = PngEffort::new(threshold, large_preset).unwrap();
            assert!(is_preset(&options(&rewrite, effort), preset), "{threshold}");
        }
        // The presets differ for this image, so the choice reaches the output.
        let input = image(
            &ihdr(64, 48, 8, 2, 0),
            &[],
            &[stored(&pixels(64, 48, 3))],
            &[],
        );
        let effort = |threshold, large_preset| PngEffort::new(threshold, large_preset).unwrap();
        let at = optimise_png(&input, unlimited(), effort(3_072, 0)).unwrap();
        let above = optimise_png(&input, unlimited(), effort(3_071, 0)).unwrap();
        assert!(at.len() < above.len(), "{} >= {}", at.len(), above.len());
        // Above the threshold, the cheaper preset given is the one used.
        let above_at_1 = optimise_png(&input, unlimited(), effort(3_071, 1)).unwrap();
        assert_ne!(above_at_1, above);
        assert_eq!(
            optimise_png(&input, unlimited(), effort(1, 1)),
            Ok(above_at_1)
        );
    }

    /// A 32×32 RGB APNG of stored streams: the default image, then a 16×16
    /// frame at (8, 8). 1024 + 256 pixels.
    fn animation() -> Vec<u8> {
        image(
            &ihdr(32, 32, 8, 2, 0),
            &[
                chunk(b"acTL", &actl(2, 0)),
                chunk(b"fcTL", &fctl(0, (32, 32), (0, 0))),
            ],
            &[stored(&pixels(32, 32, 3))],
            &[
                chunk(b"fcTL", &fctl(1, (16, 16), (8, 8))),
                chunk(b"fdAT", &fdat(2, &stored(&pixels(16, 16, 3)))),
            ],
        )
    }

    #[test]
    fn an_animation_counts_its_frames_for_the_preset() {
        let rewrite = chunks::canonical(&animation()).unwrap();
        assert_eq!(rewrite.pixels, 1_024 + 256);
        let effort = PngEffort::new(1_279, 0).unwrap();
        assert!(is_preset(&options(&rewrite, effort), 0));
        let effort = PngEffort::new(1_280, 0).unwrap();
        assert!(is_preset(&options(&rewrite, effort), 2));
    }

    /// The data of every chunk of `kind` in `png`, in order.
    fn chunks_of(png: &[u8], kind: [u8; 4]) -> Vec<Vec<u8>> {
        let mut at = chunks::SIGNATURE.len();
        let mut found = Vec::new();
        while let Some(header) = png.get(at..at + 8) {
            let len =
                usize::try_from(u32::from_be_bytes(header[0..4].try_into().unwrap())).unwrap();
            if header[4..8] == kind[..] {
                found.push(png[at + 8..at + 8 + len].to_vec());
            }
            at += 12 + len;
        }
        found
    }

    #[test]
    fn an_animation_is_recompressed_with_its_frames() {
        let input = animation();
        let out = optimise_png(&input, unlimited(), PngEffort::DEFAULT).unwrap();
        assert!(
            out.len() < input.len() / 2,
            "{} of {}",
            out.len(),
            input.len()
        );
        let count = |kind: &[u8]| out.windows(4).filter(|w| *w == kind).count();
        assert_eq!((count(b"acTL"), count(b"fcTL"), count(b"fdAT")), (1, 2, 1));
        // The frame itself is recompressed, not only the default image: its
        // stored stream of 16 rows of 49 bytes shrinks to less than half.
        let [uploaded] = &chunks_of(&input, *b"fdAT")[..] else {
            panic!("one fdAT in the upload");
        };
        let [stored_frame] = &chunks_of(&out, *b"fdAT")[..] else {
            panic!("one fdAT in the output");
        };
        assert_eq!(uploaded.len(), 4 + 2 + 5 + 16 * 49 + 4);
        assert!(
            stored_frame.len() - 4 < (uploaded.len() - 4) / 2,
            "{} of {}",
            stored_frame.len(),
            uploaded.len()
        );
        // oxipng wrote the animation the way the walk writes it.
        assert_eq!(
            chunks::canonical(&out).map(|rewrite| rewrite.png),
            Ok(out.clone())
        );
    }

    #[test]
    fn a_frame_with_an_unknown_filter_type_is_dropped_with_the_frames_after_it() {
        // The third frame's first row starts with filter type 5, which no
        // decoder knows, so that frame is the animation's first defect: it is
        // dropped, and the two frames before it are stored as an animation
        // of two, where the encoder alone would refuse the whole upload.
        let mut rows = pixels(8, 8, 3);
        rows[0] = 5;
        let upload = |frames: u32, third: &[u8]| {
            let mut after = vec![
                chunk(b"fcTL", &fctl(1, (16, 16), (8, 8))),
                chunk(b"fdAT", &fdat(2, &stored(&pixels(16, 16, 3)))),
            ];
            if !third.is_empty() {
                after.push(chunk(b"fcTL", &fctl(3, (8, 8), (0, 0))));
                after.push(chunk(b"fdAT", &fdat(4, &stored(third))));
            }
            image(
                &ihdr(32, 32, 8, 2, 0),
                &[
                    chunk(b"acTL", &actl(frames, 0)),
                    chunk(b"fcTL", &fctl(0, (32, 32), (0, 0))),
                ],
                &[stored(&pixels(32, 32, 3))],
                &after,
            )
        };
        let broken = upload(3, &rows);
        let two = upload(2, &[]);
        let out = optimise_png(&broken, unlimited(), PngEffort::DEFAULT).unwrap();
        assert_eq!(
            Ok(out.clone()),
            optimise_png(&two, unlimited(), PngEffort::DEFAULT)
        );
        assert_eq!(chunks_of(&out, *b"acTL"), [actl(2, 0)]);
        assert!(out.len() <= broken.len());
    }

    #[test]
    fn an_animation_above_the_totals_is_refused_with_its_own_error() {
        // 4097 frames of 1x1: the default image and 4096 more.
        let mut after = Vec::new();
        for index in 1..=4096_u32 {
            after.push(chunk(b"fcTL", &fctl(2 * index - 1, (1, 1), (0, 0))));
            after.push(chunk(b"fdAT", &fdat(2 * index, &stored(&[0, 0, 0, 0]))));
        }
        let input = image(
            &ihdr(1, 1, 8, 2, 0),
            &[
                chunk(b"acTL", &actl(4097, 0)),
                chunk(b"fcTL", &fctl(0, (1, 1), (0, 0))),
            ],
            &[stored(&[0, 0, 0, 0])],
            &after,
        );
        assert_eq!(
            optimise_png(&input, unlimited(), PngEffort::DEFAULT),
            Err(ImageError::AnimationTooLarge {
                format: ImageFormat::Png,
                frames: 4097,
                pixels: 4097,
            })
        );
    }

    #[test]
    fn the_encoder_output_is_stored_unless_it_is_larger_than_the_upload() {
        let (encoded, rewrite) = (vec![1; 10], vec![2; 8]);
        assert_eq!(choose(10, encoded.clone(), rewrite.clone()), encoded);
        assert_eq!(choose(11, encoded.clone(), rewrite.clone()), encoded);
        assert_eq!(choose(9, encoded, rewrite.clone()), rewrite);
    }

    #[test]
    fn an_upload_compressed_harder_than_the_preset_keeps_the_rewrite() {
        // Compressed by oxipng itself at its slowest preset, the upload is
        // smaller than what preset 2 writes, so the rewrite is stored: the
        // upload's own chunks, which the rewrite reproduces exactly.
        let mut slow = Options::from_preset(6);
        slow.strip = StripChunks::None;
        // A gradient with a little noise (xorshift), which preset 6's
        // exhaustive filter search compresses about 2 % better.
        let mut rows = Vec::new();
        let mut noise = 1_u32;
        for y in 0..48_u8 {
            rows.push(0);
            for i in 0..64 * 3_u8 {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                let jitter = u8::try_from(noise % 7).unwrap();
                rows.push((i / 3 * 4).wrapping_add(y * 3).wrapping_add(jitter));
            }
        }
        let source = image(&ihdr(64, 48, 8, 2, 0), &[], &[stored(&rows)], &[]);
        let upload = oxipng::optimize_from_memory(&source, &slow).unwrap();
        let rewrite = chunks::canonical(&upload).unwrap();
        let encoded =
            oxipng::optimize_from_memory(&rewrite.png, &options(&rewrite, PngEffort::DEFAULT))
                .unwrap();
        assert!(
            encoded.len() > upload.len(),
            "{} <= {}",
            encoded.len(),
            upload.len()
        );
        assert_eq!(
            optimise_png(&upload, unlimited(), PngEffort::DEFAULT),
            Ok(rewrite.png)
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn any_image_keeps_its_size_and_never_grows(
            width in 1..24_u32,
            height in 1..24_u32,
            seed in any::<u8>(),
            tail in vec(any::<u8>(), 0..16),
        ) {
            let mut rows = pixels(width, height, 3);
            rows.iter_mut().enumerate().filter(|(i, _)| i % 7 == 0).for_each(|(_, b)| *b ^= seed);
            // Keep the filter type bytes valid (0-4).
            let row = 1 + 3 * usize::try_from(width).unwrap();
            rows.iter_mut().step_by(row).for_each(|b| *b %= 5);
            let input = [image(&ihdr(width, height, 8, 2, 0), &[], &[stored(&rows)], &[]), tail].concat();
            let out = optimise_png(&input, unlimited(), PngEffort::DEFAULT).unwrap();
            prop_assert!(out.len() <= input.len());
            prop_assert_eq!(
                inspect(&out, unlimited()).map(|h| h.dimensions),
                Ok(Dimensions::new(width, height).unwrap())
            );
        }

        #[test]
        fn the_choice_is_never_larger_than_the_upload_and_prefers_the_encoder(
            upload in 0..64_usize,
            encoded in 0..64_usize,
            rewrite in 0..64_usize,
        ) {
            prop_assume!(rewrite <= upload);
            let chosen = choose(upload, vec![1; encoded], vec![2; rewrite]);
            prop_assert!(chosen.len() <= upload);
            // The encoder's output whenever it fits, whatever the rewrite's size.
            let expected = if encoded <= upload { vec![1; encoded] } else { vec![2; rewrite] };
            prop_assert_eq!(chosen, expected);
        }

        #[test]
        fn arbitrary_and_damaged_input_never_panics(
            damage in vec((any::<prop::sample::Index>(), any::<u8>()), 0..8),
            // Cut only now and then: a cut file has no IEND and is refused.
            cut in prop::option::weighted(0.2, any::<prop::sample::Index>()),
            noise in vec(any::<u8>(), 0..64),
        ) {
            let mut bytes = rgb(&[chunk(b"eXIf", &camera())], &[]);
            for (at, byte) in damage {
                let at = at.index(bytes.len());
                bytes[at] = byte;
            }
            if let Some(cut) = cut {
                bytes.truncate(cut.index(bytes.len()));
            }
            for input in [&bytes[..], &noise] {
                if let Ok(out) = optimise_png(input, PixelBudget::DEFAULT, PngEffort::DEFAULT) {
                    prop_assert!(out.len() <= input.len());
                }
            }
        }
    }
}
