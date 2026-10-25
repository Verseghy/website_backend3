//! The PNG reader: the dimensions in the `IHDR` chunk, which PNG requires to
//! be the first chunk, right after the 8-byte signature.
//!
//! An animated PNG keeps its canvas in the same `IHDR`; its frames must fit
//! inside it. The reader checks what PNG requires of the chunk: its length,
//! its CRC, the side limit of 2³¹ − 1 and a defined image type.

use super::{Dimensions, HeaderDefect, HeaderError, bytes_at, dimensions, take};

/// The largest width or height that PNG allows, 2³¹ − 1.
const MAX_SIDE: u32 = 0x7FFF_FFFF;

/// Reads the dimensions of a PNG, whose first 8 bytes are the signature.
///
/// - `bytes`: the whole input.
pub(super) fn read(bytes: &[u8]) -> Result<Dimensions, HeaderError> {
    // The chunk: length (4), type (4), data (13), CRC over type and data (4).
    let chunk = take(bytes, 8, 25)?;
    if chunk[..8] != *b"\0\0\0\x0DIHDR" {
        return Err(HeaderDefect::PngFirstChunk.into());
    }
    if chunk[21..] != crc32(&chunk[4..21]).to_be_bytes() {
        return Err(HeaderDefect::PngChecksum.into());
    }
    let width = u32::from_be_bytes(bytes_at(chunk, 8)?);
    let height = u32::from_be_bytes(bytes_at(chunk, 12)?);
    let [depth, colour, compression, filter, interlace] = bytes_at(chunk, 16)?;
    if !valid_image_type(depth, colour) || compression != 0 || filter != 0 || interlace > 1 {
        return Err(HeaderDefect::PngImageType.into());
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(HeaderDefect::DimensionTooLarge.into());
    }
    dimensions(width, height)
}

/// Whether PNG allows the bit `depth` for the `colour` type: greyscale (0)
/// 1-16, indexed (3) 1-8, and truecolour (2) and the alpha types (4, 6) 8 or 16.
fn valid_image_type(depth: u8, colour: u8) -> bool {
    match colour {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        _ => false,
    }
}

/// The CRC-32 that PNG chunks carry (the ISO 3309 polynomial, reflected),
/// computed bit by bit: the reader only ever checks the 17 bytes of `IHDR`,
/// so a lookup table would cost more than it saves.
///
/// - `bytes`: the chunk type and data.
pub(super) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::image_optimise::header::samples;
    use proptest::{collection::vec, prelude::*};

    fn dims(bytes: &[u8]) -> Result<(u32, u32), HeaderError> {
        read(bytes).map(|d| (d.width(), d.height()))
    }

    fn corrupt(defect: HeaderDefect) -> Result<(u32, u32), HeaderError> {
        Err(HeaderError::Corrupt(defect))
    }

    /// `samples::png(67, 41)` with IHDR data byte `offset` set to `byte` and the CRC redone.
    fn with_data(offset: usize, byte: u8) -> Vec<u8> {
        let mut bytes = samples::png(67, 41);
        bytes[16 + offset] = byte;
        let crc = crc32(&bytes[12..29]).to_be_bytes();
        bytes[29..].copy_from_slice(&crc);
        bytes
    }

    #[test]
    fn crc_matches_the_reference_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn reads_the_ihdr_dimensions() {
        assert_eq!(dims(&samples::png(67, 41)), Ok((67, 41)));
        assert_eq!(dims(&samples::png(MAX_SIDE, 1)), Ok((MAX_SIDE, 1)));
        // Interlaced (Adam7) images have the same header.
        assert_eq!(dims(&with_data(12, 1)), Ok((67, 41)));
    }

    #[test]
    fn exactly_the_defined_image_types_are_accepted() {
        assert!(valid_image_type(1, 0) && valid_image_type(16, 0) && valid_image_type(8, 3));
        assert!(!valid_image_type(16, 3) && !valid_image_type(4, 2) && !valid_image_type(8, 7));
        // The sample is 8-bit truecolour: a 3-bit depth or colour type 5 is undefined.
        assert_eq!(dims(&with_data(8, 3)), corrupt(HeaderDefect::PngImageType));
        assert_eq!(dims(&with_data(9, 5)), corrupt(HeaderDefect::PngImageType));
        assert_eq!(dims(&with_data(9, 3)), Ok((67, 41)));
    }

    #[test]
    fn unknown_methods_are_refused() {
        assert_eq!(dims(&with_data(10, 1)), corrupt(HeaderDefect::PngImageType));
        assert_eq!(dims(&with_data(11, 1)), corrupt(HeaderDefect::PngImageType));
        assert_eq!(dims(&with_data(12, 2)), corrupt(HeaderDefect::PngImageType));
    }

    #[test]
    fn a_first_chunk_other_than_ihdr_is_refused() {
        let mut idat = samples::png(67, 41);
        idat[12..16].copy_from_slice(b"IDAT");
        assert_eq!(dims(&idat), corrupt(HeaderDefect::PngFirstChunk));
        let mut long = samples::png(67, 41);
        long[11] = 14;
        assert_eq!(dims(&long), corrupt(HeaderDefect::PngFirstChunk));
    }

    #[test]
    fn a_wrong_checksum_is_refused() {
        let mut bytes = samples::png(67, 41);
        bytes[32] ^= 1;
        assert_eq!(dims(&bytes), corrupt(HeaderDefect::PngChecksum));
    }

    #[test]
    fn sides_outside_the_png_range_are_refused() {
        assert_eq!(
            dims(&samples::png(MAX_SIDE + 1, 1)),
            corrupt(HeaderDefect::DimensionTooLarge)
        );
        assert_eq!(
            dims(&samples::png(1, u32::MAX)),
            corrupt(HeaderDefect::DimensionTooLarge)
        );
        assert_eq!(
            dims(&samples::png(0, 1)),
            corrupt(HeaderDefect::ZeroDimension)
        );
    }

    proptest! {
        #[test]
        fn any_single_bit_flip_in_the_chunk_is_caught(at in 8..33_usize, bit in 0..8_u32) {
            let mut bytes = samples::png(67, 41);
            bytes[at] ^= 1 << bit;
            prop_assert!(matches!(dims(&bytes), Err(HeaderError::Corrupt(_))));
        }

        #[test]
        fn crc_detects_every_single_bit_error(
            bytes in vec(any::<u8>(), 1..64),
            at in any::<prop::sample::Index>(),
            bit in 0..8_u32,
        ) {
            let mut flipped = bytes.clone();
            flipped[at.index(bytes.len())] ^= 1 << bit;
            prop_assert_ne!(crc32(&bytes), crc32(&flipped));
        }

        #[test]
        fn the_image_type_check_matches_its_table(depth in any::<u8>(), colour in any::<u8>()) {
            let expected = match colour {
                0 => depth.is_power_of_two() && depth <= 16,
                3 => depth.is_power_of_two() && depth <= 8,
                2 | 4 | 6 => depth == 8 || depth == 16,
                _ => false,
            };
            prop_assert_eq!(valid_image_type(depth, colour), expected);
        }
    }
}
