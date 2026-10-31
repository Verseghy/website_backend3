//! GIF uploads: the metadata strip (ADR 0015 §7: "Metadata is removed.
//! Pixels and animation are not touched.", amended by ADR 0033 §3 and §4).
//!
//! A GIF is a header, a logical screen and its colour table, then a sequence
//! of blocks up to the trailer (`GIF89a` §15-27). [`optimise_gif`] walks the
//! blocks and writes a new file from the ones that make up the picture and
//! its timing:
//!
//! | Block | In the stored file |
//! |---|---|
//! | header, logical screen, global colour table | copied |
//! | image: descriptor, local colour table, LZW data | copied, so every frame keeps its bytes |
//! | graphic control extension | the delay, disposal and transparency of the next frame, as one 4-byte sub-block: copied when it is written so, which every encoder does |
//! | application extension `NETSCAPE2.0` or `ANIMEXTS1.0` | rewritten with its loop sub-block alone when that is its first sub-block, dropped otherwise |
//! | application extension `ICCRGBG1012` | rewritten with the ICC profile alone, which ADR 0015 §7 keeps |
//! | comment, other application extensions (XMP, IPTC), plain text, unknown labels | dropped |
//! | the trailer | copied when the upload has one; anything after it is dropped |
//!
//! The kept blocks are copied as they are: the strip removes the blocks that
//! viewers do not show, and does not look inside the image data or the
//! profile of the ones they do.
//!
//! **Read as the `gif` crate reads it** (ADR 0033 §3, decision D78). The
//! check decode uses the `gif` crate, the decoder that the header reader
//! follows, and the strip accepts what it decodes:
//!
//! - **The trailer may be missing.** The `gif` crate decodes every frame of a
//!   GIF that ends after a whole block instead of at its trailer, and only
//!   then reports the end of the input, as browsers show such a file. The
//!   strip accepts it once it holds an image, and stores it without a
//!   trailer, as uploaded, so it never grows; a GIF that ends inside a block,
//!   or before its first image, is refused, as the `gif` crate refuses it.
//! - **A graphic control extension** counts with its last data sub-block,
//!   which the `gif` crate requires to hold exactly 4 bytes, whatever comes
//!   before it. The strip writes it as that one sub-block, which every
//!   decoder reads, and refuses any other last sub-block, as the `gif` crate
//!   does.
//! - **The loop count** is read from the first data sub-block after the
//!   application identifier, and only when that is a loop sub-block of 3
//!   bytes (id 1 and a 16-bit count), as the `gif` crate reads it, and
//!   Chromium's decoder, Wuffs, too
//!   (`wuffs@4be865da:std/gif/decode_gif.wuffs`): a loop sub-block after
//!   another one, such as Netscape's buffering hint, sets no loop count. The
//!   strip keeps that first sub-block alone and drops an extension whose
//!   first sub-block is anything else, which both read as no loop count.
//!   Firefox also reads a later loop sub-block
//!   (`mozilla-central@19272f1f7a2c:image/decoders/nsGIFDecoder2.cpp`), so it
//!   loops such an upload and plays its stored file once, while Chromium
//!   plays both once. The `gif` crate reads only `NETSCAPE2.0`; Wuffs and
//!   Firefox also read `ANIMEXTS1.0`, which some older encoders write, so it
//!   is kept by the same rule.
//! - **Plain text extensions** are a rendering block in `GIF89a` §25, but no
//!   browser draws them, and the `gif` crate and the browsers' decoders skip
//!   them, so they are text that no viewer shows. They apply a graphic
//!   control extension that precedes one to the next image instead, so the
//!   control extension stays where it is and the animation keeps its timing.
//! - **A GIF with no image** is refused, with or without its trailer: no
//!   viewer shows an image of it. The `gif` crate accepts some such files
//!   and refuses others, depending on the block before the trailer (it looks
//!   one byte past the logical screen for a block), so following it there
//!   would accept or refuse the same empty picture by chance.
//!
//! **CPU limits** (ADR 0033 §4, decision D79). The check decodes every frame,
//! so its time grows with the frames and their pixels, which the per-canvas
//! budget does not bound. Before anything is decoded, the frames that the
//! block structure declares are held to [`super::MAX_ANIMATION_FRAMES`] and
//! [`super::MAX_ANIMATION_PIXELS`], each frame at the size its descriptor
//! gives, the same totals and the same refusal as for WebP. The walk stops
//! at the descriptor that passes a limit, so the stripped copy of a crafted
//! file never grows past the frames within the limits.
//!
//! Decoding is pure computation over the bytes, so this module stays in
//! `src/domain/` (ADR 0008).

use core::num::NonZeroU64;
use std::io::Read as _;

use super::header::colour_table_len;
use super::outcome::{check_animation_size, inspect_as};
use super::{DecodeDefect, ImageError, ImageFormat, Optimised, PixelBudget, Treatment};

