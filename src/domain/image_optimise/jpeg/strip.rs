//! The lossless metadata strip of a JPEG (ADR 0015 §7).
//!
//! The strip rewrites the marker structure and copies the compressed image
//! data byte for byte, so the pixels a decoder produces do not change. It
//! walks every segment up to the end of image (EOI), with the marker reader
//! of the header check, and keeps:
//!
//! - the tables, the frame header and every scan with its entropy-coded
//!   data, unchanged;
//! - JFIF `APP0` segments, without their embedded thumbnail;
//! - the EXIF `APP1` segment that Chromium reads, reduced to its
//!   Orientation tag ([`Orientation::tiff_replacing`]), or nothing when it
//!   has none. Chromium takes the first `APP1` whose payload starts with
//!   `Exif\0` and holds more than the 6-byte identifier (whatever its sixth
//!   byte), and no later one (`skia@62095415:src/codec/SkJpegMetadataDecoderImpl.cpp`,
//!   `read_metadata`). ADR 0033 §3 follows that reference reading, so an
//!   orientation in a second EXIF segment is dropped with the rest of it,
//!   as the browser ignores it too;
//! - ICC profile `APP2` segments, unchanged;
//! - Adobe `APP14` segments, cut to the 12 bytes that libjpeg reads
//!   (`examine_app14`, `APP14_DATA_LEN`): their colour transform flag tells
//!   the decoder how to read the components (CMYK, YCCK or RGB), so those
//!   bytes are coding parameters rather than metadata. Anything a writer
//!   puts after them is not read by any decoder and goes.
//!
//! Everything else goes: the rest of EXIF (GPS, serial numbers, dates, the
//! thumbnail), XMP and extended XMP (`APP1`), IPTC and Photoshop data
//! (`APP13`), comments (COM), JFXX thumbnails (`APP0`), the multi-picture
//! index (`APP2` MPF) with the extra images that follow the end of image,
//! every other `APPn` segment, DNL segments (which libjpeg ignores), and fill
//! and stray bytes between segments. `APPn` and COM segments after the first
//! scan are dropped too, ICC included: a decoder reads the colour profile
//! with the header, before the first scan, so moving such a segment forward
//! would change the colours rather than keep them.
//!
//! The marker rules follow libjpeg (`read_markers` in
//! `libjpeg-turbo@3695679:src/jdmarker.c`), like the header check: an
//! `APPn`, COM or DNL length below 2 skips only the length field, and a
//! second start of image or frame header, a scan before the frame header,
//! the end of image before the first scan, and the markers libjpeg does not
//! know are refused.
//!
//! **The scan cap** (ADR 0033 §4). The strip counts the scans (SOS
//! segments) as it walks them, and refuses an image with more than the cap
//! it is given before anything is decoded. A progressive JPEG can repeat
//! scans that hold only "EOB runs", a few bytes each, and libjpeg processes
//! every block of the image for each of them, so a small crafted upload
//! could otherwise hold the one-at-a-time optimiser for minutes. The strip
//! is the right place: it already walks every scan to find its end, so the
//! count costs nothing, and it runs before the decode check.

use super::super::{
    DecodeDefect,
    exif::Orientation,
    header::{self, HeaderError, jpeg::next_marker},
};

/// Why the strip refuses an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The segments are malformed or cut short.
    Defect(DecodeDefect),
    /// The image has more scans than the cap the strip was given.
    TooManyScans,
}

impl From<DecodeDefect> for Refusal {
    fn from(defect: DecodeDefect) -> Self {
        Self::Defect(defect)
    }
}

/// The marker codes the strip handles by name.
const SOI: u8 = 0xD8;
/// End of image.
const EOI: u8 = 0xD9;
/// Start of scan.
const SOS: u8 = 0xDA;
/// Define number of lines.
const DNL: u8 = 0xDC;
/// Temporary private use: a marker without a length.
const TEM: u8 = 0x01;
/// `APP0`, which JFIF and JFXX use.
const APP0: u8 = 0xE0;
/// `APP1`, which EXIF and XMP use.
const APP1: u8 = 0xE1;
/// `APP2`, which ICC profiles and MPF use.
const APP2: u8 = 0xE2;
/// `APP14`, which the Adobe segment uses.
const APP14: u8 = 0xEE;
/// Comment.
const COM: u8 = 0xFE;

