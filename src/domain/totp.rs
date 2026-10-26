//! TOTP codes for two-factor sign-in (ADR 0006 §6): RFC 6238 over HOTP
//! (RFC 4226) with HMAC-SHA-1, 6 digits and 30-second steps from the Unix
//! epoch, the settings common authenticator apps use.
//!
//! [`verify`] is a pure function of the secret, the submitted code, the last
//! used step (`users.totp_last_step`) and the current time, which the caller
//! passes in (ADR 0008 rule 1). It accepts the previous, current and next
//! step, refuses a code for the last used step or an earlier one, and
//! returns the step it accepted.
//!
//! The replay rule holds only if every caller stores that step:
//!
//! - A sign-in stores it with a compare-and-set, `UPDATE users SET
//!   totp_last_step = $step WHERE id = $id AND (totp_last_step IS NULL OR
//!   totp_last_step < $step)`, and treats zero updated rows as
//!   [`CodeRejection::Replayed`]. With a plain read, verify and write, two
//!   concurrent sign-ins with one code would both succeed.
//! - Enrolment confirms its first code with `verify(secret, code, None,
//!   now)` and stores the returned step as `totp_last_step` in the write
//!   that turns two-factor on, so the confirming code cannot also sign in.
//!
//! Steps are `i64` like `users.totp_last_step` (`bigint`). HOTP's counter is
//! unsigned, so a step before the epoch has no code. [`TotpSecret`] never
//! prints its bytes, so a secret cannot reach a log through `{:?}`.

use std::fmt;

use hmac::{Hmac, KeyInit as _, Mac as _, digest::Key};
use sha1::Sha1;

/// The length of a TOTP secret: 20 bytes (ADR 0006 §6), the length RFC 4226
/// §4 recommends.
pub const SECRET_LEN: usize = 20;

/// The number of digits of a code.
pub const DIGITS: usize = 6;

/// The length of a time step in seconds (RFC 6238's default `X`, `T0 = 0`).
pub const STEP_SECS: i64 = 30;

/// How many steps around the current one are accepted (ADR 0006 §6, "Clock
/// skew"): one before and one after.
pub const SKEW_STEPS: i64 = 1;

/// `10^DIGITS`, the modulus of HOTP's truncation.
const CODE_MODULUS: u32 = 1_000_000;

/// A TOTP shared secret of [`SECRET_LEN`] bytes. It has no `PartialEq` and
/// no `Display`, and its `Debug` hides the bytes.
#[derive(Clone)]
pub struct TotpSecret([u8; SECRET_LEN]);

impl TotpSecret {
    /// Wraps the secret's bytes.
    ///
    /// - `bytes`: for a new enrolment, random bytes that the caller reads
    ///   from the operating system (`src/domain/` reads no randomness).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; SECRET_LEN]) -> Self {
        Self(bytes)
    }

    /// The secret's bytes, for the encryption at rest and the enrolment's
    /// `otpauth://` URI.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SECRET_LEN] {
        &self.0
    }
}

impl fmt::Debug for TotpSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TotpSecret(<redacted>)")
    }
}

/// Why [`verify`] refused a code. The variants are for logs and tests; a
/// sign-in answers all of them alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodeRejection {
    /// The code is not exactly [`DIGITS`] ASCII digits.
    #[error("a TOTP code is exactly 6 ASCII digits")]
    Malformed,
    /// The code belongs to none of the accepted steps.
    #[error("the TOTP code matches none of the previous, current and next step")]
    Mismatch,
    /// The code belongs only to steps at or before the last used step.
    #[error("the TOTP code belongs to a step that was already used")]
    Replayed,
}

/// The time step that contains an instant: `floor(unix_secs / 30)`.
///
/// - `unix_secs`: the instant as seconds since the Unix epoch, for example
///   `now.timestamp()` of the injected clock.
#[must_use]
pub const fn step_at(unix_secs: i64) -> i64 {
    unix_secs.div_euclid(STEP_SECS)
}

/// The code of a step, as [`DIGITS`] digits with leading zeros: what an
/// authenticator app shows. Only tests and fixtures use it; every check of
/// a submitted code goes through [`verify`], which applies the replay rule.
///
/// - `secret`: the account's TOTP secret.
/// - `step`: the time step (HOTP's counter); [`step_at`] gives it.
#[must_use]
pub fn code_for_step(secret: &TotpSecret, step: u64) -> String {
    let code = hotp(secret.as_bytes(), step, CODE_MODULUS);
    format!("{code:0DIGITS$}")
}

