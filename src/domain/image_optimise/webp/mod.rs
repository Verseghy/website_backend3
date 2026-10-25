//! WebP uploads: the metadata strip and the lossy re-encode (ADR 0015 §7:
//! "Metadata is removed. Lossy WebP is re-encoded at the same quality target
//! and kept only under the same at-least-5 % rule as JPEG. Lossless WebP
//! keeps its pixels and animation.", amended by ADR 0033 §1, §3 and §4).
//!
//! **What is kept.** The strip writes a new file from the chunks that
//! libwebp reads to show the image:
//!
//! | Chunk | In the stored file |
//! |---|---|
//! | `VP8 ` or `VP8L` of a simple file | copied; nothing after it is read |
//! | `VP8X` | rewritten: the canvas is kept, the flags say what is kept |
//! | `ICCP` | the first one, copied, when the ICC flag is set (ADR 0015 §7 keeps the profile) |
//! | `ALPH` and `VP8 `, or `VP8L`, of a still image | the ones libwebp's decoder reads, copied (below) |
//! | `ANIM` of an animation | the first one, rewritten with its 6 bytes: background colour and loop count |
//! | `ANMF` of an animation | each frame rewritten with its 16-byte header and its image chunks |
//! | `EXIF` | the first one, when the EXIF flag is set, replaced by the Orientation tag alone when it has one ([`Orientation`]) |
//! | `XMP `, other chunks, bytes after the RIFF size | dropped |
//!
//! A `VP8X` file needs the flag for libwebp to read its `ICCP` or `EXIF`
//! chunk, so an unflagged one is invisible to every viewer and is dropped
//! like any other metadata. A file left with no metadata, no alpha chunk and
//! no animation is written in the simple format, which says the same with
//! 18 bytes less. The image chunks and the kept profile are copied as they
//! are: the strip removes the chunks that viewers do not show, and does not
//! look inside the ones they do. The EXIF chunk is read with the reader that
//! JPEG and PNG use, as Chromium reads it (ADR 0033 §3): its TIFF structure
//! starts the chunk, with no `Exif` identifier in front, as Skia's parser
//! requires.
//!
//! **Read as libwebp reads it** (ADR 0033 §3, decision D78). libwebp has two
//! readers of the chunks after `VP8X`, and they disagree on some layouts:
//!
//! - its decoder (`WebPDecode`: `ParseHeadersInternal` and
//!   `ParseOptionalChunks` in `libwebp@1.6.0:src/dec/webp_dec.c`), which
//!   decodes a still image and which the header check of #211 and the decode
//!   check follow. It refuses every animation;
//! - its demuxer (`ParseVP8XChunks`, `ParseAnimationFrame`, `StoreFrame` and
//!   `IsValidExtendedFormat` in `src/demux/demux.c`), which its animation
//!   decoder reads an animation with, which is its only reader of the profile
//!   and the EXIF, and which browsers run before they decode. It is stricter
//!   than the decoder about a still image's layout.
//!
//! The strip follows each where it is libwebp's reader, so every file that
//! libwebp decodes passes:
//!
//! - **A still image** is read as the decoder reads it ([`decoded_still`]):
//!   the chunks up to the first bitstream must be whole, and are skipped; the
//!   last `ALPH` chunk among them is the alpha of a `VP8 ` bitstream whatever
//!   the alpha flag says, since a `VP8L` bitstream has its own; nothing after
//!   the bitstream is read. So a second image, a second `VP8X` chunk, an
//!   `ANIM` chunk, an `ALPH` chunk after the bitstream, reserved `VP8X` flags
//!   or a chunk cut short after the image, which the demuxer refuses, do not
//!   refuse the file. The stored file keeps the decoder's `ALPH` chunk with
//!   the alpha flag as uploaded: the demuxer reads that chunk only with the
//!   flag, so each reader shows the stored file as it showed the upload. The
//!   reserved flags are written clear.
//! - **An animation** is read as the demuxer reads it ([`walk`]), and the
//!   strip refuses what it refuses, reserved `VP8X` flags included:
//!   - an `ANIM` chunk counts with its padded size: one of 5 bytes and its
//!     pad byte is read as 6 bytes, the pad byte being the high byte of the
//!     loop count; an `ANMF` chunk likewise needs a padded size of 16;
//!   - an `ANMF` chunk holds its frame's image chunks right after its
//!     16-byte header. libwebp reads whatever follows them as chunks of the
//!     file, also inside the `ANMF` chunk: a profile or EXIF block there
//!     counts as the file's, in file order; a nested `ANMF` chunk there is the
//!     next frame; and an image chunk there is refused, as image chunks
//!     outside a frame are. An `ANMF` chunk with no image chunk after its
//!     header is not a frame, and one with an `ALPH` chunk but no bitstream,
//!     or with its `ALPH` chunk after its bitstream, is refused;
//!   - an `ANMF` header that declares 2³² pixels or more is refused, although
//!     libwebp takes the frame's size from its bitstream, because libwebp
//!     refuses it first.
//! - **The profile and the EXIF** are the first flagged ones in file order,
//!   as the demuxer reads them, the chunks after an `ANMF` frame's image
//!   included (in a still image too, where the demuxer shows no frame of it).
//!   Where the demuxer would refuse a still image, it reads no further, and
//!   neither does the strip: metadata after that point are dropped.
//!
//! A simple file whose bitstream chunk ends the RIFF size without the pad
//! byte that its odd size needs is read as the decoder reads it, and stored
//! as uploaded, without the byte, which would make the output larger than
//! the upload (ADR 0015 §7 never lets it grow), as a GIF without its trailer
//! is; after a `VP8X` chunk the decoder refuses that layout too.
//! `docs/design/image-encoders.md` lists every layout with what each reader
//! and the strip do.
//!
//! **The treatment** then depends on the image ([`Content`]):
//!
//! - every still image must have the canvas size, as libwebp requires, and
//!   libwebp decodes the stored file; an image that does not decode is
//!   refused as corrupt;
//! - a lossy still image is then re-encoded at quality 85 from its `VP8 `
//!   bitstream. Its `ALPH` chunk, if it has one, is kept as it is. The
//!   re-encode, with the kept metadata written back, is stored only when it
//!   passes the keep rule of ADR 0033 §1: [`reencode_is_kept`] first, then
//!   [`reencode_meets_ssim_floor`] on the luma SSIM of the re-encode against
//!   the decoded original. Otherwise the stripped file is stored, also when
//!   the encoder fails ([`Optimised::re_encode_failure`]), and
//!   [`Treatment::ReEncodeDiscarded`] says why;
//! - a lossless still image keeps its bitstream, and an animation keeps every
//!   frame as it is: animated WebP is never re-encoded, lossy frames
//!   included (ADR 0033 §3), so its frames do not change;
//! - an animation is held to [`super::MAX_ANIMATION_FRAMES`] and
//!   [`super::MAX_ANIMATION_PIXELS`] (ADR 0033 §4) while its frames are
//!   read, so the walk stops at the frame that passes a limit and a file of
//!   millions of frames costs no memory for them; every frame must then fit
//!   the canvas. Only then does libwebp decode each frame on its own, and
//!   only then is the stripped copy written.

mod codec;
mod riff;

use riff::Chunk;

use super::header::bitstream_dimensions;
use super::outcome::{check_animation_size, inspect_as};
use super::{
    DecodeDefect, Dimensions, ImageError, ImageFormat, NotReencoded, Optimised, Orientation,
    PixelBudget, Treatment, reencode_is_kept, reencode_meets_ssim_floor,
};

/// The `VP8X` flag for an ICC profile (RFC 9649 §2.7).
const ICC: u8 = 0x20;
/// The `VP8X` flag for alpha.
const ALPHA: u8 = 0x10;
/// The `VP8X` flag for EXIF metadata.
const EXIF: u8 = 0x08;
/// The `VP8X` flag for XMP metadata, which the strip drops.
const XMP: u8 = 0x04;
/// The `VP8X` flag for animation.
const ANIMATION: u8 = 0x02;

/// The `VP8X` flags that libwebp defines (`ALL_VALID_FLAGS` in
/// `src/webp/mux_types.h`). The others are reserved: libwebp's decoder
/// ignores them, and its demuxer refuses a file that sets one.
const VALID_FLAGS: u8 = ICC | ALPHA | EXIF | XMP | ANIMATION;

/// The size of an `ANMF` frame header: position, size, duration and flags.
const FRAME_HEADER: usize = 16;

/// The size of an `ANIM` payload: background colour and loop count.
const ANIM_SIZE: usize = 6;

/// The pixel count from which libwebp refuses a frame header
/// (`MAX_IMAGE_AREA` in `src/webp/format_constants.h`).
const MAX_IMAGE_AREA: u64 = 1 << 32;

/// A still image or the image of one frame: an optional `ALPH` chunk and a
/// bitstream chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Image<'a> {
    /// The `ALPH` payload that libwebp reads with a `VP8 ` bitstream: its
    /// decoder's in a still image ([`decoded_still`]), its demuxer's in a
    /// frame ([`store_frame`]).
    alpha: Option<&'a [u8]>,
    /// The `VP8 ` or `VP8L` chunk.
    bitstream: Chunk<'a>,
    /// The width and height that the bitstream declares.
    dimensions: Dimensions,
}

impl Image<'_> {
    /// Whether the bitstream is lossless (`VP8L`).
    fn is_lossless(&self) -> bool {
        self.bitstream.fourcc == *b"VP8L"
    }

    /// Appends the image's chunks to `out`: the `ALPH` chunk, when there is
    /// one, then the bitstream, as libwebp requires.
    ///
    /// - `out`: the chunks written so far.
    fn write(&self, out: &mut Vec<u8>) {
        if let Some(alpha) = self.alpha {
            riff::write_chunk(out, *b"ALPH", alpha);
        }
        riff::write_chunk(out, self.bitstream.fourcc, self.bitstream.payload);
    }
}

/// One frame of an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frame<'a> {
    /// The 16-byte `ANMF` header: position, size, duration and flags.
    header: &'a [u8],
    /// The frame's image.
    image: Image<'a>,
}

impl Frame<'_> {
    /// The frame's top-left corner on the canvas. The header holds half of
    /// each coordinate in 24 bits, so frames start at even positions.
    fn offset(&self) -> (u64, u64) {
        let half = |at: usize| 2 * u64::from(u24(self.header, at));
        (half(0), half(3))
    }
}

/// The 24-bit little-endian number at `at` of `bytes`, which holds it.
///
/// - `bytes`: an `ANMF` header or a `VP8X` payload.
/// - `at`: the offset of the number's lowest byte.
fn u24(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], 0])
}

/// What a WebP shows, which decides its treatment.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content<'a> {
    /// A still image, lossy (`VP8 `, possibly with `ALPH`) or lossless.
    Still(Image<'a>),
    /// An animation: the `ANIM` payload and the frames in file order.
    Animation {
        /// The 6 bytes of the first `ANIM` chunk, its pad byte included when
        /// it declares 5.
        anim: &'a [u8],
        /// Every frame, at least one.
        frames: Vec<Frame<'a>>,
    },
}

/// The EXIF Orientation that the strip keeps, and the length of the TIFF
/// structure it was read from, which bounds the length of its replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeptExif {
    /// The orientation, read as Chromium reads it.
    orientation: Orientation,
    /// The length of the original `EXIF` payload.
    original_len: usize,
}

/// The parts of a WebP that the strip keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parts<'a> {
    /// The `VP8X` flags, or 0 for a simple file. Only the alpha bit is
    /// carried over as it is, also when the decoder's `ALPH` chunk is kept
    /// without it; the others are set from what is kept, and the reserved
    /// ones are left clear.
    flags: u8,
    /// The ICC profile, when it is kept.
    icc: Option<&'a [u8]>,
    /// The EXIF Orientation, when it is kept.
    exif: Option<KeptExif>,
    /// The image or the animation.
    content: Content<'a>,
    /// Whether the file is a simple one whose bitstream chunk ends the RIFF
    /// size without the pad byte that its odd size needs ([`parse`]). The
    /// stored file ends the same way, so it is never larger than the upload.
    unpadded: bool,
}

/// Why a re-encode is not stored: the reason the keep rule or the encoder
/// gave, and the encoder's own words when it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Discarded {
    /// The reason, as JPEG reports it.
    reason: NotReencoded,
    /// libwebp's reason, for the log, when `reason` is
    /// [`NotReencoded::EncoderFailed`].
    failure: Option<String>,
}

impl Discarded {
    /// A re-encode that failed because of `why`.
    ///
    /// - `why`: what libwebp or the check of its output reported.
    fn failed(why: impl Into<String>) -> Self {
        Self {
            reason: NotReencoded::EncoderFailed,
            failure: Some(why.into()),
        }
    }
}

impl From<NotReencoded> for Discarded {
    fn from(reason: NotReencoded) -> Self {
        Self {
            reason,
            failure: None,
        }
    }
}

