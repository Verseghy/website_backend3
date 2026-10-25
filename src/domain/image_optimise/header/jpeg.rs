//! The JPEG reader: the dimensions in the frame header (`SOFn`), found the
//! way libjpeg finds it.
//!
//! A JPEG is a sequence of marker segments (ITU-T T.81, Annex B). The frame
//! header may follow any number of metadata (`APPn`, COM) and table (DQT, DHT,
//! DAC, DRI) segments, and more tables may follow it before the first scan
//! (SOS). libjpeg is the reference decoder here: `MozJPEG`, the class of
//! encoder that ADR 0015 §7 names, is built on libjpeg-turbo and shares its
//! marker reader (`read_markers` and `next_marker` in
//! `libjpeg-turbo@3695679:src/jdmarker.c`). So the reader walks the markers
//! up to the first scan as that code does,
//! stepping over each segment by its length without looking inside it, and
//! the frame header it budgets is the one libjpeg allocates for:
//!
//! - Between segments, bytes other than `FF`, `FF` fill bytes and stuffed
//!   `FF 00` pairs are skipped, as libjpeg skips them (with a warning).
//! - `APPn`, COM and DNL segments are skipped by their length; a length
//!   below 2 skips only the length field, as libjpeg's `skip_variable` does.
//! - A second start of image, a second frame header, a scan before the frame
//!   header, the end of the image before the first scan, and the markers that
//!   libjpeg does not know are refused, as libjpeg refuses them. A second
//!   frame header matters most: a decoder that took the later one would
//!   allocate for a size that was never budgeted.
//!
//! Any of the thirteen frame header types is read; whether the decoder
//! supports that type (lossless, hierarchical, arithmetic coding) is up to
//! the decoder.

use super::{Dimensions, HeaderDefect, HeaderError, bytes_at, dimensions, take};

/// Reads the dimensions of a JPEG, whose first two bytes are the start of
/// image marker `FF D8`.
///
/// - `bytes`: the whole input.
pub(super) fn read(bytes: &[u8]) -> Result<Dimensions, HeaderError> {
    let mut at = 2;
    let mut frame = None;
    loop {
        let (code, next) = next_marker(bytes, at)?;
        at = next;
        match code {
            // RSTm and TEM stand alone, without a length.
            0xD0..=0xD7 | 0x01 => {}
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                if frame.is_some() {
                    return Err(HeaderDefect::JpegDuplicateFrameHeader.into());
                }
                let (dimensions, length) = frame_header(bytes, at)?;
                frame = Some(dimensions);
                at += length;
            }
            // The first scan: the frame header before it describes the image.
            0xDA => return frame.ok_or(HeaderError::Corrupt(HeaderDefect::JpegNoFrameHeader)),
            0xD9 if frame.is_some() => return Err(HeaderDefect::JpegNoScan.into()),
            0xD9 => return Err(HeaderDefect::JpegNoFrameHeader.into()),
            // DHT, DAC, DQT and DRI: libjpeg parses them and refuses a length
            // that cannot hold their content.
            0xC4 | 0xCC | 0xDB | 0xDD => at += segment_length(bytes, at)?,
            // APPn, COM and DNL: libjpeg skips them by their length, and for a
            // length below 2 it skips the length field and nothing more.
            0xE0..=0xEF | 0xFE | 0xDC => {
                at += usize::from(u16::from_be_bytes(bytes_at(bytes, at)?)).max(2);
            }
            other => return Err(HeaderDefect::JpegUnexpectedMarker(other).into()),
        }
    }
}

/// The code of the next marker at or after offset `at`, and the offset just
/// past it, as libjpeg's `next_marker` finds them: bytes before an `FF` are
/// skipped, any number of `FF` fill bytes may come before the code (T.81
/// B.1.1.2), and a stuffed `FF 00` pair is data, so it is skipped too.
fn next_marker(bytes: &[u8], mut at: usize) -> Result<(u8, usize), HeaderError> {
    loop {
        let rest = bytes.get(at..).unwrap_or_default();
        at += rest
            .iter()
            .position(|&byte| byte == 0xFF)
            .ok_or(HeaderError::Truncated)?;
        let mut code = 0xFF;
        while code == 0xFF {
            at += 1;
            [code] = bytes_at(bytes, at)?;
        }
        at += 1;
        if code != 0x00 {
            return Ok((code, at));
        }
    }
}