/// The byte that starts an extension block (`GIF89a` §23-26).
const EXTENSION: u8 = 0x21;

/// The byte that starts an image descriptor (`GIF89a` §20).
const IMAGE: u8 = 0x2C;

/// The trailer, the byte that ends a GIF (`GIF89a` §27).
const TRAILER: u8 = 0x3B;

/// The label of a graphic control extension (`GIF89a` §23).
const GRAPHIC_CONTROL: u8 = 0xF9;

/// The label of an application extension (`GIF89a` §26).
const APPLICATION: u8 = 0xFF;

/// The application identifiers and authentication codes of the loop
/// extension: Netscape's, and the one some older encoders write, which
/// browsers read the same way.
const LOOP_APPLICATIONS: [&[u8]; 2] = [b"NETSCAPE2.0", b"ANIMEXTS1.0"];

/// The application identifier and authentication code of an embedded ICC
/// profile (ICC, "Embedding ICC profiles in GIF files"); the `gif` crate
/// reads it as the image's profile.
const ICC_APPLICATION: &[u8] = b"ICCRGBG1012";

/// The stripped copy of a GIF, and what its block structure declares.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stripped {
    /// The bytes to store, never more than the input.
    bytes: Vec<u8>,
    /// Whether the input, and so `bytes`, ends with the trailer.
    trailer: bool,
    /// The number of image descriptors: the frames.
    frames: u64,
    /// The pixels of all frames, each at the size its descriptor gives.
    pixels: u64,
}

/// Optimises a GIF upload: strips its metadata, keeps every frame byte for
/// byte, holds its frames to the animation limits, and checks that the
/// result decodes.
///
/// - `bytes`: the whole upload.
/// - `budget`: the pixel budget for the canvas, which holds every frame; it
///   also bounds the memory the check decode may use for one frame.
///
/// # Errors
///
/// The errors of [`super::inspect`], which runs first;
/// [`ImageError::WrongFormat`] when the upload is a JPEG, PNG or WebP;
/// [`ImageError::AnimationTooLarge`] when it declares more frames or pixels
/// than ADR 0033 §4 allows; and [`ImageError::Undecodable`] when a block is
/// malformed or cut short as the `gif` crate reads it, the GIF has no image
/// ([`DecodeDefect::GifNoImage`]), or a frame does not decode
/// ([`DecodeDefect::GifData`]).
pub fn optimise_gif(bytes: &[u8], budget: PixelBudget) -> Result<Optimised, ImageError> {
    let header = inspect_as(bytes, budget, ImageFormat::Gif)?;
    // The animation limits hold while the blocks are walked, before any
    // frame is decoded (ADR 0033 §4).
    let stripped = strip(bytes)?;
    check_decodes(&stripped, budget)?;
    Ok(Optimised {
        header,
        bytes: stripped.bytes,
        treatment: Treatment::Stripped,
        re_encode_failure: None,
    })
}

/// An [`ImageError::Undecodable`] GIF for `defect`.
///
/// - `defect`: what the walk over the blocks or the decode found.
fn undecodable(defect: DecodeDefect) -> ImageError {
    ImageError::Undecodable {
        format: ImageFormat::Gif,
        defect,
    }
}

/// The stripped copy of a GIF, block by block, as the module documentation
/// lists them, with the frames and pixels it declares.
///
/// - `bytes`: the whole input, whose signature [`super::sniff`] matched.
///
/// # Errors
///
/// [`ImageError::Undecodable`] for a block that is malformed or cut short
/// as the `gif` crate reads it and for a GIF without an image, and
/// [`ImageError::AnimationTooLarge`] at the image descriptor that passes
/// a limit of ADR 0033 §4.
fn strip(bytes: &[u8]) -> Result<Stripped, ImageError> {
    let cut_short = || undecodable(DecodeDefect::GifCutShort);
    // The signature (6 bytes), the logical screen descriptor (7) and the
    // global colour table that its packed field announces.
    let screen_end = 13 + colour_table_len(*bytes.get(10).ok_or_else(cut_short)?);
    let mut stripped = Stripped {
        bytes: bytes.get(..screen_end).ok_or_else(cut_short)?.to_vec(),
        trailer: false,
        frames: 0,
        pixels: 0,
    };
    let out = &mut stripped.bytes;
    let mut at = screen_end;
    loop {
        let introducer = bytes.get(at).copied();
        // The end of the input where a block would start, or the trailer:
        // the `gif` crate has decoded every frame by then (module
        // documentation), and a GIF with none has no image to show.
        if matches!(introducer, None | Some(TRAILER)) && stripped.frames == 0 {
            return Err(undecodable(DecodeDefect::GifNoImage));
        }
        match introducer {
            None => return Ok(stripped),
            Some(TRAILER) => {
                out.push(TRAILER);
                stripped.trailer = true;
                return Ok(stripped);
            }
            Some(IMAGE) => {
                // The descriptor (10 bytes: position, size, then its packed
                // field), its colour table, the LZW minimum code size, then
                // the image data.
                let descriptor = bytes.get(at..at + 10).ok_or_else(cut_short)?;
                let side =
                    |at: usize| u64::from(u16::from_le_bytes([descriptor[at], descriptor[at + 1]]));
                stripped.frames += 1;
                stripped.pixels = stripped.pixels.saturating_add(side(5) * side(7));
                // At the descriptor that passes a limit, before its data is
                // read or copied (ADR 0033 §4).
                check_animation_size(ImageFormat::Gif, stripped.frames, stripped.pixels)?;
                let end = chain_end(bytes, at + 10 + colour_table_len(descriptor[9]) + 1)
                    .map_err(undecodable)?;
                out.extend_from_slice(&bytes[at..end]);
                at = end;
            }
            Some(EXTENSION) => {
                let label = *bytes.get(at + 1).ok_or_else(cut_short)?;
                let end = chain_end(bytes, at + 2).map_err(undecodable)?;
                let chain = &bytes[at + 2..end];
                match label {
                    GRAPHIC_CONTROL => out.extend(graphic_control(chain).map_err(undecodable)?),
                    APPLICATION => out.extend(application(chain)),
                    _ => {}
                }
                at = end;
            }
            Some(_) => return Err(undecodable(DecodeDefect::GifBlock)),
        }
    }
}

