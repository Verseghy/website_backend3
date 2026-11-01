//! The one format of backend3's configured keys: base64 of at least 64
//! random bytes (ADR 0023 §11, ADR 0006 §1 and §6).
//!
//! Two keys come in through the configuration in this format: the admin
//! session's JWT secret (`auth.jwt.secret`, `JWT_SECRET` in production) and
//! the TOTP encryption key (`settings.admin_auth.totp_encryption_key`,
//! `TOTP_ENCRYPTION_KEY`). Both are decoded here, with the rules that Loco's
//! JWT applies to its secret, so a key that boots is a key Loco can sign
//! with, and both are refused with the same messages when they are not.
//!
//! No message here contains the value or a character of it: a refused value
//! may be a wrongly pasted secret, and the messages reach the operator's
//! terminal and the pod log.
//!
//! Everything here is pure (ADR 0008 rule 1): the configured value is an
//! argument.

use base64::{DecodeError, Engine as _, engine::general_purpose::STANDARD};

/// The minimum decoded length of a configured key (ADR 0023 §11).
pub const CONFIGURED_KEY_MIN_LEN: usize = 64;

/// Why a configured key was refused. No message contains the value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The value is the empty string.
    #[error("is empty")]
    Empty,
    /// The value is not standard base64 with `=` padding.
    #[error(
        "is not valid base64 ({reason}); use the standard alphabet with `=` padding, on one line"
    )]
    NotBase64 {
        /// What the decoder found, without the offending character.
        reason: String,
    },
    /// The value decodes to fewer than [`CONFIGURED_KEY_MIN_LEN`] bytes.
    #[error("decodes to {len} bytes; it must be base64 of at least 64 random bytes")]
    TooShort {
        /// The decoded length.
        len: usize,
    },
}

/// Decodes a configured key and checks its length.
///
/// - `encoded`: the configured value, as the configuration holds it. It is
///   decoded with the engine that Loco's JWT uses for `JWT_SECRET`
///   (jsonwebtoken's `EncodingKey::from_base64_secret`: `STANDARD`,
///   canonical padding, no trailing bits; base64 0.23 here and 0.22 under
///   jsonwebtoken, which decode alike). Nothing is trimmed: a trailing
///   newline or a line break is refused, because Loco would refuse it too,
///   at the first sign-in instead of at boot.
///
/// # Errors
///
/// [`KeyError::Empty`], [`KeyError::NotBase64`] (whitespace and line breaks
/// included) or [`KeyError::TooShort`] for fewer than
/// [`CONFIGURED_KEY_MIN_LEN`] decoded bytes.
pub fn decode_configured_key(encoded: &str) -> Result<Vec<u8>, KeyError> {
    if encoded.is_empty() {
        return Err(KeyError::Empty);
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|error| KeyError::NotBase64 {
            reason: decode_error_reason(&error),
        })?;
    if decoded.len() < CONFIGURED_KEY_MIN_LEN {
        return Err(KeyError::TooShort { len: decoded.len() });
    }
    Ok(decoded)
}

/// Describes a decoding error without the offending character, which may
/// belong to a wrongly pasted secret.
///
/// - `error`: the decoder's error.
fn decode_error_reason(error: &DecodeError) -> String {
    match error {
        DecodeError::InvalidByte(offset, _) | DecodeError::InvalidLastSymbol { offset, .. } => {
            format!("an unexpected character at position {offset}")
        }
        DecodeError::InvalidLength(len) => format!("{len} characters is not a base64 length"),
        DecodeError::InvalidPadding => "missing or wrong `=` padding".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_key_of_64_bytes_or_more_decodes_to_its_bytes() {
        let bytes: Vec<u8> = (0..=63).collect();
        assert_eq!(decode_configured_key(&STANDARD.encode(&bytes)), Ok(bytes));
        let long = vec![0xa5_u8; 200];
        assert_eq!(decode_configured_key(&STANDARD.encode(&long)), Ok(long));
    }

    #[test]
    fn every_refusal_says_what_is_wrong() {
        let cases = [
            (String::new(), "is empty"),
            (STANDARD.encode([1_u8; 63]), "decodes to 63 bytes"),
            ("QUJDR".to_owned(), "5 characters is not a base64 length"),
            (
                "not base64, a pasted password".to_owned(),
                "an unexpected character at position 3",
            ),
            (
                format!("{}\n", STANDARD.encode([7_u8; 64])),
                "an unexpected character at position 88",
            ),
            // `STANDARD` requires canonical padding.
            (
                STANDARD.encode([7_u8; 64]).trim_end_matches('=').to_owned(),
                "missing or wrong `=` padding",
            ),
            // The URL-safe alphabet is not what Loco decodes.
            (
                STANDARD.encode([0xfb_u8; 66]).replace('+', "-"),
                "an unexpected character at position 0",
            ),
        ];
        for (value, problem) in cases {
            let message = decode_configured_key(&value)
                .expect_err("refused")
                .to_string();
            assert!(message.contains(problem), "{value:?}: {message}");
        }
    }

    #[test]
    fn no_message_quotes_the_value() {
        for value in ["Pasted-Secret-With-Dashes", "Pasted Secret", "x\u{7}y"] {
            let message = decode_configured_key(value)
                .expect_err("refused")
                .to_string();
            assert!(!message.contains(value), "{message}");
            assert!(
                !message.contains('-') && !message.contains('\u{7}'),
                "{message}"
            );
        }
    }

    #[test]
    fn trailing_bits_are_refused_as_loco_refuses_them() {
        // "QR==" decodes to 0x41 with non-zero trailing bits in canonical
        // base64 ("QQ==" is the canonical form): base64 0.23's STANDARD here
        // and 0.22's under jsonwebtoken both refuse it.
        let mut value = STANDARD.encode([0x41_u8; 64]);
        assert!(value.ends_with("QQ=="), "{value}");
        value.replace_range(value.len() - 3..value.len() - 2, "R");
        let message = decode_configured_key(&value)
            .expect_err("refused")
            .to_string();
        assert!(message.contains("at position 85"), "{message}");
    }

    proptest! {
        #[test]
        fn decoding_round_trips_exactly_from_64_bytes_on(
            bytes in proptest::collection::vec(any::<u8>(), 0..=160),
        ) {
            let decoded = decode_configured_key(&STANDARD.encode(&bytes));
            if bytes.is_empty() {
                prop_assert_eq!(decoded, Err(KeyError::Empty));
            } else if bytes.len() < CONFIGURED_KEY_MIN_LEN {
                prop_assert_eq!(decoded, Err(KeyError::TooShort { len: bytes.len() }));
            } else {
                prop_assert_eq!(decoded, Ok(bytes));
            }
        }

        #[test]
        fn arbitrary_text_never_panics_and_never_appears_in_the_message(
            value in "\\PC{12,80}",
        ) {
            if let Err(error) = decode_configured_key(&value) {
                prop_assert!(!error.to_string().contains(&value), "{}", error);
            }
        }
    }
}
