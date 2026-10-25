//! The GIF reader: the logical screen, grown to hold every frame.
//!
//! A GIF declares its canvas in the Logical Screen Descriptor, right after
//! the signature (`GIF89a` §18), and each frame gives its own position and
//! size in an Image Descriptor (§20). Nothing in the format keeps a frame
//! inside the screen, and decoders treat such a frame differently: the `gif`
//! crate lets frames be "arbitrarily larger or offset in relation to the
//! screen" unless its caller asks for a check, and leaves clipping them or
//! growing the canvas to that caller (`check_frame_consistency` in
//! `gif@0.14.2:src/reader/mod.rs`). The screen alone therefore does not bound
//! what a decoder allocates. So the reader walks the blocks after the screen,
//! stepping over colour tables, extensions and image data by their sizes
//! without decoding them, and returns the smallest canvas, from the top-left
//! corner, that holds the screen and every frame. A screen with a zero side
//! gets its size from the frames the same way.
//!
//! The walk ends at the trailer or where the input ends: a body that is cut
//! short is the decoder's to refuse (see the parent module). A block that
//! starts with an unknown byte is refused, as the `gif` crate refuses it by
//! default: the format gives no length for such a block, so no reader can
//! tell where the next frame starts.

use super::{Dimensions, HeaderDefect, HeaderError, bytes_at, dimensions};

/// The byte that starts an extension block (`GIF89a` §23-26).
const EXTENSION: u8 = 0x21;

/// The byte that starts an image descriptor (`GIF89a` §20).
const IMAGE: u8 = 0x2C;

/// The trailer, the byte that ends a GIF (`GIF89a` §27).
const TRAILER: u8 = 0x3B;

/// Reads the dimensions of a GIF, whose first 6 bytes are `GIF87a` or
/// `GIF89a`: the logical screen, grown to hold every frame.
///
/// - `bytes`: the whole input.
pub(super) fn read(bytes: &[u8]) -> Result<Dimensions, HeaderError> {
    // The signature (6 bytes), then the logical screen descriptor: width (2),
    // height (2), packed fields, background colour index, aspect ratio.
    let [.., w0, w1, h0, h1, packed, _background, _aspect] = bytes_at::<13>(bytes, 0)?;
    let mut width = u32::from(u16::from_le_bytes([w0, w1]));
    let mut height = u32::from(u16::from_le_bytes([h0, h1]));
    let mut at = 13 + colour_table_len(packed);
    // One block per pass, until the trailer or the end of the input.
    while let Some(&introducer) = bytes.get(at) {
        match introducer {
            TRAILER => break,
            // The introducer, the label, then data sub-blocks.
            EXTENSION => at = skip_sub_blocks(bytes, at + 2),
            IMAGE => {
                // A descriptor that is cut short gives no frame to draw.
                let Ok([_, l0, l1, t0, t1, w0, w1, h0, h1, flags]) = bytes_at(bytes, at) else {
                    break;
                };
                // A side of u16 plus an offset of u16 always fits a u32.
                let reach = |offset, side| {
                    u32::from(u16::from_le_bytes(offset)) + u32::from(u16::from_le_bytes(side))
                };
                width = width.max(reach([l0, l1], [w0, w1]));
                height = height.max(reach([t0, t1], [h0, h1]));
                // The descriptor (10 bytes), its colour table, the LZW
                // minimum code size (1), then the image data.
                at = skip_sub_blocks(bytes, at + 10 + colour_table_len(flags) + 1);
            }
            other => return Err(HeaderDefect::GifBlock(other).into()),
        }
    }
    dimensions(width, height)
}

/// The length of the colour table that a packed field announces: none when
/// its top bit is clear, otherwise 2ⁿ⁺¹ entries of 3 bytes, where n is its
/// low 3 bits (`GIF89a` §18 and §20).
pub(in crate::domain::image_optimise) fn colour_table_len(packed: u8) -> usize {
    if packed & 0x80 == 0 {
        0
    } else {
        3 << ((packed & 0x07) + 1)
    }
}

