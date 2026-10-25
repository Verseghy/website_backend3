//! The PNG files of the image corpus through
//! `domain::image_optimise::optimise_png` (ADR 0015 §7, "Image tests", and
//! the bullets ADR 0033 adds to them).
//!
//! The pixels are compared through the `png` crate, a decoder independent of
//! oxipng and libdeflate, and the decoder of Chromium: every image of both
//! files, the default image and each animation frame with its frame
//! control, is decoded to 16-bit RGBA, so a change of colour type, bit
//! depth, palette or interlacing that oxipng makes counts as no change, and
//! any changed pixel or frame fails.
//!
//! The corpus files come from real encoders but cover few shapes, so a
//! property test also builds valid PNGs of every colour type, bit depth and
//! interlace method, with palettes, transparency, colour and HDR chunks and
//! an animation frame, and compares them the same way. It found that oxipng
//! 10.2.1 turns opaque pixels transparent when it reduces a 16-bit image
//! whose transparent colour matches no pixel, which the optimiser now
//! prevents. Where the `png` crate and libpng read a file differently (a
//! `tRNS` longer than the palette's indexable part but not than the
//! palette, a `tRNS` sample with bits above a low bit depth), the generator
//! leaves that file out, and the chunk walk's unit tests pin libpng's
//! reading.

use std::io::Cursor;

use backend3::domain::image_optimise::{
    DecodeDefect, Dimensions, ImageError, ImageFormat, ImageHeader, PixelBudget, PngEffort, inspect,
};
use png::{BitDepth, ColorType, Decoder, Transformations};
use proptest::{
    collection::vec,
    prelude::*,
    sample::{Index, select},
};

use super::image_optimise::{CORPUS, corpus_file, is_over_budget};

/// The chunks of a PNG, as type and data.
type Chunks = Vec<([u8; 4], Vec<u8>)>;

/// One image of a PNG as the `png` crate decodes it: its frame control
/// without the sequence number (`None` for a default image outside the
/// animation or a still image), its width and height, and its 16-bit RGBA
/// samples.
type Image = (Option<[u32; 8]>, u32, u32, Vec<u16>);

/// The chunk types an optimised PNG may hold: the image itself and the
/// ancillary chunks that ADR 0015 §7 and ADR 0033 §6 keep.
const ALLOWED: [&[u8; 4]; 16] = [
    b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND", b"iCCP", b"sRGB", b"gAMA", b"cHRM", b"eXIf",
    b"cICP", b"mDCV", b"cLLI", b"acTL", b"fcTL", b"fdAT",
];

/// Optimises `input` with the default budget and effort.
fn optimise(input: &[u8]) -> Result<Vec<u8>, ImageError> {
    backend3::domain::image_optimise::optimise_png(input, PixelBudget::DEFAULT, PngEffort::DEFAULT)
}

/// The PNG files of the corpus that hold a whole image.
fn png_paths() -> Vec<&'static str> {
    CORPUS
        .iter()
        .filter(|&&(path, format, ..)| format == ImageFormat::Png && !is_over_budget(path))
        .map(|&(path, ..)| path)
        .collect()
}

/// The chunks of a PNG up to `IEND`, as type and data, and the bytes after
/// `IEND`. The input must have whole chunks, as an optimised file has.
fn chunks(png: &[u8]) -> (Chunks, Vec<u8>) {
    let mut at = 8;
    let mut found = Vec::new();
    loop {
        let len = usize::try_from(u32::from_be_bytes(png[at..at + 4].try_into().unwrap())).unwrap();
        let kind: [u8; 4] = png[at + 4..at + 8].try_into().unwrap();
        found.push((kind, png[at + 8..at + 8 + len].to_vec()));
        at += 12 + len;
        if kind == *b"IEND" {
            return (found, png[at..].to_vec());
        }
    }
}

/// The data of the first chunk of `kind`, if any.
fn chunk_data(png: &[u8], kind: &[u8]) -> Option<Vec<u8>> {
    chunks(png)
        .0
        .into_iter()
        .find(|(k, _)| k[..] == *kind)
        .map(|(_, data)| data)
}

/// How many chunks of `kind` a PNG holds.
fn count(png: &[u8], kind: [u8; 4]) -> usize {
    chunks(png).0.iter().filter(|(k, _)| *k == kind).count()
}