/// Optimises a WebP upload: strips its metadata, checks that the result
/// decodes, and re-encodes a lossy still image, which is stored when that
/// saves at least 5 % and keeps a luma SSIM of at least 0.98.
///
/// - `bytes`: the whole upload.
/// - `budget`: the pixel budget for the canvas, which libwebp also holds to
///   while it decodes.
///
/// # Errors
///
/// The errors of [`super::inspect`], which runs first;
/// [`ImageError::WrongFormat`] when the upload is a JPEG, PNG or GIF;
/// [`ImageError::AnimationTooLarge`] when an animation has more frames or
/// pixels than ADR 0033 §4 allows; and [`ImageError::Undecodable`] when its
/// chunks are malformed as libwebp reads them, an image does not fit its
/// canvas, or libwebp cannot decode it ([`DecodeDefect::WebpData`]).
pub fn optimise_webp(bytes: &[u8], budget: PixelBudget) -> Result<Optimised, ImageError> {
    let header = inspect_as(bytes, budget, ImageFormat::WebP)?;
    let parts = parse(bytes)?;
    let (bytes, treatment, re_encode_failure) = match &parts.content {
        Content::Animation { frames, .. } => {
            // The limits held while the frames were read; the canvas and
            // every frame's decode come before the stripped copy is written,
            // so a refused animation never costs a copy of itself.
            check_frames_fit(frames, header.dimensions)?;
            let mut chunks = Vec::new();
            for frame in frames {
                chunks.clear();
                frame.image.write(&mut chunks);
                codec::check_decodes(&chunks, budget)?;
            }
            (
                assemble(&parts, header.dimensions),
                Treatment::Stripped,
                None,
            )
        }
        Content::Still(image) => {
            let stripped = assemble(&parts, header.dimensions);
            codec::check_decodes(&stripped, budget)?;
            if image.is_lossless() {
                (stripped, Treatment::Stripped, None)
            } else {
                let re_encoded =
                    re_encode(&parts, image, header.dimensions, budget, stripped.len());
                choose(stripped, re_encoded)
            }
        }
    };
    Ok(Optimised {
        header,
        bytes,
        treatment,
        re_encode_failure,
    })
}

/// An [`ImageError::Undecodable`] WebP for `defect`.
///
/// - `defect`: what the reading found.
fn undecodable(defect: DecodeDefect) -> ImageError {
    ImageError::Undecodable {
        format: ImageFormat::WebP,
        defect,
    }
}

/// The parts of a WebP that the strip keeps, as the module documentation
/// lists them.
///
/// - `bytes`: a whole WebP file: an upload that the header check accepted,
///   or libwebp's own output.
///
/// # Errors
///
/// [`ImageError::Undecodable`] when libwebp would refuse the file, and
/// [`ImageError::AnimationTooLarge`] as soon as an animation passes a
/// limit of ADR 0033 §4.
fn parse(bytes: &[u8]) -> Result<Parts<'_>, ImageError> {
    let body = riff::body(bytes).map_err(undecodable)?;
    let (first, next, unpadded) = match riff::chunk_at(body, 0) {
        Ok((chunk, next)) => (chunk, next, false),
        // A simple file whose bitstream ends the RIFF size without the pad
        // byte that its odd size needs: libwebp's decoder reads it, since it
        // measures the chunk by its own size (ParseVP8Header), so the strip
        // stores it as uploaded, without the byte, which would make the
        // output larger than the upload.
        Err(reason) => match riff::unpadded_chunk(body) {
            Some(chunk) if matches!(&chunk.fourcc, b"VP8 " | b"VP8L") => (chunk, body.len(), true),
            _ => return Err(undecodable(reason)),
        },
    };
    match &first.fourcc {
        b"VP8 " | b"VP8L" => Ok(Parts {
            flags: 0,
            icc: None,
            exif: None,
            content: Content::Still(image(None, first).map_err(undecodable)?),
            unpadded,
        }),
        b"VP8X" => extended(first.payload, &body[next..]),
        _ => Err(undecodable(DecodeDefect::WebpFirstChunk)),
    }
}

/// The parts of an extended (`VP8X`) file: a still image as libwebp's
/// decoder reads it, an animation as its demuxer reads it, and the metadata
/// as the demuxer reads them (the module documentation gives the rules).
///
/// - `vp8x`: the `VP8X` payload: flags, 3 reserved bytes, the canvas.
/// - `body`: everything after the `VP8X` chunk, up to the RIFF size.
///
/// # Errors
///
/// As [`parse`].
fn extended<'a>(vp8x: &[u8], body: &'a [u8]) -> Result<Parts<'a>, ImageError> {
    // The header check refuses any other size, as libwebp's decoder does.
    if vp8x.len() != 10 {
        return Err(undecodable(DecodeDefect::WebpVp8x));
    }
    let flags = vp8x[0];
    // The canvas sides minus one, 24 bits each, so never zero.
    let canvas = Dimensions::new(u24(vp8x, 4) + 1, u24(vp8x, 7) + 1)
        .ok_or_else(|| undecodable(DecodeDefect::WebpVp8x))?;
    let (content, demuxed) = if flags & ANIMATION == 0 {
        // libwebp's decoder reads a still image, whatever its demuxer makes
        // of the layout; the demuxer's refusal only ends the metadata.
        let image = decoded_still(body, canvas).map_err(undecodable)?;
        (Content::Still(image), demux(flags, body))
    } else {
        // The decoder refuses every animation, and the demuxer, through
        // which libwebp's animation decoder reads one, refuses reserved
        // flags (IsValidExtendedFormat) and every malformed chunk.
        if flags & !VALID_FLAGS != 0 {
            return Err(undecodable(DecodeDefect::WebpReservedFlags));
        }
        let mut demuxed = demux(flags, body);
        if let Some(refusal) = demuxed.refused.take() {
            return Err(refusal);
        }
        // An ANMF chunk needs an ANIM chunk before it, so frames imply one.
        let anim = demuxed
            .anim
            .filter(|_| !demuxed.frames.is_empty())
            .ok_or_else(|| undecodable(DecodeDefect::WebpNoFrame))?;
        let frames = core::mem::take(&mut demuxed.frames);
        (Content::Animation { anim, frames }, demuxed)
    };
    Ok(Parts {
        flags,
        icc: demuxed.icc,
        exif: demuxed.exif.and_then(|tiff: &[u8]| {
            Orientation::from_tiff(tiff).map(|orientation| KeptExif {
                orientation,
                original_len: tiff.len(),
            })
        }),
        content,
        unpadded: false,
    })
}

/// The still image of an extended file, as libwebp's decoder reads it
/// (`ParseOptionalChunks` and `ParseVP8Header` in
/// `libwebp@1.6.0:src/dec/webp_dec.c`): the chunks before the first `VP8 `
/// or `VP8L` chunk must be whole and are skipped, whatever they are; the
/// last `ALPH` chunk among them is the alpha of a `VP8 ` bitstream, whatever
/// the alpha flag says; and nothing after the bitstream is read.
///
/// - `body`: everything after the `VP8X` chunk.
/// - `canvas`: the canvas of the `VP8X` chunk, which the image must fill.
///
/// # Errors
///
/// What libwebp's decoder refuses: a chunk cut short before the bitstream
/// ([`DecodeDefect::WebpCutShort`]), no bitstream
/// ([`DecodeDefect::WebpNoImage`]), a malformed bitstream header, or an
/// image that is not the size of its canvas
/// ([`DecodeDefect::WebpCanvasSize`]).
fn decoded_still(body: &[u8], canvas: Dimensions) -> Result<Image<'_>, DecodeDefect> {
    let mut alpha = None;
    let mut at = 0;
    while at < body.len() {
        // Every chunk must fit with its pad byte, the bitstream's included:
        // ParseOptionalChunks counts the padded size of the chunk it stops
        // at against the RIFF size.
        let (chunk, next) = riff::chunk_at(body, at)?;
        match &chunk.fourcc {
            // ParseOptionalChunks keeps the last one it passes.
            b"ALPH" => alpha = Some(chunk.payload),
            b"VP8 " | b"VP8L" => {
                // A VP8L bitstream carries its own alpha, and the decoder
                // ignores an ALPH chunk with it.
                let alpha = alpha.filter(|_| chunk.fourcc == *b"VP8 ");
                let image = image(alpha, chunk)?;
                // libwebp's decoder and demuxer (CheckFrameBounds) both
                // refuse a still image that is not the size of its canvas;
                // the simple file the strip may write would have no canvas
                // left to disagree with.
                if image.dimensions != canvas {
                    return Err(DecodeDefect::WebpCanvasSize);
                }
                return Ok(image);
            }
            _ => {}
        }
        at = next;
    }
    Err(DecodeDefect::WebpNoImage)
}

/// What libwebp's demuxer reads from the chunks after the `VP8X` chunk, up
/// to where it stops.
#[derive(Debug, Default)]
struct Demuxed<'a> {
    /// The payload of the first flagged `ICCP` chunk.
    icc: Option<&'a [u8]>,
    /// The payload of the first flagged `EXIF` chunk.
    exif: Option<&'a [u8]>,
    /// The 6 bytes of the first `ANIM` chunk, its pad byte included when it
    /// declares 5.
    anim: Option<&'a [u8]>,
    /// The frames of an animation, in file order; never more than
    /// [`super::MAX_ANIMATION_FRAMES`].
    frames: Vec<Frame<'a>>,
    /// Why the demuxer refuses the file, where the walk stopped; `None` when
    /// it read every chunk.
    refused: Option<ImageError>,
}

/// The chunks after the `VP8X` chunk, as libwebp's demuxer reads them
/// ([`walk`]), with the reason it stopped, if it did.
///
/// - `flags`: the `VP8X` flags.
/// - `body`: everything after the `VP8X` chunk.
fn demux(flags: u8, body: &[u8]) -> Demuxed<'_> {
    let mut demuxed = Demuxed::default();
    if let Err(refusal) = walk(flags, body, &mut demuxed) {
        demuxed.refused = Some(refusal);
    }
    demuxed
}

/// Reads the chunks after the `VP8X` chunk into `demuxed` as libwebp's
/// `ParseVP8XChunks` reads them (the module documentation lists the rules),
/// and stops at the first one it refuses. A still image is only walked
/// through here, for the metadata and for where the demuxer stops:
/// [`decoded_still`] reads its image.
///
/// - `flags`: the `VP8X` flags.
/// - `body`: everything after the `VP8X` chunk.
/// - `demuxed`: what has been read so far.
///
/// # Errors
///
/// [`ImageError::Undecodable`] with what the demuxer refuses, and
/// [`ImageError::AnimationTooLarge`] at the frame that passes a limit of
/// ADR 0033 §4.
fn walk<'a>(flags: u8, body: &'a [u8], demuxed: &mut Demuxed<'a>) -> Result<(), ImageError> {
    let animation = flags & ANIMATION != 0;
    let mut still = false;
    let mut pixels: u64 = 0;
    let mut at = 0;
    while at < body.len() {
        let (chunk, next) = riff::chunk_at(body, at).map_err(undecodable)?;
        match &chunk.fourcc {
            b"VP8X" => return Err(undecodable(DecodeDefect::WebpSecondVp8x)),
            b"ALPH" | b"VP8 " | b"VP8L" => {
                // The demuxer refuses image chunks outside the frames of an
                // animation, also after an ANIM chunk without the flag, and
                // a second still image.
                if animation || demuxed.anim.is_some() {
                    return Err(undecodable(DecodeDefect::WebpImageOutsideFrames));
                }
                if still {
                    return Err(undecodable(DecodeDefect::WebpSecondImage));
                }
                let stored = store_frame(body, at).map_err(undecodable)?;
                // ParseSingleImage drops the alpha of a still image without
                // the flag, wherever it is; with the flag, the validation
                // requires the ALPH chunk before the bitstream.
                if flags & ALPHA != 0 && stored.alpha_follows_image {
                    return Err(undecodable(DecodeDefect::WebpAlphaAfterBitstream));
                }
                still = true;
                at = stored.end;
                continue;
            }
            b"ANIM" => {
                // The demuxer refuses every ANIM chunk whose padded size is
                // below 6 and reads the first one's 6 bytes from its padded
                // payload, so a pad byte can be part of them.
                let payload = body
                    .get(at + 8..at + 8 + ANIM_SIZE)
                    .filter(|_| next - (at + 8) >= ANIM_SIZE)
                    .ok_or_else(|| undecodable(DecodeDefect::WebpAnimSize))?;
                demuxed.anim = demuxed.anim.or(Some(payload));
            }
            b"ANMF" => {
                // The demuxer refuses a frame before the ANIM chunk, with or
                // without the animation flag.
                if demuxed.anim.is_none() {
                    return Err(undecodable(DecodeDefect::WebpFrameBeforeAnim));
                }
                let (frame, end) =
                    animation_frame(body, at, next, animation).map_err(undecodable)?;
                if let Some(frame) = frame {
                    pixels = pixels.saturating_add(frame.image.dimensions.pixels());
                    // At the frame that passes a limit, before it is kept, so
                    // the list never holds more than the most frames however
                    // many the file declares (ADR 0033 §4). It holds at most
                    // MAX_ANIMATION_FRAMES, so its length fits in a u64.
                    let kept = u64::try_from(demuxed.frames.len()).unwrap_or(u64::MAX);
                    check_animation_size(ImageFormat::WebP, kept.saturating_add(1), pixels)?;
                    demuxed.frames.push(frame);
                }
                // The demuxer goes on after the frame's image, inside the
                // ANMF chunk (see the module documentation).
                at = end;
                continue;
            }
            // The first flagged profile and EXIF block are the ones viewers
            // read (WebPDemuxGetChunk with index 1).
            b"ICCP" if flags & ICC != 0 => demuxed.icc = demuxed.icc.or(Some(chunk.payload)),
            b"EXIF" if flags & EXIF != 0 => demuxed.exif = demuxed.exif.or(Some(chunk.payload)),
            _ => {}
        }
        at = next;
    }
    Ok(())
}

