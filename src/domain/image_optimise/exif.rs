//! The EXIF Orientation tag, the one EXIF field an upload keeps
//! (ADR 0015 §7).
//!
//! EXIF is where cameras and phones write the GPS position, the camera's
//! serial number, the time, the owner's name and a thumbnail. ADR 0015 §7
//! removes all of it except Orientation, because the optimiser never turns
//! the stored pixels: the tag is what still tells a viewer how to show them.
//! So an EXIF block is not edited field by field. It is read for its
//! Orientation and replaced by the smallest TIFF structure that holds only
//! that tag ([`Orientation::tiff_replacing`]), or dropped when it has none.
//! Nothing else from the original can survive, whatever the original holds.
//!
//! EXIF data is a TIFF structure: a byte-order mark, the offset of the first
//! image file directory (IFD0) and, in that IFD, a count and 12-byte entries
//! of tag, type, count and value, then the 4-byte offset of the next IFD.
//!
//! **Read as the reference decoder reads it.** The maintainer chose to
//! follow the reference decoders where ADR 0015 §7 is silent (ADR 0033 §3,
//! decision D78), and, asked again because D78's consequence had said the
//! malformed shapes would stay dropped, kept this reading with every rule
//! below: "Keep the Chromium reading as built" (decision D100,
//! 2026-10-26). For EXIF orientation the reference is Chromium, the engine
//! of most of the site's visitors and editors, which reads it with Skia's
//! EXIF parser (`skia@62095415:rust/exif/exif_parse.rs`, built in through
//! `SK_EXIF_PARSE_WITH_RUST` in `chromium/src@bd57e07c:skia/BUILD.gn`).
//! [`Orientation::from_tiff`] reads what that parser reads:
//!
//! - the TIFF header `II*\0` or `MM\0*` and the offset of IFD0;
//! - the entries of IFD0 in order, as long as each one is whole: the count
//!   may claim more entries than the data holds, and the next-IFD offset
//!   may be missing;
//! - the Orientation tag `0x0112` of type SHORT, count 1 and a value from
//!   1 to 8. An Orientation entry of another type, count or value is
//!   skipped, and a later valid one counts;
//! - the first `ExifIFD` pointer of IFD0 (tag `0x8769`, type LONG, count 1),
//!   whose sub-IFD is walked at that point of IFD0's order, so an
//!   Orientation there counts when none came before it. Pointers inside the
//!   sub-IFD are not followed.
//!
//! The first valid Orientation in that order is the image's. Firefox
//! (`mozilla-central@08f5d929b54d:image/decoders/EXIF.cpp`) agrees on a
//! missing next-IFD offset, but stops at an Orientation entry that is not a
//! SHORT of count 1, so it shows such an image as stored where Chromium
//! turns it; the stored file then holds only the valid tag, and both turn
//! it. Which EXIF segment of a JPEG is read is the strip's rule
//! (`jpeg/strip.rs`), also Chromium's.
//!
//! **Never larger.** One shape that Chromium reads is not read here: IFD0
//! starting inside the 8-byte header (an offset below 8), which no writer
//! produces. Such a structure can be shorter than any IFD that holds an
//! entry, so its replacement could make the upload larger, which ADR 0015
//! §7 never allows; its orientation is dropped instead. Every structure that
//! [`Orientation::from_tiff`] accepts holds IFD0 at offset 8 or later with
//! at least one whole entry, so it is at least [`ORIENTATION_TIFF_MIN_LEN`]
//! bytes long, and [`Orientation::tiff_replacing`] is never longer than it.
//!
//! The JPEG strip uses this module for its APP1 segment; the PNG `eXIf`
//! chunk and the WebP `EXIF` chunk hold the same TIFF structure.

/// The TIFF tag number of Orientation.
const ORIENTATION_TAG: u16 = 0x0112;

/// The TIFF tag number of the `ExifIFD` pointer, the offset of the EXIF
/// sub-IFD.
const EXIF_IFD_TAG: u16 = 0x8769;

