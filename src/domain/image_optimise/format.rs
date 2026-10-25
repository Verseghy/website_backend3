//! Magic-byte sniffing of the four image formats that uploads accept
//! (ADR 0015 §6).
//!
//! The format comes from the content, never from the file name: ADR 0015 §6
//! lets an upload in only when the sniffed format agrees with its extension,
//! and that check needs a sniffer that a renamed file cannot fool.

use core::fmt;

/// An image format that backend3 accepts for upload (ADR 0015 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// JPEG (JFIF, EXIF and plain JPEG streams alike).
    Jpeg,
    /// PNG, including animated PNG.
    Png,
    /// GIF, versions 87a and 89a.
    Gif,
    /// WebP: lossy, lossless or extended.
    WebP,
}

impl ImageFormat {
    /// Every accepted format.
    pub const ALL: [Self; 4] = [Self::Jpeg, Self::Png, Self::Gif, Self::WebP];

    /// The usual name of the format, for messages and logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Gif => "GIF",
            Self::WebP => "WebP",
        }
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The PNG signature, which every PNG file starts with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Names the format of `bytes` from their signature, or returns `None` when
/// they are none of the four formats.
///
/// - `bytes`: the start of the upload. At most the first 12 bytes are read.
///
/// The signatures are: JPEG `FF D8 FF` (the start-of-image marker and the
/// first byte of the next marker), the 8-byte PNG signature, `GIF87a` or
/// `GIF89a`, and a RIFF header of form `WEBP` (`RIFF`, four size bytes,
/// `WEBP`). A part of a signature is not enough.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(&PNG_SIGNATURE) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*, sample::select};

    /// One full signature per format; the WebP size bytes may be anything.
    fn signature(format: ImageFormat) -> Vec<u8> {
        match format {
            ImageFormat::Jpeg => vec![0xFF, 0xD8, 0xFF],
            ImageFormat::Png => PNG_SIGNATURE.to_vec(),
            ImageFormat::Gif => b"GIF89a".to_vec(),
            ImageFormat::WebP => b"RIFF\xFF\0\0\0WEBP".to_vec(),
        }
    }

    #[test]
    fn names() {
        let names: Vec<String> = ImageFormat::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(names, ["JPEG", "PNG", "GIF", "WebP"]);
    }

    #[test]
    fn signatures_are_recognised() {
        for format in ImageFormat::ALL {
            assert_eq!(sniff(&signature(format)), Some(format), "{format}");
        }
        assert_eq!(sniff(b"GIF87a"), Some(ImageFormat::Gif));
    }

    #[test]
    fn near_misses_are_not() {
        for bytes in [
            &b"\xFF\xD8"[..],
            b"\xFF\xD8\x00",
            b"\x89PNG\r\n\x1A",
            b"\x89PNG\r\n\x1A\x00",
            b"GIF88a",
            b"gif89a",
            b"RIFF\0\0\0\0WAVE",
            b"RIFF\0\0\0\0WEB",
            b"<svg",
            b"",
        ] {
            assert_eq!(sniff(bytes), None, "{bytes:?}");
        }
    }

    proptest! {
        #[test]
        fn a_signature_decides_whatever_follows(
            format in select(ImageFormat::ALL.to_vec()),
            tail in vec(any::<u8>(), 0..32),
        ) {
            let bytes = [signature(format), tail].concat();
            prop_assert_eq!(sniff(&bytes), Some(format));
        }

        #[test]
        fn every_signature_byte_counts(
            format in select(ImageFormat::ALL.to_vec()),
            at in 0..12_usize,
            byte in any::<u8>(),
        ) {
            let mut bytes = signature(format);
            // The four WebP size bytes are not part of the signature.
            prop_assume!(at < bytes.len() && bytes[at] != byte);
            prop_assume!(!(format == ImageFormat::WebP && (4..8).contains(&at)));
            bytes[at] = byte;
            let still_gif = format == ImageFormat::Gif && at == 4 && byte == b'7';
            prop_assert_eq!(sniff(&bytes), still_gif.then_some(ImageFormat::Gif));
        }
    }
}
