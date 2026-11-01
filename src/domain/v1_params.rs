//! The parameter rules of the public read API (public-api.md §2.5, §8).
//!
//! axum's `Path` and `Query` extractors answer a value they cannot
//! deserialize with their own `400 text/plain`, and Loco wraps only
//! JSON-body rejections (public-api.md §2.5). So a public handler takes its
//! path and query values as strings and checks them here, and the controller
//! turns every [`Rejection`] into the JSON error of §8
//! (`crate::dtos::v1::error::Error`). The checks need no database, and §8
//! runs them before any query: no value a client sends reaches PostgreSQL
//! unchecked, and none can make it fail (a `text` column cannot even hold
//! U+0000).
//!
//! What each rule of §2.1 and §2.5 becomes here:
//!
//! - **Query decoding** ([`Query::parse`]): the query string is read by the
//!   `application/x-www-form-urlencoded` rules that browsers write it with,
//!   the parser of the WHATWG URL Standard, which the `url` crate implements:
//!   pairs are separated by `&` only, `+` is a space, `%2B` is `+`, `%23` is
//!   `#`, names are decoded like values, and bytes that are not UTF-8 after
//!   percent-decoding become U+FFFD, as in a browser. A parameter without
//!   `=` (`?token`) is present with an empty value.
//! - **Repeated and unknown parameters** ([`Query::first`]): the first value
//!   of a name wins; later values and unknown names are ignored. Names are
//!   compared exactly, so `LIMIT` is an unknown parameter.
//! - **`{id}`** ([`id`]): ASCII digits only, no sign and no spaces, with the
//!   value 1 to [`MAX_ID`] (2^53 − 1, so that the id stays exact as a JSON
//!   number, ADR 0005); leading zeros are accepted, `007` is 7. Otherwise
//!   [`Rejection::Id`] (`400 invalid_id`).
//! - **`limit`** ([`limit`]): absent means [`Limit::DEFAULT`]; present, it
//!   must be ASCII digits with the value 1 to [`Limit::MAX`], the cap that
//!   ADR 0023 §4 makes a hardening rule (parity PLT-38). `limit=` and
//!   `limit=+5` are not digits. Otherwise [`Rejection::Limit`]
//!   (`400 invalid_limit`).
//! - **`token`** ([`token`]): any string of at most [`MAX_TOKEN_CHARS`]
//!   characters without U+0000. Absent means "no preview"; present, even
//!   empty, means a preview (public-api.md §7.3). A longer token or one with
//!   U+0000 can match no stored token, so it is [`Rejection::Token`]
//!   (`404 not_found` without a database query).
//! - **The check order** (§8): when several parameters are wrong, the first
//!   failing check in this order wins: the path parameters, then `limit`,
//!   `before`, `featured` and `q`. [`Rejection`]'s variants are declared in
//!   that order, so its `Ord` is the check order and
//!   [`Rejection::CHECK_ORDER`] lists it. A handler checks its parameters in
//!   that order and returns the first rejection (`?`), so the answer does not
//!   depend on how the client ordered the query string. `token` is not in
//!   §8's list: its refusal is a `404`, which the endpoint would also answer
//!   for a token that matches nothing, so it comes after every `400` check.
//!
//! A path value that axum cannot percent-decode into UTF-8 never reaches
//! these functions: axum's `Path` refuses it first. A handler that takes
//! `Result<Path<String>, PathRejection>` answers that refusal with the
//! parameter's rejection, because such a value is not digits either.
//!
//! The other rows of §2.5 (`{year}`, `{month}`, `{isoYear}`, `{isoWeek}`,
//! `{slug}`, `before`, `featured` and `q`) join this module with the endpoint
//! slices that read them, each as a [`Rejection`] variant at its place in the
//! check order.

use std::borrow::Cow;

use url::form_urlencoded;

/// The largest `{id}`: 2^53 − 1, the largest integer that every JSON client
/// reads exactly (public-api.md §2.5, ADR 0005 §3).
pub const MAX_ID: i64 = 9_007_199_254_740_991;

/// The most characters (Unicode scalar values) a `token` may have.
pub const MAX_TOKEN_CHARS: usize = 255;

/// The query parameter that sets the page size of a paged list.
pub const LIMIT_PARAM: &str = "limit";

/// The query parameter that carries a preview token.
pub const TOKEN_PARAM: &str = "token";