/// The length of the table or frame header segment whose length field starts
/// at `at`. The length counts the two bytes of the field itself, so libjpeg
/// refuses a length below 2 for these segments.
fn segment_length(bytes: &[u8], at: usize) -> Result<usize, HeaderError> {
    let length = usize::from(u16::from_be_bytes(bytes_at(bytes, at)?));
    if length < 2 {
        return Err(HeaderDefect::JpegSegmentLength.into());
    }
    Ok(length)
}

/// The dimensions in the frame header whose length field starts at `at`,
/// and the length of that segment.
///
/// The frame header is the length (2 bytes), the sample precision (1), the
/// height (2), the width (2), the component count (1) and 3 bytes per
/// component. It must be present in full, as libjpeg's `get_sof` requires.
fn frame_header(bytes: &[u8], at: usize) -> Result<(Dimensions, usize), HeaderError> {
    let length = segment_length(bytes, at)?;
    let [_, _, _precision, h0, h1, w0, w1, components] = bytes_at(bytes, at)?;
    if components == 0 || length != 8 + 3 * usize::from(components) {
        return Err(HeaderDefect::JpegFrameHeader.into());
    }
    take(bytes, at, length)?;
    let dimensions = dimensions(
        u16::from_be_bytes([w0, w1]).into(),
        u16::from_be_bytes([h0, h1]).into(),
    )?;
    Ok((dimensions, length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::image_optimise::header::samples;
    use proptest::{collection::vec, prelude::*, sample::select};

    /// The bytes at the end of [`samples::jpeg`]: its SOF0 segment (19 bytes)
    /// and the SOS marker (2).
    const FRAME_AND_SCAN: usize = 21;

    fn dims(bytes: &[u8]) -> Result<(u32, u32), HeaderError> {
        read(bytes).map(|d| (d.width(), d.height()))
    }

    fn corrupt(defect: HeaderDefect) -> Result<(u32, u32), HeaderError> {
        Err(HeaderError::Corrupt(defect))
    }

    /// `samples::jpeg(67, 41)` with `before` inserted before its frame header
    /// and `after` between the frame header and the scan.
    fn around_frame(before: &[u8], after: &[u8]) -> Vec<u8> {
        let sample = samples::jpeg(67, 41);
        let (head, tail) = sample.split_at(sample.len() - FRAME_AND_SCAN);
        let (frame, scan) = tail.split_at(FRAME_AND_SCAN - 2);
        [head, before, frame, after, scan].concat()
    }

    /// `samples::jpeg(67, 41)` with `extra` inserted before its frame header.
    fn before_frame(extra: &[u8]) -> Vec<u8> {
        around_frame(extra, &[])
    }

    /// `samples::jpeg(67, 41)` with the frame header's byte `offset` set to `byte`.
    fn frame_with(offset: usize, byte: u8) -> Vec<u8> {
        let mut bytes = samples::jpeg(67, 41);
        let at = bytes.len() - FRAME_AND_SCAN + offset;
        bytes[at] = byte;
        bytes
    }

    #[test]
    fn reads_the_frame_header_after_metadata_and_tables() {
        assert_eq!(dims(&samples::jpeg(67, 41)), Ok((67, 41)));
        assert_eq!(dims(&samples::jpeg(u16::MAX, 1)), Ok((65_535, 1)));
        // An EXIF segment whose payload holds FF bytes, a comment, a DHT and a DRI.
        let segments = [
            &b"\xFF\xE1\0\x0AExif\xFF\xD9\xFF\xC0"[..],
            b"\xFF\xFE\0\x04hi",
            b"\xFF\xC4\0\x02",
            b"\xFF\xDD\0\x04\0\x10",
        ]
        .concat();
        assert_eq!(dims(&around_frame(&segments, &segments)), Ok((67, 41)));
    }

    #[test]
    fn every_frame_type_is_read() {
        for code in [
            0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
        ] {
            assert_eq!(dims(&frame_with(1, code)), Ok((67, 41)), "{code:02X}");
        }
    }

    #[test]
    fn a_one_component_frame_is_read() {
        let bytes = [
            &b"\xFF\xD8\xFF\xC0\0\x0B\x08\0\x29\0\x43\x01\x01\x11\0"[..],
            b"\xFF\xDA",
        ]
        .concat();
        assert_eq!(dims(&bytes), Ok((67, 41)));
    }

    #[test]
    fn fill_bytes_and_standalone_markers_are_skipped() {
        assert_eq!(
            dims(&before_frame(b"\xFF\xFF\xFF\xD0\xFF\x01\xFF\xD7\xFF")),
            Ok((67, 41))
        );
    }

    #[test]
    fn stray_bytes_and_stuffed_zeros_between_segments_are_skipped() {
        // libjpeg skips them with a warning (JWRN_EXTRANEOUS_DATA).
        let stray = b"\x00\x12\xFF\x00\x7F\xFF\xFF\x00";
        assert_eq!(dims(&around_frame(stray, stray)), Ok((67, 41)));
        assert_eq!(
            dims(b"\xFF\xD8\x00\xFF\xC0\0\x0B\x08\0\x01\0\x02\x01\x01\x11\0\x00\xFF\xDA"),
            Ok((2, 1))
        );
    }

    #[test]
    fn short_metadata_lengths_skip_only_the_length_field() {
        // libjpeg's skip_variable skips nothing for a length below 2, so the
        // next marker starts right after the length field.
        let bogus = b"\xFF\xE0\0\x01\xFF\xFE\0\0\xFF\xDC\0\x01";
        assert_eq!(dims(&around_frame(bogus, bogus)), Ok((67, 41)));
    }

    #[test]
    fn a_dnl_segment_before_the_scan_is_skipped() {
        let dnl = b"\xFF\xDC\0\x04\x01\x00";
        assert_eq!(dims(&around_frame(dnl, dnl)), Ok((67, 41)));
    }

    #[test]
    fn unexpected_markers_are_refused() {
        for code in [0x02, 0xBF, 0xC8, 0xD8, 0xDE, 0xDF, 0xF0, 0xF7, 0xFD] {
            for bytes in [
                before_frame(&[0xFF, code, 0, 2]),
                around_frame(&[], &[0xFF, code, 0, 2]),
            ] {
                assert_eq!(
                    dims(&bytes),
                    corrupt(HeaderDefect::JpegUnexpectedMarker(code)),
                    "{code:02X}"
                );
            }
        }
    }

    #[test]
    fn a_second_frame_header_is_refused() {
        // A small frame, then a huge one that a lenient decoder might use.
        let second = samples::jpeg_frame(u16::MAX, u16::MAX);
        assert_eq!(
            dims(&around_frame(&[], &second)),
            corrupt(HeaderDefect::JpegDuplicateFrameHeader)
        );
        let progressive = [&[0xFF, 0xC2][..], &second[2..]].concat();
        assert_eq!(
            dims(&around_frame(&[], &progressive)),
            corrupt(HeaderDefect::JpegDuplicateFrameHeader)
        );
    }

    #[test]
    fn a_scan_or_the_end_before_any_frame_is_refused() {
        assert_eq!(
            dims(&before_frame(b"\xFF\xDA\0\x02")),
            corrupt(HeaderDefect::JpegNoFrameHeader)
        );
        assert_eq!(
            dims(b"\xFF\xD8\xFF\xD9"),
            corrupt(HeaderDefect::JpegNoFrameHeader)
        );
    }

    #[test]
    fn the_end_before_the_first_scan_is_refused() {
        let mut bytes = samples::jpeg(67, 41);
        *bytes.last_mut().unwrap() = 0xD9;
        assert_eq!(dims(&bytes), corrupt(HeaderDefect::JpegNoScan));
    }

    #[test]
    fn short_table_and_frame_lengths_are_refused() {
        for table in [0xC4, 0xCC, 0xDB, 0xDD] {
            for bytes in [
                before_frame(&[0xFF, table, 0, 1]),
                around_frame(&[], &[0xFF, table, 0, 0]),
            ] {
                assert_eq!(
                    dims(&bytes),
                    corrupt(HeaderDefect::JpegSegmentLength),
                    "{table:02X}"
                );
            }
        }
        assert_eq!(
            dims(&frame_with(3, 1)),
            corrupt(HeaderDefect::JpegSegmentLength)
        );
    }

    #[test]
    fn malformed_frame_headers_are_refused() {
        // No components with a matching length of 8; no components with the
        // length of three; lengths that do not match three components.
        assert_eq!(
            dims(b"\xFF\xD8\xFF\xC0\0\x08\x08\0\x29\0\x43\0"),
            corrupt(HeaderDefect::JpegFrameHeader)
        );
        assert_eq!(
            dims(&frame_with(9, 0)),
            corrupt(HeaderDefect::JpegFrameHeader)
        );
        assert_eq!(
            dims(&frame_with(3, 18)),
            corrupt(HeaderDefect::JpegFrameHeader)
        );
        assert_eq!(
            dims(&frame_with(3, 7)),
            corrupt(HeaderDefect::JpegFrameHeader)
        );
    }

    #[test]
    fn a_zero_side_is_refused() {
        // A zero height means a DNL marker after the first scan.
        assert_eq!(
            dims(&samples::jpeg(67, 0)),
            corrupt(HeaderDefect::ZeroDimension)
        );
        assert_eq!(
            dims(&samples::jpeg(0, 41)),
            corrupt(HeaderDefect::ZeroDimension)
        );
    }

    /// What may come between two markers: a metadata or table segment with
    /// any payload (`FF` bytes included) after up to two fill bytes, stray
    /// bytes other than `FF`, or a stuffed `FF 00` pair.
    fn any_segment() -> impl Strategy<Value = Vec<u8>> {
        let code = select(
            [
                0xC4_u8, 0xCC, 0xDB, 0xDC, 0xDD, 0xE0, 0xE1, 0xE2, 0xED, 0xEE, 0xEF, 0xFE,
            ]
            .to_vec(),
        );
        let segment =
            (code, vec(any::<u8>(), 0..300), 0..3_usize).prop_map(|(code, payload, fill)| {
                let length = u16::try_from(payload.len() + 2).unwrap().to_be_bytes();
                [vec![0xFF; fill + 1], vec![code], length.to_vec(), payload].concat()
            });
        prop_oneof![
            4 => segment,
            1 => vec(0..0xFF_u8, 1..8),
            1 => Just(vec![0xFF, 0x00]),
        ]
    }

    proptest! {
        #[test]
        fn segments_and_stray_bytes_never_change_the_dimensions(
            width in 1..=u16::MAX,
            height in 1..=u16::MAX,
            before in vec(any_segment(), 0..6),
            after in vec(any_segment(), 0..4),
        ) {
            let sample = samples::jpeg(width, height);
            let (head, tail) = sample.split_at(sample.len() - FRAME_AND_SCAN);
            let (frame, scan) = tail.split_at(FRAME_AND_SCAN - 2);
            let bytes = [head, &before.concat(), frame, &after.concat(), scan].concat();
            prop_assert_eq!(dims(&bytes), Ok((width.into(), height.into())));
        }

        #[test]
        fn any_bytes_after_the_start_of_image_never_panic(tail in vec(any::<u8>(), 0..200)) {
            let bytes = [&[0xFF, 0xD8][..], &tail].concat();
            if let Ok((width, height)) = dims(&bytes) {
                prop_assert!(width >= 1 && height >= 1);
            }
        }
    }
}