/// The offset just past the chain of data sub-blocks that starts at `at`:
/// each a length byte and that many bytes, until a zero length (`GIF89a`
/// §15).
///
/// - `bytes`: the whole input.
/// - `at`: the offset of the first length byte.
fn chain_end(bytes: &[u8], mut at: usize) -> Result<usize, DecodeDefect> {
    loop {
        let len = usize::from(*bytes.get(at).ok_or(DecodeDefect::GifCutShort)?);
        at += 1 + len;
        if len == 0 {
            return Ok(at);
        }
        // A chain cut short leaves `at` at or past the end, where the next
        // pass finds no length byte and refuses it.
    }
}

/// The stored form of a graphic control extension: its last data
/// sub-block, which is the one the `gif` crate reads, as the only one.
///
/// - `chain`: the extension's sub-blocks after its label, terminator
///   included.
///
/// # Errors
///
/// [`DecodeDefect::GifGraphicControl`] when the last sub-block does not hold exactly 4 bytes,
/// or there is none, as the `gif` crate refuses it.
fn graphic_control(chain: &[u8]) -> Result<[u8; 8], DecodeDefect> {
    let control: [u8; 4] = sub_blocks(chain)
        .last()
        .and_then(|block| block.try_into().ok())
        .ok_or(DecodeDefect::GifGraphicControl)?;
    let [flags, delay_low, delay_high, transparent] = control;
    Ok([
        EXTENSION,
        GRAPHIC_CONTROL,
        4,
        flags,
        delay_low,
        delay_high,
        transparent,
        0,
    ])
}

/// The data of each sub-block of a complete chain, terminator excluded.
///
/// - `chain`: a chain whose end [`chain_end`] found.
fn sub_blocks(chain: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = chain;
    core::iter::from_fn(move || {
        let (&len, tail) = rest.split_first()?;
        let (data, after) = tail.split_at_checked(usize::from(len))?;
        rest = after;
        (len != 0).then_some(data)
    })
}

/// The stored form of an application extension: the loop sub-block of a
/// loop extension when it is the first sub-block, the only one the `gif`
/// crate reads (`read_application_extension` in
/// `gif@0.14.2:src/reader/mod.rs`), the profile of an ICC extension, and
/// nothing for any other application, such as XMP or IPTC.
///
/// - `chain`: the extension's sub-blocks after its label, terminator
///   included. The first sub-block holds the 11 bytes of the identifier and
///   the authentication code.
fn application(chain: &[u8]) -> Vec<u8> {
    let mut blocks = sub_blocks(chain);
    let Some(identifier) = blocks.next().filter(|id| id.len() == 11) else {
        return Vec::new();
    };
    let mut out = [&[EXTENSION, APPLICATION, 11][..], identifier].concat();
    if LOOP_APPLICATIONS.contains(&identifier) {
        // The loop sub-block: id 1 and a 16-bit loop count, 3 bytes in all.
        // The gif crate reads the first sub-block only, and only in that
        // form, so a later one, after Netscape's buffering hint for example,
        // sets no loop count; an extension without it says nothing.
        let Some(repeat) = blocks
            .next()
            .filter(|block| block.len() == 3 && block[0] == 1)
        else {
            return Vec::new();
        };
        out.push(3);
        out.extend_from_slice(repeat);
    } else if identifier == ICC_APPLICATION {
        let profile: Vec<u8> = blocks.flatten().copied().collect();
        if profile.is_empty() {
            return Vec::new();
        }
        for piece in profile.chunks(255) {
            // `chunks(255)` makes every piece at most 255 bytes long.
            out.push(u8::try_from(piece.len()).unwrap_or(u8::MAX));
            out.extend_from_slice(piece);
        }
    } else {
        return Vec::new();
    }
    out.push(0);
    out
}

