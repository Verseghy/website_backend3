//! Where a zlib stream ends, how much it decodes to, and whether its rows
//! are rows of a PNG.
//!
//! A PNG keeps three kinds of zlib stream that the optimiser may store: the
//! image data (`IDAT`), the data of each animation frame (`fdAT`) and the
//! ICC profile (`iCCP`). Decoders stop at the end of a stream and ignore what
//! follows it in the chunk, so bytes appended after the stream would travel
//! with the image unseen. This check finds the exact end, so the caller can
//! cut it there, and counts the decoded bytes, so the caller can hold them to
//! the size it expects. For image data, [`check_rows`] also reads the
//! filter-type byte at the start of every row, which a decoder refuses when
//! it is not one of PNG's five filter types.
//!
//! The stream is inflated into a 32 KiB ring, the largest window zlib
//! allows. [`check_rows`] only looks at the output as it passes, so checking
//! a 50-megapixel image needs no buffer for its pixels; [`inflate`] also
//! keeps it, for the ICC profile, whose content the caller must read. Both
//! stop one window past the size they expect, so one decompression bomb
//! costs no more work than an honest stream of that size, and [`check_rows`]
//! stops at the first bad filter-type byte. The caller bounds how many
//! streams it inflates.

use miniz_oxide::inflate::{
    TINFLStatus,
    core::{
        DecompressorOxide, decompress,
        inflate_flags::{TINFL_FLAG_COMPUTE_ADLER32, TINFL_FLAG_PARSE_ZLIB_HEADER},
    },
};

/// The ring that holds the decoded window: 32 KiB, a power of two.
const WINDOW: usize = 32 * 1024;

/// The highest filter type PNG defines: 0 None, 1 Sub, 2 Up, 3 Average and
/// 4 Paeth (PNG 3rd edition, §9.2). libpng refuses any other ("bad adaptive
/// filter value"), and so does the `png` crate.
const MAX_FILTER_TYPE: u8 = 4;

/// A complete, valid zlib stream at the start of the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Stream {
    /// The length of the stream in bytes, up to and including its Adler-32.
    pub(super) len: usize,
    /// How many bytes it decodes to.
    pub(super) decoded: u64,
}

/// One pass of an image's rows, as its zlib stream holds them: a still
/// image has one, an Adam7-interlaced one up to seven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pass {
    /// The bytes of one row, its filter-type byte included; at least 2.
    pub(super) row: u64,
    /// How many rows the pass has; at least 1.
    pub(super) rows: u64,
}

/// Checks that the zlib stream at the start of `data` holds exactly the rows
/// of `passes`, each starting with a filter-type byte from 0 to 4, and
/// returns the length of the stream.
///
/// - `data`: the bytes that should hold the image data's zlib stream;
///   anything after the stream's end is not read.
/// - `passes`: the rows the image data must decode to, pass by pass in
///   order, with no empty pass; [`super::chunks::image_rows`] gives them for
///   an `IHDR`.
///
/// Returns `None` when the stream is not valid zlib, as for [`inflate`],
/// when it decodes to more or fewer bytes than the rows hold, or as soon as
/// a row starts with another filter type.
pub(super) fn check_rows(data: &[u8], passes: &[Pass]) -> Option<usize> {
    let expected = passes.iter().try_fold(0_u64, |sum, pass| {
        pass.row
            .checked_mul(pass.rows)
            .and_then(|size| sum.checked_add(size))
    })?;
    let mut rows = RowStarts::new(passes);
    let stream = walk(data, expected, |piece| rows.check(piece))?;
    (stream.decoded == expected).then_some(stream.len)
}

/// Checks the zlib stream at the start of `data`, and returns where it ends,
/// how much it decodes to and what.
///
/// - `data`: the bytes that should hold one zlib stream; anything after the
///   stream's end is not read.
/// - `limit`: the most decoded bytes the caller accepts; the buffer never
///   grows past it.
///
/// Returns `None` when the stream is not valid zlib (a bad header, a preset
/// dictionary, which PNG forbids, a deflate error or a wrong Adler-32, which
/// libpng refuses in image data), when `data` ends before the stream does,
/// or when it decodes to more than `limit` bytes.
pub(super) fn inflate(data: &[u8], limit: u64) -> Option<(Stream, Vec<u8>)> {
    let mut out = Vec::new();
    let stream = walk(data, limit, |piece| {
        out.extend_from_slice(piece);
        true
    })?;
    Some((stream, out))
}

