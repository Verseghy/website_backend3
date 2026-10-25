//! The structural similarity index (SSIM) of two greyscale images, the
//! measure of the keep rule of a lossy re-encode (ADR 0015 §7, amended by
//! ADR 0033 §1).
//!
//! ADR 0015 §7 calls quality 85 visually lossless and asks that every
//! re-encoded JPEG and lossy WebP reach an SSIM of at least 0.98 on the luma
//! channel at full resolution, compared with the stripped original. Half of
//! the measured photographs did not (`docs/design/image-encoders.md`), so
//! ADR 0033 made that measure part of the keep rule: a re-encode below the
//! floor is discarded and the stripped original stored. The upload pipeline
//! and the corpus tests therefore call this one implementation, and what the
//! tests check is exactly what an upload is held to.
//!
//! This is the SSIM of Wang, Bovik, Sheikh and Simoncelli (IEEE Transactions
//! on Image Processing 13(4), 2004): an 11 × 11 Gaussian window with
//! σ = 1.5, K1 = 0.01, K2 = 0.03 and L = 255, averaged over every position
//! where the window fits inside the image. "At full resolution" means no
//! downsampling first: the authors' reference code shrinks large images
//! before it compares them, which would hide the fine detail a re-encode
//! loses.
//!
//! **Streaming.** The window is separable, so each image row is blurred
//! across once, and the last 11 blurred rows are blurred down to give one row
//! of window positions. [`LumaSsim`] takes the rows one at a time and keeps
//! only those 11 rows of five sums (a, b, a², b², ab), never the image:
//! about 520 bytes per column (11 rows × 5 sums × 8 bytes, plus three
//! one-row buffers). At the widest JPEG, 65,535 pixels, that is about
//! 33 MiB; at the 8,660 columns of a 50-megapixel photograph, about 4.3 MiB.
//! A caller holds the rows it compares and passes them in.
//!
//! **Determinism.** The arithmetic is `f64` in a fixed order (each sum from
//! the first tap of the window to the last, the positions row by row), so
//! the same two images always give the same value, bit for bit, whichever
//! caller computes it. [`mean_ssim`] over two whole planes is the same
//! computation.

use super::Dimensions;

/// The side of the Gaussian window, in pixels. An image needs at least this
/// many rows and columns to be compared.
pub const SSIM_WINDOW: u32 = 11;

/// The window side as an index range length.
const WINDOW: usize = 11;

/// The standard deviation of the Gaussian window.
const SIGMA: f64 = 1.5;

/// `(K1 × L)²` for 8-bit samples: K1 = 0.01, L = 255.
const C1: f64 = (0.01 * 255.0) * (0.01 * 255.0);

/// `(K2 × L)²` for 8-bit samples: K2 = 0.03, L = 255.
const C2: f64 = (0.03 * 255.0) * (0.03 * 255.0);

/// The number of window sums kept per position: a, b, a², b² and ab.
const SUMS: usize = 5;

/// Why two images cannot be compared. The messages are English, for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SsimError {
    /// A side is below [`SSIM_WINDOW`], so the window fits nowhere.
    #[error("SSIM needs at least {SSIM_WINDOW}x{SSIM_WINDOW} pixels, got {width}x{height}")]
    TooSmall {
        /// The width of the images.
        width: u32,
        /// The height of the images.
        height: u32,
    },
    /// A row of one of the two images does not hold one sample per column.
    #[error("a row holds {found} samples, not {width}")]
    RowLength {
        /// The width of the images.
        width: u32,
        /// The number of samples the row holds.
        found: usize,
    },
    /// More rows were given than the images have.
    #[error("more than {height} rows were given")]
    TooManyRows {
        /// The height of the images.
        height: u32,
    },
    /// The result was asked for before every row was given.
    #[error("{given} of {height} rows were given")]
    MissingRows {
        /// The height of the images.
        height: u32,
        /// The number of rows given.
        given: u32,
    },
    /// A whole plane passed to [`mean_ssim`] does not hold
    /// `width × height` samples.
    #[error("a plane holds {found} samples, not {expected}")]
    PlaneLength {
        /// `width × height`.
        expected: u64,
        /// The number of samples the plane holds.
        found: usize,
    },
}