/// Every image of a PNG, in order: the default image, then each animation
/// frame, as [`Image`].
fn images(png: &[u8]) -> Vec<Image> {
    let mut decoder = Decoder::new(Cursor::new(png));
    decoder.set_transformations(Transformations::EXPAND | Transformations::ALPHA);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let mut found = Vec::new();
    // The default image always decodes; a frame after it only when the file
    // is animated, and the decoder then reports an error after the last.
    while let Ok(frame) = reader.next_frame(&mut buf) {
        let bytes = &buf[..frame.buffer_size()];
        let samples: Vec<u16> = match reader.output_color_type().1 {
            BitDepth::Sixteen => bytes
                .chunks(2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .collect(),
            BitDepth::Eight => bytes.iter().map(|&b| u16::from(b) * 257).collect(),
            other => panic!("EXPAND leaves no {other:?} samples"),
        };
        let rgba = match reader.output_color_type().0 {
            ColorType::Rgba => samples,
            ColorType::GrayscaleAlpha => samples
                .chunks(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            other => panic!("EXPAND and ALPHA leave no {other:?}"),
        };
        let info = reader.info();
        let control = info
            .frame_control()
            .filter(|_| info.is_animated())
            .map(|c| {
                [
                    c.width,
                    c.height,
                    c.x_offset,
                    c.y_offset,
                    u32::from(c.delay_num),
                    u32::from(c.delay_den),
                    c.dispose_op as u32,
                    c.blend_op as u32,
                ]
            });
        found.push((control, frame.width, frame.height, rgba));
        if !info.is_animated() {
            break;
        }
    }
    found
}

/// The first image of a PNG as width, height and 16-bit RGBA samples.
fn rgba16(png: &[u8]) -> (u32, u32, Vec<u16>) {
    let (_, width, height, rgba) = images(png).swap_remove(0);
    (width, height, rgba)
}

/// The ICC profile of a PNG, decompressed by the `png` crate.
fn icc_profile(png: &[u8]) -> Option<Vec<u8>> {
    let reader = Decoder::new(Cursor::new(png)).read_info().unwrap();
    reader.info().icc_profile.as_ref().map(|p| p.to_vec())
}

fn optimised(path: &str) -> (Vec<u8>, Vec<u8>) {
    let input = corpus_file(path);
    let out = optimise(&input).unwrap_or_else(|e| panic!("{path}: {e}"));
    (input, out)
}

#[test]
fn every_png_keeps_its_pixels_frames_and_size_and_never_grows() {
    for path in png_paths() {
        let (input, out) = optimised(path);
        assert!(
            out.len() <= input.len(),
            "{path}: {} > {}",
            out.len(),
            input.len()
        );
        assert_eq!(
            inspect(&out, PixelBudget::DEFAULT),
            Ok(ImageHeader {
                format: ImageFormat::Png,
                dimensions: Dimensions::new(67, 41).unwrap(),
            }),
            "{path}"
        );
        assert_eq!(images(&out), images(&input), "{path}");
    }
}

#[test]
fn every_png_keeps_only_the_allowed_chunks_and_nothing_after_iend() {
    for path in png_paths() {
        let (_, out) = optimised(path);
        let (found, after) = chunks(&out);
        for (kind, _) in &found {
            assert!(
                ALLOWED.contains(&kind),
                "{path}: {}",
                String::from_utf8_lossy(kind)
            );
        }
        assert_eq!(after, b"", "{path}");
    }
}

#[test]
fn camera_metadata_is_reduced_to_the_orientation() {
    let (input, out) = optimised("png/metadata.png");
    // exiv2 wrote a little-endian structure; only Orientation 6 is left.
    let orientation = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    assert_eq!(chunk_data(&out, b"eXIf").as_deref(), Some(&orientation[..]));
    // The colour chunk stays as it was; the rest is gone, the GPS position,
    // the camera make and serial number with it.
    assert_eq!(chunk_data(&out, b"cHRM"), chunk_data(&input, b"cHRM"));
    assert!(chunk_data(&out, b"cHRM").is_some());
    for kind in [b"bKGD", b"pHYs", b"tEXt"] {
        assert_eq!(
            chunk_data(&out, kind),
            None,
            "{}",
            String::from_utf8_lossy(kind)
        );
    }
    for gone in [&b"backend3"[..], b"0000000000"] {
        assert!(!out.windows(gone.len()).any(|w| w == gone));
    }
}

#[test]
fn the_icc_profile_is_kept_and_its_text_copy_dropped() {
    let (input, out) = optimised("png/icc-profile.png");
    let profile = icc_profile(&input).unwrap();
    assert_eq!(profile.len(), 464);
    assert_eq!(icc_profile(&out), Some(profile));
    assert!(chunk_data(&out, b"cHRM").is_some());
    assert_eq!(chunk_data(&out, b"zTXt"), None);
}

#[test]
fn an_appended_payload_and_a_private_chunk_are_stripped() {
    let (input, out) = optimised("png/payload.png");
    assert!(input.ends_with(b"</html>\n"));
    let (found, after) = chunks(&out);
    assert_eq!(after, b"");
    assert!(found.iter().all(|(kind, _)| kind != b"prVt"));
    for gone in [&b"payload"[..], b"backend3 test data"] {
        assert!(!out.windows(gone.len()).any(|w| w == gone));
    }
    assert_eq!(rgba16(&out), rgba16(&corpus_file("png/rgb.png")));
}

#[test]
fn an_animated_png_keeps_every_frame_and_loses_its_metadata() {
    for path in ["png/animated.png", "png/animated-metadata.png"] {
        let (input, out) = optimised(path);
        // Three frames, the first of them the default image; the frame
        // controls and pixels are compared by the first test.
        let frames = images(&input);
        assert_eq!(frames.len(), 3, "{path}");
        assert!(
            frames.iter().all(|(control, ..)| control.is_some()),
            "{path}"
        );
        assert_eq!(images(&out), frames, "{path}");
        assert_eq!(
            chunk_data(&out, b"acTL"),
            chunk_data(&input, b"acTL"),
            "{path}"
        );
        assert_eq!(count(&out, *b"fcTL"), 3, "{path}");
        // ffmpeg's density chunk is dropped as in a still image.
        assert_eq!(chunk_data(&out, b"pHYs"), None, "{path}");
    }
    let (_, out) = optimised("png/animated-metadata.png");
    assert_eq!(chunk_data(&out, b"tEXt"), None);
    assert!(!out.windows(8).any(|w| w == b"backend3"));
    let orientation = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    assert_eq!(chunk_data(&out, b"eXIf").as_deref(), Some(&orientation[..]));
}

/// The chunks of animated.png, in order: `IHDR`, `pHYs`, `acTL`, frame 0's
/// `fcTL` (before the data), `IDAT`, then frame 1's `fcTL` and `fdAT`, frame
/// 2's `fcTL` and `fdAT`, and `IEND`. The indices the defects below use.
const ACTL: usize = 2;
const FIRST_FCTL: usize = 3;
const IDAT: usize = 4;
const SECOND_FCTL: usize = 5;
const SECOND_FDAT: usize = 6;
const THIRD_FCTL: usize = 7;
const THIRD_FDAT: usize = 8;

/// A PNG of `chunks`, each with its CRC, except those whose index is in
/// `lost`, whose CRC is broken.
fn assemble(chunks: &[([u8; 4], Vec<u8>)], lost: &[usize]) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1A\n".to_vec();
    for (index, (kind, data)) in chunks.iter().enumerate() {
        let mut written = chunk(*kind, data);
        if lost.contains(&index) {
            *written.last_mut().unwrap() ^= 1;
        }
        png.extend(written);
    }
    png
}

/// The sequence number `sequence` written over the first 4 bytes of an
/// `fcTL` or `fdAT`'s `data`.
fn renumber(data: &mut [u8], sequence: u32) {
    data[0..4].copy_from_slice(&sequence.to_be_bytes());
}

/// An `fdAT`'s `data` whose frame rows were changed by `change`: inflated,
/// changed and compressed again, with the sequence number kept.
fn change_rows(data: &mut Vec<u8>, change: impl Fn(&mut Vec<u8>)) {
    let mut rows = miniz_oxide::inflate::decompress_to_vec_zlib(&data[4..]).unwrap();
    change(&mut rows);
    let stream = miniz_oxide::deflate::compress_to_vec_zlib(&rows, 6);
    data.truncate(4);
    data.extend_from_slice(&stream);
}

/// The frame count and play count of a PNG's `acTL` as the `png` crate
/// reads them, after decoding every frame the animation announces: the test
/// fails if any of them does not decode, or if one more follows.
fn every_frame_decodes(png: &[u8]) -> (u32, u32) {
    let mut decoder = Decoder::new(Cursor::new(png));
    decoder.set_transformations(Transformations::EXPAND | Transformations::ALPHA);
    let mut reader = decoder.read_info().unwrap();
    let info = reader.info();
    let control = info.animation_control.expect("an animated PNG");
    // The default image is decoded first; it is a frame of the animation
    // only when an fcTL comes before the data.
    let images = control.num_frames + u32::from(info.frame_control.is_none());
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    for index in 0..images {
        reader
            .next_frame(&mut buf)
            .unwrap_or_else(|e| panic!("image {index} of {images}: {e}"));
    }
    assert!(
        reader.next_frame(&mut buf).is_err(),
        "an image past acTL's count"
    );
    (control.num_frames, control.num_plays)
}

/// A defect planted in animated.png: what it is, the change of the chunks
/// that makes it, the chunks whose CRC is broken afterwards, and the frames
/// kept.
type Defect = (&'static str, fn(&mut Chunks), &'static [usize], usize);

/// One defect of each kind that ADR 0033 §6 lists, made in animated.png
/// (three frames, the first the default image, played forever), with the
/// frames kept: those before the frame the defect lies in, or, for a defect
/// in no frame, those that ended before it. These are the sequence numbers,
/// the frame controls and misplaced frame chunks; [`frame_defects`] has the
/// rest.
fn control_defects() -> [Defect; 12] {
    [
        (
            "a sequence gap",
            |c| renumber(&mut c[THIRD_FDAT].1, 5),
            &[],
            2,
        ),
        (
            "a repeated sequence number",
            |c| renumber(&mut c[SECOND_FDAT].1, 1),
            &[],
            1,
        ),
        (
            "a first sequence number of 1",
            |c| renumber(&mut c[FIRST_FCTL].1, 1),
            &[],
            0,
        ),
        (
            "a frame outside the canvas",
            |c| c[THIRD_FCTL].1[15] = 1,
            &[],
            2,
        ),
        (
            "a frame of zero width",
            |c| c[THIRD_FCTL].1[4..8].fill(0),
            &[],
            2,
        ),
        (
            "an fcTL of 25 bytes",
            |c| c[THIRD_FCTL].1.truncate(25),
            &[],
            2,
        ),
        (
            "an unknown dispose operation",
            |c| c[SECOND_FCTL].1[24] = 3,
            &[],
            1,
        ),
        (
            "an unknown blend operation",
            |c| c[THIRD_FCTL].1[25] = 2,
            &[],
            2,
        ),
        (
            "an fdAT shorter than its number",
            |c| c[THIRD_FDAT].1 = vec![0, 0, 0],
            &[],
            2,
        ),
        (
            "a second fcTL before the data",
            |c| {
                let mut second = c[FIRST_FCTL].clone();
                renumber(&mut second.1, 1);
                c.insert(IDAT, second);
            },
            &[],
            0,
        ),
        (
            "a first fcTL short of the canvas",
            |c| c[FIRST_FCTL].1[7] = 66,
            &[],
            0,
        ),
        (
            "an fdAT before the data",
            |c| {
                let mut early = c[SECOND_FDAT].clone();
                renumber(&mut early.1, 1);
                c.insert(IDAT, early);
            },
            &[],
            0,
        ),
    ]
}

/// The defects of where the frames end and of their data, and the frames
/// that the count of acTL drops: with [`lost_chunk_defects`], the rest of
/// [`control_defects`].
fn frame_defects() -> [Defect; 9] {
    [
        (
            "an fdAT right after the data, in no frame",
            |c| {
                c.insert(
                    SECOND_FCTL,
                    (*b"fdAT", [&1_u32.to_be_bytes()[..], b"junk"].concat()),
                );
            },
            &[],
            1,
        ),
        (
            "an fdAT parted from the one before",
            |c| {
                let data = c[SECOND_FDAT].1.clone();
                let tail = [&3_u32.to_be_bytes()[..], &data[40..]].concat();
                c[SECOND_FDAT].1.truncate(40);
                c.insert(SECOND_FDAT + 1, (*b"tEXt", b"a\0b".to_vec()));
                c.insert(SECOND_FDAT + 2, (*b"fdAT", tail));
            },
            &[],
            1,
        ),
        (
            "a last frame without an fdAT",
            |c| {
                c.remove(THIRD_FDAT);
            },
            &[],
            2,
        ),
        (
            "a middle frame without an fdAT",
            |c| {
                c.remove(SECOND_FDAT);
            },
            &[],
            1,
        ),
        (
            "frame data one row short",
            |c| change_rows(&mut c[THIRD_FDAT].1, |rows| rows.truncate(rows.len() - 269)),
            &[],
            2,
        ),
        (
            "frame data with an unknown filter type",
            |c| change_rows(&mut c[SECOND_FDAT].1, |rows| rows[269] = 5),
            &[],
            1,
        ),
        (
            "frame data with a wrong Adler-32",
            |c| *c[THIRD_FDAT].1.last_mut().unwrap() ^= 1,
            &[],
            2,
        ),
        // No defect: frames past the count acTL announces are dropped
        // whatever they hold, and fewer frames are announced as they are.
        ("more frames than announced", |c| c[ACTL].1[3] = 2, &[], 2),
        (
            "a broken frame past the count",
            |c| {
                c[ACTL].1[3] = 2;
                c[THIRD_FCTL].1[15] = 1;
            },
            &[],
            2,
        ),
    ]
}

/// The defects of a chunk lost to its CRC: after the data, in a frame or
/// between two `fdAT` chunks it parts, and before the data, before frame 0's
/// `fcTL` or inside frame 0, where no frame is left. With
/// [`frame_defects`], the rest of [`control_defects`].
fn lost_chunk_defects() -> [Defect; 7] {
    [
        (
            "an fdAT parted by a chunk lost to its CRC",
            |c| {
                let data = c[SECOND_FDAT].1.clone();
                let tail = [&3_u32.to_be_bytes()[..], &data[40..]].concat();
                c[SECOND_FDAT].1.truncate(40);
                c.insert(SECOND_FDAT + 1, (*b"tEXt", b"a\0b".to_vec()));
                c.insert(SECOND_FDAT + 2, (*b"fdAT", tail));
            },
            &[SECOND_FDAT + 1],
            1,
        ),
        ("an fcTL lost to its CRC", |_| {}, &[THIRD_FCTL], 2),
        ("an fdAT lost to its CRC", |_| {}, &[SECOND_FDAT], 1),
        (
            "an acTL before the data lost to its CRC",
            |c| c.insert(FIRST_FCTL, c[ACTL].clone()),
            &[FIRST_FCTL],
            0,
        ),
        // Inside frame 0, between its fcTL and the data: frame 0 has not
        // ended, so a lost chunk there lies in it and no frame is left.
        (
            "an acTL lost inside frame 0",
            |c| c.insert(IDAT, c[ACTL].clone()),
            &[IDAT],
            0,
        ),
        (
            "an fcTL lost inside frame 0",
            |c| c.insert(IDAT, c[SECOND_FCTL].clone()),
            &[IDAT],
            0,
        ),
        (
            "an fdAT lost inside frame 0",
            |c| {
                let mut early = c[SECOND_FDAT].clone();
                renumber(&mut early.1, 1);
                c.insert(IDAT, early);
            },
            &[IDAT],
            0,
        ),
    ]
}

#[test]
fn a_broken_animation_keeps_the_frames_before_its_first_defect() {
    // The stored file announces the frames kept, plays as the upload did,
    // decodes every frame in the png crate, and holds exactly the upload's
    // first frames, pixels, controls and all.
    let original = corpus_file("png/animated.png");
    let (parts, _) = chunks(&original);
    assert_eq!(parts.len(), 10);
    assert_eq!(parts[THIRD_FDAT].0, *b"fdAT");
    let frames = images(&original);
    for (what, change, lost, kept) in control_defects()
        .into_iter()
        .chain(frame_defects())
        .chain(lost_chunk_defects())
    {
        let mut parts = parts.clone();
        change(&mut parts);
        let input = assemble(&parts, lost);
        let out = optimise(&input).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(out.len() <= input.len(), "{what}");
        let stored = images(&out);
        if kept == 0 {
            // No frame is left: the still default image, with no animation
            // chunk.
            for kind in [b"acTL", b"fcTL", b"fdAT"] {
                assert_eq!(count(&out, *kind), 0, "{what}");
            }
            assert_eq!(stored.len(), 1, "{what}");
            assert_eq!((stored[0].0, &stored[0].3), (None, &frames[0].3), "{what}");
        } else {
            let announced = u32::try_from(kept).unwrap();
            assert_eq!(every_frame_decodes(&out), (announced, 0), "{what}");
            assert_eq!(count(&out, *b"fcTL"), kept, "{what}");
            assert_eq!(stored, frames[..kept], "{what}");
        }
    }
}

#[test]
fn fewer_frames_than_announced_are_kept_and_announced_as_they_are() {
    // acTL announces five; the three frames are kept, and acTL says three.
    let (mut parts, _) = chunks(&corpus_file("png/animated.png"));
    parts[ACTL].1[3] = 5;
    let out = optimise(&assemble(&parts, &[])).unwrap();
    assert_eq!(every_frame_decodes(&out), (3, 0));
    assert_eq!(images(&out), images(&corpus_file("png/animated.png")));
    // An acTL after the data is ignored, intact or lost to its CRC.
    let (mut parts, _) = chunks(&corpus_file("png/animated.png"));
    let late = parts[ACTL].clone();
    parts.insert(SECOND_FCTL, late);
    for lost in [&[][..], &[SECOND_FCTL]] {
        let out = optimise(&assemble(&parts, lost)).unwrap();
        assert_eq!(every_frame_decodes(&out), (3, 0));
    }
}

#[test]
fn every_known_dispose_and_blend_operation_is_kept() {
    // animated.png uses operation 0 throughout. Dispose 1 and 2 and blend 1,
    // in frame 0 and in the frames after the data, are no defect: every
    // frame is kept with its operations, which `images` compares.
    let original = corpus_file("png/animated.png");
    let (mut parts, _) = chunks(&original);
    for (index, dispose, blend) in [(FIRST_FCTL, 1, 1), (SECOND_FCTL, 2, 1), (THIRD_FCTL, 1, 0)] {
        parts[index].1[24] = dispose;
        parts[index].1[25] = blend;
    }
    let input = assemble(&parts, &[]);
    let out = optimise(&input).unwrap();
    assert_eq!(every_frame_decodes(&out), (3, 0));
    let frames = images(&input);
    assert_eq!(frames.len(), 3);
    assert_eq!(
        frames
            .iter()
            .map(|(control, ..)| control.map(|c| (c[6], c[7])))
            .collect::<Vec<_>>(),
        [Some((1, 1)), Some((2, 1)), Some((1, 0))]
    );
    assert_eq!(images(&out), frames);
}

/// A chunk outside the animation planted between animated.png's second and
/// third frames: what it is, the chunks, and which of them lose their CRC.
type Outside = (&'static str, Chunks, &'static [usize]);

#[test]
fn still_image_chunks_between_the_frames_never_cut_the_animation() {
    // The png crate stops at each of these, so it decodes only the first two
    // images of the upload. They are still-image chunks, which libpng
    // ignores in this RGBA image: the walk drops them, keeps every frame,
    // and the stored file decodes in full (ADR 0033 §6, png/apng.rs).
    let original = corpus_file("png/animated.png");
    let (parts, _) = chunks(&original);
    let frames = images(&original);
    let palette = || (*b"PLTE", vec![0; 3]);
    let cases: [Outside; 5] = [
        (
            "an IDAT after the first run",
            vec![parts[IDAT].clone()],
            &[],
        ),
        ("an empty PLTE", vec![(*b"PLTE", Vec::new())], &[]),
        ("a PLTE of 771 bytes", vec![(*b"PLTE", vec![0; 771])], &[]),
        ("a second PLTE", vec![palette(), palette()], &[]),
        ("a PLTE with a bad CRC", vec![palette()], &[THIRD_FCTL]),
    ];
    for (what, extra, lost) in cases {
        let mut changed = parts.clone();
        changed.splice(THIRD_FCTL..THIRD_FCTL, extra);
        let input = assemble(&changed, lost);
        assert_eq!(images(&input).len(), 2, "{what}: the png crate stops");
        let out = optimise(&input).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(out.len() <= input.len(), "{what}");
        assert_eq!(every_frame_decodes(&out), (3, 0), "{what}");
        assert_eq!(images(&out), frames, "{what}");
    }
}

#[test]
fn an_animation_cut_short_or_with_an_undecodable_default_image_is_refused() {
    // The still-image checks come first (ADR 0033 §6): a file cut inside its
    // animation has no IEND, and a default image that does not decode is
    // corrupt, whatever frames come before or after.
    let original = corpus_file("png/animated.png");
    let cut = original.len() - 12 - 40;
    assert_eq!(
        optimise(&original[..cut]),
        Err(ImageError::Undecodable {
            format: ImageFormat::Png,
            defect: DecodeDefect::PngTruncated,
        })
    );
    let (mut parts, _) = chunks(&original);
    *parts[IDAT].1.last_mut().unwrap() ^= 1;
    assert_eq!(
        optimise(&assemble(&parts, &[])),
        Err(ImageError::Undecodable {
            format: ImageFormat::Png,
            defect: DecodeDefect::PngImageData,
        })
    );
}

#[test]
fn hdr_chunks_are_kept_byte_for_byte_and_the_text_dropped() {
    let (input, out) = optimised("png/hdr.png");
    for kind in [b"cICP", b"mDCV", b"cLLI"] {
        let kept = chunk_data(&out, kind);
        assert!(kept.is_some(), "{}", String::from_utf8_lossy(kind));
        assert_eq!(
            kept,
            chunk_data(&input, kind),
            "{}",
            String::from_utf8_lossy(kind)
        );
    }
    assert_eq!(chunk_data(&out, b"cICP"), Some(vec![9, 16, 0, 1]));
    assert_eq!(chunk_data(&out, b"tEXt"), None);
    assert!(!out.windows(8).any(|w| w == b"backend3"));
    // A 16-bit RGB image whose code points describe RGB samples stays RGB.
    assert_eq!(out[25], 2);
}

#[test]
fn the_over_budget_png_is_refused_before_decoding() {
    assert_eq!(
        optimise(&corpus_file("over-budget/camera-50mp.png")),
        Err(ImageError::OverPixelBudget {
            format: ImageFormat::Png,
            dimensions: Dimensions::new(8_688, 5_792).unwrap(),
            budget: PixelBudget::DEFAULT,
        })
    );
}

#[test]
fn an_animation_beyond_the_frame_total_is_refused_from_its_frame_headers() {
    // A 1x1 animation of 4097 frames: the default image and 4096 more, each
    // frame's data a valid stream. 4096 frames in all are accepted.
    let animation = |frames: u32| {
        let pixel = miniz_oxide::deflate::compress_to_vec_zlib(&[0, 0], 6);
        let mut before = vec![chunk(
            *b"acTL",
            &[&frames.to_be_bytes()[..], &[0; 4]].concat(),
        )];
        let control = |sequence: u32| {
            let fields = [sequence, 1, 1, 0, 0];
            let mut data: Vec<u8> = fields.iter().flat_map(|f| f.to_be_bytes()).collect();
            data.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
            chunk(*b"fcTL", &data)
        };
        before.push(control(0));
        let mut after = Vec::new();
        for index in 1..frames {
            after.push(control(2 * index - 1));
            after.push(chunk(
                *b"fdAT",
                &[&(2 * index).to_be_bytes()[..], &pixel].concat(),
            ));
        }
        let mut png = build((1, 1, 8, 0, 0), &before, &[0, 0], 6, 1);
        let iend = png.len() - 12;
        png.splice(iend..iend, after.concat());
        png
    };
    let at = animation(4096);
    let out = optimise(&at).unwrap();
    assert_eq!(count(&out, *b"fcTL"), 4096);
    assert_eq!(
        optimise(&animation(4097)),
        Err(ImageError::AnimationTooLarge {
            format: ImageFormat::Png,
            frames: 4097,
            pixels: 4097,
        })
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn damaged_pngs_never_panic_and_any_result_is_a_valid_png(
        path in select(png_paths()),
        damage in vec((any::<Index>(), any::<u8>()), 1..8),
        // Cut only now and then: a cut file has no IEND and is refused.
        cut in prop::option::weighted(0.2, any::<Index>()),
    ) {
        let mut bytes = corpus_file(path);
        for (at, byte) in damage {
            let at = at.index(bytes.len());
            bytes[at] = byte;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut.index(bytes.len()));
        }
        if let Ok(out) = optimise(&bytes) {
            prop_assert!(out.len() <= bytes.len());
            let (found, after) = chunks(&out);
            prop_assert!(after.is_empty());
            prop_assert!(found.iter().all(|(kind, _)| ALLOWED.contains(&kind)));
            // An independent decoder reads what was stored, every frame: a
            // damaged animation is cut so that each frame acTL announces
            // decodes.
            let stored = images(&out);
            prop_assert_eq!((stored[0].1, stored[0].2), (67, 41));
            if count(&out, *b"acTL") == 1 {
                let (frames, _) = every_frame_decodes(&out);
                prop_assert_eq!(count(&out, *b"fcTL"), usize::try_from(frames).unwrap());
            }
        }
    }
}

/// A chunk of `kind` with `data`, its length and its CRC.
fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let len = u32::try_from(data.len()).unwrap().to_be_bytes();
    let crc = crc32fast::hash(&[&kind[..], data].concat()).to_be_bytes();
    [&len[..], &kind, data, &crc].concat()
}

