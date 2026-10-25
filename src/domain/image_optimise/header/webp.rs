//! The WebP reader: the dimensions in the first chunk of the RIFF container
//! (RFC 9649).
//!
//! The first chunk is one of three: `VP8 ` holds a lossy image, `VP8L` a
//! lossless one, and `VP8X` starts the extended format (alpha, animation or
//! metadata) and gives the canvas that every frame is composed onto. The
//! reader makes the RIFF, VP8, VP8L and VP8X header checks that libwebp makes
//! before it decodes (`ParseHeadersInternal` and `VP8GetInfo` in
//! `libwebp@1844042:src/dec/`). It stops at the `VP8X` chunk, so two of
//! libwebp's later checks are left to the decoder: that a still image inside
//! a `VP8X` file has the canvas size, and that every animation frame fits the
//! canvas (`CheckFrameBounds` in `src/demux/demux.c`).

use super::{Dimensions, HeaderDefect, HeaderError, bytes_at, dimensions};

/// Where the first chunk's payload starts: after `RIFF`, the RIFF size,
/// `WEBP`, and the chunk's `FourCC` and size.
const PAYLOAD: usize = 20;

/// The largest RIFF size that libwebp accepts (`MAX_CHUNK_PAYLOAD`).
const MAX_RIFF_SIZE: u32 = u32::MAX - 9;

/// Reads the dimensions of a WebP, whose first 12 bytes are `RIFF`, the RIFF
/// size and `WEBP`.
///
/// - `bytes`: the whole input.
pub(super) fn read(bytes: &[u8]) -> Result<Dimensions, HeaderError> {
    let riff_size = u32::from_le_bytes(bytes_at(bytes, 4)?);
    let [f0, f1, f2, f3, s0, s1, s2, s3] = bytes_at(bytes, 12)?;
    let chunk_size = u32::from_le_bytes([s0, s1, s2, s3]);
    // The RIFF size counts `WEBP` and at least one chunk header (12 bytes),
    // and the first chunk must fit in it.
    if !(12..=MAX_RIFF_SIZE).contains(&riff_size) || chunk_size > riff_size - 12 {
        return Err(HeaderDefect::WebpChunkSize.into());
    }
    match &[f0, f1, f2, f3] {
        b"VP8 " => lossy(bytes, chunk_size),
        b"VP8L" => lossless(bytes, chunk_size),
        b"VP8X" => extended(bytes, chunk_size),
        _ => Err(HeaderDefect::WebpFirstChunk.into()),
    }
}

/// A VP8 key frame header (RFC 6386 §9.1): a 3-byte frame tag, the start
/// code, then the 14-bit width and height. The top 2 bits of each are a
/// scaling hint that decoders do not apply, so they are ignored.
fn lossy(bytes: &[u8], chunk_size: u32) -> Result<Dimensions, HeaderError> {
    if chunk_size < 10 {
        return Err(HeaderDefect::WebpChunkSize.into());
    }
    let [t0, t1, t2, c0, c1, c2, w0, w1, h0, h1] = bytes_at(bytes, PAYLOAD)?;
    // The tag: key frame flag (0 for a key frame), 3 bits of profile, the
    // show flag, then 19 bits of first-partition size.
    let tag = u32::from_le_bytes([t0, t1, t2, 0]);
    let key_frame = tag & 1 == 0;
    let shown = (tag >> 4) & 1 == 1;
    if !key_frame || (tag >> 1) & 7 > 3 || !shown || tag >> 5 >= chunk_size {
        return Err(HeaderDefect::Vp8FrameTag.into());
    }
    if [c0, c1, c2] != [0x9D, 0x01, 0x2A] {
        return Err(HeaderDefect::Vp8StartCode.into());
    }
    let side = |low, high| u32::from(u16::from_le_bytes([low, high]) & 0x3FFF);
    dimensions(side(w0, w1), side(h0, h1))
}

