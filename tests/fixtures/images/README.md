# Image corpus

Sample images for the tests of the upload image pipeline
(`src/domain/image_optimise/`,
[ADR 0015](../../../docs/design/adr/0015-media-separate-static-server-media-library-image-optimisatio.md)
§7, "Image tests"). `tests/domain/image_optimise.rs` reads every file here
and checks the header it gives.

## Rules for every file

- **Licensed for this repository.** A file made for backend3 is part of the
  repository and shares its licence (`AGPL-3.0-only`). A file from anywhere
  else needs a licence that allows it here and a row in
  [`THIRD_PARTY.md`](../../../THIRD_PARTY.md) with its path, source, author,
  SPDX licence identifier and any change made to it, in the PR that adds it;
  the first such PR adds a "Test images" section there for these rows.
- **No people**, no faces, no names, and no metadata that points at anyone:
  no real GPS positions, camera serial numbers or author fields. A test that
  needs such metadata writes invented values into the file.
- **Listed.** Every file has a row in `CORPUS` in
  `tests/domain/image_optimise.rs` with its format and stored dimensions; a
  file that is not listed fails the tests.
- **Small.** A few kilobytes each, so the repository stays light.

## Layout

One folder per format, plus `over-budget/`.

| Path | What it covers |
|---|---|
| `jpeg/baseline.jpg` | Baseline JPEG (SOF0), 4:2:0, with a JFIF APP0 segment |
| `jpeg/progressive.jpg` | Progressive JPEG (SOF2) |
| `jpeg/grayscale.jpg` | One component, so a frame header of 11 bytes |
| `jpeg/comment.jpg` | A COM segment before the frame header |
| `jpeg/exif-orientation.jpg` | `baseline.jpg` with an EXIF APP1 segment that holds only Orientation 6 (rotate 90°); the stored dimensions stay 67×41 |
| `jpeg/camera.jpg` | `progressive.jpg` with the metadata a camera or phone writes, every value invented: an EXIF APP1 segment (Orientation 6, make, model, date, a serial number, a GPS position in open sea, and a thumbnail in IFD1), an XMP APP1 segment, an ICC profile (APP2), an MPF index (APP2), IPTC data (APP13), a comment, and a second image appended after the end of image |
| `jpeg/cmyk.jpg` | CMYK with an Adobe APP14 segment, which the optimiser decodes but does not re-encode |
| `jpeg/high-quality.jpg` | Quality 95 with full-resolution chroma (4:4:4), as cameras save their originals: the re-encode at quality 85 is more than 5 % smaller but stays below the SSIM floor, so the stripped original is kept (below) |
| `jpeg/soft-444.jpg` | The picture with soft edges (a blur of 1.5 pixels), at quality 95 with 4:4:4 chroma: the re-encode reaches the SSIM floor and is kept, with its chroma halved to 4:2:0 |
| `jpeg/low-quality.jpg` | Quality 60: compressed harder than the re-encode at quality 85 would be, so the optimiser keeps the stripped original |
| `png/rgb.png` | 8-bit truecolour |
| `png/rgba-interlaced.png` | 8-bit truecolour with alpha, Adam7 interlaced |
| `png/palette.png` | 8-bit indexed colour |
| `png/grey16.png` | 16-bit greyscale |
| `png/animated.png` | Animated PNG (`acTL`, `fcTL`, `fdAT`) |
| `gif/still.gif` | GIF89a, one frame |
| `gif/gif87a.gif` | GIF87a |
| `gif/animated.gif` | GIF89a, three frames and a loop extension |
| `webp/lossy.webp` | Simple lossy WebP (`VP8 `) |
| `webp/lossless.webp` | Simple lossless WebP (`VP8L`) |
| `webp/lossy-alpha.webp` | Extended WebP (`VP8X`) with an alpha channel |
| `webp/animated.webp` | Extended WebP (`VP8X`) with three frames |
| `over-budget/camera-50mp.*` | The header, and nothing after it, of an 8688×5792 image (50,320,896 pixels, a common 50-megapixel camera size) in each format. For JPEG the header runs to the marker of the first scan; the GIF file holds no frame, so its logical screen is its canvas |

Every image outside `over-budget/` is 67×41, which is not square, so a
reader that swaps the width and the height fails.

The files in `over-budget/` end exactly where their header ends. The test
shows that they are refused for the pixel budget, which is just under their
size, and accepted with a larger budget: the refusal comes from the header
alone, since there is nothing else to decode.

## The re-encoded JPEGs and the SSIM floor

ADR 0015 §7 asks every re-encoded JPEG of the corpus to reach a luma SSIM
of at least 0.98, and
[ADR 0033](../../../docs/design/adr/0033-image-optimisation-ssim-floor-reference-parsing-cpu-limits-a.md)
§1 makes that floor part of the keep rule: a re-encode below it is
discarded and the stripped original stored. The corpus covers each side:

- `progressive.jpg` and `camera.jpg` were saved at quality 85 with 4:2:0
  chroma, much as the re-encode writes them, and reach 0.990, so they are
  re-encoded. `camera.jpg` is `progressive.jpg` with metadata spliced in,
  so it adds the metadata cases, not a second picture.
- `high-quality.jpg` is the usual camera original, quality 95 with 4:4:4
  chroma. Its re-encode at quality 85 is more than 5 % smaller but reaches
  only 0.974, as half of the photographs measured in
  [`docs/design/image-encoders.md`](../../../docs/design/image-encoders.md)
  do, so it keeps its stripped original.
- `soft-444.jpg` is the same kind of original with softer edges. Its
  re-encode reaches 0.994 and is kept, so the chroma test still sees a
  4:4:4 original whose chroma the re-encode halves to 4:2:0 (ADR 0033 §2).

## How the files were made

All files were generated for backend3 on 2026-10-25, with ImageMagick
7.1.2-34 (libjpeg-turbo 3.1.3, libpng 1.6.58, libwebp 1.6.0) and ffmpeg 8.1.3.
The pictures are a two-colour gradient with a rectangle and a circle drawn on
it. Other tool versions write other bytes, so the files are kept as they are
rather than regenerated.

```sh
draw() { magick -size 67x41 gradient:'#1f6f8b-#f2c14e' -fill '#c0392b' -draw 'rectangle 6,5 30,22' -fill '#2e7d32' -draw 'circle 48,28 56,34' "$@"; }
png='-define png:exclude-chunks=date,time'
draw -strip -quality 85 -sampling-factor 2x2 jpeg/baseline.jpg
draw -strip -quality 85 -interlace JPEG jpeg/progressive.jpg
draw -strip -colorspace Gray -quality 85 jpeg/grayscale.jpg
draw -strip -quality 85 -set comment 'backend3 test corpus' jpeg/comment.jpg
draw -strip $png PNG24:png/rgb.png
draw -strip -alpha set -channel A -evaluate set 60% +channel -interlace PNG $png PNG32:png/rgba-interlaced.png
draw -strip -colors 16 $png PNG8:png/palette.png
draw -strip -colorspace Gray -depth 16 $png PNG:png/grey16.png
draw -strip gif/still.gif
draw -strip GIF87:gif/gif87a.gif
draw -strip -quality 80 webp/lossy.webp
draw -strip -define webp:lossless=true webp/lossless.webp
draw -strip -alpha set -channel A -evaluate set 60% +channel -quality 80 webp/lossy-alpha.webp

frames() { magick \( -size 67x41 gradient:'#1f6f8b-#f2c14e' \) \( -size 67x41 gradient:'#f2c14e-#1f6f8b' \) \( -size 67x41 xc:'#2e7d32' \) -strip -set delay 20 -loop 0 "$@"; }
frames gif/animated.gif
frames -quality 80 webp/animated.webp
ffmpeg -i gif/animated.gif -map_metadata -1 -fflags +bitexact -flags +bitexact -plays 0 -f apng png/animated.png

# jpeg/exif-orientation.jpg: baseline.jpg with an APP1 segment spliced in after
# its 18-byte APP0 segment: "Exif\0\0", a little-endian TIFF header and one IFD
# entry, Orientation (0x0112) = 6.
{ head -c 20 jpeg/baseline.jpg
  printf '\xff\xe1\x00\x22Exif\x00\x00II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00\x06\x00\x00\x00\x00\x00\x00\x00'
  tail -c +21 jpeg/baseline.jpg; } > jpeg/exif-orientation.jpg

draw -strip -colorspace CMYK -quality 90 jpeg/cmyk.jpg
draw -strip -quality 60 jpeg/low-quality.jpg
draw -strip -quality 95 -sampling-factor 1x1 jpeg/high-quality.jpg
draw -blur 0x1.5 -strip -quality 95 -sampling-factor 1x1 jpeg/soft-444.jpg

# jpeg/camera.jpg: progressive.jpg with metadata segments spliced in after its
# SOI, and the EXIF thumbnail appended after its end of image. segment() writes
# a marker and a payload file, with the length field that counts itself.
segment() { len=$(( $(stat -c %s "$2") + 2 )); printf "\xff\x$1\x$(printf %02x $((len >> 8)))\x$(printf %02x $((len & 255)))"; cat "$2"; }
magick -size 8x5 xc:'#1f6f8b' -strip -quality 50 thumb.jpg     # 287 bytes
# A 584-byte RGB profile with sRGB primaries and transfer, written by lcms2.
ffmpeg -f lavfi -i 'color=c=0x1f6f8b:s=67x41,format=rgb24,setparams=color_primaries=bt709:color_trc=iec61966-2-1,iccgen=force=1' \
  -frames:v 1 -map_metadata -1 -fflags +bitexact -flags +bitexact icc.png
magick icc.png icc:profile.icc
# EXIF, big-endian: IFD0 (Make "backend3 test", Model "Corpus", Orientation 6,
# DateTime, the EXIF and GPS IFD pointers, IFD1 at 258), the EXIF IFD
# (BodySerialNumber "TEST-0001"), the GPS IFD (1°2'3" N, 4°5'6" E), and IFD1
# with the thumbnail at offset 288.
{ printf 'Exif\0\0MM\x00\x2a\x00\x00\x00\x08\x00\x06'
  printf '\x01\x0f\x00\x02\x00\x00\x00\x0e\x00\x00\x00\x56\x01\x10\x00\x02\x00\x00\x00\x07\x00\x00\x00\x64'
  printf '\x01\x12\x00\x03\x00\x00\x00\x01\x00\x06\x00\x00\x01\x32\x00\x02\x00\x00\x00\x14\x00\x00\x00\x6c'
  printf '\x87\x69\x00\x04\x00\x00\x00\x01\x00\x00\x00\x80\x88\x25\x00\x04\x00\x00\x00\x01\x00\x00\x00\x9c'
  printf '\x00\x00\x01\x02backend3 test\0Corpus\0\0'; printf '2026:10:25 12:00:00\0'
  printf '\x00\x01\xa4\x31\x00\x02\x00\x00\x00\x0a\x00\x00\x00\x92\x00\x00\x00\x00TEST-0001\0\x00\x04'
  printf '\x00\x01\x00\x02\x00\x00\x00\x02N\x00\x00\x00\x00\x02\x00\x05\x00\x00\x00\x03\x00\x00\x00\xd2'
  printf '\x00\x03\x00\x02\x00\x00\x00\x02E\x00\x00\x00\x00\x04\x00\x05\x00\x00\x00\x03\x00\x00\x00\xea\x00\x00\x00\x00'
  printf '\x00\x00\x00\x01\x00\x00\x00\x01\x00\x00\x00\x02\x00\x00\x00\x01\x00\x00\x00\x03\x00\x00\x00\x01'
  printf '\x00\x00\x00\x04\x00\x00\x00\x01\x00\x00\x00\x05\x00\x00\x00\x01\x00\x00\x00\x06\x00\x00\x00\x01'
  printf '\x00\x02\x02\x01\x00\x04\x00\x00\x00\x01\x00\x00\x01\x20\x02\x02\x00\x04\x00\x00\x00\x01\x00\x00\x01\x1f\x00\x00\x00\x00'
  cat thumb.jpg; } > exif.bin
{ printf 'http://ns.adobe.com/xap/1.0/\0'
  printf '<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="backend3 corpus"/></rdf:RDF></x:xmpmeta>'; } > xmp.bin
{ printf 'ICC_PROFILE\0\x01\x01'; cat profile.icc; } > icc.bin
printf 'MPF\0MM\x00\x2a\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00' > mpf.bin
printf 'Photoshop 3.0\0''8BIM\x04\x04\x00\x00\x00\x00\x00\x17\x1c\x02\x00\x00\x02\x00\x04\x1c\x02\x78\x00\x0btest record\x00' > iptc.bin
printf 'backend3 corpus comment' > com.bin
{ head -c 2 jpeg/progressive.jpg
  segment e1 exif.bin; segment e1 xmp.bin; segment e2 icc.bin; segment e2 mpf.bin; segment ed iptc.bin; segment fe com.bin
  tail -c +3 jpeg/progressive.jpg
  cat thumb.jpg; } > jpeg/camera.jpg

# over-budget/: a solid colour at 8688x5792, cut after the header. The JPEG
# header ends with the marker of the first scan (SOS, FF DA), at byte 189,
# right after the frame header (SOF0); the PNG header after IHDR (33 bytes);
# the GIF header after the logical screen descriptor (13); the WebP header
# after the VP8 frame header (30).
big() { ffmpeg -f lavfi -i color=c=0x1f6f8b:s=8688x5792 -frames:v 1 -map_metadata -1 -fflags +bitexact -flags +bitexact "$@"; }
big -pix_fmt yuvj420p -q:v 5 full.jpg && head -c 189 full.jpg > over-budget/camera-50mp.jpg
big -pix_fmt rgb24 full.png && head -c 33 full.png > over-budget/camera-50mp.png
big full.gif && head -c 13 full.gif > over-budget/camera-50mp.gif
big -c:v libwebp -quality 80 full.webp && head -c 30 full.webp > over-budget/camera-50mp.webp
```