/// The identifier of a JFIF `APP0` segment.
const JFIF: &[u8] = b"JFIF\0";
/// A JFIF payload without its thumbnail: identifier (5), version (2),
/// units (1), densities (2 + 2) and the thumbnail size (1 + 1). libjpeg
/// ignores a shorter one.
const JFIF_LEN: usize = 14;
/// The identifier of an EXIF `APP1` segment, as the strip writes it.
const EXIF: &[u8] = b"Exif\0\0";
/// What an `APP1` payload must start with for Chromium to read it as EXIF:
/// the identifier without its padding byte, which Skia does not check.
const EXIF_SIGNATURE: &[u8] = b"Exif\0";
/// The identifier of an ICC profile `APP2` segment.
const ICC_PROFILE: &[u8] = b"ICC_PROFILE\0";
/// The identifier of an Adobe `APP14` segment.
const ADOBE: &[u8] = b"Adobe";
/// The Adobe payload that libjpeg reads: identifier (5), version (2), the
/// two flag words (2 + 2) and the colour transform (1). libjpeg ignores a
/// shorter one and skips whatever follows these bytes in a longer one.
const ADOBE_LEN: usize = 12;

/// A JPEG after the metadata strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stripped {
    /// The stripped JPEG, from SOI to EOI.
    pub(super) bytes: Vec<u8>,
    /// The kept JFIF, EXIF and ICC segments, with their markers, in their
    /// order: what a re-encode of the image carries over. The Adobe segment
    /// is not among them, because it describes this coding of the pixels and
    /// not the next one.
    pub(super) metadata: Vec<u8>,
}

/// Strips the metadata of a JPEG.
///
/// - `bytes`: the whole upload, starting with SOI.
/// - `max_scans`: the most scans (SOS segments) the image may have; the
///   optimiser passes [`super::MAX_JPEG_SCANS`]. A JPEG has at least one
///   scan, so 0 refuses every image.
///
/// # Errors
///
/// [`Refusal::TooManyScans`] as soon as the scan after the `max_scans`th
/// one starts, before its data is read; and [`Refusal::Defect`] with
/// [`DecodeDefect::JpegCutShort`] when the input ends inside a segment or
/// the scan data, or before EOI; [`DecodeDefect::JpegMarker`] for a marker
/// in a place libjpeg refuses (including input that does not start with
/// SOI); [`DecodeDefect::JpegSegmentLength`] for a table, frame or scan
/// header whose length is below 2.
pub(super) fn strip(bytes: &[u8], max_scans: usize) -> Result<Stripped, Refusal> {
    let (first, mut at) = marker(bytes, 0)?;
    if first != SOI || at != 2 {
        return Err(DecodeDefect::JpegMarker(first).into());
    }
    let mut kept = Kept {
        bytes: Vec::with_capacity(bytes.len()),
        metadata: Vec::new(),
        exif_seen: false,
    };
    kept.bytes.extend([0xFF, SOI]);
    let mut framed = false;
    let mut scanned = false;
    let mut scans = 0_usize;
    loop {
        let (code, next) = marker(bytes, at)?;
        at = next;
        match code {
            EOI if scanned => break,
            // RSTn and TEM stand alone; outside the scan data they mean nothing.
            0xD0..=0xD7 | TEM => {}
            SOS if framed => {
                // Counted before the scan's data is walked, so a refused
                // image costs no more than the scans the cap allows.
                scans += 1;
                if scans > max_scans {
                    return Err(Refusal::TooManyScans);
                }
                let header = fixed(bytes, at)?;
                let end = scan_end(bytes, at + header.len())?;
                kept.segment(SOS, &bytes[at..end]);
                at = end;
                scanned = true;
            }
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF if !framed => {
                let segment = fixed(bytes, at)?;
                kept.segment(code, segment);
                framed = true;
                at += segment.len();
            }
            // DHT, DAC, DQT and DRI.
            0xC4 | 0xCC | 0xDB | 0xDD => {
                let segment = fixed(bytes, at)?;
                kept.segment(code, segment);
                at += segment.len();
            }
            0xE0..=0xEF | COM | DNL => {
                let (payload, next) = variable(bytes, at)?;
                if !scanned {
                    kept.metadata_segment(code, payload);
                }
                at = next;
            }
            other => return Err(DecodeDefect::JpegMarker(other).into()),
        }
    }
    kept.bytes.extend([0xFF, EOI]);
    Ok(Stripped {
        bytes: kept.bytes,
        metadata: kept.metadata,
    })
}

