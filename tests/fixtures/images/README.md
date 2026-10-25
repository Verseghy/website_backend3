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