/// Where the next row of [`check_rows`] starts in the decoded output, as the
/// output passes in pieces.
struct RowStarts<'a> {
    /// The passes still to come after the current one.
    passes: std::slice::Iter<'a, Pass>,
    /// The row length of the current pass.
    row: u64,
    /// The rows of the current pass not yet started.
    left: u64,
    /// The offset in the decoded output where the next row starts, or
    /// `u64::MAX` once every row has started.
    next: u64,
    /// The offset of the first byte of the next piece.
    at: u64,
}

impl<'a> RowStarts<'a> {
    /// The row starts of `passes`, the first at offset 0.
    ///
    /// - `passes`: as for [`check_rows`].
    fn new(passes: &'a [Pass]) -> Self {
        let mut starts = Self {
            passes: passes.iter(),
            row: 0,
            left: 0,
            next: 0,
            at: 0,
        };
        starts.next_pass();
        starts
    }

    /// Moves to the next pass that has rows, or past the last row when there
    /// is none.
    fn next_pass(&mut self) {
        match self.passes.find(|pass| pass.rows > 0) {
            Some(pass) => (self.row, self.left) = (pass.row, pass.rows),
            None => self.next = u64::MAX,
        }
    }

    /// Checks the filter-type bytes inside `piece`, the next piece of the
    /// decoded output; `false` at the first one above 4.
    ///
    /// - `piece`: the decoded bytes that follow the previous piece.
    fn check(&mut self, piece: &[u8]) -> bool {
        let end = self.at + u64::try_from(piece.len()).unwrap_or(u64::MAX);
        while self.next < end {
            // In range: `next` lies inside this piece.
            let Ok(index) = usize::try_from(self.next - self.at) else {
                return false;
            };
            if piece[index] > MAX_FILTER_TYPE {
                return false;
            }
            self.left -= 1;
            self.next = self.next.saturating_add(self.row);
            if self.left == 0 {
                self.next_pass();
            }
        }
        self.at = end;
        true
    }
}

/// The inflate loop of [`check_rows`] and [`inflate`], with their `None`
/// cases.
///
/// - `data`, `limit`: as for [`inflate`].
/// - `sink`: receives every piece of decoded output, in order, and returns
///   `false` to stop the inflate, which then fails.
fn walk(data: &[u8], limit: u64, mut sink: impl FnMut(&[u8]) -> bool) -> Option<Stream> {
    let mut state = Box::<DecompressorOxide>::default();
    let mut ring = vec![0; WINDOW];
    let flags = TINFL_FLAG_PARSE_ZLIB_HEADER | TINFL_FLAG_COMPUTE_ADLER32;
    let (mut read, mut at, mut decoded) = (0_usize, 0_usize, 0_u64);
    loop {
        let (status, consumed, written) =
            decompress(&mut state, data.get(read..)?, &mut ring, at, flags);
        read += consumed;
        decoded += u64::try_from(written).ok()?;
        if decoded > limit {
            return None;
        }
        // One call writes from `at` towards the end of the ring and never
        // past it (it reports HasMoreOutput there), so the new output is
        // the contiguous piece `at..at + written`.
        if !sink(&ring[at..at + written]) {
            return None;
        }
        at = (at + written) & (WINDOW - 1);
        match status {
            TINFLStatus::Done => return Some(Stream { len: read, decoded }),
            // The ring is full: go round again. A call that made no progress
            // would loop for ever, so it counts as a failure.
            TINFLStatus::HasMoreOutput if consumed + written > 0 => {}
            _ => return None,
        }
    }
}

#[cfg(test)]
pub(in crate::domain::image_optimise) mod samples {
    //! zlib streams built without a compressor, from stored deflate blocks.

