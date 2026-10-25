//! The WebP and GIF files of the image corpus through `optimise_webp` and
//! `optimise_gif` (ADR 0015 §7, "Image tests").
//!
//! The in-module tests use small images that libwebp encodes on the spot and
//! GIFs built by hand; these use the corpus files that `ImageMagick`, libwebp
//! and exiv2 wrote, with real metadata in them. Each test decodes the input
//! and the output independently, with libwebp's decoders (through webpx) and
//! the `gif` crate, to check what a viewer would see.

use backend3::domain::image_optimise::{
    Dimensions, ImageError, ImageFormat, NotReencoded, Optimised, Orientation, PixelBudget,
    SSIM_FLOOR, Treatment, inspect, mean_ssim, optimise_gif, optimise_webp, reencode_is_kept,
};
use proptest::{
    collection::vec,
    prelude::*,
    sample::{Index, select},
};

use super::image_optimise::{CORPUS, corpus_file, is_over_budget};

/// A payload that a polyglot file hides after the image: a ZIP header and
/// a script.
const PAYLOAD: &[u8] = b"PK\x03\x04<script>alert(1)</script>";

/// Every WebP and GIF of the corpus that holds a whole image.
fn files() -> impl Iterator<Item = (&'static str, ImageFormat)> {
    CORPUS
        .iter()
        .filter(|(path, format, ..)| {
            !is_over_budget(path) && matches!(format, ImageFormat::WebP | ImageFormat::Gif)
        })
        .map(|&(path, format, ..)| (path, format))
}

/// The optimiser for `format`.
fn optimise(format: ImageFormat, bytes: &[u8]) -> Result<Optimised, ImageError> {
    match format {
        ImageFormat::WebP => optimise_webp(bytes, PixelBudget::DEFAULT),
        ImageFormat::Gif => optimise_gif(bytes, PixelBudget::DEFAULT),
        other => panic!("{other} is not handled here"),
    }
}

/// What each file is expected to become (ADR 0033 §1): the two lossy WebP
/// files at q95, one opaque and one transparent, are re-encoded; the q80 ones
/// gain nothing from quality 85; `lossy-textured.webp` would be more than 5 %
/// smaller but stays below the SSIM floor; and nothing else is re-encoded,
/// `lossless-noise.webp` included, although a lossy encode of it would be
/// 23 % smaller.
fn expected_treatment(path: &str) -> Treatment {
    match path {
        "webp/lossy-metadata.webp" | "webp/lossy-alpha-q95.webp" => Treatment::ReEncoded,
        "webp/lossy.webp" | "webp/lossy-alpha.webp" => {
            Treatment::ReEncodeDiscarded(NotReencoded::NotSmallerEnough)
        }
        "webp/lossy-textured.webp" => Treatment::ReEncodeDiscarded(NotReencoded::BelowSsimFloor),
        _ => Treatment::Stripped,
    }
}

/// The chunks of a WebP file, as `(FourCC, payload)` pairs.
fn chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let size = usize::try_from(u32::from_le_bytes(bytes[4..8].try_into().unwrap())).unwrap();
    let body = &bytes[12..8 + size];
    let mut chunks = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let len =
            usize::try_from(u32::from_le_bytes(body[at + 4..at + 8].try_into().unwrap())).unwrap();
        chunks.push((
            body[at..at + 4].try_into().unwrap(),
            body[at + 8..at + 8 + len].to_vec(),
        ));
        at += 8 + len + len % 2;
    }
    chunks
}

/// The payload of the first `fourcc` chunk of a WebP file.
fn chunk(bytes: &[u8], fourcc: [u8; 4]) -> Option<Vec<u8>> {
    chunks(bytes)
        .into_iter()
        .find(|(f, _)| *f == fourcc)
        .map(|(_, p)| p)
}

/// Every frame of a WebP as libwebp's animation decoder composes it, which
/// also plays a still image as one frame: RGBA pixels and duration.
fn webp_frames(bytes: &[u8]) -> Vec<(Vec<u8>, u32)> {
    let mut decoder = webpx::AnimationDecoder::new(bytes).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        frames.push((frame.data, frame.duration_ms));
    }
    frames
}

/// A GIF frame as the `gif` crate decodes it.
type GifFrame = (Vec<u8>, u16, [u16; 4], u8, Option<u8>);

/// Every frame of a GIF as the `gif` crate decodes it: indexed pixels,
/// delay, position and size, disposal, transparency. Also its loop setting
/// and ICC profile.
fn gif_frames(bytes: &[u8]) -> (Vec<GifFrame>, gif::Repeat, Option<Vec<u8>>) {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = options.read_info(bytes).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        let place = [frame.left, frame.top, frame.width, frame.height];
        frames.push((
            frame.buffer.to_vec(),
            frame.delay,
            place,
            frame.dispose as u8,
            frame.transparent,
        ));
    }
    let profile = decoder.icc_profile().map(<[u8]>::to_vec);
    (frames, decoder.repeat(), profile)
}

