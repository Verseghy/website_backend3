//! The first step of every image upload: what the bytes are, and whether
//! their size is safe to decode (ADR 0015 §6 and §7).
//!
//! Every new image upload is optimised before its URL exists (ADR 0015 §7).
//! Optimising means decoding, and a decoder allocates memory for every pixel
//! that the header declares, whatever the size of the file: a few hundred
//! bytes of PNG or GIF can declare billions of pixels. So before any decoder
//! runs, [`inspect`] reads only the header and answers three questions:
//!
//! 1. **Which format is it?** The magic bytes decide ([`sniff`]); the file
//!    name and the content type the client sent are not trusted. Only JPEG,
//!    PNG, GIF and WebP are images here (ADR 0015 §6). Everything else, SVG
//!    and HTML included, is [`ImageError::UnrecognisedFormat`].
//! 2. **How large is it?** A reader per format takes the stored width and
//!    height from the header: the JPEG frame header, the PNG `IHDR` chunk,
//!    the GIF logical screen grown to hold every frame, or the first WebP
//!    chunk. The EXIF Orientation tag plays no part, because ADR 0015 §7
//!    never changes the stored pixels or dimensions.
//! 3. **Does it fit the pixel budget?** An image above the [`PixelBudget`]
//!    (50 megapixels unless configured otherwise) is refused from its header,
//!    so nothing is decoded and no memory is allocated for it.
//!
//! Each reader follows the header rules of its format's reference decoder:
//! libjpeg's marker reader, libpng's `IHDR` checks, the block structure of
//! the `gif` crate, and libwebp's RIFF, VP8, VP8L and VP8X checks. The
//! dimensions it budgets are then the ones that decoder allocates for. It
//! refuses a header that is cut short ([`ImageError::Truncated`]) or one that
//! the decoder refuses as malformed ([`ImageError::Corrupt`]), and nothing
//! the decoder accepts, because ADR 0015 §7 counts as corrupt only an image
//! that does not decode. Nothing is decoded here. A body that is cut short or
//! corrupt passes and is refused by the decode that follows, which ADR 0015
//! §7 requires; that decoder keeps its own memory limits as a second line.
//!
//! Animations are budgeted by their canvas. An animated PNG or WebP keeps
//! every frame inside its canvas: APNG requires it, and libwebp refuses a
//! frame that does not fit. A GIF has no such rule, so its reader grows the
//! canvas to hold every frame. Code that walks the frames can still hold each
//! one to the budget with [`PixelBudget::admits`].
//!
//! Everything here is pure (ADR 0008): bytes in, a typed result out. The
//! caller reads the upload, passes the budget from its settings, and maps each
//! [`ImageError`] to the Hungarian message the editor sees.

mod budget;
mod format;
mod header;

pub use budget::{Dimensions, PixelBudget, PixelBudgetError};
pub use format::{ImageFormat, sniff};
pub use header::HeaderDefect;

/// What the header of an accepted image says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageHeader {
    /// The format that the magic bytes name.
    pub format: ImageFormat,
    /// The stored width and height; for an animation, its canvas. For a GIF,
    /// the canvas that holds the logical screen and every frame.
    pub dimensions: Dimensions,
}

/// Why an upload is not accepted as an image. The messages are English, for
/// logs; the caller shows the editor a Hungarian one (ADR 0021 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// The bytes start with none of the four signatures.
    #[error("the file is not a JPEG, PNG, GIF or WebP image")]
    UnrecognisedFormat,
    /// The signature names a format, but the input ends inside its header.
    #[error("the {format} header is cut short")]
    Truncated {
        /// The sniffed format.
        format: ImageFormat,
    },
    /// The header is malformed.
    #[error("the {format} header is malformed: {defect}")]
    Corrupt {
        /// The sniffed format.
        format: ImageFormat,
        /// What is wrong with the header.
        defect: HeaderDefect,
    },
    /// The header declares more pixels than the budget allows.
    #[error(
        "the {format} image is {dimensions} ({} pixels), above the pixel budget of {} pixels",
        .dimensions.pixels(),
        .budget.max_pixels()
    )]
    OverPixelBudget {
        /// The sniffed format.
        format: ImageFormat,
        /// The dimensions the header declares.
        dimensions: Dimensions,
        /// The budget they exceed.
        budget: PixelBudget,
    },
}