    /// `data` as a zlib stream of stored (uncompressed) deflate blocks.
    pub(in crate::domain::image_optimise) fn stored(data: &[u8]) -> Vec<u8> {
        // CMF 0x78 (deflate, 32 KiB window) and FLG 0x01: no dictionary,
        // and 0x7801 is a multiple of 31, as RFC 1950 requires.
        let mut stream = vec![0x78, 0x01];
        let mut blocks = data.chunks(0xFFFF).peekable();
        if blocks.peek().is_none() {
            stream.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
        }
        while let Some(block) = blocks.next() {
            let last = u8::from(blocks.peek().is_none());
            let len = u16::try_from(block.len()).unwrap();
            stream.push(last);
            stream.extend_from_slice(&len.to_le_bytes());
            stream.extend_from_slice(&(!len).to_le_bytes());
            stream.extend_from_slice(block);
        }
        stream.extend_from_slice(&adler32(data).to_be_bytes());
        stream
    }

    /// The Adler-32 of `data` (RFC 1950).
    pub(in crate::domain::image_optimise) fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1_u32, 0_u32);
        for &byte in data {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        (b << 16) | a
    }
}

#[cfg(test)]
mod tests {
    use super::samples::stored;
    use super::*;
    use miniz_oxide::deflate::compress_to_vec_zlib;
    use proptest::{collection::vec, prelude::*};

    /// The stream at the start of `data`, when it decodes to at most `limit`
    /// bytes, with nothing looking at its output.
    fn check(data: &[u8], limit: u64) -> Option<Stream> {
        walk(data, limit, |_| true)
    }

    #[test]
    fn a_stream_reports_its_length_and_decoded_size() {
        let data = b"backend3 test corpus".repeat(500);
        for stream in [stored(&data), compress_to_vec_zlib(&data, 9)] {
            assert_eq!(
                check(&stream, 10_000),
                Some(Stream {
                    len: stream.len(),
                    decoded: 10_000
                })
            );
        }
    }

    #[test]
    fn bytes_after_the_stream_are_not_part_of_it() {
        let stream = compress_to_vec_zlib(b"pixels", 6);
        let with_tail = [&stream[..], b"<script>payload</script>"].concat();
        assert_eq!(check(&with_tail, 6).map(|s| s.len), Some(stream.len()));
    }

    #[test]
    fn more_output_than_the_limit_is_refused() {
        // 8 MiB of zeros compress to about 8 KiB: a small bomb.
        let bomb = compress_to_vec_zlib(&vec![0; 8 << 20], 9);
        assert!(bomb.len() < 16 << 10);
        assert_eq!(check(&bomb, 1 << 20), None);
        assert!(check(&bomb, 8 << 20).is_some());
        // inflate stops at the same point, so its buffer stays bounded.
        assert_eq!(inflate(&bomb, 1 << 20), None);
    }

    #[test]
    fn broken_streams_are_refused() {
        let good = stored(b"pixels");
        let mut adler = good.clone();
        *adler.last_mut().unwrap() ^= 1;
        let mut header = good.clone();
        header[1] = 0x02;
        let mut dictionary = good.clone();
        // FDICT set, with the check bits redone: 0x7820 is a multiple of 31.
        dictionary[1] = 0x20;
        for broken in [
            &adler[..],
            &header,
            &dictionary,
            &good[..good.len() - 1],
            &good[..2],
            &[],
        ] {
            assert_eq!(check(broken, 1 << 20), None, "{broken:?}");
        }
    }

    /// `rows` rows of `row` bytes, each starting with filter type
    /// `index % 5` and holding the row index in the rest.
    fn rows_of(row: u64, rows: u64) -> Vec<u8> {
        let mut data = Vec::new();
        for index in 0..rows {
            data.push(u8::try_from(index % 5).unwrap());
            data.extend(std::iter::repeat_n(
                u8::try_from(index % 251).unwrap(),
                usize::try_from(row - 1).unwrap(),
            ));
        }
        data
    }

    #[test]
    fn rows_with_known_filter_types_pass_and_report_the_stream() {
        let passes = [Pass { row: 7, rows: 3 }, Pass { row: 4, rows: 2 }];
        let data = [rows_of(7, 3), rows_of(4, 2)].concat();
        for stream in [stored(&data), compress_to_vec_zlib(&data, 6)] {
            let with_tail = [&stream[..], b"payload"].concat();
            assert_eq!(check_rows(&with_tail, &passes), Some(stream.len()));
        }
        // An empty pass between two others has no rows to check.
        let gapped = [passes[0], Pass { row: 9, rows: 0 }, passes[1]];
        assert_eq!(
            check_rows(&stored(&data), &gapped),
            Some(stored(&data).len())
        );
    }