/// The normalised 1-D Gaussian weights; the 2-D window is their outer
/// product, so the image is blurred in two passes.
fn weights() -> [f64; WINDOW] {
    let centre = 5.0;
    let mut weights = [0.0; WINDOW];
    for (offset, weight) in (0_u8..).zip(weights.iter_mut()) {
        let distance = f64::from(offset) - centre;
        *weight = (-(distance * distance) / (2.0 * SIGMA * SIGMA)).exp();
    }
    let sum: f64 = weights.iter().sum();
    weights.map(|weight| weight / sum)
}

/// A side of the images as an index bound. Every side backend3 compares is
/// at most a few tens of thousands of pixels, far below `usize::MAX` on the
/// 64-bit platforms it runs on.
fn index(side: u32) -> usize {
    usize::try_from(side).unwrap_or(usize::MAX)
}

/// The mean luma SSIM of two greyscale images of the same size, computed
/// row by row (see the module documentation).
///
/// Build it with [`LumaSsim::new`], give it every row of both images in
/// order with [`LumaSsim::push_row`], then read the result with
/// [`LumaSsim::finish`].
#[derive(Debug, Clone)]
pub struct LumaSsim {
    /// The width of both images.
    width: u32,
    /// The height of both images.
    height: u32,
    /// The number of window positions across a row: `width - 10`.
    columns: usize,
    /// The rows given so far.
    given: u32,
    /// The Gaussian weights of one direction.
    weights: [f64; WINDOW],
    /// The five per-pixel values of the row being blurred: a, b, a², b²
    /// and ab, one `width`-long block each.
    values: Vec<f64>,
    /// The horizontally blurred sums of the last 11 rows: 11 slots, each
    /// holding the five sums in `columns`-long blocks. The row `y` lives in
    /// slot `y % 11`.
    across: Vec<f64>,
    /// The vertically blurred sums of one row of window positions, in
    /// `columns`-long blocks.
    window: Vec<f64>,
    /// The sum of the SSIM of every window position so far.
    total: f64,
}

impl LumaSsim {
    /// A comparison of two images of `dimensions`, before any row is given.
    ///
    /// - `dimensions`: the stored size of both images.
    ///
    /// # Errors
    ///
    /// [`SsimError::TooSmall`] when the width or the height is below
    /// [`SSIM_WINDOW`].
    pub fn new(dimensions: Dimensions) -> Result<Self, SsimError> {
        let (width, height) = (dimensions.width(), dimensions.height());
        if width < SSIM_WINDOW || height < SSIM_WINDOW {
            return Err(SsimError::TooSmall { width, height });
        }
        let columns = index(width - SSIM_WINDOW + 1);
        Ok(Self {
            width,
            height,
            columns,
            given: 0,
            weights: weights(),
            values: vec![0.0; SUMS * index(width)],
            across: vec![0.0; WINDOW * SUMS * columns],
            window: vec![0.0; SUMS * columns],
            total: 0.0,
        })
    }

    /// Gives the next row of both images: the first call takes row 0.
    ///
    /// - `a`, `b`: the row of each image, one 8-bit sample per column. The
    ///   measure is symmetric, so which image is which does not matter.
    ///
    /// # Errors
    ///
    /// [`SsimError::RowLength`] when a row does not hold one sample per
    /// column, and [`SsimError::TooManyRows`] when every row was given
    /// already. The comparison is unchanged by a refused row.
    pub fn push_row(&mut self, a: &[u8], b: &[u8]) -> Result<(), SsimError> {
        let width = index(self.width);
        for row in [a, b] {
            if row.len() != width {
                return Err(SsimError::RowLength {
                    width: self.width,
                    found: row.len(),
                });
            }
        }
        if self.given == self.height {
            return Err(SsimError::TooManyRows {
                height: self.height,
            });
        }
        // The five values of each pixel, in the order of `SUMS`.
        for (x, (&pa, &pb)) in a.iter().zip(b).enumerate() {
            let (pa, pb) = (f64::from(pa), f64::from(pb));
            for (sum, value) in [pa, pb, pa * pa, pb * pb, pa * pb].into_iter().enumerate() {
                self.values[sum * width + x] = value;
            }
        }
        // Blur across: each sum starts at 0 and adds the taps in order, so
        // the result does not depend on how the loops are arranged.
        let columns = self.columns;
        let slot = index(self.given % SSIM_WINDOW);
        for sum in 0..SUMS {
            let start = (slot * SUMS + sum) * columns;
            let out = &mut self.across[start..start + columns];
            out.fill(0.0);
            let values = &self.values[sum * width..(sum + 1) * width];
            for (tap, &weight) in self.weights.iter().enumerate() {
                for (out, &value) in out.iter_mut().zip(&values[tap..tap + columns]) {
                    *out += weight * value;
                }
            }
        }
        self.given += 1;
        if self.given >= SSIM_WINDOW {
            self.add_window_row(index(self.given - SSIM_WINDOW));
        }
        Ok(())
    }

