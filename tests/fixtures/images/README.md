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
| `png/animated.png` | Animated PNG (`acTL`, `fcTL`, `fdAT`): three frames, the first of them the default image |
| `png/animated-metadata.png` | `animated.png` with a `tEXt` comment and an `eXIf` that holds Orientation 6 and an invented camera make |
| `png/metadata.png` | 8-bit truecolour with `cHRM`, `bKGD`, `pHYs`, a `tEXt` comment and an `eXIf` that holds Orientation 6, a camera make, a body serial number and a GPS position, all invented |
| `png/icc-profile.png` | 8-bit truecolour with an `iCCP` chunk holding an invented 464-byte display profile, `cHRM`, and the same profile again as text in `zTXt`, as ImageMagick writes it |
| `png/payload.png` | `rgb.png` with a private chunk (`prVt`) before the image data and an HTML snippet after `IEND` |
| `png/hdr.png` | 16-bit truecolour with the HDR colour chunks of the PNG third edition, all values invented: `cICP` (BT.2020 primaries, the PQ transfer, full range), `mDCV` (a BT.2020 mastering display of 1000 cd/m²) and `cLLI`, and a `tEXt` comment |
| `gif/still.gif` | GIF89a, one frame |
| `gif/gif87a.gif` | GIF87a |
| `gif/animated.gif` | GIF89a, three frames and a loop extension |
| `gif/metadata.gif` | The frames of `animated.gif` with a comment and an ICC profile extension before every frame and an XMP application extension before the trailer |
| `webp/lossy.webp` | Simple lossy WebP (`VP8 `) |
| `webp/lossless.webp` | Simple lossless WebP (`VP8L`) |
| `webp/lossy-alpha.webp` | Extended WebP (`VP8X`) with an alpha channel |
| `webp/animated.webp` | Extended WebP (`VP8X`) with three frames |
| `webp/lossy-metadata.webp` | Lossy WebP at quality 95 with an ICC profile, EXIF (Orientation 6, an invented GPS position and serial number) and XMP; quality 95 makes the re-encode at 85 worth keeping |
| `webp/lossless-metadata.webp` | `lossless.webp`'s picture with the same metadata |
| `webp/animated-metadata.webp` | `animated.webp`'s frames with the same metadata |
| `webp/lossless-noise.webp` | Simple lossless WebP of grey noise, which a lossy encoder at quality 85 would store 23 % smaller, so treating it as lossy would change its pixels |
| `webp/lossy-alpha-q95.webp` | `lossy-alpha.webp`'s picture at quality 95, a transparent image whose re-encode at 85 is kept |
| `webp/lossy-textured.webp` | The picture with Gaussian grain added, at quality 95, as a textured photograph is saved: the re-encode at quality 85 is much more than 5 % smaller but stays below the SSIM floor, so the stripped original is kept (below) |
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

## The re-encoded lossy WebPs and the SSIM floor

The same floor holds for a still lossy WebP (ADR 0033 §1), measured on the
luma planes that libwebp decodes:

- `lossy-metadata.webp` and `lossy-alpha-q95.webp`, one opaque and one
  transparent, were saved at quality 95 from the smooth picture. Their
  re-encodes at quality 85 reach 0.990 and are kept; the transparent one
  keeps its `ALPH` chunk byte for byte.
- `lossy-textured.webp` is the same picture with grain, at quality 95. Its
  re-encode is 0.39 of its size but reaches only 0.961, so it keeps its
  stripped original, as the textured photographs measured in
  [`docs/design/image-encoders.md`](../../../docs/design/image-encoders.md)
  do.
- `lossy.webp` and `lossy-alpha.webp`, at quality 80, gain less than 5 %
  from a re-encode at quality 85, so the size test keeps their stripped
  originals before any SSIM is measured.

## How the files were made

All files were generated for backend3 on 2026-10-25, with ImageMagick
7.1.2-34 (libjpeg-turbo 3.1.3, libpng 1.6.58, libwebp 1.6.0), ffmpeg 8.1.3
and, for the metadata files, exiv2 0.28.9.
The pictures are a two-colour gradient with a rectangle and a circle drawn on
it, except `webp/lossless-noise.webp`, which is ImageMagick's random noise
from a fixed seed, and `webp/lossy-textured.webp`, which adds ImageMagick's
Gaussian noise from a fixed seed to the picture. Other tool versions write
other bytes, so the files are kept as they are rather than regenerated.

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
draw -seed 7 -attenuate 0.6 +noise Gaussian -strip -quality 95 webp/lossy-textured.webp
draw -strip -alpha set -channel A -evaluate set 60% +channel -quality 95 webp/lossy-alpha-q95.webp
magick -seed 7 -size 67x41 xc:gray +noise Random -strip -define webp:lossless=true webp/lossless-noise.webp