/// A parameter that failed its check (public-api.md §2.5), one variant per
/// parameter.
///
/// The variants are declared in the check order of public-api.md §8, so the
/// derived `Ord` is that order: when several parameters are wrong, the
/// smallest rejection is the one to answer. A new parameter is added at its
/// place in that order and to [`Self::CHECK_ORDER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rejection {
    /// A path `{id}` that is not digits or is outside 1 to [`MAX_ID`]:
    /// `400 invalid_id`.
    Id,
    /// A `limit` that is not digits or is outside 1 to [`Limit::MAX`]:
    /// `400 invalid_limit`.
    Limit,
    /// A `token` longer than [`MAX_TOKEN_CHARS`] or with U+0000, which no
    /// stored token can equal: `404 not_found`, without a database query.
    Token,
}

impl Rejection {
    /// Every rejection, in the check order of public-api.md §8.
    pub const CHECK_ORDER: [Self; 3] = [Self::Id, Self::Limit, Self::Token];

    /// The name of the parameter, as the path template or the query string
    /// writes it, for log lines and error descriptions.
    #[must_use]
    pub const fn parameter(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Limit => LIMIT_PARAM,
            Self::Token => TOKEN_PARAM,
        }
    }
}

/// The decoded query string of one request (public-api.md §2.1).
///
/// It keeps every pair in the order the client sent them, so that
/// [`Query::first`] can answer the first value of a name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query<'a> {
    /// The decoded `(name, value)` pairs, in request order.
    pairs: Vec<(Cow<'a, str>, Cow<'a, str>)>,
}

impl<'a> Query<'a> {
    /// Decodes a raw query string by the `application/x-www-form-urlencoded`
    /// rules (the module documentation lists them).
    ///
    /// - `raw`: the query string as the request carried it, without the
    ///   leading `?` and still percent-encoded (axum's `RawQuery`), or `None`
    ///   when the request had no query.
    #[must_use]
    pub fn parse(raw: Option<&'a str>) -> Self {
        Self {
            pairs: raw
                .map(|raw| form_urlencoded::parse(raw.as_bytes()).collect())
                .unwrap_or_default(),
        }
    }

    /// The decoded value of the first parameter called `name`, or `None` when
    /// the query has no such parameter. Later parameters of the same name are
    /// ignored (public-api.md §2.1).
    ///
    /// - `name`: the decoded parameter name, compared exactly.
    #[must_use]
    pub fn first(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_ref())
    }
}

/// The page size of a paged list: 1 to [`Limit::MAX`] items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Limit(u8);

impl Limit {
    /// The page size when the request sets none (public-api.md §10).
    pub const DEFAULT: Self = Self(20);

    /// The largest page size a request may ask for (public-api.md §10, ADR
    /// 0023 §4).
    pub const MAX: Self = Self(100);

    /// The number of items, for the query's `LIMIT`.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Checks a path `{id}`.
///
/// - `raw`: the path segment as axum decoded it.
///
/// # Errors
///
/// [`Rejection::Id`] unless `raw` is ASCII digits with a value from 1 to
/// [`MAX_ID`].
pub fn id(raw: &str) -> Result<i64, Rejection> {
    decimal_in(raw, 1, MAX_ID.unsigned_abs())
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(Rejection::Id)
}

/// Checks the `limit` query parameter.
///
/// - `raw`: its first value ([`Query::first`] of [`LIMIT_PARAM`]), or `None`
///   when the request has none.
///
/// # Errors
///
/// [`Rejection::Limit`] when a value is given and it is not ASCII digits with
/// a value from 1 to [`Limit::MAX`].
pub fn limit(raw: Option<&str>) -> Result<Limit, Rejection> {
    let Some(raw) = raw else {
        return Ok(Limit::DEFAULT);
    };
    decimal_in(raw, 1, u64::from(Limit::MAX.get()))
        .and_then(|value| u8::try_from(value).ok())
        .map(Limit)
        .ok_or(Rejection::Limit)
}

/// Checks the `token` query parameter of a preview (public-api.md §7.3).
///
/// - `raw`: its first value ([`Query::first`] of [`TOKEN_PARAM`]), or `None`
///   when the request has none.
///
/// Returns `None` without a token, and the token itself otherwise; an empty
/// token is a token, which matches nothing.
///
/// # Errors
///
/// [`Rejection::Token`] when the token has more than [`MAX_TOKEN_CHARS`]
/// characters or contains U+0000.
pub fn token(raw: Option<&str>) -> Result<Option<&str>, Rejection> {
    match raw {
        None => Ok(None),
        // `nth` stops after the 256th character, so a long token is not
        // counted to its end.
        Some(raw) if raw.contains('\0') || raw.chars().nth(MAX_TOKEN_CHARS).is_some() => {
            Err(Rejection::Token)
        }
        Some(raw) => Ok(Some(raw)),
    }
}

/// The value of `raw` when it is ASCII digits only and the value lies in
/// `min..=max`; `None` otherwise.
///
/// Rust's integer parsing accepts a leading `+` (public-api.md §2.5), so the
/// digits are checked first. Leading zeros are dropped before the length
/// check, so `0007` is 7 however many zeros it has, while a value too large
/// for `u64` is out of every range without being parsed.
fn decimal_in(raw: &str, min: u64, max: u64) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let significant = raw.trim_start_matches('0');
    // `u64::MAX` has 20 digits; anything longer is above every range.
    if significant.len() > 20 {
        return None;
    }
    let value = if significant.is_empty() {
        0
    } else {
        significant.parse::<u64>().ok()?
    };
    (min..=max).contains(&value).then_some(value)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn ids_are_digits_from_one_to_two_to_the_53_minus_one() {
        for (raw, expected) in [
            ("1", 1),
            ("7", 7),
            ("600", 600),
            // Leading zeros are accepted (public-api.md §2.5).
            ("007", 7),
            ("0000000000000000000000000000000000000001", 1),
            ("9007199254740991", MAX_ID),
            ("09007199254740991", MAX_ID),
        ] {
            assert_eq!(id(raw), Ok(expected), "{raw:?}");
        }
        assert_eq!(MAX_ID, (1_i64 << 53) - 1);
    }