/// The luma plane of a lossy WebP, as libwebp decodes it to YUV 4:2:0, with
/// its dimensions: the plane that the keep rule compares (ADR 0033 §1).
fn luma(bytes: &[u8]) -> (Vec<u8>, Dimensions) {
    let planes = webpx::decode_yuv(bytes).unwrap();
    let width = usize::try_from(planes.width).unwrap();
    let plane = planes
        .y
        .chunks(planes.y_stride)
        .flat_map(|row| &row[..width])
        .copied()
        .collect();
    (plane, Dimensions::new(planes.width, planes.height).unwrap())
}

#[test]
fn format_dimensions_and_header_never_change_and_the_output_is_never_larger() {
    for (path, format) in files() {
        let input = corpus_file(path);
        let optimised = optimise(format, &input).unwrap_or_else(|e| panic!("{path}: {e}"));
        let header = inspect(&input, PixelBudget::DEFAULT).unwrap();
        assert_eq!(optimised.header, header, "{path}");
        assert_eq!(
            inspect(&optimised.bytes, PixelBudget::DEFAULT),
            Ok(header),
            "{path}"
        );
        assert!(optimised.bytes.len() <= input.len(), "{path}");
        assert_eq!(optimised.treatment, expected_treatment(path), "{path}");
        assert_eq!(optimised.re_encode_failure, None, "{path}");
        // Optimising the output again changes nothing (or re-encodes it once
        // more, which the keep rule has to justify).
        let again = optimise(format, &optimised.bytes).unwrap();
        if again.treatment != Treatment::ReEncoded {
            assert_eq!(again.bytes, optimised.bytes, "{path}");
        }
    }
}

#[test]
fn an_appended_payload_is_never_stored() {
    for (path, format) in files() {
        let input = corpus_file(path);
        let polyglot = [&input[..], PAYLOAD].concat();
        let clean = optimise(format, &input).unwrap();
        let optimised = optimise(format, &polyglot).unwrap();
        assert_eq!(optimised, clean, "{path}");
        assert!(
            !optimised.bytes.windows(4).any(|w| w == b"PK\x03\x04"),
            "{path}"
        );
    }
}

#[test]
fn webp_frames_and_lossless_pixels_are_identical() {
    for (path, _) in files().filter(|(path, format)| {
        *format == ImageFormat::WebP && expected_treatment(path) != Treatment::ReEncoded
    }) {
        let input = corpus_file(path);
        let optimised = optimise_webp(&input, PixelBudget::DEFAULT).unwrap();
        let (before, after) = (webp_frames(&input), webp_frames(&optimised.bytes));
        let expected_frames = if path.starts_with("webp/animated") {
            3
        } else {
            1
        };
        assert_eq!(before.len(), expected_frames, "{path}");
        assert_eq!(after, before, "{path}");
    }
}

#[test]
fn gif_frames_timing_and_loop_are_identical() {
    for (path, _) in files().filter(|(_, format)| *format == ImageFormat::Gif) {
        let input = corpus_file(path);
        let optimised = optimise_gif(&input, PixelBudget::DEFAULT).unwrap();
        let (before, after) = (gif_frames(&input), gif_frames(&optimised.bytes));
        let expected_frames = if path == "gif/animated.gif" || path == "gif/metadata.gif" {
            3
        } else {
            1
        };
        assert_eq!(before.0.len(), expected_frames, "{path}");
        assert_eq!(after, before, "{path}");
    }
}

#[test]
fn webp_metadata_keeps_only_the_profile_and_the_orientation() {
    for path in [
        "webp/lossy-metadata.webp",
        "webp/lossless-metadata.webp",
        "webp/animated-metadata.webp",
    ] {
        let input = corpus_file(path);
        // The corpus file holds what the test removes.
        assert!(chunk(&input, *b"XMP ").is_some(), "{path}");
        let original_exif = chunk(&input, *b"EXIF").unwrap();
        assert!(
            original_exif.windows(10).any(|w| w == b"0000000000"),
            "{path}"
        );
        assert_eq!(
            Orientation::from_tiff(&original_exif),
            Orientation::new(6),
            "{path}"
        );

        let bytes = optimise_webp(&input, PixelBudget::DEFAULT).unwrap().bytes;
        let kept: Vec<[u8; 4]> = chunks(&bytes).into_iter().map(|(f, _)| f).collect();
        assert!(!kept.contains(b"XMP "), "{path}: {kept:?}");
        assert_eq!(chunk(&bytes, *b"ICCP"), chunk(&input, *b"ICCP"), "{path}");
        let orientation_only = Orientation::new(6)
            .unwrap()
            .tiff_replacing(original_exif.len());
        assert_eq!(chunk(&bytes, *b"EXIF"), Some(orientation_only), "{path}");
        // VP8X flags: ICC and EXIF, animation as before, XMP gone.
        let flags = chunk(&bytes, *b"VP8X").unwrap()[0];
        let animation = chunk(&input, *b"VP8X").unwrap()[0] & 0x02;
        assert_eq!(flags, 0x20 | 0x08 | animation, "{path}");
        // No trace of the serial number, the GPS block or the XMP packet.
        for trace in [&b"0000000000"[..], b"backend3 test corpus", b"xmpmeta"] {
            assert!(!bytes.windows(trace.len()).any(|w| w == trace), "{path}");
        }
    }
}

