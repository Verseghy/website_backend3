//! The image corpus of `tests/fixtures/images/` through
//! `domain::image_optimise::inspect`.
//!
//! The in-module tests use headers built by hand; these use the output of
//! real encoders (libjpeg-turbo, libpng, libwebp, the GIF and APNG writers of
//! `ImageMagick` and `ffmpeg`), so the readers are checked against what
//! common encoders write. Every corpus file must be listed in [`CORPUS`], so
//! a file that a later image slice adds gets its expected header too.

use std::{fs, path::Path};

use backend3::domain::image_optimise::{
    Dimensions, HeaderDefect, ImageError, ImageFormat, ImageHeader, PixelBudget, inspect, sniff,
};
use proptest::{
    collection::vec,
    prelude::*,
    sample::{Index, select},
};

use crate::support::repository_root;

/// The corpus directory, relative to the repository root.
const CORPUS_DIR: &str = "tests/fixtures/images";

/// Every corpus file: its path below [`CORPUS_DIR`], its format and its
/// stored width and height. The files under `over-budget/` are headers only.
const CORPUS: &[(&str, ImageFormat, u32, u32)] = &[
    ("gif/animated.gif", ImageFormat::Gif, 67, 41),
    ("gif/gif87a.gif", ImageFormat::Gif, 67, 41),
    ("gif/still.gif", ImageFormat::Gif, 67, 41),
    ("jpeg/baseline.jpg", ImageFormat::Jpeg, 67, 41),
    ("jpeg/comment.jpg", ImageFormat::Jpeg, 67, 41),
    // Orientation 6 rotates the display only; the stored size stays 67x41.
    ("jpeg/exif-orientation.jpg", ImageFormat::Jpeg, 67, 41),
    ("jpeg/grayscale.jpg", ImageFormat::Jpeg, 67, 41),
    ("jpeg/progressive.jpg", ImageFormat::Jpeg, 67, 41),
    (
        "over-budget/camera-50mp.gif",
        ImageFormat::Gif,
        8_688,
        5_792,
    ),
    (
        "over-budget/camera-50mp.jpg",
        ImageFormat::Jpeg,
        8_688,
        5_792,
    ),
    (
        "over-budget/camera-50mp.png",
        ImageFormat::Png,
        8_688,
        5_792,
    ),
    (
        "over-budget/camera-50mp.webp",
        ImageFormat::WebP,
        8_688,
        5_792,
    ),
    ("png/animated.png", ImageFormat::Png, 67, 41),
    ("png/grey16.png", ImageFormat::Png, 67, 41),
    ("png/palette.png", ImageFormat::Png, 67, 41),
    ("png/rgb.png", ImageFormat::Png, 67, 41),
    ("png/rgba-interlaced.png", ImageFormat::Png, 67, 41),
    ("webp/animated.webp", ImageFormat::WebP, 67, 41),
    ("webp/lossless.webp", ImageFormat::WebP, 67, 41),
    ("webp/lossy-alpha.webp", ImageFormat::WebP, 67, 41),
    ("webp/lossy.webp", ImageFormat::WebP, 67, 41),
];

/// The bytes of a corpus file.
fn corpus_file(path: &str) -> Vec<u8> {
    let full = repository_root().join(CORPUS_DIR).join(path);
    fs::read(&full).unwrap_or_else(|error| panic!("{}: {error}", full.display()))
}

/// Whether a corpus file is a header-only file of `over-budget/`.
fn is_over_budget(path: &str) -> bool {
    path.starts_with("over-budget/")
}

fn unlimited() -> PixelBudget {
    PixelBudget::new(u64::MAX).unwrap()
}

fn header(format: ImageFormat, width: u32, height: u32) -> ImageHeader {
    ImageHeader {
        format,
        dimensions: Dimensions::new(width, height).unwrap(),
    }
}

