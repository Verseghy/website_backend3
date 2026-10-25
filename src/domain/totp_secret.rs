//! The TOTP secret at rest, `users.totp_secret_encrypted` (ADR 0006 §6), so
//! that a database dump alone does not reveal TOTP secrets.
//!
//! - AES-256-GCM with a 96-bit nonce that is fresh and random for every
//!   encryption, and the account's `users.id` (8 bytes, big-endian) as
//!   associated data: the same bytes copied into another account's row do
//!   not decrypt there.
//! - The stored value is nonce, ciphertext and tag: 12 + 20 + 16 = 48 bytes.
//! - The cipher key is HKDF-SHA-256 (RFC 5869, no salt, info [`KEY_INFO`])
//!   of the configured key, base64 of at least 64 random bytes. HKDF keeps
//!   this key apart from any other use of the configured secret, and the
//!   `v1` in the info names the derivation for a later rotation.
//! - The local environments share one public throwaway key,
//!   [`PUBLISHED_LOCAL_KEY`], whose decoded text says so (ADR 0034 §2,
//!   item 3); production refuses it at boot (ADR 0034 §3).
//!
//! Everything here is pure (ADR 0008 rule 1): the configured key arrives as
//! the string the settings hold, and the nonce is an argument.
//!
//! Key and secret bytes are not zeroed when they are dropped, and ADR 0006
//! does not ask for it. It would protect little: the arrays are `Copy`, so
//! every move leaves a copy that no `Drop` reaches, and Loco keeps the
//! configured key as text in its `Config` for the life of the process.

use std::fmt;

use aes_gcm::{
    Aes256Gcm, Key, KeyInit as _, Nonce, Tag,
    aead::{AeadInOut as _, inout::InOutBuf},
};
use base64::{DecodeError, Engine as _, engine::general_purpose::STANDARD};
use hkdf::Hkdf;
use sha2::Sha256;

use super::totp::{SECRET_LEN, TotpSecret};

/// The HKDF info string of the cipher key. Changing it changes the key, so
/// every stored secret would need re-encrypting.
pub const KEY_INFO: &[u8] = b"backend3 totp-secret v1";

/// The minimum decoded length of the configured key (ADR 0023 §11).
pub const CONFIGURED_KEY_MIN_LEN: usize = 64;

/// The length of the GCM nonce.
pub const NONCE_LEN: usize = 12;

/// The length of the GCM tag.
pub const TAG_LEN: usize = 16;

/// The length of a stored value: nonce, ciphertext and tag.
pub const ENCRYPTED_LEN: usize = NONCE_LEN + SECRET_LEN + TAG_LEN;

/// What [`PUBLISHED_LOCAL_KEY`] decodes to: a sentence that says the key is
/// public and throwaway, so whoever decodes it cannot mistake it for a
/// secret. At 110 bytes it meets [`CONFIGURED_KEY_MIN_LEN`].
pub const PUBLISHED_LOCAL_KEY_TEXT: &[u8] = b"backend3 TOTP encryption key: public and throwaway, \
for development, test and e2e only; production refuses it.";

/// The configured key of `config/development.yaml`, `config/test.yaml` and
/// `config/e2e.yaml`: the base64 of [`PUBLISHED_LOCAL_KEY_TEXT`].
///
/// It is public, like the compose database's `loco`/`loco`, and protects
/// nothing; a fixed value keeps local runs and tests reproducible. The
/// local environments may use it: it is a fixed throwaway key whose own
/// text says that it is public and throwaway, which ADR 0034 §2 item 3
/// allows
/// (`docs/design/adr/0034-test-hashes-and-throwaway-keys-refused-in-production.md`).
/// Production refuses it at boot
/// ([`TotpEncryptionKey::is_published_local_key`], run by
/// `Settings::refuse_published_keys` in `src/settings.rs`; ADR 0034 §3).
pub const PUBLISHED_LOCAL_KEY: &str = "YmFja2VuZDMgVE9UUCBlbmNyeXB0aW9uIGtleTogcHVibGljIGFuZCB0aHJvd2F3YXksIGZvciBkZXZlbG9wbWVudCwgdGVzdCBhbmQgZTJlIG9ubHk7IHByb2R1Y3Rpb24gcmVmdXNlcyBpdC4=";