/// The TIFF field type SHORT: an unsigned 16-bit integer.
const SHORT: u16 = 3;

/// The TIFF field type LONG: an unsigned 32-bit integer.
const LONG: u16 = 4;

/// The length of an IFD entry: tag, type, count and value.
const ENTRY_LEN: usize = 12;

/// The length of [`Orientation::to_tiff`]'s output: the 8-byte TIFF header,
/// an IFD of one 12-byte entry with its 2-byte count, and the 4-byte offset
/// of the next IFD. It replaces every EXIF structure at least this long.
pub const ORIENTATION_TIFF_LEN: usize = 26;

/// The length of the shortest structure that [`Orientation::from_tiff`]
/// accepts, and of the replacement of a structure shorter than
/// [`ORIENTATION_TIFF_LEN`]: the header, the count and one entry, without
/// the next-IFD offset, which Chromium and Firefox do not need.
pub const ORIENTATION_TIFF_MIN_LEN: usize = 22;

/// The EXIF Orientation of an image: which of the eight turns and flips a
/// viewer applies to the stored pixels before showing them. 1 shows them as
/// stored, 6 turns them 90° clockwise, and so on (TIFF 6.0, tag 274).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Orientation(u16);

impl Orientation {
    /// The orientation with the TIFF value `value`, or `None` when it is not
    /// one of the eight defined values 1 to 8.
    ///
    /// - `value`: the value of the Orientation tag.
    #[must_use]
    pub const fn new(value: u16) -> Option<Self> {
        if matches!(value, 1..=8) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// The TIFF value of the orientation, 1 to 8.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }

    /// Reads the Orientation of a TIFF structure, the payload of an EXIF
    /// block after its 6-byte `Exif` identifier, as Chromium reads it (the
    /// module documentation lists the rules).
    ///
    /// - `tiff`: the TIFF structure, starting with its byte-order mark.
    ///
    /// Returns `None` when the header is malformed or cut short, when IFD0
    /// starts inside the header, or when neither IFD0 nor its EXIF sub-IFD
    /// holds a valid Orientation entry. A structure that this accepts is
    /// never shorter than [`ORIENTATION_TIFF_MIN_LEN`].
    #[must_use]
    pub fn from_tiff(tiff: &[u8]) -> Option<Self> {
        let order = ByteOrder::of(tiff)?;
        let ifd0 = usize::try_from(order.u32_at(tiff, 4)?).ok()?;
        // The one shape Chromium reads and this does not (module
        // documentation, "Never larger").
        if ifd0 < 8 {
            return None;
        }
        let mut followed_exif_ifd = false;
        for entry in Entries::new(tiff, order, ifd0) {
            if let Some(orientation) = entry.orientation() {
                return Some(orientation);
            }
            // Skia follows only the first well-formed pointer, which also
            // keeps a crafted IFD0 full of pointers from costing more than
            // one walk of the sub-IFD.
            if entry.tag == EXIF_IFD_TAG && entry.field_type == LONG && entry.count == 1 {
                if followed_exif_ifd {
                    continue;
                }
                followed_exif_ifd = true;
                let Some(sub_ifd) = order
                    .u32_at(tiff, entry.at + 8)
                    .and_then(|offset| usize::try_from(offset).ok())
                else {
                    continue;
                };
                if let Some(orientation) =
                    Entries::new(tiff, order, sub_ifd).find_map(|entry| entry.orientation())
                {
                    return Some(orientation);
                }
            }
        }
        None
    }

    /// The smallest TIFF structure that holds only this orientation: a
    /// little-endian header, IFD0 with the one Orientation entry, and no
    /// next IFD.
    #[must_use]
    pub fn to_tiff(self) -> [u8; ORIENTATION_TIFF_LEN] {
        let [low, high] = self.0.to_le_bytes();
        [
            b'I', b'I', 0x2A, 0, 8, 0, 0, 0, // header: little-endian, IFD0 at offset 8
            1, 0, // one entry
            0x12, 0x01, 3, 0, 1, 0, 0, 0, low, high, 0, 0, // Orientation, SHORT, count 1
            0, 0, 0, 0, // no next IFD
        ]
    }

    /// The TIFF structure that replaces an EXIF structure of `original_len`
    /// bytes: [`Orientation::to_tiff`] when the original is at least that
    /// long, and otherwise its first [`ORIENTATION_TIFF_MIN_LEN`] bytes,
    /// without the next-IFD offset. That shorter form is the shape of such
    /// an original (it, too, lacks the offset), and Chromium and Firefox
    /// read it, so the stored image is turned as the upload was, and its
    /// metadata never grows (ADR 0015 §7).
    ///
    /// - `original_len`: the length of the TIFF structure that
    ///   [`Orientation::from_tiff`] read this orientation from; never below
    ///   [`ORIENTATION_TIFF_MIN_LEN`] for a structure it accepts.
    #[must_use]
    pub fn tiff_replacing(self, original_len: usize) -> Vec<u8> {
        let len = if original_len >= ORIENTATION_TIFF_LEN {
            ORIENTATION_TIFF_LEN
        } else {
            ORIENTATION_TIFF_MIN_LEN
        };
        self.to_tiff()[..len].to_vec()
    }
}

/// One whole 12-byte entry of an IFD.
#[derive(Debug, Clone, Copy)]
struct Entry<'a> {
    /// The structure the entry sits in.
    tiff: &'a [u8],
    /// Its byte order.
    order: ByteOrder,
    /// The offset of the entry in `tiff`.
    at: usize,
    /// The tag number.
    tag: u16,
    /// The field type.
    field_type: u16,
    /// The number of values.
    count: u32,
}