/// A PNG of `ihdr` (width, height, bit depth, colour type, interlace), the
/// chunks `before`, the raw image data `raw` compressed at `level` in
/// `pieces` IDAT chunks, and IEND.
fn build(
    ihdr: (u32, u32, u8, u8, u8),
    before: &[Vec<u8>],
    raw: &[u8],
    level: u8,
    pieces: usize,
) -> Vec<u8> {
    let (width, height, depth, colour, interlace) = ihdr;
    let header = [
        &width.to_be_bytes()[..],
        &height.to_be_bytes(),
        &[depth, colour, 0, 0, interlace],
    ]
    .concat();
    let stream = miniz_oxide::deflate::compress_to_vec_zlib(raw, level);
    let mut png = b"\x89PNG\r\n\x1A\n".to_vec();
    png.extend(chunk(*b"IHDR", &header));
    for c in before {
        png.extend_from_slice(c);
    }
    for piece in stream.chunks(stream.len().div_ceil(pieces.max(1)).max(1)) {
        png.extend(chunk(*b"IDAT", piece));
    }
    png.extend(chunk(*b"IEND", &[]));
    png
}

/// The raw image data of a `width × height` image whose pixel `(x, y)` has
/// the samples `pixel(x, y)`, each of `depth` bits, every row with filter
/// type 0, in Adam7 passes when `interlace` is set.
fn raw_rows(
    width: u32,
    height: u32,
    depth: u8,
    interlace: bool,
    pixel: &dyn Fn(u32, u32) -> Vec<u16>,
) -> Vec<u8> {
    let passes: &[(u32, u32, u32, u32)] = if interlace {
        &[
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
    } else {
        &[(0, 0, 1, 1)]
    };
    let mut raw = Vec::new();
    for &(x0, y0, dx, dy) in passes {
        if x0 >= width || y0 >= height {
            continue;
        }
        for y in (y0..height).step_by(usize::try_from(dy).unwrap()) {
            raw.push(0);
            let (mut byte, mut bits) = (0_u8, 0_u8);
            for x in (x0..width).step_by(usize::try_from(dx).unwrap()) {
                for sample in pixel(x, y) {
                    match depth {
                        16 => raw.extend_from_slice(&sample.to_be_bytes()),
                        8 => raw.push(u8::try_from(sample).unwrap()),
                        _ => {
                            byte |= u8::try_from(sample).unwrap() << (8 - depth - bits);
                            bits += depth;
                            if bits == 8 {
                                raw.push(byte);
                                (byte, bits) = (0, 0);
                            }
                        }
                    }
                }
            }
            if bits > 0 {
                raw.push(byte);
            }
        }
    }
    raw
}

/// A minimal ICC profile of colour `space` that libpng accepts, as the unit
/// tests of the chunk walk build it. Invented; it describes no device.
fn profile(space: [u8; 4]) -> Vec<u8> {
    let mut profile = vec![0; 144];
    profile[0..4].copy_from_slice(&144_u32.to_be_bytes());
    profile[8] = 2;
    profile[12..16].copy_from_slice(b"mntr");
    profile[16..20].copy_from_slice(&space);
    profile[20..24].copy_from_slice(b"XYZ ");
    profile[36..40].copy_from_slice(b"acsp");
    profile[128..132].copy_from_slice(&1_u32.to_be_bytes());
    profile[132..136].copy_from_slice(b"wtpt");
    profile[136..140].copy_from_slice(&132_u32.to_be_bytes());
    profile[140..144].copy_from_slice(&12_u32.to_be_bytes());
    profile
}

/// Asserts that `optimise_png` keeps the pixels of `input` and never grows.
fn assert_same_pixels(input: &[u8]) -> Vec<u8> {
    let out = optimise(input).unwrap();
    assert!(out.len() <= input.len(), "{} > {}", out.len(), input.len());
    assert_eq!(rgba16(&out), rgba16(input));
    out
}

#[test]
fn a_16_bit_transparent_colour_matching_no_pixel_stays_unmatched() {
    // Every sample repeats its high byte, so oxipng could store 8 bits; the
    // transparent colour 0x0034 matches none of them, but its low byte is a
    // pixel value. Greyscale, then truecolour.
    let grey: Vec<u8> = (0..64_u8)
        .flat_map(|y| {
            let mut row = vec![0];
            row.extend((0..64_u8).flat_map(|x| [x ^ y, x ^ y]));
            row
        })
        .collect();
    let input = build(
        (64, 64, 16, 0, 0),
        &[chunk(*b"tRNS", &[0, 0x34])],
        &grey,
        6,
        1,
    );
    assert_same_pixels(&input);
    let colour = raw_rows(32, 32, 16, false, &|x, y| {
        let v = u16::from(u8::try_from((x * 7 + y) % 256).unwrap());
        vec![v * 257, (v ^ 0x12) * 257, (v ^ 0x34) * 257]
    });
    let input = build(
        (32, 32, 16, 2, 0),
        &[chunk(*b"tRNS", &[0, 0x12, 0, 0x34, 0, 0x56])],
        &colour,
        6,
        1,
    );
    assert_same_pixels(&input);
}

#[test]
fn a_transparency_longer_than_the_palette_leaves_the_image_opaque() {
    // 1-bit indexed, two palette entries, three alpha values: libpng and
    // the png crate both ignore the tRNS.
    let raw = raw_rows(8, 2, 1, false, &|x, _| vec![u16::from(x % 2 == 0)]);
    let input = build(
        (8, 2, 1, 3, 0),
        &[
            chunk(*b"PLTE", &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60]),
            chunk(*b"tRNS", &[0, 0, 0]),
        ],
        &raw,
        6,
        1,
    );
    let out = assert_same_pixels(&input);
    assert!(rgba16(&out).2.chunks(4).all(|pixel| pixel[3] == u16::MAX));
}

