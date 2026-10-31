//! The JPEG files of the image corpus through
//! `domain::image_optimise::optimise_jpeg` (ADR 0015 §7, "Image tests", as
//! amended by ADR 0033).
//!
//! For every JPEG in `tests/fixtures/images/jpeg/` these tests assert what
//! ADR 0015 §7 asks of the optimiser:
//!
//! - the format and the stored dimensions stay, so the stored name keeps its
//!   extension;
//! - the Orientation tag stays, every other EXIF field (GPS included) goes,
//!   and so do XMP, IPTC, comments, thumbnails and appended images, while
//!   the ICC profile stays;
//! - the output is never larger than the upload;
//! - every re-encoded file reaches a luma SSIM of at least 0.98 at full
//!   resolution, compared with the stripped original, measured with the
//!   same `mean_ssim` that the keep rule uses (ADR 0033 §1);
//! - the strip leaves every decoded pixel as it was, and a re-encode keeps
//!   the colours (its chroma channels stay close to the stripped original's).
//!
//! [`JPEG_CORPUS`] fixes which of the two candidates each file keeps, so a
//! change of the encoder or of its settings shows up here instead of
//! silently changing what is stored.
//!
//! What the corpus covers: every file is the same 67 × 41 synthetic picture
//! (a gradient with two shapes). `progressive.jpg` and `camera.jpg` were
//! saved at quality 85 with 4:2:0 chroma, like the re-encode itself, and
//! `camera.jpg` is `progressive.jpg` with camera metadata spliced in; both
//! are re-encoded. `high-quality.jpg` is the common camera original,
//! quality 95 with full-resolution (4:4:4) chroma. Its quality-85 re-encode
//! is more than 5 % smaller but reaches only 0.974, as half of the
//! photographs measured in `docs/design/image-encoders.md` do, so the SSIM
//! floor keeps its stripped original: the case that ADR 0033 §1 was
//! written for. `soft-444.jpg` is the same kind of original with soft
//! edges: its re-encode reaches 0.994 and is kept, so a 4:4:4 original
//! whose chroma the re-encode halves to 4:2:0 (ADR 0033 §2) stays covered
//! by the chroma test.

use std::panic;

use backend3::domain::image_optimise::{
    Dimensions, ImageFormat, JpegKept, NotReencoded, OptimisedJpeg, PixelBudget, SSIM_FLOOR,
    inspect, mean_ssim, optimise_jpeg, sniff, strip_jpeg,
};
use proptest::{collection::vec, prelude::*, sample::Index, sample::select};

use super::image_optimise::{CORPUS, corpus_file};

const REENCODED: JpegKept = JpegKept::Reencoded;
const NOT_SMALLER: JpegKept = JpegKept::Stripped(NotReencoded::NotSmallerEnough);
const BELOW_FLOOR: JpegKept = JpegKept::Stripped(NotReencoded::BelowSsimFloor);
const COLOUR_MODEL: JpegKept = JpegKept::Stripped(NotReencoded::ColourModel);

/// Every JPEG of the corpus that decodes, and the bytes it keeps.
const JPEG_CORPUS: &[(&str, JpegKept)] = &[
    ("jpeg/baseline.jpg", NOT_SMALLER),
    ("jpeg/camera.jpg", REENCODED),
    ("jpeg/cmyk.jpg", COLOUR_MODEL),
    ("jpeg/comment.jpg", NOT_SMALLER),
    ("jpeg/exif-orientation.jpg", NOT_SMALLER),
    ("jpeg/grayscale.jpg", NOT_SMALLER),
    ("jpeg/high-quality.jpg", BELOW_FLOOR),
    ("jpeg/low-quality.jpg", NOT_SMALLER),
    ("jpeg/progressive.jpg", REENCODED),
    ("jpeg/soft-444.jpg", REENCODED),
];