/// Decodes every frame of the stripped GIF with the `gif` crate, so a GIF
/// whose image data does not decode is refused (ADR 0015 §7).
///
/// A GIF without its trailer is read with one appended after the last
/// block, without copying it: the `gif` crate has decoded every frame when
/// it finds the input's end there (module documentation), so this changes
/// nothing but the end of the stream it reports.
///
/// - `stripped`: the stripped GIF.
/// - `budget`: the pixel budget. Frames are decoded to one byte per pixel,
///   so it is also the memory limit for one frame.
///
/// # Errors
///
/// [`DecodeDefect::GifData`] when the `gif` crate does not decode a frame.
/// Its own reason is not kept: as JPEG's and PNG's decode checks do, the
/// refusal names the decoder that refused the data.
fn check_decodes(stripped: &Stripped, budget: PixelBudget) -> Result<(), ImageError> {
    let undecodable = |_: ::gif::DecodingError| undecodable(DecodeDefect::GifData);
    let mut options = ::gif::DecodeOptions::new();
    options.set_color_output(::gif::ColorOutput::Indexed);
    // The gif crate's own default is 50,000,000 bytes, which would refuse a
    // frame that a larger budget admits. The header check has held the
    // canvas, which holds every frame, to the budget already, so this limit
    // never refuses a frame within it; it also bounds the buffers the crate
    // grows, such as the ICC profile. A budget admits at least one pixel, so
    // the limit is never zero.
    let limit = NonZeroU64::new(budget.max_pixels()).unwrap_or(NonZeroU64::MIN);
    options.set_memory_limit(::gif::MemoryLimit::Bytes(limit));
    let end: &[u8] = if stripped.trailer { &[] } else { &[TRAILER] };
    let mut decoder = options
        .read_info(stripped.bytes.as_slice().chain(end))
        .map_err(undecodable)?;
    while decoder.read_next_frame().map_err(undecodable)?.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::header::samples;
    use super::super::{Dimensions, ImageError, MAX_ANIMATION_FRAMES, MAX_ANIMATION_PIXELS};
    use super::*;
    use proptest::{collection::vec, prelude::*};

    /// A `width`×`height` frame at the origin whose LZW data holds one pixel:
    /// the descriptor, a minimum code size of 2, then one sub-block with the
    /// codes clear (4), colour 0 and end (5), 3 bits each, packed from the
    /// lowest bit: `0x44 0x01`.
    fn frame_of(width: u16, height: u16) -> Vec<u8> {
        let ([w0, w1], [h0, h1]) = (width.to_le_bytes(), height.to_le_bytes());
        vec![0x2C, 0, 0, 0, 0, w0, w1, h0, h1, 0, 2, 2, 0x44, 0x01, 0]
    }

    /// A 1×1 frame whose LZW data decodes.
    fn pixel_frame() -> Vec<u8> {
        frame_of(1, 1)
    }

    /// The 1×1 logical screen with a two-entry global colour table.
    const SCREEN: &[u8] = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xFF\xFF\xFF";

    /// A 1×1 `GIF89a` with a two-entry global colour table, `blocks` before its
    /// frame, then `after` after the frame and the trailer.
    fn gif_with(blocks: &[&[u8]], after: &[u8]) -> Vec<u8> {
        [SCREEN, &blocks.concat(), &pixel_frame(), &[TRAILER], after].concat()
    }

    const CONTROL: &[u8] = b"\x21\xF9\x04\x04\x0A\0\0\0";
    const COMMENT: &[u8] = b"\x21\xFE\x05hello\x03abc\0";
    const LOOP: &[u8] = b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\x05\0\0";
    const PLAIN_TEXT: &[u8] = b"\x21\x01\x0C\0\0\0\0\x01\0\x01\0\x01\x01\x01\0\x02hi\0";

    fn optimise(bytes: &[u8]) -> Result<Optimised, ImageError> {
        optimise_gif(bytes, PixelBudget::DEFAULT)
    }

    fn stripped(bytes: &[u8]) -> Result<Vec<u8>, ImageError> {
        strip(bytes).map(|stripped| stripped.bytes)
    }

    fn corrupt(defect: DecodeDefect) -> Result<Optimised, ImageError> {
        Err(ImageError::Undecodable {
            format: ImageFormat::Gif,
            defect,
        })
    }

    /// The loop count that the `gif` crate reads from `bytes`, after every
    /// frame, or `None` when it refuses the file.
    fn gif_repeat(bytes: &[u8]) -> Option<::gif::Repeat> {
        let mut decoder = ::gif::DecodeOptions::new().read_info(bytes).ok()?;
        while decoder.read_next_frame().ok()?.is_some() {}
        Some(decoder.repeat())
    }

    /// The delays of the frames that the `gif` crate decodes from `bytes`,
    /// and the error that ends the stream, if one does.
    fn gif_crate(bytes: &[u8]) -> (Vec<u16>, Option<String>) {
        let mut delays = Vec::new();
        let mut decoder = match ::gif::DecodeOptions::new().read_info(bytes) {
            Ok(decoder) => decoder,
            Err(error) => return (delays, Some(error.to_string())),
        };
        loop {
            match decoder.read_next_frame() {
                Ok(Some(frame)) => delays.push(frame.delay),
                Ok(None) => return (delays, None),
                Err(error) => return (delays, Some(error.to_string())),
            }
        }
    }

    #[test]
    fn keeps_frames_control_and_loop_and_drops_the_rest() {
        let xmp = [b"\x21\xFF\x0BXMP DataXMP\x04<x/>".as_slice(), &[1, 0xFF, 0]].concat();
        let unknown = b"\x21\x77\x02ab\0";
        let input = gif_with(
            &[LOOP, COMMENT, &xmp, PLAIN_TEXT, unknown, CONTROL],
            b"PK\x03\x04payload",
        );
        let optimised = optimise(&input).unwrap();
        assert_eq!(optimised.bytes, gif_with(&[LOOP, CONTROL], b""));
        assert_eq!(optimised.treatment, Treatment::Stripped);
        assert_eq!(optimised.header.dimensions, Dimensions::new(1, 1).unwrap());
    }

    #[test]
    fn rewrites_the_loop_extension_with_its_first_sub_block_alone() {
        // The loop sub-block first, Netscape's buffering hint after it: the
        // gif crate reads 5 loops from both.
        let first = b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\x05\0\x05\x02\0\x10\0\0\x02xy\0";
        let output = stripped(&gif_with(&[first], b"")).unwrap();
        assert_eq!(output, gif_with(&[LOOP], b""));
        assert_eq!(gif_repeat(&output), Some(::gif::Repeat::Finite(5)));
        assert_eq!(gif_repeat(&output), gif_repeat(&gif_with(&[first], b"")));
        let animexts = b"\x21\xFF\x0BANIMEXTS1.0\x03\x01\x02\0\0";
        assert_eq!(
            stripped(&gif_with(&[animexts], b"")).unwrap(),
            gif_with(&[animexts], b"")
        );
        // A first sub-block that is not 3 bytes of id 1 sets no loop count
        // for the gif crate, also with a loop sub-block after it, so the
        // extension says nothing and goes: the gif crate reads Finite(0),
        // play once, from the upload and the stored file alike.
        for no_loop in [
            &b"\x21\xFF\x0BNETSCAPE2.0\x05\x02\0\x10\0\0\x03\x01\x05\0\0"[..],
            b"\x21\xFF\x0BNETSCAPE2.0\x02\x01\x02\x03\x01\0\0\0",
            b"\x21\xFF\x0BNETSCAPE2.0\x04\x01\x05\0\0\0",
            b"\x21\xFF\x0BNETSCAPE2.0\x03\x02\x05\0\0",
            b"\x21\xFF\x0BNETSCAPE2.0\0",
        ] {
            let input = gif_with(&[no_loop], b"");
            let output = stripped(&input).unwrap();
            assert_eq!(output, gif_with(&[], b""), "{no_loop:x?}");
            assert_eq!(gif_repeat(&input), Some(::gif::Repeat::Finite(0)));
            assert_eq!(gif_repeat(&output), gif_repeat(&input), "{no_loop:x?}");
        }
    }

    #[test]
    fn a_gif_without_an_image_is_refused_whatever_comes_before_its_trailer() {
        // The gif crate accepts some of these and refuses others, by a look
        // one byte past the logical screen; no viewer shows an image of any.
        for (blocks, gif_crate_accepts) in [
            (&[][..], false),
            (CONTROL, true),
            (COMMENT, true),
            (LOOP, true),
        ] {
            let input = [SCREEN, blocks, &[TRAILER]].concat();
            assert_eq!(gif_crate(&input).0, Vec::<u16>::new());
            assert_eq!(
                gif_crate(&input).1.is_none(),
                gif_crate_accepts,
                "{blocks:x?}"
            );
            assert_eq!(
                optimise(&input),
                corrupt(DecodeDefect::GifNoImage),
                "{blocks:x?}"
            );
            // Without the trailer, too, which the gif crate refuses in
            // every case (ADR 0033, Context, D104).
            let open = &input[..input.len() - 1];
            assert!(gif_crate(open).1.is_some(), "{blocks:x?}");
            assert_eq!(
                optimise(open),
                corrupt(DecodeDefect::GifNoImage),
                "{blocks:x?}"
            );
        }
    }

    #[test]
    fn an_animation_is_refused_at_the_descriptor_that_passes_a_limit() {
        // 100,000 frames, then a block cut short: the walk stops at frame
        // 4097 and never reaches the end, so the stripped copy holds no more
        // than the frames within the limits.
        let input = [SCREEN, &pixel_frame().repeat(100_000), &[EXTENSION]].concat();
        let too_large = ImageError::AnimationTooLarge {
            format: ImageFormat::Gif,
            frames: MAX_ANIMATION_FRAMES + 1,
            pixels: MAX_ANIMATION_FRAMES + 1,
        };
        assert_eq!(strip(&input), Err(too_large));
        assert_eq!(optimise(&input), Err(too_large));
        // The log names where the walk stopped, not the declared total.
        assert_eq!(
            too_large.to_string(),
            "the GIF animation reaches 4097 frames and 4097 pixels; the optimiser takes at most \
             4096 frames and 268435456 pixels in all"
        );
    }

    #[test]
    fn keeps_the_icc_profile_in_full_sub_blocks() {
        let profile: Vec<u8> = (0..=255).chain(0..44).collect();
        let small_blocks: Vec<u8> = profile
            .chunks(100)
            .flat_map(|piece| [&[u8::try_from(piece.len()).unwrap()][..], piece].concat())
            .collect();
        let icc = [b"\x21\xFF\x0BICCRGBG1012".as_slice(), &small_blocks, &[0]].concat();
        let output = stripped(&gif_with(&[&icc], b"")).unwrap();
        let expected = [
            b"\x21\xFF\x0BICCRGBG1012\xFF".as_slice(),
            &profile[..255],
            &[45],
            &profile[255..],
            &[0],
        ]
        .concat();
        assert_eq!(output, gif_with(&[&expected], b""));
        let mut decoder = ::gif::DecodeOptions::new().read_info(&output[..]).unwrap();
        while decoder.read_next_frame().unwrap().is_some() {}
        assert_eq!(decoder.icc_profile(), Some(&profile[..]));
        // An empty profile is dropped.
        let empty = b"\x21\xFF\x0BICCRGBG1012\0";
        assert_eq!(
            stripped(&gif_with(&[empty], b"")).unwrap(),
            gif_with(&[], b"")
        );
    }

    #[test]
    fn a_gif_without_its_trailer_is_kept_as_the_gif_crate_reads_it() {
        let whole = gif_with(&[CONTROL], b"");
        let open = &whole[..whole.len() - 1];
        // The gif crate decodes the frame, then reports the missing end.
        assert_eq!(
            gif_crate(open),
            (vec![10], Some("Unexpected End of File".into()))
        );
        let optimised = optimise(open).unwrap();
        assert_eq!(optimised.bytes, open);
        assert_eq!(optimised.treatment, Treatment::Stripped);
        // Two frames, the metadata dropped, still no trailer added.
        let two = [SCREEN, COMMENT, &pixel_frame(), CONTROL, &pixel_frame()].concat();
        let optimised = optimise(&two).unwrap();
        assert_eq!(
            optimised.bytes,
            [SCREEN, &pixel_frame(), CONTROL, &pixel_frame()].concat()
        );
        assert_eq!(gif_crate(&two).0, gif_crate(&optimised.bytes).0);
    }

    #[test]
    fn a_gif_that_ends_inside_a_block_or_before_its_image_is_refused() {
        let whole = gif_with(&[CONTROL], b"");
        // Inside the frame's data, the colour table and the screen.
        for cut in [whole.len() - 3, 14, 13] {
            assert_eq!(
                optimise(&whole[..cut]),
                corrupt(DecodeDefect::GifCutShort),
                "{cut}"
            );
            assert!(gif_crate(&whole[..cut]).1.is_some(), "{cut}");
        }
        // After the screen and after the control extension: no image yet,
        // and the gif crate never reaches the end of its header.
        for cut in [SCREEN.len(), SCREEN.len() + CONTROL.len()] {
            assert_eq!(
                optimise(&whole[..cut]),
                corrupt(DecodeDefect::GifNoImage),
                "{cut}"
            );
            assert_eq!(
                gif_crate(&whole[..cut]),
                (vec![], Some("Unexpected End of File".into()))
            );
        }
        // A byte that starts no block where one is due is refused, by the
        // gif crate too.
        let unknown_block = [
            &gif_with(&[], b"")[..19],
            &[0x00],
            &pixel_frame(),
            &[TRAILER],
        ]
        .concat();
        assert_eq!(
            stripped(&unknown_block),
            Err(undecodable(DecodeDefect::GifBlock))
        );
        assert!(gif_crate(&unknown_block).1.is_some());
    }

    #[test]
    fn a_graphic_control_extension_counts_with_its_last_sub_block() {
        // What the gif crate reads: earlier sub-blocks do not count.
        for control in [
            &b"\x21\xF9\x03abc\x04\x04\x0A\0\0\0"[..],
            b"\x21\xF9\x04\x09\x09\x09\x09\x04\x04\x0A\0\0\0",
        ] {
            let input = gif_with(&[control], b"");
            assert_eq!(gif_crate(&input), (vec![10], None));
            let optimised = optimise(&input).unwrap();
            assert_eq!(optimised.bytes, gif_with(&[CONTROL], b""));
            assert_eq!(gif_crate(&optimised.bytes), gif_crate(&input));
        }
        // A last sub-block of another length, and none at all, are refused,
        // by the gif crate too.
        for control in [
            &b"\x21\xF9\x05\x04\x0A\0\0\0\0"[..],
            b"\x21\xF9\x04\x04\x0A\0\0\x02ab\0",
            b"\x21\xF9\0",
        ] {
            let input = gif_with(&[control], b"");
            assert_eq!(optimise(&input), corrupt(DecodeDefect::GifGraphicControl));
            assert_eq!(
                gif_crate(&input),
                (vec![], Some("control extension has wrong length".into()))
            );
        }
    }

    #[test]
    fn animations_past_the_limits_are_refused_before_any_decode() {
        let frames = |count: u64, frame: &[u8]| {
            let count = usize::try_from(count).unwrap();
            [SCREEN, &frame.repeat(count), &[TRAILER]].concat()
        };
        // The most frames decode; one more is refused.
        let at = frames(MAX_ANIMATION_FRAMES, &pixel_frame());
        assert!(optimise(&at).is_ok());
        let over = frames(MAX_ANIMATION_FRAMES + 1, &pixel_frame());
        assert_eq!(
            optimise(&over),
            Err(ImageError::AnimationTooLarge {
                format: ImageFormat::Gif,
                frames: MAX_ANIMATION_FRAMES + 1,
                pixels: MAX_ANIMATION_FRAMES + 1,
            })
        );
        // 16 frames of 4096×4096 declare exactly the most pixels, and reach
        // the decoder; a 1×1 frame more is refused before it.
        let big = frame_of(4096, 4096);
        assert_eq!(16 * 4096 * 4096, MAX_ANIMATION_PIXELS);
        let at = frames(16, &big);
        assert!(!matches!(
            optimise(&at),
            Err(ImageError::AnimationTooLarge { .. })
        ));
        let over = [SCREEN, &big.repeat(16), &pixel_frame(), &[TRAILER]].concat();
        assert_eq!(
            optimise(&over),
            Err(ImageError::AnimationTooLarge {
                format: ImageFormat::Gif,
                frames: 17,
                pixels: MAX_ANIMATION_PIXELS + 1,
            })
        );
    }

    #[test]
    fn the_budget_replaces_the_gif_crates_own_memory_limit() {
        // 7072×7072 is 50,013,184 pixels, just above the gif crate's own
        // default limit of 50,000,000 bytes (`gif@0.14.2:src/reader/mod.rs`).
        // The frame is LZW-coded in full by the gif crate's encoder, as the
        // decode refuses image data that ends before the last pixel.
        let (side, pixels) = (7072_u16, 7072 * 7072);
        assert!(pixels > 50_000_000);
        let mut encoder = ::gif::Encoder::new(Vec::new(), 1, 1, &[0, 0, 0, 255, 255, 255]).unwrap();
        let frame = ::gif::Frame::from_indexed_pixels(side, side, vec![0; 7072 * 7072], None);
        encoder.write_frame(&frame).unwrap();
        let input = encoder.into_inner().unwrap();
        // The gif crate's default limit refuses the frame at one byte per
        // pixel, as the check decodes it.
        let mut options = ::gif::DecodeOptions::new();
        options.set_color_output(::gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(input.as_slice()).unwrap();
        assert!(matches!(
            decoder.read_next_frame(),
            Err(::gif::DecodingError::OutOfMemory)
        ));
        // A budget that admits the frame lets it decode: the limit is the
        // budget, not that default.
        let fits = PixelBudget::new(pixels).unwrap();
        let optimised = optimise_gif(&input, fits).unwrap();
        assert_eq!(
            (optimised.bytes, optimised.treatment),
            (input.clone(), Treatment::Stripped)
        );
        // One pixel less, and the header check refuses it before any decode:
        // its canvas holds every frame, so the limit never refuses a frame
        // within the budget.
        let below = PixelBudget::new(pixels - 1).unwrap();
        assert_eq!(
            optimise_gif(&input, below),
            Err(ImageError::OverPixelBudget {
                format: ImageFormat::Gif,
                dimensions: Dimensions::new(7072, 7072).unwrap(),
                budget: below,
            })
        );
    }

    #[test]
    fn refuses_image_data_that_does_not_decode() {
        // A minimum code size of 12 is above the GIF maximum of 11. It sits
        // before the data sub-block (3 bytes), its terminator and the trailer.
        let mut bytes = gif_with(&[], b"");
        let lzw = bytes.len() - 6;
        bytes[lzw] = 12;
        assert_eq!(optimise(&bytes), corrupt(DecodeDefect::GifData));
        // Without the trailer too: the appended one changes nothing else.
        assert_eq!(
            optimise(&bytes[..bytes.len() - 1]),
            corrupt(DecodeDefect::GifData)
        );
    }

    #[test]
    fn hands_header_refusals_and_other_formats_back() {
        assert_eq!(
            optimise(&samples::gif(10_000, 5_001)),
            Err(ImageError::OverPixelBudget {
                format: ImageFormat::Gif,
                dimensions: Dimensions::new(10_000, 5_001).unwrap(),
                budget: PixelBudget::DEFAULT,
            })
        );
        assert_eq!(
            optimise(&samples::png(1, 1)),
            Err(ImageError::WrongFormat {
                expected: ImageFormat::Gif,
                found: ImageFormat::Png,
            })
        );
    }

    #[test]
    fn sub_blocks_stop_at_the_terminator() {
        let blocks: Vec<&[u8]> = sub_blocks(b"\x02ab\x01c\0\x01d").collect();
        assert_eq!(blocks, [&b"ab"[..], b"c"]);
        assert_eq!(chain_end(b"\x02ab\x01c\0\x01d", 0), Ok(6));
        assert_eq!(chain_end(b"\x02ab\x05c", 0), Err(DecodeDefect::GifCutShort));
    }

    /// An extension block of `label` whose sub-blocks are `blocks`.
    fn extension(label: u8, blocks: Vec<Vec<u8>>) -> Vec<u8> {
        let mut bytes = vec![EXTENSION, label];
        for block in blocks {
            bytes.push(u8::try_from(block.len()).unwrap());
            bytes.extend(block);
        }
        bytes.push(0);
        bytes
    }

    /// Any extension block: a label and up to three sub-blocks, or a loop
    /// extension whose sub-blocks are loop, buffering or other ones of
    /// every length.
    fn any_extension() -> impl Strategy<Value = Vec<u8>> {
        let sub_block = prop_oneof![
            (any::<u16>()).prop_map(|count| [&[1][..], &count.to_le_bytes()].concat()),
            vec(any::<u8>(), 1..6).prop_map(|rest| [&[1][..], &rest].concat()),
            vec(any::<u8>(), 1..6).prop_map(|rest| [&[2][..], &rest].concat()),
            vec(any::<u8>(), 1..20),
        ];
        prop_oneof![
            (any::<u8>(), vec(vec(any::<u8>(), 1..20), 0..3))
                .prop_map(|(label, blocks)| extension(label, blocks)),
            vec(sub_block, 0..3).prop_map(|blocks| {
                let mut all = vec![b"NETSCAPE2.0".to_vec()];
                all.extend(blocks);
                extension(APPLICATION, all)
            }),
        ]
    }

    proptest! {
        #[test]
        fn the_frames_survive_any_extensions_around_them(
            before in vec(any_extension(), 0..4),
            after in vec(any::<u8>(), 0..16),
        ) {
            let before: Vec<&[u8]> = before.iter().map(Vec::as_slice).collect();
            let input = gif_with(&before, &after);
            if let Ok(output) = stripped(&input) {
                prop_assert!(output.len() <= input.len());
                prop_assert!(output.ends_with(&[&pixel_frame()[..], &[TRAILER]].concat()));
                // Stripping is idempotent: a second pass changes nothing.
                prop_assert_eq!(stripped(&output), Ok(output.clone()));
                // The gif crate reads the same frames, delays and loop
                // count from both, when it reads the upload at all: unlike
                // browsers, it refuses an unknown extension label, which the
                // strip drops.
                let original = gif_crate(&input);
                if original.1.is_none() {
                    prop_assert_eq!(gif_crate(&output), original);
                    prop_assert_eq!(gif_repeat(&output), gif_repeat(&input));
                }
            }
        }

        #[test]
        fn the_totals_are_those_the_descriptors_declare(
            sizes in vec((0..300_u16, 0..300_u16), 1..12),
            trailer in any::<bool>(),
        ) {
            let frames: Vec<u8> = sizes.iter().flat_map(|&(w, h)| frame_of(w, h)).collect();
            let end: &[u8] = if trailer { &[TRAILER] } else { &[] };
            let input = [SCREEN, &frames, end].concat();
            let stripped = strip(&input).unwrap();
            prop_assert_eq!(stripped.frames, u64::try_from(sizes.len()).unwrap());
            let pixels: u64 = sizes.iter().map(|&(w, h)| u64::from(w) * u64::from(h)).sum();
            prop_assert_eq!(stripped.pixels, pixels);
            prop_assert_eq!(stripped.trailer, trailer);
            prop_assert_eq!(stripped.bytes, input);
        }

        #[test]
        fn arbitrary_blocks_never_panic(tail in vec(any::<u8>(), 0..120)) {
            let input = [&samples::gif(67, 41)[..], &tail].concat();
            if let Ok(optimised) = optimise(&input) {
                prop_assert!(optimised.bytes.len() <= input.len());
                prop_assert_eq!(optimise(&optimised.bytes).map(|o| o.bytes), Ok(optimised.bytes));
            }
        }
    }
}
