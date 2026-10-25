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