/// The largest mean absolute difference, in 8-bit levels, that each chroma
/// channel (Cb and Cr) of a re-encode may have from the stripped original's.
/// ADR 0015 §7 sets no chroma target, and ADR 0033 §2 accepts the 4:2:0
/// chroma of the re-encode "within the tested bound": this bound. It
/// catches a re-encode that loses or mixes up the colours. The corpus
/// re-encodes stay within 1.7 (the most for `soft-444.jpg`, whose 4:4:4
/// chroma the re-encode halves to 4:2:0; `high-quality.jpg`, at 4.3, now
/// keeps its stripped original under the SSIM floor), while swapping Cb and
/// Cr scores about 50 and flat chroma about 29. A mean, not SSIM: SSIM of a
/// nearly flat chroma plane rates even flat chroma at 0.6 to 0.8.
const CHROMA_MEAN_ERROR_LIMIT: f64 = 8.0;

/// The EXIF segment that holds only Orientation 6, in the form the strip
/// writes it.
const ORIENTATION_6: &[u8] =
    b"\xFF\xE1\0\x22Exif\0\0II\x2A\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";

fn optimise(path: &str) -> (Vec<u8>, OptimisedJpeg) {
    let upload = corpus_file(path);
    let result = optimise_jpeg(&upload, PixelBudget::DEFAULT)
        .unwrap_or_else(|error| panic!("{path}: {error}"));
    (upload, result)
}