/// The derived AES-256 key, made once at boot from the configured key and
/// passed to the code that encrypts or decrypts (ADR 0008 rule 2). `Debug`
/// hides the bytes, and equality looks at every byte, so its time does not
/// depend on where two keys differ.
#[derive(Clone)]
pub struct TotpEncryptionKey([u8; 32]);

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

/// Why a stored value did not decrypt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecryptError {
    /// The value is not [`ENCRYPTED_LEN`] bytes long.
    #[error("an encrypted TOTP secret is 48 bytes, this one is {len}")]
    Length {
        /// The stored length.
        len: usize,
    },
    /// The tag does not verify: another account's value, another key, or a
    /// changed byte.
    #[error("the encrypted TOTP secret does not authenticate for this account and key")]
    Authentication,
}

impl TotpEncryptionKey {
    /// Decodes the configured key and derives the cipher key.
    ///
    /// - `encoded`: the configured value, which production reads from
    ///   `TOTP_ENCRYPTION_KEY`. It is decoded with the engine that Loco's
    ///   JWT uses for `JWT_SECRET` (base64 0.22, `STANDARD`, canonical
    ///   padding). Nothing is trimmed: a trailing newline is refused.
    ///
    /// # Errors
    ///
    /// [`KeyError::Empty`], [`KeyError::NotBase64`] (whitespace and line
    /// breaks included) or [`KeyError::TooShort`].
    pub fn from_base64(encoded: &str) -> Result<Self, KeyError> {
        if encoded.is_empty() {
            return Err(KeyError::Empty);
        }
        let configured = STANDARD
            .decode(encoded)
            .map_err(|error| KeyError::NotBase64 {
                reason: decode_error_reason(&error),
            })?;
        Self::derive(&configured)
    }

    /// Derives the cipher key from the decoded configured key with
    /// HKDF-SHA-256, no salt (RFC 5869 §2.2) and the info [`KEY_INFO`].
    ///
    /// - `configured`: the configured key's bytes.
    ///
    /// # Errors
    ///
    /// [`KeyError::TooShort`] for fewer than [`CONFIGURED_KEY_MIN_LEN`] bytes.
    pub fn derive(configured: &[u8]) -> Result<Self, KeyError> {
        if configured.len() < CONFIGURED_KEY_MIN_LEN {
            return Err(KeyError::TooShort {
                len: configured.len(),
            });
        }
        Ok(Self(hkdf_sha256(configured, KEY_INFO)))
    }

    /// Whether this key was derived from [`PUBLISHED_LOCAL_KEY`], the public
    /// throwaway key of the local environments, which production refuses
    /// (ADR 0034 §3).
    ///
    /// The derived keys are compared, not the configured strings, so the
    /// answer does not depend on how the same bytes were written. A key
    /// that differs from the published bytes in any byte, or is a prefix of
    /// them, is another key.
    #[must_use]
    pub fn is_published_local_key(&self) -> bool {
        // The published text is longer than the minimum, so `derive` cannot
        // refuse it; a unit test pins that.
        Self::derive(PUBLISHED_LOCAL_KEY_TEXT).is_ok_and(|published| *self == published)
    }
}

impl fmt::Debug for TotpEncryptionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TotpEncryptionKey(<redacted>)")
    }
}

impl PartialEq for TotpEncryptionKey {
    fn eq(&self, other: &Self) -> bool {
        // No request path compares keys: the typed settings derive equality
        // for their tests, and the boot check compares the configured key
        // with the public one once (`is_published_local_key`). Folding every
        // byte still avoids an early exit.
        let difference = self
            .0
            .iter()
            .zip(&other.0)
            .fold(0_u8, |acc, (l, r)| acc | (l ^ r));
        difference == 0
    }
}

impl Eq for TotpEncryptionKey {}

/// The associated data of an account's secret: its `users.id`, big-endian.
///
/// - `user_id`: the account's `users.id`.
#[must_use]
pub const fn associated_data(user_id: i64) -> [u8; 8] {
    user_id.to_be_bytes()
}