#[test]
fn fully_transparent_pixels_keep_their_colour() {
    let raw = raw_rows(16, 16, 8, false, &|x, y| {
        let alpha = if (x + y) % 2 == 0 { 0 } else { 255 };
        vec![
            u16::try_from(x * 16).unwrap(),
            u16::try_from(y * 16).unwrap(),
            7,
            alpha,
        ]
    });
    assert_same_pixels(&build((16, 16, 8, 6, 0), &[], &raw, 6, 1));
}

/// The values drawn for one generated PNG of the structure-aware property.
#[derive(Debug, Clone)]
struct Spec {
    /// The colour type and bit depth, a legal pair.
    shape: (u8, u8),
    /// The width, the height and whether the image is interlaced.
    size: (u32, u32, bool),
    /// How the pixels are drawn: 0 random, 1 a few distinct ones, 2 16-bit
    /// samples that repeat their high byte, 3 some fully transparent ones.
    mode: u8,
    /// The samples of the few distinct pixels of mode 1.
    prototypes: Vec<u16>,
    /// The samples of the random pixels.
    noise: Vec<u16>,
    /// Picks the number of palette entries.
    entries: u16,
    /// A `tRNS` chunk: which pixel or length it is built from, and whether
    /// its colour is moved off that pixel's.
    transparency: Option<(Index, bool)>,
    /// The length of a truecolour image's suggested palette, in entries.
    suggested: u8,
    /// The palette and alpha bytes.
    bytes: Vec<u8>,
    /// Whether to add `sRGB`, `gAMA` and `iCCP`.
    colour_chunks: (bool, bool, bool),
    /// Whether to add the HDR chunks `cICP`, `mDCV` and `cLLI`.
    hdr: bool,
    /// An animation frame after the image data: picks its size and its
    /// offset inside the canvas, and says whether the default image is the
    /// animation's first frame.
    frame: Option<(Index, Index, bool)>,
    /// The zlib level of the image data and how many IDAT chunks hold it.
    stream: (u8, usize),
}