    /// Blurs the last 11 rows down into the row of window positions whose
    /// window starts at image row `top`, and adds the SSIM of each position
    /// to the total.
    fn add_window_row(&mut self, top: usize) {
        let columns = self.columns;
        for sum in 0..SUMS {
            let out = &mut self.window[sum * columns..(sum + 1) * columns];
            out.fill(0.0);
            for (tap, &weight) in self.weights.iter().enumerate() {
                let start = (((top + tap) % WINDOW) * SUMS + sum) * columns;
                for (out, &value) in out.iter_mut().zip(&self.across[start..start + columns]) {
                    *out += weight * value;
                }
            }
        }
        let [mean_a, mean_b, square_a, square_b, product] =
            [0, 1, 2, 3, 4].map(|sum| &self.window[sum * columns..(sum + 1) * columns]);
        for x in 0..columns {
            let (mean_a, mean_b) = (mean_a[x], mean_b[x]);
            let variance_a = square_a[x] - mean_a * mean_a;
            let variance_b = square_b[x] - mean_b * mean_b;
            let covariance = product[x] - mean_a * mean_b;
            self.total += ((2.0 * mean_a * mean_b + C1) * (2.0 * covariance + C2))
                / ((mean_a * mean_a + mean_b * mean_b + C1) * (variance_a + variance_b + C2));
        }
    }

    /// The mean SSIM over every window position: 1.0 for identical images,
    /// lower the more they differ, and never above 1.
    ///
    /// # Errors
    ///
    /// [`SsimError::MissingRows`] when not every row was given.
    pub fn finish(self) -> Result<f64, SsimError> {
        if self.given != self.height {
            return Err(SsimError::MissingRows {
                height: self.height,
                given: self.given,
            });
        }
        // Both factors come from `u32` sides, so their product is exact in
        // f64 for every image backend3 can hold.
        let positions =
            f64::from(self.width - SSIM_WINDOW + 1) * f64::from(self.height - SSIM_WINDOW + 1);
        Ok(self.total / positions)
    }
}