#[test]
fn gif_metadata_keeps_only_the_profile_and_the_loop() {
    let input = corpus_file("gif/metadata.gif");
    let has = |bytes: &[u8], text: &[u8]| bytes.windows(text.len()).any(|w| w == text);
    assert!(has(&input, b"backend3 test corpus") && has(&input, b"XMP DataXMP"));
    let bytes = optimise_gif(&input, PixelBudget::DEFAULT).unwrap().bytes;
    assert!(!has(&bytes, b"backend3 test corpus"));
    assert!(!has(&bytes, b"XMP DataXMP"));
    let (_, repeat, profile) = gif_frames(&bytes);
    assert_eq!(repeat, gif::Repeat::Infinite);
    assert_eq!(profile.map(|p| p.len()), Some(132));
}

#[test]
fn every_lossy_re_encode_reaches_ssim_0_98_on_luma() {
    let mut re_encoded = 0;
    for (path, _) in
        files().filter(|(path, format)| *format == ImageFormat::WebP && path.contains("lossy"))
    {
        let input = corpus_file(path);
        let optimised = optimise_webp(&input, PixelBudget::DEFAULT).unwrap();
        if optimised.treatment != Treatment::ReEncoded {
            continue;
        }
        // The stripped original keeps the input's bitstream, so its luma is
        // the input's; the measure is the keep rule's own mean_ssim.
        let ((original, dimensions), (output, _)) = (luma(&input), luma(&optimised.bytes));
        let score = mean_ssim(&original, &output, dimensions).unwrap();
        assert!(score >= SSIM_FLOOR, "{path}: SSIM {score}");
        // The alpha channel is not re-encoded: its chunk is kept as it is.
        let alpha = |bytes: &[u8]| -> Vec<u8> {
            let (rgba, ..) = webpx::decode_rgba(bytes).unwrap();
            rgba.iter().skip(3).step_by(4).copied().collect()
        };
        assert_eq!(alpha(&optimised.bytes), alpha(&input), "{path}");
        assert_eq!(
            chunk(&optimised.bytes, *b"ALPH"),
            chunk(&input, *b"ALPH"),
            "{path}"
        );
        re_encoded += 1;
    }
    // One opaque and one transparent file.
    assert_eq!(
        re_encoded, 2,
        "the corpus must hold an opaque and a transparent lossy WebP that are re-encoded"
    );
}

#[test]
fn a_textured_lossy_webp_below_the_floor_keeps_its_stripped_original() {
    // ADR 0033: the corpus holds a lossy WebP whose re-encode is at least
    // 5 % smaller but below the SSIM floor. Re-encode it here as the
    // optimiser does, from its YUV planes at quality 85 and method 4.
    let input = corpus_file("webp/lossy-textured.webp");
    let planes = webpx::decode_yuv(&input).unwrap();
    let re_encode = webpx::Encoder::new_yuv((&planes).into())
        .quality(85.0)
        .method(4)
        .encode(webpx::Unstoppable)
        .unwrap();
    assert!(reencode_is_kept(input.len(), re_encode.len()));
    let ((original, dimensions), (re_encoded, _)) = (luma(&input), luma(&re_encode));
    let score = mean_ssim(&original, &re_encoded, dimensions).unwrap();
    assert!(score < SSIM_FLOOR, "SSIM {score}");
    let optimised = optimise_webp(&input, PixelBudget::DEFAULT).unwrap();
    assert_eq!(
        optimised.treatment,
        Treatment::ReEncodeDiscarded(NotReencoded::BelowSsimFloor)
    );
    assert_eq!(optimised.bytes, input);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn damaged_corpus_files_never_panic(
        (path, format) in select(files().collect::<Vec<_>>()),
        damage in vec((any::<Index>(), any::<u8>()), 1..6),
        cut in any::<Index>(),
        tail in vec(any::<u8>(), 0..8),
    ) {
        let mut bytes = corpus_file(path);
        for (at, byte) in damage {
            let at = at.index(bytes.len());
            bytes[at] = byte;
        }
        bytes.truncate(cut.index(bytes.len() + 1));
        bytes.extend(tail);
        if let Ok(optimised) = optimise(format, &bytes) {
            prop_assert!(optimised.bytes.len() <= bytes.len());
            prop_assert_eq!(inspect(&optimised.bytes, PixelBudget::DEFAULT), Ok(optimised.header));
        }
    }
}