    #[test]
    fn other_ids_are_invalid() {
        for raw in [
            "",
            "0",
            "000",
            "9007199254740992",
            "18446744073709551616",
            "99999999999999999999999999999999",
            "+5",
            "-5",
            " 5",
            "5 ",
            "5a",
            "1e3",
            "0x10",
            "1.0",
            "1_000",
            // Digits outside ASCII: fullwidth one and Arabic-Indic three.
            "\u{ff11}",
            "\u{663}",
            "5\0",
            "%35",
        ] {
            assert_eq!(id(raw), Err(Rejection::Id), "{raw:?}");
        }
    }

    #[test]
    fn an_absent_limit_is_twenty() {
        assert_eq!(limit(None), Ok(Limit::DEFAULT));
        assert_eq!(Limit::DEFAULT.get(), 20);
    }

    #[test]
    fn limits_are_digits_from_one_to_a_hundred() {
        for (raw, expected) in [("1", 1), ("2", 2), ("20", 20), ("100", 100), ("0100", 100)] {
            assert_eq!(limit(Some(raw)).map(Limit::get), Ok(expected), "{raw:?}");
        }
        assert_eq!(Limit::MAX.get(), 100);
    }

    #[test]
    fn other_limits_are_invalid() {
        // `0`, `101` and `+5` are the cases public-api.md §7.1 names.
        for raw in [
            "",
            "0",
            "00",
            "101",
            "1000",
            "+5",
            "-1",
            " 5",
            "5 ",
            "1.5",
            "ten",
            "\u{ff15}",
            "99999999999999999999999",
        ] {
            assert_eq!(limit(Some(raw)), Err(Rejection::Limit), "{raw:?}");
        }
    }

    #[test]
    fn tokens_up_to_255_characters_without_nul_are_kept() {
        assert_eq!(token(None), Ok(None));
        // public-api.md §7.3: an empty token is present and matches nothing.
        assert_eq!(token(Some("")), Ok(Some("")));
        // The legacy shape: 80 characters of [A-Za-z0-9] (§7.3).
        let legacy = "aZ09".repeat(20);
        assert_eq!(token(Some(&legacy)), Ok(Some(legacy.as_str())));
        let longest = "t".repeat(MAX_TOKEN_CHARS);
        assert_eq!(token(Some(&longest)), Ok(Some(longest.as_str())));
        // Characters, not bytes: 255 two-byte characters are 510 bytes.
        let accented = "é".repeat(MAX_TOKEN_CHARS);
        assert_eq!(token(Some(&accented)), Ok(Some(accented.as_str())));
        // A space is kept as it is: the decoded `+` of a token that a hop
        // turned into a space then matches nothing (§7.3).
        assert_eq!(token(Some("a b")), Ok(Some("a b")));
    }

    #[test]
    fn longer_tokens_and_tokens_with_nul_are_not_found() {
        let too_long = "t".repeat(MAX_TOKEN_CHARS + 1);
        let accented = "é".repeat(MAX_TOKEN_CHARS + 1);
        for raw in [too_long.as_str(), accented.as_str(), "\0", "abc\0", "\0abc"] {
            assert_eq!(token(Some(raw)), Err(Rejection::Token), "{raw:?}");
        }
    }