impl Spec {
    /// The samples per pixel.
    fn channels(&self) -> usize {
        match self.shape.0 {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            _ => 4,
        }
    }

    /// The palette entries: up to 3 more than the bit depth indexes.
    fn entries(&self) -> usize {
        1 + usize::from(self.entries) % ((1_usize << self.shape.1.min(8)) + 3).min(256)
    }

    /// The palette entries the pixels can index.
    fn usable(&self) -> usize {
        self.entries().min(1 << self.shape.1.min(8))
    }

    /// The largest sample value.
    fn max(&self) -> u16 {
        u16::try_from((1_u32 << self.shape.1) - 1).unwrap()
    }

    /// The samples of pixel `(x, y)`.
    fn pixel(&self, x: u32, y: u32) -> Vec<u16> {
        let (colour, depth) = self.shape;
        let channels = self.channels();
        let index = usize::try_from(y * self.size.0 + x).unwrap();
        (0..channels)
            .map(|channel| {
                let drawn = match self.mode {
                    1 => self.prototypes[(index % 3) * channels + channel],
                    _ => self.noise[index * channels + channel],
                };
                let value = if colour == 3 {
                    drawn % u16::try_from(self.usable()).unwrap()
                } else {
                    drawn & self.max()
                };
                let alpha = channels.is_multiple_of(2) && channel == channels - 1;
                match self.mode {
                    2 if depth == 16 => (value & 0xFF) * 257,
                    3 if alpha && value.is_multiple_of(2) => 0,
                    _ => value,
                }
            })
            .collect()
    }