frames() { magick \( -size 67x41 gradient:'#1f6f8b-#f2c14e' \) \( -size 67x41 gradient:'#f2c14e-#1f6f8b' \) \( -size 67x41 xc:'#2e7d32' \) -strip -set delay 20 -loop 0 "$@"; }
frames gif/animated.gif
frames -quality 80 webp/animated.webp
ffmpeg -i gif/animated.gif -map_metadata -1 -fflags +bitexact -flags +bitexact -plays 0 -f apng png/animated.png

# png/metadata.png: a comment, a density and invented EXIF fields (exiv2 0.28.9).
draw -set comment 'backend3 test corpus' -density 72 $png PNG24:png/metadata.png
exiv2 -M'set Exif.Image.Orientation Short 6' -M'set Exif.Image.Make Ascii backend3' \
  -M'set Exif.Photo.BodySerialNumber Ascii 0000000000' \
  -M'set Exif.GPSInfo.GPSLatitudeRef Ascii N' -M'set Exif.GPSInfo.GPSLatitude Rational 1/1 2/1 3/1' \
  -M'set Exif.GPSInfo.GPSLongitudeRef Ascii E' -M'set Exif.GPSInfo.GPSLongitude Rational 4/1 5/1 6/1' \
  png/metadata.png

# png/icc-profile.png: an ICC v2.1 display profile written by hand (sRGB
# primaries, gamma 2.2, description "backend3 test"), attached by ImageMagick.
# -strip would remove iCCP too, so only the date, time and bKGD are excluded.
{ printf '\x00\x00\x01\xd0\x00\x00\x00\x00\x02\x10\x00\x00mntrRGB XYZ '
  printf '\x07\xea\x00\x0a\x00\x19\x00\x00\x00\x00\x00\x00acsp'
  printf '\x00%.0s' $(seq 1 28)
  printf '\x00\x00\xf6\xd6\x00\x01\x00\x00\x00\x00\xd3\x2d'
  printf '\x00%.0s' $(seq 1 48)
  printf '\x00\x00\x00\x09'
  printf 'desc\x00\x00\x00\xf0\x00\x00\x00\x68cprt\x00\x00\x01\x58\x00\x00\x00\x16'
  printf 'wtpt\x00\x00\x01\x70\x00\x00\x00\x14rXYZ\x00\x00\x01\x84\x00\x00\x00\x14'
  printf 'gXYZ\x00\x00\x01\x98\x00\x00\x00\x14bXYZ\x00\x00\x01\xac\x00\x00\x00\x14'
  printf 'rTRC\x00\x00\x01\xc0\x00\x00\x00\x0egTRC\x00\x00\x01\xc0\x00\x00\x00\x0e'
  printf 'bTRC\x00\x00\x01\xc0\x00\x00\x00\x0e'
  printf 'desc\x00\x00\x00\x00\x00\x00\x00\x0ebackend3 test\x00'
  printf '\x00%.0s' $(seq 1 78)
  printf 'text\x00\x00\x00\x00AGPL-3.0-only\x00\x00\x00'
  printf 'XYZ \x00\x00\x00\x00\x00\x00\xf6\xd6\x00\x01\x00\x00\x00\x00\xd3\x2d'
  printf 'XYZ \x00\x00\x00\x00\x00\x00\x6f\xa2\x00\x00\x38\xf5\x00\x00\x03\x90'
  printf 'XYZ \x00\x00\x00\x00\x00\x00\x62\x99\x00\x00\xb7\x85\x00\x00\x18\xda'
  printf 'XYZ \x00\x00\x00\x00\x00\x00\x24\xa0\x00\x00\x0f\x84\x00\x00\xb6\xcf'
  printf 'curv\x00\x00\x00\x00\x00\x00\x00\x01\x02\x33\x00\x00'; } > corpus.icc
draw -profile corpus.icc -define png:exclude-chunks=date,time,bKGD PNG24:png/icc-profile.png

# png/payload.png: rgb.png with a prVt chunk spliced in after IHDR (its CRC
# from coreutils cksum) and an HTML snippet appended after IEND.
crc=$(printf 'prVtbackend3 test data' | cksum -a crc32b --raw | od -An -tx1 | tr -d ' \n' | sed 's/../\\x&/g')
{ head -c 33 png/rgb.png
  printf '\x00\x00\x00\x12prVtbackend3 test data'; printf "$crc"
  tail -c +34 png/rgb.png
  printf '<html><body>backend3 test payload after IEND</body></html>\n'; } > png/payload.png

