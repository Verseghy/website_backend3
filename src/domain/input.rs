//! The input rules that every admin write shares (the write contract;
//! parity SHR-09, SHR-13; ADR 0005, ADR 0009 "Column types").
//!
//! An admin create or update reads its request DTO, then hands each field's
//! raw text to the functions here, which give the value to store or say
//! which field broke which rule ([`FieldError`]). The resource's domain
//! module (`super::labels` for labels) names its fields and limits and calls
//! them; the DTO layer turns a [`FieldError`] into the `422` envelope of
//! ADR 0021 §4, with the field and the rule's `params`.
//!
//! The rules, the same for every resource:
//!
//! - **Text is trimmed as the admin trims it** (SHR-09): the code points at
//!   either end that ECMAScript's `String.prototype.trim` removes are never
//!   stored ([`trim`], [`is_trimmed`]). backend1 trimmed every input before
//!   it saved it. The admin checks its fields in TypeScript, where trimming
//!   is that `trim`, as the editor's alt text check already does
//!   (ADR 0013 §2), so the server removes the same code points and the two
//!   never disagree on whether a field is empty. Rust's [`str::trim`]
//!   (Unicode `White_Space`) is not used: it removes U+0085, which the
//!   admin keeps, and keeps U+FEFF, the byte-order mark, which the admin
//!   removes. With it a name pasted with a byte-order mark at an end would
//!   be stored with it, a second label that looks like the first, and the
//!   live-name index, which compares the stored text, would let it in.
//! - **Empty optional text is NULL** (SHR-09): an optional text field that is
//!   absent, empty or only white space is stored as NULL, never as `''`, so
//!   "no value" has one form in the database ([`optional_text`]).
//! - **Every column limit is checked here first** (SHR-13): a value that the
//!   database would refuse is refused with a field error before any
//!   statement runs, so the editor sees a message at the input and never a
//!   server error. The length limits of ADR 0009 live in domain validation,
//!   not in the schema, and are counted in characters (Unicode scalar
//!   values), as PostgreSQL's `length()` counts them; a colour is checked
//!   against the `#rrggbb` form of its CHECK ([`is_color`]); and the one
//!   character a PostgreSQL `text` value cannot hold, U+0000, is refused in
//!   every text field, where the database would fail the whole statement
//!   ([`NUL`]).
//!
//! Each function checks one field and returns the first rule it breaks, in a
//! fixed order (required, then the characters, then the length), so the same
//! input always gets the same answer. A resource checks its fields in the
//! order its form shows them and stops at the first refusal: the envelope of
//! ADR 0021 §4 names one field.

use std::fmt;

/// The one character that a PostgreSQL `text` value cannot hold: a value
/// with it fails the whole statement ("invalid byte sequence for encoding
/// UTF8: 0x00"), so every text field refuses it before the write.
pub const NUL: char = '\0';

/// A rule that a field of an admin write broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldRule {
    /// The field is required, but it is empty or only white space once
    /// trimmed.
    Required,
    /// The trimmed text is longer than `max` characters.
    TooLong {
        /// The most characters the field takes.
        max: usize,
    },
    /// The text holds [`NUL`], which the database cannot store.
    InvalidCharacter,
    /// The colour is not `#rrggbb` ([`is_color`]).
    InvalidColor,
}

impl fmt::Display for FieldRule {
    /// The rule as the end of an English sentence that starts with the
    /// field's name, for the envelope's `description`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Required => {
                formatter.write_str("is required, but it is empty or only white space")
            }
            Self::TooLong { max } => {
                write!(formatter, "is longer than {max} characters once trimmed")
            }
            Self::InvalidCharacter => {
                formatter.write_str("holds the character U+0000, which cannot be stored")
            }
            Self::InvalidColor => formatter.write_str("is not a colour of the form #rrggbb"),
        }
    }
}