    /// The `tRNS` chunk, if any. For greyscale and truecolour it is a
    /// pixel's colour, or one moved off it in the high byte at 16 bits (so
    /// that it matches no pixel whose samples repeat their high byte) or in
    /// the lowest bit below. For an indexed image it has 1 to 2 entries more
    /// than the palette; lengths between what the bit depth indexes and the
    /// palette, where libpng and the png crate disagree, are left out.
    fn transparency(&self) -> Option<Vec<u8>> {
        let (at, moved) = self.transparency?;
        let (colour, depth) = self.shape;
        match colour {
            0 | 2 => {
                let x = u32::try_from(at.index(usize::try_from(self.size.0).unwrap())).unwrap();
                let flip = match (moved, depth) {
                    (false, _) => 0,
                    (true, 16) => 0x0100,
                    (true, _) => 1,
                };
                let data: Vec<u8> = self
                    .pixel(x, 0)
                    .iter()
                    .flat_map(|&sample| ((sample ^ flip).min(self.max())).to_be_bytes())
                    .collect();
                Some(chunk(*b"tRNS", &data))
            }
            3 => {
                let len = 1 + at.index(self.entries() + 2);
                let len = if len > self.usable() && len <= self.entries() {
                    self.usable()
                } else {
                    len
                };
                Some(chunk(*b"tRNS", &self.bytes[self.bytes.len() - len..]))
            }
            _ => None,
        }
    }

