# Image encoders

[ADR 0015](adr/0015-media-separate-static-server-media-library-image-optimisatio.md)
§7 has every new image upload optimised before its URL exists, and leaves the
choice of library to measurement: output size, perceptual similarity, CPU
time on the node, and build cost on the hub
([ADR 0022](adr/0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md)).
This file records each measurement and its result, one section per format.
The code is in `src/domain/image_optimise/`; its tests run over the corpus in
`tests/fixtures/images/`.

## JPEG

### Result

JPEG uploads are decoded and re-encoded with **MozJPEG** (the C library, a
fork of libjpeg-turbo), through the [`mozjpeg`](https://crates.io/crates/mozjpeg)
crate 0.10.13 and [`mozjpeg-sys`](https://crates.io/crates/mozjpeg-sys)
2.2.3, which builds MozJPEG 4 from source with the C compiler of the build
stage.

| Setting | Value | Why |
|---|---|---|
| Decoder | libjpeg-turbo's, inside MozJPEG | the decoder browsers use, so the decode check and the re-encode see the pixels the site's visitors see |
| Colour models re-encoded | greyscale and YCbCr, decoded into their own model (no RGB round trip) | RGB, CMYK and YCCK images keep their stripped original; they are rare and need the Adobe segment of their own coding. [ADR 0033](adr/0033-image-optimisation-ssim-floor-reference-parsing-cpu-limits-a.md) §2 amends ADR 0015 §7, which re-encoded every JPEG, to say so |
| Quality | 85 on libjpeg's scale, kept only at a luma SSIM of at least 0.98 | ADR 0015 §7, and the SSIM floor that ADR 0033 §1 adds to the keep rule (below) |
| Encoder settings | MozJPEG's defaults: N. Robidoux's quantisation tables, trellis quantisation, progressive with optimised scans, optimised Huffman tables, 4:2:0 chroma subsampling, also for a 4:4:4 original | the smallest files of every candidate (below); 4:2:0 also for a 4:4:4 original, as ADR 0033 §2 accepts |
| Build features | `default-features = false` | the default adds NASM-assembled SIMD; UBI 9 has no `nasm` package (checked in `ubi9/ubi` with its three repositories on 2026-10-25), so the image builds MozJPEG's portable C code |
| Licence | IJG for `mozjpeg`, `IJG AND Zlib AND BSD-3-Clause` for `mozjpeg-sys` | a `deny.toml` exception scoped to the two crates, which the maintainer accepted; the IJG notice is in [`THIRD_PARTY.md`](../../THIRD_PARTY.md) |

libjpeg reports fatal errors through a callback that must not return, and the
crate unwinds from it back into Rust. Every call into the library is wrapped
in `catch_unwind`, so a corrupt upload becomes an error value, never a panic
of the request.

### Candidates

| Route | Encoder | Decoder | Licences |
|---|---|---|---|
| **MozJPEG (C)** | `mozjpeg` 0.10.13 | the same library | IJG; IJG AND Zlib AND BSD-3-Clause |
| mozjpeg-rs | [`mozjpeg-rs`](https://crates.io/crates/mozjpeg-rs) 0.9.2, a pure-Rust port of MozJPEG's encoder | [`zune-jpeg`](https://crates.io/crates/zune-jpeg) 0.5.15 or [`jpeg-decoder`](https://crates.io/crates/jpeg-decoder) 0.3.2 | BSD-3-Clause (the crate states that it is based in part on the work of the Independent JPEG Group); MIT OR Apache-2.0 OR Zlib; MIT OR Apache-2.0 |
| jpeg-encoder | [`jpeg-encoder`](https://crates.io/crates/jpeg-encoder) 0.7.1, a pure-Rust libjpeg-style encoder (Annex K tables, no trellis) | `zune-jpeg` 0.5.15 | (MIT OR Apache-2.0) AND IJG |

Not measured: `zenjpeg` 0.8.4 (`AGPL-3.0-only OR LicenseRef-Imazen-Commercial`)
and `jpegli-rs` 0.12.0 (`AGPL-3.0-or-later`). Both licences are outside the
`deny.toml` list, and adding them is a decision of its own; both crates are
also under a year old, with fewer than 10,000 downloads each (crates.io,
2026-10-25).

### Method

- **Images.** 20 JPEGs at 2.7 to 14.0 megapixels, 167.6 megapixels in all.
  The *camera* set is 10 photographs from Wikimedia Commons under CC0, with no
  people in them, as published (quality 94-100, mostly 4:4:4, with EXIF, XMP,
  IPTC and ICC metadata): *Agraulis vanillae at Isla Margarita*, *Basílica de
  nuestra señora de la Chiquinquira*, *Big Mac hamburger*, *Ceiling Civitas
  Dei, Entrance of the Cathedral, Aachen, Germany*, *Chlosyne lacinia*,
  *Domesticated goose head, Chaguaramal, Venezuela*, *General Rafael Urdaneta
  Bridge view from the lake to Cabimas side*, *International Commerce Centre on
  Victoria Harbour*, *Széchenyi Chain Bridge in Budapest at night* and *Watch
  tower of the dam, Embalse de los Bermejales, Andalusia, Spain*. The *phone*
  set is the same 10 saved again by ImageMagick at quality 92 with 4:2:0, as
  phone cameras write them. The images are not committed.
- **Pipeline.** Per file, as in production: the metadata strip of this
  module, the decode, the encode at quality 85 with the stripped metadata
  spliced in, and the keep rule as it then stood (at least 5 % smaller;
  the SSIM floor of ADR 0033 §1 came after this measurement).
- **Size.** The bytes stored after the keep rule, summed over a set, divided
  by the stripped originals' bytes. The strip alone took the 20 files from
  68.1 MB to 67.6 MB.
- **Similarity.** Mean SSIM on the luma channel at full resolution, compared
  with the stripped original, with the function the corpus tests and now
  the keep rule use (`src/domain/image_optimise/ssim.rs`, then test code
  that gives the same values bit for bit: 11 × 11 Gaussian window,
  σ = 1.5). It reads
  slightly lower than ffmpeg's `ssim` filter (8 × 8 blocks): 0.950 against
  0.957 on the hardest file, 0.988 against 0.989 on an easy one.
- **CPU and memory.** Thread CPU time of the decode and the encode, one
  thread, release build, on an AMD Ryzen 9 5900X (AVX2), two interleaved
  rounds; and the peak resident memory of one decode and encode of one
  8660 × 5773 image, the 50-megapixel pixel budget: a baseline JPEG with
  4:2:0 chroma at quality 92, of smooth photograph-like content (a 4.3 MiB
  file). Other modes and contents need more memory; the last section has
  them for the chosen encoder. The edge node's CPU is not known yet
  ([ADR 0023](adr/0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)),
  so the ratios matter more than the seconds.
- **Build cost.** The research gap-14 §6.2 method: CPU seconds (user + sys)
  of `cargo build --release --locked` from an empty `target/`. The machine
  was shared with other builds, so two rounds of the whole backend3 build
  varied by 7 % (799 and 857 s for the same tree), more than any candidate
  adds. Each candidate's own dependency subtree was therefore also built
  cold on its own, three rounds; that overstates its share slightly, since
  `libc` and `cc` are in backend3's graph already.
- **The hub's build.** The figures here ran on a local workstation, and
  they stand, as the OpenAPI spike's did
  ([openapi-spike.md](openapi-spike.md) §4): Pipelines-as-Code builds
  `master` only
  ([ADR 0022](adr/0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md)),
  so a branch has no BuildRun to compare. The duration of the first
  `master` BuildRun after this change is added here in the next docs
  change.

### Results

| Route | Stored / stripped, camera set | Stored / stripped, phone set | Mean SSIM (lowest) | CPU per megapixel | Peak memory, 50 MP sample | Build: own subtree cold, CPU s | Crates added |
|---|---:|---:|---:|---:|---:|---:|---:|
| **MozJPEG (C)** | **0.199** | **0.590** | 0.978 (0.950) | 0.113 s | **456 MiB** | 13.7 | 5 |
| mozjpeg-rs + zune-jpeg | 0.199 | 0.590 | 0.978 (0.950) | **0.064 s** | 824 MiB | 15.5 | 10 |
| jpeg-encoder + zune-jpeg | 0.262 | 0.871 | 0.984 (0.958) | 0.022 s | 441 MiB | 1.6 | 3 |

Full cold builds of backend3, mean of two rounds: 828 s without an encoder,
836 s with MozJPEG (+0.9 %), 865 s with mozjpeg-rs and zune-jpeg, 887 s with
mozjpeg-rs and jpeg-decoder, 865 s with jpeg-encoder and zune-jpeg, all
within the 7 % noise.

Why MozJPEG:

1. **Size.** It and mozjpeg-rs write the smallest files (mozjpeg-rs is a
   port of the same encoder; the two differ by under 0.5 % per file).
   jpeg-encoder's files are 32 % larger on the camera set and 48 % larger on
   the phone set, so the 5 % rule keeps fewer of its re-encodes.
2. **The decoder.** zune-jpeg, the fast pure-Rust decoder, decoded a valid
   sequential JPEG with one scan per component (jpeg-encoder's baseline
   output) to wrong pixels without an error: luma SSIM 0.04 against
   libjpeg-turbo's decode of the same file, while libjpeg-turbo, ffmpeg and
   jpeg-decoder agree with each other. A wrong decode would be re-encoded
   and stored. MozJPEG decodes with libjpeg-turbo, as browsers do.
3. **Memory.** 456 MiB for the 50-megapixel sample against 824 MiB for
   mozjpeg-rs; the node is small.
4. **Maturity.** Mozilla first released MozJPEG in 2014. mozjpeg-rs
   0.9.2 is ten months old, and its README says that not all of its code has
   been reviewed by a person.

What it costs: 1.8 times the CPU of mozjpeg-rs (0.113 against 0.064 s per
megapixel here, about 1.4 s for a 12-megapixel photograph, without SIMD), a C
build step of about 14 CPU seconds, `unsafe` FFI inside the dependency, and
the IJG exception. If the CPU time on the node proves too high, mozjpeg-rs
with jpeg-decoder writes the same files; the change stays inside
`src/domain/image_optimise/jpeg/reencode.rs`.

The quantisation tables were measured too: MozJPEG with the Annex K tables
of libjpeg instead of its default ones stores 0.211 and 0.623 of the
stripped bytes (6 % more), with a mean SSIM of 0.980 (0.955 lowest).

### SSIM and quality 85

ADR 0015 §7 calls quality 85 visually lossless and asks the corpus tests to
show an SSIM of at least 0.98 for every re-encoded JPEG. On the 20
photographs, quality 85 does not reach 0.98 for 10 of them, with any of the
encoders (jpeg-encoder: 7 of 20):

| MozJPEG quality | Stored / stripped, camera set | Stored / stripped, phone set | Files below SSIM 0.98 |
|---:|---:|---:|---:|
| 85 | 0.199 | 0.590 | 10 of 20 |
| 88 | 0.229 | 0.684 | 9 of 20 |
| 90 | 0.257 | 0.790 | 3 of 20 |

The files below 0.98 are the ones with fine texture: foliage, fur, water and
a night scene.

The maintainer kept quality 85 and added that measure to the keep rule
([ADR 0033](adr/0033-image-optimisation-ssim-floor-reference-parsing-cpu-limits-a.md)
§1): a re-encode is kept only when it is at least 5 % smaller **and** its
luma SSIM against the decoded stripped original is at least 0.98, and the
stripped original is stored otherwise. On the 20 photographs, the 10 below
0.98 therefore keep their stripped original; the stored-to-stripped ratios
above were measured before the floor and were not measured again. The SSIM
is `src/domain/image_optimise/ssim.rs`, the function that this measurement
and the corpus tests use; it streams row by row, so the pass needs about
520 bytes per image column rather than a copy of the image (its cost is
under "The SSIM pass" below).

The corpus shows the same split. `progressive.jpg` and `camera.jpg` were
saved at quality 85 with 4:2:0 chroma, and their re-encodes reach 0.990, so
they are kept. `high-quality.jpg` (quality 95, 4:4:4), the camera case,
reaches only 0.974, so it keeps its stripped original; `soft-444.jpg`, the
same kind of original with softer edges, reaches 0.994 and is re-encoded
([the corpus README](../../tests/fixtures/images/README.md)).

### Chroma subsampling

The re-encode writes 4:2:0 chroma, MozJPEG's default, whatever the
original's subsampling, as the maintainer accepted (ADR 0033 §2). Camera
originals are mostly 4:4:4 (the camera set above), so their re-encode halves
the colour resolution in both directions, which a luma SSIM cannot see. On
the corpus, the chroma channels of the re-encodes differ from the stripped
originals' by 0.6 to 1.5 levels of 255 on average for the 4:2:0 originals,
and by up to 1.6 for `soft-444.jpg` (4.3 for `high-quality.jpg`, which the
SSIM floor now keeps as it is). The corpus test bounds the difference at 8
levels, so that a re-encode that loses or swaps the colours fails; swapped
Cb and Cr score about 50. What keeping the original's subsampling would cost
in size was not measured.

### Time and memory per upload

Measured with one `optimise_jpeg` call per process, release build, on the
same machine, on 8660 × 5773 images (49,994,180 pixels, just within the
50-megapixel budget). Apart from the comparison sample, ImageMagick made
them with fine grain, so their files are as large as a detailed
photograph's or larger. *Peak* is the process's peak resident memory
(`VmHWM`), which includes the upload read into memory and about 2 MiB for
the harness. *Above the upload* is the peak less the resident memory just
before the call: the stripped copy, plus the decoder's and the encoder's
buffers. This first table was measured before the SSIM floor of ADR 0033
§1; the next section has the same measurement with it.

| Input | File | Kept | Peak | Above the upload | CPU s |
|---|---:|---|---:|---:|---:|
| baseline, 4:2:0, quality 92, smooth (the comparison sample) | 4.3 MiB | re-encode | 453 MiB | 446 MiB | 3.0 |
| baseline, 4:2:0, quality 92 | 14.2 MiB | re-encode | 516 MiB | 499 MiB | 5.5 |
| progressive, 4:2:0, quality 92 | 13.6 MiB | re-encode | 509 MiB | 493 MiB | 5.8 |
| baseline, 4:4:4, quality 95 | 40.2 MiB | re-encode | 563 MiB | 521 MiB | 6.1 |
| progressive, 4:4:4, quality 95 | 37.7 MiB | re-encode | 559 MiB | 518 MiB | 6.6 |
| progressive, greyscale, quality 92 | 12.7 MiB | re-encode | 320 MiB | 304 MiB | 4.8 |
| baseline, CMYK, quality 90 | 55.8 MiB | stripped original | 114 MiB | 56 MiB | 0.9 |
| progressive, CMYK, quality 90 | 52.7 MiB | stripped original | 489 MiB | 434 MiB | 1.9 |
| progressive, 4:4:4, quality 100, heavy noise | 131.7 MiB | re-encode | 862 MiB | 728 MiB | 20.7 |

- **A colour re-encode is the peak.** It holds the decoded samples (3
  bytes per pixel) and MozJPEG's encoder buffers: 9.3 to 10.2 bytes per
  pixel in all, beyond the stripped copy, for the photograph-like files,
  whatever their mode. The last file, at the 128 MiB upload limit, needs
  12.5: its re-encode is large too, and MozJPEG keeps the candidate scans
  of its scan optimisation in memory.
- **An image that is only decoded needs less.** A baseline CMYK or YCCK
  image needs almost nothing beyond its stripped copy, because the decode
  check reads one row at a time and keeps none. A progressive one holds
  libjpeg's coefficient buffer for the whole image, 8 bytes per pixel for
  four components. Greyscale re-encodes need about 6 bytes per pixel.
- **Sizing.** At the 50-megapixel budget, one optimisation needs up to
  about 730 MiB beyond the upload that the caller holds, and the upload
  itself is at most 128 MiB: about 860 MiB on top of the process's
  baseline in the worst case, and about 560 MiB for a detailed 50-megapixel
  camera photograph. That is well above one decoded image (200 MiB as RGBA
  at 50 megapixels), so backend3's memory limit
  ([ADR 0023](adr/0023-operations-hardening-rate-limits-monitoring-backups-secrets.md)
  §8) is sized from these figures. The SSIM pass does not change them
  (next section).
- **Time.** 0.11 to 0.13 CPU seconds per megapixel for a photograph (5.5
  to 6.6 s at the budget), and 0.41 for the extreme file, before the SSIM
  pass. ADR 0015 §7 runs one optimisation at a time per process, on a
  blocking thread.

### The SSIM pass

A re-encode that passes the size test is decoded once more, to its luma
only, one row at a time, and each row goes into the streaming SSIM with the
original's row. Measured on 2026-10-25 on the same machine and in the same
way, on new 8660 × 5773 inputs: an ImageMagick plasma fractal, smooth or
with Gaussian grain added (`-attenuate 0.5 +noise Gaussian`, and 2.5 for the
last file). The grained files' re-encodes are more than 5 % smaller but stay below the
floor (SSIM 0.76 to 0.92), so those files pass the size test, pay for the
whole pass, and still keep their stripped original: the most CPU an upload
of each mode can cost.

| Input | File | Kept | Peak | Above the upload | CPU s | SSIM pass, CPU s |
|---|---:|---|---:|---:|---:|---:|
| baseline, 4:2:0, quality 92, smooth | 2.3 MiB | re-encode (SSIM 0.997) | 445 MiB | 441 MiB | 3.7 | 1.3 to 1.5 |
| baseline, 4:2:0, quality 92, grain | 16.1 MiB | stripped original (SSIM 0.758) | 528 MiB | 510 MiB | 7.8 | 1.8 |
| progressive, 4:4:4, quality 95, grain | 42.3 MiB | stripped original (SSIM 0.779) | 579 MiB | 534 MiB | 9.3 | 1.4 |
| progressive, greyscale, quality 92, grain | 14.0 MiB | stripped original (below the floor) | 332 MiB | 316 MiB | 7.6 | not measured apart |
| progressive, CMYK, quality 90, grain | 55.1 MiB | stripped original (colour model, no SSIM) | 494 MiB | 437 MiB | 2.1 | none |
| progressive, 4:4:4, quality 100, heavy grain | 127.5 MiB | stripped original (SSIM 0.921) | 828 MiB | 698 MiB | 22.2 | 1.8 |

- **CPU.** The pass costs 1.3 to 1.8 CPU seconds at the budget, about 0.03
  to 0.04 s per megapixel: about 1.1 s for the SSIM arithmetic, whatever
  the content, and the rest for the luma decode of the re-encode. It runs
  inside the blocking optimisation, as ADR 0015 §7 has it.
- **Memory.** The pass itself needs about 147 MiB above what the optimiser
  already holds at that point: libjpeg's coefficient buffer for the whole
  progressive re-encode (2 bytes per coefficient, 1.5 components per pixel
  in 4:2:0), plus about 4.3 MiB for the SSIM's 11 rows of sums. It runs
  after the encoder has freed its buffers, which need more, so the peaks
  stay within the first table's range, and the sizing above holds.

### The scan cap

ADR 0033 §4 caps a JPEG at 100 scans, counted by the metadata strip before
the decode check (`MAX_JPEG_SCANS` in `src/domain/image_optimise/jpeg/`).
libjpeg passes over every block of the image for each scan, whatever data
the scan holds, so a scan costs time in proportion to the pixels rather than
to its bytes. To see what a cap bounds, extra scans were appended to a
progressive 8660 × 5773 MozJPEG file of 5 scans and the file decoded as the
decode check decodes it, on the same machine:

| Extra scans | Kind | File | Scans | Decode CPU s |
|---|---|---:|---:|---:|
| none | | 0.3 MiB | 5 | 0.24 |
| luma AC scans of EOB runs (one Huffman code, about 140 bytes each) | first | 0.3 MiB | 100 | 0.51 |
| the same | first | 0.4 MiB | 500 | 1.66 |
| luma AC refinement scans of EOB runs (about 2 KiB each) | refinement | 0.5 MiB | 100 | 0.65 |
| the same | refinement | 1.3 MiB | 500 | 2.36 |
| interleaved DC refinement scans (about 146 KiB each) | refinement | 13.9 MiB | 100 | 0.47 |
| the same | refinement | 71.2 MiB | 500 | 1.37 |

These scans cost 2 to 5 ms each at the budget. The review of this slice
measured a costlier kind on the same machine with the same `mozjpeg` build:
luma AC refinement scans of EOB runs on an 8000 × 6250 greyscale file of its
own (225 KiB at 500 scans). libjpeg's refinement decoder visits every
coefficient position of every block during an EOB run, so each such scan
costs about 27 ms:

| Scans | Decode CPU s |
|---:|---:|
| 2 | 0.09 |
| 102 | 2.84 |
| 500 | 13.79 |

Through `optimise_jpeg`, the 500-scan file took 16.5 s, against 1.2 s for
the 2-scan one.

- **Why 100.** At 100 scans, the costliest kind measured adds at most about
  2.8 CPU seconds to an upload. libjpeg-turbo's TurboJPEG API refuses more
  than 500 scans when asked to refuse "an unreasonably large number of
  scans" (`TJFLAG_LIMITSCANS`, `libjpeg-turbo@36956793:src/turbojpeg.c`), but
  500 would allow about 14 CPU seconds here, more on the node's slower
  cores, while a second upload waits behind the one-at-a-time semaphore
  within the same 120 s `/api/admin` timeout. 100 is libjpeg-turbo's own
  limit for the scan scripts that its cjpeg and jpegtran read (`MAX_SCANS`,
  `src/rdswitch.c`), so no progressive file those tools can write is
  refused. Asked to keep 100, raise it to 500 or lower it to 64, the
  maintainer answered "Keep 100, as built" on 2026-10-26 (research:
  DECISIONS R16, D101; ADR 0033 §4).
- **Real files are far below it.** libjpeg's progressive script writes 10
  scans for a colour image, MozJPEG wrote 4 to 11 for every re-encode
  measured here, and a sequential JPEG has one scan per component.
- **Without a cap**, a 128 MiB upload could hold hundreds of thousands of
  such scans: hours of decoding with the optimiser's semaphore held.

### Decisions

The maintainer answered the three questions this section once left open on
2026-10-25 (research: DECISIONS R15), and ADR 0033 records them:

1. **Quality 85 and SSIM 0.98** (D73): "Keep q85 and add an SSIM ≥ 0.98
   floor to the keep rule, for JPEG and lossy WebP" (ADR 0033 §1).
2. **Chroma subsampling, and RGB, CMYK and YCCK images** (D77): "4:2:0, and
   non-YCbCr files keep the stripped original, in an amending ADR of §7"
   (ADR 0033 §2).
3. **A cap on progressive scans** (D79): "WebP limits as built, the same
   totals for GIF, and a JPEG scan cap" (ADR 0033 §4, and the section above).

## PNG

### Result

PNG uploads are recompressed losslessly by **oxipng 10.2.1** with
libdeflate, after a chunk walk that keeps only what
[ADR 0015](adr/0015-media-separate-static-server-media-library-image-optimisatio.md)
§7 and
[ADR 0033](adr/0033-image-optimisation-ssim-floor-reference-parsing-cpu-limits-a.md)
§6 keep (the last section).

| Setting | Value | Why |
|---|---|---|
| Encoder | oxipng 10.2.1, default features off, libdeflate through `libdeflater` | the smallest files for their CPU time of the candidates below; no command-line tool, no rayon (one optimisation uses one thread), no Zopfli |
| Preset | 2 up to `settings.image_optimisation.png.threshold_pixels` pixels, `large_preset` above it (0, or 1 if the node allows it); the default threshold is 6,000,000, with preset 0 above | ADR 0033 §5: preset 2 below a pixel count and a cheaper preset above it, so a PNG at the pixel budget finishes inside the 120-second `/api/admin` timeout (below). The production values are set from a measurement on the node |
| Pixel count | the canvas, plus every frame of a kept animation at its own size | the encoder recompresses each image of the file |
| Chunks | `StripChunks::None` on the walk's rewrite | oxipng may not swap an `iCCP` for `sRGB` or drop one |
| Pixels | `optimize_alpha` and `scale_16` off; bit-depth reduction off for a 16-bit `tRNS` sample whose bytes differ; greyscale reduction off when `cICP` is kept | no decoded pixel may change, and no colour may be read differently |
| Time limit | none | a deadline would read the clock and make the output depend on the machine; the preset bounds the time instead |
| Licences | oxipng MIT, libdeflater and libdeflate-sys Apache-2.0, the bundled libdeflate MIT, rgb MIT, bytemuck Zlib OR Apache-2.0 OR MIT | all on the `deny.toml` list; `cargo deny` passes |

### How the PNG encoders were measured

| Criterion | Method |
|---|---|
| Output size | The bytes each candidate writes for each measurement image, against the input. |
| Similarity | Lossless candidates must decode to the same pixels. The tests compare every image of every corpus PNG, frames included, and generated files of every colour type, bit depth and interlace method, with an animation frame, as 16-bit RGBA through the `png` crate, a decoder independent of oxipng. |
| CPU time | `/usr/bin/time` user + system seconds of one optimisation, in a release build, on one thread (ADR 0015 §7 runs one optimisation at a time on a blocking thread), with its peak resident memory. |
| Build cost | The CPU seconds (user + system) of a cold `cargo build --release` from an empty `target/`: first of a scratch crate whose only dependency is the candidate, then of backend3 with and without it (research: gap-14 §6.2). CPU seconds approximate the wall time of the same build on one CPU, which is how a cold build on the hub is projected. |
| Licence | `cargo deny check bans licenses sources` passes with the candidate's whole tree (ADR 0022 §4). |

The figures come from a desktop with 24 hardware threads (AMD Ryzen 9
5900X), while other work ran on the same machine, so the CPU seconds carry
a few per cent of noise. The candidates were built with rustc 1.98.1 and
the whole optimiser with the repository's 1.99.1. The presets were timed
again on 2026-10-25 for the threshold (below), with the same images; the
figures matched the first run to within 2 %.

The build figures here ran on this desktop, and they stand, as the JPEG
section's and the OpenAPI spike's did
([openapi-spike.md](openapi-spike.md) §4): Pipelines-as-Code builds
`master` only
([ADR 0022](adr/0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md)),
so a branch has no BuildRun to compare, and no cold build was timed on a
GitHub runner. Two figures are still to come, and are added here when they
exist:

1. the duration of the first `master` PAC BuildRun after this change,
   compared with the one before it, in the next docs change (ADR 0022 §1);
2. the CPU time and peak memory on the edge node, ADR 0015 §7's "CPU time
   on the node", from which k8s-backend3-deploy sets the threshold
   (ADR 0033 §5). The node's CPU is not known yet (ADR 0023), and the
   hub's build nodes have much slower cores than this desktop (research:
   gap-14 §6.2), so the times below are a lower bound.

The candidates were:

- **oxipng 10.2.1** with libdeflate, through `libdeflater` (Apache-2.0),
  which compiles the bundled C libdeflate (MIT) with the C compiler of the
  build stage. Presets 0 to 4 were measured; preset 2 is oxipng's default.
- **oxipng with Zopfli** (its `zopfli` feature, pure Rust), on the filters
  of preset 2.
- **The `png` crate's own encoder** (0.18.1, pure Rust) at
  `Compression::High` with adaptive filtering: decode, then write the same
  colour type and bit depth again. It is the pure-Rust path, with no C build
  step.

### Measurement images

They are not committed: the corpus images are too small to time, and the
large ones are 30 MB and more each. The commands below make them, with
ImageMagick 7.1.2-34 and the DejaVu Sans font, and FFmpeg 8.1.3 for the
animations; running them again gives the same files. They show no people.

| Image | Content | Input |
|---|---|---:|
| chart | 1200×800 line chart, 8-colour palette | 6,871 B |
| logo | 800×800 RGBA, translucent discs | 28,661 B |
| screenshot | 1920×1080 RGB, a timetable-like page with text | 104,872 B |
| photo | 3000×2000 RGB, a blurred plasma fractal (photo-like detail) | 4,159,285 B |
| camera-50mp | 8688×5792 RGB (50.3 MP): the same content at the size of a 50-megapixel camera, just above the pixel budget | 32,287,917 B |
| photo-budget | 8640×5760 RGB (49.8 MP), cut from camera-50mp to fit the budget | 31,935,714 B |
| rgba16-budget | 8640×5760 16-bit RGBA, a gradient at constant translucency: a small file that decodes to 398 MB | 420,890 B |
| noise-rgb | 6000×6000 RGB random noise: the largest file per pixel, near the 128 MiB upload limit | 108,071,441 B |
| noise-rgba16 | 4000×4000 16-bit RGBA random noise, at the 128 MiB upload limit | 128,079,897 B |
| photo16x8-budget | photo-budget as 16-bit RGBA at 50 % opacity: each sample's low byte repeats its high one, as in a 16-bit export of an 8-bit photograph | 44,918,152 B |
| photo16-limit | the plasma photograph at 16-bit RGBA, 50 % opacity and 11 significant bits per sample, just under the 128 MiB upload limit (with all 16 bits it is 222 MB) | 124,468,612 B |
| apng2-photo16, apng3-photo16 | APNGs of two and three different 16-bit photographs made like photo16x8-budget, each frame the whole canvas, the first the default image: 99.6 and 149.3 megapixels in all, the second just under the 128 MiB upload limit | 93,168,490 B and 120,627,704 B |
| apng5-rgba16 | an APNG of five 16-bit gradients like rgba16-budget at 50 % to 54 % opacity: 249 megapixels in all, just under the 256 Mi-pixel total of ADR 0033 §4 | 2,041,481 B |

```sh
# chart: a line chart with axes, quantised to an 8-colour palette.
magick -size 1200x800 xc:white -fill none -strokewidth 1 -stroke '#7f7f7f' \
  -draw 'line 100,100 100,700' -draw 'line 100,700 1100,700' \
  -strokewidth 4 -stroke '#1f77b4' \
  -draw 'polyline 100,640 250,520 400,580 550,300 700,360 850,200 1000,260 1100,150' \
  -stroke '#d62728' \
  -draw 'polyline 100,600 250,610 400,450 550,500 700,420 850,460 1000,380 1100,400' \
  -colors 8 PNG8:chart.png

# logo: three translucent discs on a transparent canvas.
magick -size 800x800 xc:none \
  -fill 'rgba(31,119,180,0.6)' -draw 'circle 300,300 300,80' \
  -fill 'rgba(214,39,40,0.6)' -draw 'circle 500,300 500,80' \
  -fill 'rgba(44,160,44,0.6)' -draw 'circle 400,480 400,260' \
  PNG32:logo.png

# screenshot: a timetable-like page, a grid of cells with subjects and rooms.
set -- Matematika Történelem Biológia Fizika Kémia Irodalom Angol Német Informatika
cells=''
for row in 0 1 2 3 4 5 6 7 8; do
  for col in 0 1 2 3 4 5; do
    x=$((120 + col * 290)); y=$((140 + row * 100))
    cells="$cells stroke #c8c8c8 fill white rectangle $x,$y $((x + 280)),$((y + 90)) stroke none"
    if [ $(((row + col) % 3)) -ne 0 ]; then
      eval subject=\${$(((row * 7 + col * 5) % 9 + 1))}
      cells="$cells fill #1f3864 text $((x + 16)),$((y + 40)) '$subject'"
      cells="$cells fill #7f7f7f text $((x + 16)),$((y + 72)) '$((100 + row * 10 + col)). terem'"
    fi
  done
done
magick -size 1920x1080 xc:'#f4f4f4' -font DejaVu-Sans -pointsize 22 \
  -fill '#1f3864' -draw 'rectangle 0,0 1919,90' \
  -fill white -pointsize 36 -draw 'text 120,60 "Órarend"' -pointsize 22 \
  -strokewidth 1 -draw "$cells" PNG24:screenshot.png

# photo: photo-like detail, a blurred plasma fractal.
magick -seed 14 -size 3000x2000 plasma:fractal -blur 0x2 PNG24:photo.png

# camera-50mp, and photo-budget cut from it.
magick -seed 14 -size 8688x5792 plasma:fractal -blur 0x2 PNG24:camera-50mp.png
magick camera-50mp.png -crop 8640x5760+0+0 +repage PNG24:photo-budget.png

# rgba16-budget: a 16-bit vertical gradient at constant 50 % opacity.
magick -size 8640x5760 gradient:black-white -alpha set -channel A \
  -evaluate set 50% +channel -depth 16 PNG64:rgba16-budget.png

# noise-rgb and noise-rgba16: random noise, which no filter or compressor
# shrinks, at the largest sizes the 128 MiB upload limit lets through.
magick -seed 7 -size 6000x6000 xc: +noise Random -depth 8 PNG24:noise-rgb.png
magick -seed 7 -size 4000x4000 xc: +noise Random -alpha set -channel A \
  +noise Random +channel -depth 16 PNG64:noise-rgba16.png

# photo16x8-budget and photo16-limit: 16-bit RGBA at the budget.
magick photo-budget.png -alpha set -channel A -evaluate set 50% +channel \
  -depth 16 PNG64:photo16x8-budget.png
magick -seed 14 -size 8640x5760 plasma:fractal -blur 0x2 -alpha set -channel A \
  -evaluate set 50% +channel -depth 11 -depth 16 PNG64:photo16-limit.png

# apng2-photo16 and apng3-photo16: full-canvas frames of three different
# 16-bit photographs. The three-frame file is written at FFmpeg's highest
# compression, which brings it under the upload limit.
for i in 1 2 3; do
  magick -seed $((13 + i)) -size 8640x5760 plasma:fractal -blur 0x2 -alpha set \
    -channel A -evaluate set 50% +channel -depth 8 -depth 16 PNG64:frame$i.png
done
ffmpeg -framerate 1 -i frame%d.png -frames:v 2 -plays 0 -f apng apng2-photo16.png
ffmpeg -framerate 1 -i frame%d.png -frames:v 3 -plays 0 -pred mixed \
  -compression_level 9 -f apng apng3-photo16.png

# apng5-rgba16: five full-canvas 16-bit gradients, each at another opacity.
for i in 1 2 3 4 5; do
  magick -size 8640x5760 gradient:black-white -alpha set -channel A \
    -evaluate set $((49 + i))% +channel -depth 16 PNG64:grad$i.png
done
ffmpeg -framerate 1 -i grad%d.png -frames:v 5 -plays 0 -f apng apng5-rgba16.png
```

### Results

Output bytes, with the share of the input, and CPU seconds of one
optimisation of the image as it is (without the chunk walk). Presets 0 to 2
of the photographs and the noise images are from the second run:

| Image | oxipng 0 | oxipng 1 | oxipng 2 | oxipng 3 | oxipng 4 | Zopfli | `png` crate |
|---|---:|---:|---:|---:|---:|---:|---:|
| chart | | 6,096 (89 %), 0.03 s | 5,639 (82 %), 0.08 s | 5,639 (82 %), 0.11 s | 5,087 (74 %), 0.18 s | 4,743 (69 %), 3.0 s | 8,217 (120 %), 0.00 s |
| logo | | 23,123 (81 %), 0.06 s | 22,913 (80 %), 0.12 s | 22,913 (80 %), 0.20 s | 21,179 (74 %), 0.33 s | 20,422 (71 %), 9.3 s | 27,971 (98 %), 0.01 s |
| screenshot | | 89,814 (86 %), 0.16 s | 82,404 (79 %), 0.46 s | 82,404 (79 %), 0.66 s | 79,756 (76 %), 1.2 s | 75,369 (72 %), 44 s | 99,895 (95 %), 0.04 s |
| photo | 4,050,310 (97 %), 0.62 s | 3,654,956 (88 %), 3.6 s | 3,649,171 (88 %), 6.7 s | 3,335,453 (80 %), 16.7 s | 3,334,899 (80 %), 16.3 s | 3,677,620 (88 %), 288 s | 4,177,896 (100.4 %), 5.6 s |
| camera-50mp | 31,997,355 (99 %), 5.2 s | 28,252,592 (88 %), 32.5 s | 28,117,581 (87 %), 59.0 s | 26,193,029 (81 %), 122.5 s | | | 32,306,225 (100.06 %), 44.8 s |
| photo-budget | 31,645,564 (99 %), 5.1 s | 27,943,524 (88 %), 32.1 s | 27,792,598 (87 %), 58.0 s | | | | |
| rgba16-budget | 219,827 (52 %), 3.1 s | 229,803 (55 %), 4.9 s | 219,874 (52 %), 7.4 s | | | | |
| noise-rgb | 108,015,299 (99.9 %), 3.0 s | 108,015,299 (99.9 %), 5.8 s | 108,015,299 (99.9 %), 8.4 s | | | | |
| noise-rgba16 | 128,014,964 (99.9 %), 3.5 s | 128,014,964 (99.9 %), 7.2 s | 128,014,964 (99.9 %), 10.2 s | | | | |

Peak resident memory of one optimisation with oxipng preset 2: 11-19 MiB
for chart, logo and screenshot, 56 MiB for photo, 441 MiB for camera-50mp
(the `png` crate: 238 MiB), 767 MiB for rgba16-budget and 520-615 MiB for
the noise images. Preset 0 peaks higher than preset 2 on large photographs
and noise (7 % on camera-50mp, 19 % on the noise images, at most 734 MiB),
and the same on rgba16-budget.

The whole optimiser, `optimise_png` (the chunk walk, then oxipng at preset
2), in a release build of backend3:

| Image | Output | CPU seconds | Peak memory |
|---|---:|---:|---:|
| chart | 5,470 B (80 %) | 0.08 | 11 MiB |
| logo | 22,726 B (79 %) | 0.12 | 12 MiB |
| screenshot | 82,217 B (78 %) | 0.45 | 19 MiB |
| photo | 3,648,984 B (88 %) | 6.6 | 60 MiB |
| photo-budget | 27,792,411 B (87 %) | 57.7 | 468 MiB |
| rgba16-budget | 219,705 B (52 %) | 7.4 | 767 MiB |

The outputs are a little smaller than oxipng's alone because the chunk walk
has removed the text, time and background chunks that ImageMagick writes.
The walk checks every CRC and inflates the image data once to find where
its stream ends, about a tenth of the time on the photo.

The whole optimiser at the budget and the upload limit, measured on
2026-10-25 with the same release build: at preset 0, which the default
effort uses above 6 megapixels, and at the presets the table names. The
peak is the resident memory of the whole process; it includes the upload,
which the caller holds while the optimiser runs, so the last column, the
peak above the upload, is what the optimisation itself adds:

| Image | Preset | Output | CPU seconds | Peak | Above the upload |
|---|---:|---:|---:|---:|---:|
| photo-budget | 0 | 31,645,377 B (99 %) | 5.6 | 497 MiB | 465 MiB |
| rgba16-budget | 0 | 219,658 B (52 %) | 3.3 | 762 MiB | 760 MiB |
| noise-rgba16 | 2 | 128,014,777 B (99.9 %) | 10.8 | 738 MiB | 613 MiB |
| photo16x8-budget | 0 | 44,901,525 B (100 %) | 10.4 | 1,076 MiB | 1,031 MiB |
| photo16x8-budget | 1 | 36,262,568 B (81 %) | 98.7 | 1,026 MiB | 981 MiB |
| photo16x8-budget | 2 | 36,178,525 B (81 %) | 151.8 | 1,026 MiB | 981 MiB |
| photo16-limit | 0 | 124,422,861 B (100 %) | 15.3 | 1,484 MiB | 1,363 MiB |
| photo16-limit | 1 | 117,661,540 B (95 %) | 74.0 | 1,358 MiB | 1,238 MiB |
| photo16-limit | 2 | 117,726,994 B (95 %) | 110.4 | 1,360 MiB | 1,239 MiB |
| apng2-photo16 | 0 | 92,850,901 B (99.7 %) | 15.7-17.0 | 1,457 MiB | 1,366 MiB |
| apng3-photo16 | 0 | 120,197,575 B (99.6 %) | 20.7 | 1,536 MiB | 1,419 MiB |
| apng5-rgba16 | 0 | 2,033,936 B (99.6 %) | 19.0-21.2 | 1,147 MiB | 1,143 MiB |

Memory follows the decoded size first: oxipng holds the decoded image and
filtered copies of it, eight bytes a pixel for 16-bit RGBA against three
for 8-bit RGB. The file adds to it as well, through the rewrite and
oxipng's compressed candidates, which is why photo16-limit, at 124 MB,
adds about 330 MiB more than photo16x8-budget, of the same size in
pixels. An
animation adds a frame's working copies to the decoded default image,
which oxipng keeps while it recompresses each frame in turn, so the peak
grows with the size of one frame and the length of the file, not with the
number of frames: five frames of rgba16-budget peak 380 MiB above the
still image, three 16-bit photographs only 50 MiB above two.

### At the pixel budget

- **Memory.** One optimisation of an image at the budget peaks at about
  470 MiB above the upload for an 8-bit RGB photograph and 760 MiB to
  1,370 MiB for 16-bit RGBA, the more the larger its file. The largest
  measured is an animation: apng3-photo16, three 16-bit photographs in
  120.6 MB, peaks 1,419 MiB above the upload and 1,536 MiB with it. These
  are the PNG worst case at the budget and the 128 MiB upload limit (the
  table above). ADR 0015 §7 runs one optimisation at a time per process.
  The maintainer's answer to D81 (research: DECISIONS R15) keeps a high
  memory limit from the measured worst case and a request well below it;
  these figures are the PNG part of that worst case. The preset changes
  them little: preset 0 peaks 5-10 % above preset 2 on the 16-bit
  photographs.
- **Time.** oxipng's time grows with the decoded size, so the widest
  pixels cost the most: at preset 2, about 3.05 CPU seconds per megapixel
  for photo16x8-budget, 2.2 for photo16-limit and 1.16 for an 8-bit RGB
  photograph; random noise, the largest file per pixel, costs less than
  0.65, because libdeflate gives up on it quickly. ADR 0015 §7 answers the
  upload only when the final bytes are in place, one optimisation at a
  time, and ADR 0023 §5 gives `/api/admin` a 120-second router timeout, so
  at preset 2 photo-budget's 58 CPU seconds, and photo16x8-budget's 152,
  would leave no room for the upload's transfer, a second upload waiting
  behind the first, or slower cores on the node.

### Choice of the preset and the default threshold

The maintainer's answer to D82 (research: DECISIONS R15, ADR 0033 §5) is
preset 2 below a pixel-count threshold and a cheaper preset, 1 or lower,
above it, with the threshold set on the node. The default, which holds
until that measurement, is derived from the figures above:

1. One upload's 120 seconds must hold its transfer and two optimisations:
   its own and the one it may wait for behind the semaphore. Leaving about
   20 seconds for the transfer, each optimisation gets about 50 seconds on
   the node.
2. The node's cores are not known; assumed twice as slow as this desktop,
   one optimisation gets about **25 CPU seconds here**.
3. Preset 2 reaches 25 seconds at about 8 megapixels of the costliest
   pixels measured, 16-bit RGBA (photo16x8-budget, 3.05 CPU seconds per
   megapixel), and at about 21 megapixels of an 8-bit RGB photograph.
   Preset 1 takes 32 to 99 seconds at the budget, above 25, so the cheaper
   preset is **0**, which takes at most about 21 seconds at the budget and
   the upload limit: 15.3 for photo16-limit and 19.0 to 21.2 for the
   249-megapixel apng5-rgba16.
4. The threshold is **6,000,000 pixels**, a margin below 8 megapixels for
   the noise of these figures and for content costlier than the measured
   photographs: preset 2 then takes at most about 18 seconds. It keeps
   preset 2 for screenshots up to 1440p (3.7 megapixels) and for
   photographs up to 6 megapixels.

A threshold derived from the 8-bit RGB photograph alone would be about 16
megapixels, but a 16-bit RGBA image of that size takes about 49 CPU
seconds at preset 2 here, twice the share of step 2.

The price is size above the threshold: preset 0 leaves a photograph at
99-100 % of its size, where preset 2 reaches 80-95 %, and graphics lose
part of their saving (at preset 0 the screenshot keeps 94 % of its size
instead of 78 %, the chart 92 % instead of 80 % and the logo 82 % instead
of 79 %); gradients and noise come out the same at both. On
the node, k8s-backend3-deploy times presets 0 to 2 on photo-budget,
photo16x8-budget and apng3-photo16, sets `PNG_PRESET_THRESHOLD_PIXELS`
(read by `config/production.yaml`) to the pixel count at which preset 2
takes about a quarter of the timeout there for the costliest of them, and
may raise the cheaper preset to 1 if its time at the budget fits the same
share. Preset 2 below the threshold is fixed by ADR 0033 §5.
`PngEffort::new` refuses a threshold of 0 and a cheaper preset above 1,
and the server stops at boot with the key and what to change.

An animated PNG counts the pixels of every kept frame, since the encoder
recompresses each one. At the animation totals of ADR 0033 §4, which the
maintainer kept for APNG at the WebP and GIF values (D102, research:
DECISIONS R16), apng5-rgba16, 249 megapixels in five frames, took 19.0 to
21.2 CPU seconds at preset 0, inside the 25 seconds of step 2; the 16-bit
photographs, the costliest pixels at preset 0 (0.31 CPU seconds a
megapixel for photo16-limit), fit the upload limit only three frames at a
time, 149 megapixels and 20.7 seconds. The node timing above, which
includes apng3-photo16, shows whether that holds on the node.

### The other candidates

- The `png` crate cannot reduce colour type, bit depth or palette, so it
  wrote files larger than the input for the chart and both photos. Under
  the "never larger" rule backend3 would then keep the input's image data,
  so the pure-Rust path optimises much less for the same CPU time. It saves
  about 6 CPU seconds of build.
- Zopfli is 9-16 % smaller than libdeflate on graphics but 40-100 times
  slower, and larger on the photo; inside an upload request that is too
  slow.
- Preset 3 equals preset 2 on these graphics, and preset 4 is 3-10 %
  smaller at 2.3-2.8 times the CPU time, still under two seconds. On the
  photo both are 9 % smaller at about 2.5 times the CPU time; at the budget
  that is 122.5 seconds on camera-50mp (7 % smaller), past the 120-second
  timeout even on this desktop. Preset 1 halves preset 2's time for 1-9 %
  larger graphics.
- The C build step is libdeflate's 11 C files, compiled by the `gcc` that
  the Containerfile's rust stage already installs, so the Containerfile
  does not change; a local `podman build` of it compiles libdeflate in the
  UBI 9 stage. libdeflate picks its SIMD code paths at run time from the
  CPU it runs on, so the binary keeps the default target (ADR 0022 §2).

Cold release build of the candidate alone (three runs for oxipng and `png`):

| Candidate | Crates compiled | CPU seconds |
|---|---:|---:|
| oxipng, default features off | 14 (oxipng, libdeflater, libdeflate-sys, rgb, bytemuck, indexmap and the build tools) | 10.4-10.9 |
| oxipng with Zopfli | 17 | 12.8 |
| `png` crate | 12 | 4.7-5.1 |

A cold release build of the whole of backend3 took 874 CPU seconds before
oxipng was added and 852 after it, while other builds loaded the machine:
the difference is inside the noise. Until the first `master` BuildRun
after this change, the isolated figure above, about 11 CPU seconds or
1.3 % of the whole build, is the estimate of the cost on the hub.

### What the optimiser does with a PNG

`optimise_png` first rewrites the upload from the parts that ADR 0015 §7
and ADR 0033 §6 keep, then lets oxipng re-encode that rewrite. For a still
image the rules follow what libpng 1.6.58 does with each chunk; libpng does
not decode animations, so the animation follows the PNG specification
(third edition) as the `png` crate, Chromium's PNG decoder, reads it. The
module documentation of `src/domain/image_optimise/png/chunks.rs` and
`png/apng.rs` lists the rules one by one, with the few places where the
walk is stricter.

- **Kept:** `IHDR`, `PLTE` (for indexed colour; cut to the entries the bit
  depth can index), the first valid `tRNS` (the transparency of the
  pixels; its samples masked to the bit depth, as PNG requires of
  decoders), the image data, and the first valid `sRGB`, `gAMA`, `cHRM`,
  `cICP`, `mDCV` and `cLLI` and the first `iCCP` before `PLTE` and the
  data, where PNG places them. The HDR chunks are kept byte for byte when
  libpng takes them: `cICP` with matrix coefficients 0 (PNG holds RGB
  only) and a full-range flag of 0 or 1, and `mDCV` and `cLLI` with light
  levels up to 2³¹ − 1. An `iCCP` is kept only if its profile passes
  libpng's checks: an ICC profile of its declared size, at most 8,000,000
  bytes, whose tags lie inside it, in the colour space of the colour type.
  The profile is kept byte for byte; oxipng may recompress its zlib
  stream. The first `eXIf` before the data is read as ADR 0033 §3 reads
  the EXIF Orientation and rewritten as the TIFF structure of that tag
  alone, 26 bytes or the 22-byte form for a shorter original, or dropped
  when it has no valid Orientation.
- **Animation:** an APNG keeps every frame with its `fcTL` up to the
  animation's first defect; each frame's data is checked to decode to
  exactly the frame's size, cut at the end of its zlib stream, written as
  one `fdAT` and recompressed by oxipng like `IDAT`, with the sequence
  numbers counted again. Every frame lies inside the canvas, which the
  pixel budget bounds, so no frame is larger than the budget allows.
- **A broken animation** (ADR 0033 §6, D103, "Keep the frames before the
  defect"): the file is first checked as a still PNG is, so a file cut
  short inside its animation, or whose default image does not decode, is
  refused as corrupt whatever its frames hold, except that an animation
  above the totals (below) is refused for its totals even when its
  default image does not decode, since they are counted first. Otherwise
  the first defect of the animation, in file order, decides what is kept:
  a sequence number
  that is not the next one, an `fcTL` of the wrong size or values, a
  frame outside the canvas, a misplaced `fdAT`, a frame without one,
  an animation chunk lost to its CRC, or frame data that is not one zlib
  stream of exactly the frame's rows with known filter types, among the
  rest the module lists. The frame it lies in is dropped with every later
  frame, and the frames before it are kept; one that lies in no frame,
  such as an `fdAT` right after the image data, keeps the frames that
  ended before it. At most as many frames as `acTL` announces are kept,
  and a frame past that number is no defect. The stored `acTL` announces
  the frames kept, with the upload's play count; when none is left, the
  animation chunks are dropped and the still default image is stored.
  Visitors see the frames that Chromium and Firefox both show before the
  damage. A chunk outside the animation is judged by the still-image
  rules wherever it stands, between the frames too, and never cuts the
  animation: an `IDAT` after the first run, or a `PLTE` of an image that
  is not indexed that is not 3 to 768 bytes long, follows another or does
  not match its CRC, is dropped, as libpng ignores it. The `png` crate
  stops at each of these, so on such a file the stored animation plays
  frames past the point where Chromium stops on the upload; they are the
  upload's own frames, and the stored file decodes in full.
- **The totals:** an animation whose kept frames, counted from the chunks
  alone, are more than 4096 or hold more than 256 Mi pixels with the
  default image is refused from its frame headers before any image data
  is inflated, the default image's included (only the first `iCCP`
  profile may have been inflated by then), as an animated WebP or GIF
  above those totals is (`ImageError::AnimationTooLarge`; D102). A frame
  whose data then turns out not to decode still counts. The walk checks the
  frame chunks as it reads them and copies a frame's data only while the
  frames up to it are within the totals, so a file of many frame chunks
  costs no more memory than its own size.
- **Dropped:** every other chunk, including text, time, physical size,
  background and significant-bit, and private or unknown ancillary
  chunks; a truecolour image's suggested palette, which still places the
  colour chunks where libpng takes it; colour, HDR, `tRNS` and `eXIf`
  chunks that are misplaced, repeated or malformed, which decoders ignore;
  an `iCCP` after the first,
  unread, so that an upload costs at most one profile of inflate work;
  image data after the first run of `IDAT`; everything after `IEND`; and
  the bytes after the end of the zlib stream of the image data, a frame or
  the profile.
- **Refused** as not decodable, as libpng refuses them: a cut-short file or
  one without `IEND`, a critical chunk with a wrong CRC or an unknown type,
  a second `IHDR` or a second palette of an indexed image, an indexed image
  without a valid palette before its data, an empty suggested palette
  where libpng would take one, and image data that is not one zlib stream
  (its Adler-32 included) of the declared size or has a row of an unknown
  filter type. Image data that decodes to more than the declared size is
  refused too, where libpng only warns: the extra decoded bytes are data no
  viewer shows. libpng only warns about a wrong CRC on `IEND` and on the
  palette of an image that is not indexed, and so does the walk.
- **The stored bytes** are oxipng's output whenever it is no larger than
  the upload. oxipng runs with `force`, so it always writes its own
  encoding rather than handing back the rewrite when it cannot beat the
  rewrite's image data. Only when its output is larger than the upload,
  because the upload was compressed harder than the preset compresses, is
  the rewrite stored: written chunk by chunk, with the upload's own image
  data and frame streams, checked to decode to exactly the image and cut
  where each stream ends. The rewrite is never larger than the upload, so
  neither is the result.
- **oxipng's settings:** `StripChunks::None`, so it keeps the rewrite's
  chunks (recompressing only the profile, whose name it then sets to
  `icc`, and the frames): other settings would let it replace an sRGB-like
  profile by `sRGB`, or drop `iCCP` when `sRGB` is also present, although
  PNG gives `iCCP` precedence. oxipng 10.2.1 keeps a 16-bit `tRNS` value
  when it reduces an image to 8 bits, so a transparent colour that matched
  no pixel would match some; when the kept `tRNS` of a 16-bit image has a
  sample whose two bytes differ, its bit-depth reduction is turned off.
  oxipng knows `iCCP` and `sRGB` but not `cICP`, so greyscale reduction is
  turned off when `cICP` is kept. With `acTL` present, oxipng itself turns
  off every reduction and interlacing change, since the frames share the
  format of `IHDR`.
- **Never changed:** the format, the width, the height, every decoded
  pixel and every kept frame with its timing, offset, disposal and
  blending.
  oxipng may change the colour type, bit depth, palette order, filters and
  interlacing (it removes Adam7) of a still image, which are encoding
  choices.

## WebP

ADR 0015 §7: "Metadata is removed. Lossy WebP is re-encoded at the same
quality target and kept only under the same at-least-5 % rule as JPEG.
Lossless WebP keeps its pixels and animation."
[ADR 0033](adr/0033-image-optimisation-ssim-floor-reference-parsing-cpu-limits-a.md)
amends it: a still lossy WebP's re-encode is kept only when it is also at a
luma SSIM of at least 0.98 (§1), an animated lossy WebP is stripped only
(§3), the chunks are read as libwebp reads them (§3), and an animation is
held to 4096 frames and 256 Mi pixels (§4). Only the lossy still image
needs an encoder; lossless and animated WebP are stripped and decoded, never
re-encoded (`src/domain/image_optimise/webp/mod.rs` lists which chunks are
kept). The maintainer accepted libwebp as a C build dependency for this.

### Method

- **Size and similarity.** Each input is re-encoded with the settings
  below. The size is compared with the at-least-5 % rule, and the
  similarity is the keep rule's own measure: the mean SSIM of the two luma
  (Y) planes that libwebp decodes, at full resolution, with
  `src/domain/image_optimise/ssim.rs` (11 × 11 Gaussian window, σ = 1.5),
  the function JPEG's measurement and the corpus tests use.
- **Images.** The photographs are the 10 CC0 photographs of the JPEG
  measurement above (2.7 to 14.0 megapixels, no people), saved by
  ImageMagick 7.1.2-34 (libwebp 1.6.0) as lossy WebP at quality 80, 90 and
  95, as cameras, phones and editing tools write them. The settings were
  chosen earlier on ImageMagick's built-in pictures (`logo:`, 640×480;
  `wizard:`, 480×640; `rose:` enlarged to 700×460) and on seeded plasma
  fractals, blurred like a photograph (3000×2000 and 6000×4000), each as
  lossy WebP at quality 75, 90 and 100. The memory and worst-case inputs
  are synthetic patterns of 7000×7000 pixels (49 megapixels, just under the
  default budget of 50) that libwebp and the `gif` crate wrote. None of
  them is committed.
- **CPU time.** Wall time of one call, release build, or the thread CPU
  time where a table says so, on a 24-thread AMD Ryzen 9 5900X workstation
  shared with other builds, so the figures are upper bounds for that
  machine. The edge node's cores are slower, so only the ratios between
  candidates and paths carry over.
- **Memory.** The peak resident set (`/usr/bin/time`, maximum RSS) of a
  process that reads one file and runs the optimiser on it, release build.
- **Build cost.** The research's method (research: gap-14 §6.2): a cold
  `cargo build --release --locked` from an empty `target/` on the same
  machine, timed with `/usr/bin/time`. CPU seconds (user + sys) approximate
  the build's time on one CPU and are compared with and without the
  dependency. `cargo build --timings` gives the share of each new crate.

### Candidates

| Candidate | Licence | Verdict |
|---|---|---|
| libwebp 1.6.0 through [`webpx`](https://crates.io/crates/webpx/0.4.1) 0.4.1 and `libwebp-sys` 0.14.4 | webpx `MIT OR Apache-2.0`; libwebp-sys `MIT`; the libwebp sources it bundles `BSD-3-Clause` | **Chosen.** libwebp is the reference encoder that `cwebp` and the browsers' decoders come from. webpx is a safe API over it, so backend3 keeps `unsafe_code = "forbid"`, and it exposes libwebp's decoder limits, scaled decoding, YUV input and output, and error reasons. |
| libwebp through [`webp`](https://crates.io/crates/webp/0.3.1) 0.3.1 and `libwebp-sys` 0.9.6 | `MIT OR Apache-2.0`; `MIT` | Not chosen, not measured. Its decoder has no limits and gives no error reason (`Option`), its encoder takes only RGB or RGBA, and its animation decoder holds every frame in memory at once. Its libwebp is a snapshot from after 1.3.1: its `vendor/` sources carry the CVE-2023-4863 fix (`HuffmanTablesSegment` in `src/utils/huffman_utils.h`), but none of the fixes of releases 1.4 to 1.6. |
| [`image-webp`](https://crates.io/crates/image-webp/0.2.4) 0.2.4, pure Rust | `MIT OR Apache-2.0` | Not usable: its encoder writes lossless WebP only. |
| [`zenwebp`](https://crates.io/crates/zenwebp/0.4.6) 0.4.6, a pure-Rust port of libwebp | `AGPL-3.0-only OR LicenseRef-Imazen-Commercial` | Not measured. `AGPL-3.0-only` is not on the `deny.toml` list, and adding a licence needs its own PR (ADR 0022 §4). |

### Settings

| Setting | Value | Why |
|---|---|---|
| Quality | 85 | ADR 0015 §7's "same quality target" as JPEG; libwebp's scale follows JPEG's |
| Method (effort) | 4, libwebp's default | Method 6 was 0-5 % smaller at 1.3-2.4 times the encode time (table below) |
| Input | the `VP8 ` bitstream decoded to YUV 4:2:0 planes | VP8 stores YUV 4:2:0, so the re-encode adds no colour conversion and holds 1.5 bytes per pixel instead of 4 |
| Keep rule | at least 5 % smaller, then a luma SSIM of at least 0.98 against the decoded original | ADR 0033 §1, the rule JPEG keeps (`reencode_is_kept`, `reencode_meets_ssim_floor`); the SSIM compares the Y plane of the original's decode with the Y plane of the re-encode's, and runs only for a re-encode that passed the size test |
| Alpha | not re-encoded: the `ALPH` chunk is kept as it is | it is lossless already and independent of the colour bitstream, so a transparent image keeps its alpha exactly |
| Threads | none | the decoders and the encoder run on the blocking thread of the upload (ADR 0015 §7) |
| Decode check | every pixel decoded, scaled down to one pixel | proves the image decodes without holding a decoded copy of it |
| Decoder limits | the pixel budget per image | webpx's other defaults are not backend3's rules: its 64 MiB input limit would refuse uploads that ADR 0015 §6 allows (128 MiB), and libwebp checks its 16383-pixel side limit itself |
| Animation limits | at most 4096 frames and 256 Mi pixels in all frames, counted while the frames are read and refused at the frame that passes one, before any frame is decoded or the stripped copy is written | the decode check costs CPU time per frame, which the per-canvas budget does not bound; webpx's defaults for untrusted input, which ADR 0033 §4 accepts for WebP, GIF and APNG (`MAX_ANIMATION_FRAMES`, `MAX_ANIMATION_PIXELS`, shared with the PNG optimiser) |
| Metadata | written by backend3 | the ICC profile and the Orientation-only EXIF go back into the re-encode (`VP8X`, `ICCP`, `EXIF`); the EXIF is read with the reader JPEG uses (ADR 0033 §3) |

An animation is checked frame by frame: each frame's image chunks are
decoded as a still image, as libwebp's own animation decoder does before it
composes them, and backend3 checks that every frame fits the canvas. Composing
the frames onto the canvas would only show that the decoded frames fit, and
would cost the canvas size for every frame, however small the frame.

### Results

The photographs, through `optimise_webp` as uploads, measured on 2026-10-25.
"Stored" is the bytes stored after the keep rule of ADR 0033 §1, summed over
the set and divided by the input's bytes (the inputs carry no metadata, so
they are what the strip leaves); "size rule alone" is what the 5 % rule
without the SSIM floor would have stored.

| Set | Re-encodes at least 5 % smaller | Of those, below SSIM 0.98 | Stored | Size rule alone | SSIM of the re-encodes | Time per file |
|---|---:|---:|---:|---:|---|---:|
| quality 80 | 0 of 10 (0.976 to 1.069 of the input) | — | 1.000 | 1.000 | 0.988 to 0.998 | 0.12 to 1.0 s |
| quality 90 | 10 of 10 (0.667 to 0.863) | 1 (0.979) | 0.806 | 0.792 | 0.979 to 0.995 | 0.22 to 1.5 s |
| quality 95 | 10 of 10 (0.327 to 0.663) | 5 (0.958 to 0.979) | 0.786 | 0.490 | 0.958 to 0.991 | 0.24 to 1.8 s |

The photographs below the floor at quality 95 are the textured ones the
JPEG measurement found: foliage, fur, water and a night scene. The settings
were chosen on the earlier inputs, by size and time; the keep rule did not
yet have its SSIM half, so their similarity is not repeated here (their
lowest SSIM, on an older 8 × 8 measure, was 0.984):

| Input | Pixels | Input bytes | Method 4 bytes | Ratio | Decode (ms) | Encode, method 4 (ms) | Method 6 bytes | Encode, method 6 (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| logo, q75 | 0.3 MP | 17,816 | 19,988 | 1.122 | 1.6 | 18.7 | 19,840 | 34.6 |
| wizard, q75 | 0.3 MP | 22,054 | 25,416 | 1.152 | 2.0 | 19.7 | 25,026 | 48.4 |
| rose, q75 | 0.3 MP | 8,540 | 11,280 | 1.321 | 1.5 | 27.6 | 11,308 | 42.8 |
| plasma, q75 | 6 MP | 131,984 | 165,044 | 1.250 | 40.1 | 362 | 164,370 | 470 |
| logo, q90 | 0.3 MP | 27,652 | 24,056 | 0.870 | 2.1 | 19.4 | 23,386 | 36.1 |
| wizard, q90 | 0.3 MP | 42,186 | 35,500 | 0.842 | 5.2 | 22.9 | 33,850 | 55.6 |
| rose, q90 | 0.3 MP | 14,942 | 12,736 | 0.852 | 1.9 | 22.5 | 12,982 | 32.5 |
| plasma, q90 | 6 MP | 352,850 | 253,422 | 0.718 | 33.1 | 374 | 250,478 | 662 |
| plasma, q90 | 24 MP | 1,156,430 | 753,964 | 0.652 | 182 | 1,112 | 749,654 | 1,528 |
| logo, q100 | 0.3 MP | 44,766 | 22,882 | 0.511 | 2.8 | 18.3 | 22,454 | 33.5 |
| wizard, q100 | 0.3 MP | 78,174 | 32,650 | 0.418 | 4.8 | 21.1 | 31,250 | 50.2 |
| rose, q100 | 0.3 MP | 33,026 | 12,230 | 0.370 | 2.6 | 22.4 | 12,360 | 35.1 |
| plasma, q100 | 6 MP | 837,128 | 233,322 | 0.279 | 63.4 | 434 | 229,006 | 621 |

The transparent path re-encodes the same `VP8 ` bitstream and keeps the
`ALPH` chunk, so only the colour changes. SSIM is the keep rule's:

| Input | Pixels | Input bytes | Stored bytes | Ratio | SSIM | Stored | Alpha |
|---|---:|---:|---:|---:|---:|---|---|
| corpus `lossy-metadata.webp` (opaque, q95) | 2,747 | 3,398 | 670 | 0.197 | 0.9904 | re-encode | — |
| corpus `lossy-alpha-q95.webp` (60 % alpha, q95) | 2,747 | 688 | 514 | 0.747 | 0.9904 | re-encode | identical |
| corpus `lossy-textured.webp` (grain, q95) | 2,747 | 3,180 | 3,180 | 1.000 | 0.9606 | stripped original | — |
| synthetic pattern, opaque, q95 | 49 MP | 5,593,848 | 4,925,494 | 0.881 | 0.9904 | re-encode | — |
| synthetic pattern, alpha gradient, q95 | 49 MP | 5,605,726 | 4,950,308 | 0.883 | 0.9910 | re-encode | identical |

What this means:

- A WebP saved at quality 75 to 80, the default of most tools, does not get
  5 % smaller at quality 85, so it is stored stripped, as ADR 0015 §7 wants.
  The re-encode pays off for WebP saved at quality 90 and above.
- The SSIM floor costs little at quality 90, where 9 of 10 photographs keep
  their saving, and half the saving at quality 95, where the textured half
  keeps its stripped original: 0.786 of the input stored instead of 0.490.
  Every stored re-encode reaches 0.98, as ADR 0033 §1 asks; the corpus
  tests hold the corpus's two re-encoded files, one opaque and one
  transparent, to it, check that the alpha channel does not change, and
  check that `lossy-textured.webp` keeps its stripped original.
- libwebp can fail to encode a picture that decodes: generating the 49 MP
  inputs at quality 100 and method 0 failed with `Partition0Overflow`. The
  shipped settings did not fail on any input, and a failure would store the
  stripped original with the reason for the log
  (`Optimised::re_encode_failure`, `Treatment::ReEncodeDiscarded`).

### The SSIM pass

A re-encode that passes the size test is decoded once more, to YUV, and its
Y plane goes into the streaming SSIM row by row with the original's, which
the re-encode kept from its own decode (1 byte per pixel; the chroma planes
are freed after the encode). Measured at 49 MP on the synthetic inputs:

| Input | Re-encode | SSIM | Luma decode, CPU s | SSIM arithmetic, CPU s |
|---|---:|---:|---:|---:|
| opaque pattern, q95 | 0.881 of the input | 0.9904 | 0.34 | 1.08 |
| transparent pattern, q95 | 0.880 | 0.9910 | 0.35 | 1.07 |
| grey gradient with grain of ±8 levels, q95 | 0.436 | 0.9218 | 0.40 | 1.07 |

- **CPU.** The pass costs about 1.4 to 1.5 CPU seconds at 49 MP, as JPEG's
  does: about 1.1 s of SSIM arithmetic, whatever the content, and the rest
  for the decode of the re-encode.
- **Memory.** While it runs, the optimiser holds the original's luma (1 byte
  per pixel), the re-encode's YUV planes (1.5 bytes per pixel), the SSIM's
  11 rows of sums (about 3.5 MiB at 7,000 columns), the stripped file and the
  re-encode. For the opaque path that is the new peak, 214 MB at 49 MP
  instead of the 182 MB of the encoder; for the transparent path the decode
  check's alpha plane stays the peak (next table).

### Memory and CPU time per path

Peak resident memory and time of one optimisation at 49 MP, the most the
default budget admits, release build. The peak includes the input file and
the stripped copy that the optimiser writes (both a few MB here, except
where the table says); a process that only reads the file peaks at 5 to
30 MB, and at 133 MB for the crafted 128 MiB uploads. Measured on
2026-10-25; the animation and GIF rows again on 2026-10-26, after the limits
moved into the walk over the chunks and blocks.

| Path | Input | Peak memory | Bytes per pixel | Wall time |
|---|---|---:|---:|---:|
| Lossless still: decode check | `VP8L`, 3.1 MB | 200 MB | 4.1 | 0.29 s |
| Lossy opaque still: check, re-encode and SSIM, re-encode kept | `VP8 ` q95, 5.6 MB | 214 MB | 4.4 | 4.8 s |
| Lossy transparent still: the same, re-encode kept | `ALPH` + `VP8 ` q95, 5.6 MB | 254 MB | 5.2 | 4.9 s |
| Lossy still below the floor: the same, stripped original kept | `VP8 ` q95 with grain, 14.0 MB | 239 MB | 4.9 | 6.0 s |
| Animation at the pixel limit: 5 frames of 49 MP | `VP8L` frames, 15 MB | 213 MB | 4.3 | 1.6 s |
| Animation at both limits: 4096 frames of 256×256 | `VP8L` frames, 28 MB | 59 MB | — | 1.2 s |
| Animation past the pixel limit: 6 frames of 49 MP | 18 MB, refused at the 6th frame, before decoding | 20 MB | — | 0.01 s |
| Animation of 4096 frames of 1×1 on a 49 MP canvas | `VP8L` frames, 0.2 MB | 4 MB | — | 0.03 s |
| Crafted animation: 3,532,044 frames of 1×1 in a 128 MiB upload | header-only `VP8L` frames, refused at frame 4097 | 134 MB | — | 0.12 s |
| GIF at the pixel limit: 5 frames of 49 MP | 42.6 MB | 134 MB | — | 0.5 s |
| GIF at both limits: 4096 frames of 256×256 | 52.6 MB | 105 MB | — | 0.5 s |
| GIF past the pixel limit: 6 frames of 49 MP | 51.1 MB, refused at the 6th descriptor, before decoding | 94 MB | — | 0.05 s |
| GIF past the frame limit: 4097 frames of 1×1 | 0.1 MB, refused before decoding | 3 MB | — | 0.00 s |
| Crafted GIF: 8,947,847 frames of 1×1 in a 128 MiB upload | refused at frame 4097 | 134 MB | — | 0.14 s |

- The lossless check holds libwebp's own copy of the pixels (4 bytes per
  pixel); a lossy check holds only row buffers, plus 1 byte per pixel for an
  alpha plane, and libwebp decodes a compressed alpha plane through a 4-byte
  buffer, which is why the transparent path is the highest. The re-encode
  holds the YUV planes, a copy of them in the encoder's picture, and the
  encoder's own state; the SSIM pass after it is described above.
- An animation peaks at its largest frame, never at the canvas times the
  frame count. Composing every frame onto the canvas, as libwebp's animation
  decoder does, held two RGBA canvases and a copy of each frame (about 12
  bytes per pixel, 576 MB at 49 MP) and cost the canvas for every frame: an
  animation of 1,000 frames of 1×1 on a 49 MP canvas took 188 s that way,
  and takes 7 ms frame by frame.
- On top of these, the caller holds the upload, and the optimiser writes a
  stripped copy of at most the same size; a lossy re-encode adds a smaller
  file. A 128 MiB upload therefore adds up to about 256 MiB to the peaks
  above. backend3's memory limit in the deployment is sized from these
  figures, not from a decoded RGBA image alone.
- **A crafted animation** costs no more than its upload. The walk over a
  WebP's chunks and a GIF's blocks counts the frames and their pixels as it
  goes and stops at the frame that passes a limit, before the frame is kept
  and before the stripped copy is written; the frames of a WebP animation
  are decoded before it too. Before, the whole frame list and the stripped
  copy were built first: the crafted WebP above peaked at 484 MB, 350 MB
  above its input, and the crafted GIF at 264 MB.
- **CPU.** Within the limits of ADR 0033 §4 the decode check of an
  animation takes at most about 1.5 s here, and a GIF's about 0.5 s: its
  check costs CPU time in proportion to the decoded pixels of all its
  frames, about 2,000 megapixels per second here, and the limit holds those
  to 256 Mi. Before the limit, a GIF was bounded only by the upload size: a
  12-bit LZW code stands for at most about 4,096 pixels, so up to about
  2,700 pixels per byte, and 200 frames of 49 MP in 6.7 MB took 4.9 s. A
  still lossy WebP costs the most, about 5 to 6 s at 49 MP, a quarter of it
  the SSIM pass.

### Build cost

| Build | Compiled crates | CPU seconds (user + sys) | Wall time (24 threads, shared) |
|---|---:|---:|---:|
| Base (the image header slice, before the JPEG slice) | 440 | 811.4 | 77 s |
| With webpx and gif | 451 | 850.8 | 83 s |
| Difference | +11 | **+39.4 (+4.9 %)** | +6 s |

The 11 new crates are webpx, libwebp-sys, gif, weezl, rgb, imgref, bytemuck,
bytemuck_derive, zenpixels, whereat and enough; the build-script crates that
libwebp-sys uses (cc, glob, pkg-config) were already in the graph. Most of the
cost is the libwebp-sys build script, which compiles libwebp's C sources with
`cc`: 12.3 s of the new crates' 23.1 s of wall time per unit, spread over
several cores. Each figure is one cold build under a shared load, so a few
percent of noise is expected. The build was measured with webpx's
`animation` feature on; only the tests use it now, with `icc`, both Rust
code over the same libwebp, so the release build compiles slightly less of
webpx and none less of libwebp. The JPEG slice came after this measurement;
on its base, rgb and bytemuck are already built for mozjpeg, so these
crates add slightly less than the table shows.

The Containerfile needs no change: its rust stage already installs gcc as
the linker and C compiler for build scripts (ADR 0022 §2), and libwebp-sys
needs neither bindgen nor clang. Its build script passes
`-march=<target arch>`, which gcc does not accept for `x86_64` and drops, and
it enables no instruction set above SSE2 unless the target has it, so the
image keeps the default `x86_64-unknown-linux-gnu` target that the hub's
CPUs and the edge node run (ADR 0022 §2). libwebp is linked statically, so
the `ubi-micro` runtime image needs no shared library for it: a local
`podman build` of the Containerfile succeeded, and its binary links only
`libgcc_s`, `libm` and `libc`. Until the upload code calls the optimisers,
the linker leaves libwebp out of the binary; the compile cost above is paid
from the start.

On the hub, the first PAC BuildRun took 10m52s (Verseghy/website_backend3#196).
A 4.9 % increase in CPU time is about half a minute there. These local
figures stand, as the JPEG and PNG sections' and the OpenAPI spike's do
([openapi-spike.md](openapi-spike.md) §4): Pipelines-as-Code builds
`master` only
([ADR 0022](adr/0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md)),
so a branch has no BuildRun to compare, and no cold build is timed on a
GitHub runner before the merge. The duration of the first `master` PAC
BuildRun after this change, compared with the one before it, is added here
in the next docs change.

### Licences

`cargo deny check bans licenses sources` passes with the new crates.
cargo-deny reads only the licence each crate declares, and libwebp-sys
declares `MIT` for its Rust bindings while it also compiles the libwebp
sources it bundles, which are `BSD-3-Clause` (its `vendor/COPYING`) with
Google's additional patent grant (`vendor/PATENTS`, which licenses the
Google patent claims that the WebM implementations necessarily infringe and
ends that licence for anyone who sues over them). `deny.toml` therefore
clarifies libwebp-sys as `MIT AND BSD-3-Clause`, tied to the hash of
`vendor/COPYING`; both licences are on the list. cargo-deny ignores the
clarification without a warning when the file changes, so an update of
libwebp-sys re-reads both files and renews the hash.

### Security

libwebp is C code that parses untrusted uploads. Before any of it runs,
`inspect` refuses a file above the pixel budget from its header, the strip
checks the chunk structure as libwebp reads it (next section), every
bitstream header that it keeps and every frame's place on the canvas, and an
animation is held to its frame and pixel limits while its frames are read,
so a crafted file of millions of frames is refused at frame 4097 and costs
no memory beyond the upload (the table above). libwebp's decoders then see
only the stored file, or one frame's chunks at a time, never the upload's
own layout. The decoders are held to the same budget. The
libwebp version moves with libwebp-sys, which webpx requires as `^0.14.2`,
so a webpx release that Dependabot proposes or a lock file update picks a
new one up; the `cargo-deny check advisories` job watches the whole graph
(ADR 0022 §3).

### Reading the chunks as libwebp does

ADR 0033 §3 follows the reference decoders where ADR 0015 §7 is silent, as
the header check of Verseghy/website_backend3#211 does. libwebp 1.6.0 has two
readers of the chunks after `VP8X`, and they disagree on some layouts:

- **its decoder**, `WebPDecode` (`ParseHeadersInternal` and
  `ParseOptionalChunks` in `src/dec/webp_dec.c`), which decodes a still
  image, and which #211's header check and the strip's decode check follow.
  It refuses every animation;
- **its demuxer** (`ParseVP8XChunks`, `ParseAnimationFrame`, `StoreFrame`
  and `IsValidExtendedFormat` in `src/demux/demux.c`), through which its
  animation decoder reads an animation and `WebPDemuxGetChunk` reads the
  profile and the EXIF. Browsers run it before they decode: Firefox refuses
  what it refuses and then decodes a still image's whole file with the
  decoder (`mozilla-central@19272f1f7a2c:image/decoders/nsWebPDecoder.cpp`).

So that every WebP that libwebp decodes passes, the strip reads a still
image as the decoder reads it, an animation as the demuxer reads it, and the
profile and the EXIF as the demuxer reads them, as far as it reads the file.
The tests compare each layout with both readers through webpx (`decode_rgba`
for the decoder; `AnimationDecoder`, `get_icc_profile` and `get_exif` for the
demuxer), and a proptest checks that a still image with random chunks around
it, a second image among them, passes exactly when the decoder decodes it,
with the decoder's pixels, in a file that the demuxer reads too.

A still image, a `VP8X` file without the animation flag:

| Layout | libwebp's decoder | libwebp's demuxer | The strip |
|---|---|---|---|
| an `ALPH` chunk before the bitstream, without the alpha flag | reads it: the image shows its alpha | drops it: the image is opaque | keeps it without the flag, so each reader shows the stored file as it showed the upload |
| an `ALPH` chunk after the bitstream, without the flag | ignores it | ignores it | drops it |
| an `ALPH` chunk after the bitstream, with the flag | ignores it: the image is opaque | refuses the file | drops it: the image is opaque |
| two `ALPH` chunks before the bitstream | reads the last | refuses the file | keeps the last |
| an `ALPH` chunk with a `VP8L` bitstream, which has its own alpha | ignores it | refuses the file | drops it |
| a second image | reads the first | refuses the file | keeps the first |
| a second `VP8X` chunk, an `ANIM` chunk or an `ANMF` chunk before the image | skips them | refuses the file | drops them |
| an `ANIM` and an `ANMF` chunk after the image | ignores them | shows no frame of them, reads a profile or EXIF after the frame's image as the file's | drops them, keeps that metadata if it is the first flagged |
| reserved `VP8X` flags | ignores them | refuses the file | accepts it and writes them clear |
| fewer than 8 bytes, or a chunk cut short, after the image | ignores them | refuses the file | drops them |
| a profile or EXIF after the point where the demuxer refuses the file | does not read metadata | reads no further | drops it |
| a chunk cut short before the image, no image, a malformed bitstream header, or an image of another size than the canvas | refuses the file | refuses the file | refuses it |
| a simple file whose bitstream ends the RIFF size without the pad byte its odd size needs | reads it | refuses the file | stores it as uploaded, without the byte, which would make the output larger than the upload (as a GIF without its trailer) |
| the same after a `VP8X` chunk | refuses the file (it counts the pad byte) | refuses the file | refuses it |

An animation, which libwebp's decoder refuses whatever its layout:

| Layout | libwebp's demuxer | The strip |
|---|---|---|
| `ANIM` of 5 bytes and its pad byte | reads 6 bytes from the padded payload | accepts it; the pad byte is the high byte of the loop count, kept as libwebp reads it |
| `ANIM` of a padded size below 6 | refuses the file | refuses it |
| a profile or EXIF after a frame's image, inside its `ANMF` chunk | reads it as the file's, in file order | keeps it if it is the first flagged one |
| an `ANMF` chunk after a frame's image, inside its `ANMF` chunk | reads it as the next frame | keeps it as the next frame |
| an image chunk after a frame's image, or outside every frame | refuses the file | refuses it |
| an `ANMF` chunk with no image chunk | no frame | no frame |
| no frame that holds an image: only an `ANIM` chunk, or only `ANMF` chunks without an image chunk | refuses the file | refuses it (`WebpNoFrame`), as it refuses a GIF with no image (ADR 0033 §3, D104) |
| an `ANMF` chunk with `ALPH` and no bitstream, `ALPH` after its bitstream, or `ALPH` with a `VP8L` bitstream | refuses the file | refuses it |
| an `ANMF` header of 2³² pixels or more | refuses the file | refuses it |
| a frame's image that runs past the end of its `ANMF` chunk | refuses the file | refuses it |
| reserved `VP8X` flags | refuses the file | refuses it |
| more than 4096 frames or 256 Mi pixels in all frames | reads it (webpx's animation decoder refuses it by its limits) | refuses it at the frame that passes a limit (ADR 0033 §4) |

## GIF

ADR 0015 §7: "Metadata is removed. Pixels and animation are not touched."
GIF needs no encoder: the stored file is the parser-based strip of
`src/domain/image_optimise/gif.rs`, which keeps the frames byte for byte.
To refuse a GIF that does not decode, every frame of the stripped file is
decoded with the [`gif`](https://crates.io/crates/gif/0.14.2) crate 0.14.2
(`MIT OR Apache-2.0`, pure Rust, from the image-rs project), to one byte per
pixel and with the pixel budget as its memory limit per frame. Its build cost
is part of the table above (gif and its LZW decoder, weezl, about 3 s of wall
time per unit together), and its memory and CPU time are in the table of
paths.

ADR 0033 amends the GIF row twice:

- **The `gif` crate is the reference** (§3), checked in its 0.14.2 source
  (`src/reader/decoder.rs` and `src/reader/mod.rs`), with a test per layout
  that compares the strip with the `gif` crate, and a proptest that the
  `gif` crate reads the same frames, delays and loop count from the upload
  and the stored file:

  | Layout | The `gif` crate | The strip |
  |---|---|---|
  | a GIF that ends after a whole block, without its trailer | decodes every frame, then reports the end of the input (`UnexpectedEof`) | accepts it and stores it without a trailer, as uploaded; the decode check reads it with one appended |
  | a GIF that ends inside a block | refuses it | refuses it |
  | a GIF with no image, with or without its trailer | accepts some, with a block before the trailer, and refuses others, by a look one byte past the logical screen | refuses every one as corrupt, ADR 0033 §3's one exception (decision D104): no viewer shows an image of it |
  | a graphic control extension whose last data sub-block holds 4 bytes, after others | reads that last sub-block | stores it as that sub-block alone |
  | a graphic control extension whose last sub-block holds another length, or none | refuses it | refuses it |
  | a `NETSCAPE2.0` loop extension | reads the loop count from the first sub-block after the identifier, and only when it is 3 bytes of id 1 | keeps that first sub-block alone; drops an extension whose first sub-block is anything else, which it reads as no loop count |
  | an `ANIMEXTS1.0` loop extension | skips it | the same rule as `NETSCAPE2.0`, since browsers read it |
  | a plain text extension | skips it | drops it; a graphic control extension before it stays, so the timing does not change |
  | an extension of an unknown label | refuses it by default | drops it and accepts the file, as browsers skip it |

  Browsers read the loop count as the `gif` crate does, except for one
  layout. Chromium's GIF decoder, Wuffs, reads only the first sub-block, of
  3 bytes and id 1, of either identifier
  (`wuffs@4be865da:std/gif/decode_gif.wuffs`). Firefox reads either
  identifier too, but takes a loop sub-block wherever it is among the
  sub-blocks (`mozilla-central@19272f1f7a2c:image/decoders/nsGIFDecoder2.cpp`),
  so an upload whose loop sub-block follows Netscape's buffering one loops
  in Firefox, and its stored file, like the upload in Chromium and the `gif`
  crate, plays once.
- **The same totals as WebP** (§4): at most 4096 frames and 256 Mi pixels,
  counted from the image descriptors while the blocks are walked, and
  refused with the error of WebP and APNG (`ImageError::AnimationTooLarge`)
  at the descriptor that passes a limit, before anything is decoded and
  before the rest of the stripped copy is written.
  At the limits the check takes about 0.5 s at 49 MP (the table of paths).