/// The paths of every file below `dir`, relative to `root`.
fn files_below(root: &Path, dir: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_below(root, &path, found);
        } else {
            let relative = path.strip_prefix(root).unwrap();
            found.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn every_corpus_file_is_listed() {
    let root = repository_root().join(CORPUS_DIR);
    let mut found = Vec::new();
    files_below(&root, &root, &mut found);
    found.retain(|path| path != "README.md");
    found.sort();
    let listed: Vec<&str> = CORPUS.iter().map(|(path, ..)| *path).collect();
    assert_eq!(found, listed, "CORPUS must list every file, sorted by path");
}

#[test]
fn every_corpus_file_gives_its_header_under_the_default_budget() {
    for &(path, format, width, height) in CORPUS {
        let expected = if is_over_budget(path) {
            Err(ImageError::OverPixelBudget {
                format,
                dimensions: Dimensions::new(width, height).unwrap(),
                budget: PixelBudget::DEFAULT,
            })
        } else {
            Ok(header(format, width, height))
        };
        assert_eq!(
            inspect(&corpus_file(path), PixelBudget::DEFAULT),
            expected,
            "{path}"
        );
    }
}

#[test]
fn over_budget_files_are_refused_from_the_header_alone() {
    for &(path, format, width, height) in CORPUS.iter().filter(|(path, ..)| is_over_budget(path)) {
        let bytes = corpus_file(path);
        // The file is a complete header and nothing more: a larger budget
        // accepts it, and without its last byte it is cut short.
        assert_eq!(
            inspect(&bytes, unlimited()),
            Ok(header(format, width, height)),
            "{path}"
        );
        assert_eq!(
            inspect(&bytes[..bytes.len() - 1], unlimited()),
            Err(ImageError::Truncated { format }),
            "{path}"
        );
    }
}

#[test]
fn every_prefix_is_truncated_until_the_header_is_complete() {
    for &(path, format, ..) in CORPUS {
        let bytes = corpus_file(path);
        let full = inspect(&bytes, unlimited());
        let header_end = (0..=bytes.len())
            .find(|&len| inspect(&bytes[..len], unlimited()) == full)
            .unwrap();
        for len in 0..bytes.len() {
            let prefix = &bytes[..len];
            let expected = if len >= header_end {
                full
            } else if sniff(prefix).is_none() {
                Err(ImageError::UnrecognisedFormat)
            } else {
                Err(ImageError::Truncated { format })
            };
            assert_eq!(
                inspect(prefix, unlimited()),
                expected,
                "{path}, {len} bytes"
            );
        }
    }
}

#[test]
fn damaged_corpus_headers_are_refused() {
    let corrupt = |format, defect| Err(ImageError::Corrupt { format, defect });
    let mut png = corpus_file("png/rgb.png");
    png[32] ^= 0x01;
    assert_eq!(
        inspect(&png, unlimited()),
        corrupt(ImageFormat::Png, HeaderDefect::PngChecksum)
    );

    // A JPEG whose frame header comes twice, as libjpeg refuses it.
    let jpeg = corpus_file("jpeg/baseline.jpg");
    let sof = jpeg
        .windows(2)
        .position(|pair| pair == [0xFF, 0xC0])
        .unwrap();
    let end = sof + 2 + usize::from(u16::from_be_bytes([jpeg[sof + 2], jpeg[sof + 3]]));
    let twice = [&jpeg[..end], &jpeg[sof..]].concat();
    assert_eq!(
        inspect(&twice, unlimited()),
        corrupt(ImageFormat::Jpeg, HeaderDefect::JpegDuplicateFrameHeader)
    );

    // A GIF whose first block, after the screen and its colour table, has an
    // unknown type.
    let mut gif = corpus_file("gif/still.gif");
    let blocks = gif_blocks_start(&gif);
    gif[blocks] = 0x00;
    assert_eq!(
        inspect(&gif, unlimited()),
        corrupt(ImageFormat::Gif, HeaderDefect::GifBlock(0x00))
    );

    let mut webp = corpus_file("webp/lossy.webp");
    webp[23] = 0x9C;
    assert_eq!(
        inspect(&webp, unlimited()),
        corrupt(ImageFormat::WebP, HeaderDefect::Vp8StartCode)
    );
}

#[test]
fn damage_that_decoders_accept_is_read_as_they_read_it() {
    // Stray bytes after the JFIF segment of a JPEG: libjpeg skips them.
    let jpeg = corpus_file("jpeg/baseline.jpg");
    let stray = [&jpeg[..20], &[0x00, 0x7F], &jpeg[20..]].concat();
    assert_eq!(
        inspect(&stray, unlimited()),
        Ok(header(ImageFormat::Jpeg, 67, 41))
    );

    // A GIF screen that is smaller than the frame, or has a zero side: the
    // canvas grows to hold the frame.
    for screen in [[1, 0, 1, 0], [0, 0, 41, 0]] {
        let mut gif = corpus_file("gif/still.gif");
        gif[6..10].copy_from_slice(&screen);
        assert_eq!(
            inspect(&gif, unlimited()),
            Ok(header(ImageFormat::Gif, 67, 41)),
            "{screen:?}"
        );
    }
}

/// Where the blocks of a GIF start: after the 13-byte screen descriptor and
/// the global colour table that its packed field announces.
fn gif_blocks_start(gif: &[u8]) -> usize {
    let packed = gif[10];
    let table = if packed & 0x80 == 0 {
        0
    } else {
        3 << ((packed & 0x07) + 1)
    };
    13 + table
}

proptest! {
    #[test]
    fn damaged_corpus_files_never_panic(
        (path, ..) in select(CORPUS),
        damage in vec((any::<Index>(), any::<u8>()), 1..8),
        cut in any::<Index>(),
    ) {
        let mut bytes = corpus_file(path);
        for (at, byte) in damage {
            let at = at.index(bytes.len());
            bytes[at] = byte;
        }
        bytes.truncate(cut.index(bytes.len() + 1));
        if let Ok(header) = inspect(&bytes, PixelBudget::DEFAULT) {
            prop_assert_eq!(Some(header.format), sniff(&bytes));
            prop_assert!(PixelBudget::DEFAULT.admits(header.dimensions));
        }
    }
}