/// The stripped original of a corpus upload, which ADR 0015 §7 compares a
/// re-encode with.
fn stripped(path: &str, upload: &[u8]) -> Vec<u8> {
    strip_jpeg(upload, PixelBudget::DEFAULT).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// The segments of a JPEG before its first scan, as `(marker, payload)`,
/// and the length of everything after the end of image.
fn segments(jpeg: &[u8]) -> (Vec<(u8, &[u8])>, usize) {
    let mut found = Vec::new();
    let mut at = 2;
    while jpeg[at + 1] != 0xDA {
        let length = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        found.push((jpeg[at + 1], &jpeg[at + 4..at + 2 + length]));
        at += 2 + length;
    }
    // The first end of image after the first scan ends the image: in scan
    // data an FF byte is never followed by D9, and an appended image has an
    // end of image of its own.
    let end = at
        + jpeg[at..]
            .windows(2)
            .position(|pair| pair == [0xFF, 0xD9])
            .unwrap()
        + 2;
    (found, jpeg.len() - end)
}

/// A JPEG decoded by libjpeg into its own channels, one plane each: the
/// luma plane alone for greyscale, Y, Cb and Cr at full resolution for
/// YCbCr. Returns the planes, the width and the height.
fn planes(jpeg: &[u8]) -> (Vec<Vec<u8>>, usize, usize) {
    panic::catch_unwind(|| {
        let decompress = mozjpeg::Decompress::new_mem(jpeg).unwrap();
        let (width, height) = decompress.size();
        let grey = decompress.color_space() == mozjpeg::ColorSpace::JCS_GRAYSCALE;
        let mut started = if grey {
            decompress.grayscale().unwrap()
        } else {
            decompress
                .to_colorspace(mozjpeg::ColorSpace::JCS_YCbCr)
                .unwrap()
        };
        let samples: Vec<u8> = started.read_scanlines().unwrap();
        started.finish().unwrap();
        let channels = if grey { 1 } else { 3 };
        let planes = (0..channels)
            .map(|channel| {
                samples
                    .iter()
                    .skip(channel)
                    .step_by(channels)
                    .copied()
                    .collect()
            })
            .collect();
        (planes, width, height)
    })
    .expect("libjpeg decodes the file")
}

#[test]
fn every_decodable_jpeg_of_the_corpus_is_listed() {
    let jpegs: Vec<&str> = CORPUS
        .iter()
        .filter(|(path, format, ..)| *format == ImageFormat::Jpeg && path.starts_with("jpeg/"))
        .map(|(path, ..)| *path)
        .collect();
    let listed: Vec<&str> = JPEG_CORPUS.iter().map(|(path, _)| *path).collect();
    assert_eq!(jpegs, listed);
}

#[test]
fn every_file_keeps_its_expected_candidate() {
    for &(path, expected) in JPEG_CORPUS {
        assert_eq!(optimise(path).1.kept, expected, "{path}");
    }
}

#[test]
fn the_format_and_dimensions_stay_and_nothing_grows() {
    for &(path, _) in JPEG_CORPUS {
        let (upload, result) = optimise(path);
        let header = inspect(&upload, PixelBudget::DEFAULT).unwrap();
        assert_eq!(result.header, header, "{path}");
        assert_eq!(
            inspect(&result.bytes, PixelBudget::DEFAULT),
            Ok(header),
            "{path}"
        );
        // The extension guarantee: the optimiser returns bytes, not a
        // name, and the upload keeps its name, so its extension stays
        // right exactly when the bytes are still a JPEG.
        assert_eq!(sniff(&result.bytes), Some(ImageFormat::Jpeg), "{path}");
        assert!(result.bytes.len() <= upload.len(), "{path}");
    }
}

#[test]
fn orientation_stays_and_the_rest_of_the_metadata_goes() {
    let (upload, result) = optimise("jpeg/camera.jpg");
    let (before, appended) = segments(&upload);
    assert!(appended > 0, "the corpus file carries an appended image");
    for marker in [0xE1, 0xED, 0xFE] {
        assert!(
            before.iter().any(|(code, _)| *code == marker),
            "{marker:02X}"
        );
    }
    let (after, appended) = segments(&result.bytes);
    assert_eq!(appended, 0);
    let markers: Vec<u8> = after.iter().map(|(code, _)| *code).collect();
    assert!(
        !markers.contains(&0xED) && !markers.contains(&0xFE),
        "{markers:02X?}"
    );
    let exif: Vec<&[u8]> = after
        .iter()
        .filter(|(code, _)| *code == 0xE1)
        .map(|(_, payload)| *payload)
        .collect();
    assert_eq!(exif, [&ORIENTATION_6[4..]]);
    // The invented make, serial number, date and GPS position are gone, and
    // so is the MPF index of the appended image.
    for text in [
        &b"backend3 test"[..],
        b"TEST-0001",
        b"2026:10:25",
        b"MPF\0",
        b"xmpmeta",
        b"test record",
    ] {
        assert!(upload.windows(text.len()).any(|window| window == text));
        assert!(
            !result
                .bytes
                .windows(text.len())
                .any(|window| window == text),
            "{text:?}"
        );
    }
    // A file whose EXIF holds only the orientation keeps exactly that.
    let (_, minimal) = optimise("jpeg/exif-orientation.jpg");
    assert!(
        minimal
            .bytes
            .windows(ORIENTATION_6.len())
            .any(|window| window == ORIENTATION_6)
    );
}

#[test]
fn the_icc_profile_stays_byte_for_byte() {
    let (upload, result) = optimise("jpeg/camera.jpg");
    let icc = |jpeg| -> Vec<Vec<u8>> {
        segments(jpeg)
            .0
            .into_iter()
            .filter(|(code, payload)| *code == 0xE2 && payload.starts_with(b"ICC_PROFILE\0"))
            .map(|(_, payload)| payload.to_vec())
            .collect()
    };
    let kept = icc(&result.bytes);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept, icc(&upload));
}

#[test]
fn a_cmyk_image_keeps_its_adobe_segment() {
    let (_, result) = optimise("jpeg/cmyk.jpg");
    let (after, _) = segments(&result.bytes);
    assert!(
        after
            .iter()
            .any(|(code, payload)| *code == 0xEE && payload.starts_with(b"Adobe"))
    );
}

