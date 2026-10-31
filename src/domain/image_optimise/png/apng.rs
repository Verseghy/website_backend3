//! The animation of an animated PNG (APNG): which frames the optimiser keeps
//! and how it writes them (ADR 0033 §6).
//!
//! An APNG is a PNG whose `acTL` chunk announces an animation. Its frames are
//! described by `fcTL` chunks and, after the image data, held in `fdAT`
//! chunks: a 4-byte sequence number and then image data compressed like
//! `IDAT`, for a frame of the size and offset its `fcTL` gives. The `IDAT`
//! image is the default image: a decoder without APNG support shows it, and
//! it is the animation's first frame when an `fcTL` comes before it. ADR 0033
//! §6 keeps the animation, so the chunk walk keeps `acTL`, `fcTL` and `fdAT`
//! and the encoder recompresses each frame's data losslessly, as it does
//! `IDAT`.
//!
//! libpng 1.6.58, which the chunk walk follows for still images, does not
//! decode animations: the three chunk types are ancillary chunks it skips.
//! So the file is first checked as a still PNG is, and a file those checks
//! refuse is refused whatever its animation holds, one cut short inside its
//! animation included. The one exception is which of two refusals comes
//! first: the totals below are counted before any image data is inflated,
//! so an animation above them is refused for its totals even when its
//! default image does not decode. What follows applies to a file the
//! still-image checks accept. The rules
//! are those of the PNG specification (third edition, §11.3.6, which took
//! APNG in) as the `png` crate 0.18.1 applies them; that crate is the PNG
//! decoder of Chromium, the browser of most of the site's visitors and
//! editors. Three checks of the image data are carried over to each frame:
//! its CRC, its zlib Adler-32, and data that decodes to exactly its rows.
//!
//! **The animation** is the one announced by the first `acTL` before the
//! image data that [`control_fits`]: 8 bytes, 1 to 2³¹ − 1 frames and at most
//! 2³¹ − 1 plays. The `png` crate ignores every other intact `acTL`, and so
//! does the walk; a PNG without such an `acTL` is a still image, and its
//! `fcTL` and `fdAT` chunks are dropped.
//!
//! **The frames.** When an `fcTL` comes before the image data, the default
//! image is frame 0, which starts at that `fcTL` and ends with the image
//! data. Every `fcTL` after the image data starts a frame, whose data is the
//! `fdAT` chunks after it, and that frame ends at the next `fcTL` or at
//! `IEND`.
//!
//! **The first defect** is the first of these, in file order:
//!
//! - an `fcTL` or `fdAT` whose sequence number is not the next one, counted
//!   from 0 over both chunk types, and an `fdAT` shorter than its 4-byte
//!   sequence number;
//! - an `fcTL` that is not 26 bytes long, or whose frame is smaller than
//!   1×1, reaches outside the canvas of `IHDR`, or has a dispose operation
//!   other than 0 to 2 or a blend operation other than 0 or 1;
//! - a second `fcTL` before the image data, and a first one there whose
//!   frame does not cover the whole canvas;
//! - an `fdAT` before the image data, an `fdAT` after it that no `fcTL`
//!   after the image data comes before, and an `fdAT` that another chunk
//!   parts from the `fdAT` chunks before it in its frame;
//! - a frame after the image data that ends without an `fdAT`;
//! - an `fcTL` or `fdAT` lost to its CRC, and an `acTL` before the image
//!   data lost to its CRC, which might have been the one that announces the
//!   animation (an `acTL` after the image data is ignored, intact or not);
//! - a frame whose data does not start with one zlib stream, its Adler-32
//!   matching, that decodes to exactly the frame's rows, each starting with
//!   a filter type from 0 to 4 ([`Animation::checked`]; the bytes after the
//!   end of that stream are neither read nor stored).
//!
//! **Where it lies.** A defective `fcTL` lies in the frame that it starts,
//! or would start; both kinds of defective `fcTL` before the image data lie
//! in frame 0. Any other defective chunk lies in the frame that has not
//! ended when the chunk is read, or in no frame when no frame has started
//! yet or every frame before it has ended, such as an `fdAT` right after the
//! image data. A frame after the image data that ends without an `fdAT`, or
//! a frame whose data does not decode, is itself the defect and lies in that
//! frame. So a defect before the image data always leaves no frame: frame 0
//! has not ended there.
//!
//! **What is kept.** The frame in which the first defect lies is dropped,
//! with every later frame, and the frames before it are kept; a defect that
//! lies in no frame keeps every frame that ended before it. Of those, at most
//! the number of frames that `acTL` announces is kept: a frame past that
//! number is dropped whatever it holds, and is no defect, so the walk stops
//! reading the animation where it starts. The stored `acTL` announces the
//! number of frames kept, with its play count unchanged, and the kept chunks
//! are numbered again from 0. When no frame is kept, `acTL`, `fcTL` and
//! `fdAT` are all dropped and the still default image is stored, as for a PNG
//! without animation. Visitors see the frames before the defect, which
//! Chromium and Firefox both show; ADR 0033 §6 records the decision (D103)
//! and the frames this loses where Chromium shows more.
//!
//! **Chunks outside the animation** are judged by the still-image checks
//! wherever they stand, between the frames too, and they never cut the
//! animation: the first defect is only one of the list above. The walk drops
//! what libpng ignores there, among it chunks at which the `png` crate, with
//! its default options, stops decoding: an `IDAT` after the first run, and,
//! in an image that is not
//! indexed, a `PLTE` that is not 3 to 768 bytes long, follows another
//! `PLTE`, or does not match its CRC. On such a file the stored animation
//! plays frames past the point where Chromium stops on the upload. They are
//! the upload's own frames, and the stored file, which no longer holds those
//! chunks, decodes in full. Cutting there instead would judge the chunks of
//! the still image by a second decoder, against the reference of ADR 0033
//! §3, and would drop frames that every decoder shows from the stored file.
//!
//! **The pixel budget** applies as the header check applies it: the canvas
//! of `IHDR` is budgeted ([`super::super::inspect`]), and every frame lies
//! inside it, so no frame is larger. **The totals** of
//! [`MAX_ANIMATION_FRAMES`] and [`MAX_ANIMATION_PIXELS`] bound the work of
//! all frames together (ADR 0033 §4). They count the default image and the
//! frames that the chunks alone keep, those that would be kept if no frame's
//! data were damaged, each at its own size: the default image's pixels
//! always count, and it counts as a frame only when it is frame 0. They are
//! checked once every chunk is read, before any image data is inflated, the
//! default image's included; only the first `iCCP` profile, which the walk
//! reads as it goes, may have been inflated by then. So a frame whose data
//! then turns out not to decode still counts, and an animation above the
//! totals is refused for them even when its default image does not decode.
//! An animation above either is refused rather than cut, as an animated WebP
//! or GIF above them is, so the editor learns why.
//!
//! **Memory.** The walk hands each frame chunk to [`Frames`] as it reads it,
//! and the rules are checked as they come. A frame's data is copied only
//! while the frames up to it are within the totals; the copies are the
//! frames' compressed data, which the rewrite needs, so they never take more
//! than the upload's own size. Once a frame past the totals has ended, a
//! later defect can no longer bring the animation back within them, so the
//! refusal is certain and the copies are freed. Past the first defect or the
//! count `acTL` announces, no chunk is read into the animation.
//!
//! The kept frames are written with one `fdAT` per frame (more only for a
//! frame of over 2 GiB of data), cut at the end of its zlib stream, and the
//! sequence numbers counted again from 0. `acTL` keeps its 8 bytes, and each
//! other chunk written stands for input chunks at least as large, so the
//! animation never makes the rewrite larger than the upload.

use super::chunks::{MAX_CHUNK, image_rows, write_chunk};
use super::zlib;
use crate::domain::image_optimise::{MAX_ANIMATION_FRAMES, MAX_ANIMATION_PIXELS};

