//! The RIFF container of WebP (RFC 9649 §2), read and written chunk by chunk.
//!
//! A chunk is a `FourCC`, a 32-bit little-endian payload size, the payload,
//! and a zero pad byte when the size is odd. The reader follows libwebp,
//! whose demuxer (`ParseVP8XChunks` and `StoreFrame` in
//! `libwebp@1.6.0:src/demux/demux.c`) and decoder (`ParseOptionalChunks` in
//! `src/dec/webp_dec.c`) agree here: every chunk they read, pad byte
//! included, must fit inside the RIFF size, and bytes after the RIFF size
//! are not part of the file, which libwebp ignores and the strip drops.

use super::super::DecodeDefect;

/// One chunk of a WebP file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Chunk<'a> {
    /// The chunk's `FourCC`, for example `VP8 ` or `EXIF`.
    pub(super) fourcc: [u8; 4],
    /// The payload, without the pad byte.
    pub(super) payload: &'a [u8],
}

/// The chunks of the file, after `RIFF`, the RIFF size and `WEBP`, up to
/// the end that the RIFF size gives.
///
/// - `bytes`: the whole input, which the header check has already accepted
///   as a WebP (its RIFF size is at least 12).
///
/// # Errors
///
/// [`DecodeDefect::WebpRiffSize`] when the input ends before the end that
/// its RIFF size gives, and [`DecodeDefect::WebpCutShort`] when it is too
/// short for the size field.
pub(super) fn body(bytes: &[u8]) -> Result<&[u8], DecodeDefect> {
    let size = bytes
        .get(4..8)
        .and_then(|size| <[u8; 4]>::try_from(size).ok())
        .ok_or(DecodeDefect::WebpCutShort)?;
    let end = usize::try_from(u32::from_le_bytes(size))
        .ok()
        .and_then(|size| size.checked_add(8))
        .ok_or(DecodeDefect::WebpCutShort)?;
    bytes.get(12..end).ok_or(DecodeDefect::WebpRiffSize)
}

/// The chunk that starts at offset `at` of `body`, and the offset after it
/// and its pad byte.
///
/// - `body`: the chunks of a file or of an `ANMF` frame.
/// - `at`: the offset of the chunk's `FourCC`.
///
/// # Errors
///
/// [`DecodeDefect::WebpCutShort`] when fewer than 8 bytes are left at `at`,
/// or the chunk and its pad byte run past the end of `body`.
pub(super) fn chunk_at(body: &[u8], at: usize) -> Result<(Chunk<'_>, usize), DecodeDefect> {
    let header: [u8; 8] = body
        .get(at..at.checked_add(8).ok_or(DecodeDefect::WebpCutShort)?)
        .and_then(|header| header.try_into().ok())
        .ok_or(DecodeDefect::WebpCutShort)?;
    let [f0, f1, f2, f3, s0, s1, s2, s3] = header;
    let size = usize::try_from(u32::from_le_bytes([s0, s1, s2, s3]))
        .map_err(|_| DecodeDefect::WebpCutShort)?;
    let start = at + 8;
    let end = start.checked_add(size).ok_or(DecodeDefect::WebpCutShort)?;
    let payload = body.get(start..end).ok_or(DecodeDefect::WebpCutShort)?;
    let next = end + (size & 1);
    if next > body.len() {
        return Err(DecodeDefect::WebpCutShort);
    }
    let fourcc = [f0, f1, f2, f3];
    Ok((Chunk { fourcc, payload }, next))
}

/// The chunk that starts `body` when it ends `body` without the pad byte
/// that its odd size needs, a layout that [`chunk_at`] refuses. libwebp's
/// decoder reads such a bitstream chunk in a simple file, since it measures
/// the chunk by its own size (`ParseVP8Header` in `src/dec/webp_dec.c`),
/// while its demuxer refuses it.
///
/// - `body`: the chunks of a file, after the RIFF header.
pub(super) fn unpadded_chunk(body: &[u8]) -> Option<Chunk<'_>> {
    let [f0, f1, f2, f3, s0, s1, s2, s3] = *body.first_chunk::<8>()?;
    let size = usize::try_from(u32::from_le_bytes([s0, s1, s2, s3])).ok()?;
    (size % 2 == 1 && body.len().checked_sub(8) == Some(size)).then(|| Chunk {
        fourcc: [f0, f1, f2, f3],
        payload: &body[8..],
    })
}

/// Every chunk of `body`, which must end exactly after its last chunk; for
/// the tests, which list the chunks of a file. The strip itself walks a
/// file chunk by chunk with [`chunk_at`], because libwebp reads the chunks
/// after a frame's image as chunks of the file, across the end of the
/// frame's `ANMF` chunk.
///
/// - `body`: the chunks of a file or of an `ANMF` frame.
#[cfg(test)]
pub(super) fn chunks(body: &[u8]) -> Result<Vec<Chunk<'_>>, DecodeDefect> {
    let mut chunks = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let (chunk, next) = chunk_at(body, at)?;
        chunks.push(chunk);
        at = next;
    }
    Ok(chunks)
}