    /// The whole PNG.
    fn png(&self) -> Vec<u8> {
        let (colour, depth) = self.shape;
        let (width, height, interlace) = self.size;
        let (srgb, gama, iccp) = self.colour_chunks;
        let mut before = Vec::new();
        if srgb {
            before.push(chunk(*b"sRGB", &[0]));
        }
        if gama {
            before.push(chunk(*b"gAMA", &45_455_u32.to_be_bytes()));
        }
        if iccp {
            let space = if colour & 2 == 0 { *b"GRAY" } else { *b"RGB " };
            let stream = miniz_oxide::deflate::compress_to_vec_zlib(&profile(space), 6);
            before.push(chunk(*b"iCCP", &[&b"p\0\0"[..], &stream].concat()));
        }
        if self.hdr {
            // BT.709 primaries with the sRGB transfer, an invented mastering
            // display of 1000 cd/m², and invented light levels.
            before.push(chunk(*b"cICP", &[1, 13, 0, 0]));
            let mut mdcv = vec![
                0x7D, 0x00, 0x40, 0x00, 0x1D, 0x4C, 0x75, 0x30, 0x0B, 0xB8, 0x05, 0xDC,
            ];
            mdcv.extend_from_slice(&[0x3D, 0x13, 0x40, 0x42]);
            mdcv.extend_from_slice(&10_000_000_u32.to_be_bytes());
            mdcv.extend_from_slice(&50_u32.to_be_bytes());
            before.push(chunk(*b"mDCV", &mdcv));
            before.push(chunk(*b"cLLI", &[0, 0x98, 0x96, 0x80, 0, 0x3D, 0x09, 0]));
        }
        if let Some((_, _, true)) = self.frame {
            before.push(chunk(*b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]));
            before.push(chunk(*b"fcTL", &fctl(0, (width, height), (0, 0))));
        } else if self.frame.is_some() {
            before.push(chunk(*b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        }
        if colour == 3 {
            before.push(chunk(*b"PLTE", &self.bytes[..3 * self.entries()]));
        }
        before.extend(self.transparency());
        if self.suggested > 0 && colour != 3 {
            // A suggested palette, which only a truecolour image may have
            // and every decoder ignores.
            before.push(chunk(
                *b"PLTE",
                &self.bytes[..3 * usize::from(self.suggested)],
            ));
        }
        let raw = raw_rows(width, height, depth, interlace, &|x, y| self.pixel(x, y));
        let ihdr = (width, height, depth, colour, u8::from(interlace));
        let mut png = build(ihdr, &before, &raw, self.stream.0, self.stream.1);
        if let Some((size, offset, first)) = self.frame {
            // A frame of the same format, its pixels those of the canvas one
            // column to the right, so every sample stays a valid one.
            let (w, h) = (
                1 + u32::try_from(size.index(usize::try_from(width).unwrap())).unwrap(),
                1 + u32::try_from(size.index(usize::try_from(height).unwrap())).unwrap(),
            );
            let x = u32::try_from(offset.index(usize::try_from(width - w + 1).unwrap())).unwrap();
            let y = u32::try_from(offset.index(usize::try_from(height - h + 1).unwrap())).unwrap();
            let rows = raw_rows(w, h, depth, interlace, &|fx, fy| {
                self.pixel((fx + 1) % width, fy)
            });
            let data = miniz_oxide::deflate::compress_to_vec_zlib(&rows, self.stream.0);
            let sequence = u32::from(first);
            let after = [
                chunk(*b"fcTL", &fctl(sequence, (w, h), (x, y))),
                chunk(
                    *b"fdAT",
                    &[&(sequence + 1).to_be_bytes()[..], &data].concat(),
                ),
            ]
            .concat();
            let iend = png.len() - 12;
            png.splice(iend..iend, after);
        }
        png
    }
}

/// The data of an `fcTL` chunk numbered `sequence` for a frame of
/// `width × height` at `(x, y)`, shown for 1/10 s, with operations 0.
fn fctl(sequence: u32, (width, height): (u32, u32), (x, y): (u32, u32)) -> Vec<u8> {
    let mut data: Vec<u8> = [sequence, width, height, x, y]
        .iter()
        .flat_map(|f| f.to_be_bytes())
        .collect();
    data.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
    data
}

/// Valid PNGs of every colour type and bit depth, interlaced or not, with
/// optional palettes (longer than the bit depth can index, too), `tRNS`
/// chunks (over-long ones included) and colour chunks, and pixels that are
/// random, few distinct ones, 16-bit samples that repeat their high byte,
/// or partly fully transparent; with HDR chunks and an animation frame
/// whose default image is or is not part of the animation.
fn specs() -> impl Strategy<Value = Spec> {
    let shape = prop_oneof![
        select(vec![1_u8, 2, 4, 8, 16]).prop_map(|depth| (0_u8, depth)),
        select(vec![8_u8, 16]).prop_map(|depth| (2_u8, depth)),
        select(vec![1_u8, 2, 4, 8]).prop_map(|depth| (3_u8, depth)),
        select(vec![8_u8, 16]).prop_map(|depth| (4_u8, depth)),
        select(vec![8_u8, 16]).prop_map(|depth| (6_u8, depth)),
    ];
    (
        (shape, (1..=17_u32, 1..=9_u32, any::<bool>()), 0..4_u8),
        (vec(any::<u16>(), 12), vec(any::<u16>(), 17 * 9 * 4)),
        (
            any::<u16>(),
            prop::option::of((any::<Index>(), any::<bool>())),
            0..4_u8,
        ),
        vec(any::<u8>(), 3 * 260),
        (any::<bool>(), any::<bool>(), any::<bool>()),
        (0..=10_u8, 1..=3_usize),
        (
            any::<bool>(),
            prop::option::of((any::<Index>(), any::<Index>(), any::<bool>())),
        ),
    )
        .prop_map(
            |(
                (shape, size, mode),
                (prototypes, noise),
                (entries, transparency, suggested),
                bytes,
                colour_chunks,
                stream,
                (hdr, frame),
            )| Spec {
                shape,
                size,
                mode,
                prototypes,
                noise,
                entries,
                transparency,
                suggested,
                bytes,
                colour_chunks,
                hdr,
                frame,
                stream,
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn every_valid_png_keeps_its_pixels_frames_and_chunks_and_never_grows(spec in specs()) {
        let input = spec.png();
        let out = optimise(&input).unwrap();
        prop_assert!(out.len() <= input.len(), "{} > {}", out.len(), input.len());
        let (found, after) = chunks(&out);
        prop_assert!(after.is_empty());
        prop_assert!(found.iter().all(|(kind, _)| ALLOWED.contains(&kind)));
        prop_assert_eq!(images(&out), images(&input));
        prop_assert_eq!(icc_profile(&out), icc_profile(&input));
        for kind in [b"cICP", b"mDCV", b"cLLI", b"acTL"] {
            prop_assert_eq!(chunk_data(&out, kind), chunk_data(&input, kind));
        }
        prop_assert_eq!(count(&out, *b"fcTL"), count(&input, *b"fcTL"));
    }
}