/// The image chunks that libwebp's `StoreFrame` reads from `start` on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stored<'a> {
    /// The payload of the first `ALPH` chunk, if one was read.
    alpha: Option<&'a [u8]>,
    /// Whether that `ALPH` chunk came after the bitstream, which libwebp's
    /// validation refuses wherever it keeps the alpha.
    alpha_follows_image: bool,
    /// The `VP8 ` or `VP8L` chunk, if one was read.
    bitstream: Option<Chunk<'a>>,
    /// The offset after the last chunk read, where libwebp goes on.
    end: usize,
}

/// The image chunks from `start` on, as libwebp's `StoreFrame` reads them:
/// the first `ALPH` chunk and the first bitstream, in either order, until
/// another chunk, a second one of either, or the end. A `VP8L` bitstream
/// with an `ALPH` chunk is refused, and so is a bitstream header that
/// libwebp's `WebPGetFeatures` refuses, also in a frame that is never shown.
///
/// - `body`: everything after the `VP8X` chunk.
/// - `start`: the offset of the first chunk to read.
fn store_frame(body: &[u8], start: usize) -> Result<Stored<'_>, DecodeDefect> {
    // libwebp needs a chunk header here, and after every chunk it reads,
    // unless the file ends there; `chunk_at` refuses fewer than 8 bytes
    // as a chunk cut short, as libwebp does.
    let mut stored = Stored {
        alpha: None,
        alpha_follows_image: false,
        bitstream: None,
        end: start,
    };
    loop {
        let (chunk, next) = riff::chunk_at(body, stored.end)?;
        match &chunk.fourcc {
            b"ALPH" if stored.alpha.is_none() => {
                stored.alpha = Some(chunk.payload);
                stored.alpha_follows_image = stored.bitstream.is_some();
            }
            b"VP8L" if stored.alpha.is_some() => return Err(DecodeDefect::WebpLosslessAlpha),
            b"VP8 " | b"VP8L" if stored.bitstream.is_none() => {
                bitstream_dimensions(chunk.fourcc, chunk.payload)
                    .map_err(|_| DecodeDefect::WebpBitstreamHeader)?;
                stored.bitstream = Some(chunk);
            }
            _ => break,
        }
        stored.end = next;
        if stored.end == body.len() {
            break;
        }
    }
    Ok(stored)
}

/// An [`Image`] of `alpha` and `bitstream`, with the dimensions that the
/// bitstream's header declares, checked as the header check does.
///
/// - `alpha`: the `ALPH` payload to keep, if any.
/// - `bitstream`: the `VP8 ` or `VP8L` chunk.
fn image<'a>(alpha: Option<&'a [u8]>, bitstream: Chunk<'a>) -> Result<Image<'a>, DecodeDefect> {
    let dimensions = bitstream_dimensions(bitstream.fourcc, bitstream.payload)
        .map_err(|_| DecodeDefect::WebpBitstreamHeader)?;
    Ok(Image {
        alpha,
        bitstream,
        dimensions,
    })
}

/// An `ANMF` chunk, as libwebp's `ParseAnimationFrame` reads it: its 16-byte
/// header, then the image chunks right after it ([`store_frame`]), which
/// must end inside the chunk. Returns the frame, when there is one to show,
/// and the offset where the walk over the file goes on: after the frame's
/// image, inside the `ANMF` chunk.
///
/// - `body`: everything after the `VP8X` chunk.
/// - `at`: the offset of the `ANMF` chunk.
/// - `next`: the offset after the `ANMF` chunk and its pad byte.
/// - `animation`: whether the animation flag is set. Without it, libwebp
///   reads the chunk the same way but shows no frame of it.
fn animation_frame(
    body: &[u8],
    at: usize,
    next: usize,
    animation: bool,
) -> Result<(Option<Frame<'_>>, usize), DecodeDefect> {
    let start = at + 8;
    // The padded size counts, as for ANIM.
    let header = body
        .get(start..start + FRAME_HEADER)
        .filter(|_| next - start >= FRAME_HEADER)
        .ok_or(DecodeDefect::WebpFrameHeaderSize)?;
    let (width, height) = (u24(header, 6) + 1, u24(header, 9) + 1);
    if u64::from(width) * u64::from(height) >= MAX_IMAGE_AREA {
        return Err(DecodeDefect::WebpFrameArea);
    }
    let stored = store_frame(body, start + FRAME_HEADER)?;
    if stored.end > next {
        return Err(DecodeDefect::WebpFrameOverrun);
    }
    let frame = match (animation, stored.alpha, stored.bitstream) {
        // libwebp adds no frame without the flag, or without an image chunk.
        (false, ..) | (true, None, None) => None,
        (true, _, None) => return Err(DecodeDefect::WebpNoBitstream),
        (true, _, Some(_)) if stored.alpha_follows_image => {
            return Err(DecodeDefect::WebpAlphaAfterBitstream);
        }
        (true, alpha, Some(bitstream)) => Some(Frame {
            header,
            image: image(alpha, bitstream)?,
        }),
    };
    Ok((frame, stored.end))
}