/// The offset just past the chain of data sub-blocks that starts at `at`.
/// Each sub-block is a length byte and that many bytes, and a zero length
/// ends the chain (`GIF89a` §15). When the input ends first, the offset is at
/// or past its end, which ends the walk.
fn skip_sub_blocks(bytes: &[u8], mut at: usize) -> usize {
    while let Some(&len) = bytes.get(at) {
        at += 1 + usize::from(len);
        if len == 0 {
            break;
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::image_optimise::header::{HeaderDefect, samples};
    use proptest::{collection::vec, prelude::*};

    fn dims(bytes: &[u8]) -> Result<(u32, u32), HeaderError> {
        read(bytes).map(|d| (d.width(), d.height()))
    }

    /// A GIF with a screen of `width × height` and the given frames.
    fn with_frames(width: u16, height: u16, frames: &[[u16; 4]]) -> Vec<u8> {
        let frames = frames
            .iter()
            .map(|&[l, t, w, h]| samples::gif_frame(l, t, w, h));
        [samples::gif(width, height), frames.flatten().collect()].concat()
    }

    #[test]
    fn reads_the_logical_screen() {
        assert_eq!(dims(&samples::gif(67, 41)), Ok((67, 41)));
        let mut gif87a = samples::gif(u16::MAX, 2);
        gif87a[4] = b'7';
        assert_eq!(dims(&gif87a), Ok((65_535, 2)));
        // Frames inside the screen do not change it.
        let inside = with_frames(67, 41, &[[0, 0, 67, 41], [10, 20, 57, 21]]);
        assert_eq!(dims(&[&inside[..], &[TRAILER]].concat()), Ok((67, 41)));
    }

    #[test]
    fn a_frame_beyond_the_screen_grows_the_canvas() {
        // A few dozen bytes that declare more than 4 gigapixels.
        let huge = with_frames(1, 1, &[[0, 0, u16::MAX, u16::MAX]]);
        assert_eq!(dims(&huge), Ok((65_535, 65_535)));
        let offset = with_frames(67, 41, &[[0, 0, 67, 41], [60, 40, 10, 10]]);
        assert_eq!(dims(&offset), Ok((70, 50)));
        let far = with_frames(1, 1, &[[u16::MAX, u16::MAX, u16::MAX, u16::MAX]]);
        assert_eq!(dims(&far), Ok((131_070, 131_070)));
    }

    #[test]
    fn a_screen_with_a_zero_side_takes_its_size_from_the_frames() {
        assert_eq!(dims(&with_frames(0, 0, &[[0, 0, 67, 41]])), Ok((67, 41)));
        assert_eq!(dims(&with_frames(0, 41, &[[0, 0, 67, 1]])), Ok((67, 41)));
    }

    #[test]
    fn tables_extensions_and_image_data_are_stepped_over() {
        let bytes = [
            // A screen of 67x41 with a global colour table of 4 entries.
            &b"GIF89a\x43\0\x29\0\x81\0\0"[..],
            &[0x2C; 12],
            // A graphic control extension, then a comment in two sub-blocks.
            b"\x21\xF9\x04\0\0\0\0\0",
            b"\x21\xFE\x02\x2C\x2C\x01\x3B\0",
            // A frame with a local colour table of 2 entries, and image data
            // whose bytes look like blocks.
            b"\x2C\0\0\0\0\x43\0\x29\0\x80",
            &[0x2C; 6],
            b"\x02\x03\x2C\x3B\x21\x01\x00\0",
            // A second, larger frame, then the trailer and bytes after it.
            &with_frames(0, 0, &[[0, 0, 100, 50]])[13..],
            &[TRAILER],
            &samples::gif_frame(0, 0, u16::MAX, u16::MAX),
        ]
        .concat();
        assert_eq!(dims(&bytes), Ok((100, 50)));
    }

    #[test]
    fn an_unknown_block_is_refused() {
        for byte in [0x00, 0x2B, 0x3A, 0xFF] {
            let mut bytes = with_frames(67, 41, &[[0, 0, 67, 41]]);
            bytes.push(byte);
            assert_eq!(
                dims(&bytes),
                Err(HeaderError::Corrupt(HeaderDefect::GifBlock(byte))),
                "{byte:02X}"
            );
        }
    }

    #[test]
    fn a_zero_canvas_or_a_short_screen_is_refused() {
        assert_eq!(
            dims(&samples::gif(0, 41)),
            Err(HeaderError::Corrupt(HeaderDefect::ZeroDimension))
        );
        assert_eq!(
            dims(&with_frames(0, 0, &[[0, 0, 0, 0]])),
            Err(HeaderError::Corrupt(HeaderDefect::ZeroDimension))
        );
        assert_eq!(
            dims(&samples::gif(67, 41)[..12]),
            Err(HeaderError::Truncated)
        );
    }

    #[test]
    fn colour_tables_have_three_bytes_per_entry() {
        assert_eq!(colour_table_len(0x7F), 0);
        assert_eq!(colour_table_len(0x80), 6);
        assert_eq!(colour_table_len(0x81), 12);
        assert_eq!(colour_table_len(0xF7), 768);
    }

    proptest! {
        #[test]
        fn any_screen_reads_back(width in any::<u16>(), height in any::<u16>(), rest in any::<[u8; 3]>()) {
            let mut bytes = samples::gif(width, height);
            bytes[10..].copy_from_slice(&rest);
            let expected = (width != 0 && height != 0).then_some((width.into(), height.into()));
            prop_assert_eq!(dims(&bytes).ok(), expected);
        }

        #[test]
        fn the_canvas_holds_the_screen_and_every_frame(
            screen in any::<[u16; 2]>(),
            frames in vec(any::<[u16; 4]>(), 0..5),
            trailer in any::<bool>(),
        ) {
            let mut bytes = with_frames(screen[0], screen[1], &frames);
            if trailer {
                bytes.push(TRAILER);
            }
            let reach = |side: usize| {
                frames
                    .iter()
                    .map(|frame| u32::from(frame[side]) + u32::from(frame[side + 2]))
                    .fold(u32::from(screen[side]), u32::max)
            };
            let expected = (reach(0) != 0 && reach(1) != 0).then_some((reach(0), reach(1)));
            prop_assert_eq!(dims(&bytes).ok(), expected);
        }

        #[test]
        fn any_blocks_after_the_screen_never_panic(tail in vec(any::<u8>(), 0..200)) {
            let screen = samples::gif(67, 41);
            match dims(&[&screen[..], &tail].concat()) {
                Ok((width, height)) => prop_assert!(width >= 67 && height >= 41),
                Err(error) => prop_assert!(matches!(
                    error,
                    HeaderError::Corrupt(HeaderDefect::GifBlock(_))
                )),
            }
        }

        #[test]
        fn a_cut_short_body_reads_the_frames_it_holds(
            frames in vec(any::<[u16; 4]>(), 1..4),
            cut in any::<prop::sample::Index>(),
        ) {
            // Cutting a GIF never makes the canvas larger than the whole file's.
            let bytes = with_frames(1, 1, &frames);
            let whole = dims(&bytes).unwrap();
            let part = dims(&bytes[..13 + cut.index(bytes.len() - 12)]).unwrap();
            prop_assert!(part.0 <= whole.0 && part.1 <= whole.1);
        }
    }
}