    #[test]
    fn plus_is_a_space_and_escapes_are_decoded() {
        // public-api.md §2.1: `+` is a space, `%2B` is `+`, `%23` is `#`.
        let query = Query::parse(Some("q=a+b&plus=%2B&hash=%23&mixed=1%2B1+%3D+2"));
        assert_eq!(query.first("q"), Some("a b"));
        assert_eq!(query.first("plus"), Some("+"));
        assert_eq!(query.first("hash"), Some("#"));
        assert_eq!(query.first("mixed"), Some("1+1 = 2"));
        // UTF-8 and the NUL of `%00` come through decoded, for the checks.
        let query = Query::parse(Some("q=%C3%BCnnep&token=a%00b"));
        assert_eq!(query.first("q"), Some("ünnep"));
        assert_eq!(query.first("token"), Some("a\0b"));
        // Bytes that are not UTF-8 become U+FFFD, as in a browser.
        assert_eq!(Query::parse(Some("q=%FF")).first("q"), Some("\u{fffd}"));
    }

    #[test]
    fn names_are_decoded_too() {
        let query = Query::parse(Some("%6Cimit=5&a+b=c"));
        assert_eq!(query.first(LIMIT_PARAM), Some("5"));
        assert_eq!(query.first("a b"), Some("c"));
    }

    #[test]
    fn the_first_of_a_repeated_parameter_wins() {
        let query = Query::parse(Some("limit=5&limit=7&token=a&token=b"));
        assert_eq!(query.first(LIMIT_PARAM), Some("5"));
        assert_eq!(query.first(TOKEN_PARAM), Some("a"));
        // A first value that is invalid still wins over a later valid one.
        let query = Query::parse(Some("limit=0&limit=5"));
        assert_eq!(limit(query.first(LIMIT_PARAM)), Err(Rejection::Limit));
    }

    #[test]
    fn missing_and_unknown_parameters_are_absent() {
        for raw in [
            None,
            Some(""),
            Some("&"),
            Some("&&"),
            Some("other=1&LIMIT=5"),
        ] {
            let query = Query::parse(raw);
            assert_eq!(query.first(LIMIT_PARAM), None, "{raw:?}");
            assert_eq!(limit(query.first(LIMIT_PARAM)), Ok(Limit::DEFAULT));
        }
        assert_eq!(Query::parse(None), Query::default());
    }

    #[test]
    fn a_parameter_without_a_value_is_present_and_empty() {
        let query = Query::parse(Some("token&limit="));
        assert_eq!(query.first(TOKEN_PARAM), Some(""));
        assert_eq!(token(query.first(TOKEN_PARAM)), Ok(Some("")));
        assert_eq!(query.first(LIMIT_PARAM), Some(""));
        assert_eq!(limit(query.first(LIMIT_PARAM)), Err(Rejection::Limit));
    }

    #[test]
    fn only_ampersands_separate_pairs() {
        // The WHATWG parser does not split on `;`, unlike some servers.
        let query = Query::parse(Some("limit=5;limit=7&&=x&token==a"));
        assert_eq!(query.first(LIMIT_PARAM), Some("5;limit=7"));
        assert_eq!(query.first(""), Some("x"));
        // Only the first `=` separates the name from the value.
        assert_eq!(query.first(TOKEN_PARAM), Some("=a"));
    }

