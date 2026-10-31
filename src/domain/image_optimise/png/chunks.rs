//! The PNG chunk walk: everything the optimiser keeps, and nothing else,
//! rewritten as a new PNG before the encoder sees it.
//!
//! ADR 0015 §7, as ADR 0033 §6 amends it, drops the ancillary chunks of a
//! PNG except the colour chunks (`iCCP`, `sRGB`, `gAMA`, `cHRM`), the HDR
//! colour chunks of the PNG third edition (`cICP`, `mDCV`, `cLLI`), the
//! animation chunks (`acTL`, `fcTL`, `fdAT`) and an `eXIf` reduced to
//! Orientation. The walk applies that list and writes a new file from the
//! parts it keeps, so nothing it does not understand reaches the encoder:
//! not a chunk of an unknown type, not the bytes after `IEND`, and not the
//! bytes after the end of a zlib stream. What it keeps besides the list is
//! the image itself: `IHDR`, `PLTE` and `tRNS`, which is ancillary by name
//! but holds the transparency of the pixels, and the image data.
//!
//! Each chunk of a still image is judged as libpng 1.6.58 judges it, checked
//! by feeding it the same files (through `ImageMagick` 7.1.2) and by reading
//! its source (`pngrutil.c`, `pngset.c`, `pngread.c`) for the HDR chunks,
//! the palette, the checksums and the filter types. libpng does not decode
//! animations, so the animation chunks follow the PNG specification as
//! Chromium's decoder, the `png` crate, reads it; [`super::apng`] has those
//! rules. Where the walk departs from libpng, the last column says so and
//! why:
//!
//! | Found | libpng | Here |
//! |---|---|---|
//! | a chunk cut short, no `IEND` | error | [`DecodeDefect::PngTruncated`] |
//! | a length above 2³¹ − 1, a type that is not four letters | error | [`DecodeDefect::PngChunk`] |
//! | a wrong CRC on `IHDR`, `IDAT`, an indexed image's `PLTE` or an unknown critical chunk | error | [`DecodeDefect::PngChecksum`] |
//! | a wrong CRC on `IEND`, or on the `PLTE` of an image that is not indexed | a warning; the chunk is taken as if it were intact | the same: `IEND` ends the file, and the `PLTE` places the colour chunks as below (neither is kept) |
//! | a wrong CRC on an ancillary chunk | discarded | dropped |
//! | an unknown critical chunk | error | [`DecodeDefect::PngUnknownCriticalChunk`] |
//! | a second `IHDR`, a second `PLTE` in an indexed image | error | [`DecodeDefect::PngChunkOrder`] |
//! | an indexed image without a valid `PLTE` before its data | error | [`DecodeDefect::PngPalette`] |
//! | an empty `PLTE` in a truecolour image, where libpng would take a palette (below) | error ("Invalid palette") | [`DecodeDefect::PngPalette`] |
//! | a first run of `IDAT` that is not one zlib stream (a wrong Adler-32 included) or decodes to less than `IHDR` declares | error | [`DecodeDefect::PngImageData`] |
//! | a row of the image data whose filter-type byte is above 4 | error ("bad adaptive filter value") | [`DecodeDefect::PngImageData`] |
//! | a first run of `IDAT` that decodes to more than `IHDR` declares | warning, the rest is ignored | [`DecodeDefect::PngImageData`]: stricter, because the extra decoded bytes are data no viewer shows, and they would travel inside the stream the optimiser may keep |
//! | `IDAT` chunks after the first run | ignored | dropped |
//! | a `PLTE` in a greyscale image | ignored | dropped |
//! | a truecolour image's suggested `PLTE` before the data, the first libpng takes, of whole entries, at most 256, after no `tRNS` or `bKGD` libpng took | taken; the colour chunks after it are ignored | dropped, as oxipng drops it; the colour chunks after it are dropped |
//! | any other `PLTE` of a truecolour image: after the data, a second one, not whole entries or more than 256, or after a `tRNS` or `bKGD` libpng took | ignored; the colour chunks after it still count | dropped; the colour chunks after it are kept as if it were not there |
//! | a palette longer than the bit depth can index | the extra entries are discarded | cut |
//! | a `tRNS` after the data, before an indexed image's `PLTE`, of the wrong length, longer than the palette, or in an image with alpha; a second one | ignored | dropped |
//! | a `tRNS` sample with bits set above the bit depth | a warning ("out-of-range samples"); decoders read it differently | masked to the bit depth, as PNG requires of decoders (3rd edition, §11.3.1.1), so every decoder and the encoder read one value |
//! | a colour or HDR chunk after `PLTE` or the data; `sRGB`, `cHRM`, `gAMA`, `cICP`, `mDCV` or `cLLI` of the wrong length; an invalid `sRGB` | ignored | dropped |
//! | a second `sRGB`, `gAMA`, `cHRM`, `cICP`, `mDCV` or `cLLI` after a first one libpng took (for `gAMA`, any of the right length, a zero gamma included; for the HDR chunks, any of the right length, valid values or not) | ignored | dropped |
//! | a `cICP` whose matrix coefficients are not 0 (PNG stores RGB only); an `mDCV` or `cLLI` with a light level above 2³¹ − 1 | ignored | dropped |
//! | a `cICP` whose full-range flag is not 0 or 1 | taken | dropped: stricter, because the `png` crate refuses the whole file for it, so the stored file decodes in both |
//! | an `iCCP` whose profile libpng rejects (not an ICC profile, the wrong colour space for the colour type, a declared size or a tag outside the data, above libpng's default limit of 8,000,000 bytes) | ignored | dropped |
//! | an `iCCP` after the first one | ignored after a valid one, read after an invalid one | dropped without inflating it, so an upload costs at most one profile of inflate work |
//! | an `eXIf` after the data, where PNG does not allow it | read | dropped |
//! | a second `eXIf` | ignored | dropped |
//! | `acTL`, `fcTL`, `fdAT` | ignored (no APNG support) | kept up to the animation's first defect, as [`super::apng`] says, with `acTL` announcing the frames kept; with no frame kept, dropped, as libpng drops them. Every other chunk between the frames is judged by the rows above and never cuts the animation, also where the `png` crate stops at it (an `IDAT` after the first run, a `PLTE` of an image that is not indexed) |
//!
//! The PNG that comes out is never larger than the one that went in: every
//! chunk it writes stands for an input chunk at least as large, and the
//! image data is the input's stream cut at its end. That keeps the
//! optimiser's "never larger" rule when the encoder cannot beat the input.

use super::apng;
use super::zlib::{self, Pass};
use crate::domain::image_optimise::{DecodeDefect, Orientation};

/// The 8-byte signature of every PNG.
pub(super) const SIGNATURE: [u8; 8] = *b"\x89PNG\r\n\x1A\n";

/// The largest chunk length PNG allows, 2³¹ − 1.
pub(super) const MAX_CHUNK: usize = 0x7FFF_FFFF;

/// The largest decoded ICC profile kept: libpng's default limit on the
/// memory of one chunk (`PNG_USER_CHUNK_MALLOC_MAX`), above which libpng
/// ignores the profile. Display profiles are a few kilobytes.
const MAX_PROFILE: u64 = 8_000_000;

/// The smallest ICC profile: the 128-byte header and the tag count.
const PROFILE_HEADER: usize = 132;

/// The largest light level `mDCV` and `cLLI` may hold, 2³¹ − 1 (in units of
/// 0.0001 cd/m²), above which libpng ignores the chunk.
const MAX_LIGHT_LEVEL: u32 = 0x7FFF_FFFF;

/// The Adam7 passes: first column, first row, column step, row step.
const ADAM7: [(u64, u64, u64, u64); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// The rewrite of an upload, and what the encoder must be told about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rewrite {
    /// The rewritten PNG.
    pub(super) png: Vec<u8>,
    /// Whether the image is 16-bit with a kept `tRNS` sample whose two bytes
    /// differ. Such a colour matches no pixel once every sample repeats its
    /// high byte in its low byte, which is when oxipng 10.2.1 reduces the
    /// image to 8 bits; it then keeps the 16-bit `tRNS` value, whose low
    /// byte decoders compare with the 8-bit samples, so opaque pixels would
    /// turn transparent. The encoder must keep the bit depth for it.
    pub(super) wide_transparency: bool,
    /// Whether a `cICP` chunk is kept, which describes RGB samples; the
    /// encoder must then keep the image in colour.
    pub(super) coded_colour: bool,
    /// The pixels the encoder works on: the canvas, and every frame of a
    /// kept animation after the image data, each at its own size. The
    /// encoder's preset is chosen by it (ADR 0033 §5).
    pub(super) pixels: u64,
}

/// Why the walk refuses an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The file does not decode, as libpng would refuse it.
    Defect(DecodeDefect),
    /// The animation is above the totals of [`super::apng`].
    AnimationTooLarge(apng::TooLarge),
}

impl From<DecodeDefect> for Refusal {
    fn from(defect: DecodeDefect) -> Self {
        Self::Defect(defect)
    }
}

/// One chunk of the input.
struct Chunk<'a> {
    /// The four-letter type.
    kind: [u8; 4],
    /// The chunk data.
    data: &'a [u8],
    /// Whether the CRC matches.
    intact: bool,
}

/// Where the walk is relative to the image data.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    /// No `IDAT` yet.
    Before,
    /// Inside the first run of `IDAT` chunks.
    In,
    /// Past it: the image data is complete.
    After,
}

/// Rewrites the PNG `bytes` from the chunks that ADR 0015 §7 and ADR 0033 §6
/// keep.
///
/// - `bytes`: a whole PNG whose signature and `IHDR` [`super::super::inspect`]
///   has accepted, so its canvas fits the pixel budget.
///
/// # Errors
///
/// [`Refusal::Defect`] with the [`DecodeDefect`] of the table in the module
/// documentation, for a file that libpng refuses or that has more image data
/// than it declares; [`Refusal::AnimationTooLarge`] for an animation above
/// the totals.
pub(super) fn canonical(bytes: &[u8]) -> Result<Rewrite, Refusal> {
    let (first, mut at) = read_chunk(bytes, SIGNATURE.len())?;
    if first.kind != *b"IHDR" || first.data.len() != 13 {
        return Err(DecodeDefect::PngChunkOrder.into());
    }
    let mut walk = Walk::new(first.data);
    loop {
        let (chunk, next) = read_chunk(bytes, at)?;
        at = next;
        if !walk.take(&chunk)? {
            return walk.finish();
        }
    }
}