    #[test]
    fn a_filter_type_above_four_is_refused_in_any_row_of_any_pass() {
        let passes = [Pass { row: 7, rows: 3 }, Pass { row: 4, rows: 2 }];
        let data = [rows_of(7, 3), rows_of(4, 2)].concat();
        // The first byte of every row, in both passes.
        for start in [0, 7, 14, 21, 25] {
            for filter in [5, 6, 0xFF] {
                let mut broken = data.clone();
                broken[start] = filter;
                assert_eq!(
                    check_rows(&stored(&broken), &passes),
                    None,
                    "{start} {filter}"
                );
            }
        }
        // A byte above 4 inside a row is a sample, not a filter type.
        let mut sample = data.clone();
        sample[1] = 0xFF;
        sample[22] = 9;
        assert!(check_rows(&stored(&sample), &passes).is_some());
    }

    #[test]
    fn rows_must_decode_to_exactly_their_size() {
        let passes = [Pass { row: 7, rows: 3 }];
        let data = rows_of(7, 3);
        assert_eq!(check_rows(&stored(&data[..20]), &passes), None);
        let long = [&data[..], &[0]].concat();
        assert_eq!(check_rows(&stored(&long), &passes), None);
        assert_eq!(check_rows(b"not zlib", &passes), None);
        // Sizes that do not fit 64 bits are refused, not wrapped.
        let huge = [Pass {
            row: u64::MAX,
            rows: 2,
        }];
        assert_eq!(check_rows(&stored(&data), &huge), None);
    }

    proptest! {
        #[test]
        fn exactly_the_rows_with_filter_types_up_to_four_pass(
            shape in vec((2..40_u64, 1..12_u64), 1..8),
            filters in vec(any::<u8>(), 96),
            level in 0..=10_u8,
        ) {
            // Rows whose filter-type bytes come from `filters`, a sample
            // byte of 0xFF elsewhere.
            let passes: Vec<Pass> = shape.iter().map(|&(row, rows)| Pass { row, rows }).collect();
            let mut data = Vec::new();
            let mut valid = true;
            let mut index = 0;
            for pass in &passes {
                for _ in 0..pass.rows {
                    let filter = filters[index % filters.len()];
                    index += 1;
                    valid &= filter <= 4;
                    data.push(filter);
                    data.extend(std::iter::repeat_n(0xFF, usize::try_from(pass.row - 1).unwrap()));
                }
            }
            let stream = compress_to_vec_zlib(&data, level);
            prop_assert_eq!(check_rows(&stream, &passes), valid.then_some(stream.len()));
        }

        #[test]
        fn every_stream_is_measured_exactly(
            data in vec(any::<u8>(), 0..70_000),
            level in 0..=10_u8,
            tail in vec(any::<u8>(), 0..32),
        ) {
            let stream = compress_to_vec_zlib(&data, level);
            let decoded = u64::try_from(data.len()).unwrap();
            let found = check(&[&stream[..], &tail].concat(), decoded);
            prop_assert_eq!(found, Some(Stream { len: stream.len(), decoded }));
            if decoded > 0 {
                prop_assert_eq!(check(&stream, decoded - 1), None);
            }
            // inflate measures the same stream and returns its exact output,
            // also when the output wraps the ring several times.
            let found = inflate(&[&stream[..], &tail].concat(), decoded);
            prop_assert_eq!(found, Some((Stream { len: stream.len(), decoded }, data.clone())));
            if decoded > 0 {
                prop_assert_eq!(inflate(&stream, decoded - 1), None);
            }
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in vec(any::<u8>(), 0..256), limit in any::<u64>()) {
            if let Some(stream) = check(&bytes, limit) {
                prop_assert!(stream.len <= bytes.len());
                prop_assert!(stream.decoded <= limit);
            }
            if let Some((stream, out)) = inflate(&bytes, limit) {
                prop_assert_eq!(Some(stream), check(&bytes, limit));
                prop_assert_eq!(u64::try_from(out.len()).unwrap(), stream.decoded);
            }
        }
    }
}