/// The output of the strip as it is built.
struct Kept {
    /// The stripped JPEG so far.
    bytes: Vec<u8>,
    /// The kept JFIF, EXIF and ICC segments so far.
    metadata: Vec<u8>,
    /// Whether the EXIF segment that Chromium reads came already; later
    /// ones are dropped unread, as Chromium ignores them (module
    /// documentation).
    exif_seen: bool,
}

impl Kept {
    /// Appends a segment whose bytes from its length field on are `rest`.
    fn segment(&mut self, code: u8, rest: &[u8]) {
        self.bytes.extend([0xFF, code]);
        self.bytes.extend(rest);
    }

    /// Appends a segment made of the `payload` parts, with its marker and
    /// length field, to the stripped JPEG, and returns where it starts.
    fn rebuilt(&mut self, code: u8, payload: &[&[u8]]) -> usize {
        let start = self.bytes.len();
        let length = 2 + payload.iter().map(|part| part.len()).sum::<usize>();
        // Every payload here is a payload of the input or shorter, so its
        // length fits the two bytes of the length field.
        let length = u16::try_from(length).unwrap_or(u16::MAX);
        self.bytes.extend([0xFF, code]);
        self.bytes.extend(length.to_be_bytes());
        for part in payload {
            self.bytes.extend(*part);
        }
        start
    }

    /// Appends a metadata segment with `payload` to both the stripped JPEG
    /// and the metadata a re-encode carries over.
    fn metadata(&mut self, code: u8, payload: &[&[u8]]) {
        let start = self.rebuilt(code, payload);
        self.metadata.extend_from_slice(&self.bytes[start..]);
    }

    /// Applies the metadata policy of the module documentation to an
    /// `APPn`, COM or DNL segment before the first scan.
    fn metadata_segment(&mut self, code: u8, payload: &[u8]) {
        match code {
            APP0 if payload.starts_with(JFIF) && payload.len() >= JFIF_LEN => {
                // The thumbnail size becomes 0x0 and its pixels are dropped.
                self.metadata(APP0, &[&payload[..JFIF_LEN - 2], &[0, 0]]);
            }
            // Skia's test: the signature, and more than the 6-byte
            // identifier, so a segment with no TIFF data does not count and
            // a later one is read instead.
            APP1 if payload.starts_with(EXIF_SIGNATURE)
                && payload.len() > EXIF.len()
                && !self.exif_seen =>
            {
                self.exif_seen = true;
                let tiff = &payload[EXIF.len()..];
                if let Some(orientation) = Orientation::from_tiff(tiff) {
                    // The original's identifier is 6 bytes too, so the new
                    // segment is never longer than the old one.
                    self.metadata(APP1, &[EXIF, &orientation.tiff_replacing(tiff.len())]);
                }
            }
            APP2 if payload.starts_with(ICC_PROFILE) => self.metadata(APP2, &[payload]),
            APP14 if payload.starts_with(ADOBE) && payload.len() >= ADOBE_LEN => {
                // In the stripped JPEG only (see `Stripped::metadata`), and
                // only the bytes libjpeg reads, as the JFIF arm does.
                self.rebuilt(APP14, &[&payload[..ADOBE_LEN]]);
            }
            _ => {}
        }
    }
}

/// The next marker at or after `at`, with [`next_marker`] of the header
/// check, and the offset just past it.
fn marker(bytes: &[u8], at: usize) -> Result<(u8, usize), DecodeDefect> {
    next_marker(bytes, at).map_err(cut_short)
}

/// The defect for a read past the end of the input: the header helpers fail
/// only with [`HeaderError::Truncated`] there.
fn cut_short(_: HeaderError) -> DecodeDefect {
    DecodeDefect::JpegCutShort
}

/// The bytes of a table, frame header or scan header segment from its length
/// field at `at` to its end. libjpeg refuses a length below 2 for these.
fn fixed(bytes: &[u8], at: usize) -> Result<&[u8], DecodeDefect> {
    let length = usize::from(u16::from_be_bytes(
        header::bytes_at(bytes, at).map_err(cut_short)?,
    ));
    if length < 2 {
        return Err(DecodeDefect::JpegSegmentLength);
    }
    header::take(bytes, at, length).map_err(cut_short)
}

