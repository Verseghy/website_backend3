//! Header-only dimension readers, one per format.
//!
//! Each reader gets the whole input, whose signature [`super::sniff`] has
//! already matched, and returns the stored dimensions without decoding. Only
//! the GIF reader reads past the header: GIF frames are not bounded by the
//! screen, so it steps over the image data to reach each frame's size. Every
//! read is bounds-checked, through [`take`] or [`bytes_at`], which turn a
//! read past the end of the input into [`HeaderError::Truncated`], or through
//! `slice::get` where the end of the input ends a walk, so no input can make
//! a reader index out of bounds or panic.

mod gif;
pub(super) mod jpeg;
mod png;
mod webp;

use super::{Dimensions, ImageFormat};

/// What is wrong with a malformed header. The messages are English, for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum HeaderDefect {
    /// The width or the height is zero; for a GIF, a side of the canvas that
    /// holds the screen and every frame. A JPEG whose height is zero takes it
    /// from a DNL marker after the first scan, which libjpeg does not support
    /// either.
    #[error("the width or the height is zero")]
    ZeroDimension,
    /// The size is above the format's own limit: a PNG side above 2³¹ − 1, or
    /// a WebP canvas of 2³² pixels or more (libwebp refuses both).
    #[error("the dimensions are above the format's limit")]
    DimensionTooLarge,
    /// A JPEG marker that libjpeg refuses before the first scan: a second
    /// start of image, or a code it does not know (JPG, DHP, EXP, the `JPGn`
    /// extensions and the reserved codes).
    #[error("the JPEG marker 0x{0:02X} is not allowed before the first scan")]
    JpegUnexpectedMarker(u8),
    /// A JPEG table or frame header segment whose length is below 2, the size
    /// of the length field itself.
    #[error("a JPEG segment length is below 2")]
    JpegSegmentLength,
    /// The JPEG frame header has no components, or a length that does not
    /// match their count.
    #[error("the JPEG frame header is malformed")]
    JpegFrameHeader,
    /// A second JPEG frame header before the first scan.
    #[error("the JPEG data has a second frame header")]
    JpegDuplicateFrameHeader,
    /// A JPEG scan or the end of the image comes before any frame header.
    #[error("the JPEG data has no frame header")]
    JpegNoFrameHeader,
    /// The JPEG image ends after its frame header, before any scan.
    #[error("the JPEG data has no scan")]
    JpegNoScan,
    /// A GIF block starts with a byte that is not an extension (`21`), an
    /// image (`2C`) or the trailer (`3B`).
    #[error("the GIF block type 0x{0:02X} is unknown")]
    GifBlock(u8),
    /// The first PNG chunk is not a 13-byte `IHDR` chunk.
    #[error("the first PNG chunk is not IHDR")]
    PngFirstChunk,
    /// The checksum of the PNG `IHDR` chunk does not match its content.
    #[error("the PNG IHDR checksum does not match")]
    PngChecksum,
    /// The PNG `IHDR` names a bit depth and colour type pair, or a
    /// compression, filter or interlace method, that PNG does not define.
    #[error("the PNG IHDR names an undefined image type or method")]
    PngImageType,
    /// The RIFF size or the size of the first WebP chunk is out of range or
    /// does not fit the RIFF size.
    #[error("the WebP chunk sizes are inconsistent")]
    WebpChunkSize,
    /// The first WebP chunk is not `VP8 `, `VP8L` or `VP8X`.
    #[error("the first WebP chunk is not VP8, VP8L or VP8X")]
    WebpFirstChunk,
    /// The VP8 frame tag does not describe a shown key frame of a known
    /// profile whose first partition fits the chunk.
    #[error("the VP8 frame tag is not a valid key frame")]
    Vp8FrameTag,
    /// The VP8 key frame lacks its start code `9D 01 2A`.
    #[error("the VP8 start code is missing")]
    Vp8StartCode,
    /// The VP8L header has a wrong signature byte or a version other than 0.
    #[error("the VP8L header is malformed")]
    Vp8lHeader,
}

/// Why a reader returned no dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderError {
    /// The input ends inside the header.
    Truncated,
    /// The header is malformed.
    Corrupt(HeaderDefect),
}

impl From<HeaderDefect> for HeaderError {
    fn from(defect: HeaderDefect) -> Self {
        Self::Corrupt(defect)
    }
}

/// The stored dimensions of an image of `format`.
///
/// - `format`: the format [`super::sniff`] found for `bytes`.
/// - `bytes`: the whole input, starting with that format's signature.
pub(super) fn read_dimensions(
    format: ImageFormat,
    bytes: &[u8],
) -> Result<Dimensions, HeaderError> {
    match format {
        ImageFormat::Jpeg => jpeg::read(bytes),
        ImageFormat::Png => png::read(bytes),
        ImageFormat::Gif => gif::read(bytes),
        ImageFormat::WebP => webp::read(bytes),
    }
}

/// `width × height` as [`Dimensions`], or [`HeaderDefect::ZeroDimension`].
fn dimensions(width: u32, height: u32) -> Result<Dimensions, HeaderError> {
    Dimensions::new(width, height).ok_or(HeaderError::Corrupt(HeaderDefect::ZeroDimension))
}

/// The `len` bytes of `bytes` that start at offset `at`, or
/// [`HeaderError::Truncated`] when the input ends first.
pub(super) fn take(bytes: &[u8], at: usize, len: usize) -> Result<&[u8], HeaderError> {
    at.checked_add(len)
        .and_then(|end| bytes.get(at..end))
        .ok_or(HeaderError::Truncated)
}