/// What the walk has kept so far.
struct Walk<'a> {
    /// The 13 bytes of `IHDR`.
    ihdr: &'a [u8],
    /// The bit depth.
    depth: u8,
    /// The colour type.
    colour: u8,
    /// The palette an indexed image uses, cut to what its bit depth indexes.
    palette: Option<&'a [u8]>,
    /// Whether libpng has taken a `PLTE`, which places the colour chunks:
    /// libpng ignores a colour chunk after it, also after a truecolour
    /// image's suggested palette, but not after a palette it ignored.
    placed: bool,
    /// The kept `tRNS` data.
    transparency: Option<Vec<u8>>,
    /// Whether libpng has taken a `bKGD` of a truecolour image before the
    /// data, after which it ignores a suggested palette.
    background: bool,
    /// The colour, HDR and `eXIf` chunks, in the order they came.
    kept: Vec<([u8; 4], Vec<u8>)>,
    /// The chunk types whose one place is filled: an HDR chunk type by any
    /// chunk of the right length and place, valid values or not, as libpng
    /// counts it; `iCCP` and `eXIf` by the first one read, so a later one
    /// is dropped unread.
    claimed: Vec<[u8; 4]>,
    /// The animation chunks, checked as they come.
    frames: apng::Frames<'a>,
    /// Whether the chunk read last was an `fdAT`.
    after_fdat: bool,
    /// The joined data of the first run of `IDAT` chunks.
    data: Vec<u8>,
    /// Where the walk is relative to that run.
    run: Run,
}

impl<'a> Walk<'a> {
    /// A walk of the image whose `IHDR` data is `ihdr`.
    ///
    /// - `ihdr`: the 13 bytes of `IHDR`.
    fn new(ihdr: &'a [u8]) -> Self {
        Self {
            ihdr,
            depth: ihdr[8],
            colour: ihdr[9],
            palette: None,
            placed: false,
            transparency: None,
            background: false,
            kept: Vec::new(),
            claimed: Vec::new(),
            frames: apng::Frames::new(ihdr),
            after_fdat: false,
            data: Vec::new(),
            run: Run::Before,
        }
    }

    /// Takes the next chunk after `IHDR`; `false` once it is `IEND`.
    ///
    /// - `chunk`: the chunk, as read.
    fn take(&mut self, chunk: &Chunk<'a>) -> Result<bool, DecodeDefect> {
        // Any other chunk ends the run of image data, also one dropped below
        // for its CRC: libpng reads the image from the first run only. In the
        // same way, any chunk between two fdAT chunks parts them.
        if chunk.kind != *b"IDAT" && self.run == Run::In {
            self.run = Run::After;
        }
        let follows_fdat = std::mem::replace(&mut self.after_fdat, chunk.kind == *b"fdAT");
        let critical = chunk.kind[0].is_ascii_uppercase();
        // libpng 1.6.58 checks the CRC of IEND, and of the PLTE of an image
        // that is not indexed, as it checks an ancillary chunk's, and then
        // takes the chunk all the same: a mismatch is only a warning there
        // (png_handle_IEND, png_handle_PLTE). Neither chunk's data is kept.
        let crc_is_a_warning =
            chunk.kind == *b"IEND" || (chunk.kind == *b"PLTE" && self.colour != 3);
        let before = self.run == Run::Before;
        if !chunk.intact && !crc_is_a_warning {
            // An animation chunk lost to its CRC is a defect of the animation
            // where it lies (ADR 0033 §6), except an acTL after the data,
            // which is ignored, intact or not.
            match &chunk.kind {
                b"acTL" if before => self.frames.announce(None),
                b"fcTL" => self.frames.control(None, before),
                b"fdAT" => self.frames.data(None, before, follows_fdat),
                _ => {}
            }
            return if critical {
                Err(DecodeDefect::PngChecksum)
            } else {
                Ok(true)
            };
        }
        let colour_place = before && !self.placed;
        let has = |kind: &[u8; 4]| self.kept.iter().any(|(k, _)| k == kind);
        match &chunk.kind {
            b"IEND" => return Ok(false),
            // libpng reads the image from the first run and warns about
            // later IDAT chunks ("Too many IDATs found"); they are dropped.
            b"IDAT" if self.run == Run::After => {}
            b"IDAT" => {
                self.data.extend_from_slice(chunk.data);
                self.run = Run::In;
            }
            b"IHDR" => return Err(DecodeDefect::PngChunkOrder),
            b"PLTE" => self.take_palette(chunk.data)?,
            _ if critical => return Err(DecodeDefect::PngUnknownCriticalChunk),
            b"tRNS" if before && self.transparency.is_none() => {
                self.transparency =
                    valid_transparency(chunk.data, self.colour, self.depth, self.palette);
            }
            // Dropped, but it decides whether libpng takes a later suggested
            // palette, and so where the colour chunks may stand.
            b"bKGD" if before => {
                self.background |= background_taken(chunk.data, self.colour, self.depth);
            }
            b"iCCP" if colour_place && !self.claimed.contains(&chunk.kind) => {
                self.claimed.push(chunk.kind);
                if let Some(data) = profile_chunk(chunk.data, self.colour) {
                    self.kept.push((chunk.kind, data));
                }
            }
            b"sRGB" | b"gAMA" | b"cHRM" if colour_place && !has(&chunk.kind) => {
                if fixed_colour_chunk_fits(chunk.kind, chunk.data) {
                    self.kept.push((chunk.kind, chunk.data.to_vec()));
                }
            }
            b"cICP" | b"mDCV" | b"cLLI"
                if colour_place
                    && !self.claimed.contains(&chunk.kind)
                    && Some(chunk.data.len()) == hdr_chunk_len(chunk.kind) =>
            {
                self.claimed.push(chunk.kind);
                if hdr_chunk_fits(chunk.kind, chunk.data) {
                    self.kept.push((chunk.kind, chunk.data.to_vec()));
                }
            }
            b"eXIf" if before && !self.claimed.contains(&chunk.kind) => {
                self.claimed.push(chunk.kind);
                // The TIFF structure as ADR 0033 §3 reads it, replaced by
                // one that holds the orientation alone and is never longer.
                if let Some(orientation) = Orientation::from_tiff(chunk.data) {
                    let reduced = orientation.tiff_replacing(chunk.data.len());
                    self.kept.push((chunk.kind, reduced));
                }
            }
            b"acTL" if before => self.frames.announce(Some(chunk.data)),
            b"fcTL" => self.frames.control(Some(chunk.data), before),
            b"fdAT" => self.frames.data(Some(chunk.data), before, follows_fdat),
            _ => {}
        }
        Ok(true)
    }

    /// Takes a `PLTE` chunk.
    ///
    /// - `data`: its data.
    fn take_palette(&mut self, data: &'a [u8]) -> Result<(), DecodeDefect> {
        match self.colour {
            // libpng ignores a palette in a greyscale image.
            0 | 4 => {}
            3 if self.palette.is_some() => return Err(DecodeDefect::PngChunkOrder),
            // After the data: the data had no palette, refused in `finish`.
            3 if self.run != Run::Before => {}
            3 => {
                self.palette =
                    Some(palette_entries(data, self.depth).ok_or(DecodeDefect::PngPalette)?);
                self.placed = true;
            }
            // A truecolour image's suggested palette does not change its
            // pixels and is dropped, as oxipng drops it. libpng takes it, and
            // then ignores the colour chunks after it, only when it comes
            // before the data, is the first it takes, holds whole entries,
            // at most 256 of them, and follows no tRNS or bKGD that libpng
            // took (png_handle_PLTE); otherwise it ignores the palette. An
            // empty one passes those checks, and libpng then stops at it
            // ("Invalid palette", png_set_PLTE).
            _ => {
                let taken = self.run == Run::Before
                    && !self.placed
                    && data.len().is_multiple_of(3)
                    && data.len() <= 3 * 256
                    && self.transparency.is_none()
                    && !self.background;
                if taken && data.is_empty() {
                    return Err(DecodeDefect::PngPalette);
                }
                self.placed |= taken;
            }
        }
        Ok(())
    }

    /// Checks the animation and the image data, and writes the rewrite.
    fn finish(self) -> Result<Rewrite, Refusal> {
        if self.colour == 3 && self.palette.is_none() {
            return Err(DecodeDefect::PngPalette.into());
        }
        // The animation's totals first, from its frame controls alone, so an
        // animation above them is refused before any image data is inflated,
        // the default image's included (ADR 0033 §4). Only the first iCCP
        // profile has been inflated by now, during the walk, and at most
        // MAX_PROFILE bytes of it.
        let animation = self.frames.finish().map_err(Refusal::AnimationTooLarge)?;
        // Then the image data: a file whose default image does not decode is
        // refused whatever its animation holds, unless the totals above have
        // already refused the animation.
        let rows = image_rows(self.ihdr).ok_or(DecodeDefect::PngImageData)?;
        let stream = zlib::check_rows(&self.data, &rows).ok_or(DecodeDefect::PngImageData)?;
        // Last, each frame's data, up to the first frame that does not
        // decode, which is dropped with every later frame; with no frame
        // left, the still image is stored.
        let animation = animation.and_then(|animation| animation.checked(self.ihdr));
        let canvas = u64::from(u32::from_be_bytes([
            self.ihdr[0],
            self.ihdr[1],
            self.ihdr[2],
            self.ihdr[3],
        ])) * u64::from(u32::from_be_bytes([
            self.ihdr[4],
            self.ihdr[5],
            self.ihdr[6],
            self.ihdr[7],
        ]));
        let wide_transparency = self.depth == 16
            && self
                .transparency
                .as_ref()
                .is_some_and(|trns| trns.chunks(2).any(|sample| sample[0] != sample[1]));

        let mut out = SIGNATURE.to_vec();
        write_chunk(&mut out, *b"IHDR", self.ihdr);
        for (kind, data) in &self.kept {
            write_chunk(&mut out, *kind, data);
        }
        if let Some(animation) = &animation {
            animation.write_control(&mut out);
        }
        if let Some(palette) = self.palette {
            write_chunk(&mut out, *b"PLTE", palette);
        }
        if let Some(trns) = &self.transparency {
            write_chunk(&mut out, *b"tRNS", trns);
        }
        if let Some(animation) = &animation {
            animation.write_first(&mut out);
        }
        for piece in self.data[..stream].chunks(MAX_CHUNK) {
            write_chunk(&mut out, *b"IDAT", piece);
        }
        if let Some(animation) = &animation {
            animation.write_frames(&mut out);
        }
        write_chunk(&mut out, *b"IEND", &[]);
        Ok(Rewrite {
            png: out,
            wide_transparency,
            coded_colour: self.kept.iter().any(|(kind, _)| kind == b"cICP"),
            pixels: animation.map_or(canvas, |animation| animation.pixels()),
        })
    }
}