/// The payload of an `APPn`, COM or DNL segment whose length field starts at
/// `at`, and the offset after the segment. For a length below 2, libjpeg's
/// `skip_variable` skips only the length field, so the payload is empty.
fn variable(bytes: &[u8], at: usize) -> Result<(&[u8], usize), DecodeDefect> {
    let length = usize::from(u16::from_be_bytes(
        header::bytes_at(bytes, at).map_err(cut_short)?,
    ));
    if length < 2 {
        return Ok((&[], at + 2));
    }
    let payload = header::take(bytes, at + 2, length - 2).map_err(cut_short)?;
    Ok((payload, at + length))
}

/// The end of the entropy-coded data that starts at `at`: the offset of the
/// first `FF` of the marker after it. In the data, an `FF` is followed by a
/// stuffed `00` or is a restart marker `D0`-`D7`, possibly after `FF` fill
/// bytes, and the data goes on; any other code ends it. Fill bytes before
/// the ending marker are not part of the data, and libjpeg discards them.
fn scan_end(bytes: &[u8], mut at: usize) -> Result<usize, DecodeDefect> {
    loop {
        let rest = bytes.get(at..).unwrap_or_default();
        let ff = at
            + rest
                .iter()
                .position(|&byte| byte == 0xFF)
                .ok_or(DecodeDefect::JpegCutShort)?;
        let code_at = ff
            + 1
            + bytes
                .get(ff + 1..)
                .unwrap_or_default()
                .iter()
                .take_while(|&&byte| byte == 0xFF)
                .count();
        match bytes.get(code_at) {
            None => return Err(DecodeDefect::JpegCutShort),
            Some(0x00 | 0xD0..=0xD7) => at = code_at + 1,
            Some(_) => return Ok(ff),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*, sample::select};

    /// A segment with `payload`, its marker and its length field.
    fn segment(code: u8, payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len() + 2).unwrap().to_be_bytes();
        [&[0xFF, code], &length[..], payload].concat()
    }

    /// The tables, frame header and scan of a small three-component image,
    /// as an encoder writes them; the scan data holds a stuffed zero and a
    /// restart marker. These pass the strip unchanged.
    fn image() -> Vec<u8> {
        image_with(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56])
    }

    /// The image of [`image`] with `data` as the entropy-coded data of its
    /// scan.
    fn image_with(data: &[u8]) -> Vec<u8> {
        [
            segment(0xDB, &[0; 65]),
            segment(
                0xC0,
                &[8, 0, 41, 0, 67, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1],
            ),
            segment(0xC4, &[0; 17]),
            segment(0xDD, &[0, 4]),
            segment(SOS, &[3, 1, 0, 2, 0x11, 3, 0x11, 0, 0x3F, 0]),
            data.to_vec(),
        ]
        .concat()
    }

    /// SOI, `before` the image, the image, EOI and `after`.
    fn jpeg(before: &[&[u8]], after: &[u8]) -> Vec<u8> {
        [
            &[0xFF, SOI][..],
            &before.concat(),
            &image(),
            &[0xFF, EOI],
            after,
        ]
        .concat()
    }

    /// The minimal EXIF segment with orientation `value`.
    fn exif(value: u16) -> Vec<u8> {
        let tiff = Orientation::new(value).unwrap().to_tiff();
        segment(APP1, &[EXIF, &tiff].concat())
    }

    /// An EXIF segment as a camera writes it: make, orientation 6, a GPS
    /// pointer and a GPS IFD with an invented position, then a thumbnail.
    fn camera_exif() -> Vec<u8> {
        let mut tiff = b"MM\0\x2A\0\0\0\x08\0\x03".to_vec();
        tiff.extend(b"\x01\x0F\0\x02\0\0\0\x04Cam\0");
        tiff.extend(b"\x01\x12\0\x03\0\0\0\x01\0\x06\0\0");
        tiff.extend(b"\x88\x25\0\x04\0\0\0\x01\0\0\0\x2C");
        tiff.extend([0; 4]);
        tiff.extend(b"\0\x01\0\x02\0\x02\0\0\0\x02N\0\0\0\0\0\0\0");
        tiff.extend([0xFF, 0xD8, 0xFF, 0xD9]);
        segment(APP1, &[EXIF, &tiff].concat())
    }

    fn icc(sequence: u8) -> Vec<u8> {
        segment(
            APP2,
            &[ICC_PROFILE, &[sequence, 2], b"profile data"].concat(),
        )
    }

    /// A JFIF segment at 72 dpi with a `side × side` RGB thumbnail.
    fn jfif(side: u8) -> Vec<u8> {
        let thumbnail = vec![0x7F; 3 * usize::from(side) * usize::from(side)];
        segment(
            APP0,
            &[JFIF, &[1, 2, 1, 0, 72, 0, 72, side, side], &thumbnail].concat(),
        )
    }

    fn adobe(transform: u8) -> Vec<u8> {
        segment(APP14, &[ADOBE, &[0, 100, 0, 0, 0, 0, transform]].concat())
    }

    /// The strip with no scan cap, for the tests of everything else.
    fn strip(bytes: &[u8]) -> Result<Stripped, Refusal> {
        super::strip(bytes, usize::MAX)
    }

    fn stripped(bytes: &[u8]) -> Stripped {
        strip(bytes).unwrap()
    }

    /// A progressive-style image of `scans` scans: the frame and tables of
    /// [`image`], its first scan, then `scans - 1` short scans of one byte
    /// of data each, as a crafted file repeats them.
    fn with_scans(scans: usize) -> Vec<u8> {
        let extra = [segment(SOS, &[1, 1, 0, 1, 63, 0x10]), vec![0x00]].concat();
        [
            &[0xFF, SOI][..],
            &image(),
            &extra.repeat(scans - 1),
            &[0xFF, EOI],
        ]
        .concat()
    }

    #[test]
    fn scans_up_to_the_cap_pass_and_one_more_is_refused() {
        for cap in [1, 2, 10, 100] {
            let at_cap = with_scans(cap);
            assert_eq!(
                super::strip(&at_cap, cap).map(|kept| kept.bytes),
                Ok(at_cap.clone())
            );
            assert_eq!(
                super::strip(&with_scans(cap + 1), cap),
                Err(Refusal::TooManyScans),
                "{cap}"
            );
        }
        assert_eq!(super::strip(&jpeg(&[], &[]), 0), Err(Refusal::TooManyScans));
    }

    #[test]
    fn the_cap_refuses_before_reading_the_scan_after_it() {
        // The scan after the cap is cut short: the cap, not the cut, is
        // reported, because its data is never read.
        let mut bytes = with_scans(3);
        bytes.truncate(bytes.len() - 3);
        assert_eq!(super::strip(&bytes, 2), Err(Refusal::TooManyScans));
        assert_eq!(
            super::strip(&bytes, 3),
            Err(Refusal::Defect(DecodeDefect::JpegCutShort))
        );
    }

    #[test]
    fn the_exif_segment_chromium_reads_is_the_one_kept() {
        // D78 shape 3: Chromium reads only the first EXIF segment, so an
        // orientation in a second one is dropped with it.
        let without = segment(APP1, &[EXIF, b"MM\0\x2A\0\0\0\x08\0\0\0\0\0\0"].concat());
        assert_eq!(
            stripped(&jpeg(&[&without, &exif(3)], &[])).bytes,
            jpeg(&[], &[])
        );
        // An XMP segment before it does not count, nor does an EXIF
        // segment that holds no TIFF data at all: then the next one is read.
        let xmp = segment(APP1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>");
        let empty = segment(APP1, EXIF);
        assert_eq!(
            stripped(&jpeg(&[&xmp, &empty, &exif(3)], &[])).bytes,
            jpeg(&[&exif(3)], &[])
        );
        // Skia does not check the identifier's padding byte.
        let padded = segment(
            APP1,
            &[
                b"Exif\0\xFF".as_slice(),
                &Orientation::new(5).unwrap().to_tiff(),
            ]
            .concat(),
        );
        assert_eq!(
            stripped(&jpeg(&[&padded], &[])).bytes,
            jpeg(&[&exif(5)], &[])
        );
        // "Exif" without its NUL is not EXIF.
        let unterminated = segment(
            APP1,
            &[
                b"Exif\x01\0".as_slice(),
                &Orientation::new(5).unwrap().to_tiff(),
            ]
            .concat(),
        );
        assert_eq!(stripped(&jpeg(&[&unterminated], &[])).bytes, jpeg(&[], &[]));
    }

    #[test]
    fn a_short_exif_block_gets_a_short_replacement() {
        // D78 shape 1 in a JPEG: IFD0 without the next-IFD offset keeps its
        // orientation, and the segment does not grow.
        let short_tiff = &Orientation::new(8).unwrap().to_tiff()[..22];
        let upload = jpeg(&[&segment(APP1, &[EXIF, short_tiff].concat())], &[]);
        let result = stripped(&upload);
        assert_eq!(result.bytes, upload);
        assert_eq!(result.metadata, segment(APP1, &[EXIF, short_tiff].concat()));
    }

    #[test]
    fn the_image_data_passes_unchanged() {
        let plain = jpeg(&[], &[]);
        let result = stripped(&plain);
        assert_eq!(result.bytes, plain);
        assert_eq!(result.metadata, b"");
    }

    #[test]
    fn exif_keeps_only_its_orientation() {
        let result = stripped(&jpeg(&[&camera_exif()], &[]));
        assert_eq!(result.bytes, jpeg(&[&exif(6)], &[]));
        assert_eq!(result.metadata, exif(6));
        // Only the first EXIF segment counts.
        assert_eq!(
            stripped(&jpeg(&[&exif(8), &exif(3)], &[])).bytes,
            jpeg(&[&exif(8)], &[])
        );
    }

    #[test]
    fn icc_and_jfif_are_kept_without_the_jfif_thumbnail() {
        let result = stripped(&jpeg(&[&jfif(2), &icc(1), &icc(2)], &[]));
        assert_eq!(result.metadata, [jfif(0), icc(1), icc(2)].concat());
        assert_eq!(result.bytes, jpeg(&[&jfif(0), &icc(1), &icc(2)], &[]));
        // libjpeg reads every JFIF segment, so each one stays.
        let twice = jpeg(&[&jfif(1), &jfif(0)], &[]);
        assert_eq!(stripped(&twice).bytes, jpeg(&[&jfif(0), &jfif(0)], &[]));
    }

    #[test]
    fn the_adobe_segment_stays_in_the_image_but_not_in_the_metadata() {
        let result = stripped(&jpeg(&[&adobe(2), &adobe(1)], &[]));
        assert_eq!(result.bytes, jpeg(&[&adobe(2), &adobe(1)], &[]));
        assert_eq!(result.metadata, b"");
        // A short one is not an Adobe segment for libjpeg, so it is dropped.
        let short = segment(APP14, b"Adobe\0d");
        assert_eq!(stripped(&jpeg(&[&short], &[])).bytes, jpeg(&[], &[]));
        // A longer one keeps only the 12 bytes libjpeg reads.
        let long = segment(
            APP14,
            &[ADOBE, &[0, 100, 0, 0, 0, 0, 2], b"written by an editor"].concat(),
        );
        assert_eq!(
            stripped(&jpeg(&[&long], &[])).bytes,
            jpeg(&[&adobe(2)], &[])
        );
    }

    #[test]
    fn other_metadata_is_dropped() {
        let dropped = [
            segment(APP1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>"),
            segment(APP1, b"http://ns.adobe.com/xmp/extension/\0data"),
            segment(0xED, b"Photoshop 3.0\08BIM\x04\x04"),
            segment(COM, b"taken by someone"),
            segment(APP0, b"JFXX\0\x10thumbnail"),
            segment(APP0, b"JFIF\0\x01\x02"),
            segment(APP2, b"MPF\0II*\0"),
            segment(0xEC, b"Ducky"),
            segment(0xEF, b""),
            segment(DNL, &[0, 41]),
        ];
        assert_eq!(
            stripped(&jpeg(&dropped.each_ref().map(Vec::as_slice), &[])).bytes,
            jpeg(&[], &[])
        );
    }

    #[test]
    fn metadata_after_the_first_scan_and_after_the_end_is_dropped() {
        // A progressive image: an ICC segment and a comment before its second
        // scan, then an image appended after the end, as MPF writers do.
        let first = [&[0xFF, SOI][..], &icc(1), &image()].concat();
        let second_scan = [segment(SOS, &[1, 1, 0, 1, 5, 0]), vec![0xAB]].concat();
        let bytes = [
            first.clone(),
            icc(2),
            segment(COM, b"late"),
            second_scan.clone(),
            vec![0xFF, EOI],
            b"\xFF\xD8\xFF\xE1\0\x04second image".to_vec(),
        ]
        .concat();
        let result = stripped(&bytes);
        assert_eq!(result.bytes, [first, second_scan, vec![0xFF, EOI]].concat());
        assert_eq!(result.metadata, icc(1));
    }

    #[test]
    fn fill_stray_and_standalone_bytes_between_segments_are_dropped() {
        let noise: &[u8] = b"\x00\x12\xFF\xFF\xFF\xD0\xFF\x01\xFF\x00";
        let mut bytes = jpeg(&[noise, &icc(1), noise], &[]);
        // Fill bytes before the end of image, after the scan data.
        let eoi = bytes.len() - 2;
        bytes.splice(eoi..eoi, [0xFF, 0xFF]);
        assert_eq!(stripped(&bytes).bytes, jpeg(&[&icc(1)], &[]));
    }

    #[test]
    fn fill_bytes_inside_the_scan_data_stay_with_it() {
        // T.81 B.1.1.2 allows `FF` fill bytes before any marker, a restart
        // marker inside a scan included, and libjpeg's bit reader skips them
        // and reads `FF FF 00` as one stuffed `FF`. Either way the scan goes
        // on, so these bytes are data and pass byte for byte; ending the scan
        // at their first `FF` would drop the data after them.
        for data in [
            &[0x12, 0xFF, 0xFF, 0xD0, 0x56][..],
            &[0x12, 0xFF, 0xFF, 0x00, 0x78],
            &[0x12, 0xFF, 0xFF, 0xFF, 0xD3, 0x56, 0xFF, 0xFF, 0x00, 0x78],
            // A restart marker with fill bytes just before the end of image.
            &[0x12, 0xFF, 0xFF, 0xD0],
        ] {
            let bytes = [&[0xFF, SOI][..], &image_with(data), &[0xFF, EOI]].concat();
            assert_eq!(stripped(&bytes).bytes, bytes, "{data:02X?}");
        }
    }

    #[test]
    fn short_variable_lengths_skip_only_the_length_field() {
        let bogus = b"\xFF\xE1\0\x01\xFF\xFE\0\0";
        assert_eq!(stripped(&jpeg(&[bogus], &[])).bytes, jpeg(&[], &[]));
    }

    #[test]
    fn misplaced_markers_are_refused() {
        let frame = segment(0xC0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
        let scan = [segment(SOS, &[1, 1, 0, 0, 0x3F, 0]), vec![0]].concat();
        let cases: [(Vec<u8>, u8); 6] = [
            (
                [&[0xFF, SOI][..], &scan, &frame, &[0xFF, EOI]].concat(),
                SOS,
            ),
            ([&[0xFF, SOI][..], &frame, &[0xFF, EOI]].concat(), EOI),
            (
                [&[0xFF, SOI][..], &frame, &frame, &scan, &[0xFF, EOI]].concat(),
                0xC0,
            ),
            (
                [&[0xFF, SOI][..], &frame, &scan, &frame, &[0xFF, EOI]].concat(),
                0xC0,
            ),
            (
                [&[0xFF, SOI][..], &frame, &scan, &[0xFF, SOI]].concat(),
                SOI,
            ),
            (
                [&[0xFF, SOI][..], &frame, &scan, &[0xFF, 0xF0, 0, 2]].concat(),
                0xF0,
            ),
        ];
        for (bytes, code) in cases {
            assert_eq!(
                strip(&bytes),
                Err(Refusal::Defect(DecodeDefect::JpegMarker(code))),
                "{code:02X}"
            );
        }
        assert_eq!(
            strip(b"\x00\xFF\xD8"),
            Err(Refusal::Defect(DecodeDefect::JpegMarker(SOI)))
        );
        assert_eq!(
            strip(b"\xFF\xD9"),
            Err(Refusal::Defect(DecodeDefect::JpegMarker(EOI)))
        );
    }

    #[test]
    fn short_table_lengths_are_refused() {
        for code in [0xC0, 0xC4, 0xCC, 0xDB, 0xDD] {
            let bytes = [0xFF, SOI, 0xFF, code, 0, 1];
            assert_eq!(
                strip(&bytes),
                Err(Refusal::Defect(DecodeDefect::JpegSegmentLength)),
                "{code:02X}"
            );
        }
        let frame = segment(0xC0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
        let bytes = [&[0xFF, SOI][..], &frame, &[0xFF, SOS, 0, 0]].concat();
        assert_eq!(
            strip(&bytes),
            Err(Refusal::Defect(DecodeDefect::JpegSegmentLength))
        );
    }

    #[test]
    fn every_cut_short_input_is_refused() {
        let bytes = jpeg(&[&camera_exif(), &icc(1)], &[]);
        for len in 0..bytes.len() {
            let result = strip(&bytes[..len]);
            assert!(
                matches!(
                    result,
                    Err(Refusal::Defect(
                        DecodeDefect::JpegCutShort | DecodeDefect::JpegMarker(_)
                    ))
                ),
                "{len}: {result:?}"
            );
        }
        assert_eq!(
            strip(&bytes[..bytes.len() - 1]),
            Err(Refusal::Defect(DecodeDefect::JpegCutShort))
        );
    }

    /// A metadata segment of any kind the policy names, with any payload.
    fn any_metadata() -> impl Strategy<Value = Vec<u8>> {
        let identifier = select(vec![
            (APP0, JFIF),
            (APP0, b"JFXX\0".as_slice()),
            (APP1, EXIF),
            (APP1, b"http://ns.adobe.com/xap/1.0/\0".as_slice()),
            (APP2, ICC_PROFILE),
            (APP2, b"MPF\0".as_slice()),
            (APP14, ADOBE),
            (0xED, b"Photoshop 3.0\0".as_slice()),
            (COM, b"".as_slice()),
            (0xE5, b"".as_slice()),
        ]);
        (identifier, vec(any::<u8>(), 0..64))
            .prop_map(|((code, id), payload)| segment(code, &[id, &payload].concat()))
    }

    proptest! {
        #[test]
        fn the_strip_never_grows_and_is_idempotent(
            before in vec(any_metadata(), 0..8),
            tail in vec(any::<u8>(), 0..16),
        ) {
            let parts: Vec<&[u8]> = before.iter().map(Vec::as_slice).collect();
            let bytes = jpeg(&parts, &tail);
            let once = stripped(&bytes);
            prop_assert!(once.bytes.len() <= bytes.len());
            prop_assert!(once.bytes.ends_with(&[image(), vec![0xFF, EOI]].concat()));
            let twice = stripped(&once.bytes);
            prop_assert_eq!(&twice.bytes, &once.bytes);
            prop_assert_eq!(twice.metadata, once.metadata);
        }

        #[test]
        fn kept_metadata_holds_nothing_but_jfif_orientation_and_icc(
            before in vec(any_metadata(), 0..8),
        ) {
            let parts: Vec<&[u8]> = before.iter().map(Vec::as_slice).collect();
            let result = stripped(&jpeg(&parts, &[]));
            let mut at = 0;
            while at < result.metadata.len() {
                let code = result.metadata[at + 1];
                let length = usize::from(u16::from_be_bytes([result.metadata[at + 2], result.metadata[at + 3]]));
                let payload = &result.metadata[at + 4..at + 2 + length];
                match code {
                    APP0 => prop_assert_eq!(payload.len(), JFIF_LEN),
                    // The identifier and one of the two replacement forms
                    // (26 or 22 bytes), which holds only an orientation.
                    APP1 => {
                        prop_assert!(payload.starts_with(EXIF));
                        let tiff = &payload[EXIF.len()..];
                        prop_assert!(tiff.len() == 26 || tiff.len() == 22, "{}", tiff.len());
                        let orientation = Orientation::from_tiff(tiff);
                        prop_assert!(orientation.is_some());
                        prop_assert_eq!(tiff, &orientation.unwrap().to_tiff()[..tiff.len()]);
                    }
                    APP2 => prop_assert!(payload.starts_with(ICC_PROFILE)),
                    other => prop_assert!(false, "unexpected marker {other:02X}"),
                }
                at += 2 + length;
            }
        }

        #[test]
        fn arbitrary_bytes_after_the_start_of_image_never_panic(tail in vec(any::<u8>(), 0..300)) {
            let bytes = [&[0xFF, SOI][..], &tail].concat();
            if let Ok(result) = strip(&bytes) {
                prop_assert!(result.bytes.len() <= bytes.len());
                prop_assert!(result.bytes.starts_with(&[0xFF, SOI]));
                prop_assert!(result.bytes.ends_with(&[0xFF, EOI]));
            }
        }
    }
}