/// A field of an admin write that its validation refused: which field, and
/// which rule it broke.
///
/// `field` is the field's name as the request DTO spells it, because the
/// envelope's `field` is that spelling (ADR 0021 §4): the resource's domain
/// module names it once (`super::labels::NAME_FIELD`), and the DTO's tests
/// check that the DTO reads a member of that name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{field} {rule}")]
pub struct FieldError {
    /// The refused field, as the request DTO spells it.
    pub field: &'static str,
    /// The rule it broke.
    pub rule: FieldRule,
}

impl FieldError {
    /// The refusal of `field` for `rule`.
    ///
    /// - `field`: the field, as the request DTO spells it.
    /// - `rule`: the rule it broke.
    #[must_use]
    pub const fn new(field: &'static str, rule: FieldRule) -> Self {
        Self { field, rule }
    }
}

/// Whether ECMAScript's `String.prototype.trim` removes `c`: its
/// `WhiteSpace` and `LineTerminator` code points (ECMA-262, "White Space"
/// and "Line Terminators"; the `Zs` category as of Unicode 16).
///
/// The admin trims in TypeScript, where trimming is that `trim`: the
/// editor's alt text check (ADR 0013 §2) and the fields of its forms. So
/// the server trims the same code points, in every admin write ([`trim`])
/// and in the rich-text validator's alt text check
/// (`super::rich_text::validate::is_blank`). Rust's `char::is_whitespace`
/// differs by two: U+0085, which it trims and `trim` does not, and U+FEFF,
/// the other way round. With Rust's set, a value the admin accepted could
/// be refused here, or one it blocked accepted.
///
/// - `c`: the character.
#[must_use]
pub const fn is_trimmed(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// `text` without the code points at either end that ECMAScript's
/// `String.prototype.trim` removes ([`is_trimmed`]): the trim of every
/// admin input, as the admin trims it.
///
/// - `text`: the text to trim.
#[must_use]
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_trimmed)
}