/// Reads the header of an upload and checks it against the pixel budget,
/// without decoding anything.
///
/// - `bytes`: the whole upload, not a prefix of fixed size. A JPEG frame
///   header may follow any number of metadata segments, so a prefix can end
///   before it and be refused as truncated; and a GIF frame that a prefix
///   cuts off would not be budgeted.
/// - `budget`: the largest pixel count the caller is prepared to decode;
///   [`PixelBudget::DEFAULT`] is the 50 megapixels of ADR 0015 §7.
///
/// # Errors
///
/// In the order of checking: [`ImageError::UnrecognisedFormat`] when no
/// signature matches, [`ImageError::Truncated`] or [`ImageError::Corrupt`]
/// when the header cannot be read, and [`ImageError::OverPixelBudget`] when
/// its dimensions exceed `budget`.
pub fn inspect(bytes: &[u8], budget: PixelBudget) -> Result<ImageHeader, ImageError> {
    let format = sniff(bytes).ok_or(ImageError::UnrecognisedFormat)?;
    let dimensions = header::read_dimensions(format, bytes).map_err(|error| match error {
        header::HeaderError::Truncated => ImageError::Truncated { format },
        header::HeaderError::Corrupt(defect) => ImageError::Corrupt { format, defect },
    })?;
    if !budget.admits(dimensions) {
        return Err(ImageError::OverPixelBudget {
            format,
            dimensions,
            budget,
        });
    }
    Ok(ImageHeader { format, dimensions })
}

#[cfg(test)]
mod tests {
    use super::header::samples;
    use super::*;
    use proptest::{collection::vec, prelude::*, sample::Index};

    fn unlimited() -> PixelBudget {
        PixelBudget::new(u64::MAX).unwrap()
    }

    fn dimensions(width: u32, height: u32) -> Dimensions {
        Dimensions::new(width, height).unwrap()
    }

    /// A well-formed header of each format, with dimensions that format can hold.
    fn any_header() -> impl Strategy<Value = (ImageFormat, u32, u32, Vec<u8>)> {
        /// A header strategy for a format whose sides are `u16`.
        fn sides16(
            format: ImageFormat,
            max: u16,
            sample: fn(u16, u16) -> Vec<u8>,
        ) -> impl Strategy<Value = (ImageFormat, u32, u32, Vec<u8>)> {
            (1..=max, 1..=max).prop_map(move |(w, h)| (format, w.into(), h.into(), sample(w, h)))
        }
        prop_oneof![
            sides16(ImageFormat::Jpeg, u16::MAX, samples::jpeg),
            sides16(ImageFormat::Gif, u16::MAX, samples::gif),
            sides16(ImageFormat::WebP, 0x3FFF, samples::webp_lossy),
            sides16(ImageFormat::WebP, 0x4000, samples::webp_lossless),
            (1..=0x7FFF_FFFF_u32, 1..=0x7FFF_FFFF_u32).prop_map(|(w, h)| (
                ImageFormat::Png,
                w,
                h,
                samples::png(w, h)
            )),
            // VP8X sides go up to 2^24, but the canvas must stay below 2^32 pixels.
            (1..=1_u32 << 24)
                .prop_flat_map(|w| (Just(w), 1..=(u32::MAX / w).min(1 << 24)))
                .prop_map(|(w, h)| (ImageFormat::WebP, w, h, samples::webp_extended(w, h))),
        ]
    }

    #[test]
    fn the_default_budget_admits_50_megapixels_and_no_more() {
        // 10000x5000 is exactly 50,000,000 pixels; one more row is above it.
        let at = inspect(&samples::jpeg(10_000, 5_000), PixelBudget::DEFAULT);
        assert_eq!(
            at.map(|header| header.dimensions),
            Ok(dimensions(10_000, 5_000))
        );
        assert_eq!(
            inspect(&samples::jpeg(10_000, 5_001), PixelBudget::DEFAULT),
            Err(ImageError::OverPixelBudget {
                format: ImageFormat::Jpeg,
                dimensions: dimensions(10_000, 5_001),
                budget: PixelBudget::DEFAULT,
            })
        );
    }