    #[test]
    fn rejections_follow_the_check_order_of_section_8() {
        // public-api.md §8: path parameters, then `limit`, `before`,
        // `featured` and `q`; the `404` of a token comes after every `400`.
        assert_eq!(
            Rejection::CHECK_ORDER,
            [Rejection::Id, Rejection::Limit, Rejection::Token]
        );
        assert!(
            Rejection::CHECK_ORDER
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        // Every variant is listed at its place: this match stops compiling
        // when a variant is added, until this test, and so CHECK_ORDER, name
        // it too.
        let place = |rejection: Rejection| match rejection {
            Rejection::Id => 0,
            Rejection::Limit => 1,
            Rejection::Token => 2,
        };
        for (index, rejection) in Rejection::CHECK_ORDER.into_iter().enumerate() {
            assert_eq!(place(rejection), index, "{rejection:?}");
        }
        // The smallest rejection is the one to answer.
        let all_wrong = [Rejection::Token, Rejection::Limit, Rejection::Id];
        assert_eq!(all_wrong.iter().min(), Some(&Rejection::Id));
    }

    #[test]
    fn a_path_parameter_is_reported_before_the_query() {
        // `/api/v1/authors/abc/posts?limit=0&token=<NUL>` has three wrong
        // parameters: checked in order, the id is the one reported.
        let query = Query::parse(Some("token=%00&limit=0"));
        let checked = id("abc")
            .and_then(|_| limit(query.first(LIMIT_PARAM)))
            .and_then(|_| token(query.first(TOKEN_PARAM)));
        assert_eq!(checked.map(|_| ()), Err(Rejection::Id));
        let checked = id("1")
            .and_then(|_| limit(query.first(LIMIT_PARAM)))
            .and_then(|_| token(query.first(TOKEN_PARAM)));
        assert_eq!(checked.map(|_| ()), Err(Rejection::Limit));
    }

    #[test]
    fn rejections_name_their_parameter() {
        assert_eq!(Rejection::Id.parameter(), "id");
        assert_eq!(Rejection::Limit.parameter(), "limit");
        assert_eq!(Rejection::Token.parameter(), "token");
    }

    /// The reference reading of a digit parameter: the value of `raw` when it
    /// is non-empty ASCII digits, as a `u128` (saturated), or `None`.
    fn reference_value(raw: &str) -> Option<u128> {
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some(raw.bytes().fold(0_u128, |value, digit| {
            value
                .saturating_mul(10)
                .saturating_add(u128::from(digit - b'0'))
        }))
    }

    /// Strings that look like parameter values: digits with signs, spaces and
    /// other characters mixed in, and any text at all.
    fn parameter_values() -> impl Strategy<Value = String> {
        prop_oneof![
            "[0-9]{0,25}",
            "[0-9+ \\-.a-z\u{ff10}-\u{ff19}]{0,12}",
            "0{0,30}[1-9][0-9]{0,18}",
            any::<String>(),
        ]
    }

    proptest! {
        #[test]
        fn no_value_makes_a_check_panic(raw in any::<String>(), name in any::<String>()) {
            let query = Query::parse(Some(&raw));
            let _ = query.first(&name);
            let _ = id(&raw);
            let _ = limit(Some(&raw));
            let _ = token(Some(&raw));
        }

        #[test]
        fn an_id_is_accepted_exactly_when_it_is_digits_in_range(raw in parameter_values()) {
            let expected = reference_value(&raw)
                .filter(|value| (1..=u128::from(MAX_ID.unsigned_abs())).contains(value));
            prop_assert_eq!(
                id(&raw).ok().map(|value| u128::from(value.unsigned_abs())),
                expected,
                "{:?}",
                raw
            );
        }

        #[test]
        fn every_id_in_range_is_read_back_with_any_leading_zeros(
            value in 1..=MAX_ID,
            zeros in 0_usize..40,
        ) {
            let raw = format!("{}{value}", "0".repeat(zeros));
            prop_assert_eq!(id(&raw), Ok(value));
        }

        #[test]
        fn a_limit_is_accepted_exactly_when_it_is_digits_from_1_to_100(raw in parameter_values()) {
            let expected = reference_value(&raw).filter(|value| (1..=100).contains(value));
            let result = limit(Some(&raw));
            prop_assert_eq!(result.ok().map(|limit| u128::from(limit.get())), expected, "{:?}", raw);
            if let Ok(limit) = result {
                prop_assert!((1..=Limit::MAX.get()).contains(&limit.get()));
            }
        }

        #[test]
        fn a_token_is_kept_exactly_when_it_is_short_and_has_no_nul(raw in any::<String>()) {
            let expected = raw.chars().count() <= MAX_TOKEN_CHARS && !raw.contains('\0');
            match token(Some(&raw)) {
                Ok(kept) => {
                    prop_assert!(expected);
                    prop_assert_eq!(kept, Some(raw.as_str()));
                }
                Err(rejection) => {
                    prop_assert!(!expected);
                    prop_assert_eq!(rejection, Rejection::Token);
                }
            }
        }

        #[test]
        fn a_long_token_is_never_kept(extra in 1_usize..64, character in any::<char>()) {
            let raw: String = std::iter::repeat_n(character, MAX_TOKEN_CHARS + extra).collect();
            prop_assert_eq!(token(Some(&raw)), Err(Rejection::Token));
        }

        #[test]
        fn encoded_pairs_decode_back_and_the_first_of_a_name_wins(
            pairs in proptest::collection::vec(("[a-c+& =%#]{0,3}", any::<String>()), 0..8),
        ) {
            // The encoder browsers use for forms: a space becomes `+`, every
            // other special byte `%XX`.
            let raw = form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&pairs)
                .finish();
            let query = Query::parse(Some(&raw));
            for (name, _) in &pairs {
                let first = pairs
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.as_str());
                prop_assert_eq!(query.first(name), first, "{:?}", raw);
            }
        }
    }
}