/// The 32-bit little-endian size field for `len` bytes.
///
/// Every payload written here is at most as large as a chunk or a file that
/// was read with a 32-bit size, so `len` always fits.
fn size_field(len: usize) -> [u8; 4] {
    u32::try_from(len)
        .expect("written payloads are never larger than the 32-bit ones they come from")
        .to_le_bytes()
}

/// Appends the chunk `fourcc` with `payload`, and its pad byte, to `out`.
///
/// - `out`: the chunks written so far.
/// - `fourcc`: the chunk's `FourCC`.
/// - `payload`: the chunk's payload.
pub(super) fn write_chunk(out: &mut Vec<u8>, fourcc: [u8; 4], payload: &[u8]) {
    write_chunk_with(out, fourcc, |out| out.extend_from_slice(payload));
}

/// Appends the chunk `fourcc` to `out`, its payload written by `payload`,
/// then its pad byte. The size field is filled in afterwards, so a chunk
/// that holds other chunks, such as an `ANMF` frame, is written in place
/// rather than built in a buffer of its own and copied.
///
/// - `out`: the chunks written so far.
/// - `fourcc`: the chunk's `FourCC`.
/// - `payload`: appends the payload to the buffer it is given.
pub(super) fn write_chunk_with(
    out: &mut Vec<u8>,
    fourcc: [u8; 4],
    payload: impl FnOnce(&mut Vec<u8>),
) {
    let start = out.len();
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(&[0; 4]);
    payload(out);
    let len = out.len() - start - 8;
    out[start + 4..start + 8].copy_from_slice(&size_field(len));
    if len % 2 == 1 {
        out.push(0);
    }
}

/// A WebP file whose chunks `chunks` writes, in file order, into the buffer
/// it is given, which already holds the RIFF header. Writing in place keeps
/// one copy of the image data, however large.
///
/// - `chunks`: appends the chunks, with [`write_chunk`] or
///   [`write_chunk_with`].
pub(super) fn file_with(chunks: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut out = b"RIFF\0\0\0\0WEBP".to_vec();
    chunks(&mut out);
    // The RIFF size counts everything after itself: `WEBP` and the chunks.
    let size = size_field(out.len() - 8);
    out[4..8].copy_from_slice(&size);
    out
}