    #[test]
    fn a_gif_frame_beyond_its_screen_is_held_to_the_budget() {
        // A 1x1 screen with one frame of a 50 MP camera's size.
        let bytes = [samples::gif(1, 1), samples::gif_frame(0, 0, 8_688, 5_792)].concat();
        assert_eq!(
            inspect(&bytes, PixelBudget::DEFAULT),
            Err(ImageError::OverPixelBudget {
                format: ImageFormat::Gif,
                dimensions: dimensions(8_688, 5_792),
                budget: PixelBudget::DEFAULT,
            })
        );
    }

    #[test]
    fn other_content_is_unrecognised() {
        // The types that editor.md §10.1 names, and a few near misses.
        for bytes in [
            &b""[..],
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            b"<?xml version=\"1.0\"?><svg/>",
            b"<!DOCTYPE html><html></html>",
            b"\0\0\0\x18ftypheic\0\0\0\0mif1heic",
            b"\0\0\0\x1Cftypavif\0\0\0\0avifmif1miaf",
            b"%PDF-1.7",
            b"PK\x03\x04",
            b"\xFF\xD8",
            b"GIF89",
            b"RIFF\x24\0\0\0WAVEfmt ",
        ] {
            assert_eq!(
                inspect(bytes, unlimited()),
                Err(ImageError::UnrecognisedFormat)
            );
        }
    }

    #[test]
    fn errors_name_the_format_and_the_numbers() {
        let over = inspect(&samples::png(8_688, 5_792), PixelBudget::DEFAULT).unwrap_err();
        assert_eq!(
            over.to_string(),
            "the PNG image is 8688x5792 (50320896 pixels), above the pixel budget of 50000000 pixels"
        );
        let cut = inspect(&samples::gif(1, 1)[..8], unlimited()).unwrap_err();
        assert_eq!(cut.to_string(), "the GIF header is cut short");
        assert_eq!(
            inspect(&samples::jpeg(1, 0), unlimited())
                .unwrap_err()
                .to_string(),
            "the JPEG header is malformed: the width or the height is zero"
        );
    }

    proptest! {
        #[test]
        fn a_well_formed_header_reads_back_its_dimensions((format, w, h, bytes) in any_header()) {
            prop_assert_eq!(
                inspect(&bytes, unlimited()),
                Ok(ImageHeader { format, dimensions: dimensions(w, h) })
            );
        }

        #[test]
        fn the_budget_refuses_exactly_the_images_above_it((format, w, h, bytes) in any_header()) {
            let pixels = u64::from(w) * u64::from(h);
            prop_assert!(inspect(&bytes, PixelBudget::new(pixels).unwrap()).is_ok());
            if let Ok(budget) = PixelBudget::new(pixels - 1) {
                prop_assert_eq!(
                    inspect(&bytes, budget),
                    Err(ImageError::OverPixelBudget { format, dimensions: dimensions(w, h), budget })
                );
            }
        }

        #[test]
        fn every_cut_short_header_is_truncated(
            (format, _w, _h, bytes) in any_header(),
            cut in any::<Index>(),
        ) {
            let prefix = &bytes[..cut.index(bytes.len())];
            let expected = match sniff(prefix) {
                None => ImageError::UnrecognisedFormat,
                Some(sniffed) => {
                    prop_assert_eq!(sniffed, format);
                    ImageError::Truncated { format }
                }
            };
            prop_assert_eq!(inspect(prefix, unlimited()), Err(expected));
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in vec(any::<u8>(), 0..80), max in 1..=u64::MAX) {
            let result = inspect(&bytes, PixelBudget::new(max).unwrap());
            prop_assert_eq!(result == Err(ImageError::UnrecognisedFormat), sniff(&bytes).is_none());
        }

        #[test]
        fn damaged_headers_never_panic_and_never_exceed_the_budget(
            (_format, _w, _h, mut bytes) in any_header(),
            damage in vec((any::<Index>(), any::<u8>()), 0..6),
            cut in any::<Index>(),
            tail in vec(any::<u8>(), 0..16),
            max in 1..=u64::from(u32::MAX),
        ) {
            for (at, byte) in damage {
                let at = at.index(bytes.len());
                bytes[at] = byte;
            }
            bytes.truncate(cut.index(bytes.len() + 1));
            bytes.extend(tail);
            let budget = PixelBudget::new(max).unwrap();
            if let Ok(header) = inspect(&bytes, budget) {
                prop_assert_eq!(Some(header.format), sniff(&bytes));
                prop_assert!(budget.admits(header.dimensions));
            }
        }
    }
}