#[test]
fn every_reencoded_file_reaches_the_ssim_floor() {
    let mut reencoded = 0;
    for &(path, _) in JPEG_CORPUS {
        let (upload, result) = optimise(path);
        if result.kept != JpegKept::Reencoded {
            continue;
        }
        reencoded += 1;
        let (original, width, height) = planes(&stripped(path, &upload));
        let (output, ..) = planes(&result.bytes);
        let size = Dimensions::new(
            u32::try_from(width).unwrap(),
            u32::try_from(height).unwrap(),
        )
        .unwrap();
        let ssim = mean_ssim(&original[0], &output[0], size).unwrap();
        // ADR 0015 §7's 0.98, written out, so the test does not follow a
        // change of the constant it checks.
        assert!(ssim >= 0.98 && ssim >= SSIM_FLOOR, "{path}: SSIM {ssim}");
    }
    assert!(reencoded >= 2, "the corpus has re-encoded files");
}

/// The mean absolute difference of two planes of the same size, in levels.
fn mean_absolute_error(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "planes of different sizes");
    let total: f64 = a
        .iter()
        .zip(b)
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum();
    total / f64::from(u32::try_from(a.len()).expect("the plane fits a u32 sample count"))
}

#[test]
fn the_mean_absolute_error_counts_levels() {
    assert!(mean_absolute_error(&[10, 20, 30, 40], &[10, 20, 30, 40]).abs() < f64::EPSILON);
    assert!((mean_absolute_error(&[0, 255, 7, 9], &[2, 251, 7, 9]) - 1.5).abs() < f64::EPSILON);
}

#[test]
fn every_reencoded_file_keeps_its_colours() {
    for &(path, _) in JPEG_CORPUS {
        let (upload, result) = optimise(path);
        if result.kept != JpegKept::Reencoded {
            continue;
        }
        let (original, width, height) = planes(&stripped(path, &upload));
        let (output, ..) = planes(&result.bytes);
        assert_eq!(original.len(), output.len(), "{path}: the colour model");
        assert_eq!(original[0].len(), width * height, "{path}");
        for (name, channel) in [("Cb", 1), ("Cr", 2)] {
            let error = mean_absolute_error(&original[channel], &output[channel]);
            assert!(
                error <= CHROMA_MEAN_ERROR_LIMIT,
                "{path}: {name} differs by {error} levels on average"
            );
        }
    }
}

/// The samples of a JPEG as libjpeg decodes it by default: greyscale, RGB,
/// or CMYK for CMYK and YCCK.
fn pixels(jpeg: &[u8]) -> Vec<u8> {
    panic::catch_unwind(|| {
        let (mozjpeg::Format::RGB(mut started)
        | mozjpeg::Format::Gray(mut started)
        | mozjpeg::Format::CMYK(mut started)) =
            mozjpeg::Decompress::new_mem(jpeg).unwrap().image().unwrap();
        let samples: Vec<u8> = started.read_scanlines().unwrap();
        started.finish().unwrap();
        samples
    })
    .expect("libjpeg decodes the file")
}

#[test]
fn the_strip_leaves_every_decoded_pixel_as_it_was() {
    for &(path, kept) in JPEG_CORPUS {
        let upload = corpus_file(path);
        let stripped = stripped(path, &upload);
        assert_eq!(pixels(&stripped), pixels(&upload), "{path}");
        // A file that is not re-encoded is stored as exactly that strip.
        if kept != JpegKept::Reencoded {
            assert_eq!(optimise(path).1.bytes, stripped, "{path}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn damaged_corpus_jpegs_never_panic(
        (path, _) in select(JPEG_CORPUS),
        damage in vec((any::<Index>(), any::<u8>()), 1..8),
        cut in any::<Index>(),
    ) {
        let mut upload = corpus_file(path);
        for (at, byte) in damage {
            let at = at.index(upload.len());
            upload[at] = byte;
        }
        upload.truncate(cut.index(upload.len() + 1).max(upload.len() * 3 / 4));
        if let Ok(result) = optimise_jpeg(&upload, PixelBudget::DEFAULT) {
            prop_assert!(result.bytes.len() <= upload.len());
            prop_assert_eq!(inspect(&result.bytes, PixelBudget::DEFAULT), Ok(result.header));
        }
    }
}