/// The largest frame count and play count `acTL` may hold, 2³¹ − 1.
const MAX_COUNT: u32 = 0x7FFF_FFFF;

/// The animation chunks of one file, checked as the walk reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Frames<'a> {
    /// The canvas width, from `IHDR`.
    width: u32,
    /// The canvas height, from `IHDR`.
    height: u32,
    /// The pixels of the canvas, the default image's, which always count
    /// toward the totals.
    canvas: u64,
    /// The data of the first valid `acTL` before the image data.
    actl: Option<&'a [u8]>,
    /// The `fcTL` data of the default image while it is frame 0 and kept.
    first: Option<&'a [u8]>,
    /// The frames after the image data kept so far, the last of them perhaps
    /// still open, while they are within the totals: always the first of
    /// the frames that `after` counts.
    held: Vec<Frame<'a>>,
    /// How many frames after the image data are kept so far, the open one
    /// included, held or not.
    after: u64,
    /// The pixels of the canvas and of the frames that `after` counts.
    pixels: u64,
    /// `pixels` before the open frame was added, for when it is dropped.
    closed_pixels: u64,
    /// The sequence number the next `fcTL` or `fdAT` must carry. It is 64
    /// bits wide, so the number after 2³² − 1, which no chunk can carry,
    /// needs no case of its own.
    sequence: u64,
    /// Whether the open frame after the image data has an `fdAT` yet.
    with_data: bool,
    /// Whether the animation has ended: at its first defect, or where a
    /// frame past the count `acTL` announces starts. No later chunk changes
    /// what is kept.
    ended: bool,
}

/// An animation with at least one frame, ready to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Animation<'a> {
    /// The `acTL` data as uploaded; its play count is kept, its frame count
    /// rewritten.
    actl: &'a [u8],
    /// The `fcTL` data of the default image, when it is the first frame.
    first: Option<&'a [u8]>,
    /// The frames after the image data.
    frames: Vec<Frame<'a>>,
    /// The pixels of the canvas.
    canvas: u64,
}

/// One frame after the image data.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame<'a> {
    /// Its `fcTL` data.
    control: &'a [u8],
    /// Its compressed data: the `fdAT` payloads joined, cut at the end of
    /// the zlib stream once checked.
    data: Vec<u8>,
}

/// Why an animation is refused rather than cut: its totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TooLarge {
    /// The frames the chunks keep.
    pub(super) frames: u64,
    /// The pixels of the canvas and of those frames after the image data.
    pub(super) pixels: u64,
}

/// Whether `data` is an `acTL` that the `png` crate takes: 8 bytes, 1 to
/// 2³¹ − 1 frames and at most 2³¹ − 1 plays.
///
/// - `data`: the chunk data.
pub(super) fn control_fits(data: &[u8]) -> bool {
    let [frames, plays] = [0, 4].map(|at| word(data, at));
    data.len() == 8 && (1..=MAX_COUNT).contains(&frames) && plays <= MAX_COUNT
}

/// The big-endian 32-bit word at `at` in `data`, or 0 when it does not fit;
/// every caller has checked the length first, or treats 0 as invalid.
///
/// - `data`: the bytes.
/// - `at`: the offset of the word.
fn word(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map_or(0, u32::from_be_bytes)
}

/// The pixels of the frame that the `fcTL` data `control` describes.
///
/// - `control`: an `fcTL`'s data, 26 bytes.
fn frame_pixels(control: &[u8]) -> u64 {
    u64::from(word(control, 4)) * u64::from(word(control, 8))
}

impl<'a> Frames<'a> {
    /// No animation chunks yet, for the image whose `IHDR` data is `ihdr`.
    ///
    /// - `ihdr`: the 13 bytes of `IHDR`, whose fields the header reader
    ///   checked.
    pub(super) fn new(ihdr: &[u8]) -> Self {
        let (width, height) = (word(ihdr, 0), word(ihdr, 4));
        let canvas = u64::from(width) * u64::from(height);
        Self {
            width,
            height,
            canvas,
            actl: None,
            first: None,
            held: Vec::new(),
            after: 0,
            pixels: canvas,
            closed_pixels: canvas,
            sequence: 0,
            with_data: false,
            ended: false,
        }
    }

    /// Takes an `acTL` chunk read before the image data; the walk passes no
    /// `acTL` after it, which is ignored, intact or not.
    ///
    /// - `data`: the chunk data, or `None` when the chunk was lost to its
    ///   CRC. An intact one announces the animation when it is the first
    ///   that [`control_fits`]. A lost one is a defect, since it might have
    ///   been that one, and before the image data a defect leaves no frame.
    ///   Once a defect has ended the animation before the image data, no
    ///   frame is kept whatever acTL comes, so no case of its own is needed.
    pub(super) fn announce(&mut self, data: Option<&'a [u8]>) {
        match data {
            None => self.drop_every_frame(),
            Some(data) if self.actl.is_none() && control_fits(data) => self.actl = Some(data),
            Some(_) => {}
        }
    }

    /// Takes an `fcTL` chunk.
    ///
    /// - `data`: the chunk data, or `None` when the chunk was lost to its
    ///   CRC, which makes it a defective `fcTL`.
    /// - `before_data`: whether it came before the image data.
    pub(super) fn control(&mut self, data: Option<&'a [u8]>, before_data: bool) {
        if self.ended {
            return;
        }
        if !before_data {
            // The fcTL ends the open frame. One that ends without an fdAT is
            // the defect, and lies in that frame, before this fcTL is read.
            if self.after > 0 && !self.with_data {
                return self.drop_open_frame();
            }
            // That frame is kept by its chunks now. If the totals are passed
            // with it, the animation is refused whatever follows, so the
            // copies of the frames' data are no longer needed.
            if !self.within_totals() {
                self.held = Vec::new();
            }
            // A frame past the count acTL announces is dropped, whatever it
            // holds, and is no defect; nothing after it is kept. Without an
            // acTL there is no animation.
            if self
                .actl
                .is_none_or(|actl| self.count() >= u64::from(word(actl, 0)))
            {
                self.ended = true;
                return;
            }
        }
        let Some(data) = data.filter(|data| self.control_allowed(data, before_data)) else {
            // A defective fcTL lies in the frame it would start: frame 0
            // before the image data, which leaves no frame; after it, a new
            // frame, so the frames before it are kept.
            if before_data {
                self.drop_every_frame();
            } else {
                self.ended = true;
            }
            return;
        };
        if before_data {
            self.first = Some(data);
        } else {
            self.after += 1;
            self.closed_pixels = self.pixels;
            self.pixels = self.pixels.saturating_add(frame_pixels(data));
            self.with_data = false;
            // The totals only grow until the open frame is dropped, and then
            // the animation has ended, so the held frames stay the first.
            if self.within_totals() {
                self.held.push(Frame {
                    control: data,
                    data: Vec::new(),
                });
            }
        }
        self.sequence += 1;
    }

    /// Whether the intact `fcTL` data `data` is one the specification allows
    /// here: 26 bytes, the next sequence number, a frame of at least 1×1
    /// inside the canvas, known dispose (0-2) and blend (0-1) operations,
    /// and before the image data the only one, covering the canvas (inside
    /// the canvas, the full width and height also place it at 0, 0).
    ///
    /// - `data`: the chunk data.
    /// - `before_data`: whether it came before the image data.
    fn control_allowed(&self, data: &[u8], before_data: bool) -> bool {
        let (w, h, x, y) = (word(data, 4), word(data, 8), word(data, 12), word(data, 16));
        data.len() == 26
            && u64::from(word(data, 0)) == self.sequence
            && w > 0
            && h > 0
            && x.checked_add(w).is_some_and(|right| right <= self.width)
            && y.checked_add(h).is_some_and(|bottom| bottom <= self.height)
            && data[24] <= 2
            && data[25] <= 1
            && (!before_data
                || (self.first.is_none() && (x, y, w, h) == (0, 0, self.width, self.height)))
    }