/// The chunk at `at`, and where the next one starts.
///
/// - `bytes`: the whole input.
/// - `at`: the offset of the chunk's length field.
fn read_chunk(bytes: &[u8], at: usize) -> Result<(Chunk<'_>, usize), DecodeDefect> {
    let header = bytes.get(at..at + 8).ok_or(DecodeDefect::PngTruncated)?;
    let len = usize::try_from(u32::from_be_bytes([
        header[0], header[1], header[2], header[3],
    ]))
    .map_err(|_| DecodeDefect::PngChunk)?;
    let kind = [header[4], header[5], header[6], header[7]];
    if len > MAX_CHUNK || !kind.iter().all(u8::is_ascii_alphabetic) {
        return Err(DecodeDefect::PngChunk);
    }
    let end = at + 8 + len;
    let crc = bytes.get(end..end + 4).ok_or(DecodeDefect::PngTruncated)?;
    let intact = crc32fast::hash(&bytes[at + 4..end]).to_be_bytes() == crc;
    let chunk = Chunk {
        kind,
        data: &bytes[at + 8..end],
        intact,
    };
    Ok((chunk, end + 4))
}

/// Appends a chunk of `kind` with `data` and its CRC to `out`.
///
/// - `out`: the PNG being written.
/// - `kind`: the chunk type.
/// - `data`: the chunk data, at most [`MAX_CHUNK`] bytes.
pub(super) fn write_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).expect("chunk data stays within 2^31 - 1 bytes");
    let mut crc = crc32fast::Hasher::new();
    crc.update(&kind);
    crc.update(data);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// Whether an `sRGB`, `gAMA` or `cHRM` chunk is one libpng takes, which
/// also makes it the one libpng holds a later chunk of its type against.
///
/// - `kind`: `sRGB`, `gAMA` or `cHRM`.
/// - `data`: the chunk data.
///
/// The three have fixed lengths (1, 4 and 32 bytes), so any other length is
/// malformed, and also could not carry extra bytes. libpng ignores an sRGB
/// rendering intent above 3, but takes any 4-byte gamma, zero included, and
/// ignores the gAMA chunks after it; the first is therefore kept as it is.
fn fixed_colour_chunk_fits(kind: [u8; 4], data: &[u8]) -> bool {
    match &kind {
        b"sRGB" => data.len() == 1 && data[0] <= 3,
        b"gAMA" => data.len() == 4,
        b"cHRM" => data.len() == 32,
        _ => false,
    }
}

/// The length of an HDR colour chunk: 4 bytes for `cICP`, 24 for `mDCV` and
/// 8 for `cLLI`; `None` for any other type. A chunk of another length is
/// malformed and does not take the type's one place.
///
/// - `kind`: the chunk type.
fn hdr_chunk_len(kind: [u8; 4]) -> Option<usize> {
    match &kind {
        b"cICP" => Some(4),
        b"mDCV" => Some(24),
        b"cLLI" => Some(8),
        _ => None,
    }
}

/// Whether an HDR colour chunk of the right length holds values that the
/// reference decoders take, so it is kept.
///
/// - `kind`: `cICP`, `mDCV` or `cLLI`.
/// - `data`: the chunk data, of [`hdr_chunk_len`] bytes.
///
/// A `cICP` needs matrix coefficients 0, since PNG holds RGB samples only
/// (libpng's `png_set_cICP` and the `png` crate both refuse others), and a
/// full-range flag of 0 or 1 (the `png` crate refuses the whole file for
/// another value, so it is dropped although libpng takes it). An `mDCV` or a
/// `cLLI` needs light levels of at most 2³¹ − 1, as libpng's setters check;
/// the chromaticities of `mDCV` are 16-bit and always in range.
fn hdr_chunk_fits(kind: [u8; 4], data: &[u8]) -> bool {
    let words_fit = |from: usize| {
        data[from..]
            .chunks(4)
            .all(|word| u32::from_be_bytes([word[0], word[1], word[2], word[3]]) <= MAX_LIGHT_LEVEL)
    };
    match &kind {
        b"cICP" => data.len() == 4 && data[2] == 0 && data[3] <= 1,
        b"mDCV" => data.len() == 24 && words_fit(16),
        b"cLLI" => data.len() == 8 && words_fit(0),
        _ => false,
    }
}

/// The data of an `iCCP` chunk as it is kept, or `None` when libpng would
/// ignore the profile, so it is dropped.
///
/// - `data`: the chunk data: a profile name of 1-79 bytes, a NUL,
///   compression method 0 and a zlib stream. It is kept cut at the end of
///   that stream.
/// - `colour`: the colour type of `IHDR`, which decides the colour space the
///   profile must have.
fn profile_chunk(data: &[u8], colour: u8) -> Option<Vec<u8>> {
    let name = data.iter().position(|&byte| byte == 0)?;
    if !(1..=79).contains(&name) || data.get(name + 1) != Some(&0) {
        return None;
    }
    let start = name + 2;
    let (stream, profile) = zlib::inflate(&data[start..], MAX_PROFILE)?;
    profile_fits(&profile, colour).then(|| data[..start + stream.len].to_vec())
}

/// Whether `profile` is an ICC profile that libpng 1.6.58 accepts for an
/// image of colour type `colour`. These are the checks libpng's
/// `png_icc_check_length`, `png_icc_check_header` and
/// `png_icc_check_tag_table` reject a profile for; the ones it only warns
/// about are left out.
///
/// - `profile`: the decoded profile.
/// - `colour`: the colour type of `IHDR`.
fn profile_fits(profile: &[u8], colour: u8) -> bool {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(
            profile.get(at..at + 4)?.try_into().ok()?,
        ))
    };
    let field = |at: usize| profile.get(at..at + 4);
    let Ok(len) = u32::try_from(profile.len()) else {
        return false;
    };
    let Some(tags) = word(128) else {
        return false;
    };
    // PNG requires an RGB profile for colour types 2, 3 and 6, and a grey
    // one for 0 and 4 (PNG 3rd edition, §11.3.2.3).
    let space: &[u8] = if colour & 2 == 0 { b"GRAY" } else { b"RGB " };
    let header = profile.len() >= PROFILE_HEADER
        && u64::from(len) <= MAX_PROFILE
        && word(0) == Some(len)
        // Version 4 profiles are padded to four bytes.
        && (profile[8] <= 3 || len % 4 == 0)
        && u64::from(len) >= 132 + 12 * u64::from(tags)
        && word(64).is_some_and(|intent| intent < 0xFFFF)
        && field(36) == Some(b"acsp")
        && field(16) == Some(space)
        && !matches!(field(12), Some(b"abst" | b"link"))
        && matches!(field(20), Some(b"XYZ " | b"Lab "));
    // Every tag lies inside the profile. The header check above has already
    // bounded the tag table by the profile's length.
    let tag_fits = |index: u32| -> Option<bool> {
        let entry = usize::try_from(index)
            .ok()?
            .checked_mul(12)?
            .checked_add(PROFILE_HEADER)?;
        let (start, size) = (word(entry + 4)?, word(entry + 8)?);
        Some(start <= len && size <= len - start)
    };
    header && (0..tags).all(|index| tag_fits(index) == Some(true))
}

/// The `tRNS` data as it is kept, or `None` when libpng ignores the chunk.
///
/// - `data`: the chunk data.
/// - `colour`: the colour type of `IHDR`.
/// - `depth`: its bit depth.
/// - `palette`: the palette read so far, cut to what `depth` can index.
///
/// Greyscale and truecolour need one 2-byte sample per channel. Below 16
/// bits only the low bits count (PNG 3rd edition, §11.3.1.1), so the
/// samples are masked to them. An indexed image needs 1 to as many entries
/// as its palette has: libpng ignores a longer `tRNS` whole, rather than
/// cutting it.
fn valid_transparency(
    data: &[u8],
    colour: u8,
    depth: u8,
    palette: Option<&[u8]>,
) -> Option<Vec<u8>> {
    match colour {
        0 | 2 if data.len() == if colour == 0 { 2 } else { 6 } => {
            let mask = if depth >= 16 {
                u16::MAX
            } else {
                (1_u16 << depth) - 1
            };
            Some(
                data.chunks(2)
                    .flat_map(|sample| {
                        (u16::from_be_bytes([sample[0], sample[1]]) & mask).to_be_bytes()
                    })
                    .collect(),
            )
        }
        3 => {
            let entries = palette?.len() / 3;
            (1..=entries).contains(&data.len()).then(|| data.to_vec())
        }
        _ => None,
    }
}

/// The palette an indexed image can use: at most 2^`depth` entries of the
/// `PLTE` data, or `None` when it is not a valid palette (1 to 256 entries
/// of three bytes), which libpng refuses.
///
/// - `palette`: the `PLTE` data.
/// - `depth`: the bit depth, 1, 2, 4 or 8.
fn palette_entries(palette: &[u8], depth: u8) -> Option<&[u8]> {
    if palette.is_empty() || !palette.len().is_multiple_of(3) || palette.len() > 3 * 256 {
        return None;
    }
    // Pixels cannot index past 2^depth entries, so libpng discards those.
    let usable = 3_usize << depth.min(8);
    Some(&palette[..palette.len().min(usable)])
}