/// A VP8L header: the signature byte `2F`, then 14 bits of width − 1,
/// 14 bits of height − 1, an alpha hint and a 3-bit version that must be 0.
fn lossless(bytes: &[u8], chunk_size: u32) -> Result<Dimensions, HeaderError> {
    if chunk_size < 5 {
        return Err(HeaderDefect::WebpChunkSize.into());
    }
    let [signature, b0, b1, b2, b3] = bytes_at(bytes, PAYLOAD)?;
    let bits = u32::from_le_bytes([b0, b1, b2, b3]);
    if signature != 0x2F || bits >> 29 != 0 {
        return Err(HeaderDefect::Vp8lHeader.into());
    }
    dimensions((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1)
}

/// A VP8X chunk: flags (1 byte), reserved (3), then the canvas width − 1 and
/// height − 1 in 24 bits each. libwebp refuses a canvas of 2³² pixels or more.
fn extended(bytes: &[u8], chunk_size: u32) -> Result<Dimensions, HeaderError> {
    if chunk_size != 10 {
        return Err(HeaderDefect::WebpChunkSize.into());
    }
    let [_flags, _, _, _, w0, w1, w2, h0, h1, h2] = bytes_at(bytes, PAYLOAD)?;
    let width = u32::from_le_bytes([w0, w1, w2, 0]) + 1;
    let height = u32::from_le_bytes([h0, h1, h2, 0]) + 1;
    if u64::from(width) * u64::from(height) >= 1 << 32 {
        return Err(HeaderDefect::DimensionTooLarge.into());
    }
    dimensions(width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::image_optimise::header::samples;
    use proptest::prelude::*;

    fn dims(bytes: &[u8]) -> Result<(u32, u32), HeaderError> {
        read(bytes).map(|d| (d.width(), d.height()))
    }

    fn corrupt(defect: HeaderDefect) -> Result<(u32, u32), HeaderError> {
        Err(HeaderError::Corrupt(defect))
    }

    /// `bytes` with the byte at `at` set to `byte`.
    fn with(mut bytes: Vec<u8>, at: usize, byte: u8) -> Vec<u8> {
        bytes[at] = byte;
        bytes
    }

    #[test]
    fn reads_each_kind_of_first_chunk() {
        assert_eq!(dims(&samples::webp_lossy(67, 41)), Ok((67, 41)));
        assert_eq!(dims(&samples::webp_lossy(0x3FFF, 1)), Ok((16_383, 1)));
        assert_eq!(dims(&samples::webp_lossless(67, 41)), Ok((67, 41)));
        assert_eq!(
            dims(&samples::webp_lossless(0x4000, 0x4000)),
            Ok((16_384, 16_384))
        );
        assert_eq!(dims(&samples::webp_extended(67, 41)), Ok((67, 41)));
        assert_eq!(
            dims(&samples::webp_extended(1 << 24, 255)),
            Ok((16_777_216, 255))
        );
        // The animation and alpha flags do not change the canvas.
        assert_eq!(
            dims(&with(samples::webp_extended(67, 41), 20, 0x12)),
            Ok((67, 41))
        );
    }

    #[test]
    fn vp8_scaling_bits_are_ignored() {
        let scaled = with(with(samples::webp_lossy(67, 41), 27, 0xC0), 29, 0x40);
        assert_eq!(dims(&scaled), Ok((67, 41)));
        assert_eq!(
            dims(&samples::webp_lossy(0xC000, 41)),
            corrupt(HeaderDefect::ZeroDimension)
        );
    }

    #[test]
    fn invalid_vp8_frame_tags_are_refused() {
        let sample = samples::webp_lossy(67, 41);
        // An interframe; profile 4; a hidden frame; a first partition of 16 bytes in a 10-byte chunk.
        for (at, byte) in [(20, 0x11), (20, 0x18), (20, 0x00), (21, 0x02)] {
            assert_eq!(
                dims(&with(sample.clone(), at, byte)),
                corrupt(HeaderDefect::Vp8FrameTag),
                "{at}: {byte:02X}"
            );
        }
        // Profile 3 with a first partition of 1 byte is fine.
        assert_eq!(dims(&with(sample.clone(), 20, 0x16 | 0x20)), Ok((67, 41)));
        // The first partition must be smaller than the chunk, as in libwebp:
        // 10 bytes in this 10-byte chunk are refused, 9 are not.
        let partition = |size: u32| {
            let tag = ((size << 5) | 0x10).to_le_bytes();
            with(with(sample.clone(), 20, tag[0]), 21, tag[1])
        };
        assert_eq!(dims(&partition(10)), corrupt(HeaderDefect::Vp8FrameTag));
        assert_eq!(dims(&partition(9)), Ok((67, 41)));
        assert_eq!(
            dims(&with(sample, 24, 0x02)),
            corrupt(HeaderDefect::Vp8StartCode)
        );
    }

    #[test]
    fn invalid_vp8l_headers_are_refused() {
        let sample = samples::webp_lossless(67, 41);
        assert_eq!(
            dims(&with(sample.clone(), 20, 0x2E)),
            corrupt(HeaderDefect::Vp8lHeader)
        );
        assert_eq!(
            dims(&with(sample, 24, 0x20)),
            corrupt(HeaderDefect::Vp8lHeader)
        );
    }

    #[test]
    fn inconsistent_sizes_are_refused() {
        let chunk = |fourcc: &[u8; 4], len: usize| samples::webp(*fourcc, &vec![0; len]);
        assert_eq!(
            dims(&chunk(b"VP8 ", 9)),
            corrupt(HeaderDefect::WebpChunkSize)
        );
        assert_eq!(
            dims(&chunk(b"VP8L", 4)),
            corrupt(HeaderDefect::WebpChunkSize)
        );
        assert_eq!(
            dims(&chunk(b"VP8X", 11)),
            corrupt(HeaderDefect::WebpChunkSize)
        );
        let lossy = samples::webp_lossy(67, 41);
        let with_riff_size = |riff_size: u32| {
            let mut bytes = lossy.clone();
            bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
            bytes
        };
        // A RIFF size below 12, above libwebp's maximum (MAX_CHUNK_PAYLOAD,
        // 2³² − 10, written out so a wrong constant fails), or smaller than
        // the first chunk.
        for riff_size in [11_u32, u32::MAX - 8, 21] {
            assert_eq!(
                dims(&with_riff_size(riff_size)),
                corrupt(HeaderDefect::WebpChunkSize),
                "{riff_size}"
            );
        }
        // libwebp's maximum itself, and a RIFF size that just holds the chunk.
        // The RIFF size is not checked against the input: a body that is cut
        // short is left to the decoder.
        for riff_size in [u32::MAX - 9, 22] {
            assert_eq!(
                dims(&with_riff_size(riff_size)),
                Ok((67, 41)),
                "{riff_size}"
            );
        }
    }

    #[test]
    fn other_first_chunks_are_refused() {
        for fourcc in [*b"ALPH", *b"ANIM", *b"VP8l", *b"EXIF"] {
            assert_eq!(
                dims(&samples::webp(fourcc, &[0; 10])),
                corrupt(HeaderDefect::WebpFirstChunk)
            );
        }
    }

    #[test]
    fn a_canvas_of_2_to_the_32_pixels_is_refused() {
        assert_eq!(
            dims(&samples::webp_extended(1 << 16, 1 << 16)),
            corrupt(HeaderDefect::DimensionTooLarge)
        );
        assert_eq!(
            dims(&samples::webp_extended(1 << 16, (1 << 16) - 1)),
            Ok((1 << 16, (1 << 16) - 1))
        );
    }

    proptest! {
        #[test]
        fn lossy_sides_are_the_low_14_bits(width in 1..=u16::MAX, height in 1..=u16::MAX) {
            let expected = (u32::from(width & 0x3FFF), u32::from(height & 0x3FFF));
            let result = dims(&samples::webp_lossy(width, height));
            if expected.0 == 0 || expected.1 == 0 {
                prop_assert_eq!(result, corrupt(HeaderDefect::ZeroDimension));
            } else {
                prop_assert_eq!(result, Ok(expected));
            }
        }

        #[test]
        fn any_first_chunk_never_panics(
            fourcc in any::<[u8; 4]>(),
            payload in proptest::collection::vec(any::<u8>(), 0..24),
        ) {
            let named = [*b"VP8 ", *b"VP8L", *b"VP8X"];
            let result = dims(&samples::webp(fourcc, &payload));
            if !named.contains(&fourcc) {
                prop_assert_eq!(result, corrupt(HeaderDefect::WebpFirstChunk));
            }
        }
    }
}
