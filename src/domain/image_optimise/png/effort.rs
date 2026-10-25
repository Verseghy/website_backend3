//! How hard the PNG encoder works, by the size of the image (ADR 0033 §5).
//!
//! ADR 0015 §7 optimises an upload inside its request, one optimisation at a
//! time per process, and ADR 0023 §5 cuts an `/api/admin` request off at
//! 120 seconds. oxipng's time grows with the pixel count: at its preset 2, an
//! 8-bit photograph at the 50-megapixel pixel budget took 57.7 CPU seconds
//! on the measuring machine and a 16-bit one 152, and preset 3 took 122.5
//! for the 8-bit one. The maintainer chose to keep preset 2 below a
//! pixel-count threshold and to use a cheaper preset, 1 or lower, above it
//! (decision D82), so that a PNG at the budget finishes inside the timeout
//! with room for the upload's transfer and for a second upload queued
//! behind the semaphore.
//!
//! [`PngEffort`] holds the threshold and the cheaper preset; preset 2 below
//! the threshold is [`PNG_PRESET`], a constant, because the decision fixes
//! it. The two it holds are configuration: the production threshold is set
//! from a measurement on the production node, whose cores are not those of
//! the machine the measurements ran on, and the same measurement says
//! whether preset 1 fits above it. The settings build the effort once at
//! boot through [`PngEffort::new`], which refuses a value that would break
//! the rule, and the caller passes it to [`super::optimise_png`]; nothing
//! here reads the environment (ADR 0008).

/// The preset at or below the threshold: preset 2, the one the encoder
/// measurement chose and decision D82 keeps there (ADR 0033 §5).
pub const PNG_PRESET: u8 = 2;

/// The highest preset allowed above the threshold: ADR 0033 §5 says "a
/// cheaper preset (1 or lower)" there.
pub const LARGE_PNG_PRESET_MAX: u8 = 1;

/// The oxipng presets for a PNG optimisation, chosen by the pixel count of
/// the image (ADR 0033 §5). Every value of this type passed the checks of
/// [`PngEffort::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PngEffort {
    /// The largest pixel count optimised with [`PNG_PRESET`].
    threshold_pixels: u64,
    /// The preset above the threshold, at most [`LARGE_PNG_PRESET_MAX`].
    large_preset: u8,
}

/// Why a [`PngEffort`] is refused. Each message says what to change; the
/// settings add the key that holds the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PngEffortError {
    /// A threshold of 0 pixels would send every image to the cheaper preset,
    /// which is almost certainly a missing or mistyped value.
    #[error(
        "threshold_pixels is 0; set the pixel count above which the cheaper preset is used \
         (the default is {})",
        PngEffort::DEFAULT.threshold_pixels
    )]
    ZeroThreshold,
    /// The preset above the threshold is not one of the cheap presets that
    /// ADR 0033 §5 allows there.
    #[error(
        "large_preset is {large_preset}; above the threshold ADR 0033 §5 allows preset \
         {LARGE_PNG_PRESET_MAX} or lower"
    )]
    LargePresetTooSlow {
        /// The preset that was given.
        large_preset: u8,
    },
}

impl PngEffort {
    /// The default: preset 2 up to 6 megapixels, and preset 0 above.
    ///
    /// Derived from the measurements of `docs/design/image-encoders.md` on
    /// the measuring machine (a Ryzen 9 5900X, one thread). oxipng's time
    /// grows with the decoded size, so the costliest pixel is the widest: at
    /// preset 2 a 16-bit RGBA photograph took about 3.05 CPU seconds per
    /// megapixel, an 8-bit RGB one about 1.16, and random noise, which
    /// libdeflate gives up on, under 0.65. The 120-second timeout must hold
    /// an upload's transfer and two optimisations, its own and the one it
    /// may wait for behind the semaphore, on a node assumed twice as slow,
    /// so one optimisation gets at most about 25 seconds of the measuring
    /// machine. Preset 2 reaches that at about 8 megapixels of 16-bit RGBA;
    /// 6 megapixels leaves a margin, at about 18 seconds. Above it, preset 1
    /// would take 32 to 99 seconds at the 50-megapixel budget, too much,
    /// and preset 0 took at most about 21: 15 for a 16-bit photograph at
    /// the budget near the 128 MiB upload limit, and 21 for an animation of
    /// 249 megapixels. The price is size: above the threshold a photograph
    /// keeps 99 to 100 % of its size instead of 80 to 95 %, and a
    /// screenshot about 94 % instead of 78 %. The production value is set
    /// on the node (ADR 0033 §5); this default only has to be safe until
    /// then.
    pub const DEFAULT: Self = Self {
        threshold_pixels: 6_000_000,
        large_preset: 0,
    };