/// Encrypts a TOTP secret for the `users.totp_secret_encrypted` of one row.
///
/// - `key`: the derived key from the settings.
/// - `user_id`: the `users.id` of the row the value is written to.
/// - `nonce`: 12 random bytes that the caller reads from the operating
///   system for every call. Never a counter, a constant or a reused value:
///   one nonce used twice under one key breaks GCM's confidentiality and
///   authenticity. Random nonces stay within GCM's bound for about 2^32
///   encryptions under one key (NIST SP 800-38D §8.3); backend3 makes one
///   per enrolment.
/// - `secret`: the secret to store.
///
/// # Panics
///
/// Never: AES-GCM refuses only inputs longer than its limits (about 64 GiB),
/// and these are 20 and 8 bytes.
#[must_use]
pub fn encrypt(
    key: &TotpEncryptionKey,
    user_id: i64,
    nonce: [u8; NONCE_LEN],
    secret: &TotpSecret,
) -> [u8; ENCRYPTED_LEN] {
    let mut ciphertext = *secret.as_bytes();
    let tag = Aes256Gcm::new(&Key::<Aes256Gcm>::from(key.0))
        .encrypt_inout_detached(
            &Nonce::from(nonce),
            &associated_data(user_id),
            InOutBuf::from(&mut ciphertext[..]),
        )
        .expect("20 bytes of plaintext and 8 of associated data are within the GCM limits");
    let mut stored = [0_u8; ENCRYPTED_LEN];
    stored[..NONCE_LEN].copy_from_slice(&nonce);
    stored[NONCE_LEN..NONCE_LEN + SECRET_LEN].copy_from_slice(&ciphertext);
    stored[NONCE_LEN + SECRET_LEN..].copy_from_slice(&tag);
    stored
}

/// Decrypts a row's stored TOTP secret.
///
/// - `key`: the derived key from the settings.
/// - `user_id`: the `users.id` of the row the value was read from.
/// - `stored`: the row's `users.totp_secret_encrypted`.
///
/// aes-gcm checks the tag in constant time before it decrypts anything, so
/// a refused value yields no plaintext.
///
/// # Errors
///
/// [`DecryptError::Length`] for a value that is not [`ENCRYPTED_LEN`] bytes,
/// [`DecryptError::Authentication`] when the tag does not verify.
pub fn decrypt(
    key: &TotpEncryptionKey,
    user_id: i64,
    stored: &[u8],
) -> Result<TotpSecret, DecryptError> {
    if stored.len() != ENCRYPTED_LEN {
        return Err(DecryptError::Length { len: stored.len() });
    }
    let mut nonce = [0_u8; NONCE_LEN];
    nonce.copy_from_slice(&stored[..NONCE_LEN]);
    let mut plaintext = [0_u8; SECRET_LEN];
    plaintext.copy_from_slice(&stored[NONCE_LEN..NONCE_LEN + SECRET_LEN]);
    let mut tag = [0_u8; TAG_LEN];
    tag.copy_from_slice(&stored[NONCE_LEN + SECRET_LEN..]);
    Aes256Gcm::new(&Key::<Aes256Gcm>::from(key.0))
        .decrypt_inout_detached(
            &Nonce::from(nonce),
            &associated_data(user_id),
            InOutBuf::from(&mut plaintext[..]),
            &Tag::from(tag),
        )
        .map_err(|_| DecryptError::Authentication)?;
    Ok(TotpSecret::from_bytes(plaintext))
}

/// HKDF-SHA-256 without salt, expanded to 32 bytes.
///
/// - `input_key`: the input keying material.
/// - `info`: the context string.
fn hkdf_sha256(input_key: &[u8], info: &[u8]) -> [u8; 32] {
    let mut output = [0_u8; 32];
    // `expand` refuses only more than 255 hash lengths (RFC 5869 §2.3).
    Hkdf::<Sha256>::new(None, input_key)
        .expand(info, &mut output)
        .expect("32 bytes are within HKDF-SHA-256's output limit");
    output
}