/// A WebP file that holds `chunks`, which [`write_chunk`] wrote; for the
/// tests, which build files from chunk lists.
///
/// - `chunks`: the chunks, in file order.
#[cfg(test)]
pub(super) fn file(chunks: &[u8]) -> Vec<u8> {
    file_with(|out| out.extend_from_slice(chunks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*};

    #[test]
    fn reads_chunks_with_their_pad_bytes() {
        let mut body = Vec::new();
        write_chunk(&mut body, *b"ABCD", b"odd");
        write_chunk(&mut body, *b"EFGH", b"even");
        assert_eq!(body, b"ABCD\x03\0\0\0odd\0EFGH\x04\0\0\0even");
        assert_eq!(
            chunks(&body),
            Ok(vec![
                Chunk {
                    fourcc: *b"ABCD",
                    payload: b"odd"
                },
                Chunk {
                    fourcc: *b"EFGH",
                    payload: b"even"
                },
            ])
        );
    }

    #[test]
    fn a_chunk_or_pad_byte_past_the_end_is_refused() {
        assert_eq!(
            chunks(b"ABCD\x03\0\0\0odd"),
            Err(DecodeDefect::WebpCutShort)
        );
        assert_eq!(
            chunks(b"ABCD\x04\0\0\0odd"),
            Err(DecodeDefect::WebpCutShort)
        );
        assert_eq!(chunks(b"ABCD\0\0"), Err(DecodeDefect::WebpCutShort));
        assert_eq!(
            chunks(b"ABCD\xFF\xFF\xFF\xFFodd\0"),
            Err(DecodeDefect::WebpCutShort)
        );
        assert_eq!(chunks(b""), Ok(vec![]));
        // An offset whose header would end past the address space, which no
        // caller passes, is refused rather than wrapped.
        assert_eq!(
            chunk_at(b"ABCD", usize::MAX),
            Err(DecodeDefect::WebpCutShort)
        );
    }

    #[test]
    fn a_chunk_without_its_pad_byte_is_found_only_where_it_ends_the_body() {
        assert_eq!(
            unpadded_chunk(b"ABCD\x03\0\0\0odd"),
            Some(Chunk {
                fourcc: *b"ABCD",
                payload: b"odd"
            })
        );
        // With its pad byte, with more after it, of an even size, or cut
        // short, it is not one.
        for body in [
            &b"ABCD\x03\0\0\0odd\0"[..],
            b"ABCD\x03\0\0\0oddXY",
            b"ABCD\x04\0\0\0even",
            b"ABCD\x05\0\0\0odd",
            b"ABCD\x03\0\0",
            b"",
        ] {
            assert_eq!(unpadded_chunk(body), None, "{body:?}");
        }
    }

    #[test]
    fn the_body_ends_where_the_riff_size_says() {
        let whole = file(b"ABCD\x00\0\0\0");
        assert_eq!(whole, b"RIFF\x0C\0\0\0WEBPABCD\0\0\0\0");
        assert_eq!(body(&whole), Ok(&b"ABCD\0\0\0\0"[..]));
        let appended = [&whole[..], b"<script>"].concat();
        assert_eq!(body(&appended), Ok(&b"ABCD\0\0\0\0"[..]));
        assert_eq!(
            body(&whole[..whole.len() - 1]),
            Err(DecodeDefect::WebpRiffSize)
        );
        // Too short for the size field: the header check passes no such
        // input, but the reader refuses it on its own.
        assert_eq!(body(b"RIFF\x0C\0"), Err(DecodeDefect::WebpCutShort));
    }

    #[test]
    fn nested_chunks_are_written_in_place() {
        let whole = file_with(|out| {
            write_chunk_with(out, *b"ANMF", |out| {
                out.extend_from_slice(b"head");
                write_chunk(out, *b"VP8L", b"odd");
            });
        });
        assert_eq!(
            whole,
            b"RIFF\x1C\0\0\0WEBPANMF\x10\0\0\0headVP8L\x03\0\0\0odd\0"
        );
        let frame = chunks(body(&whole).unwrap()).unwrap()[0];
        assert_eq!(frame.fourcc, *b"ANMF");
        assert_eq!(
            chunks(&frame.payload[4..]).unwrap()[0].payload,
            b"odd".as_slice()
        );
        // An odd nested payload gets its pad byte too.
        let odd = file_with(|out| write_chunk_with(out, *b"ABCD", |out| out.push(7)));
        assert_eq!(odd, b"RIFF\x0E\0\0\0WEBPABCD\x01\0\0\0\x07\0");
    }

    proptest! {
        #[test]
        fn writing_in_place_equals_writing_a_built_payload(
            inner in vec((any::<[u8; 4]>(), vec(any::<u8>(), 0..9)), 0..4),
            head in vec(any::<u8>(), 0..5),
        ) {
            let mut built = head.clone();
            for (fourcc, payload) in &inner {
                write_chunk(&mut built, *fourcc, payload);
            }
            let mut expected = Vec::new();
            write_chunk(&mut expected, *b"ANMF", &built);
            let in_place = file_with(|out| {
                write_chunk_with(out, *b"ANMF", |out| {
                    out.extend_from_slice(&head);
                    for (fourcc, payload) in &inner {
                        write_chunk(out, *fourcc, payload);
                    }
                });
            });
            prop_assert_eq!(in_place, file(&expected));
        }

        #[test]
        fn written_chunks_read_back(payloads in vec((any::<[u8; 4]>(), vec(any::<u8>(), 0..9)), 0..5)) {
            let mut written = Vec::new();
            for (fourcc, payload) in &payloads {
                write_chunk(&mut written, *fourcc, payload);
            }
            let whole = file(&written);
            let read = chunks(body(&whole).unwrap()).unwrap();
            let read: Vec<([u8; 4], Vec<u8>)> =
                read.iter().map(|chunk| (chunk.fourcc, chunk.payload.to_vec())).collect();
            prop_assert_eq!(read, payloads);
        }

        #[test]
        fn a_last_chunk_is_unpadded_exactly_when_its_pad_byte_is_missing(
            fourcc in any::<[u8; 4]>(),
            payload in vec(any::<u8>(), 0..9),
        ) {
            let mut padded = Vec::new();
            write_chunk(&mut padded, fourcc, &payload);
            let unpadded = &padded[..8 + payload.len()];
            let expected = Chunk { fourcc, payload: &payload };
            if payload.len() % 2 == 1 {
                prop_assert_eq!(unpadded_chunk(unpadded), Some(expected));
                prop_assert_eq!(chunk_at(unpadded, 0), Err(DecodeDefect::WebpCutShort));
            } else {
                prop_assert_eq!(unpadded_chunk(unpadded), None);
            }
            prop_assert_eq!(unpadded_chunk(&padded), None);
            prop_assert_eq!(chunk_at(&padded, 0), Ok((expected, padded.len())));
        }

        #[test]
        fn arbitrary_bodies_never_panic(body in vec(any::<u8>(), 0..64)) {
            if let Ok(chunks) = chunks(&body) {
                let total: usize = chunks.iter().map(|c| 8 + c.payload.len() + c.payload.len() % 2).sum();
                prop_assert_eq!(total, body.len());
            }
        }
    }
}
