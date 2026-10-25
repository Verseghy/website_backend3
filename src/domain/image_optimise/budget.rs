//! The stored size of an image and the pixel budget it must fit
//! (ADR 0015 §7).
//!
//! A decoder needs memory in proportion to the pixel count, not to the file
//! size, so the budget counts pixels: `width × height` of the stored image,
//! or of the canvas of an animation. It is configurable and has no format of
//! its own, so the caller builds it from its settings with
//! [`PixelBudget::new`], which refuses a budget that admits nothing.

use core::fmt;

/// The stored width and height of an image in pixels, both at least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dimensions {
    width: u32,
    height: u32,
}

impl Dimensions {
    /// The dimensions `width × height`, or `None` when either is zero: such
    /// an image has no pixels to show, and its pixel count says nothing about
    /// what a decoder would allocate.
    ///
    /// - `width`, `height`: the sides in pixels, as the header stores them.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self { width, height })
        }
    }

    /// The stored width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// The stored height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// The pixel count, `width × height`. It cannot overflow: the largest
    /// product of two `u32` values fits in a `u64`.
    #[must_use]
    pub fn pixels(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

impl fmt::Display for Dimensions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}x{}", self.width, self.height)
    }
}

/// The largest pixel count that backend3 decodes for one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelBudget {
    max_pixels: u64,
}

impl PixelBudget {
    /// The default of ADR 0015 §7: 50 megapixels, 50,000,000 pixels.
    pub const DEFAULT: Self = Self {
        max_pixels: 50_000_000,
    };

    /// A budget of `max_pixels` pixels.
    ///
    /// - `max_pixels`: the largest pixel count to admit, inclusive.
    ///
    /// # Errors
    ///
    /// [`PixelBudgetError::Zero`] when `max_pixels` is 0, which would refuse
    /// every image.
    pub const fn new(max_pixels: u64) -> Result<Self, PixelBudgetError> {
        if max_pixels == 0 {
            Err(PixelBudgetError::Zero)
        } else {
            Ok(Self { max_pixels })
        }
    }

    /// The largest pixel count this budget admits.
    #[must_use]
    pub const fn max_pixels(self) -> u64 {
        self.max_pixels
    }

    /// Whether an image of `dimensions` fits: its pixel count is at most the
    /// budget.
    ///
    /// - `dimensions`: the stored size of the image, or of one frame.
    #[must_use]
    pub fn admits(self, dimensions: Dimensions) -> bool {
        dimensions.pixels() <= self.max_pixels
    }
}

impl Default for PixelBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Why a pixel budget cannot be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PixelBudgetError {
    /// A budget of zero pixels.
    #[error("the pixel budget must be at least 1 pixel; the default is 50000000 (50 megapixels)")]
    Zero,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn zero_sides_have_no_dimensions() {
        assert_eq!(Dimensions::new(0, 1), None);
        assert_eq!(Dimensions::new(1, 0), None);
        assert_eq!(Dimensions::new(0, 0), None);
        let one = Dimensions::new(1, 1).unwrap();
        assert_eq!((one.width(), one.height(), one.pixels()), (1, 1, 1));
    }

    #[test]
    fn the_largest_dimensions_do_not_overflow() {
        let largest = Dimensions::new(u32::MAX, u32::MAX).unwrap();
        assert_eq!(largest.pixels(), 18_446_744_065_119_617_025);
        assert_eq!(largest.to_string(), "4294967295x4294967295");
    }

    #[test]
    fn the_default_is_50_megapixels_inclusive() {
        assert_eq!(PixelBudget::default(), PixelBudget::DEFAULT);
        assert_eq!(PixelBudget::DEFAULT.max_pixels(), 50_000_000);
        assert!(PixelBudget::DEFAULT.admits(Dimensions::new(10_000, 5_000).unwrap()));
        assert!(!PixelBudget::DEFAULT.admits(Dimensions::new(8_688, 5_792).unwrap()));
    }

    #[test]
    fn a_zero_budget_is_refused_with_the_fix() {
        assert_eq!(PixelBudget::new(0), Err(PixelBudgetError::Zero));
        assert!(
            PixelBudgetError::Zero
                .to_string()
                .contains("at least 1 pixel")
        );
        assert_eq!(PixelBudget::new(1).map(PixelBudget::max_pixels), Ok(1));
    }

    proptest! {
        #[test]
        fn dimensions_exist_exactly_for_nonzero_sides(width in any::<u32>(), height in any::<u32>()) {
            let dimensions = Dimensions::new(width, height);
            prop_assert_eq!(dimensions.is_some(), width != 0 && height != 0);
            if let Some(dimensions) = dimensions {
                prop_assert_eq!((dimensions.width(), dimensions.height()), (width, height));
                prop_assert_eq!(u128::from(dimensions.pixels()), u128::from(width) * u128::from(height));
                prop_assert_eq!(dimensions.to_string(), format!("{width}x{height}"));
            }
        }

        #[test]
        fn a_budget_admits_exactly_the_pixel_counts_up_to_it(
            max in 1..=u64::MAX,
            width in 1..=u32::MAX,
            height in 1..=u32::MAX,
        ) {
            let budget = PixelBudget::new(max).unwrap();
            prop_assert_eq!(budget.max_pixels(), max);
            let dimensions = Dimensions::new(width, height).unwrap();
            prop_assert_eq!(budget.admits(dimensions), dimensions.pixels() <= max);
            let exact = PixelBudget::new(dimensions.pixels()).unwrap();
            prop_assert!(exact.admits(dimensions));
        }
    }
}