    /// Takes an `fdAT` chunk.
    ///
    /// - `data`: the chunk data, the sequence number included, or `None`
    ///   when the chunk was lost to its CRC, which makes it a defect.
    /// - `before_data`: whether it came before the image data.
    /// - `follows_data`: whether the chunk read just before it was an `fdAT`
    ///   too.
    pub(super) fn data(&mut self, data: Option<&'a [u8]>, before_data: bool, follows_data: bool) {
        if self.ended {
            return;
        }
        if self.after == 0 {
            // No fcTL after the image data comes before it, so it is a
            // defect. Before the image data it leaves no frame; after it, it
            // lies in no frame, and frame 0, if any, has ended and is kept.
            if before_data {
                self.drop_every_frame();
            } else {
                self.ended = true;
            }
            return;
        }
        // An fdAT that another chunk parts from the fdAT chunks before it in
        // its frame breaks the frame, as the png crate reads it, and so does
        // one with the wrong number; either lies in the open frame.
        let Some(data) = data.filter(|data| {
            data.len() >= 4
                && u64::from(word(data, 0)) == self.sequence
                && (follows_data || !self.with_data)
        }) else {
            return self.drop_open_frame();
        };
        self.with_data = true;
        if self.holds_open_frame()
            && let Some(frame) = self.held.last_mut()
        {
            frame.data.extend_from_slice(&data[4..]);
        }
        self.sequence += 1;
    }

    /// The animation, once every chunk up to `IEND` has been taken:
    /// `Ok(None)` when no frame is kept, so the frame chunks are dropped, and
    /// `Ok(Some(_))` with the frames' data still to be checked by
    /// [`Animation::checked`]. No image data, the default image's included,
    /// has been inflated yet.
    ///
    /// # Errors
    ///
    /// [`TooLarge`] when the frames that the chunks keep are above
    /// [`MAX_ANIMATION_FRAMES`] or [`MAX_ANIMATION_PIXELS`].
    pub(super) fn finish(mut self) -> Result<Option<Animation<'a>>, TooLarge> {
        let Some(actl) = self.actl else {
            return Ok(None);
        };
        // IEND ends the open frame; one that ends without an fdAT is the
        // defect, in that frame.
        if !self.ended && self.after > 0 && !self.with_data {
            self.drop_open_frame();
        }
        let frames = self.count();
        if frames == 0 {
            return Ok(None);
        }
        if !self.within_totals() {
            return Err(TooLarge {
                frames,
                pixels: self.pixels,
            });
        }
        // Within the totals, every kept frame is held.
        debug_assert_eq!(u64::try_from(self.held.len()).ok(), Some(self.after));
        Ok(Some(Animation {
            actl,
            first: self.first,
            frames: self.held,
            canvas: self.canvas,
        }))
    }

    /// The frames kept so far, the default image included when it is frame
    /// 0, and the open frame after the image data included.
    fn count(&self) -> u64 {
        self.after + u64::from(self.first.is_some())
    }

    /// Whether the frames kept so far are within [`MAX_ANIMATION_FRAMES`]
    /// and [`MAX_ANIMATION_PIXELS`].
    fn within_totals(&self) -> bool {
        self.count() <= MAX_ANIMATION_FRAMES && self.pixels <= MAX_ANIMATION_PIXELS
    }

    /// Whether the open frame after the image data is held, so its `fdAT`
    /// data is copied: the held frames are the first ones, so it is held
    /// exactly when every frame that `after` counts is.
    fn holds_open_frame(&self) -> bool {
        u64::try_from(self.held.len()).is_ok_and(|held| held == self.after)
    }

    /// Drops the open frame after the image data, in which the first defect
    /// lies, and ends the animation; the frames before it are kept. Only
    /// called while a frame is open (`after` is at least 1).
    fn drop_open_frame(&mut self) {
        if self.holds_open_frame() {
            self.held.pop();
        }
        self.after = self.after.saturating_sub(1);
        self.pixels = self.closed_pixels;
        self.ended = true;
    }

    /// Ends the animation with no frame kept, for a defect before the image
    /// data: it lies in frame 0, or in no frame where none has ended.
    fn drop_every_frame(&mut self) {
        self.first = None;
        self.ended = true;
    }
}

impl Animation<'_> {
    /// The animation with each frame's data checked and cut at the end of
    /// its zlib stream, up to the first frame whose data is not one stream
    /// of exactly its rows: that frame is the defect, and it is dropped
    /// with every later one, whose data is never inflated. `None` when no
    /// frame is left, so the frame chunks are dropped.
    ///
    /// - `ihdr`: the 13 bytes of `IHDR`, whose bit depth, colour type and
    ///   interlace method every frame shares.
    pub(super) fn checked(mut self, ihdr: &[u8]) -> Option<Self> {
        let mut decoded = 0;
        for frame in &mut self.frames {
            let frame_ihdr = [&frame.control[4..12], &ihdr[8..13]].concat();
            let Some(len) =
                image_rows(&frame_ihdr).and_then(|rows| zlib::check_rows(&frame.data, &rows))
            else {
                break;
            };
            frame.data.truncate(len);
            decoded += 1;
        }
        self.frames.truncate(decoded);
        (self.count() > 0).then_some(self)
    }

    /// The frames kept, the default image included when it is the first.
    fn count(&self) -> usize {
        self.frames.len() + usize::from(self.first.is_some())
    }

    /// The pixels the encoder works on: the canvas, and every kept frame
    /// after the image data at its own size.
    pub(super) fn pixels(&self) -> u64 {
        self.frames
            .iter()
            .map(|frame| frame_pixels(frame.control))
            .fold(self.canvas, u64::saturating_add)
    }

    /// Writes `acTL`, the chunk that announces the animation, to `out`: the
    /// number of frames kept and the uploaded play count.
    ///
    /// - `out`: the PNG being written, before `PLTE` and the image data.
    pub(super) fn write_control(&self, out: &mut Vec<u8>) {
        // At most as many frames are kept as the uploaded acTL announces,
        // which is at most 2^31 - 1.
        let frames = u32::try_from(self.count()).map_or(MAX_COUNT, |count| count.min(MAX_COUNT));
        write_chunk(
            out,
            *b"acTL",
            &[&frames.to_be_bytes()[..], &self.actl[4..8]].concat(),
        );
    }

    /// Writes the `fcTL` of the default image, if it is the first frame, to
    /// `out`, numbered 0.
    ///
    /// - `out`: the PNG being written, right before the image data.
    pub(super) fn write_first(&self, out: &mut Vec<u8>) {
        if let Some(control) = self.first {
            write_chunk(out, *b"fcTL", &numbered(0, &control[4..]));
        }
    }

    /// Writes the frames after the image data to `out`, numbered on from
    /// the default image's `fcTL`.
    ///
    /// - `out`: the PNG being written, right after the image data.
    pub(super) fn write_frames(&self, out: &mut Vec<u8>) {
        let mut sequence = u32::from(self.first.is_some());
        for frame in &self.frames {
            write_chunk(out, *b"fcTL", &numbered(sequence, &frame.control[4..]));
            sequence += 1;
            // An fdAT holds the 4-byte number and at most 2^31 - 5 bytes of
            // data; one frame of the budget holds far less.
            for piece in frame.data.chunks(MAX_CHUNK - 4) {
                write_chunk(out, *b"fdAT", &numbered(sequence, piece));
                sequence += 1;
            }
        }
    }
}

/// `rest` after the big-endian sequence number `sequence`.
///
/// - `sequence`: the number of the chunk in the animation.
/// - `rest`: what follows the number in the chunk.
fn numbered(sequence: u32, rest: &[u8]) -> Vec<u8> {
    [&sequence.to_be_bytes()[..], rest].concat()
}