impl Entry<'_> {
    /// The orientation this entry gives: an Orientation tag of type SHORT
    /// and count 1, whose value (in the first two bytes of the value field)
    /// is 1 to 8. Anything else gives none, and the walk goes on.
    fn orientation(&self) -> Option<Orientation> {
        if self.tag != ORIENTATION_TAG || self.field_type != SHORT || self.count != 1 {
            return None;
        }
        Orientation::new(self.order.u16_at(self.tiff, self.at + 8)?)
    }
}

/// The whole entries of the IFD at an offset, in order. Like Skia's walk,
/// it stops at the first entry that the data does not hold whole, whatever
/// the count claims, and needs no next-IFD offset after the last one.
struct Entries<'a> {
    /// The structure the IFD sits in.
    tiff: &'a [u8],
    /// Its byte order.
    order: ByteOrder,
    /// The offset of the next entry.
    next: usize,
    /// The entries the count still claims.
    left: u16,
}

impl<'a> Entries<'a> {
    /// The entries of the IFD at `ifd`; none when its count is past the end.
    fn new(tiff: &'a [u8], order: ByteOrder, ifd: usize) -> Self {
        let left = order.u16_at(tiff, ifd).unwrap_or(0);
        Self {
            tiff,
            order,
            next: ifd.saturating_add(2),
            left,
        }
    }
}

impl<'a> Iterator for Entries<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        let at = self.next;
        self.tiff.get(at..at.checked_add(ENTRY_LEN)?)?;
        self.left -= 1;
        self.next = at + ENTRY_LEN;
        Some(Entry {
            tiff: self.tiff,
            order: self.order,
            at,
            tag: self.order.u16_at(self.tiff, at)?,
            field_type: self.order.u16_at(self.tiff, at + 2)?,
            count: self.order.u32_at(self.tiff, at + 4)?,
        })
    }
}

/// The byte order that a TIFF structure declares in its first four bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteOrder {
    /// `II*\0`: little-endian.
    Little,
    /// `MM\0*`: big-endian.
    Big,
}

impl ByteOrder {
    /// The byte order of `tiff`, or `None` when it starts with neither mark.
    fn of(tiff: &[u8]) -> Option<Self> {
        match tiff.get(..4)? {
            b"II\x2A\0" => Some(Self::Little),
            b"MM\0\x2A" => Some(Self::Big),
            _ => None,
        }
    }