/// The rows the image data of `ihdr` decodes to, pass by pass, each with its
/// filter-type byte: one pass for a still image, the passes of Adam7 that
/// hold pixels for an interlaced one. `None` when the size does not fit 64
/// bits, which no decoder could allocate.
///
/// - `ihdr`: the 13 bytes of `IHDR`, or of a frame of an animation (its
///   width and height, then the canvas's bit depth, colour type and
///   interlace method), whose fields the header reader checked.
pub(super) fn image_rows(ihdr: &[u8]) -> Option<Vec<Pass>> {
    let width = u64::from(u32::from_be_bytes(ihdr.get(0..4)?.try_into().ok()?));
    let height = u64::from(u32::from_be_bytes(ihdr.get(4..8)?.try_into().ok()?));
    let channels: u64 = match ihdr.get(9)? {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    let bits = u64::from(*ihdr.get(8)?) * channels;
    // A pass of `width × height` pixels, or none when it holds no pixel.
    let pass = |width: u64, height: u64| -> Option<Option<Pass>> {
        if width == 0 || height == 0 {
            return Some(None);
        }
        let row = width.checked_mul(bits)?.div_ceil(8).checked_add(1)?;
        row.checked_mul(height)?;
        Some(Some(Pass { row, rows: height }))
    };
    let passes = if *ihdr.get(12)? == 0 {
        vec![pass(width, height)?]
    } else {
        ADAM7
            .iter()
            .map(|&(x, y, dx, dy)| {
                pass(
                    width.saturating_sub(x).div_ceil(dx),
                    height.saturating_sub(y).div_ceil(dy),
                )
            })
            .collect::<Option<Vec<_>>>()?
    };
    let passes: Vec<Pass> = passes.into_iter().flatten().collect();
    // The whole stream must fit 64 bits too.
    passes
        .iter()
        .try_fold(0_u64, |sum, pass| sum.checked_add(pass.row * pass.rows))?;
    Some(passes)
}

/// Whether libpng 1.6.58 takes a `bKGD` chunk, read before the data, of a
/// truecolour image (colour type 2 or 6): 6 bytes, and below 16 bits a zero
/// high byte in each sample (`png_handle_bKGD`). For other colour types the
/// answer does not matter, since only a truecolour image's suggested palette
/// gives way to it, and it is `false`.
///
/// - `data`: the chunk data.
/// - `colour`: the colour type of `IHDR`.
/// - `depth`: its bit depth.
fn background_taken(data: &[u8], colour: u8, depth: u8) -> bool {
    matches!(colour, 2 | 6)
        && data.len() == 6
        && (depth == 16 || (data[0] == 0 && data[2] == 0 && data[4] == 0))
}

#[cfg(test)]
mod tests {
    use super::super::apng::samples::{actl, fctl, fdat};
    use super::super::samples::{camera, exif_little_endian};
    use super::super::samples::{chunk, icc, ihdr, image, pixels, profile, rgb};
    use super::*;
    use crate::domain::image_optimise::png::zlib::samples::stored;
    use miniz_oxide::deflate::compress_to_vec_zlib;
    use proptest::{collection::vec, prelude::*};

    /// The chunk types of a PNG, in order.
    fn kinds(png: &[u8]) -> Vec<String> {
        let mut at = SIGNATURE.len();
        let mut found = Vec::new();
        while let Ok((chunk, next)) = read_chunk(png, at) {
            found.push(String::from_utf8_lossy(&chunk.kind).into_owned());
            at = next;
        }
        found
    }

    /// The data of the first chunk of `kind` in a PNG.
    fn data_of(png: &[u8], kind: &[u8]) -> Option<Vec<u8>> {
        let mut at = SIGNATURE.len();
        while let Ok((chunk, next)) = read_chunk(png, at) {
            if chunk.kind[..] == *kind {
                return Some(chunk.data.to_vec());
            }
            at = next;
        }
        None
    }

    /// The rewritten PNG of `input`, which must be accepted.
    fn rewrite(input: &[u8]) -> Vec<u8> {
        canonical(input).unwrap().png
    }

    /// A 2×2 1-bit indexed PNG with the chunks `before` its data.
    fn indexed(before: &[Vec<u8>]) -> Vec<u8> {
        // Two rows of a filter byte and one byte of 1-bit indices.
        image(&ihdr(2, 2, 1, 3, 0), before, &[stored(&[0; 4])], &[])
    }

    /// The rows of the image data of `ihdr`, as (row length, rows).
    fn rows_of(ihdr: &[u8]) -> Option<Vec<(u64, u64)>> {
        image_rows(&ihdr[8..21]).map(|passes| passes.iter().map(|p| (p.row, p.rows)).collect())
    }

    #[test]
    fn image_rows_count_every_row_and_pass() {
        // 67x41 RGB: 41 rows of 1 + 201 bytes.
        assert_eq!(rows_of(&ihdr(67, 41, 8, 2, 0)), Some(vec![(202, 41)]));
        // 1-bit greyscale rounds each row up to a byte.
        assert_eq!(rows_of(&ihdr(9, 2, 1, 0, 0)), Some(vec![(3, 2)]));
        // Adam7 1x1: only the first pass has a pixel.
        assert_eq!(rows_of(&ihdr(1, 1, 8, 6, 1)), Some(vec![(5, 1)]));
        // Adam7 8x8 RGBA16: the passes are 1x1, 1x1, 2x1, 2x2, 4x2, 4x4, 8x4.
        let passes = [(1, 1), (1, 1), (2, 1), (2, 2), (4, 2), (4, 4), (8, 4)];
        let expected = passes.iter().map(|&(w, h)| (1 + 8 * w, h)).collect();
        assert_eq!(rows_of(&ihdr(8, 8, 16, 6, 1)), Some(expected));
        // Adam7 3x1: the passes without pixels are left out.
        assert_eq!(
            rows_of(&ihdr(3, 1, 8, 0, 1)),
            Some(vec![(2, 1), (2, 1), (2, 1)])
        );
        // Too large for 64 bits.
        let huge = ihdr(0x7FFF_FFFF, 0x7FFF_FFFF, 16, 6, 0);
        assert_eq!(rows_of(&huge), None);
        // An unknown colour type has no rows.
        assert_eq!(rows_of(&ihdr(1, 1, 8, 5, 0)), None);
    }

    #[test]
    fn the_allow_list_is_all_that_is_kept() {
        let input = rgb(
            &[
                chunk(b"tEXt", b"Comment\0backend3 test corpus"),
                chunk(b"sRGB", &[0]),
                chunk(b"gAMA", &45_455_u32.to_be_bytes()),
                chunk(b"cHRM", &[7; 32]),
                chunk(b"iCCP", &icc(b"Display", &profile(*b"RGB "))),
                chunk(b"eXIf", &camera()),
                chunk(b"pHYs", &[0, 0, 11, 19, 0, 0, 11, 19, 1]),
                chunk(b"tIME", &[7, 234, 10, 25, 12, 0, 0]),
                chunk(b"cICP", &[1, 13, 0, 1]),
                chunk(b"prVt", b"private data"),
            ],
            &[
                chunk(b"zTXt", b"Author\0\0x"),
                chunk(b"bKGD", &[0, 0, 0, 0, 0, 0]),
            ],
        );
        let out = canonical(&input).unwrap();
        assert_eq!(
            kinds(&out.png),
            [
                "IHDR", "sRGB", "gAMA", "cHRM", "iCCP", "eXIf", "cICP", "IDAT", "IEND"
            ]
        );
        assert_eq!(data_of(&out.png, b"cICP"), Some(vec![1, 13, 0, 1]));
        assert!(out.coded_colour);
        assert_eq!(out.pixels, 12);
        // The kept chunks keep their data; eXIf is reduced to Orientation.
        assert_eq!(
            data_of(&out.png, b"gAMA"),
            Some(45_455_u32.to_be_bytes().to_vec())
        );
        assert_eq!(data_of(&out.png, b"cHRM"), Some(vec![7; 32]));
        assert_eq!(
            data_of(&out.png, b"eXIf"),
            Orientation::from_tiff(&camera()).map(|o| o.tiff_replacing(camera().len()))
        );
        assert!(out.png.len() < input.len());
        assert!(!out.wide_transparency);
    }

    #[test]
    fn everything_after_iend_and_after_the_streams_is_cut() {
        let plain = rgb(&[], &[]);
        let with_tail = [&plain[..], b"PK\x03\x04<html>payload"].concat();
        assert_eq!(canonical(&with_tail), canonical(&plain));
        assert_eq!(rewrite(&plain), plain);

        // Bytes after the zlib stream, inside the image data and the profile.
        let tail = b"payload";
        let rgb_profile = icc(b"p", &profile(*b"RGB "));
        let padded = image(
            &ihdr(4, 3, 8, 2, 0),
            &[chunk(b"iCCP", &[&rgb_profile[..], tail].concat())],
            &[[&stored(&pixels(4, 3, 3))[..], tail].concat()],
            &[],
        );
        let clean = image(
            &ihdr(4, 3, 8, 2, 0),
            &[chunk(b"iCCP", &rgb_profile)],
            &[stored(&pixels(4, 3, 3))],
            &[],
        );
        assert_eq!(rewrite(&padded), clean);
    }

    #[test]
    fn colour_chunks_after_plte_or_the_data_or_repeated_are_dropped() {
        // In an indexed image, the colour chunks must come before PLTE.
        let input = indexed(&[
            chunk(b"sRGB", &[4]),
            chunk(b"cHRM", &[1; 31]),
            chunk(b"gAMA", &[0; 3]),
            // The first gAMA of the right length counts, even a zero one,
            // as in libpng; the next is a duplicate.
            chunk(b"gAMA", &[0; 4]),
            chunk(b"gAMA", &2_u32.to_be_bytes()),
            chunk(b"PLTE", &[0; 6]),
            chunk(b"sRGB", &[0]),
            chunk(b"cHRM", &[1; 32]),
        ]);
        let out = rewrite(&input);
        assert_eq!(kinds(&out), ["IHDR", "gAMA", "PLTE", "IDAT", "IEND"]);
        assert_eq!(data_of(&out, b"gAMA"), Some(vec![0; 4]));
        // After the data, in a truecolour image.
        let late = rgb(
            &[],
            &[
                chunk(b"sRGB", &[0]),
                chunk(b"gAMA", &[0, 0, 1, 0]),
                chunk(b"cHRM", &[1; 32]),
                chunk(b"iCCP", &icc(b"p", &profile(*b"RGB "))),
            ],
        );
        assert_eq!(kinds(&rewrite(&late)), ["IHDR", "IDAT", "IEND"]);
        // After a truecolour image's suggested palette; a greyscale image's
        // palette is ignored and does not count.
        let suggested = rgb(&[chunk(b"PLTE", &[9; 6]), chunk(b"sRGB", &[0])], &[]);
        assert_eq!(kinds(&rewrite(&suggested)), ["IHDR", "IDAT", "IEND"]);
        let grey = image(
            &ihdr(2, 1, 8, 0, 0),
            &[chunk(b"PLTE", &[9; 6]), chunk(b"sRGB", &[0])],
            &[stored(&[0, 1, 2])],
            &[],
        );
        assert_eq!(kinds(&rewrite(&grey)), ["IHDR", "sRGB", "IDAT", "IEND"]);
        // A second sRGB after a valid one.
        let twice = rgb(&[chunk(b"sRGB", &[0]), chunk(b"sRGB", &[1])], &[]);
        assert_eq!(data_of(&rewrite(&twice), b"sRGB"), Some(vec![0]));
    }

    #[test]
    fn only_the_first_profile_is_read_and_only_a_valid_one_kept() {
        let good = chunk(b"iCCP", &icc(b"p", &profile(*b"RGB ")));
        // A non-profile payload, which png decoders would hand on as a
        // profile; then a valid profile, which is never inflated.
        let payload = chunk(
            b"iCCP",
            &icc(b"p", &b"<script>alert(1)</script>".repeat(50)),
        );
        let out = rewrite(&rgb(&[payload, good.clone()], &[]));
        assert_eq!(kinds(&out), ["IHDR", "IDAT", "IEND"]);
        // A profile decoding to nothing, a wrong name or method.
        for broken in [
            icc(b"p", b""),
            icc(b"", &profile(*b"RGB ")),
            icc(&[b'n'; 80], &profile(*b"RGB ")),
            [&b"p\0\x01"[..], &stored(&profile(*b"RGB "))].concat(),
        ] {
            let out = rewrite(&rgb(&[chunk(b"iCCP", &broken)], &[]));
            assert_eq!(data_of(&out, b"iCCP"), None, "{broken:?}");
        }
        // Many bombs: only the first is inflated, so the rest cost a CRC.
        let bomb = chunk(
            b"iCCP",
            &[&b"p\0\0"[..], &compress_to_vec_zlib(&vec![0; 9_000_000], 9)].concat(),
        );
        let bombs: Vec<Vec<u8>> = std::iter::repeat_n(bomb, 64).chain([good]).collect();
        assert_eq!(kinds(&rewrite(&rgb(&bombs, &[]))), ["IHDR", "IDAT", "IEND"]);
    }

    #[test]
    fn a_profile_must_pass_libpngs_checks_for_the_colour_type() {
        let rgb_profile = profile(*b"RGB ");
        let grey_profile = profile(*b"GRAY");
        assert!(profile_fits(&rgb_profile, 2));
        assert!(profile_fits(&rgb_profile, 3));
        assert!(profile_fits(&rgb_profile, 6));
        assert!(profile_fits(&grey_profile, 0));
        assert!(profile_fits(&grey_profile, 4));
        assert!(!profile_fits(&grey_profile, 2));
        assert!(!profile_fits(&rgb_profile, 0));
        let changed = |at: usize, bytes: &[u8]| {
            let mut changed = rgb_profile.clone();
            changed[at..at + bytes.len()].copy_from_slice(bytes);
            changed
        };
        for (what, broken) in [
            ("size", changed(0, &145_u32.to_be_bytes())),
            ("signature", changed(36, b"xxxx")),
            ("colour space", changed(16, b"CMYK")),
            ("class", changed(12, b"link")),
            ("class", changed(12, b"abst")),
            ("connection space", changed(20, b"RGB ")),
            ("intent", changed(64, &0xFFFF_u32.to_be_bytes())),
            ("tag count", changed(128, &2_u32.to_be_bytes())),
            ("tag start", changed(136, &145_u32.to_be_bytes())),
            ("tag size", changed(140, &13_u32.to_be_bytes())),
            ("version 4 padding", {
                let mut v4 = [&changed(8, &[4])[..], &[0]].concat();
                v4[0..4].copy_from_slice(&145_u32.to_be_bytes());
                v4
            }),
            ("too short", rgb_profile[..131].to_vec()),
        ] {
            assert!(!profile_fits(&broken, 2), "{what}");
        }
        // A version 2 profile need not be padded.
        let mut odd = [&rgb_profile[..], &[0]].concat();
        odd[0..4].copy_from_slice(&145_u32.to_be_bytes());
        assert!(profile_fits(&odd, 2));
    }

    #[test]
    fn a_profile_above_libpngs_limit_is_dropped() {
        let mut big = profile(*b"RGB ");
        let len = u32::try_from(MAX_PROFILE).unwrap() + 4;
        big.resize(usize::try_from(len).unwrap(), 0);
        big[0..4].copy_from_slice(&len.to_be_bytes());
        let mut fits = big.clone();
        fits.truncate(usize::try_from(MAX_PROFILE).unwrap());
        fits[0..4].copy_from_slice(&u32::try_from(MAX_PROFILE).unwrap().to_be_bytes());
        let data = |profile: &[u8]| [&b"p\0\0"[..], &compress_to_vec_zlib(profile, 1)].concat();
        assert_eq!(profile_chunk(&data(&big), 2), None);
        assert!(profile_chunk(&data(&fits), 2).is_some());
    }

    #[test]
    fn a_bad_ancillary_crc_drops_the_chunk_and_a_bad_critical_one_refuses() {
        let mut gama = chunk(b"gAMA", &45_455_u32.to_be_bytes());
        *gama.last_mut().unwrap() ^= 1;
        assert_eq!(canonical(&rgb(&[gama], &[])), canonical(&rgb(&[], &[])));
        let mut png = rgb(&[], &[]);
        let crc_of_idat = png.len() - 12 - 1;
        png[crc_of_idat] ^= 1;
        assert_eq!(canonical(&png), Err(DecodeDefect::PngChecksum.into()));
    }

    #[test]
    fn transparency_is_kept_only_where_libpng_uses_it() {
        // A 1-bit image indexes 2 entries: a palette of 4 is cut to 2, and a
        // tRNS longer than that is ignored whole, so the pixels stay opaque.
        for trns in [&[1, 2, 3][..], &[0, 0, 0]] {
            let out = rewrite(&indexed(&[chunk(b"PLTE", &[9; 12]), chunk(b"tRNS", trns)]));
            assert_eq!(data_of(&out, b"PLTE"), Some(vec![9; 6]));
            assert_eq!(data_of(&out, b"tRNS"), None, "{trns:?}");
        }
        let out = rewrite(&indexed(&[
            chunk(b"PLTE", &[9; 6]),
            chunk(b"tRNS", &[1, 2]),
        ]));
        assert_eq!(data_of(&out, b"tRNS"), Some(vec![1, 2]));
        // Before PLTE, empty, after the data, or a second one after a valid
        // one: dropped. An invalid first one does not hide a valid second.
        let before = indexed(&[chunk(b"tRNS", &[1]), chunk(b"PLTE", &[9; 6])]);
        assert_eq!(data_of(&rewrite(&before), b"tRNS"), None);
        let empty = indexed(&[chunk(b"PLTE", &[9; 6]), chunk(b"tRNS", &[])]);
        assert_eq!(data_of(&rewrite(&empty), b"tRNS"), None);
        let after = rgb(&[], &[chunk(b"tRNS", &[0; 6])]);
        assert_eq!(data_of(&rewrite(&after), b"tRNS"), None);
        let grey =
            |before: &[Vec<u8>]| image(&ihdr(2, 1, 8, 0, 0), before, &[stored(&[0, 1, 2])], &[]);
        let twice = grey(&[chunk(b"tRNS", &[0, 1]), chunk(b"tRNS", &[0, 2])]);
        assert_eq!(data_of(&rewrite(&twice), b"tRNS"), Some(vec![0, 1]));
        let second = grey(&[chunk(b"tRNS", &[1]), chunk(b"tRNS", &[0, 2])]);
        assert_eq!(data_of(&rewrite(&second), b"tRNS"), Some(vec![0, 2]));
        // The wrong length for truecolour, and any tRNS with an alpha channel.
        let out = rewrite(&rgb(&[chunk(b"tRNS", &[0; 4])], &[]));
        assert_eq!(data_of(&out, b"tRNS"), None);
        let rgba = image(
            &ihdr(1, 1, 8, 6, 0),
            &[chunk(b"tRNS", &[0; 6])],
            &[stored(&[0; 5])],
            &[],
        );
        assert_eq!(data_of(&rewrite(&rgba), b"tRNS"), None);
        // A suggested palette of a truecolour image is dropped.
        let out = rewrite(&rgb(&[chunk(b"PLTE", &[9; 6])], &[]));
        assert_eq!(kinds(&out), ["IHDR", "IDAT", "IEND"]);
    }

    #[test]
    fn transparent_samples_are_masked_to_the_bit_depth() {
        // 2-bit grey: 0x0107 is 3 once masked, as PNG requires of decoders.
        let grey = image(
            &ihdr(4, 1, 2, 0, 0),
            &[chunk(b"tRNS", &[1, 7])],
            &[stored(&[0, 0x1B])],
            &[],
        );
        assert_eq!(data_of(&rewrite(&grey), b"tRNS"), Some(vec![0, 3]));
        // 8-bit truecolour keeps the low byte of each sample.
        let out = rewrite(&rgb(&[chunk(b"tRNS", &[1, 2, 3, 4, 5, 6])], &[]));
        assert_eq!(data_of(&out, b"tRNS"), Some(vec![0, 2, 0, 4, 0, 6]));
    }

    #[test]
    fn a_wide_transparent_colour_is_reported_for_16_bit_images() {
        let grey16 = |trns: &[u8]| {
            let data = stored(&[0, 0x12, 0x12, 0x34, 0x34]);
            canonical(&image(
                &ihdr(2, 1, 16, 0, 0),
                &[chunk(b"tRNS", trns)],
                &[data],
                &[],
            ))
            .unwrap()
        };
        assert!(grey16(&[0, 0x34]).wide_transparency);
        assert!(!grey16(&[0x34, 0x34]).wide_transparency);
        let rgb16 = |trns: &[u8]| {
            let data = stored(&[0; 7]);
            canonical(&image(
                &ihdr(1, 1, 16, 2, 0),
                &[chunk(b"tRNS", trns)],
                &[data],
                &[],
            ))
            .unwrap()
        };
        assert!(rgb16(&[1, 1, 2, 2, 3, 4]).wide_transparency);
        assert!(!rgb16(&[1, 1, 2, 2, 3, 3]).wide_transparency);
        // No transparency, or 8 bits: nothing to report.
        assert!(
            !canonical(&rgb(&[chunk(b"tRNS", &[0, 1, 0, 2, 0, 3])], &[]))
                .unwrap()
                .wide_transparency
        );
    }

    #[test]
    fn exif_is_read_once_and_only_before_the_data() {
        let orientation = Orientation::from_tiff(&camera())
            .unwrap()
            .tiff_replacing(camera().len());
        // After the data, where PNG does not allow it.
        let late = rgb(&[], &[chunk(b"eXIf", &camera())]);
        assert_eq!(data_of(&rewrite(&late), b"eXIf"), None);
        // A second one, even when the first holds no Orientation.
        let empty = exif_little_endian(8, &[]);
        let twice = rgb(&[chunk(b"eXIf", &empty), chunk(b"eXIf", &camera())], &[]);
        assert_eq!(data_of(&rewrite(&twice), b"eXIf"), None);
        let kept = rgb(&[chunk(b"eXIf", &camera()), chunk(b"eXIf", &empty)], &[]);
        assert_eq!(data_of(&rewrite(&kept), b"eXIf"), Some(orientation));
        // The JPEG identifier in front of the TIFF structure: libpng refuses
        // such a chunk, and the reader of ADR 0033 §3 finds no TIFF header.
        let prefixed = rgb(
            &[chunk(b"eXIf", &[&b"Exif\0\0"[..], &camera()].concat())],
            &[],
        );
        assert_eq!(data_of(&rewrite(&prefixed), b"eXIf"), None);
        // A structure shorter than the 26-byte replacement gets its 22-byte
        // form, so the chunk never grows.
        let short = exif_little_endian(8, &[(0x0112, 3, 1, 3)]);
        let short = &short[..22];
        let out = rewrite(&rgb(&[chunk(b"eXIf", short)], &[]));
        assert_eq!(data_of(&out, b"eXIf").map(|data| data.len()), Some(22));
    }

    #[test]
    fn each_refusal_names_its_defect() {
        let plain = rgb(&[], &[]);
        let idat = || stored(&pixels(4, 3, 3));
        let rgb_with = |before: &[Vec<u8>], data: &[Vec<u8>], after: &[Vec<u8>]| {
            image(&ihdr(4, 3, 8, 2, 0), before, data, after)
        };
        let mut bad_adler = idat();
        *bad_adler.last_mut().unwrap() ^= 1;
        let whole = idat();
        let (head, tail) = whole.split_at(10);
        let mut bad_text = chunk(b"tEXt", b"a\0b");
        *bad_text.last_mut().unwrap() ^= 1;
        let cases = [
            (
                plain[..plain.len() - 12].to_vec(),
                DecodeDefect::PngTruncated,
            ),
            (
                plain[..plain.len() - 1].to_vec(),
                DecodeDefect::PngTruncated,
            ),
            (
                rgb(&[b"\xFF\0\0\0gAMA".to_vec()], &[]),
                DecodeDefect::PngChunk,
            ),
            (rgb(&[chunk(b"gA1A", &[0; 4])], &[]), DecodeDefect::PngChunk),
            (
                rgb(&[chunk(b"CgBI", &[0; 4])], &[]),
                DecodeDefect::PngUnknownCriticalChunk,
            ),
            (
                rgb(&[ihdr(4, 3, 8, 2, 0)], &[]),
                DecodeDefect::PngChunkOrder,
            ),
            (
                indexed(&[chunk(b"PLTE", &[0; 3]), chunk(b"PLTE", &[0; 3])]),
                DecodeDefect::PngChunkOrder,
            ),
            (indexed(&[]), DecodeDefect::PngPalette),
            (
                indexed(&[chunk(b"PLTE", &[0; 4])]),
                DecodeDefect::PngPalette,
            ),
            (
                indexed(&[chunk(b"PLTE", &[0; 3 * 257])]),
                DecodeDefect::PngPalette,
            ),
            (
                image(
                    &ihdr(2, 2, 1, 3, 0),
                    &[],
                    &[stored(&[0; 4])],
                    &[chunk(b"PLTE", &[0; 3])],
                ),
                DecodeDefect::PngPalette,
            ),
            (rgb_with(&[], &[], &[]), DecodeDefect::PngImageData),
            (
                rgb_with(&[], &[stored(&[0; 38])], &[]),
                DecodeDefect::PngImageData,
            ),
            // More data than declared: stricter than libpng, which warns.
            (
                rgb_with(&[], &[stored(&[0; 40])], &[]),
                DecodeDefect::PngImageData,
            ),
            (
                rgb_with(&[], &[b"not zlib".to_vec()], &[]),
                DecodeDefect::PngImageData,
            ),
            (rgb_with(&[], &[bad_adler], &[]), DecodeDefect::PngImageData),
            // A chunk dropped for its CRC still ends the first run, which is
            // then too short.
            (
                image(
                    &ihdr(4, 3, 8, 2, 0),
                    &[],
                    &[head.to_vec()],
                    &[bad_text, chunk(b"IDAT", tail)],
                ),
                DecodeDefect::PngImageData,
            ),
        ];
        for (input, defect) in cases {
            assert_eq!(canonical(&input), Err(defect.into()), "{:?}", kinds(&input));
        }
    }

    #[test]
    fn a_suggested_palette_places_the_colour_chunks_only_where_libpng_takes_it() {
        let colour = || {
            vec![
                chunk(b"iCCP", &icc(b"p", &profile(*b"RGB "))),
                chunk(b"cICP", &[1, 13, 0, 0]),
            ]
        };
        // The chunk types kept, leaving out a kept tRNS.
        let with = |before: Vec<Vec<u8>>| {
            let mut found = kinds(&rewrite(&rgb(&[before, colour()].concat(), &[])));
            found.retain(|kind| kind != "tRNS");
            found
        };
        let kept = ["IHDR", "iCCP", "cICP", "IDAT", "IEND"];
        let placed = ["IHDR", "IDAT", "IEND"];
        // Taken: whole entries, up to 256, before the data.
        assert_eq!(with(vec![chunk(b"PLTE", &[9; 6])]), placed);
        assert_eq!(with(vec![chunk(b"PLTE", &[9; 3 * 256])]), placed);
        // Ignored, so the colour chunks after it count: not whole entries,
        // more than 256, or after a tRNS or a bKGD that libpng took.
        for (what, before) in [
            ("partial entry", vec![chunk(b"PLTE", &[9; 4])]),
            ("257 entries", vec![chunk(b"PLTE", &[9; 3 * 257])]),
            (
                "after tRNS",
                vec![chunk(b"tRNS", &[0; 6]), chunk(b"PLTE", &[9; 6])],
            ),
            (
                "after bKGD",
                vec![chunk(b"bKGD", &[0, 1, 0, 2, 0, 3]), chunk(b"PLTE", &[9; 6])],
            ),
        ] {
            assert_eq!(with(before), kept, "{what}");
        }
        // A tRNS or bKGD that libpng ignores does not count; nor does a
        // palette libpng ignored, so a valid one after it is taken.
        for (what, before) in [
            (
                "after an invalid tRNS",
                vec![chunk(b"tRNS", &[0; 4]), chunk(b"PLTE", &[9; 6])],
            ),
            (
                "after an 8-bit bKGD with a high byte",
                vec![chunk(b"bKGD", &[1, 1, 0, 2, 0, 3]), chunk(b"PLTE", &[9; 6])],
            ),
            (
                "after a short bKGD",
                vec![chunk(b"bKGD", &[0, 1]), chunk(b"PLTE", &[9; 6])],
            ),
            (
                "after an ignored palette",
                vec![chunk(b"PLTE", &[9; 4]), chunk(b"PLTE", &[9; 6])],
            ),
        ] {
            assert_eq!(with(before), placed, "{what}");
        }
        // A 16-bit image's bKGD may use the high bytes.
        let rgb16 = |before: &[Vec<u8>]| {
            kinds(&rewrite(&image(
                &ihdr(1, 1, 16, 2, 0),
                &[before, &colour()].concat(),
                &[stored(&[0; 7])],
                &[],
            )))
        };
        assert_eq!(
            rgb16(&[chunk(b"bKGD", &[1, 1, 2, 2, 3, 3]), chunk(b"PLTE", &[9; 6])]),
            kept
        );
        assert_eq!(rgb16(&[chunk(b"PLTE", &[9; 6])]), placed);
        // An empty palette where libpng would take one stops libpng; after
        // a tRNS, where it is ignored, or after a taken one, it does not.
        assert_eq!(
            canonical(&rgb(&[chunk(b"PLTE", &[])], &[])),
            Err(DecodeDefect::PngPalette.into())
        );
        assert_eq!(
            with(vec![chunk(b"tRNS", &[0; 6]), chunk(b"PLTE", &[])]),
            kept
        );
        assert_eq!(
            with(vec![chunk(b"PLTE", &[9; 6]), chunk(b"PLTE", &[])]),
            placed
        );
    }

    #[test]
    fn libpng_reads_the_crc_of_iend_and_of_an_optional_palette_as_a_warning() {
        let damaged = |mut chunk: Vec<u8>| {
            *chunk.last_mut().unwrap() ^= 1;
            chunk
        };
        // IEND with a wrong CRC still ends the file.
        let mut plain = rgb(&[], &[]);
        let last = plain.len() - 1;
        plain[last] ^= 1;
        assert_eq!(rewrite(&plain), rgb(&[], &[]));
        // A truecolour image's palette with a wrong CRC still places the
        // colour chunks; a greyscale image's is still ignored.
        let suggested = rgb(
            &[damaged(chunk(b"PLTE", &[9; 6])), chunk(b"sRGB", &[0])],
            &[],
        );
        assert_eq!(kinds(&rewrite(&suggested)), ["IHDR", "IDAT", "IEND"]);
        let grey = image(
            &ihdr(2, 1, 8, 0, 0),
            &[damaged(chunk(b"PLTE", &[9; 6])), chunk(b"sRGB", &[0])],
            &[stored(&[0, 1, 2])],
            &[],
        );
        assert_eq!(kinds(&rewrite(&grey)), ["IHDR", "sRGB", "IDAT", "IEND"]);
        // An indexed image's palette is critical to it: refused.
        let palette = indexed(&[damaged(chunk(b"PLTE", &[9; 6]))]);
        assert_eq!(canonical(&palette), Err(DecodeDefect::PngChecksum.into()));
    }

    #[test]
    fn a_row_with_an_unknown_filter_type_is_refused() {
        // Filter type 5 in the last row, then type 4, which is kept.
        for (filter, refused) in [(5, true), (0xFF, true), (4, false)] {
            let mut rows = pixels(4, 3, 3);
            rows[2 * 13] = filter;
            let input = image(&ihdr(4, 3, 8, 2, 0), &[], &[stored(&rows)], &[]);
            assert_eq!(
                canonical(&input).is_err_and(|e| e == Refusal::Defect(DecodeDefect::PngImageData)),
                refused,
                "{filter}"
            );
        }
    }

    #[test]
    fn what_libpng_ignores_is_dropped_rather_than_refused() {
        let clean = rgb(&[], &[]);
        // IDAT chunks after the first run, a second or late suggested
        // palette, all after a complete image. A late empty one is out of
        // place for libpng, which ignores it before it reads it.
        for after in [
            vec![chunk(b"tEXt", b"a\0b"), chunk(b"IDAT", b"junk")],
            vec![chunk(b"PLTE", &[0; 3])],
            vec![chunk(b"PLTE", &[])],
        ] {
            assert_eq!(rewrite(&rgb(&[], &after)), clean);
        }
        let twice = rgb(&[chunk(b"PLTE", &[0; 3]), chunk(b"PLTE", &[0; 3])], &[]);
        assert_eq!(rewrite(&twice), clean);
    }

    #[test]
    fn split_image_data_is_joined_and_kept_as_it_was() {
        let stream = stored(&pixels(4, 3, 3));
        let (head, tail) = stream.split_at(10);
        let split = image(
            &ihdr(4, 3, 8, 2, 0),
            &[],
            &[head.to_vec(), Vec::new(), tail.to_vec()],
            &[],
        );
        let joined = image(
            &ihdr(4, 3, 8, 2, 0),
            &[],
            std::slice::from_ref(&stream),
            &[],
        );
        assert_eq!(rewrite(&split), joined);
    }

    /// The data of an `mDCV` chunk with these light levels: BT.2020
    /// primaries and a D65 white point, invented.
    fn mdcv(max: u32, min: u32) -> Vec<u8> {
        let mut data = vec![
            0x8A, 0x48, 0x39, 0x08, 0x21, 0x34, 0x9B, 0xAA, 0x19, 0x96, 0x08, 0xFC,
        ];
        data.extend_from_slice(&[0x3D, 0x13, 0x40, 0x42]);
        data.extend_from_slice(&max.to_be_bytes());
        data.extend_from_slice(&min.to_be_bytes());
        data
    }

    /// The data of a `cLLI` chunk with these light levels.
    fn clli(max: u32, average: u32) -> Vec<u8> {
        [max.to_be_bytes(), average.to_be_bytes()].concat()
    }

    #[test]
    fn hdr_chunks_are_kept_as_they_are_where_libpng_takes_them() {
        let hdr = [
            chunk(b"cICP", &[9, 16, 0, 1]),
            chunk(b"mDCV", &mdcv(10_000_000, 1)),
            chunk(b"cLLI", &clli(10_000_000, 4_000_000)),
        ];
        let out = rewrite(&rgb(&hdr, &[]));
        assert_eq!(
            kinds(&out),
            ["IHDR", "cICP", "mDCV", "cLLI", "IDAT", "IEND"]
        );
        assert_eq!(data_of(&out, b"mDCV"), Some(mdcv(10_000_000, 1)));
        // At the largest light level, and with the full-range flag 0.
        let edge = rgb(
            &[
                chunk(b"cICP", &[1, 13, 0, 0]),
                chunk(b"mDCV", &mdcv(MAX_LIGHT_LEVEL, MAX_LIGHT_LEVEL)),
                chunk(b"cLLI", &clli(MAX_LIGHT_LEVEL, 0)),
            ],
            &[],
        );
        assert_eq!(kinds(&rewrite(&edge)).len(), 6);
        // After the image data, after an indexed image's palette, or after
        // a truecolour image's suggested one: dropped.
        assert_eq!(kinds(&rewrite(&rgb(&[], &hdr))), ["IHDR", "IDAT", "IEND"]);
        let palette = indexed(&[&[chunk(b"PLTE", &[0; 6])][..], &hdr].concat());
        assert_eq!(kinds(&rewrite(&palette)), ["IHDR", "PLTE", "IDAT", "IEND"]);
        let suggested = rgb(&[&[chunk(b"PLTE", &[0; 6])][..], &hdr].concat(), &[]);
        assert_eq!(kinds(&rewrite(&suggested)), ["IHDR", "IDAT", "IEND"]);
    }

    #[test]
    fn invalid_hdr_values_are_dropped_and_still_take_the_place() {
        for (kind, invalid, valid) in [
            // Matrix coefficients 1 (YCbCr), or a full-range flag of 2.
            (b"cICP", vec![9, 16, 1, 1], vec![9, 16, 0, 1]),
            (b"cICP", vec![9, 16, 0, 2], vec![9, 16, 0, 1]),
            (b"mDCV", mdcv(MAX_LIGHT_LEVEL + 1, 1), mdcv(1, 1)),
            (b"mDCV", mdcv(1, MAX_LIGHT_LEVEL + 1), mdcv(1, 1)),
            (b"cLLI", clli(MAX_LIGHT_LEVEL + 1, 0), clli(1, 1)),
            (b"cLLI", clli(0, MAX_LIGHT_LEVEL + 1), clli(1, 1)),
        ] {
            let name = String::from_utf8_lossy(kind);
            assert_eq!(
                data_of(&rewrite(&rgb(&[chunk(kind, &invalid)], &[])), kind),
                None,
                "{name}"
            );
            // libpng counts the invalid one as the chunk, so a valid one
            // after it is a duplicate; a valid first one is kept alone.
            let twice = rgb(&[chunk(kind, &invalid), chunk(kind, &valid)], &[]);
            assert_eq!(data_of(&rewrite(&twice), kind), None, "{name}");
            let first = rgb(&[chunk(kind, &valid), chunk(kind, &invalid)], &[]);
            assert_eq!(
                data_of(&rewrite(&first), kind),
                Some(valid.clone()),
                "{name}"
            );
            // One of the wrong length does not take the place.
            let long = [&valid[..], &[0]].concat();
            let after_long = rgb(&[chunk(kind, &long), chunk(kind, &valid)], &[]);
            assert_eq!(data_of(&rewrite(&after_long), kind), Some(valid), "{name}");
        }
    }

    /// A 4x3 RGB APNG: `acTL` for `frames`, the chunks `before` the data
    /// after it, the image data, then the chunks `after`.
    fn animated(frames: u32, before: &[Vec<u8>], after: &[Vec<u8>]) -> Vec<u8> {
        let control = chunk(b"acTL", &actl(frames, 0));
        rgb(&[&[control][..], before].concat(), after)
    }

    #[test]
    fn a_valid_animation_is_kept_after_the_kept_chunks() {
        let frame = stored(&pixels(2, 2, 3));
        let input = animated(
            2,
            &[
                chunk(b"tEXt", b"a\0b"),
                chunk(b"fcTL", &fctl(0, (4, 3), (0, 0))),
                chunk(b"sRGB", &[0]),
            ],
            &[
                chunk(b"fcTL", &fctl(1, (2, 2), (2, 1))),
                chunk(b"fdAT", &fdat(2, &frame)),
                chunk(b"tEXt", b"c\0d"),
            ],
        );
        let out = canonical(&input).unwrap();
        assert_eq!(
            kinds(&out.png),
            [
                "IHDR", "sRGB", "acTL", "fcTL", "IDAT", "fcTL", "fdAT", "IEND"
            ]
        );
        assert_eq!(out.pixels, 12 + 4);
        assert!(out.png.len() < input.len());
        // Rewriting the rewrite changes nothing.
        assert_eq!(canonical(&out.png).map(|again| again.png), Ok(out.png));
    }

    /// `chunk` with its CRC broken, so the walk reads it as lost.
    fn lost(mut chunk: Vec<u8>) -> Vec<u8> {
        *chunk.last_mut().unwrap() ^= 1;
        chunk
    }

    /// A 4x3 RGB APNG of three frames, `plays` plays and the chunks `extra`
    /// before the data, after its `acTL` for `frames` frames: the default
    /// image as frame 0, a 2x2 frame and a 1x1 frame. The frame chunks after
    /// the data are returned apart, so a test can change them.
    fn three_frames(frames: u32, extra: &[Vec<u8>]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let before = [
            &[chunk(b"acTL", &actl(frames, 7))][..],
            extra,
            &[chunk(b"fcTL", &fctl(0, (4, 3), (0, 0)))],
        ]
        .concat();
        let after = vec![
            chunk(b"fcTL", &fctl(1, (2, 2), (1, 1))),
            chunk(b"fdAT", &fdat(2, &stored(&pixels(2, 2, 3)))),
            chunk(b"fcTL", &fctl(3, (1, 1), (3, 2))),
            chunk(b"fdAT", &fdat(4, &stored(&pixels(1, 1, 3)))),
        ];
        (before, after)
    }

    /// The `acTL` data and the number of `fcTL` chunks of a rewrite, or
    /// `None` and 0 when it holds no animation.
    fn animation_of(png: &[u8]) -> (Option<Vec<u8>>, usize) {
        let controls = kinds(png).iter().filter(|kind| *kind == "fcTL").count();
        (data_of(png, b"acTL"), controls)
    }

    #[test]
    fn a_misplaced_animation_or_one_with_no_frame_left_leaves_the_still_image() {
        let still = rewrite(&rgb(&[], &[]));
        let frame = || chunk(b"fdAT", &fdat(1, &stored(&pixels(4, 3, 3))));
        let control = || chunk(b"fcTL", &fctl(0, (4, 3), (0, 0)));
        // acTL after the data; no acTL at all; the only frame lost to its
        // CRC.
        let late = rgb(&[], &[chunk(b"acTL", &actl(1, 0)), control(), frame()]);
        assert_eq!(rewrite(&late), still);
        let none = rgb(&[], &[control(), frame()]);
        assert_eq!(rewrite(&none), still);
        assert_eq!(
            rewrite(&animated(1, &[], &[control(), lost(frame())])),
            still
        );
        // The only frame's data does not decode: found after the walk, it
        // leaves no frame either.
        let junk = chunk(b"fdAT", &fdat(1, b"junk"));
        assert_eq!(rewrite(&animated(1, &[], &[control(), junk])), still);
        // A lost chunk is the defect even where the rest would pass: here a
        // trailing fdAT whose bytes would follow the complete stream.
        let trailing = lost(chunk(b"fdAT", &fdat(2, b"junk")));
        let whole = animated(1, &[], &[control(), frame()]);
        assert_eq!(kinds(&rewrite(&whole)).len(), 6);
        assert_eq!(
            rewrite(&animated(1, &[], &[control(), frame(), trailing])),
            still
        );
        // An invalid acTL does not hide a valid later one.
        let second = rgb(
            &[chunk(b"acTL", &actl(0, 0)), chunk(b"acTL", &actl(1, 0))],
            &[control(), frame()],
        );
        assert_eq!(kinds(&rewrite(&second)).len(), 6);
        // A chunk between two fdAT of one frame parts them.
        let stream = stored(&pixels(4, 3, 3));
        let (head, tail) = stream.split_at(7);
        let parted = animated(
            1,
            &[],
            &[
                control(),
                chunk(b"fdAT", &fdat(1, head)),
                chunk(b"tEXt", b"a\0b"),
                chunk(b"fdAT", &fdat(2, tail)),
            ],
        );
        assert_eq!(rewrite(&parted), still);
    }

    #[test]
    fn a_broken_animation_keeps_its_frames_before_the_first_defect() {
        let (before, after) = three_frames(3, &[]);
        let whole = rewrite(&rgb(&before, &after));
        assert_eq!(animation_of(&whole), (Some(actl(3, 7)), 3));
        // The walk routes each chunk to the animation as the defect needs:
        // the chunks after the data, the frame they lie in, the frames kept.
        let mut gap = after.clone();
        gap[3] = chunk(b"fdAT", &fdat(5, &stored(&pixels(1, 1, 3))));
        let parted = [
            &after[..2],
            &[chunk(b"tEXt", b"a\0b"), chunk(b"fdAT", &fdat(3, b"junk"))],
            &after[2..],
        ]
        .concat();
        // A chunk lost to its CRC parts two fdAT chunks as an intact one
        // does: the png crate still reads it as a chunk between them.
        let parted_by_lost = [
            &after[..2],
            &[
                lost(chunk(b"tEXt", b"a\0b")),
                chunk(b"fdAT", &fdat(3, b"junk")),
            ],
            &after[2..],
        ]
        .concat();
        let mut lost_control = after.clone();
        lost_control[2] = lost(lost_control[2].clone());
        let mut lost_data = after.clone();
        lost_data[1] = lost(lost_data[1].clone());
        let right_after = [&[chunk(b"fdAT", &fdat(1, b"junk"))], &after[..]].concat();
        let cases: [(&str, Vec<Vec<u8>>, usize); 6] = [
            ("a sequence gap in the third frame", gap, 2),
            ("an fdAT parted from the one before", parted, 1),
            (
                "an fdAT parted by a chunk lost to its CRC",
                parted_by_lost,
                1,
            ),
            ("the third frame's fcTL lost to its CRC", lost_control, 2),
            ("the second frame's fdAT lost to its CRC", lost_data, 1),
            ("an fdAT right after the data, in no frame", right_after, 1),
        ];
        for (what, after, frames) in cases {
            let input = rgb(&before, &after);
            let out = canonical(&input).unwrap();
            let announced = u32::try_from(frames).unwrap();
            assert_eq!(
                animation_of(&out.png),
                (Some(actl(announced, 7)), frames),
                "{what}"
            );
            assert!(out.png.len() <= input.len(), "{what}");
            // The rewrite of a cut animation is a valid one: rewriting it
            // again changes nothing.
            assert_eq!(
                canonical(&out.png).map(|again| again.png),
                Ok(out.png),
                "{what}"
            );
        }
        // A frame that does not decode is the defect in its frame, found
        // after the walk; the encoder works on the frames kept.
        let mut junk = after.clone();
        junk[3] = chunk(b"fdAT", &fdat(4, b"junk"));
        let out = canonical(&rgb(&before, &junk)).unwrap();
        assert_eq!(animation_of(&out.png), (Some(actl(2, 7)), 2));
        assert_eq!(out.pixels, 12 + 4);
    }

    #[test]
    fn an_actl_lost_to_its_crc_is_a_defect_only_before_the_data() {
        // Before the data it might have announced the animation, so no
        // frame is kept, even after a valid acTL.
        let (before, after) = three_frames(3, &[lost(chunk(b"acTL", &actl(3, 7)))]);
        assert_eq!(animation_of(&rewrite(&rgb(&before, &after))), (None, 0));
        // After the data an acTL is ignored, intact or not.
        let (before, after) = three_frames(3, &[]);
        for late in [
            chunk(b"acTL", &actl(1, 0)),
            lost(chunk(b"acTL", &actl(1, 0))),
        ] {
            let input = rgb(&before, &[&[late][..], &after].concat());
            assert_eq!(animation_of(&rewrite(&input)), (Some(actl(3, 7)), 3));
        }
    }

    #[test]
    fn a_chunk_lost_inside_frame_0_before_the_data_leaves_no_frame() {
        // Between frame 0's fcTL and the image data, frame 0 has not ended:
        // an acTL, fcTL or fdAT lost to its CRC there lies in it, so no
        // frame is kept and the still image is stored.
        let (before, after) = three_frames(3, &[]);
        let inside = [
            ("acTL", lost(chunk(b"acTL", &actl(3, 7)))),
            ("fcTL", lost(chunk(b"fcTL", &fctl(1, (2, 2), (1, 1))))),
            (
                "fdAT",
                lost(chunk(b"fdAT", &fdat(1, &stored(&pixels(4, 3, 3))))),
            ),
        ];
        let still = rewrite(&rgb(&[], &[]));
        for (what, lost_chunk) in inside {
            let input = rgb(&[&before[..], &[lost_chunk]].concat(), &after);
            assert_eq!(rewrite(&input), still, "{what}");
        }
        // The controls: the same acTL lost before frame 0's fcTL, where it
        // lies in no frame and none has ended, leaves no frame either, and
        // an intact copy inside frame 0 is ignored, so every frame is kept.
        let (early, after) = three_frames(3, &[lost(chunk(b"acTL", &actl(3, 7)))]);
        assert_eq!(rewrite(&rgb(&early, &after)), still);
        let copy = chunk(b"acTL", &actl(3, 7));
        let (before, after) = three_frames(3, &[]);
        let input = rgb(&[&before[..], &[copy]].concat(), &after);
        assert_eq!(animation_of(&rewrite(&input)), (Some(actl(3, 7)), 3));
    }

    #[test]
    fn chunks_outside_the_animation_never_cut_it() {
        // Between the frames, an IDAT after the first run and, in this RGB
        // image, a PLTE of the wrong length, a second PLTE or one with a bad
        // CRC are judged as in a still image: libpng ignores them, so they
        // are dropped and every frame is kept, although the png crate stops
        // at each (ADR 0033 §6, png/apng.rs).
        let (before, after) = three_frames(3, &[]);
        let whole = rewrite(&rgb(&before, &after));
        assert_eq!(animation_of(&whole), (Some(actl(3, 7)), 3));
        let palette = || chunk(b"PLTE", &[0; 3]);
        let between: [(&str, Vec<Vec<u8>>); 5] = [
            (
                "an IDAT after the first run",
                vec![chunk(b"IDAT", &stored(&pixels(4, 3, 3)))],
            ),
            ("an empty PLTE", vec![chunk(b"PLTE", &[])]),
            ("a PLTE of 771 bytes", vec![chunk(b"PLTE", &[0; 771])]),
            ("a second PLTE", vec![palette(), palette()]),
            ("a PLTE with a bad CRC", vec![lost(palette())]),
        ];
        for (what, extra) in between {
            let input = rgb(&before, &[&after[..2], &extra, &after[2..]].concat());
            assert_eq!(rewrite(&input), whole, "{what}");
        }
        // Before the data, a second PLTE and one of 771 bytes are ignored
        // as well, and the animation is whole.
        for extra in [vec![palette(), palette()], vec![chunk(b"PLTE", &[0; 771])]] {
            let (before, after) = three_frames(3, &extra);
            assert_eq!(rewrite(&rgb(&before, &after)), whole);
        }
    }

    #[test]
    fn an_animation_cut_short_is_refused_like_a_still_png() {
        // A file cut inside its animation has no IEND, which the still-image
        // checks refuse, whatever the frames before the cut hold.
        let (before, after) = three_frames(3, &[]);
        let input = rgb(&before, &after);
        let inside_last_fdat = input.len() - 12 - 5;
        assert_eq!(
            canonical(&input[..inside_last_fdat]),
            Err(DecodeDefect::PngTruncated.into())
        );
    }

    #[test]
    fn a_default_image_that_does_not_decode_is_refused_whatever_the_animation() {
        let input = image(
            &ihdr(4, 3, 8, 2, 0),
            &[
                chunk(b"acTL", &actl(1, 0)),
                chunk(b"fcTL", &fctl(0, (4, 3), (0, 0))),
            ],
            &[stored(&[0; 5])],
            &[],
        );
        assert_eq!(canonical(&input), Err(DecodeDefect::PngImageData.into()));
    }

    #[test]
    fn an_animation_above_the_totals_is_refused() {
        // A 1x1 canvas, its default image as frame 0, and 4096 more frames.
        let mut after = Vec::new();
        for index in 1..=4096_u32 {
            after.push(chunk(b"fcTL", &fctl(2 * index - 1, (1, 1), (0, 0))));
            after.push(chunk(b"fdAT", &fdat(2 * index, &stored(&[0, 0]))));
        }
        let with_data = |data: Vec<u8>| {
            image(
                &ihdr(1, 1, 8, 0, 0),
                &[
                    chunk(b"acTL", &actl(4097, 0)),
                    chunk(b"fcTL", &fctl(0, (1, 1), (0, 0))),
                ],
                &[data],
                &after,
            )
        };
        let too_large = Err(Refusal::AnimationTooLarge(apng::TooLarge {
            frames: 4097,
            pixels: 4097,
        }));
        assert_eq!(canonical(&with_data(stored(&[0, 0]))), too_large);
        // The totals are checked before any image data is inflated, the
        // default image's included: with image data that does not decode,
        // the refusal is still the animation's (ADR 0033 §4).
        assert_eq!(canonical(&with_data(b"junk".to_vec())), too_large);
    }

    proptest! {
        #[test]
        fn the_rewrite_is_never_larger_and_keeps_only_the_list(
            extra in vec((prop::sample::select(vec![
                *b"tEXt", *b"gAMA", *b"sRGB", *b"cHRM", *b"iCCP", *b"eXIf", *b"pHYs", *b"tRNS", *b"prVt",
                *b"cICP", *b"mDCV", *b"cLLI", *b"acTL", *b"fcTL", *b"fdAT",
            ]), vec(any::<u8>(), 0..40)), 0..10),
            tail in vec(any::<u8>(), 0..32),
        ) {
            let before: Vec<Vec<u8>> = extra.iter().map(|(kind, data)| chunk(kind, data)).collect();
            let input = [rgb(&before, &[]), tail].concat();
            let out = canonical(&input).unwrap().png;
            prop_assert!(out.len() <= input.len());
            for kind in kinds(&out) {
                prop_assert!(
                    ["IHDR", "sRGB", "gAMA", "cHRM", "iCCP", "eXIf", "tRNS", "IDAT", "IEND", "cICP", "mDCV",
                     "cLLI", "acTL", "fcTL", "fdAT"]
                        .contains(&kind.as_str()),
                    "{}", kind
                );
            }
            // Rewriting the rewrite changes nothing.
            prop_assert_eq!(canonical(&out).map(|rewrite| rewrite.png), Ok(out.clone()));
        }

        #[test]
        fn a_profile_is_accepted_only_whole_and_of_its_declared_size(
            bytes in vec(any::<u8>(), 0..300),
            colour in prop::sample::select(vec![0_u8, 2, 3, 4, 6]),
            cut in 0..144_usize,
        ) {
            if profile_fits(&bytes, colour) {
                prop_assert!(bytes.len() >= PROFILE_HEADER);
                prop_assert_eq!(u32::from_be_bytes(bytes[0..4].try_into().unwrap()), u32::try_from(bytes.len()).unwrap());
            }
            // A valid profile cut short is never accepted.
            let space = if colour & 2 == 0 { b"GRAY" } else { b"RGB " };
            prop_assert!(!profile_fits(&profile(*space)[..cut], colour));
        }

        #[test]
        fn damaged_files_never_panic(
            damage in vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
            // Cut only now and then: a cut file has no IEND and is refused.
            cut in prop::option::weighted(0.2, any::<prop::sample::Index>()),
        ) {
            let profile_chunk = chunk(b"iCCP", &icc(b"p", &profile(*b"RGB ")));
            let mut bytes = rgb(&[profile_chunk, chunk(b"eXIf", &camera()), chunk(b"tRNS", &[0; 6])], &[]);
            for (at, byte) in damage {
                let at = at.index(bytes.len());
                bytes[at] = byte;
            }
            if let Some(cut) = cut {
                bytes.truncate(cut.index(bytes.len()));
            }
            if let Ok(out) = canonical(&bytes) {
                prop_assert!(out.png.len() <= bytes.len());
            }
        }
    }
}