#[cfg(test)]
pub(super) mod samples {
    //! Animation chunks built by hand for the tests of the chunk walk.

    /// The data of an `acTL` chunk for `frames` frames and `plays` plays.
    pub(in crate::domain::image_optimise) fn actl(frames: u32, plays: u32) -> Vec<u8> {
        [frames.to_be_bytes(), plays.to_be_bytes()].concat()
    }

    /// The data of an `fcTL` chunk numbered `sequence`, for a frame of
    /// `width × height` at `(x, y)`, shown for 1/10 s, disposed and blended
    /// with operation 0.
    pub(in crate::domain::image_optimise) fn fctl(
        sequence: u32,
        (width, height): (u32, u32),
        (x, y): (u32, u32),
    ) -> Vec<u8> {
        [
            &sequence.to_be_bytes()[..],
            &width.to_be_bytes(),
            &height.to_be_bytes(),
            &x.to_be_bytes(),
            &y.to_be_bytes(),
            &1_u16.to_be_bytes(),
            &10_u16.to_be_bytes(),
            &[0, 0],
        ]
        .concat()
    }

    /// The data of an `fdAT` chunk numbered `sequence`, holding `data`.
    pub(in crate::domain::image_optimise) fn fdat(sequence: u32, data: &[u8]) -> Vec<u8> {
        [&sequence.to_be_bytes()[..], data].concat()
    }
}

#[cfg(test)]
mod tests {
    use super::samples::{actl, fctl, fdat};
    use super::*;
    use crate::domain::image_optimise::png::samples::{ihdr, pixels};
    use crate::domain::image_optimise::png::zlib::samples::stored;
    use proptest::prelude::*;

    /// The 13 bytes of the `IHDR` of a 4×3 8-bit RGB image.
    fn canvas() -> Vec<u8> {
        ihdr(4, 3, 8, 2, 0)[8..21].to_vec()
    }

    /// The 13 bytes of the `IHDR` of a 1×1 8-bit greyscale image.
    fn tiny() -> Vec<u8> {
        ihdr(1, 1, 8, 0, 0)[8..21].to_vec()
    }

    /// The 13 bytes of the `IHDR` of a 16384×16384 8-bit greyscale image:
    /// 256 Mi pixels, the pixel limit exactly.
    fn big() -> Vec<u8> {
        ihdr(16_384, 16_384, 8, 0, 0)[8..21].to_vec()
    }

    /// A frame's data: a stored zlib stream of a `width × height` RGB frame.
    fn frame(width: u32, height: u32) -> Vec<u8> {
        stored(&pixels(width, height, 3))
    }

    /// The frame chunks of a valid animation of the 4×3 canvas: the default
    /// image as frame 0, then a 2×2 frame at (1, 1) in two fdAT chunks.
    fn valid() -> (Vec<u8>, Vec<Vec<u8>>) {
        let data = frame(2, 2);
        let (head, tail) = data.split_at(5);
        (
            actl(2, 0),
            vec![
                fctl(0, (4, 3), (0, 0)),
                fctl(1, (2, 2), (1, 1)),
                fdat(2, head),
                fdat(3, tail),
            ],
        )
    }

    /// The frame chunks of a valid three-frame animation of the 4×3 canvas,
    /// with 7 plays: the default image as frame 0 (chunk 0, before the
    /// data), a 2×2 frame at (1, 1) (chunks 1 and 2) and a 1×1 frame at
    /// (3, 2) (chunks 3 and 4).
    fn three() -> (Vec<u8>, Vec<Vec<u8>>) {
        (
            actl(3, 7),
            vec![
                fctl(0, (4, 3), (0, 0)),
                fctl(1, (2, 2), (1, 1)),
                fdat(2, &frame(2, 2)),
                fctl(3, (1, 1), (3, 2)),
                fdat(4, &frame(1, 1)),
            ],
        )
    }