    /// The 16-bit integer at offset `at`, or `None` past the end.
    fn u16_at(self, tiff: &[u8], at: usize) -> Option<u16> {
        let bytes = tiff.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        })
    }

    /// The 32-bit integer at offset `at`, or `None` past the end.
    fn u32_at(self, tiff: &[u8], at: usize) -> Option<u32> {
        let bytes = tiff.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*};

    /// One IFD entry in big-endian order.
    fn entry(tag: u16, field_type: u16, count: u32, value: [u8; 4]) -> Vec<u8> {
        [
            tag.to_be_bytes().as_slice(),
            &field_type.to_be_bytes(),
            &count.to_be_bytes(),
            &value,
        ]
        .concat()
    }

    /// A big-endian TIFF structure with IFD0 at `ifd` holding `entries`,
    /// followed by `tail`, the next-IFD offset and anything after it.
    fn big_endian(ifd: u32, entries: &[Vec<u8>], tail: &[u8]) -> Vec<u8> {
        let mut tiff = b"MM\0\x2A".to_vec();
        tiff.extend(ifd.to_be_bytes());
        tiff.resize(usize::try_from(ifd).unwrap().max(8), 0);
        tiff.extend(u16::try_from(entries.len()).unwrap().to_be_bytes());
        tiff.extend(entries.concat());
        tiff.extend(tail);
        tiff
    }

    /// The orientation entry with `value`, SHORT of count 1.
    fn orientation(value: u16) -> Vec<u8> {
        let [high, low] = value.to_be_bytes();
        entry(ORIENTATION_TAG, SHORT, 1, [high, low, 0, 0])
    }

    /// A camera-like IFD0: make, model, the orientation, a date and the GPS
    /// pointer, in tag order.
    fn camera_ifd(value: u16) -> Vec<Vec<u8>> {
        vec![
            entry(0x010F, 2, 4, *b"Cam\0"),
            entry(0x0110, 2, 4, *b"X1\0\0"),
            orientation(value),
            entry(0x0132, 2, 20, [0, 0, 0, 0x80]),
            entry(0x8825, 4, 1, [0, 0, 0, 0x90]),
        ]
    }

    /// A big-endian structure whose IFD0 holds `before`, then the `ExifIFD`
    /// pointer, then `after`; the sub-IFD holds `sub` and follows IFD0.
    fn with_exif_ifd(before: &[Vec<u8>], sub: &[Vec<u8>], after: &[Vec<u8>]) -> Vec<u8> {
        let ifd0_len = 2 + ENTRY_LEN * (before.len() + 1 + after.len()) + 4;
        let sub_at = u32::try_from(8 + ifd0_len).unwrap();
        let pointer = entry(EXIF_IFD_TAG, LONG, 1, sub_at.to_be_bytes());
        let ifd0: Vec<Vec<u8>> = [before, &[pointer], after].concat();
        let mut tiff = big_endian(8, &ifd0, &[0; 4]);
        tiff.extend(u16::try_from(sub.len()).unwrap().to_be_bytes());
        tiff.extend(sub.concat());
        tiff.extend([0; 4]);
        tiff
    }

    #[test]
    fn only_the_eight_defined_values_are_orientations() {
        assert_eq!(Orientation::new(0), None);
        assert_eq!(Orientation::new(9), None);
        assert_eq!(Orientation::new(u16::MAX), None);
        for value in 1..=8 {
            assert_eq!(Orientation::new(value).map(Orientation::value), Some(value));
        }
    }

    #[test]
    fn the_minimal_structure_is_the_one_the_corpus_uses() {
        // tests/fixtures/images/jpeg/exif-orientation.jpg holds these bytes.
        let six = Orientation::new(6).unwrap().to_tiff();
        assert_eq!(
            six,
            *b"II\x2A\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0"
        );
        assert_eq!(Orientation::from_tiff(&six), Orientation::new(6));
    }

    #[test]
    fn reads_the_tag_among_others_in_either_byte_order() {
        let big = big_endian(8, &camera_ifd(6), &[0; 4]);
        assert_eq!(Orientation::from_tiff(&big), Orientation::new(6));
        // The same entry in little-endian order, with IFD0 placed further on.
        let mut little = b"II\x2A\0\x10\0\0\0".to_vec();
        little.extend([0; 8]);
        little.extend([
            1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        assert_eq!(Orientation::from_tiff(&little), Orientation::new(8));
    }

    #[test]
    fn an_ifd0_without_the_next_ifd_offset_is_read() {
        // D78 shape 1: Chromium and Firefox read the entries without it.
        let lone = Orientation::new(6).unwrap().to_tiff();
        for cut in 0..=4 {
            let tiff = &lone[..ORIENTATION_TIFF_LEN - cut];
            assert_eq!(Orientation::from_tiff(tiff), Orientation::new(6), "{cut}");
        }
        let camera = big_endian(8, &camera_ifd(3), &[]);
        assert_eq!(Orientation::from_tiff(&camera), Orientation::new(3));
        // A count that claims more entries than the data holds: the whole
        // ones are read, as Skia reads them.
        let mut claims_more = big_endian(8, &[orientation(5)], &[]);
        claims_more[9] = 40;
        assert_eq!(Orientation::from_tiff(&claims_more), Orientation::new(5));
        // But an entry that is not whole is not read.
        assert_eq!(
            Orientation::from_tiff(&lone[..ORIENTATION_TIFF_MIN_LEN - 1]),
            None
        );
    }

    #[test]
    fn an_orientation_entry_that_readers_ignore_is_skipped_for_a_later_one() {
        // D78 shape 2: Skia skips an Orientation entry that is not a SHORT of
        // count 1 with a value from 1 to 8, and takes a later valid one.
        let ignored = [
            entry(ORIENTATION_TAG, SHORT, 1, [0, 0, 0, 0]),
            entry(ORIENTATION_TAG, SHORT, 1, [0, 9, 0, 0]),
            entry(ORIENTATION_TAG, LONG, 1, [0, 0, 0, 6]),
            entry(ORIENTATION_TAG, SHORT, 2, [0, 6, 0, 6]),
        ];
        for first in &ignored {
            let alone = big_endian(8, std::slice::from_ref(first), &[0; 4]);
            assert_eq!(Orientation::from_tiff(&alone), None);
            let then_valid = big_endian(8, &[first.clone(), orientation(6)], &[0; 4]);
            assert_eq!(Orientation::from_tiff(&then_valid), Orientation::new(6));
        }
        // Of two valid ones, the first counts.
        let two = big_endian(8, &[orientation(8), orientation(3)], &[0; 4]);
        assert_eq!(Orientation::from_tiff(&two), Orientation::new(8));
    }

    #[test]
    fn the_exif_sub_ifd_is_read_where_its_pointer_stands() {
        let other = entry(0x9003, 2, 4, *b"2026");
        // No Orientation in IFD0: the sub-IFD's counts.
        let only_sub = with_exif_ifd(&[], &[other.clone(), orientation(7)], &[]);
        assert_eq!(Orientation::from_tiff(&only_sub), Orientation::new(7));
        // IFD0's comes first in the walk, so it wins; after the pointer, the
        // sub-IFD's comes first.
        let ifd0_first = with_exif_ifd(&[orientation(2)], &[orientation(7)], &[]);
        assert_eq!(Orientation::from_tiff(&ifd0_first), Orientation::new(2));
        let pointer_first = with_exif_ifd(&[], &[orientation(7)], &[orientation(2)]);
        assert_eq!(Orientation::from_tiff(&pointer_first), Orientation::new(7));
        // A sub-IFD without one leaves a later IFD0 entry to count.
        let none_in_sub = with_exif_ifd(&[], &[other], &[orientation(4)]);
        assert_eq!(Orientation::from_tiff(&none_in_sub), Orientation::new(4));
        // A pointer of the wrong type is not followed.
        let mut wrong_type = only_sub.clone();
        wrong_type[12..14].copy_from_slice(&SHORT.to_be_bytes());
        assert_eq!(Orientation::from_tiff(&wrong_type), None);
        // A pointer past the end leads nowhere.
        let mut past = only_sub;
        past[18..22].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(Orientation::from_tiff(&past), None);
    }

    #[test]
    fn an_exif_pointer_that_is_not_one_long_is_skipped_for_a_later_one() {
        // Skia follows only a pointer of type LONG and count 1, and does not
        // count a malformed one as followed (`skia@62095415:rust/exif/exif_parse.rs`).
        let only_sub = with_exif_ifd(&[], &[orientation(7)], &[]);
        assert_eq!(Orientation::from_tiff(&only_sub), Orientation::new(7));
        // The pointer is IFD0's only entry: its count field is at 14..18.
        let mut count_two = only_sub;
        count_two[14..18].copy_from_slice(&2_u32.to_be_bytes());
        assert_eq!(Orientation::from_tiff(&count_two), None);

        // A count-2 pointer to a sub-IFD with orientation 3, then a count-1
        // pointer to one with orientation 5: the second is the first
        // well-formed one, so it is followed and the first is not.
        let first_sub = 8 + 2 + 2 * ENTRY_LEN + 4;
        let second_sub = first_sub + 2 + ENTRY_LEN + 4;
        let pointer = |count: u32, at: usize| {
            entry(
                EXIF_IFD_TAG,
                LONG,
                count,
                u32::try_from(at).unwrap().to_be_bytes(),
            )
        };
        let mut tiff = big_endian(8, &[pointer(2, first_sub), pointer(1, second_sub)], &[0; 4]);
        for value in [3, 5] {
            tiff.extend(1_u16.to_be_bytes());
            tiff.extend(orientation(value));
            tiff.extend([0; 4]);
        }
        assert_eq!(Orientation::from_tiff(&tiff), Orientation::new(5));
    }

    #[test]
    fn only_the_first_exif_pointer_is_followed() {
        // Two pointers to two sub-IFDs: only the first sub-IFD is read.
        let mut tiff = big_endian(8, &[], &[]);
        tiff.truncate(8);
        let second_sub = 8 + 2 + 2 * 12 + 4;
        let first_sub = second_sub + 2 + 12 + 4;
        tiff.extend(2_u16.to_be_bytes());
        tiff.extend(entry(
            EXIF_IFD_TAG,
            LONG,
            1,
            u32::try_from(first_sub).unwrap().to_be_bytes(),
        ));
        tiff.extend(entry(
            EXIF_IFD_TAG,
            LONG,
            1,
            u32::try_from(second_sub).unwrap().to_be_bytes(),
        ));
        tiff.extend([0; 4]);
        tiff.extend(1_u16.to_be_bytes());
        tiff.extend(orientation(5));
        tiff.extend([0; 4]);
        tiff.extend(1_u16.to_be_bytes());
        tiff.extend(entry(0x9003, 2, 4, *b"2026"));
        tiff.extend([0; 4]);
        assert_eq!(Orientation::from_tiff(&tiff), None);
    }

    #[test]
    fn malformed_structures_have_no_orientation() {
        let good = big_endian(8, &camera_ifd(3), &[0; 4]);
        assert_eq!(Orientation::from_tiff(&good), Orientation::new(3));
        // A wrong mark, IFD0 inside the header, or pointing past the end.
        let mut mark = good.clone();
        mark[1] = b'I';
        assert_eq!(Orientation::from_tiff(&mark), None);
        let mut inside = good.clone();
        inside[7] = 4;
        assert_eq!(Orientation::from_tiff(&inside), None);
        let mut past = good;
        past[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(Orientation::from_tiff(&past), None);
        assert_eq!(Orientation::from_tiff(b""), None);
        assert_eq!(Orientation::from_tiff(b"MM\0\x2A\0\0\0"), None);
    }

    #[test]
    fn ifd0_inside_the_header_is_the_one_shape_not_read() {
        // Big-endian, IFD0 at 6: its count is the offset's low half (6),
        // and its first entry starts at 8. Chromium reads the orientation of
        // this 20-byte structure; its 22-byte replacement would outgrow it.
        let mut overlapping = b"MM\0\x2A\0\0\0\x06".to_vec();
        overlapping.extend(orientation(6));
        assert_eq!(overlapping.len(), 20);
        assert_eq!(Orientation::from_tiff(&overlapping), None);
    }

    #[test]
    fn an_ifd_without_the_tag_has_no_orientation() {
        let ifd = [entry(0x010F, 2, 4, *b"Cam\0")];
        assert_eq!(Orientation::from_tiff(&big_endian(8, &ifd, &[0; 4])), None);
        assert_eq!(Orientation::from_tiff(&big_endian(8, &[], &[0; 4])), None);
    }

    #[test]
    fn the_replacement_is_never_longer_than_the_original() {
        let six = Orientation::new(6).unwrap();
        assert_eq!(six.tiff_replacing(40), six.to_tiff().to_vec());
        assert_eq!(
            six.tiff_replacing(ORIENTATION_TIFF_LEN),
            six.to_tiff().to_vec()
        );
        for original in ORIENTATION_TIFF_MIN_LEN..ORIENTATION_TIFF_LEN {
            let short = six.tiff_replacing(original);
            assert_eq!(short, six.to_tiff()[..ORIENTATION_TIFF_MIN_LEN]);
            // The short form reads back, as the browsers read it.
            assert_eq!(Orientation::from_tiff(&short), Some(six));
        }
    }

    proptest! {
        #[test]
        fn exactly_the_values_1_to_8_are_orientations(value in any::<u16>()) {
            let orientation = Orientation::new(value);
            prop_assert_eq!(orientation.is_some(), (1..=8).contains(&value));
            prop_assert!(orientation.is_none_or(|orientation| orientation.value() == value));
        }

        #[test]
        fn every_orientation_round_trips_in_both_forms(value in 1..=8_u16, original in 0..64_usize) {
            let orientation = Orientation::new(value).unwrap();
            prop_assert_eq!(Orientation::from_tiff(&orientation.to_tiff()), Some(orientation));
            prop_assert_eq!(Orientation::from_tiff(&orientation.tiff_replacing(original)), Some(orientation));
        }

        #[test]
        fn the_tag_is_found_wherever_ifd0_and_the_tag_sit(
            value in 1..=8_u16,
            padding in 0..64_u32,
            before in vec((0x0100..0x0112_u16, any::<[u8; 4]>()), 0..6),
            after in vec((0x0113..=u16::MAX, any::<[u8; 4]>()), 0..6),
            tail in vec(any::<u8>(), 0..16),
        ) {
            let mut entries: Vec<Vec<u8>> =
                before.iter().map(|&(tag, value)| entry(tag, 7, 4, value)).collect();
            entries.push(orientation(value));
            // `after` holds tags above 0x0112, among them possibly the
            // ExifIFD pointer, but of type UNDEFINED (7), never followed.
            entries.extend(after.iter().map(|&(tag, value)| entry(tag, 7, 4, value)));
            let tiff = big_endian(8 + padding, &entries, &tail);
            prop_assert_eq!(Orientation::from_tiff(&tiff), Orientation::new(value));
        }

        #[test]
        fn arbitrary_bytes_never_panic_and_the_replacement_never_grows(
            mark in prop_oneof![Just(b"II\x2A\0".to_vec()), Just(b"MM\0\x2A".to_vec()), vec(any::<u8>(), 0..4)],
            rest in vec(any::<u8>(), 0..80),
        ) {
            let tiff = [mark, rest].concat();
            if let Some(orientation) = Orientation::from_tiff(&tiff) {
                prop_assert!(tiff.len() >= ORIENTATION_TIFF_MIN_LEN);
                prop_assert!(orientation.tiff_replacing(tiff.len()).len() <= tiff.len());
                prop_assert!((1..=8).contains(&orientation.value()));
            }
        }
    }
}