    /// The effort with these values, once they are checked.
    ///
    /// - `threshold_pixels`: the largest pixel count, summed over every
    ///   image of the file, that is optimised with [`PNG_PRESET`]; at least
    ///   1.
    /// - `large_preset`: the oxipng preset above the threshold, 0 to
    ///   [`LARGE_PNG_PRESET_MAX`].
    ///
    /// # Errors
    ///
    /// The [`PngEffortError`] of the first check that fails, in the order of
    /// the arguments.
    pub const fn new(threshold_pixels: u64, large_preset: u8) -> Result<Self, PngEffortError> {
        if threshold_pixels == 0 {
            return Err(PngEffortError::ZeroThreshold);
        }
        if large_preset > LARGE_PNG_PRESET_MAX {
            return Err(PngEffortError::LargePresetTooSlow { large_preset });
        }
        Ok(Self {
            threshold_pixels,
            large_preset,
        })
    }

    /// The largest pixel count optimised with [`PNG_PRESET`].
    #[must_use]
    pub const fn threshold_pixels(self) -> u64 {
        self.threshold_pixels
    }

    /// The preset above the threshold.
    #[must_use]
    pub const fn large_preset(self) -> u8 {
        self.large_preset
    }

    /// The preset for a PNG of `pixels` pixels: [`PNG_PRESET`] at or below
    /// the threshold, [`PngEffort::large_preset`] above it.
    ///
    /// - `pixels`: the pixels the encoder works on, summed over every image
    ///   of the file: the canvas, and for an animated PNG every frame at its
    ///   own size as well, since oxipng recompresses each one.
    #[must_use]
    pub const fn preset_for(self, pixels: u64) -> u8 {
        if pixels <= self.threshold_pixels {
            PNG_PRESET
        } else {
            self.large_preset
        }
    }
}

impl Default for PngEffort {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_default_passes_its_own_checks() {
        let default = PngEffort::DEFAULT;
        assert_eq!(
            PngEffort::new(default.threshold_pixels(), default.large_preset()),
            Ok(default)
        );
        assert_eq!(PngEffort::default(), default);
        assert_eq!((PNG_PRESET, default.large_preset()), (2, 0));
    }

    #[test]
    fn the_preset_changes_just_above_the_threshold() {
        let effort = PngEffort::new(1_000, 1).unwrap();
        assert_eq!(effort.preset_for(0), 2);
        assert_eq!(effort.preset_for(999), 2);
        assert_eq!(effort.preset_for(1_000), 2);
        assert_eq!(effort.preset_for(1_001), 1);
        assert_eq!(effort.preset_for(u64::MAX), 1);
        let default = PngEffort::DEFAULT;
        assert_eq!(default.preset_for(6_000_000), 2);
        assert_eq!(default.preset_for(6_000_001), 0);
        // The pixel budget of ADR 0015 §7 is above the default threshold.
        assert_eq!(default.preset_for(50_000_000), 0);
    }

    #[test]
    fn each_check_refuses_with_its_own_error() {
        assert_eq!(PngEffort::new(0, 0), Err(PngEffortError::ZeroThreshold));
        assert_eq!(
            PngEffort::new(1, 2),
            Err(PngEffortError::LargePresetTooSlow { large_preset: 2 })
        );
        // The zero threshold is reported first.
        assert_eq!(PngEffort::new(0, 9), Err(PngEffortError::ZeroThreshold));
        // The edges that pass: threshold 1, the largest cheap preset.
        assert!(PngEffort::new(1, LARGE_PNG_PRESET_MAX).is_ok());
        assert!(PngEffort::new(u64::MAX, 0).is_ok());
    }

    #[test]
    fn the_messages_say_what_to_change() {
        assert_eq!(
            PngEffortError::ZeroThreshold.to_string(),
            "threshold_pixels is 0; set the pixel count above which the cheaper preset is used \
             (the default is 6000000)"
        );
        assert_eq!(
            PngEffortError::LargePresetTooSlow { large_preset: 3 }.to_string(),
            "large_preset is 3; above the threshold ADR 0033 §5 allows preset 1 or lower"
        );
    }

    proptest! {
        #[test]
        fn exactly_the_valid_values_are_accepted(
            threshold in any::<u64>(),
            large_preset in any::<u8>(),
        ) {
            let valid = threshold > 0 && large_preset <= LARGE_PNG_PRESET_MAX;
            let effort = PngEffort::new(threshold, large_preset);
            prop_assert_eq!(effort.is_ok(), valid);
            if let Ok(effort) = effort {
                prop_assert_eq!(
                    (effort.threshold_pixels(), effort.large_preset()),
                    (threshold, large_preset)
                );
            }
        }

        #[test]
        fn larger_images_never_get_a_slower_preset(
            threshold in 1..=u64::MAX,
            large_preset in 0..=LARGE_PNG_PRESET_MAX,
            a in any::<u64>(),
            b in any::<u64>(),
        ) {
            let effort = PngEffort::new(threshold, large_preset).unwrap();
            let (small, large) = (a.min(b), a.max(b));
            prop_assert!(effort.preset_for(large) <= effort.preset_for(small));
            prop_assert_eq!(effort.preset_for(small) == PNG_PRESET, small <= threshold);
            prop_assert_eq!(effort.preset_for(large) == large_preset, large > threshold);
        }
    }
}
