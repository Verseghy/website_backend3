//! The keep-or-discard rule for a lossy re-encode (ADR 0015 §7, amended by
//! ADR 0033 §1).
//!
//! A re-encode is not lossless, so it has to pay for itself and stay
//! visually lossless. ADR 0015 §7 kept the re-encoded bytes when they were at
//! least 5 % smaller than the stripped original. Half of the measured
//! photographs then stayed below the luma SSIM of 0.98 that the same ADR's
//! tests ask for (`docs/design/image-encoders.md`), so the maintainer added
//! that measure to the rule (ADR 0033 §1): a re-encode is kept only when it
//! is at least 5 % smaller **and** its luma SSIM against the decoded
//! original is at least [`SSIM_FLOOR`]. Otherwise the stripped original is
//! stored. The rule is the same for JPEG and lossy WebP, so it lives here,
//! apart from either format.
//!
//! The two halves are separate functions because they cost very different
//! amounts: the size test compares two lengths, while the SSIM needs the
//! re-encode decoded again. A caller tests the size first and measures the
//! SSIM only for a re-encode that passes it.

/// The lowest luma SSIM a kept re-encode may have, compared with the decoded
/// stripped original: ADR 0015 §7's "visually lossless" measure, which
/// ADR 0033 §1 makes part of the keep rule.
pub const SSIM_FLOOR: f64 = 0.98;

/// Whether a re-encode passes the size half of the keep rule: it is at
/// least 5 % smaller than the stripped original,
/// `reencoded_len ≤ 0.95 × stripped_len`. Exactly 5 % smaller passes.
///
/// - `stripped_len`: the size in bytes of the original after the lossless
///   metadata strip, which is what the editor would otherwise get.
/// - `reencoded_len`: the size in bytes of the re-encode, with the same
///   metadata as the stripped original.
#[must_use]
pub fn reencode_is_kept(stripped_len: usize, reencoded_len: usize) -> bool {
    // 0.95 is 19/20; in integers, so no rounding decides a boundary case.
    // Both sides fit a u128 for any usize, so the products cannot overflow.
    // The first test only matters for an empty original, where 0 ≤ 0.95 × 0
    // would keep a re-encode that saves nothing.
    reencoded_len < stripped_len && (reencoded_len as u128) * 20 <= (stripped_len as u128) * 19
}

/// Whether a re-encode passes the similarity half of the keep rule: its
/// luma SSIM against the decoded stripped original is at least
/// [`SSIM_FLOOR`]. Exactly 0.98 passes.
///
/// - `luma_ssim`: the mean SSIM of the two luma planes at full resolution,
///   from [`super::LumaSsim`]. A NaN, which no comparison of real images
///   gives, fails, so an arithmetic fault can never keep a re-encode.
#[must_use]
pub fn reencode_meets_ssim_floor(luma_ssim: f64) -> bool {
    luma_ssim >= SSIM_FLOOR
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn five_percent_smaller_is_kept_and_less_is_not() {
        assert!(reencode_is_kept(100, 95));
        assert!(!reencode_is_kept(100, 96));
        assert!(reencode_is_kept(1_000_000, 950_000));
        assert!(!reencode_is_kept(1_000_000, 950_001));
        // 19 bytes is 5 % of 380: 361 is kept, 362 is not.
        assert!(reencode_is_kept(380, 361));
        assert!(!reencode_is_kept(380, 362));
    }

    #[test]
    fn equal_or_larger_is_never_kept() {
        assert!(!reencode_is_kept(1, 1));
        assert!(!reencode_is_kept(100, 100));
        assert!(!reencode_is_kept(100, 101));
        assert!(!reencode_is_kept(0, 0));
    }

    #[test]
    fn the_largest_sizes_do_not_overflow() {
        assert!(reencode_is_kept(usize::MAX, usize::MAX / 20 * 19));
        assert!(!reencode_is_kept(usize::MAX, usize::MAX));
    }

    #[test]
    fn the_floor_is_098_inclusive() {
        assert!((SSIM_FLOOR - 0.98).abs() < f64::EPSILON);
        assert!(reencode_meets_ssim_floor(0.98));
        assert!(reencode_meets_ssim_floor(0.980_000_1));
        assert!(reencode_meets_ssim_floor(1.0));
        assert!(!reencode_meets_ssim_floor(0.979_999_9));
        assert!(!reencode_meets_ssim_floor(0.974));
        assert!(!reencode_meets_ssim_floor(-1.0));
    }

    #[test]
    fn a_nan_never_meets_the_floor() {
        assert!(!reencode_meets_ssim_floor(f64::NAN));
        assert!(!reencode_meets_ssim_floor(-f64::NAN));
    }

    proptest! {
        #[test]
        fn the_rule_is_exactly_the_95_percent_bound(stripped in any::<usize>(), reencoded in any::<usize>()) {
            let bound = u128::try_from(stripped).unwrap() * 95;
            let kept = reencoded < stripped && u128::try_from(reencoded).unwrap() * 100 <= bound;
            prop_assert_eq!(reencode_is_kept(stripped, reencoded), kept);
        }

        #[test]
        fn a_kept_reencode_is_smaller_and_so_is_any_smaller_one(
            stripped in 1..=usize::MAX,
            reencoded in any::<usize>(),
            less in any::<usize>(),
        ) {
            if reencode_is_kept(stripped, reencoded) {
                prop_assert!(reencoded < stripped);
                prop_assert!(reencode_is_kept(stripped, reencoded.saturating_sub(less)));
            }
        }

        #[test]
        fn the_floor_splits_every_value_once(ssim in -1.0..=1.0_f64, more in 0.0..=1.0_f64) {
            prop_assert_eq!(reencode_meets_ssim_floor(ssim), ssim >= 0.98);
            // Monotonic: a better SSIM never fails where a worse one passes.
            if reencode_meets_ssim_floor(ssim) {
                prop_assert!(reencode_meets_ssim_floor((ssim + more).min(1.0)));
            }
        }
    }
}