/// Checks a submitted code and returns the step it belongs to, which the
/// caller stores as the new `users.totp_last_step`.
///
/// - `secret`: the account's decrypted TOTP secret.
/// - `code`: the submitted code, exactly [`DIGITS`] ASCII digits. A caller
///   that lets users type a separator (some apps show `123 456`) removes it
///   first.
/// - `last_used_step`: the account's `users.totp_last_step`, `None` before
///   the first accepted code.
/// - `now_unix_secs`: the current instant as Unix seconds.
///
/// The code must be that of the previous, current or next step, and the
/// step must be later than `last_used_step`. If two of those steps show the
/// same digits (about one chance in a million), the later one is returned:
/// returning the earlier one would leave the later step unused, and the
/// same digits would then verify a second time (RFC 6238 §5.2). The cost is
/// that the user waits for a step with new digits. A step that enters the
/// window later is a new code, even if it happens to show the same digits.
/// All three candidates are always computed, so the time taken does not
/// reveal which step matched.
///
/// The caller stores the returned step as the module documentation says;
/// until then, the code is not used up.
///
/// # Errors
///
/// [`CodeRejection::Malformed`] for anything but [`DIGITS`] ASCII digits,
/// [`CodeRejection::Replayed`] when only steps at or before
/// `last_used_step` match, [`CodeRejection::Mismatch`] otherwise.
pub fn verify(
    secret: &TotpSecret,
    code: &str,
    last_used_step: Option<i64>,
    now_unix_secs: i64,
) -> Result<i64, CodeRejection> {
    // The digit check comes first: `str::parse` would accept a leading `+`.
    if code.len() != DIGITS || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CodeRejection::Malformed);
    }
    let submitted: u32 = code.parse().map_err(|_| CodeRejection::Malformed)?;
    let current = step_at(now_unix_secs);
    let mut accepted = None;
    let mut replayed = false;
    for offset in -SKEW_STEPS..=SKEW_STEPS {
        // `current` is at most i64::MAX / 30, so this cannot overflow; a
        // negative step has no counter and never matches.
        let step = current + offset;
        let Ok(counter) = u64::try_from(step) else {
            continue;
        };
        if hotp(secret.as_bytes(), counter, CODE_MODULUS) != submitted {
            continue;
        }
        if last_used_step.is_some_and(|last| step <= last) {
            replayed = true;
        } else {
            // The steps run in ascending order, so a later step that shows
            // the same digits replaces an earlier one (see above).
            accepted = Some(step);
        }
    }
    match accepted {
        Some(step) => Ok(step),
        None if replayed => Err(CodeRejection::Replayed),
        None => Err(CodeRejection::Mismatch),
    }
}