/// The mean luma SSIM of two whole greyscale planes: [`LumaSsim`] given
/// every row in order.
///
/// - `a`, `b`: the two planes, one 8-bit sample per pixel, row by row.
/// - `dimensions`: their size; both sides at least [`SSIM_WINDOW`].
///
/// # Errors
///
/// [`SsimError::TooSmall`] for a side below [`SSIM_WINDOW`], and
/// [`SsimError::PlaneLength`] when a plane does not hold
/// `width × height` samples.
pub fn mean_ssim(a: &[u8], b: &[u8], dimensions: Dimensions) -> Result<f64, SsimError> {
    let mut ssim = LumaSsim::new(dimensions)?;
    for plane in [a, b] {
        if u64::try_from(plane.len()).ok() != Some(dimensions.pixels()) {
            return Err(SsimError::PlaneLength {
                expected: dimensions.pixels(),
                found: plane.len(),
            });
        }
    }
    let width = index(dimensions.width());
    for (a, b) in a.chunks_exact(width).zip(b.chunks_exact(width)) {
        ssim.push_row(a, b)?;
    }
    ssim.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*};

    fn dimensions(width: u32, height: u32) -> Dimensions {
        Dimensions::new(width, height).unwrap()
    }

    /// A 32 × 24 test pattern: a gradient with a checkerboard on it.
    fn pattern() -> Vec<u8> {
        (0..24_u8)
            .flat_map(|y| {
                (0..32_u8)
                    .map(move |x| x * 6 + y * 2 + if (x / 4 + y / 4) % 2 == 0 { 20 } else { 0 })
            })
            .collect()
    }

    /// `image` with every third sample raised by `amount`.
    fn noisy(image: &[u8], amount: u8) -> Vec<u8> {
        (0_usize..)
            .zip(image)
            .map(|(i, &value)| {
                if i % 3 == 0 {
                    value.saturating_add(amount)
                } else {
                    value
                }
            })
            .collect()
    }

    /// The SSIM straight from its definition: every window position, every
    /// one of its 121 pixels weighted by the outer product of the Gaussian
    /// weights. Slow, and independent of the two-pass streaming above, so it
    /// is the oracle for the ring of rows.
    fn by_definition(a: &[u8], b: &[u8], width: usize, height: usize) -> f64 {
        let weights = weights();
        let mut total = 0.0;
        let mut positions = 0_u32;
        for top in 0..=height - WINDOW {
            for left in 0..=width - WINDOW {
                let mut sums = [0.0_f64; SUMS];
                for (dy, wy) in weights.iter().enumerate() {
                    for (dx, wx) in weights.iter().enumerate() {
                        let at = (top + dy) * width + left + dx;
                        let (pa, pb) = (f64::from(a[at]), f64::from(b[at]));
                        for (sum, value) in sums.iter_mut().zip([pa, pb, pa * pa, pb * pb, pa * pb])
                        {
                            *sum += wy * wx * value;
                        }
                    }
                }
                let [ma, mb, sa, sb, ab] = sums;
                total += ((2.0 * ma * mb + C1) * (2.0 * (ab - ma * mb) + C2))
                    / ((ma * ma + mb * mb + C1) * ((sa - ma * ma) + (sb - mb * mb) + C2));
                positions += 1;
            }
        }
        total / f64::from(positions)
    }

    #[test]
    fn the_constants_are_those_of_wang_et_al() {
        assert_eq!(SSIM_WINDOW, 11);
        assert!((SIGMA - 1.5).abs() < f64::EPSILON);
        assert!((C1 - 6.5025).abs() < 1e-12);
        assert!((C2 - 58.5225).abs() < 1e-12);
    }

    #[test]
    fn the_window_is_a_normalised_symmetric_gaussian() {
        let weights = weights();
        assert!((weights.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        for tap in 0..WINDOW {
            assert!((weights[tap] - weights[WINDOW - 1 - tap]).abs() < 1e-15);
        }
        assert!(weights[5] > weights[4] && weights[4] > weights[0]);
        // e^(-25/4.5) relative to the centre, for the outermost tap.
        assert!((weights[0] / weights[5] - (-25.0_f64 / 4.5).exp()).abs() < 1e-15);
    }

    #[test]
    fn identical_images_score_one() {
        let image = pattern();
        let ssim = mean_ssim(&image, &image, dimensions(32, 24)).unwrap();
        assert!((ssim - 1.0).abs() < 1e-12);
        let flat = vec![128; 11 * 11];
        assert!((mean_ssim(&flat, &flat, dimensions(11, 11)).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn two_flat_images_score_their_luminance_term() {
        // No variance: the structure term is C2 / C2, and the luminance term
        // is (2 × 100 × 110 + C1) / (100² + 110² + C1).
        let (a, b) = (vec![100; 15 * 12], vec![110; 15 * 12]);
        let expected = (2.0 * 100.0 * 110.0 + 6.5025) / (100.0 * 100.0 + 110.0 * 110.0 + 6.5025);
        let ssim = mean_ssim(&a, &b, dimensions(15, 12)).unwrap();
        assert!((ssim - expected).abs() < 1e-12, "{ssim}");
    }

    #[test]
    fn more_damage_scores_lower() {
        let image = pattern();
        let size = dimensions(32, 24);
        let light = mean_ssim(&image, &noisy(&image, 4), size).unwrap();
        let heavy = mean_ssim(&image, &noisy(&image, 40), size).unwrap();
        assert!(1.0 > light && light > heavy, "{light} {heavy}");
        let inverted: Vec<u8> = image.iter().map(|value| 255 - value).collect();
        assert!(mean_ssim(&image, &inverted, size).unwrap() < 0.0);
    }

    #[test]
    fn the_value_is_the_one_the_encoder_measurement_used() {
        // The implementation that docs/design/image-encoders.md measured with
        // (formerly test code in tests/domain/) gave exactly this value for
        // this pair; the streaming rewrite keeps the order of every sum.
        let image = pattern();
        let ssim = mean_ssim(&image, &noisy(&image, 4), dimensions(32, 24)).unwrap();
        assert_eq!(
            ssim.to_bits(),
            0.991_220_350_534_993_8_f64.to_bits(),
            "{ssim}"
        );
    }

    #[test]
    fn images_smaller_than_the_window_cannot_be_compared() {
        for (width, height) in [(10, 11), (11, 10), (1, 1), (10, 500)] {
            assert_eq!(
                LumaSsim::new(dimensions(width, height)).unwrap_err(),
                SsimError::TooSmall { width, height }
            );
        }
        assert!(LumaSsim::new(dimensions(11, 11)).is_ok());
    }

    #[test]
    fn rows_must_fit_the_width_and_the_height() {
        let mut ssim = LumaSsim::new(dimensions(12, 11)).unwrap();
        let row = [7; 12];
        assert_eq!(
            ssim.push_row(&row[..11], &row),
            Err(SsimError::RowLength {
                width: 12,
                found: 11
            })
        );
        assert_eq!(
            ssim.push_row(&row, &[7; 13]),
            Err(SsimError::RowLength {
                width: 12,
                found: 13
            })
        );
        for _ in 0..10 {
            ssim.push_row(&row, &row).unwrap();
        }
        assert_eq!(
            ssim.clone().finish(),
            Err(SsimError::MissingRows {
                height: 11,
                given: 10
            })
        );
        ssim.push_row(&row, &row).unwrap();
        assert_eq!(
            ssim.push_row(&row, &row),
            Err(SsimError::TooManyRows { height: 11 })
        );
        assert!((ssim.finish().unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn whole_planes_must_hold_every_pixel() {
        let size = dimensions(12, 11);
        assert_eq!(
            mean_ssim(&[0; 131], &[0; 132], size),
            Err(SsimError::PlaneLength {
                expected: 132,
                found: 131
            })
        );
        assert_eq!(
            mean_ssim(&[0; 132], &[0; 144], size),
            Err(SsimError::PlaneLength {
                expected: 132,
                found: 144
            })
        );
        assert_eq!(
            mean_ssim(&[0; 100], &[0; 100], dimensions(10, 10)),
            Err(SsimError::TooSmall {
                width: 10,
                height: 10
            })
        );
    }

    /// Two images of the same random size from 11 × 11 to 24 × 24.
    fn image_pair() -> impl Strategy<Value = (u32, u32, Vec<u8>, Vec<u8>)> {
        (11..=24_u32, 11..=24_u32).prop_flat_map(|(width, height)| {
            let pixels = usize::try_from(width * height).unwrap();
            (
                Just(width),
                Just(height),
                vec(any::<u8>(), pixels),
                vec(any::<u8>(), pixels),
            )
        })
    }

    proptest! {
        #[test]
        fn streaming_agrees_with_the_definition((width, height, a, b) in image_pair()) {
            let streamed = mean_ssim(&a, &b, dimensions(width, height)).unwrap();
            let defined = by_definition(&a, &b, index(width), index(height));
            prop_assert!((streamed - defined).abs() < 1e-9, "{streamed} {defined}");
        }

        #[test]
        fn ssim_is_symmetric_and_at_most_one((width, height, a, b) in image_pair()) {
            let size = dimensions(width, height);
            let forward = mean_ssim(&a, &b, size).unwrap();
            prop_assert_eq!(forward.to_bits(), mean_ssim(&b, &a, size).unwrap().to_bits());
            prop_assert!((-1.0 - 1e-12..=1.0 + 1e-12).contains(&forward));
            prop_assert!((mean_ssim(&a, &a, size).unwrap() - 1.0).abs() < 1e-12);
        }

        #[test]
        fn wrong_shapes_are_errors_never_panics(
            width in 1..40_u32,
            height in 1..40_u32,
            rows in vec((vec(any::<u8>(), 0..45), vec(any::<u8>(), 0..45)), 0..45),
        ) {
            let Ok(mut ssim) = LumaSsim::new(dimensions(width, height)) else {
                prop_assert!(width < SSIM_WINDOW || height < SSIM_WINDOW);
                return Ok(());
            };
            let mut given = 0;
            for (a, b) in &rows {
                if ssim.push_row(a, b).is_ok() {
                    given += 1;
                }
            }
            prop_assert!(given <= height);
            let result = ssim.finish();
            prop_assert_eq!(result.is_ok(), given == height);
        }
    }
}