/// The `N` bytes of `bytes` that start at offset `at`, as an array, or
/// [`HeaderError::Truncated`] when the input ends first.
pub(super) fn bytes_at<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], HeaderError> {
    take(bytes, at, N)?
        .try_into()
        .map_err(|_| HeaderError::Truncated)
}

/// Minimal well-formed headers for the tests. Each ends exactly where its
/// reader stops reading, so every strict prefix of one is truncated;
/// `jpeg_frame` and `gif_frame` are parts to build longer inputs from. They
/// are headers only, not decodable images, and the builders do not validate
/// their arguments, so tests can also build malformed headers with them.
#[cfg(test)]
pub(crate) mod samples {
    /// SOI, a JFIF APP0 segment, a DQT segment, a baseline frame header
    /// (SOF0) with three components, then the marker of the first scan (SOS).
    pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        bytes.extend([
            0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
        ]);
        bytes.extend([0xFF, 0xDB, 0, 67, 0]);
        bytes.extend([1_u8; 64]);
        bytes.extend(jpeg_frame(width, height));
        bytes.extend([0xFF, 0xDA]);
        bytes
    }

    /// A baseline frame header segment (SOF0) with three components.
    pub fn jpeg_frame(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xC0, 0, 17, 8];
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    /// The PNG signature and an `IHDR` chunk for 8-bit RGB, with its CRC.
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let chunk = [
            b"IHDR".as_slice(),
            &width.to_be_bytes(),
            &height.to_be_bytes(),
            &[8, 2, 0, 0, 0],
        ]
        .concat();
        let crc = super::png::crc32(&chunk).to_be_bytes();
        [b"\x89PNG\r\n\x1A\n\0\0\0\x0D".as_slice(), &chunk, &crc].concat()
    }

    /// The `GIF89a` signature and a logical screen descriptor without a
    /// global colour table.
    pub fn gif(width: u16, height: u16) -> Vec<u8> {
        [
            b"GIF89a".as_slice(),
            &width.to_le_bytes(),
            &height.to_le_bytes(),
            &[0, 0, 0],
        ]
        .concat()
    }

    /// A GIF frame at `left`, `top` of `width × height`: an image descriptor
    /// without a local colour table, the LZW minimum code size, and an empty
    /// chain of image data sub-blocks.
    pub fn gif_frame(left: u16, top: u16, width: u16, height: u16) -> Vec<u8> {
        let fields = [left, top, width, height].map(u16::to_le_bytes);
        [[0x2C].as_slice(), &fields.concat(), &[0, 2, 0]].concat()
    }

    /// A RIFF/WebP header whose only chunk is `fourcc` with `payload`.
    pub fn webp(fourcc: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len()).unwrap();
        let riff_size = (size + 12).to_le_bytes();
        [
            b"RIFF".as_slice(),
            &riff_size,
            b"WEBP",
            &fourcc,
            &size.to_le_bytes(),
            payload,
        ]
        .concat()
    }

    /// A `VP8 ` chunk holding only a key frame header: a shown key frame of
    /// profile 0 with an empty first partition.
    pub fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
        let frame = [
            [0x10, 0, 0, 0x9D, 0x01, 0x2A].as_slice(),
            &width.to_le_bytes(),
            &height.to_le_bytes(),
        ];
        webp(*b"VP8 ", &frame.concat())
    }

    /// A `VP8L` chunk holding only the 5-byte lossless header (version 0).
    /// It stores each side minus 1, so both must be at least 1.
    pub fn webp_lossless(width: u16, height: u16) -> Vec<u8> {
        let bits = (u32::from(width) - 1) | ((u32::from(height) - 1) << 14);
        webp(*b"VP8L", &[[0x2F].as_slice(), &bits.to_le_bytes()].concat())
    }

    /// A `VP8X` chunk with no flags and the given canvas. It stores each side
    /// minus 1, so both must be at least 1.
    pub fn webp_extended(width: u32, height: u32) -> Vec<u8> {
        let side = |side: u32| (side - 1).to_le_bytes();
        webp(
            *b"VP8X",
            &[
                [0, 0, 0, 0].as_slice(),
                &side(width)[..3],
                &side(height)[..3],
            ]
            .concat(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*};

    #[test]
    fn reads_past_the_end_are_truncated() {
        let bytes = [1, 2, 3];
        assert_eq!(take(&bytes, 1, 2), Ok(&bytes[1..]));
        assert_eq!(take(&bytes, 3, 0), Ok(&[][..]));
        assert_eq!(take(&bytes, 2, 2), Err(HeaderError::Truncated));
        assert_eq!(take(&bytes, usize::MAX, 2), Err(HeaderError::Truncated));
        assert_eq!(bytes_at::<2>(&bytes, 1), Ok([2, 3]));
        assert_eq!(bytes_at::<4>(&bytes, 0), Err(HeaderError::Truncated));
        assert_eq!(
            dimensions(0, 7),
            Err(HeaderError::Corrupt(HeaderDefect::ZeroDimension))
        );
    }

    proptest! {
        #[test]
        fn take_returns_exactly_the_bytes_that_exist(
            bytes in vec(any::<u8>(), 0..32),
            at in any::<usize>(),
            len in 0..40_usize,
        ) {
            match take(&bytes, at, len) {
                Ok(taken) => {
                    prop_assert_eq!(taken.len(), len);
                    prop_assert_eq!(taken, &bytes[at..at + len]);
                }
                Err(error) => {
                    prop_assert_eq!(error, HeaderError::Truncated);
                    prop_assert!(at.saturating_add(len) > bytes.len());
                }
            }
        }

        #[test]
        fn dimensions_refuse_only_zero_sides(width in any::<u32>(), height in any::<u32>()) {
            prop_assert_eq!(dimensions(width, height).is_ok(), width != 0 && height != 0);
        }
    }
}