/// HOTP (RFC 4226 §5.3): HMAC-SHA-1 of the big-endian counter, dynamic
/// truncation, then the remainder modulo `modulus` (`10^digits`; the tests
/// also use `10^8` for the 8-digit RFC 6238 vectors).
///
/// - `secret`: the shared secret.
/// - `counter`: the moving factor, here the time step.
/// - `modulus`: `10^digits`.
fn hotp(secret: &[u8; SECRET_LEN], counter: u64, modulus: u32) -> u32 {
    // HMAC zero-pads a key shorter than SHA-1's 64-byte block (RFC 2104 §2),
    // so padding the secret here gives the same MAC through the infallible
    // block-sized constructor, without an error path for a fixed length.
    let mut key = Key::<Hmac<Sha1>>::default();
    key[..SECRET_LEN].copy_from_slice(secret);
    let mut mac = Hmac::<Sha1>::new(&key);
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    // The low 4 bits of the last byte pick 4 bytes; the top bit is masked.
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    binary % modulus
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The secret of the RFC 4226 and RFC 6238 SHA-1 vectors.
    const RFC_SECRET: [u8; SECRET_LEN] = *b"12345678901234567890";

    fn rfc_secret() -> TotpSecret {
        TotpSecret::from_bytes(RFC_SECRET)
    }

    /// The code of `step`, which must not be negative.
    fn code(secret: &TotpSecret, step: i64) -> String {
        code_for_step(secret, u64::try_from(step).expect("non-negative step"))
    }

    #[test]
    fn hotp_matches_rfc_4226_appendix_d() {
        let expected = [
            755_224, 287_082, 359_152, 969_429, 338_314, 254_676, 287_922, 162_583, 399_871,
            520_489,
        ];
        for (counter, value) in (0_u64..).zip(expected) {
            assert_eq!(hotp(&RFC_SECRET, counter, CODE_MODULUS), value, "{counter}");
        }
    }

    #[test]
    fn totp_matches_rfc_6238_appendix_b() {
        // The 8-digit SHA-1 vectors; a 6-digit code is their last six
        // digits, leading zeros included (07081804 gives 081804).
        for (unix_secs, step, eight_digits) in [
            (59, 0x1, "94287082"),
            (1_111_111_109, 0x0235_23EC, "07081804"),
            (1_111_111_111, 0x0235_23ED, "14050471"),
            (1_234_567_890, 0x0273_EF07, "89005924"),
            (2_000_000_000, 0x03F9_40AA, "69279037"),
            (20_000_000_000, 0x27BC_86AA, "65353130"),
        ] {
            assert_eq!(step_at(unix_secs), step, "{unix_secs}");
            let counter = u64::try_from(step).expect("positive");
            assert_eq!(
                format!("{:08}", hotp(&RFC_SECRET, counter, 100_000_000)),
                eight_digits
            );
            assert_eq!(code_for_step(&rfc_secret(), counter), eight_digits[2..]);
            assert_eq!(
                verify(&rfc_secret(), &eight_digits[2..], None, unix_secs),
                Ok(step)
            );
        }
    }

    #[test]
    fn step_boundaries() {
        for (unix_secs, step) in [
            (0, 0),
            (29, 0),
            (30, 1),
            (-1, -1),
            (-30, -1),
            (-31, -2),
            (i64::MAX, i64::MAX / 30),
        ] {
            assert_eq!(step_at(unix_secs), step, "{unix_secs}");
        }
    }

    #[test]
    fn only_the_previous_current_and_next_step_are_accepted() {
        let now = 1_234_567_890;
        let current = step_at(now);
        for step in [current - 1, current, current + 1] {
            assert_eq!(
                verify(&rfc_secret(), &code(&rfc_secret(), step), None, now),
                Ok(step)
            );
        }
        for step in [current - 2, current + 2] {
            assert_eq!(
                verify(&rfc_secret(), &code(&rfc_secret(), step), None, now),
                Err(CodeRejection::Mismatch)
            );
        }
    }

    #[test]
    fn the_used_step_and_earlier_ones_are_replays() {
        let now = 2_000_000_000;
        let current = step_at(now);
        let this = code(&rfc_secret(), current);
        let secret = rfc_secret();
        assert_eq!(
            verify(&secret, &this, Some(current), now),
            Err(CodeRejection::Replayed)
        );
        assert_eq!(
            verify(&secret, &this, Some(current + 1), now),
            Err(CodeRejection::Replayed)
        );
        let previous = code(&secret, current - 1);
        assert_eq!(
            verify(&secret, &previous, Some(current), now),
            Err(CodeRejection::Replayed)
        );
        assert_eq!(verify(&secret, &this, Some(current - 1), now), Ok(current));
        let next = code(&secret, current + 1);
        assert_eq!(verify(&secret, &next, Some(current), now), Ok(current + 1));
    }

    #[test]
    fn two_steps_with_the_same_digits_accept_them_once() {
        // Steps 910737 and 910738 of the RFC secret both show 911617.
        let secret = rfc_secret();
        assert_eq!(code(&secret, 910_737), "911617");
        assert_eq!(code(&secret, 910_738), "911617");
        // Both in the window, as current and next or as previous and
        // current: the later step is used up, so a resubmission is a replay.
        for now in [910_737 * STEP_SECS, 910_738 * STEP_SECS] {
            assert_eq!(verify(&secret, "911617", None, now), Ok(910_738));
            assert_eq!(
                verify(&secret, "911617", Some(910_738), now),
                Err(CodeRejection::Replayed)
            );
        }
        // Accepted as the next step one step earlier, the digits come back
        // as a new code once step 910738 is in the window.
        assert_eq!(
            verify(&secret, "911617", None, 910_736 * STEP_SECS),
            Ok(910_737)
        );
        assert_eq!(
            verify(&secret, "911617", Some(910_737), 910_738 * STEP_SECS),
            Ok(910_738)
        );
    }

    #[test]
    fn malformed_codes() {
        for code in [
            "",
            "12345",
            "1234567",
            "12345a",
            "+12345",
            "-12345",
            " 12345",
            "123 45",
            "１２３４５６",
        ] {
            assert_eq!(
                verify(&rfc_secret(), code, None, 59),
                Err(CodeRejection::Malformed),
                "{code:?}"
            );
        }
    }

    #[test]
    fn before_the_epoch_and_at_the_extremes() {
        // At -1 s only step 0 (the next) has a counter; at -31 s none does.
        let step_zero = code(&rfc_secret(), 0);
        assert_eq!(verify(&rfc_secret(), &step_zero, None, -1), Ok(0));
        assert_eq!(
            verify(&rfc_secret(), &step_zero, None, -31),
            Err(CodeRejection::Mismatch)
        );
        assert_eq!(
            verify(&rfc_secret(), "123456", None, i64::MIN),
            Err(CodeRejection::Mismatch)
        );
        // Step -1 has no counter: it must not wrap around to u64::MAX.
        assert_eq!(
            verify(
                &rfc_secret(),
                &code_for_step(&rfc_secret(), u64::MAX),
                None,
                -1
            ),
            Err(CodeRejection::Mismatch)
        );
        let last = step_at(i64::MAX);
        assert_eq!(
            verify(&rfc_secret(), &code(&rfc_secret(), last), None, i64::MAX),
            Ok(last)
        );
    }

    #[test]
    fn debug_hides_the_secret() {
        assert_eq!(format!("{:?}", rfc_secret()), "TotpSecret(<redacted>)");
    }

    fn any_secret() -> impl Strategy<Value = TotpSecret> {
        any::<[u8; SECRET_LEN]>().prop_map(TotpSecret::from_bytes)
    }

    proptest! {
        #[test]
        fn a_code_is_six_ascii_digits(secret in any_secret(), step in any::<u64>()) {
            let code = code_for_step(&secret, step);
            prop_assert_eq!(code.len(), DIGITS);
            prop_assert!(code.bytes().all(|byte| byte.is_ascii_digit()), "{}", code);
        }

        #[test]
        fn the_window_and_the_replay_rule_hold(
            secret in any_secret(),
            // From step 3 on, so every step within 3 of now has a code.
            now in 90_i64..=i64::MAX,
            offset in -3_i64..=3,
            last_offset in proptest::option::of(-3_i64..=3),
        ) {
            let current = step_at(now);
            let step = current + offset;
            let submitted = code(&secret, step);
            let last = last_offset.map(|delta| current + delta);
            let result = verify(&secret, &submitted, last, now);
            // Another step of the window shows the same digits only by
            // chance, about 3 times in a million.
            let collision = (current - SKEW_STEPS..=current + SKEW_STEPS)
                .any(|other| other != step && code(&secret, other) == submitted);
            if !collision {
                let expected = if offset.abs() > SKEW_STEPS {
                    Err(CodeRejection::Mismatch)
                } else if last.is_some_and(|last| step <= last) {
                    Err(CodeRejection::Replayed)
                } else {
                    Ok(step)
                };
                prop_assert_eq!(result, expected);
            }
            if let Ok(accepted) = result {
                prop_assert!((accepted - current).abs() <= SKEW_STEPS);
                prop_assert!(last.is_none_or(|last| accepted > last));
                prop_assert_eq!(code(&secret, accepted), submitted.as_str());
                // Once the step is stored, the same digits are a replay.
                prop_assert_eq!(
                    verify(&secret, &submitted, Some(accepted), now),
                    Err(CodeRejection::Replayed)
                );
            }
        }

        #[test]
        fn anything_but_six_ascii_digits_is_malformed(code in "\\PC{0,12}", now in any::<i64>()) {
            let well_formed = code.len() == DIGITS && code.bytes().all(|byte| byte.is_ascii_digit());
            let result = verify(&rfc_secret(), &code, None, now);
            prop_assert_eq!(result == Err(CodeRejection::Malformed), !well_formed);
        }

        #[test]
        fn step_at_is_the_floor_of_the_division(unix_secs in any::<i64>()) {
            let start = i128::from(step_at(unix_secs)) * i128::from(STEP_SECS);
            prop_assert!(start <= i128::from(unix_secs));
            prop_assert!(i128::from(unix_secs) < start + i128::from(STEP_SECS));
        }
    }
}