/// The length of `text` in characters (Unicode scalar values): the unit of
/// every length limit, the one PostgreSQL's `length()` counts in a UTF-8
/// database.
///
/// - `text`: the text to measure.
///
/// The admin counts in the same unit for its own hints; JavaScript's
/// `String.length` counts UTF-16 code units, which differ for characters
/// outside the Basic Multilingual Plane (an emoji is one character here and
/// two code units there), so the server's count is the one that decides.
#[must_use]
pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// Whether `text` is a colour `#rrggbb`: a `#` and six hexadecimal digits,
/// upper- or lower-case, and nothing else.
///
/// - `text`: the text to check, already trimmed.
///
/// It is the CHECK `color ~ '^#[0-9A-Fa-f]{6}$'` of ADR 0009 ("Column
/// types"), which every legacy colour meets, so a value it accepts is never
/// refused by the database. A digit outside ASCII (`#１２３４５６`, with
/// full-width digits) is refused, as the CHECK's bracket expression refuses
/// it.
#[must_use]
pub fn is_color(text: &str) -> bool {
    text.strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// The value of a required text field: `raw`, trimmed ([`trim`]).
///
/// - `field`: the field's name, as the request DTO spells it.
/// - `raw`: the text as the request sent it.
/// - `max`: the most characters the field takes once trimmed
///   ([`char_count`]).
///
/// # Errors
///
/// A [`FieldError`] on `field`, for the first rule `raw` breaks:
/// [`FieldRule::Required`] when it is empty or only white space,
/// [`FieldRule::InvalidCharacter`] when it holds [`NUL`], and
/// [`FieldRule::TooLong`] when it is longer than `max` characters.
pub fn required_text(field: &'static str, raw: &str, max: usize) -> Result<String, FieldError> {
    let text = trim(raw);
    if text.is_empty() {
        return Err(FieldError::new(field, FieldRule::Required));
    }
    check_text(field, text, max)?;
    Ok(text.to_owned())
}

/// The value of an optional text field: `raw`, trimmed ([`trim`]), or
/// `None` (stored as NULL) when it is absent, empty or only white space
/// (SHR-09).
///
/// - `field`: the field's name, as the request DTO spells it.
/// - `raw`: the text as the request sent it, or `None` when the request
///   sent `null` or left the member out.
/// - `max`: the most characters the field takes once trimmed.
///
/// It never returns `Some("")`: an empty text and NULL would otherwise be
/// two forms of "no value" that every reader would have to treat alike.
///
/// # Errors
///
/// A [`FieldError`] on `field`, for the first rule a non-empty `raw`
/// breaks: [`FieldRule::InvalidCharacter`] when it holds [`NUL`], and
/// [`FieldRule::TooLong`] when it is longer than `max` characters.
pub fn optional_text(
    field: &'static str,
    raw: Option<&str>,
    max: usize,
) -> Result<Option<String>, FieldError> {
    let Some(text) = raw.map(trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    check_text(field, text, max)?;
    Ok(Some(text.to_owned()))
}

/// The value of a required colour field: `raw`, trimmed ([`trim`]), kept
/// as written (upper- or lower-case digits alike, as the legacy colours
/// are).
///
/// - `field`: the field's name, as the request DTO spells it.
/// - `raw`: the text as the request sent it.
///
/// # Errors
///
/// A [`FieldError`] on `field`: [`FieldRule::Required`] when `raw` is
/// empty or only white space, and [`FieldRule::InvalidColor`] when the
/// trimmed text is not `#rrggbb` ([`is_color`]).
pub fn color(field: &'static str, raw: &str) -> Result<String, FieldError> {
    let text = trim(raw);
    if text.is_empty() {
        return Err(FieldError::new(field, FieldRule::Required));
    }
    if !is_color(text) {
        return Err(FieldError::new(field, FieldRule::InvalidColor));
    }
    Ok(text.to_owned())
}

/// The checks that every non-empty, trimmed text gets: no [`NUL`], and at
/// most `max` characters.
fn check_text(field: &'static str, text: &str, max: usize) -> Result<(), FieldError> {
    if text.contains(NUL) {
        return Err(FieldError::new(field, FieldRule::InvalidCharacter));
    }
    if char_count(text) > max {
        return Err(FieldError::new(field, FieldRule::TooLong { max }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A field name for the tests.
    const FIELD: &str = "title";

    fn refused(rule: FieldRule) -> FieldError {
        FieldError::new(FIELD, rule)
    }

    #[test]
    fn a_required_text_is_trimmed_and_otherwise_kept_as_written() {
        assert_eq!(
            required_text(FIELD, "  Ünnepség a tornateremben \t\n", 64),
            Ok("Ünnepség a tornateremben".to_owned())
        );
        // Inner white space, line breaks included, stays: only the ends go.
        assert_eq!(
            required_text(FIELD, "egy\n\nkettő  három", 64),
            Ok("egy\n\nkettő  három".to_owned())
        );
        // A no-break space is trimmed too, as `String.prototype.trim` trims
        // it.
        assert_eq!(
            required_text(FIELD, "\u{a0}Hírek\u{a0}", 64),
            Ok("Hírek".to_owned())
        );
    }

    #[test]
    fn the_trimmed_characters_are_ecmascripts() {
        for c in [
            '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{200a}',
            '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}', '\u{feff}',
        ] {
            assert!(is_trimmed(c), "{c:?}");
        }
        for c in ['\u{85}', '\u{200b}', '\u{180e}', 'a', '\0'] {
            assert!(!is_trimmed(c), "{c:?}");
        }
        assert_eq!(trim("\u{feff} Hírek \u{3000}\n"), "Hírek");
        assert_eq!(trim("\u{85}Hírek\u{85}"), "\u{85}Hírek\u{85}");
        assert_eq!(trim(" \u{feff} "), "");
        assert_eq!(trim(""), "");
    }

    #[test]
    fn every_rule_trims_as_the_admin_trims() {
        // A byte-order mark pasted at either end is trimmed, so the stored
        // name is the one the live-name index compares with the others.
        assert_eq!(
            required_text(FIELD, "\u{feff}Hírek\u{feff}", 64),
            Ok("Hírek".to_owned())
        );
        // Only a byte-order mark is empty, as the admin sees it.
        assert_eq!(
            required_text(FIELD, "\u{feff}", 64),
            Err(refused(FieldRule::Required))
        );
        assert_eq!(optional_text(FIELD, Some(" \u{feff}"), 64), Ok(None));
        assert_eq!(color(FIELD, "\u{feff}#1E88E5"), Ok("#1E88E5".to_owned()));
        assert_eq!(color(FIELD, "\u{feff}"), Err(refused(FieldRule::Required)));
        // `String.prototype.trim` keeps U+0085 (NEXT LINE), so it stays,
        // and a field of only that character is not empty, in the admin
        // and here alike.
        assert_eq!(
            required_text(FIELD, "\u{85}Hírek", 64),
            Ok("\u{85}Hírek".to_owned())
        );
        assert_eq!(required_text(FIELD, "\u{85}", 64), Ok("\u{85}".to_owned()));
        assert_eq!(
            optional_text(FIELD, Some("\u{85}"), 64),
            Ok(Some("\u{85}".to_owned()))
        );
        assert_eq!(
            color(FIELD, "#1E88E5\u{85}"),
            Err(refused(FieldRule::InvalidColor))
        );
    }

    #[test]
    fn an_empty_required_text_is_refused_as_required() {
        for raw in [
            "",
            " ",
            "\t\r\n",
            "\u{a0}\u{2003}\u{3000}",
            "\u{feff}\u{2028}",
        ] {
            assert_eq!(
                required_text(FIELD, raw, 64),
                Err(refused(FieldRule::Required)),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn the_length_limit_counts_characters_after_trimming() {
        let at_limit = "ő".repeat(64);
        // 128 bytes, 64 characters: accepted.
        assert_eq!(required_text(FIELD, &at_limit, 64), Ok(at_limit.clone()));
        // White space around it does not count.
        assert_eq!(
            required_text(FIELD, &format!("  {at_limit}  "), 64),
            Ok(at_limit.clone())
        );
        assert_eq!(
            required_text(FIELD, &format!("{at_limit}ő"), 64),
            Err(refused(FieldRule::TooLong { max: 64 }))
        );
        // An emoji is one character, as PostgreSQL's `length()` counts it.
        assert_eq!(char_count("🙂"), 1);
        assert_eq!(char_count("ő"), 1);
        assert_eq!(char_count(""), 0);
    }

    #[test]
    fn a_nul_character_is_refused_before_the_length() {
        assert_eq!(
            required_text(FIELD, "Hír\0ek", 64),
            Err(refused(FieldRule::InvalidCharacter))
        );
        // A NUL is not white space, so it is never trimmed away.
        assert_eq!(
            required_text(FIELD, "\0", 64),
            Err(refused(FieldRule::InvalidCharacter))
        );
        assert_eq!(
            required_text(FIELD, &format!("{}\0", "a".repeat(80)), 64),
            Err(refused(FieldRule::InvalidCharacter))
        );
        assert_eq!(
            optional_text(FIELD, Some(" \0 "), 64),
            Err(refused(FieldRule::InvalidCharacter))
        );
    }

    #[test]
    fn an_empty_optional_text_is_none() {
        // SHR-09: absent, empty and white space only are all NULL.
        for raw in [None, Some(""), Some("   "), Some("\n\t"), Some("\u{a0}")] {
            assert_eq!(optional_text(FIELD, raw, 10), Ok(None), "{raw:?}");
        }
        assert_eq!(
            optional_text(FIELD, Some("  Leírás  "), 10),
            Ok(Some("Leírás".to_owned()))
        );
        assert_eq!(
            optional_text(FIELD, Some("tizenegy betű"), 10),
            Err(refused(FieldRule::TooLong { max: 10 }))
        );
    }

    #[test]
    fn a_colour_is_hash_and_six_hex_digits_in_either_case() {
        for raw in ["#1E88E5", "#1e88e5", "#aBcDeF", "#000000", " #FFFFFF\n"] {
            assert_eq!(color(FIELD, raw), Ok(trim(raw).to_owned()), "{raw:?}");
        }
        for raw in [
            "1E88E5",
            "#1E88E",
            "#1E88E55",
            "#1E88G5",
            "##1E88E5",
            "#1E 88E5",
            "rgb(0,0,0)",
            "#１２３４５６",
            "#ő1234",
        ] {
            assert_eq!(
                color(FIELD, raw),
                Err(refused(FieldRule::InvalidColor)),
                "{raw:?}"
            );
            assert!(!is_color(trim(raw)), "{raw:?}");
        }
        for raw in ["", "  "] {
            assert_eq!(color(FIELD, raw), Err(refused(FieldRule::Required)));
        }
    }

    #[test]
    fn a_field_error_describes_its_field_and_rule() {
        assert_eq!(
            FieldError::new("name", FieldRule::TooLong { max: 64 }).to_string(),
            "name is longer than 64 characters once trimmed"
        );
        assert_eq!(
            FieldError::new("name", FieldRule::Required).to_string(),
            "name is required, but it is empty or only white space"
        );
        assert_eq!(
            FieldError::new("color", FieldRule::InvalidColor).to_string(),
            "color is not a colour of the form #rrggbb"
        );
        assert_eq!(
            FieldError::new("name", FieldRule::InvalidCharacter).to_string(),
            "name holds the character U+0000, which cannot be stored"
        );
    }

    /// The colour rule written out character by character, as the CHECK's
    /// regular expression reads, to compare [`is_color`] with.
    fn matches_color_check(text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        chars.len() == 7 && chars[0] == '#' && chars[1..].iter().all(char::is_ascii_hexdigit)
    }

    proptest! {
        #[test]
        fn a_required_text_is_the_trimmed_input_within_the_limit(
            raw in any::<String>(),
            max in 1usize..80,
        ) {
            let trimmed = trim(&raw);
            match required_text(FIELD, &raw, max) {
                Ok(text) => {
                    prop_assert_eq!(text.as_str(), trimmed);
                    prop_assert!(!text.is_empty());
                    prop_assert!(!text.contains(NUL));
                    prop_assert!(char_count(&text) <= max);
                    // Checking the stored value again changes nothing.
                    prop_assert_eq!(required_text(FIELD, &text, max), Ok(text.clone()));
                }
                Err(error) => {
                    prop_assert_eq!(error.field, FIELD);
                    let expected = if trimmed.is_empty() {
                        FieldRule::Required
                    } else if trimmed.contains(NUL) {
                        FieldRule::InvalidCharacter
                    } else {
                        prop_assert!(char_count(trimmed) > max);
                        FieldRule::TooLong { max }
                    };
                    prop_assert_eq!(error.rule, expected);
                }
            }
        }

        #[test]
        fn an_optional_text_is_none_or_a_trimmed_non_empty_text(
            raw in proptest::option::of(any::<String>()),
            max in 1usize..80,
        ) {
            match optional_text(FIELD, raw.as_deref(), max) {
                Ok(None) => prop_assert!(raw.as_deref().is_none_or(|raw| trim(raw).is_empty())),
                Ok(Some(text)) => {
                    prop_assert!(!text.is_empty());
                    prop_assert_eq!(Some(text.as_str()), raw.as_deref().map(trim));
                    prop_assert!(char_count(&text) <= max);
                }
                Err(error) => {
                    // Only a non-empty text can break a rule, and never the
                    // required one.
                    prop_assert!(raw.as_deref().is_some_and(|raw| !trim(raw).is_empty()));
                    prop_assert_ne!(error.rule, FieldRule::Required);
                }
            }
        }

        #[test]
        fn short_text_without_nul_is_always_accepted(
            raw in "[a-zA-ZáéíóöőúüűÁÉÍÓÖŐÚÜŰ0-9]([a-zA-ZáéíóöőúüűÁÉÍÓÖŐÚÜŰ0-9 .,!?-]{0,62}[a-zA-ZáéíóöőúüűÁÉÍÓÖŐÚÜŰ0-9])?",
        ) {
            prop_assert_eq!(required_text(FIELD, &raw, 64), Ok(raw.clone()));
            prop_assert_eq!(optional_text(FIELD, Some(&raw), 64), Ok(Some(raw.clone())));
        }

        // ECMAScript's trim set is Rust's white space without U+0085, plus
        // U+FEFF.
        #[test]
        fn the_trim_set_differs_from_rusts_by_two(c in any::<char>()) {
            let expected = (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}';
            prop_assert_eq!(is_trimmed(c), expected, "{:?}", c);
        }

        #[test]
        fn trim_removes_the_trimmed_ends_and_nothing_else(
            before in "[ \t\n\u{a0}\u{feff}\u{2028}\u{3000}]{0,4}",
            text in "[a-zé0-9\u{85}]([a-zé0-9 \u{85}\u{feff}]{0,10}[a-zé0-9\u{85}])?",
            after in "[ \t\n\u{a0}\u{feff}\u{2028}\u{3000}]{0,4}",
        ) {
            let padded = format!("{before}{text}{after}");
            prop_assert_eq!(trim(&padded), text.as_str());
            let padding = format!("{before}{after}");
            prop_assert_eq!(trim(&padding), "");
        }

        #[test]
        fn a_trimmed_text_is_a_part_that_trims_to_itself(raw in any::<String>()) {
            let trimmed = trim(&raw);
            prop_assert!(raw.contains(trimmed));
            prop_assert_eq!(trim(trimmed), trimmed);
            prop_assert!(trimmed.chars().next().is_none_or(|c| !is_trimmed(c)));
            prop_assert!(trimmed.chars().next_back().is_none_or(|c| !is_trimmed(c)));
            prop_assert_eq!(trimmed.is_empty(), raw.chars().all(is_trimmed));
        }

        #[test]
        fn char_count_counts_unicode_scalar_values(a in any::<String>(), b in any::<String>()) {
            // It adds up over a concatenation, as a count of characters does.
            prop_assert_eq!(char_count(&format!("{a}{b}")), char_count(&a) + char_count(&b));
            // A character takes one to four bytes in UTF-8.
            prop_assert!(char_count(&a) <= a.len());
            prop_assert!(char_count(&a) * 4 >= a.len());
            // Inside the Basic Multilingual Plane it is JavaScript's
            // `String.length`; outside it, each character is one here and
            // two UTF-16 code units there.
            let plane_0: String = a.chars().filter(|c| c.len_utf16() == 1).collect();
            prop_assert_eq!(char_count(&plane_0), plane_0.encode_utf16().count());
            let astral = a.chars().filter(|c| c.len_utf16() == 2).count();
            prop_assert_eq!(char_count(&a), a.encode_utf16().count() - astral);
        }

        #[test]
        fn a_too_long_refusal_names_its_limit(max in any::<usize>()) {
            prop_assert_eq!(
                FieldError::new(FIELD, FieldRule::TooLong { max }).to_string(),
                format!("{FIELD} is longer than {max} characters once trimmed")
            );
        }

        #[test]
        fn is_color_is_the_check_of_the_schema(raw in any::<String>()) {
            prop_assert_eq!(is_color(&raw), matches_color_check(&raw));
        }

        #[test]
        fn every_colour_of_the_form_is_accepted_as_written(
            raw in "#[0-9A-Fa-f]{6}",
            padding in "[ \t\n]{0,3}",
        ) {
            prop_assert_eq!(color(FIELD, &format!("{padding}{raw}{padding}")), Ok(raw.clone()));
        }

        #[test]
        fn a_colour_is_accepted_exactly_when_its_trimmed_text_is_one(raw in any::<String>()) {
            match color(FIELD, &raw) {
                Ok(text) => {
                    prop_assert!(is_color(&text));
                    prop_assert_eq!(text.as_str(), trim(&raw));
                }
                Err(error) => {
                    let expected = if trim(&raw).is_empty() {
                        FieldRule::Required
                    } else {
                        FieldRule::InvalidColor
                    };
                    prop_assert_eq!(error, FieldError::new(FIELD, expected));
                }
            }
        }
    }
}