/// The check that comes before any frame of an animation is decoded: every
/// frame must fit the canvas, as libwebp's demuxer requires
/// (`CheckFrameBounds`). The frame and pixel limits of ADR 0033 §4, which
/// bound the CPU time of the decode, held while [`walk`] read the frames.
///
/// - `frames`: every frame, in file order.
/// - `canvas`: the canvas that the header check read and budgeted.
///
/// # Errors
///
/// [`ImageError::Undecodable`] for the first frame that does not fit.
fn check_frames_fit(frames: &[Frame<'_>], canvas: Dimensions) -> Result<(), ImageError> {
    for frame in frames {
        let (left, top) = frame.offset();
        let size = frame.image.dimensions;
        if left + u64::from(size.width()) > u64::from(canvas.width())
            || top + u64::from(size.height()) > u64::from(canvas.height())
        {
            return Err(undecodable(DecodeDefect::WebpFrameOutsideCanvas));
        }
    }
    Ok(())
}

/// The re-encode of a lossy still image, if it passes the keep rule:
/// libwebp's new `VP8 ` bitstream for `image`, with the image's `ALPH`
/// chunk and the kept metadata of `parts`.
///
/// - `parts`: what the strip keeps.
/// - `image`: the lossy still image of `parts`.
/// - `canvas`: the canvas, which the re-encode keeps.
/// - `budget`: the pixel budget.
/// - `stripped_len`: the size of the stripped file, which the re-encode
///   must undercut by 5 %.
///
/// # Errors
///
/// Why the re-encode is not kept: the keep rule's reason
/// ([`keep_rule`]), or [`NotReencoded::EncoderFailed`] with libwebp's
/// reason when it failed.
fn re_encode(
    parts: &Parts<'_>,
    image: &Image<'_>,
    canvas: Dimensions,
    budget: PixelBudget,
    stripped_len: usize,
) -> Result<Vec<u8>, Discarded> {
    let (encoded, luma) =
        codec::re_encode(image.bitstream.payload, budget).map_err(Discarded::failed)?;
    // libwebp's own output, a simple file with one `VP8 ` chunk.
    let new = match parse(&encoded) {
        Ok(Parts {
            content: Content::Still(new),
            ..
        }) if !new.is_lossless() && new.alpha.is_none() && new.dimensions == image.dimensions => {
            new
        }
        _ => return Err(Discarded::failed("the encoder wrote another kind of image")),
    };
    let re_encoded = Parts {
        flags: parts.flags,
        icc: parts.icc,
        exif: parts.exif,
        content: Content::Still(Image {
            alpha: image.alpha,
            ..new
        }),
        // libwebp writes whole chunks, pad bytes included.
        unpadded: false,
    };
    let re_encoded = assemble(&re_encoded, canvas);
    keep_rule(stripped_len, re_encoded.len(), || {
        codec::luma_ssim(&luma, new.bitstream.payload, budget).map_err(|error| match error {
            codec::NotMeasured::TooSmall => NotReencoded::TooSmallForSsim.into(),
            codec::NotMeasured::Failed(why) => Discarded::failed(why),
        })
    })?;
    Ok(re_encoded)
}

/// The keep rule of ADR 0033 §1 for a re-encode of `re_encoded_len` bytes:
/// [`reencode_is_kept`] against the stripped file first, because it is
/// free; only then `ssim`, which decodes the re-encode again, and
/// [`reencode_meets_ssim_floor`] on its value.
///
/// - `stripped_len`: the size of the stripped file.
/// - `re_encoded_len`: the size of the re-encode, metadata included.
/// - `ssim`: measures the luma SSIM of the re-encode against the decoded
///   original, or says why it cannot.
///
/// # Errors
///
/// [`NotReencoded::NotSmallerEnough`], the error of `ssim`, or
/// [`NotReencoded::BelowSsimFloor`].
fn keep_rule(
    stripped_len: usize,
    re_encoded_len: usize,
    ssim: impl FnOnce() -> Result<f64, Discarded>,
) -> Result<(), Discarded> {
    if !reencode_is_kept(stripped_len, re_encoded_len) {
        return Err(NotReencoded::NotSmallerEnough.into());
    }
    if reencode_meets_ssim_floor(ssim()?) {
        Ok(())
    } else {
        Err(NotReencoded::BelowSsimFloor.into())
    }
}

/// What is stored for a lossy still image, with its [`Treatment`] and the
/// reason the re-encode failed, if it did: the re-encode when it passed the
/// keep rule, the stripped file otherwise (ADR 0015 §7, ADR 0033 §1).
///
/// - `stripped`: the stripped file, which has decoded.
/// - `re_encoded`: the re-encode that passed [`keep_rule`], or why it is
///   not stored.
fn choose(
    stripped: Vec<u8>,
    re_encoded: Result<Vec<u8>, Discarded>,
) -> (Vec<u8>, Treatment, Option<String>) {
    match re_encoded {
        Ok(bytes) => (bytes, Treatment::ReEncoded, None),
        Err(Discarded { reason, failure }) => {
            (stripped, Treatment::ReEncodeDiscarded(reason), failure)
        }
    }
}

/// The file that holds `parts`: the simple format when nothing needs `VP8X`,
/// otherwise `VP8X`, `ICCP`, the image chunks and `EXIF`, in the order of
/// RFC 9649 §2.7. It is written in one buffer, so the image data is copied
/// once.
///
/// - `parts`: what is kept.
/// - `canvas`: the canvas, which the header check read and which the strip
///   and the re-encode never change.
fn assemble(parts: &Parts<'_>, canvas: Dimensions) -> Vec<u8> {
    let exif = parts
        .exif
        .map(|kept| kept.orientation.tiff_replacing(kept.original_len));
    let write_content = |out: &mut Vec<u8>| match &parts.content {
        Content::Still(image) => image.write(out),
        Content::Animation { anim, frames } => {
            riff::write_chunk(out, *b"ANIM", anim);
            for frame in frames {
                riff::write_chunk_with(out, *b"ANMF", |out| {
                    out.extend_from_slice(frame.header);
                    frame.image.write(out);
                });
            }
        }
    };
    let (animation, alpha_chunk) = match &parts.content {
        Content::Still(image) => (false, image.alpha.is_some()),
        Content::Animation { .. } => (true, false),
    };
    if !animation && !alpha_chunk && parts.icc.is_none() && exif.is_none() {
        return riff::file_with(|out| {
            write_content(out);
            // Only a simple upload is unpadded, and it keeps nothing but its
            // bitstream, so the last byte is the pad byte that write_chunk
            // added and the upload did not have.
            if parts.unpadded {
                out.pop();
            }
        });
    }
    let mut flags = parts.flags & ALPHA;
    for (kept, flag) in [
        (animation, ANIMATION),
        (parts.icc.is_some(), ICC),
        (exif.is_some(), EXIF),
    ] {
        if kept {
            flags |= flag;
        }
    }
    // The canvas sides minus one, 24 bits each.
    let side = |side: u32| (side - 1).to_le_bytes();
    let (width, height) = (side(canvas.width()), side(canvas.height()));
    let vp8x = [
        flags, 0, 0, 0, width[0], width[1], width[2], height[0], height[1], height[2],
    ];
    riff::file_with(|out| {
        riff::write_chunk(out, *b"VP8X", &vp8x);
        if let Some(icc) = parts.icc {
            riff::write_chunk(out, *b"ICCP", icc);
        }
        write_content(out);
        if let Some(exif) = &exif {
            riff::write_chunk(out, *b"EXIF", exif);
        }
    })
}

/// Real WebP bitstreams for the tests, from libwebp's own encoders.
#[cfg(test)]
mod samples {
    use webpx::{AnimationEncoder, Encoder, Unstoppable};

    /// The width and height of every sample: wide enough for the 11-pixel
    /// window of the SSIM, so a lossy sample can pass the keep rule.
    pub(super) const SIZE: (u32, u32) = (16, 16);

    /// A `width`×`height` RGBA test card: two gradients and a constant,
    /// with `alpha`.
    pub(super) fn card_of(width: u32, height: u32, alpha: u8, shift: u8) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                let x = u8::try_from((i % width) * 255 / width).unwrap();
                let y = u8::try_from((i / width) * 255 / height).unwrap();
                [x.wrapping_add(shift), y, 128, alpha]
            })
            .collect()
    }

    /// The card at [`SIZE`].
    pub(super) fn card(alpha: u8, shift: u8) -> Vec<u8> {
        card_of(SIZE.0, SIZE.1, alpha, shift)
    }

    /// A still WebP of the card in libwebp's own layout.
    pub(super) fn still(alpha: u8, lossless: bool, quality: f32) -> Vec<u8> {
        encode(&card(alpha, 0), SIZE, lossless, quality)
    }

    /// A still WebP of `rgba`, `size.0`×`size.1` pixels.
    pub(super) fn encode(rgba: &[u8], size: (u32, u32), lossless: bool, quality: f32) -> Vec<u8> {
        Encoder::new_rgba(rgba, size.0, size.1)
            .lossless(lossless)
            .quality(quality)
            .encode(Unstoppable)
            .unwrap()
    }

    /// An opaque `side`×`side` picture of fixed pseudo-random noise, which a
    /// lossless encoder can hardly compress and a lossy one can, but only
    /// by losing the detail that an SSIM sees.
    pub(super) fn noise(side: u32) -> Vec<u8> {
        let mut state = 0x2545_F491_u32;
        (0..side * side)
            .flat_map(|_| {
                // xorshift32, so the picture is the same on every run.
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let [r, g, b, _] = state.to_le_bytes();
                [r, g, b, 255]
            })
            .collect()
    }

    /// An opaque grey `side`×`side` gradient with fine noise of ±8 levels
    /// on it, the kind of texture (foliage, fur, water) that a re-encode at
    /// quality 85 smooths: much smaller, but below the SSIM floor.
    pub(super) fn textured(side: u32) -> Vec<u8> {
        let mut state = 0x2545_F491_u32;
        (0..side * side)
            .flat_map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                // A gradient from 64 to 190, plus noise from -8 to 8.
                let base = 64 + (i % side) * 126 / side;
                let value = u8::try_from(base + state % 17 - 8).unwrap();
                [value, value, value, 255]
            })
            .collect()
    }

    /// An animated WebP of three different cards.
    pub(super) fn animation() -> Vec<u8> {
        let mut encoder = AnimationEncoder::new(SIZE.0, SIZE.1).unwrap();
        for (frame, shift) in [0, 80, 160].into_iter().enumerate() {
            let at = i32::try_from(frame).unwrap() * 100;
            encoder.add_frame_rgba(&card(255, shift), at).unwrap();
        }
        encoder.finish(300).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::super::header::samples as headers;
    use super::super::{ImageError, MAX_ANIMATION_FRAMES, MAX_ANIMATION_PIXELS, inspect};
    use super::samples::{SIZE, animation, encode, noise, still};
    use super::*;
    use proptest::{collection::vec, prelude::*, sample::select};
    use webpx::AnimationDecoder;

    /// An invented ICC profile: the 132 bytes of a header with no tags.
    const ICC_PROFILE: &[u8] = &[7; 132];

    /// EXIF with Orientation 6 and a GPS IFD that holds an invented
    /// latitude reference: header, IFD0 (2 entries), the GPS IFD (1 entry).
    const EXIF_WITH_GPS: &[u8] = b"II\x2A\0\x08\0\0\0\
        \x02\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\x25\x88\x04\0\x01\0\0\0\x26\0\0\0\0\0\0\0\
        \x01\0\x01\0\x02\0\x02\0\0\0N\0\0\0\0\0\0\0";

    /// The EXIF payload that replaces [`EXIF_WITH_GPS`]: Orientation 6 alone.
    fn orientation_6() -> Vec<u8> {
        Orientation::new(6).unwrap().to_tiff().to_vec()
    }

    /// The chunks of a file, as `(FourCC, payload)` pairs.
    fn chunk_list(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        let chunks = riff::chunks(riff::body(bytes).unwrap()).unwrap();
        chunks
            .iter()
            .map(|c| (c.fourcc, c.payload.to_vec()))
            .collect()
    }

    /// The `FourCC`s of a file's chunks.
    fn fourccs(bytes: &[u8]) -> Vec<[u8; 4]> {
        chunk_list(bytes)
            .into_iter()
            .map(|(fourcc, _)| fourcc)
            .collect()
    }

    /// The payload of the first `fourcc` chunk of a file, if it has one.
    fn payload(bytes: &[u8], fourcc: [u8; 4]) -> Option<Vec<u8>> {
        chunk_list(bytes)
            .into_iter()
            .find(|(f, _)| *f == fourcc)
            .map(|(_, p)| p)
    }

    /// The `VP8X` payload for `flags` and a `canvas` of width × height.
    fn vp8x(flags: u8, canvas: (u32, u32)) -> [u8; 10] {
        let (w, h) = ((canvas.0 - 1).to_le_bytes(), (canvas.1 - 1).to_le_bytes());
        [flags, 0, 0, 0, w[0], w[1], w[2], h[0], h[1], h[2]]
    }

    /// An extended file: `VP8X` with `flags` for a `canvas` of width ×
    /// height, then `chunks`.
    fn extended_on(flags: u8, canvas: (u32, u32), chunks: &[ChunkRef<'_>]) -> Vec<u8> {
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(flags, canvas));
        for (fourcc, payload) in chunks {
            riff::write_chunk(&mut body, *fourcc, payload);
        }
        riff::file(&body)
    }

    /// An extended file on the canvas of the samples.
    fn extended_file(flags: u8, chunks: &[ChunkRef<'_>]) -> Vec<u8> {
        extended_on(flags, SIZE, chunks)
    }

    /// The image chunks of a libwebp-encoded file, which has no metadata.
    fn image_chunks_of(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        chunk_list(bytes)
            .into_iter()
            .filter(|(f, _)| f != b"VP8X")
            .collect()
    }

    /// `extended_file` around the image chunks of `bitstream_file`, with
    /// `before` and `after` around them.
    fn with_metadata(
        flags: u8,
        bitstream_file: &[u8],
        before: &[ChunkRef<'_>],
        after: &[ChunkRef<'_>],
    ) -> Vec<u8> {
        let image = image_chunks_of(bitstream_file);
        extended_file(flags, &[before, &refs(&image), after].concat())
    }

    /// A chunk to write: its `FourCC` and its payload.
    type ChunkRef<'a> = ([u8; 4], &'a [u8]);

    /// Borrowed views of owned chunks, for [`extended_file`].
    fn refs(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<ChunkRef<'_>> {
        chunks.iter().map(|(f, p)| (*f, p.as_slice())).collect()
    }

    /// The 16-byte `ANMF` header of a frame at (`left`, `top`), which must
    /// be even, with a duration of 100 ms and no flags. The size fields say
    /// 1×1; libwebp takes the size from the bitstream.
    fn frame_header(left: u32, top: u32) -> Vec<u8> {
        let (x, y) = ((left / 2).to_le_bytes(), (top / 2).to_le_bytes());
        vec![
            x[0], x[1], x[2], y[0], y[1], y[2], 0, 0, 0, 0, 0, 0, 100, 0, 0, 0,
        ]
    }

    /// An `ANMF` payload: a frame header at (`left`, `top`), then `chunks`.
    fn frame(left: u32, top: u32, chunks: &[ChunkRef<'_>]) -> Vec<u8> {
        let mut payload = frame_header(left, top);
        for (fourcc, chunk) in chunks {
            riff::write_chunk(&mut payload, *fourcc, chunk);
        }
        payload
    }

    /// An animation on `canvas` with one `ANMF` chunk per payload.
    fn animation_on(canvas: (u32, u32), frames: &[Vec<u8>]) -> Vec<u8> {
        let anim: ChunkRef<'_> = (*b"ANIM", &[0; 6]);
        let chunks: Vec<ChunkRef<'_>> = std::iter::once(anim)
            .chain(frames.iter().map(|f| (*b"ANMF", f.as_slice())))
            .collect();
        extended_on(ANIMATION, canvas, &chunks)
    }

    /// The `VP8L` payload of a 1×1 lossless image, which decodes.
    fn pixel() -> Vec<u8> {
        let file = encode(&[10, 20, 30, 255], (1, 1), true, 75.0);
        payload(&file, *b"VP8L").unwrap()
    }

    fn optimise(bytes: &[u8]) -> Result<Optimised, ImageError> {
        optimise_webp(bytes, PixelBudget::DEFAULT)
    }

    fn corrupt(defect: DecodeDefect) -> Result<Optimised, ImageError> {
        Err(ImageError::Undecodable {
            format: ImageFormat::WebP,
            defect,
        })
    }

    /// What libwebp's decoder shows of a still image (`WebPDecodeRGBA`).
    fn rgba(bytes: &[u8]) -> Vec<u8> {
        webpx::decode_rgba(bytes).unwrap().0
    }

    /// What libwebp's demuxer shows of the first frame, a still image's
    /// included, through its animation decoder: the reading that browsers
    /// run first. `None` when it refuses the file.
    fn demuxed_rgba(bytes: &[u8]) -> Option<Vec<u8>> {
        let mut decoder = AnimationDecoder::new(bytes).ok()?;
        decoder.next_frame().ok().flatten().map(|frame| frame.data)
    }

    /// How many frames libwebp's own demuxer finds, or `None` when it
    /// refuses the file.
    fn libwebp_frames(bytes: &[u8]) -> Option<u32> {
        AnimationDecoder::new(bytes)
            .ok()
            .map(|decoder| decoder.info().frame_count)
    }

    fn discarded(reason: NotReencoded) -> Treatment {
        Treatment::ReEncodeDiscarded(reason)
    }

    #[test]
    fn a_simple_file_loses_what_follows_its_riff_size() {
        let original = still(255, true, 75.0);
        let appended = [&original[..], b"PK\x03\x04<script>alert(1)</script>"].concat();
        let optimised = optimise(&appended).unwrap();
        assert_eq!(optimised.bytes, original);
        assert_eq!(optimised.treatment, Treatment::Stripped);
        assert_eq!(optimised.re_encode_failure, None);
        assert_eq!(
            optimised.header.dimensions,
            Dimensions::new(SIZE.0, SIZE.1).unwrap()
        );
    }

    #[test]
    fn a_lossy_image_is_re_encoded_only_when_it_is_smaller_and_similar_enough() {
        // Quality 100 of a smooth card: smaller at 85, and alike.
        let fine = still(255, false, 100.0);
        let optimised = optimise(&fine).unwrap();
        assert_eq!(optimised.treatment, Treatment::ReEncoded);
        assert!(optimised.bytes.len() * 100 <= fine.len() * 95);
        assert_eq!(fourccs(&optimised.bytes), [*b"VP8 "]);
        // At quality 10 a re-encode at 85 only grows: the stripped file stays.
        let coarse = still(255, false, 10.0);
        let optimised = optimise(&coarse).unwrap();
        assert_eq!(
            (optimised.treatment, optimised.bytes),
            (discarded(NotReencoded::NotSmallerEnough), coarse)
        );
        // Fine texture at quality 100 re-encodes much smaller, but loses the
        // detail that the SSIM sees: below 0.98, the stripped file stays.
        let textured = encode(&samples::textured(64), (64, 64), false, 100.0);
        let optimised = optimise(&textured).unwrap();
        assert_eq!(
            (optimised.treatment, optimised.bytes),
            (discarded(NotReencoded::BelowSsimFloor), textured)
        );
        assert_eq!(optimised.re_encode_failure, None);
    }

    #[test]
    fn an_image_too_small_for_the_ssim_window_keeps_its_stripped_file() {
        // 16×10 is a row short of the 11×11 window; 11×11 is measured.
        let short = samples::card_of(16, 10, 255, 0);
        let input = encode(&short, (16, 10), false, 100.0);
        let optimised = optimise(&input).unwrap();
        assert_eq!(
            (optimised.treatment, optimised.bytes),
            (discarded(NotReencoded::TooSmallForSsim), input)
        );
        let square = samples::card_of(11, 11, 255, 0);
        let optimised = optimise(&encode(&square, (11, 11), false, 100.0)).unwrap();
        assert_ne!(
            optimised.treatment,
            discarded(NotReencoded::TooSmallForSsim)
        );
    }

    #[test]
    fn the_keep_rule_tests_the_size_before_it_measures() {
        let measured = &std::cell::Cell::new(0);
        let ssim = |value: f64| {
            move || {
                measured.set(measured.get() + 1);
                Ok(value)
            }
        };
        // 96 of 100 is not 5 % smaller, so the SSIM is never measured.
        assert_eq!(
            keep_rule(100, 96, ssim(1.0)),
            Err(NotReencoded::NotSmallerEnough.into())
        );
        assert_eq!(measured.get(), 0);
        assert_eq!(keep_rule(100, 95, ssim(0.98)), Ok(()));
        assert_eq!(
            keep_rule(100, 95, ssim(0.979_999)),
            Err(NotReencoded::BelowSsimFloor.into())
        );
        assert_eq!(measured.get(), 2);
        // A measurement that fails passes its reason on.
        assert_eq!(
            keep_rule(100, 50, || Err(NotReencoded::TooSmallForSsim.into())),
            Err(NotReencoded::TooSmallForSsim.into())
        );
        assert_eq!(
            keep_rule(100, 50, || Err(Discarded::failed("no memory"))),
            Err(Discarded::failed("no memory"))
        );
    }

    #[test]
    fn a_lossless_image_keeps_its_bitstream_where_a_lossy_one_would_be_smaller() {
        let pixels = noise(64);
        let lossless = encode(&pixels, (64, 64), true, 75.0);
        // The test means something only if a lossy re-encode would win.
        let lossy = encode(&pixels, (64, 64), false, codec::QUALITY);
        assert!(reencode_is_kept(lossless.len(), lossy.len()));
        let image = image_chunks_of(&lossless);
        let chunks = [&[(*b"ICCP", ICC_PROFILE)][..], &refs(&image)].concat();
        let input = extended_on(ICC, (64, 64), &chunks);
        let optimised = optimise(&input).unwrap();
        assert_eq!(optimised.treatment, Treatment::Stripped);
        assert_eq!(
            payload(&optimised.bytes, *b"VP8L"),
            payload(&lossless, *b"VP8L")
        );
        assert_eq!(rgba(&optimised.bytes), pixels);
    }

    #[test]
    fn metadata_goes_except_the_profile_and_the_orientation() {
        for (lossless, quality) in [(true, 75.0), (false, 100.0), (false, 10.0)] {
            let bitstream = still(255, lossless, quality);
            let input = with_metadata(
                ICC | EXIF | 0x04,
                &bitstream,
                &[(*b"ICCP", ICC_PROFILE), (*b"ABCD", b"unknown")],
                &[(*b"EXIF", EXIF_WITH_GPS), (*b"XMP ", b"<x:xmpmeta/>")],
            );
            let optimised = optimise(&input).unwrap();
            let bytes = &optimised.bytes;
            let image = if lossless { *b"VP8L" } else { *b"VP8 " };
            assert_eq!(fourccs(bytes), [*b"VP8X", *b"ICCP", image, *b"EXIF"]);
            assert_eq!(payload(bytes, *b"VP8X").unwrap()[0], ICC | EXIF);
            assert_eq!(payload(bytes, *b"ICCP").unwrap(), ICC_PROFILE);
            assert_eq!(payload(bytes, *b"EXIF").unwrap(), orientation_6());
            if optimised.treatment != Treatment::ReEncoded {
                assert_eq!(
                    payload(bytes, image),
                    payload(&bitstream, image),
                    "{quality}"
                );
                assert_eq!(rgba(bytes), rgba(&bitstream), "{quality}");
            }
            assert_eq!(rgba(bytes).len(), rgba(&bitstream).len());
            assert!(bytes.len() < input.len());
        }
    }

    #[test]
    fn exif_is_read_with_the_shared_reader_and_never_grows() {
        let bitstream = still(255, true, 75.0);
        let exif_of = |exif: &[u8]| {
            let input = with_metadata(EXIF, &bitstream, &[], &[(*b"EXIF", exif)]);
            payload(&optimise(&input).unwrap().bytes, *b"EXIF")
        };
        // IFD0 without its next-IFD offset (22 bytes) is read, as Chromium
        // reads it, and replaced by 22 bytes, not the 26 of the full form.
        let short = &orientation_6()[..ORIENTATION_TIFF_MIN];
        assert_eq!(exif_of(short), Some(short.to_vec()));
        // An Orientation in the EXIF sub-IFD counts too.
        let in_sub_ifd = b"II\x2A\0\x08\0\0\0\x01\0\x69\x87\x04\0\x01\0\0\0\x1A\0\0\0\0\0\0\0\
            \x01\0\x12\x01\x03\0\x01\0\0\0\x03\0\0\0\0\0\0\0";
        assert_eq!(
            exif_of(in_sub_ifd),
            Some(Orientation::new(3).unwrap().to_tiff().to_vec())
        );
        // An `Exif` identifier in front of the TIFF structure is not read,
        // as Skia's parser does not read it: the stored file has no EXIF.
        let prefixed = [b"Exif\0\0".as_slice(), EXIF_WITH_GPS].concat();
        assert_eq!(exif_of(&prefixed), None);
    }

    /// The length of the short replacement of the shared EXIF reader.
    const ORIENTATION_TIFF_MIN: usize = super::super::ORIENTATION_TIFF_MIN_LEN;

    #[test]
    fn only_the_first_anim_profile_and_exif_count() {
        let second_exif = Orientation::new(3).unwrap().to_tiff();
        let input = with_metadata(
            ICC | EXIF,
            &still(255, true, 75.0),
            &[(*b"ICCP", ICC_PROFILE), (*b"ICCP", b"second")],
            &[(*b"EXIF", EXIF_WITH_GPS), (*b"EXIF", &second_exif)],
        );
        let bytes = optimise(&input).unwrap().bytes;
        assert_eq!(payload(&bytes, *b"ICCP").unwrap(), ICC_PROFILE);
        assert_eq!(payload(&bytes, *b"EXIF").unwrap(), orientation_6());
        assert_eq!(fourccs(&bytes), [*b"VP8X", *b"ICCP", *b"VP8L", *b"EXIF"]);
        // libwebp's demuxer reads the same first chunks.
        assert_eq!(
            webpx::get_icc_profile(&input).unwrap().as_deref(),
            Some(ICC_PROFILE)
        );
        assert_eq!(
            webpx::get_exif(&input).unwrap().as_deref(),
            Some(EXIF_WITH_GPS)
        );
        // An ANIM chunk keeps its 6 bytes; what follows them and a second
        // ANIM chunk are dropped.
        let pixel = pixel();
        let one_frame = frame(0, 0, &[(*b"VP8L", &pixel)]);
        let first: &[u8] = b"\x01\x02\x03\x04\x05\x00hidden";
        let input = extended_on(
            ANIMATION,
            (1, 1),
            &[
                (*b"ANIM", first),
                (*b"ANIM", b"\x09\x09\x09\x09\x09\x09"),
                (*b"ANMF", &one_frame),
            ],
        );
        let bytes = optimise(&input).unwrap().bytes;
        assert_eq!(payload(&bytes, *b"ANIM").unwrap(), &first[..6]);
        assert_eq!(fourccs(&bytes), [*b"VP8X", *b"ANIM", *b"ANMF"]);
        assert!(!bytes.windows(6).any(|w| w == b"hidden"));
    }

    #[test]
    fn an_anim_chunk_of_5_bytes_is_read_with_its_pad_byte() {
        let pixel = pixel();
        let one_frame = frame(0, 0, &[(*b"VP8L", &pixel)]);
        // ANIM declares 5 bytes; its pad byte, 0x07, is the loop count's
        // high byte for libwebp.
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(ANIMATION, (1, 1)));
        body.extend_from_slice(b"ANIM\x05\0\0\0\x01\x02\x03\x04\x05\x07");
        riff::write_chunk(&mut body, *b"ANMF", &one_frame);
        let input = riff::file(&body);
        let optimised = optimise(&input).unwrap();
        assert_eq!(
            payload(&optimised.bytes, *b"ANIM").unwrap(),
            b"\x01\x02\x03\x04\x05\x07"
        );
        let loops = |bytes: &[u8]| AnimationDecoder::new(bytes).unwrap().info().loop_count;
        assert_eq!(loops(&input), 0x0705);
        assert_eq!(loops(&optimised.bytes), loops(&input));
        // Never larger: 5 bytes and the pad byte become 6 bytes.
        assert_eq!(optimised.bytes.len(), input.len());
        // A padded size below 6 is refused, by libwebp too.
        for short in [&[0; 4][..], &[0; 3]] {
            let input = extended_on(
                ANIMATION,
                (1, 1),
                &[(*b"ANIM", short), (*b"ANMF", &one_frame)],
            );
            assert_eq!(optimise(&input), corrupt(DecodeDefect::WebpAnimSize));
            assert_eq!(libwebp_frames(&input), None);
        }
    }

    #[test]
    fn chunks_after_a_frames_image_are_read_as_the_files() {
        let pixel = pixel();
        let image: ChunkRef<'_> = (*b"VP8L", &pixel);
        // A profile and EXIF inside the first frame, after its image, count
        // as the file's, and come first in file order.
        let first = frame(
            0,
            0,
            &[image, (*b"ICCP", ICC_PROFILE), (*b"EXIF", EXIF_WITH_GPS)],
        );
        let second = frame(0, 0, &[image]);
        let input = extended_on(
            ANIMATION | ICC | EXIF,
            (1, 1),
            &[
                (*b"ANIM", &[0; 6]),
                (*b"ANMF", &first),
                (*b"ANMF", &second),
                (*b"ICCP", b"later"),
            ],
        );
        assert_eq!(
            webpx::get_icc_profile(&input).unwrap().as_deref(),
            Some(ICC_PROFILE)
        );
        let bytes = optimise(&input).unwrap().bytes;
        assert_eq!(payload(&bytes, *b"ICCP").unwrap(), ICC_PROFILE);
        assert_eq!(payload(&bytes, *b"EXIF").unwrap(), orientation_6());
        assert_eq!(
            fourccs(&bytes),
            [*b"VP8X", *b"ICCP", *b"ANIM", *b"ANMF", *b"ANMF", *b"EXIF"]
        );
        assert_eq!(libwebp_frames(&bytes), Some(2));
        // A frame nested after a frame's image is libwebp's next frame.
        let nested = [frame(0, 0, &[image]), {
            let mut inner = Vec::new();
            riff::write_chunk(&mut inner, *b"ANMF", &frame(0, 0, &[image]));
            inner
        }]
        .concat();
        let input = animation_on((1, 1), &[nested]);
        assert_eq!(libwebp_frames(&input), Some(2));
        let bytes = optimise(&input).unwrap().bytes;
        assert_eq!(fourccs(&bytes), [*b"VP8X", *b"ANIM", *b"ANMF", *b"ANMF"]);
        assert_eq!(libwebp_frames(&bytes), Some(2));
        // An image chunk there is an image outside a frame, which libwebp
        // refuses.
        let doubled = frame(0, 0, &[image, image]);
        let input = animation_on((1, 1), &[doubled]);
        assert_eq!(libwebp_frames(&input), None);
        assert_eq!(
            optimise(&input),
            corrupt(DecodeDefect::WebpImageOutsideFrames)
        );
    }

    #[test]
    fn an_anmf_chunk_without_an_image_is_not_a_frame() {
        let pixel = pixel();
        let image: ChunkRef<'_> = (*b"VP8L", &pixel);
        let real = frame(0, 0, &[image]);
        // A header alone, a header with an unknown chunk, and a 15-byte
        // ANMF chunk whose pad byte completes the header: no frame each.
        let unknown = frame(0, 0, &[(*b"ABCD", b"x")]);
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(ANIMATION, (1, 1)));
        riff::write_chunk(&mut body, *b"ANIM", &[0; 6]);
        riff::write_chunk(&mut body, *b"ANMF", &frame_header(0, 0));
        riff::write_chunk(&mut body, *b"ANMF", &unknown);
        riff::write_chunk(&mut body, *b"ANMF", &frame_header(0, 0)[..15]);
        riff::write_chunk(&mut body, *b"ANMF", &real);
        let input = riff::file(&body);
        assert_eq!(libwebp_frames(&input), Some(1));
        let bytes = optimise(&input).unwrap().bytes;
        assert_eq!(fourccs(&bytes), [*b"VP8X", *b"ANIM", *b"ANMF"]);
        // An ALPH chunk without a bitstream is a frame cut short; libwebp
        // refuses it.
        let alpha_only = animation_on((1, 1), &[frame(0, 0, &[(*b"ALPH", b"a")]), real.clone()]);
        assert_eq!(libwebp_frames(&alpha_only), None);
        assert_eq!(
            optimise(&alpha_only),
            corrupt(DecodeDefect::WebpNoBitstream)
        );
        // An empty frame header that ends the file leaves libwebp waiting
        // for a chunk, which a whole file refuses.
        let last = animation_on((1, 1), &[real.clone(), frame_header(0, 0)]);
        assert_eq!(libwebp_frames(&last), None);
        assert_eq!(optimise(&last), corrupt(DecodeDefect::WebpCutShort));
        // An image that starts in the next chunk runs past its frame.
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(ANIMATION, (1, 1)));
        riff::write_chunk(&mut body, *b"ANIM", &[0; 6]);
        riff::write_chunk(&mut body, *b"ANMF", &frame_header(0, 0));
        riff::write_chunk(&mut body, *b"VP8L", &pixel);
        let past = riff::file(&body);
        assert_eq!(libwebp_frames(&past), None);
        assert_eq!(optimise(&past), corrupt(DecodeDefect::WebpFrameOverrun));
    }

    #[test]
    fn a_frames_alpha_must_come_before_its_bitstream() {
        let transparent = image_chunks_of(&still(128, false, 75.0));
        let (alpha, bitstream) = (refs(&transparent)[0], refs(&transparent)[1]);
        assert_eq!((alpha.0, bitstream.0), (*b"ALPH", *b"VP8 "));
        let before = animation_on(SIZE, &[frame(0, 0, &[alpha, bitstream])]);
        assert_eq!(libwebp_frames(&before), Some(1));
        assert!(optimise(&before).is_ok());
        // libwebp's validation refuses an alpha chunk after the bitstream in
        // a frame, where no flag drops it.
        let after = animation_on(SIZE, &[frame(0, 0, &[bitstream, alpha])]);
        assert_eq!(libwebp_frames(&after), None);
        assert_eq!(
            optimise(&after),
            corrupt(DecodeDefect::WebpAlphaAfterBitstream)
        );
    }

    #[test]
    fn an_anmf_header_of_2_to_the_32_pixels_is_refused() {
        let pixel = pixel();
        // 65536×65536 in the size fields (minus one: 0xFFFF each); the
        // bitstream is 1×1.
        let mut big = frame(0, 0, &[(*b"VP8L", &pixel)]);
        big[6..12].copy_from_slice(&[0xFF, 0xFF, 0, 0xFF, 0xFF, 0]);
        let input = animation_on((1, 1), &[big.clone()]);
        assert_eq!(libwebp_frames(&input), None);
        assert_eq!(optimise(&input), corrupt(DecodeDefect::WebpFrameArea));
        // One row less is below the limit, and the bitstream's size counts.
        big[9..12].copy_from_slice(&[0xFE, 0xFF, 0]);
        let input = animation_on((1, 1), &[big]);
        assert_eq!(libwebp_frames(&input), Some(1));
        assert!(optimise(&input).is_ok());
    }

    #[test]
    fn a_still_image_reads_an_anmf_chunk_but_shows_no_frame_of_it() {
        let bitstream = image_chunks_of(&still(255, true, 75.0));
        let pixel = pixel();
        let unshown = frame(0, 0, &[(*b"VP8L", &pixel), (*b"ICCP", ICC_PROFILE)]);
        let chunks = [
            &refs(&bitstream)[..],
            &[(*b"ANIM", &[0; 6][..]), (*b"ANMF", &unshown)],
        ]
        .concat();
        let input = extended_file(ICC, &chunks);
        assert_eq!(
            webpx::get_icc_profile(&input).unwrap().as_deref(),
            Some(ICC_PROFILE)
        );
        let bytes = optimise(&input).unwrap().bytes;
        assert_eq!(fourccs(&bytes), [*b"VP8X", *b"ICCP", *b"VP8L"]);
        assert_eq!(payload(&bytes, *b"ICCP").unwrap(), ICC_PROFILE);
        // Without the animation flag, libwebp adds no frame, so an ALPH
        // chunk without a bitstream there is not refused either.
        let alpha_only = frame(0, 0, &[(*b"ALPH", b"a")]);
        let chunks = [
            &refs(&bitstream)[..],
            &[(*b"ANIM", &[0; 6][..]), (*b"ANMF", &alpha_only)],
        ]
        .concat();
        let input = extended_file(0, &chunks);
        // libwebp's demuxer accepts the file (it finds no profile in it).
        assert_eq!(webpx::get_icc_profile(&input).ok(), Some(None));
        assert_eq!(optimise(&input).unwrap().bytes, still(255, true, 75.0));
    }

    #[test]
    fn reserved_flags_are_written_clear_in_a_still_and_refuse_an_animation() {
        assert_eq!(!VALID_FLAGS, 0xC1);
        let bitstream = still(255, true, 75.0);
        let input = with_metadata(0xC1 | ICC, &bitstream, &[(*b"ICCP", ICC_PROFILE)], &[]);
        // libwebp's decoder reads the still image; its demuxer does not.
        assert_eq!(rgba(&input), rgba(&bitstream));
        assert_eq!(libwebp_frames(&input), None);
        let bytes = optimise(&input).unwrap().bytes;
        assert_eq!(payload(&bytes, *b"VP8X").unwrap()[0], ICC);
        assert_eq!(
            webpx::get_icc_profile(&bytes).unwrap().as_deref(),
            Some(ICC_PROFILE)
        );
        // An animation is libwebp's demuxer's to read, and it refuses one
        // with any reserved flag.
        let pixel = pixel();
        let one_frame = frame(0, 0, &[(*b"VP8L", &pixel)]);
        for reserved in [0x01, 0x40, 0x80] {
            let mut input = animation_on((1, 1), std::slice::from_ref(&one_frame));
            assert_eq!(libwebp_frames(&input), Some(1));
            // The VP8X flags sit after the RIFF header and the chunk header.
            input[20] |= reserved;
            assert_eq!(libwebp_frames(&input), None, "{reserved:#04x}");
            assert_eq!(
                optimise(&input),
                corrupt(DecodeDefect::WebpReservedFlags),
                "{reserved:#04x}"
            );
        }
    }

    #[test]
    fn a_still_image_must_have_the_size_of_its_canvas() {
        let image = image_chunks_of(&still(255, true, 75.0));
        // XMP alone: the strip would write a simple file with no canvas.
        // With a profile, the VP8X canvas would stay and disagree.
        for (flags, before) in [(0x04, vec![]), (ICC, vec![(*b"ICCP", ICC_PROFILE)])] {
            for canvas in [(67, 41), (16, 15), (17, 16)] {
                let xmp: ChunkRef<'_> = (*b"XMP ", b"x");
                let chunks = [&before[..], &refs(&image), &[xmp]].concat();
                let input = extended_on(flags, canvas, &chunks);
                assert_eq!(
                    optimise(&input),
                    corrupt(DecodeDefect::WebpCanvasSize),
                    "{canvas:?}"
                );
            }
        }
    }

    #[test]
    fn unflagged_or_empty_metadata_goes_and_the_file_becomes_simple() {
        let bitstream = still(255, true, 75.0);
        let no_orientation = b"II\x2A\0\x08\0\0\0\x01\0\x0F\x01\x02\0\x04\0\0\0Cam\0\0\0\0\0";
        for (flags, exif) in [(0, EXIF_WITH_GPS), (EXIF, &no_orientation[..])] {
            let input = with_metadata(
                flags,
                &bitstream,
                &[(*b"ICCP", ICC_PROFILE)],
                &[(*b"EXIF", exif)],
            );
            assert_eq!(optimise(&input).unwrap().bytes, bitstream);
        }
    }

    #[test]
    fn a_still_keeps_the_alpha_that_libwebps_decoder_reads() {
        let transparent = still(128, false, 10.0);
        assert_eq!(fourccs(&transparent), [*b"VP8X", *b"ALPH", *b"VP8 "]);
        let optimised = optimise(&transparent).unwrap();
        assert_eq!(optimised.bytes, transparent);
        let chunks = image_chunks_of(&transparent);
        let (alph, vp8) = (refs(&chunks)[0], refs(&chunks)[1]);
        let alpha_of =
            |rgba: &[u8]| -> Vec<u8> { rgba.iter().skip(3).step_by(4).copied().collect() };
        let opaque = vec![255; alpha_of(&rgba(&transparent)).len()];
        // Before the bitstream without the flag: the decoder reads the
        // alpha, the demuxer does not. Kept as it is, without the flag, the
        // stored file shows each reader what the upload showed it.
        let unflagged = extended_file(0, &[alph, vp8]);
        let bytes = optimise(&unflagged).unwrap().bytes;
        assert_eq!(fourccs(&bytes), [*b"VP8X", *b"ALPH", *b"VP8 "]);
        assert_eq!(payload(&bytes, *b"VP8X").unwrap()[0], 0);
        assert_eq!(rgba(&bytes), rgba(&unflagged));
        assert_eq!(alpha_of(&rgba(&bytes)), alpha_of(&rgba(&transparent)));
        assert_eq!(demuxed_rgba(&bytes), demuxed_rgba(&unflagged));
        assert_eq!(alpha_of(&demuxed_rgba(&bytes).unwrap()), opaque);
        // After the bitstream, neither reader reads it, with or without the
        // flag; the demuxer refuses the flagged one, the decoder shows the
        // image opaque, and so does the stored file.
        for flags in [0, ALPHA] {
            let after = extended_file(flags, &[vp8, alph]);
            let bytes = optimise(&after).unwrap().bytes;
            assert_eq!(fourccs(&bytes), [*b"VP8 "], "{flags}");
            assert_eq!(rgba(&bytes), rgba(&after), "{flags}");
            assert_eq!(alpha_of(&rgba(&bytes)), opaque, "{flags}");
            assert_eq!(libwebp_frames(&after).is_some(), flags == 0, "{flags}");
        }
        // Of two ALPH chunks before the bitstream, the decoder reads the
        // last; the demuxer refuses the file.
        let other = image_chunks_of(&still(200, false, 10.0));
        let two = extended_file(ALPHA, &[refs(&other)[0], alph, vp8]);
        assert_eq!(libwebp_frames(&two), None);
        let bytes = optimise(&two).unwrap().bytes;
        assert_eq!(payload(&bytes, *b"ALPH").unwrap(), alph.1);
        assert_eq!(rgba(&bytes), rgba(&two));
        assert_eq!(libwebp_frames(&bytes), Some(1));
        // A VP8L bitstream has its own alpha: the decoder ignores an ALPH
        // chunk with it, the demuxer refuses the file.
        let lossless = image_chunks_of(&still(128, true, 75.0));
        let with_alph = extended_file(ALPHA, &[alph, refs(&lossless)[0]]);
        assert_eq!(libwebp_frames(&with_alph), None);
        let bytes = optimise(&with_alph).unwrap().bytes;
        assert_eq!(fourccs(&bytes), [*b"VP8L"]);
        assert_eq!(rgba(&bytes), rgba(&with_alph));
        assert_eq!(libwebp_frames(&bytes), Some(1));
        // A transparent image is re-encoded from its VP8 bitstream, and its
        // ALPH chunk is kept as it is.
        let fine_input = still(128, false, 100.0);
        let fine = optimise(&fine_input).unwrap();
        assert_eq!(fine.treatment, Treatment::ReEncoded);
        assert_eq!(fourccs(&fine.bytes), [*b"VP8X", *b"ALPH", *b"VP8 "]);
        assert_eq!(payload(&fine.bytes, *b"VP8X").unwrap()[0], ALPHA);
        assert_eq!(
            payload(&fine.bytes, *b"ALPH"),
            payload(&fine_input, *b"ALPH")
        );
        let alpha =
            |bytes: &[u8]| -> Vec<u8> { rgba(bytes).into_iter().skip(3).step_by(4).collect() };
        assert_eq!(alpha(&fine.bytes), alpha(&fine_input));
    }

    #[test]
    fn a_failed_or_discarded_re_encode_keeps_the_stripped_file() {
        let stripped = vec![1; 100];
        assert_eq!(
            choose(stripped.clone(), Err(Discarded::failed("out of memory"))),
            (
                stripped.clone(),
                discarded(NotReencoded::EncoderFailed),
                Some("out of memory".into())
            )
        );
        assert_eq!(
            choose(stripped.clone(), Err(NotReencoded::BelowSsimFloor.into())),
            (
                stripped.clone(),
                discarded(NotReencoded::BelowSsimFloor),
                None
            )
        );
        assert_eq!(
            choose(stripped, Ok(vec![2; 95])),
            (vec![2; 95], Treatment::ReEncoded, None)
        );
    }

    #[test]
    fn an_animation_keeps_every_frame() {
        let original = animation();
        let frames = chunk_list(&original);
        assert_eq!(
            fourccs(&original),
            [*b"VP8X", *b"ANIM", *b"ANMF", *b"ANMF", *b"ANMF"]
        );
        // Metadata, an unknown chunk at the end of a frame, and an appended payload.
        let mut chunks: Vec<([u8; 4], Vec<u8>)> = frames[1..].to_vec();
        let mut first = chunks[1].1.clone();
        riff::write_chunk(&mut first, *b"ABCD", b"hidden");
        chunks[1].1 = first;
        chunks.push((*b"XMP ", b"<x:xmpmeta/>".to_vec()));
        let input = [
            &extended_file(frames[0].1[0], &refs(&chunks))[..],
            b"trailing",
        ]
        .concat();
        let optimised = optimise(&input).unwrap();
        assert_eq!(optimised.bytes, original);
        assert_eq!(optimised.treatment, Treatment::Stripped);
        assert_eq!(libwebp_frames(&optimised.bytes), Some(3));
    }

    #[test]
    fn an_animation_frame_that_does_not_decode_or_fit_is_refused() {
        let pixel = pixel();
        let mut damaged = still(255, true, 75.0);
        let end = damaged.len() - 1;
        damaged[25..end].fill(0xFF);
        let damaged = payload(&damaged, *b"VP8L").unwrap();
        // The chunk structure holds; libwebp refuses the bitstream.
        let input = animation_on(
            SIZE,
            &[
                frame(0, 0, &[(*b"VP8L", &pixel)]),
                frame(0, 0, &[(*b"VP8L", &damaged)]),
            ],
        );
        assert_eq!(optimise(&input), corrupt(DecodeDefect::WebpData));
        // A 1×1 frame fits at (14, 14), not at (16, 0) or (0, 16).
        let at = |left, top| animation_on(SIZE, &[frame(left, top, &[(*b"VP8L", &pixel)])]);
        assert!(optimise(&at(14, 14)).is_ok());
        for (left, top) in [(16, 0), (0, 16)] {
            assert_eq!(
                optimise(&at(left, top)),
                corrupt(DecodeDefect::WebpFrameOutsideCanvas),
                "{left}, {top}"
            );
        }
    }

    #[test]
    fn animations_past_the_limits_are_refused_before_any_decode() {
        let pixel = pixel();
        let one = frame(0, 0, &[(*b"VP8L", &pixel)]);
        // The most frames decode; one more is refused. The canvas of 49
        // megapixels costs nothing, since frames are not composed onto it.
        let canvas = (7000, 7000);
        let mut frames = vec![one; usize::try_from(MAX_ANIMATION_FRAMES).unwrap()];
        assert!(optimise(&animation_on(canvas, &frames)).is_ok());
        frames.push(frames[0].clone());
        assert_eq!(
            optimise(&animation_on(canvas, &frames)),
            Err(ImageError::AnimationTooLarge {
                format: ImageFormat::WebP,
                frames: MAX_ANIMATION_FRAMES + 1,
                pixels: MAX_ANIMATION_FRAMES + 1,
            })
        );
        // 16 frames of 4096×4096 hold exactly the most pixels (only their
        // headers, which would not decode): they reach the decoder, which
        // refuses them. A 1×1 frame more is refused before any decode.
        let side = 4096;
        let header_only = headers::webp_lossless(side, side)[20..].to_vec();
        let big = frame(0, 0, &[(*b"VP8L", &header_only)]);
        let mut frames = vec![big; 16];
        assert_eq!(16 * u64::from(side) * u64::from(side), MAX_ANIMATION_PIXELS);
        assert_eq!(
            optimise(&animation_on((side.into(), side.into()), &frames)),
            corrupt(DecodeDefect::WebpData)
        );
        frames.push(frame(0, 0, &[(*b"VP8L", &pixel)]));
        assert_eq!(
            optimise(&animation_on((side.into(), side.into()), &frames)),
            Err(ImageError::AnimationTooLarge {
                format: ImageFormat::WebP,
                frames: 17,
                pixels: MAX_ANIMATION_PIXELS + 1
            })
        );
    }

    #[test]
    fn malformed_extended_files_are_refused() {
        let image = image_chunks_of(&still(255, false, 50.0));
        let vp8: &[u8] = &image[0].1;
        let lossless = image_chunks_of(&still(255, true, 75.0));
        let vp8l: &[u8] = &lossless[0].1;
        // A frame with no image chunk, only an unknown one: no frame. (A
        // bare frame header that ends the file is a chunk cut short.)
        let empty_frame = frame(0, 0, &[(*b"ABCD", b"x")]);
        // A frame whose bitstream header is cut short: the frame walk reads
        // it as the header check reads a first chunk.
        let cut_bitstream = frame(0, 0, &[(*b"VP8L", &vp8l[..4])]);
        let frame = [&[0; 16][..], &{
            let mut bytes = Vec::new();
            riff::write_chunk(&mut bytes, *b"VP8 ", vp8);
            bytes
        }]
        .concat();
        let anim: &[u8] = &[0; 6];
        let lossless_frame = [&[0; 16][..], &{
            let mut bytes = Vec::new();
            riff::write_chunk(&mut bytes, *b"ALPH", b"a");
            riff::write_chunk(&mut bytes, *b"VP8L", vp8l);
            bytes
        }]
        .concat();
        // Refused by libwebp's decoder, which reads a still image, and by
        // its demuxer, which reads an animation; libwebp_frames checks the
        // demuxer below.
        let cases: [(u8, Vec<ChunkRef>, DecodeDefect); 11] = [
            (0, vec![(*b"XMP ", b"x")], DecodeDefect::WebpNoImage),
            (
                0,
                vec![(*b"VP8L", &vp8l[..4])],
                DecodeDefect::WebpBitstreamHeader,
            ),
            (
                ANIMATION,
                vec![(*b"ANIM", anim), (*b"VP8X", &[0; 10]), (*b"ANMF", &frame)],
                DecodeDefect::WebpSecondVp8x,
            ),
            (
                ANIMATION,
                vec![(*b"ANIM", anim), (*b"ANMF", &lossless_frame)],
                DecodeDefect::WebpLosslessAlpha,
            ),
            (
                ANIMATION,
                vec![(*b"ANIM", anim), (*b"VP8 ", vp8)],
                DecodeDefect::WebpImageOutsideFrames,
            ),
            (
                ANIMATION,
                vec![(*b"ANIM", anim), (*b"ANMF", &frame), (*b"VP8 ", vp8)],
                DecodeDefect::WebpImageOutsideFrames,
            ),
            (
                ANIMATION,
                vec![(*b"ANMF", &frame), (*b"ANIM", anim)],
                DecodeDefect::WebpFrameBeforeAnim,
            ),
            // An animation with no frame that holds an image, the WebP
            // counterpart of a GIF with no image (ADR 0033 §3, D104).
            (ANIMATION, vec![(*b"ANIM", anim)], DecodeDefect::WebpNoFrame),
            (
                ANIMATION,
                vec![(*b"ANIM", anim), (*b"ANMF", &empty_frame)],
                DecodeDefect::WebpNoFrame,
            ),
            (
                ANIMATION,
                vec![(*b"ANIM", anim), (*b"ANMF", &cut_bitstream)],
                DecodeDefect::WebpBitstreamHeader,
            ),
            (
                ANIMATION,
                vec![(*b"ANIM", anim), (*b"ANMF", &frame[..10])],
                DecodeDefect::WebpFrameHeaderSize,
            ),
        ];
        for (flags, chunks, reason) in cases {
            let input = extended_file(flags, &chunks);
            assert_eq!(optimise(&input), corrupt(reason), "{reason}");
            if flags & ANIMATION != 0 {
                assert_eq!(libwebp_frames(&input), None, "{reason}");
            } else {
                assert!(webpx::decode_rgba(&input).is_err(), "{reason}");
            }
        }
        // Every ANIM chunk must have a padded size of 6, not only the first.
        let input = extended_file(
            ANIMATION,
            &[(*b"ANIM", anim), (*b"ANIM", &[0; 4]), (*b"ANMF", &frame)],
        );
        assert_eq!(optimise(&input), corrupt(DecodeDefect::WebpAnimSize));
        assert_eq!(libwebp_frames(&input), None);
    }

    #[test]
    fn the_strip_refuses_itself_what_the_header_check_refuses_first() {
        // The header check refuses these uploads before the strip runs. The
        // strip also parses libwebp's own re-encode, which no header check
        // has seen, so it refuses them with its own defect too.
        let wrapped = |fourcc: [u8; 4], payload: &[u8]| {
            let mut body = Vec::new();
            riff::write_chunk(&mut body, fourcc, payload);
            riff::file(&body)
        };
        let refused = |defect| Err(undecodable(defect));
        let unknown_first = wrapped(*b"ABCD", b"x");
        assert!(matches!(
            optimise(&unknown_first),
            Err(ImageError::Corrupt {
                format: ImageFormat::WebP,
                ..
            })
        ));
        assert_eq!(
            parse(&unknown_first).map(|_| ()),
            refused(DecodeDefect::WebpFirstChunk)
        );
        let header = vp8x(0, SIZE);
        let long = [header.as_slice(), &[0, 0]].concat();
        for payload in [&header[..9], &long[..]] {
            let input = wrapped(*b"VP8X", payload);
            assert!(optimise(&input).is_err(), "{}", payload.len());
            assert_eq!(
                parse(&input).map(|_| ()),
                refused(DecodeDefect::WebpVp8x),
                "{}",
                payload.len()
            );
        }
    }

    #[test]
    fn files_cut_short_or_undecodable_are_refused() {
        let whole = still(255, true, 75.0);
        assert_eq!(
            optimise(&whole[..whole.len() - 1]),
            corrupt(DecodeDefect::WebpRiffSize)
        );
        let mut damaged = whole.clone();
        let last = damaged.len() - 1;
        damaged[25..last].fill(0xFF);
        assert_eq!(optimise(&damaged), corrupt(DecodeDefect::WebpData));
        assert_eq!(
            optimise(&headers::gif(1, 1)),
            Err(ImageError::WrongFormat {
                expected: ImageFormat::WebP,
                found: ImageFormat::Gif
            })
        );
        assert_eq!(
            optimise(&headers::webp_lossless(10_000, 5_001)),
            Err(ImageError::OverPixelBudget {
                format: ImageFormat::WebP,
                dimensions: Dimensions::new(10_000, 5_001).unwrap(),
                budget: PixelBudget::DEFAULT,
            })
        );
        // Fewer than 8 bytes before a still image's bitstream, inside the
        // RIFF size, leave libwebp's decoder waiting for a chunk header.
        let image = image_chunks_of(&whole);
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(0, SIZE));
        body.extend_from_slice(b"ABCD");
        riff::write_chunk(&mut body, image[0].0, &image[0].1);
        let cut = riff::file(&body);
        assert!(webpx::decode_rgba(&cut).is_err());
        assert_eq!(optimise(&cut), corrupt(DecodeDefect::WebpCutShort));
    }

    #[test]
    fn a_simple_file_without_its_pad_byte_is_stored_as_uploaded() {
        // A simple file whose bitstream has an odd size and no pad byte
        // inside the RIFF size: libwebp's decoder reads it, its demuxer does
        // not, and adding the byte would make the stored file larger than
        // the upload (module documentation).
        // libwebp pads its VP8 bitstreams to an even size, not its VP8L ones.
        let odd = (0..=255_u8)
            .map(|shift| encode(&samples::card(255, shift), SIZE, true, 75.0))
            .map(|file| payload(&file, *b"VP8L").unwrap())
            .find(|bitstream| bitstream.len() % 2 == 1)
            .unwrap();
        let unpadded = |before: &[u8]| {
            let mut body = before.to_vec();
            body.extend_from_slice(b"VP8L");
            body.extend_from_slice(&u32::try_from(odd.len()).unwrap().to_le_bytes());
            body.extend_from_slice(&odd);
            riff::file(&body)
        };
        let simple = unpadded(b"");
        assert_eq!(libwebp_frames(&simple), None);
        let optimised = optimise(&simple).unwrap();
        assert_eq!(optimised.bytes, simple);
        assert_eq!(optimised.treatment, Treatment::Stripped);
        assert_eq!(
            rgba(&optimised.bytes),
            webpx::decode_rgba(&simple).unwrap().0
        );
        // What follows the RIFF size still goes, and the stored file reads
        // back the same.
        let appended = [&simple[..], b"tail"].concat();
        assert_eq!(optimise(&appended).unwrap().bytes, simple);
        let canvas = Dimensions::new(SIZE.0, SIZE.1).unwrap();
        assert_eq!(assemble(&parse(&simple).unwrap(), canvas), simple);
        // After a VP8X chunk the decoder counts the pad byte too
        // (ParseOptionalChunks), and refuses the file like the demuxer.
        let mut vp8x_chunk = Vec::new();
        riff::write_chunk(&mut vp8x_chunk, *b"VP8X", &vp8x(0, SIZE));
        let extended = unpadded(&vp8x_chunk);
        assert!(webpx::decode_rgba(&extended).is_err());
        assert_eq!(libwebp_frames(&extended), None);
        assert_eq!(optimise(&extended), corrupt(DecodeDefect::WebpCutShort));
    }

    #[test]
    fn still_layouts_that_only_libwebps_decoder_reads_pass() {
        let bitstream = still(255, true, 75.0);
        let image = image_chunks_of(&bitstream);
        let vp8l: ChunkRef<'_> = refs(&image)[0];
        let other = image_chunks_of(&still(255, true, 10.0));
        let pixel = pixel();
        let one_frame = frame(0, 0, &[(*b"VP8L", &pixel)]);
        let anim: ChunkRef<'_> = (*b"ANIM", &[0; 6]);
        // Each layout refuses libwebp's demuxer, so no browser shows it,
        // while libwebp's decoder shows the first image; the strip stores
        // that image in a file both read.
        let layouts: [(u8, Vec<ChunkRef<'_>>, &str); 6] = [
            (0, vec![vp8l, refs(&other)[0]], "a second image"),
            (0, vec![(*b"VP8X", &[0; 10]), vp8l], "a second VP8X chunk"),
            (0, vec![anim, vp8l], "an ANIM chunk before the image"),
            (
                0,
                vec![anim, (*b"ANMF", &one_frame), vp8l],
                "a frame before the image",
            ),
            (0xC1, vec![vp8l], "reserved flags"),
            (0, vec![vp8l, (*b"VP8X", &[0; 10])], "a VP8X chunk after it"),
        ];
        for (flags, chunks, layout) in layouts {
            let input = extended_file(flags, &chunks);
            assert_eq!(libwebp_frames(&input), None, "{layout}");
            let optimised = optimise(&input).unwrap();
            assert_eq!(optimised.bytes, bitstream, "{layout}");
            assert_eq!(rgba(&optimised.bytes), rgba(&input), "{layout}");
            assert_eq!(libwebp_frames(&optimised.bytes), Some(1), "{layout}");
        }
        // Fewer than 8 bytes after the image, inside the RIFF size, leave the
        // demuxer waiting for a chunk header; the decoder never looks there.
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(0, SIZE));
        riff::write_chunk(&mut body, vp8l.0, vp8l.1);
        body.extend_from_slice(b"ABCD");
        let cut = riff::file(&body);
        assert_eq!(libwebp_frames(&cut), None);
        assert_eq!(optimise(&cut).unwrap().bytes, bitstream);
        assert_eq!(rgba(&cut), rgba(&bitstream));
    }

    #[test]
    fn the_metadata_end_where_libwebps_demuxer_stops() {
        let pixels = image_chunks_of(&still(255, true, 75.0));
        let vp8l: ChunkRef<'_> = refs(&pixels)[0];
        let exif: ChunkRef<'_> = (*b"EXIF", EXIF_WITH_GPS);
        let icc: ChunkRef<'_> = (*b"ICCP", ICC_PROFILE);
        let anim: ChunkRef<'_> = (*b"ANIM", &[0; 6]);
        let kept_with = |flags: u8, chunks: &[ChunkRef<'_>]| {
            let input = extended_file(ICC | EXIF | flags, chunks);
            let bytes = optimise(&input).unwrap().bytes;
            (
                payload(&bytes, *b"ICCP").is_some(),
                payload(&bytes, *b"EXIF").is_some(),
            )
        };
        let kept = |chunks: &[ChunkRef<'_>]| kept_with(0, chunks);
        // The demuxer reads the whole file: both are kept, as it reads them.
        let whole = extended_file(ICC | EXIF, &[icc, vp8l, exif]);
        assert_eq!(
            webpx::get_exif(&whole).unwrap().as_deref(),
            Some(EXIF_WITH_GPS)
        );
        assert_eq!(kept(&[icc, vp8l, exif]), (true, true));
        // It refuses the image after an ANIM chunk, or a second image, and
        // reads no further: what came before stays, what follows goes.
        assert_eq!(kept(&[icc, anim, vp8l, exif]), (true, false));
        assert_eq!(kept(&[vp8l, icc, vp8l, exif]), (true, false));
        assert_eq!(kept(&[vp8l, vp8l, icc, exif]), (false, false));
        // So does an ALPH chunk after the bitstream, with the alpha flag, but
        // not without it.
        let alph: ChunkRef<'_> = (*b"ALPH", b"a");
        assert_eq!(kept_with(ALPHA, &[icc, vp8l, alph, exif]), (true, false));
        assert_eq!(kept_with(0, &[icc, vp8l, alph, exif]), (true, true));
        // And a second VP8X chunk.
        assert_eq!(kept(&[vp8l, (*b"VP8X", &[0; 10]), exif]), (false, false));
        // A chunk cut short after the image ends it too.
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(ICC | EXIF, SIZE));
        riff::write_chunk(&mut body, icc.0, icc.1);
        riff::write_chunk(&mut body, vp8l.0, vp8l.1);
        body.extend_from_slice(b"EXIF\xFF\0\0\0");
        let bytes = optimise(&riff::file(&body)).unwrap().bytes;
        assert_eq!(fourccs(&bytes), [*b"VP8X", *b"ICCP", *b"VP8L"]);
        // Where the walk stops, it records why, as the demuxer would refuse
        // the file. For a still image only the stop is used, but the reason
        // is the one the same layout gets in an animation.
        let stops_with = |flags: u8, chunks: &[ChunkRef<'_>]| {
            let mut body = Vec::new();
            for (fourcc, chunk) in chunks {
                riff::write_chunk(&mut body, *fourcc, chunk);
            }
            demux(flags, &body).refused
        };
        let why = |defect| Some(undecodable(defect));
        assert_eq!(stops_with(0, &[icc, vp8l, exif]), None);
        assert_eq!(
            stops_with(0, &[icc, anim, vp8l, exif]),
            why(DecodeDefect::WebpImageOutsideFrames)
        );
        assert_eq!(
            stops_with(0, &[vp8l, icc, vp8l, exif]),
            why(DecodeDefect::WebpSecondImage)
        );
        assert_eq!(
            stops_with(ALPHA, &[icc, vp8l, alph, exif]),
            why(DecodeDefect::WebpAlphaAfterBitstream)
        );
        assert_eq!(stops_with(0, &[icc, vp8l, alph, exif]), None);
        assert_eq!(
            stops_with(0, &[vp8l, (*b"VP8X", &[0; 10]), exif]),
            why(DecodeDefect::WebpSecondVp8x)
        );
    }

    #[test]
    fn an_animation_is_refused_at_the_frame_that_passes_a_limit() {
        // 100,000 frames of 1×1: the walk stops at frame 4097, so the frame
        // list never grows past the limit and what follows is never read,
        // not even the cut-short chunk at the end.
        let pixel = pixel();
        let one = frame(0, 0, &[(*b"VP8L", &pixel)]);
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(ANIMATION, (1, 1)));
        riff::write_chunk(&mut body, *b"ANIM", &[0; 6]);
        for _ in 0..100_000 {
            riff::write_chunk(&mut body, *b"ANMF", &one);
        }
        body.extend_from_slice(b"ANMF\xFF\xFF\0\0");
        let too_large = Err(ImageError::AnimationTooLarge {
            format: ImageFormat::WebP,
            frames: MAX_ANIMATION_FRAMES + 1,
            pixels: MAX_ANIMATION_FRAMES + 1,
        });
        let input = riff::file(&body);
        assert_eq!(parse(&input).map(|_| ()), too_large);
        assert_eq!(optimise(&input).map(|_| ()), too_large);
        // The walk keeps the frames within the limit and no more.
        let mut demuxed = Demuxed::default();
        assert_eq!(walk(ANIMATION, &body[18..], &mut demuxed), too_large);
        assert_eq!(
            u64::try_from(demuxed.frames.len()).unwrap(),
            MAX_ANIMATION_FRAMES
        );
    }

    #[test]
    fn a_frames_image_must_end_inside_its_anmf_chunk() {
        // The frame's VP8L chunk starts inside the ANMF chunk, whose size
        // stops 4 bytes short of the bitstream's end: libwebp's
        // ParseAnimationFrame refuses a frame that reads past its chunk.
        let pixel = pixel();
        let inner = frame(0, 0, &[(*b"VP8L", &pixel)]);
        let mut body = Vec::new();
        riff::write_chunk(&mut body, *b"VP8X", &vp8x(ANIMATION, (1, 1)));
        riff::write_chunk(&mut body, *b"ANIM", &[0; 6]);
        let declared = inner.len() - 4;
        body.extend_from_slice(b"ANMF");
        body.extend_from_slice(&u32::try_from(declared).unwrap().to_le_bytes());
        body.extend_from_slice(&inner);
        let input = riff::file(&body);
        assert_eq!(libwebp_frames(&input), None);
        assert_eq!(optimise(&input), corrupt(DecodeDefect::WebpFrameOverrun));
    }

    /// Any chunk an extended file might hold, with short payloads.
    fn any_chunk() -> impl Strategy<Value = ([u8; 4], Vec<u8>)> {
        let fourccs = [
            *b"ICCP", *b"EXIF", *b"XMP ", *b"ALPH", *b"ANIM", *b"ANMF", *b"ABCD",
        ];
        (select(fourccs.to_vec()), vec(any::<u8>(), 0..24))
    }

    proptest! {
        #[test]
        fn the_strip_never_grows_a_file_and_is_idempotent(
            flags in any::<u8>(),
            lossless in any::<bool>(),
            before in vec(any_chunk(), 0..4),
            after in vec(any_chunk(), 0..4),
            tail in vec(any::<u8>(), 0..8),
        ) {
            let bitstream = still(255, lossless, 75.0);
            let input = [&with_metadata(flags, &bitstream, &refs(&before), &refs(&after))[..], &tail].concat();
            if let Ok(parts) = parse(&input) {
                let canvas = Dimensions::new(SIZE.0, SIZE.1).unwrap();
                let stripped = assemble(&parts, canvas);
                prop_assert!(stripped.len() <= input.len());
                prop_assert_eq!(assemble(&parse(&stripped).unwrap(), canvas), stripped.clone());
                for fourcc in fourccs(&stripped) {
                    prop_assert!(fourcc != *b"XMP " && fourcc != *b"ABCD");
                }
            }
            if let Ok(optimised) = optimise(&input) {
                prop_assert!(optimised.bytes.len() <= input.len());
            }
        }

        #[test]
        fn an_accepted_file_keeps_its_header(
            flags in any::<u8>(),
            canvas in prop_oneof![Just(SIZE), (1..40_u32, 1..40_u32)],
            before in vec(any_chunk(), 0..3),
            lossless in any::<bool>(),
        ) {
            let image = image_chunks_of(&still(255, lossless, 75.0));
            let chunks = [refs(&before), refs(&image)].concat();
            let input = extended_on(flags & !ANIMATION, canvas, &chunks);
            if let Ok(optimised) = optimise(&input) {
                prop_assert_eq!(inspect(&optimised.bytes, PixelBudget::DEFAULT), Ok(optimised.header));
                prop_assert_eq!(inspect(&input, PixelBudget::DEFAULT), Ok(optimised.header));
            }
        }

        #[test]
        fn an_animation_libwebp_reads_has_the_frames_it_reads(
            frames in vec((vec(any_chunk(), 0..3), any::<bool>(), vec(any_chunk(), 0..3)), 1..4),
            between in vec(any_chunk(), 0..3),
        ) {
            // Frames with chunks before and after their image (or none),
            // and chunks between them: the strip and libwebp agree on
            // whether the file is an animation, and on its frames.
            let pixel = pixel();
            let payloads: Vec<Vec<u8>> = frames
                .iter()
                .map(|(before, with_image, after)| {
                    let image: Vec<ChunkRef<'_>> =
                        if *with_image { vec![(*b"VP8L", &pixel)] } else { vec![] };
                    frame(0, 0, &[refs(before), image, refs(after)].concat())
                })
                .collect();
            let mut chunks: Vec<ChunkRef<'_>> = vec![(*b"ANIM", &[0; 6])];
            for payload in &payloads {
                chunks.push((*b"ANMF", payload));
                chunks.extend(refs(&between));
            }
            let input = extended_on(ANIMATION, (1, 1), &chunks);
            let ours = optimise(&input).ok().and_then(|optimised| libwebp_frames(&optimised.bytes));
            prop_assert_eq!(ours, libwebp_frames(&input));
        }

        #[test]
        fn a_still_passes_exactly_when_libwebps_decoder_reads_it(
            flags in any::<u8>(),
            alpha in prop_oneof![Just(255_u8), Just(128)],
            lossless in any::<bool>(),
            before in vec(any_chunk(), 0..3),
            second in any::<bool>(),
            after in vec(any_chunk(), 0..3),
        ) {
            // Random chunks around a still image, ALPH, ANIM and ANMF
            // chunks among them, and perhaps a second image: the strip
            // accepts what libwebp's decoder decodes, refuses what it
            // refuses, stores the pixels it shows, and stores a file that
            // libwebp's demuxer, and so a browser, reads too. Quality 10 is
            // never re-encoded smaller, so the pixels stay the upload's.
            let image = image_chunks_of(&still(alpha, lossless, 10.0));
            let extra = if second { image_chunks_of(&still(255, true, 75.0)) } else { vec![] };
            let chunks = [refs(&before), refs(&image), refs(&extra), refs(&after)].concat();
            let input = extended_file(flags & !ANIMATION, &chunks);
            let decoded = webpx::decode_rgba(&input).ok().map(|(pixels, ..)| pixels);
            let optimised = optimise(&input);
            prop_assert_eq!(optimised.is_ok(), decoded.is_some(), "{:?}", optimised);
            if let (Ok(optimised), Some(decoded)) = (optimised, decoded) {
                prop_assert_ne!(optimised.treatment, Treatment::ReEncoded);
                prop_assert_eq!(rgba(&optimised.bytes), decoded);
                prop_assert_eq!(libwebp_frames(&optimised.bytes), Some(1));
            }
        }

        #[test]
        fn the_keep_rule_keeps_exactly_what_passes_both_halves(
            stripped in 0..1_000_usize,
            re_encoded in 0..1_000_usize,
            ssim in -1.0..=1.0_f64,
        ) {
            let kept = keep_rule(stripped, re_encoded, || Ok(ssim)).is_ok();
            prop_assert_eq!(kept, reencode_is_kept(stripped, re_encoded) && ssim >= 0.98);
            let (bytes, treatment, failure) = choose(
                vec![0; stripped],
                keep_rule(stripped, re_encoded, || Ok(ssim)).map(|()| vec![1; re_encoded]),
            );
            prop_assert!(bytes.len() <= stripped);
            prop_assert_eq!(treatment == Treatment::ReEncoded, kept);
            prop_assert_eq!(failure, None);
        }

        #[test]
        fn arbitrary_chunks_never_panic(flags in any::<u8>(), chunks in vec(any_chunk(), 0..6)) {
            let _ = optimise(&extended_file(flags, &refs(&chunks)));
        }
    }
}