    /// `chunks` taken in order by the [`Frames`] of the canvas `ihdr`, after
    /// the `acTL` data `control`: the first `before` come before the image
    /// data, every fdAT follows the chunk before it unless its index is in
    /// `gaps`, and the chunks whose index is in `lost` were lost to their
    /// CRC.
    fn feed_lost<'a>(
        ihdr: &[u8],
        control: &'a [u8],
        chunks: &'a [Vec<u8>],
        before: usize,
        gaps: &[usize],
        lost: &[usize],
    ) -> Frames<'a> {
        let mut frames = Frames::new(ihdr);
        frames.announce(Some(control));
        for (index, data) in chunks.iter().enumerate() {
            let taken = (!lost.contains(&index)).then_some(&data[..]);
            // An fcTL here is at least 26 bytes and starts its width with a
            // zero byte (every frame is narrower than 2^24); an fdAT holds a
            // zlib header (0x78), junk text or under 26 bytes there.
            if data.len() >= 26 && data[4] == 0 {
                frames.control(taken, index < before);
            } else {
                frames.data(taken, index < before, !gaps.contains(&index) && index > 0);
            }
        }
        frames
    }

    /// [`feed_lost`] with no chunk lost.
    fn feed<'a>(
        ihdr: &[u8],
        control: &'a [u8],
        chunks: &'a [Vec<u8>],
        before: usize,
        gaps: &[usize],
    ) -> Frames<'a> {
        feed_lost(ihdr, control, chunks, before, gaps, &[])
    }

    /// The animation of `chunks`, with its frames checked, as the walk
    /// takes it; the arguments as for [`feed`].
    fn animation<'a>(
        ihdr: &[u8],
        control: &'a [u8],
        chunks: &'a [Vec<u8>],
        before: usize,
        gaps: &[usize],
    ) -> Result<Option<Animation<'a>>, TooLarge> {
        let found = feed(ihdr, control, chunks, before, gaps).finish()?;
        Ok(found.and_then(|animation| animation.checked(ihdr)))
    }

    /// The chunks an animation writes: `acTL`, the default image's `fcTL`
    /// and the frames after the image data.
    fn written(animation: &Animation<'_>) -> Vec<u8> {
        let mut out = Vec::new();
        animation.write_control(&mut out);
        animation.write_first(&mut out);
        animation.write_frames(&mut out);
        out
    }

    /// The frames kept of `frames`, checked as the walk checks them on the
    /// canvas `ihdr`, the default image included when it is frame 0: 0 when
    /// no frame is kept. The written `acTL` must announce that number and
    /// keep the play count of `control`, and as many `fcTL` chunks must be
    /// written.
    fn kept_of(ihdr: &[u8], control: &[u8], frames: Frames<'_>) -> usize {
        let Some(found) = frames.finish().unwrap().and_then(|a| a.checked(ihdr)) else {
            return 0;
        };
        let out = written(&found);
        // acTL is written first: its length, type, frame count and plays.
        // An animation always keeps a frame; 0 frames is no valid acTL.
        let count = usize::try_from(word(&out, 8)).unwrap();
        assert!(count > 0, "an animation with no frame");
        assert_eq!(&out[4..8], b"acTL");
        assert_eq!(word(&out, 12), word(control, 4), "the play count");
        let controls = out.windows(4).filter(|w| *w == b"fcTL").count();
        assert_eq!(count, controls, "acTL announces the frames written");
        count
    }

    /// The frames kept of `chunks` on the 4×3 canvas; the arguments as for
    /// [`feed`].
    fn kept(control: &[u8], chunks: &[Vec<u8>], before: usize, gaps: &[usize]) -> usize {
        kept_of(
            &canvas(),
            control,
            feed(&canvas(), control, chunks, before, gaps),
        )
    }

    #[test]
    fn a_valid_animation_is_kept_and_written_with_one_fdat_per_frame() {
        let (control, chunks) = valid();
        let found = animation(&canvas(), &control, &chunks, 1, &[])
            .unwrap()
            .unwrap();
        // The canvas and the 2x2 frame.
        assert_eq!(found.pixels(), 12 + 4);
        let mut expected = Vec::new();
        write_chunk(&mut expected, *b"acTL", &control);
        write_chunk(&mut expected, *b"fcTL", &chunks[0]);
        write_chunk(&mut expected, *b"fcTL", &chunks[1]);
        // The two fdAT chunks are joined into one, numbered 2.
        write_chunk(&mut expected, *b"fdAT", &fdat(2, &frame(2, 2)));
        assert_eq!(written(&found), expected);
    }

    #[test]
    fn a_default_image_outside_the_animation_is_renumbered_from_zero() {
        // No fcTL before the data: the frames are numbered from 0.
        let chunks = vec![fctl(0, (4, 3), (0, 0)), fdat(1, &frame(4, 3))];
        let control = actl(1, 3);
        let found = animation(&canvas(), &control, &chunks, 0, &[])
            .unwrap()
            .unwrap();
        assert_eq!(found.pixels(), 24);
        let mut out = Vec::new();
        found.write_first(&mut out);
        assert_eq!(out, Vec::<u8>::new());
        found.write_frames(&mut out);
        let mut expected = Vec::new();
        write_chunk(&mut expected, *b"fcTL", &chunks[0]);
        write_chunk(&mut expected, *b"fdAT", &chunks[1]);
        assert_eq!(out, expected);
    }

    #[test]
    fn a_frame_stream_is_cut_at_its_end() {
        let data = [&frame(2, 2)[..], b"payload"].concat();
        let chunks = vec![fctl(0, (2, 2), (0, 0)), fdat(1, &data)];
        let control = actl(1, 0);
        let found = animation(&canvas(), &control, &chunks, 0, &[])
            .unwrap()
            .unwrap();
        assert!(!written(&found).windows(7).any(|w| w == b"payload"));
    }

    #[test]
    fn a_defective_frame_control_keeps_the_frames_before_its_frame() {
        let (control, chunks) = three();
        assert_eq!(kept(&control, &chunks, 1, &[]), 3);
        // The chunk, the offset in it, the bytes written there, and the
        // frames kept: those before the frame the defective chunk lies in.
        let edits: [(&str, usize, usize, &[u8], usize); 15] = [
            ("zero width", 3, 4, &[0; 4], 2),
            ("zero height", 3, 8, &[0; 4], 2),
            ("past the right edge", 3, 12, &[0, 0, 0, 4], 2),
            ("past the bottom", 3, 16, &[0, 0, 0, 3], 2),
            ("offset overflow", 3, 12, &[0xFF; 4], 2),
            ("dispose operation", 3, 24, &[3], 2),
            ("blend operation", 1, 25, &[2], 1),
            ("first frame x offset", 0, 12, &[0, 0, 0, 1], 0),
            ("first frame width", 0, 4, &[0, 0, 0, 3], 0),
            ("first frame height", 0, 8, &[0, 0, 0, 2], 0),
            // Height 2 at y 1: inside the canvas, but not covering it.
            (
                "first frame y offset",
                0,
                8,
                &[0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1],
                0,
            ),
            ("sequence gap in an fcTL", 3, 0, &[0, 0, 0, 4], 2),
            ("sequence gap in an fdAT", 4, 0, &[0, 0, 0, 5], 2),
            ("sequence repeat", 2, 0, &[0, 0, 0, 1], 1),
            ("sequence start", 0, 0, &[0, 0, 0, 1], 0),
        ];
        for (what, index, at, bytes, frames) in edits {
            let mut changed = chunks.clone();
            changed[index][at..at + bytes.len()].copy_from_slice(bytes);
            assert_eq!(kept(&control, &changed, 1, &[]), frames, "{what}");
        }
        // The positive controls: every known operation other than 0, the
        // highest of each included, keeps every frame, in frame 0 and in a
        // frame after the data.
        for (index, at, operation) in [(3, 24, 1), (3, 24, 2), (1, 25, 1), (0, 24, 2), (0, 25, 1)] {
            let mut changed = chunks.clone();
            changed[index][at] = operation;
            assert_eq!(
                kept(&control, &changed, 1, &[]),
                3,
                "operation {operation} at byte {at} of chunk {index}"
            );
        }
        // An fcTL of 27 bytes, and one of 25 bytes, which the feed of these
        // tests would take for an fdAT, so it is handed over directly.
        let mut long = chunks.clone();
        long[3].push(0);
        assert_eq!(kept(&control, &long, 1, &[]), 2, "27 bytes");
        let mut frames = feed(&canvas(), &control, &chunks[..3], 1, &[]);
        frames.control(Some(&chunks[3][..25]), false);
        assert_eq!(kept_of(&canvas(), &control, frames), 2, "25 bytes");
    }

    /// A case of misplaced or missing frame chunks: what it shows, the frame
    /// count of acTL, the chunks, how many come before the data, the fdAT
    /// chunks parted from the chunk before them, and the frames kept.
    type Case = (
        &'static str,
        u32,
        Vec<Vec<u8>>,
        usize,
        &'static [usize],
        usize,
    );

    /// Checks every case of `cases` on the 4×3 canvas.
    fn check(cases: Vec<Case>) {
        for (what, frames, chunks, before, gaps, expected) in cases {
            assert_eq!(
                kept(&actl(frames, 0), &chunks, before, gaps),
                expected,
                "{what}"
            );
        }
    }

    /// The `fcTL` of a frame that covers the 4×3 canvas, numbered 0.
    fn whole() -> Vec<u8> {
        fctl(0, (4, 3), (0, 0))
    }

    #[test]
    fn a_misplaced_frame_chunk_before_the_data_leaves_no_frame() {
        let data = || fdat(1, &frame(4, 3));
        check(vec![
            ("fdAT before the data", 1, vec![whole(), data()], 2, &[], 0),
            (
                "fdAT before the data and frame 0",
                1,
                vec![fdat(0, &frame(4, 3)), whole()],
                2,
                &[],
                0,
            ),
            (
                "two default frames",
                2,
                vec![whole(), fctl(1, (4, 3), (0, 0))],
                2,
                &[],
                0,
            ),
            // The same, where acTL counts the two as one frame.
            (
                "two default frames as one",
                1,
                vec![whole(), fctl(1, (4, 3), (0, 0))],
                2,
                &[],
                0,
            ),
            ("no frames", 1, vec![], 0, &[], 0),
        ]);
    }

    #[test]
    fn a_missing_or_parted_frame_chunk_cuts_the_animation_where_it_lies() {
        let data = || fdat(1, &frame(4, 3));
        let long = [&whole()[..], &[0]].concat();
        check(vec![
            (
                "a middle frame without data",
                3,
                vec![
                    whole(),
                    fctl(1, (2, 2), (0, 0)),
                    fctl(2, (2, 2), (0, 0)),
                    fdat(3, &frame(2, 2)),
                ],
                1,
                &[],
                1,
            ),
            (
                "a last frame without data",
                3,
                vec![
                    whole(),
                    fctl(1, (2, 2), (0, 0)),
                    fdat(2, &frame(2, 2)),
                    fctl(3, (2, 2), (0, 0)),
                ],
                1,
                &[],
                2,
            ),
            // Right after the data, an fdAT lies in no frame: frame 0 has
            // ended with the data, and it is kept.
            (
                "fdAT right after the data",
                2,
                vec![
                    whole(),
                    data(),
                    fctl(2, (2, 2), (0, 0)),
                    fdat(3, &frame(2, 2)),
                ],
                1,
                &[],
                1,
            ),
            (
                "fdAT without a frame",
                1,
                vec![fdat(0, &frame(4, 3))],
                0,
                &[],
                0,
            ),
            (
                "short fdAT",
                2,
                vec![whole(), fctl(1, (2, 2), (0, 0)), vec![0, 0, 2]],
                1,
                &[],
                1,
            ),
            ("fcTL of 27 bytes", 1, vec![long.clone(), data()], 0, &[], 0),
            (
                "fcTL of 27 bytes after frame 0",
                2,
                vec![whole(), [&fctl(1, (2, 2), (0, 0))[..], &[0]].concat()],
                1,
                &[],
                1,
            ),
            // An fdAT that holds only its number still starts the frame's
            // data, so a later one apart from it breaks the frame, as the
            // png crate reads it.
            (
                "empty fdAT, then data apart",
                1,
                vec![whole(), fdat(1, &[]), fdat(2, &frame(4, 3))],
                0,
                &[2],
                0,
            ),
            (
                "fdAT apart in the second frame",
                2,
                vec![
                    whole(),
                    fctl(1, (2, 2), (0, 0)),
                    fdat(2, &frame(2, 2)[..5]),
                    fdat(3, &frame(2, 2)[5..]),
                ],
                1,
                &[3],
                1,
            ),
            // A frame past the count of acTL is no defect.
            (
                "a defective frame past the count",
                1,
                vec![whole(), long],
                1,
                &[],
                1,
            ),
        ]);
        // The positive controls: the same frame after the data is kept, also
        // after an fdAT that holds only its number.
        assert_eq!(kept(&actl(1, 0), &[whole(), data()], 0, &[]), 1);
        let split = vec![whole(), fdat(1, &[]), fdat(2, &frame(4, 3))];
        assert_eq!(kept(&actl(1, 0), &split, 0, &[]), 1);
    }

    #[test]
    fn a_lost_chunk_is_a_defect_where_it_lies() {
        let (control, chunks) = three();
        let lost = |indices: &[usize]| {
            kept_of(
                &canvas(),
                &control,
                feed_lost(&canvas(), &control, &chunks, 1, &[], indices),
            )
        };
        // The fcTL of the third frame, the fdAT of the second, and frame 0's
        // fcTL, before the data.
        assert_eq!(lost(&[3]), 2);
        assert_eq!(lost(&[2]), 1);
        assert_eq!(lost(&[0]), 0);
        // An fdAT right after the data lies in no frame: frame 0 is kept.
        let mut frames = feed(&canvas(), &control, &chunks[..1], 1, &[]);
        frames.data(None, false, false);
        assert_eq!(kept_of(&canvas(), &control, frames), 1);
        // An acTL lost before the data might have announced the animation:
        // no frame is kept, even after a valid one.
        let mut frames = Frames::new(&canvas());
        frames.announce(Some(&control));
        frames.announce(None);
        for (index, data) in chunks.iter().enumerate() {
            if index % 2 == 1 || index == 0 {
                frames.control(Some(data), index == 0);
            } else {
                frames.data(Some(data), false, true);
            }
        }
        assert_eq!(kept_of(&canvas(), &control, frames), 0);
    }

    #[test]
    fn the_count_actl_announces_caps_the_frames_kept() {
        let (_, chunks) = three();
        // Fewer: the frames past the count are dropped whatever they hold.
        assert_eq!(kept(&actl(1, 7), &chunks, 1, &[]), 1);
        assert_eq!(kept(&actl(2, 7), &chunks, 1, &[]), 2);
        let mut broken = chunks.clone();
        broken[3][12..16].copy_from_slice(&[0, 0, 0, 9]);
        broken[4] = fdat(4, b"junk");
        assert_eq!(kept(&actl(2, 7), &broken, 1, &[]), 2);
        // A frame past the count is never held.
        assert_eq!(feed(&canvas(), &actl(2, 7), &chunks, 1, &[]).held.len(), 1);
        // More: every frame is kept, and acTL announces the three.
        assert_eq!(kept(&actl(5, 7), &chunks, 1, &[]), 3);
    }

    #[test]
    fn rules_the_zlib_check_would_also_catch_hold_before_the_totals() {
        // A 16384x16384 canvas holds the pixel limit exactly, so any frame
        // outside the animation passes it. A frame without data, or one of
        // zero width, is the defect before the totals are counted, so no
        // frame is kept and nothing is refused or inflated.
        let last_without_data = vec![fctl(0, (1, 1), (0, 0))];
        assert_eq!(
            feed(&big(), &actl(1, 0), &last_without_data, 0, &[]).finish(),
            Ok(None)
        );
        // The same for a frame in the middle that has no data: the next
        // frame control ends it.
        let middle_without_data = vec![
            fctl(0, (1, 1), (0, 0)),
            fctl(1, (1, 1), (0, 0)),
            fdat(2, b"junk"),
        ];
        assert_eq!(
            feed(&big(), &actl(2, 0), &middle_without_data, 0, &[]).finish(),
            Ok(None)
        );
        // Zero width or zero height, with a stream that decodes to the zero
        // bytes such a frame would hold.
        for size in [(0, 1), (1, 0)] {
            let empty = vec![fctl(0, size, (0, 0)), fdat(1, &stored(&[]))];
            assert_eq!(kept(&actl(1, 0), &empty, 0, &[]), 0, "{size:?}");
        }
    }

    #[test]
    fn frame_data_that_does_not_decode_is_the_defect_in_its_frame() {
        let (control, chunks) = three();
        for data in [frame(1, 0), frame(1, 2), b"not zlib".to_vec(), Vec::new()] {
            // In the third frame, two are kept; in the second, one, and the
            // third frame's data, after it, is never inflated.
            let mut third = chunks.clone();
            third[4] = fdat(4, &data);
            assert_eq!(kept(&control, &third, 1, &[]), 2, "{data:?}");
            let mut second = chunks.clone();
            second[2] = fdat(2, &data);
            assert_eq!(kept(&control, &second, 1, &[]), 1, "{data:?}");
        }
        // A wrong Adler-32 in the second frame.
        let mut adler = chunks.clone();
        *adler[2].last_mut().unwrap() ^= 1;
        assert_eq!(kept(&control, &adler, 1, &[]), 1);
        // Outside the animation, a lone frame that does not decode leaves
        // no frame.
        let lone = vec![fctl(0, (2, 2), (0, 0)), fdat(1, &frame(2, 1))];
        assert_eq!(kept(&actl(1, 0), &lone, 0, &[]), 0);
        // A row with filter type 5, which no decoder knows, in the second
        // row of the frame; type 4 there is kept.
        for (filter, frames) in [(5, 1), (0xFF, 1), (4, 2)] {
            let mut rows = pixels(2, 2, 3);
            rows[7] = filter;
            let mut changed = valid().1;
            changed[2] = fdat(2, &stored(&rows));
            changed.truncate(3);
            assert_eq!(kept(&actl(2, 0), &changed, 1, &[]), frames, "{filter}");
        }
    }

    #[test]
    fn interlaced_frames_are_checked_pass_by_pass() {
        // A 4x3 Adam7 RGB canvas; a 3x3 frame has passes of 1x1, 1x1, 2x1,
        // 1x2 and 3x1 pixels (Adam7's second and third passes are empty
        // at 3x3): 33 bytes of rows.
        let interlaced = ihdr(4, 3, 8, 2, 1)[8..21].to_vec();
        let passes = [(1, 1), (1, 1), (2, 1), (1, 2), (3, 1)];
        let rows: Vec<u8> = passes.iter().flat_map(|&(w, h)| pixels(w, h, 3)).collect();
        let chunks = vec![fctl(0, (3, 3), (0, 0)), fdat(1, &stored(&rows))];
        let control = actl(1, 0);
        let frames = feed(&interlaced, &control, &chunks, 0, &[]);
        assert_eq!(kept_of(&interlaced, &control, frames), 1);
        // The same rows read as a still frame are 3 rows of 10 bytes: 30
        // bytes, not the 31 of the passes.
        assert_eq!(kept(&control, &chunks, 0, &[]), 0);
    }

    #[test]
    fn the_totals_refuse_at_one_above_the_limits_before_any_inflate() {
        // Data that does not decode: a refusal proves the totals came first.
        let at_limit = vec![fctl(0, (16_384, 16_384), (0, 0)), fdat(1, b"junk")];
        let over = vec![
            fctl(0, (16_384, 16_384), (0, 0)),
            fctl(1, (1, 1), (0, 0)),
            fdat(2, b"junk"),
        ];
        // The canvas counts once more when the default image is not frame 0.
        assert_eq!(
            feed(&big(), &actl(1, 0), &at_limit, 0, &[]).finish(),
            Err(TooLarge {
                frames: 1,
                pixels: 2 * 256 * 1024 * 1024
            })
        );
        let first = vec![fctl(0, (16_384, 16_384), (0, 0))];
        assert!(
            animation(&big(), &actl(1, 0), &first, 1, &[])
                .unwrap()
                .is_some()
        );
        assert_eq!(
            feed(&big(), &actl(2, 0), &over, 1, &[]).finish(),
            Err(TooLarge {
                frames: 2,
                pixels: 256 * 1024 * 1024 + 1
            })
        );
        // 4096 frames pass the frame limit; 4097 do not.
        let at = many(4096);
        assert!(
            animation(&tiny(), &actl(4096, 0), &at, 1, &[])
                .unwrap()
                .is_some()
        );
        let above = many(4097);
        assert_eq!(
            feed(&tiny(), &actl(4097, 0), &above, 1, &[]).finish(),
            Err(TooLarge {
                frames: 4097,
                pixels: 4097
            })
        );
    }

    #[test]
    fn the_totals_count_the_frames_the_chunks_keep() {
        // 4097 frames, of which acTL announces 4096: the last is past the
        // count and does not count.
        let above = many(4097);
        let fewer = actl(4096, 0);
        let frames = feed(&tiny(), &fewer, &above, 1, &[]);
        assert_eq!(kept_of(&tiny(), &fewer, frames), 4096);
        // The same where the last frame's fcTL is the first defect.
        let mut gap = above.clone();
        let last = gap.len() - 2;
        gap[last][0..4].copy_from_slice(&[0, 0, 0xFF, 0xFF]);
        let all = actl(4097, 0);
        let frames = feed(&tiny(), &all, &gap, 1, &[]);
        assert_eq!(kept_of(&tiny(), &all, frames), 4096);
        // A frame whose data does not decode still counts: the totals are
        // counted before any image data is inflated.
        let mut junk = above;
        let data = junk.len() - 1;
        junk[data] = fdat(2 * 4096, b"junk");
        assert_eq!(
            feed(&tiny(), &actl(4097, 0), &junk, 1, &[]).finish(),
            Err(TooLarge {
                frames: 4097,
                pixels: 4097
            })
        );
        // Past the pixel limit by one frame whose fdAT is the first defect:
        // the frame is dropped, its pixels with it, and the default image,
        // frame 0, is within the totals and kept.
        let cut = vec![
            fctl(0, (16_384, 16_384), (0, 0)),
            fctl(1, (1, 1), (0, 0)),
            fdat(9, &stored(&[0, 0])),
        ];
        let two = actl(2, 0);
        let frames = feed(&big(), &two, &cut, 1, &[]);
        assert_eq!(frames.pixels, 256 * 1024 * 1024);
        let found = frames.finish().unwrap().unwrap();
        assert_eq!((found.count(), found.pixels()), (1, 256 * 1024 * 1024));
        // The pixels of the frames before a dropped one still count: here
        // the first frame after the data passes the pixel total, and the
        // second is dropped at its fdAT.
        let past = vec![
            fctl(0, (1, 1), (0, 0)),
            fdat(1, &stored(&[0, 0])),
            fctl(2, (1, 1), (0, 0)),
            fdat(9, &stored(&[0, 0])),
        ];
        assert_eq!(
            feed(&big(), &actl(2, 0), &past, 0, &[]).finish(),
            Err(TooLarge {
                frames: 1,
                pixels: 256 * 1024 * 1024 + 1
            })
        );
        // A canvas past the pixel total on its own, which a pixel budget set
        // above 256 Mi pixels lets through: with no frame kept, the file is
        // a still image and nothing is refused.
        let wide = ihdr(16_385, 16_384, 8, 0, 0)[8..21].to_vec();
        let lone = vec![fctl(0, (1, 1), (0, 0)), fdat(9, &stored(&[0, 0]))];
        assert_eq!(feed(&wide, &actl(1, 0), &lone, 0, &[]).finish(), Ok(None));
    }

    #[test]
    fn a_frame_without_data_is_dropped_before_the_next_one_passes_the_count() {
        // acTL announces 4097 frames: the default image and 4096 after the
        // data, the last of which has no fdAT, and an fcTL past the count
        // follows it. That fcTL first ends the last frame, which is then the
        // defect and is dropped, so 4096 frames are kept, within the totals.
        // Were the count checked first, the frame would stay kept by its
        // chunks, and the totals passed would refuse the upload.
        let mut chunks = many(4097);
        chunks.pop();
        chunks.push(fctl(2 * 4096, (1, 1), (0, 0)));
        let control = actl(4097, 0);
        let frames = feed(&tiny(), &control, &chunks, 1, &[]);
        assert_eq!(kept_of(&tiny(), &control, frames), 4096);
        // The positive control: with its fdAT, the last frame is kept, and
        // the 4097 frames pass the frame total.
        let mut whole = many(4097);
        whole.push(fctl(2 * 4096 + 1, (1, 1), (0, 0)));
        assert!(matches!(
            feed(&tiny(), &control, &whole, 1, &[]).finish(),
            Err(TooLarge { frames: 4097, .. })
        ));
    }

    #[test]
    fn the_data_of_a_dropped_frame_is_never_joined_to_another() {
        // 4097 frames; the last, past the frame total and so not held, is
        // dropped at its second fdAT. The 4096 frames before it are kept,
        // each with exactly its own data.
        let mut chunks = many(4097);
        let pixel = stored(&[0, 0]);
        chunks.push(fdat(2 * 4096 + 1, &pixel));
        chunks.push(fdat(9, b"junk"));
        let control = actl(4097, 0);
        let frames = feed(&tiny(), &control, &chunks, 1, &[]);
        let found = frames.finish().unwrap().unwrap();
        assert_eq!(found.count(), 4096);
        assert!(found.frames.iter().all(|frame| frame.data == pixel));
    }

    /// The frame chunks of a 1×1 animation of `count` frames, the default
    /// image the first of them.
    fn many(count: u32) -> Vec<Vec<u8>> {
        let mut chunks = vec![fctl(0, (1, 1), (0, 0))];
        for index in 1..count {
            chunks.push(fctl(2 * index - 1, (1, 1), (0, 0)));
            chunks.push(fdat(2 * index, &stored(&[0, 0])));
        }
        chunks
    }

    #[test]
    fn frame_data_is_copied_only_for_frames_within_the_totals() {
        // Within the totals and the announced count, every frame is held.
        let at = many(4096);
        assert_eq!(feed(&tiny(), &actl(4096, 0), &at, 1, &[]).held.len(), 4095);
        // A frame past the frame total is not held while it is open, since
        // a defect could still drop it; once it has ended the refusal is
        // certain, and the frames before it are freed, so a crafted file of
        // many frames costs no copies of their data.
        let above = many(4097);
        assert_eq!(
            feed(&tiny(), &actl(4097, 0), &above, 1, &[]).held.len(),
            4095
        );
        let further = many(4098);
        assert_eq!(
            feed(&tiny(), &actl(4098, 0), &further, 1, &[]).held.len(),
            0
        );
        // Past the count acTL announces, with no acTL, and from the first
        // defect on, nothing is held.
        assert_eq!(feed(&tiny(), &actl(3, 0), &at, 1, &[]).held.len(), 2);
        assert_eq!(feed(&tiny(), &actl(0, 0), &at, 1, &[]).held.len(), 0);
        let mut gap = at.clone();
        gap[5][0..4].copy_from_slice(&[0, 0, 0, 9]);
        assert_eq!(feed(&tiny(), &actl(4096, 0), &gap, 1, &[]).held.len(), 2);
        // Past the pixel total: a 16384x16384 canvas and one more pixel.
        let one = vec![fctl(0, (1, 1), (0, 0)), fdat(1, &stored(&[0, 0]))];
        let control = actl(1, 0);
        let frames = feed(&big(), &control, &one, 0, &[]);
        assert_eq!(frames.held.len(), 0);
        assert_eq!(frames.pixels, 256 * 1024 * 1024 + 1);
    }

    #[test]
    fn only_the_first_valid_actl_announces_an_animation() {
        assert!(control_fits(&actl(1, 0)));
        assert!(control_fits(&actl(MAX_COUNT, MAX_COUNT)));
        assert!(!control_fits(&actl(0, 0)));
        assert!(!control_fits(&actl(MAX_COUNT + 1, 0)));
        assert!(!control_fits(&actl(1, MAX_COUNT + 1)));
        assert!(!control_fits(&actl(1, 0)[..7]));
        assert!(!control_fits(&[&actl(1, 0)[..], &[0]].concat()));
        // An invalid one does not take the place; a later valid one does
        // not replace the first.
        let (invalid, one, two) = (actl(0, 0), actl(1, 0), actl(2, 0));
        let mut frames = Frames::new(&canvas());
        frames.announce(Some(&invalid));
        frames.announce(Some(&one));
        frames.announce(Some(&two));
        assert_eq!(frames.actl, Some(&one[..]));
    }

    #[test]
    fn the_written_actl_keeps_the_play_count_and_counts_the_kept_frames() {
        // Two of three frames kept: acTL says 2, with the uploaded 7 plays,
        // and the chunks are numbered again from 0.
        let (control, mut chunks) = three();
        chunks[4] = fdat(4, b"junk");
        let found = animation(&canvas(), &control, &chunks, 1, &[])
            .unwrap()
            .unwrap();
        let mut expected = Vec::new();
        write_chunk(&mut expected, *b"acTL", &actl(2, 7));
        write_chunk(&mut expected, *b"fcTL", &chunks[0]);
        write_chunk(&mut expected, *b"fcTL", &chunks[1]);
        write_chunk(&mut expected, *b"fdAT", &chunks[2]);
        assert_eq!(written(&found), expected);
        // The encoder's pixel count is that of the kept frames.
        assert_eq!(found.pixels(), 12 + 4);
    }

    /// The defects a property below plants in one frame after the data.
    const DEFECTS: [&str; 9] = [
        "fcTL sequence",
        "fdAT sequence",
        "outside",
        "no fdAT",
        "data",
        "short fdAT",
        "lost fcTL",
        "lost fdAT",
        "fdAT apart",
    ];

    proptest! {
        #[test]
        fn any_frame_inside_the_canvas_is_kept_and_any_outside_leaves_no_frame(
            width in 1..6_u32,
            height in 1..6_u32,
            x in 0..6_u32,
            y in 0..6_u32,
        ) {
            let canvas = ihdr(5, 5, 8, 2, 0)[8..21].to_vec();
            let chunks = vec![fctl(0, (width, height), (x, y)), fdat(1, &frame(width, height))];
            let control = actl(1, 0);
            let found = animation(&canvas, &control, &chunks, 0, &[]).unwrap();
            prop_assert_eq!(found.is_some(), x + width <= 5 && y + height <= 5);
            if let Some(found) = found {
                prop_assert_eq!(found.pixels(), 25 + u64::from(width * height));
            }
        }

        #[test]
        fn arbitrary_frame_chunks_never_panic_and_never_grow(
            control in proptest::collection::vec(any::<u8>(), 0..10),
            chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..40), 0..6),
            before in 0..3_usize,
        ) {
            if let Ok(Some(found)) = animation(&canvas(), &control, &chunks, before, &[]) {
                let out = written(&found);
                let input: usize = 12 + control.len() + chunks.iter().map(|c| 12 + c.len()).sum::<usize>();
                prop_assert!(out.len() <= input);
                let controls = out.windows(4).filter(|w| *w == b"fcTL").count();
                prop_assert_eq!(usize::try_from(word(&out, 8)).unwrap(), controls);
            }
        }

        #[test]
        fn the_kept_frames_are_the_fewer_of_those_found_and_announced(
            frames in 1..12_u32,
            declared in 1..12_u32,
        ) {
            // A 1x1 animation of `frames` frames whose acTL announces
            // `declared`: the first of them are kept, as many as both allow,
            // and never more than announced are held.
            let chunks = many(frames);
            let control = actl(declared, 0);
            let fed = feed(&tiny(), &control, &chunks, 1, &[]);
            prop_assert!(fed.held.len() < usize::try_from(declared).unwrap());
            prop_assert_eq!(
                kept_of(&tiny(), &control, fed),
                usize::try_from(frames.min(declared)).unwrap()
            );
        }

        #[test]
        fn a_defect_keeps_exactly_the_frames_before_its_frame(
            count in 2..10_u32,
            at in any::<prop::sample::Index>(),
            defect in prop::sample::select(DEFECTS.to_vec()),
        ) {
            // Frame `cut`, 1 or later, of a 1x1 animation of `count` frames
            // holds the defect; its fcTL is chunk 2 * cut - 1 and its fdAT
            // chunk 2 * cut.
            let cut = u32::try_from(at.index(usize::try_from(count - 1).unwrap())).unwrap() + 1;
            let (control_at, data_at) = (2 * cut - 1, 2 * cut);
            let (control_index, data_index) = (
                usize::try_from(control_at).unwrap(),
                usize::try_from(data_at).unwrap(),
            );
            let mut chunks = many(count);
            let (mut gaps, mut lost) = (Vec::new(), Vec::new());
            match defect {
                "fcTL sequence" => chunks[control_index][0..4].copy_from_slice(&[0, 0, 0xFF, 0]),
                "fdAT sequence" => chunks[data_index][0..4].copy_from_slice(&[0, 0, 0xFF, 0]),
                "outside" => chunks[control_index][12..16].copy_from_slice(&[0, 0, 0, 1]),
                "no fdAT" => {
                    chunks.remove(data_index);
                }
                "data" => chunks[data_index] = fdat(data_at, b"junk"),
                "short fdAT" => chunks[data_index] = vec![0, 0, 0],
                "lost fcTL" => lost.push(control_index),
                "lost fdAT" => lost.push(data_index),
                _ => {
                    // An empty fdAT, then the data apart from it.
                    chunks.insert(data_index, fdat(data_at, &[]));
                    chunks[data_index + 1][0..4].copy_from_slice(&(data_at + 1).to_be_bytes());
                    gaps.push(data_index + 1);
                }
            }
            let control = actl(count, 0);
            let fed = feed_lost(&tiny(), &control, &chunks, 1, &gaps, &lost);
            let found = fed.finish().unwrap().and_then(|a| a.checked(&tiny())).unwrap();
            // The first `cut` frames, as written from the valid animation.
            let whole = many(count);
            let announced = actl(cut, 0);
            let expected = animation(&tiny(), &announced, &whole[..usize::try_from(2 * cut - 1).unwrap()], 1, &[])
                .unwrap()
                .unwrap();
            prop_assert_eq!(written(&found), written(&expected));
        }
    }
}