/// Describes a decoding error without the offending character, which may
/// belong to a wrongly pasted secret.
///
/// - `error`: the decoder's error.
fn decode_error_reason(error: &DecodeError) -> String {
    match error {
        DecodeError::InvalidByte(offset, _) | DecodeError::InvalidLastSymbol(offset, _) => {
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

    // The known answers were computed independently with Node.js's crypto
    // module (OpenSSL): hkdfSync("sha256", ikm, "", info, 32), then
    // createCipheriv("aes-256-gcm", key, nonce) with setAAD.

    /// The bytes 0, 1, ..., 63, a configured key of the minimum length.
    fn counting_bytes() -> Vec<u8> {
        (0..=63).collect()
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
    }

    fn test_key() -> TotpEncryptionKey {
        TotpEncryptionKey::derive(&counting_bytes()).expect("64 bytes")
    }

    fn test_secret() -> TotpSecret {
        TotpSecret::from_bytes(*b"12345678901234567890")
    }

    const NONCE: [u8; NONCE_LEN] = [
        0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab,
    ];

    /// `decrypt`'s result without the secret type, which has no
    /// `PartialEq`.
    fn decrypted(
        key: &TotpEncryptionKey,
        user_id: i64,
        stored: &[u8],
    ) -> Result<[u8; 20], DecryptError> {
        decrypt(key, user_id, stored).map(|secret| *secret.as_bytes())
    }

    #[test]
    fn hkdf_without_salt_matches_rfc_5869_test_case_3() {
        // SHA-256, 22 bytes of 0x0b, no salt, no info: the first 32 bytes.
        assert_eq!(
            hex(&hkdf_sha256(&[0x0b; 22], b"")),
            "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d"
        );
    }

    #[test]
    fn the_known_answers() {
        assert_eq!(
            hex(&test_key().0),
            "23b437e5835c5676e1760208e4e092c6527b5ca1ba3f550aa27ef341172ed99d"
        );
        let encoded = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+Pw==";
        assert_eq!(TotpEncryptionKey::from_base64(encoded), Ok(test_key()));
        let stored = encrypt(&test_key(), 42, NONCE, &test_secret());
        assert_eq!(
            hex(&stored),
            "a0a1a2a3a4a5a6a7a8a9aaab0407b2799f3cfa7fb73603ae8033c7beefa102b4\
             8950d5c73efe0d3d080f5981a83095ac"
        );
        assert_eq!(
            decrypted(&test_key(), 42, &stored),
            Ok(*b"12345678901234567890")
        );
    }

    #[test]
    fn another_row_another_key_or_a_changed_byte_does_not_decrypt() {
        let stored = encrypt(&test_key(), 42, NONCE, &test_secret());
        for other_row in [41, 43, 0, -42, 42 << 32] {
            assert_eq!(
                decrypted(&test_key(), other_row, &stored),
                Err(DecryptError::Authentication)
            );
        }
        let mut other = counting_bytes();
        other[63] ^= 1;
        let other = TotpEncryptionKey::derive(&other).expect("64 bytes");
        assert_ne!(other, test_key());
        assert_eq!(
            decrypted(&other, 42, &stored),
            Err(DecryptError::Authentication)
        );
        for index in 0..ENCRYPTED_LEN {
            let mut changed = stored;
            changed[index] ^= 0x01;
            assert_eq!(
                decrypted(&test_key(), 42, &changed),
                Err(DecryptError::Authentication),
                "{index}"
            );
        }
    }

    #[test]
    fn configured_key_refusals_name_the_problem_without_the_value() {
        let valid = STANDARD.encode(counting_bytes());
        let pasted = "pasted-password-pasted-password-pasted-password-pasted-password-pp";
        for (value, expected) in [
            (String::new(), "is empty"),
            (STANDARD.encode([7_u8; 63]), "decodes to 63 bytes"),
            (
                format!("{valid}\n"),
                "an unexpected character at position 88",
            ),
            (format!("{}\n{}", &valid[..64], &valid[64..]), "position 64"),
            (
                valid.trim_end_matches('=').to_owned(),
                "missing or wrong `=` padding",
            ),
            ("QUJDR".to_owned(), "5 characters is not a base64 length"),
            (valid.replace('+', "-"), "an unexpected character"),
            (pasted.to_owned(), "an unexpected character at position 6"),
        ] {
            let message = TotpEncryptionKey::from_base64(&value)
                .expect_err("refused")
                .to_string();
            assert!(message.contains(expected), "{value:?}: {message}");
            assert!(
                !message.contains("pasted") && !message.contains('-'),
                "{message}"
            );
        }
    }

    #[test]
    fn debug_and_errors_hide_the_key() {
        assert_eq!(format!("{:?}", test_key()), "TotpEncryptionKey(<redacted>)");
        assert_eq!(
            DecryptError::Length { len: 47 }.to_string(),
            "an encrypted TOTP secret is 48 bytes, this one is 47"
        );
        assert_eq!(
            associated_data(0x0102_0304_0506_0708),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn the_published_local_key_says_it_is_public_and_is_recognised() {
        assert_eq!(
            STANDARD.encode(PUBLISHED_LOCAL_KEY_TEXT),
            PUBLISHED_LOCAL_KEY
        );
        let text = std::str::from_utf8(PUBLISHED_LOCAL_KEY_TEXT).expect("readable text");
        assert!(
            text.contains("public and throwaway") && text.contains("production refuses it"),
            "{text}"
        );
        assert!(PUBLISHED_LOCAL_KEY_TEXT.len() >= CONFIGURED_KEY_MIN_LEN);
        let published = TotpEncryptionKey::from_base64(PUBLISHED_LOCAL_KEY).expect("valid");
        assert!(published.is_published_local_key());
        assert!(
            TotpEncryptionKey::derive(PUBLISHED_LOCAL_KEY_TEXT)
                .expect("valid")
                .is_published_local_key()
        );
    }

    #[test]
    fn every_other_key_is_not_the_published_one() {
        assert!(!test_key().is_published_local_key());
        let mut last_byte_changed = PUBLISHED_LOCAL_KEY_TEXT.to_vec();
        if let Some(last) = last_byte_changed.last_mut() {
            *last ^= 1;
        }
        let mut first_byte_changed = PUBLISHED_LOCAL_KEY_TEXT.to_vec();
        first_byte_changed[0] ^= 1;
        let mut longer = PUBLISHED_LOCAL_KEY_TEXT.to_vec();
        longer.push(b' ');
        for bytes in [
            last_byte_changed,
            first_byte_changed,
            longer,
            PUBLISHED_LOCAL_KEY_TEXT[..CONFIGURED_KEY_MIN_LEN].to_vec(),
            PUBLISHED_LOCAL_KEY_TEXT[1..].to_vec(),
        ] {
            let key = TotpEncryptionKey::derive(&bytes).expect("long enough");
            assert!(!key.is_published_local_key(), "{bytes:?}");
        }
    }

    fn any_key() -> impl Strategy<Value = TotpEncryptionKey> {
        proptest::collection::vec(any::<u8>(), CONFIGURED_KEY_MIN_LEN..=128)
            .prop_map(|bytes| TotpEncryptionKey::derive(&bytes).expect("long enough"))
    }

    proptest! {
        #[test]
        fn a_secret_decrypts_for_its_own_row_only(
            key in any_key(),
            user_id in any::<i64>(),
            other in any::<i64>(),
            nonce in any::<[u8; NONCE_LEN]>(),
            secret in any::<[u8; SECRET_LEN]>(),
        ) {
            let stored = encrypt(&key, user_id, nonce, &TotpSecret::from_bytes(secret));
            prop_assert_eq!(&stored[..NONCE_LEN], &nonce[..]);
            prop_assert_eq!(decrypted(&key, user_id, &stored), Ok(secret));
            if other != user_id {
                prop_assert_eq!(decrypted(&key, other, &stored), Err(DecryptError::Authentication));
            }
        }

        #[test]
        fn decrypt_refuses_every_other_length(
            key in any_key(),
            stored in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            prop_assume!(stored.len() != ENCRYPTED_LEN);
            prop_assert_eq!(decrypted(&key, 1, &stored), Err(DecryptError::Length { len: stored.len() }));
        }

        #[test]
        fn a_configured_key_needs_64_bytes(bytes in proptest::collection::vec(any::<u8>(), 1..=160)) {
            let result = TotpEncryptionKey::from_base64(&STANDARD.encode(&bytes));
            if bytes.len() >= CONFIGURED_KEY_MIN_LEN {
                prop_assert_eq!(result.map(|key| key.0), Ok(hkdf_sha256(&bytes, KEY_INFO)));
            } else {
                prop_assert_eq!(result, Err(KeyError::TooShort { len: bytes.len() }));
            }
        }

        #[test]
        fn any_string_gives_a_key_or_a_refusal_without_it(text in "\\PC{8,100}") {
            if let Err(error) = TotpEncryptionKey::from_base64(&text) {
                prop_assert!(!error.to_string().contains(text.as_str()));
            }
        }

        #[test]
        fn only_the_published_bytes_give_the_published_key(
            bytes in proptest::collection::vec(any::<u8>(), CONFIGURED_KEY_MIN_LEN..=160),
        ) {
            let key = TotpEncryptionKey::derive(&bytes).expect("long enough");
            prop_assert_eq!(key.is_published_local_key(), bytes == PUBLISHED_LOCAL_KEY_TEXT);
        }

        #[test]
        fn the_associated_data_is_the_big_endian_id(user_id in any::<i64>()) {
            prop_assert_eq!(associated_data(user_id), user_id.to_be_bytes());
        }
    }
}