# png/animated-metadata.png and png/hdr.png: chunks spliced in after IHDR
# (the first 33 bytes). chunk() writes a chunk: its length, type, data and
# CRC (coreutils cksum).
chunk() {
  printf "$(printf '%08x' "$(stat -c %s "$2")" | sed 's/../\\x&/g')"
  printf '%s' "$1"; cat "$2"
  { printf '%s' "$1"; cat "$2"; } | cksum -a crc32b --raw
}
printf 'Comment\0backend3 test corpus' > text.bin
# EXIF, little-endian: IFD0 with Make "backend3" and Orientation 6.
printf 'II*\0\x08\0\0\0\x02\0\x0f\x01\x02\0\x09\0\0\0\x26\0\0\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0backend3\0' > exif.bin
{ head -c 33 png/animated.png; chunk tEXt text.bin; chunk eXIf exif.bin
  tail -c +34 png/animated.png; } > png/animated-metadata.png
# cICP: primaries 9 (BT.2020), transfer 16 (PQ), matrix 0, full range.
# mDCV: the BT.2020 primaries and D65 in units of 0.00002, 1000 cd/m² and
# 0.0001 cd/m² in units of 0.0001 cd/m². cLLI: 1000 and 400 cd/m².
draw -strip -depth 16 $png PNG48:hdr-base.png
printf '\x09\x10\x00\x01' > cicp.bin
printf '\x8a\x48\x39\x08\x21\x34\x9b\xaa\x19\x96\x08\xfc\x3d\x13\x40\x42\x00\x98\x96\x80\x00\x00\x00\x01' > mdcv.bin
printf '\x00\x98\x96\x80\x00\x3d\x09\x00' > clli.bin
{ head -c 33 hdr-base.png; chunk cICP cicp.bin; chunk mDCV mdcv.bin; chunk cLLI clli.bin
  chunk tEXt text.bin; tail -c +34 hdr-base.png; } > png/hdr.png

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

# The metadata files. corpus.icc is an invented ICC profile: a 132-byte v2
# header for an RGB display profile with no tags, which no decoder applies.
# The EXIF values are invented: a GPS position in the open sea off West
# Africa and a serial number of zeros.
icc() { printf '\0\0\0\x84\0\0\0\0\x02\x10\0\0mntrRGB XYZ \x07\xea\0\x0a\0\x19\0\0\0\0\0\0acsp'
        head -c 28 /dev/zero; printf '\0\0\xf6\xd6\0\x01\0\0\0\0\xd3\x2d'; head -c 52 /dev/zero; }
icc > corpus.icc
meta() { exiv2 -M'set Exif.Image.Orientation Short 6' -M'set Exif.Photo.BodySerialNumber Ascii 0000000000' \
  -M'set Exif.GPSInfo.GPSLatitudeRef Ascii N' -M'set Exif.GPSInfo.GPSLatitude Rational 1/1 2/1 3/1' \
  -M'set Exif.GPSInfo.GPSLongitudeRef Ascii E' -M'set Exif.GPSInfo.GPSLongitude Rational 4/1 5/1 6/1' \
  -M'set Xmp.xmp.CreatorTool backend3 test corpus' "$1" \
  && cp corpus.icc "${1%.webp}.icc" && exiv2 -iC "$1" && rm "${1%.webp}.icc"; }
draw -strip -quality 95 webp/lossy-metadata.webp && meta webp/lossy-metadata.webp
draw -strip -define webp:lossless=true webp/lossless-metadata.webp && meta webp/lossless-metadata.webp
frames -quality 80 webp/animated-metadata.webp && meta webp/animated-metadata.webp

# gif/metadata.gif: ImageMagick writes the comment and the ICC profile before
# every frame. The XMP application extension goes before the trailer: the
# packet, then XMP's 258-byte "magic trailer" (01, FF down to 00, then the
# terminator 00), which lets a GIF reader step over the packet as sub-blocks.
frames -set comment 'backend3 test corpus' -profile corpus.icc metadata-plain.gif
xmp='<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="backend3 test corpus"/></rdf:RDF></x:xmpmeta>'
{ head -c -1 metadata-plain.gif
  printf '\x21\xff\x0bXMP DataXMP%s\x01' "$xmp"
  for i in $(seq 255 -1 0); do printf "\\x$(printf %02x "$i")"; done
  printf '\0\x3b'; } > gif/metadata.gif
```
