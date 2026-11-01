//! The text forms of calendar days and Europe/Budapest wall times (ADR 0010,
//! "Wire formats"; public-api.md §2.2 and §4).
//!
//! Both APIs write a calendar day as `YYYY-MM-DD` and the start and end of an
//! event as the naive Budapest wall time `YYYY-MM-DD HH:MM:SS`, the forms that
//! backend2 sends today. The Angular site prints them as they are: a time
//! part, a `T`, a fraction or an offset would move the printed day or hour
//! for some readers (ADR 0010, Context). This module reads and writes exactly
//! these two forms:
//!
//! - **Reading is strict.** Every field has a fixed number of digits (four
//!   for the year, two for the others), the separators are `-`, one space and
//!   `:`, and nothing comes before or after. chrono's own parser is not used:
//!   it also takes one-digit fields, signs, longer years and leading spaces,
//!   so `2026-9-1` and `+2026-09-01` would read as days. A text that has the
//!   form but names no day or time, such as `2026-02-30` or `24:00:00`, gets
//!   its own error, so a caller can tell a typo in the form from an
//!   impossible value.
//! - **Writing is canonical.** Each value has one text, and a text that was
//!   read is written back unchanged. A value that has no form (see the next
//!   two points) is still written as what it is, in a text that reading
//!   refuses, so writing never passes off one value as another.
//! - **Years run from 0001 to 9999**, the years that four digits can write.
//!   Year 0000 is refused as well: chrono counts it as 1 BC, and PostgreSQL
//!   refuses the text `0000-01-01`.
//! - **Wall times are whole seconds.** The form has no fraction, and seconds
//!   60, a leap second that chrono can hold, is refused, as
//!   [`super::budapest`] refuses it.
//!
//! Converting a day or a wall time to and from an instant is
//! [`super::budapest`]'s job (ADR 0010 point 5); this module only reads and
//! writes text. The shared DTOs `CalendarDate` and `EventWallTime`
//! (`src/dtos/shared.rs`) build on both. Other pure code in `src/domain/`,
//! which cannot use a DTO (ADR 0008 rule 1), may reuse these forms too. The
//! date part of the public keyset cursor is one candidate: public-api.md
//! §5.2 names backend2's `%Y-%m-%d`, which chrono reads more loosely than
//! this module does, and the slice that adds the cursor decides which reading
//! it keeps.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

/// The first year that the forms can write.
pub const FIRST_YEAR: i32 = 1;

/// The last year that the forms can write.
pub const LAST_YEAR: i32 = 9999;

/// The layout of a calendar day: `d` stands for an ASCII digit, and every
/// other byte must be itself.
const DATE_LAYOUT: &[u8] = b"dddd-dd-dd";

/// The layout of a wall time, as [`DATE_LAYOUT`] reads.
const WALL_TIME_LAYOUT: &[u8] = b"dddd-dd-dd dd:dd:dd";

/// chrono marks a leap second by a nanosecond field of one second or more.
const LEAP_SECOND_NANOS: u32 = 1_000_000_000;

/// chrono's writing of each form. `%Y` pads the years 0000 to 9999 to four
/// digits and gives every other year a sign; `%S` writes a leap second as 60
/// and `%.f` writes a fraction, both only for values that the checks refuse.
const DATE_FORMAT: &str = "%Y-%m-%d";
const WALL_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f";

/// Why a text is not a calendar day or a wall time, or why a value has no
/// text form.
///
/// The `Display` text says what was expected. It never repeats a text that
/// is not in the form, which may be long or not text at all; the values of
/// the other variants are short and are named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DateTextError {
    /// The text is not `YYYY-MM-DD`: another length, a character other than
    /// a digit where a digit belongs, or another separator.
    #[error("expected a calendar day in the form YYYY-MM-DD, such as 2026-09-01")]
    NotDateForm,
    /// The text is not `YYYY-MM-DD HH:MM:SS`, as for [`Self::NotDateForm`].
    #[error(
        "expected a Europe/Budapest wall time in the form YYYY-MM-DD HH:MM:SS, such as \
         2026-09-28 08:00:00"
    )]
    NotWallTimeForm,
    /// The text has the form, but its digits name no day of the calendar,
    /// such as `2026-02-30` or month 13.
    #[error("{year:04}-{month:02}-{day:02} is not a day of the calendar")]
    NoSuchDay {
        /// The year as written.
        year: i32,
        /// The month as written.
        month: u32,
        /// The day of the month as written.
        day: u32,
    },
    /// The text has the form, but its digits name no time of day, such as
    /// `24:00:00` or seconds 60.
    #[error(
        "{hour:02}:{minute:02}:{second:02} is not a time of day: hours run from 00 to 23, \
         minutes and seconds from 00 to 59"
    )]
    NoSuchTime {
        /// The hour as written.
        hour: u32,
        /// The minute as written.
        minute: u32,
        /// The second as written.
        second: u32,
    },
    /// The year is outside [`FIRST_YEAR`] to [`LAST_YEAR`].
    #[error("year {year} is outside 0001 to 9999, the years that the four-digit form can write")]
    YearOutOfRange {
        /// The year of the text or the value.
        year: i32,
    },
    /// The wall time has a fraction of a second, which the form cannot
    /// carry.
    #[error("{wall} has a fraction of a second; the form HH:MM:SS carries whole seconds only")]
    FractionOfSecond {
        /// The wall time as given.
        wall: NaiveDateTime,
    },
    /// The wall time is a leap second (seconds 60), which backend3 does not
    /// store (`budapest` refuses it too).
    #[error("{wall} is a leap second, which backend3 does not store; use seconds 00 to 59")]
    LeapSecond {
        /// The wall time as given.
        wall: NaiveDateTime,
    },
}

/// Checks that a calendar day has the text form: its year is 0001 to 9999.
///
/// - `date`: the day, for example one converted from a stored Budapest
///   midnight.
///
/// # Errors
///
/// [`DateTextError::YearOutOfRange`] for any other year.
pub fn check_date(date: NaiveDate) -> Result<NaiveDate, DateTextError> {
    check_year(date.year())?;
    Ok(date)
}

/// Checks that a wall time has the text form: its year is 0001 to 9999, and
/// it is a whole second that is not a leap second.
///
/// - `wall`: the wall time, for example one converted from a stored event
///   start.
///
/// # Errors
///
/// [`DateTextError::YearOutOfRange`], [`DateTextError::LeapSecond`] and
/// [`DateTextError::FractionOfSecond`], checked in this order.
pub fn check_wall_time(wall: NaiveDateTime) -> Result<NaiveDateTime, DateTextError> {
    check_year(wall.year())?;
    match wall.nanosecond() {
        0 => Ok(wall),
        nanos if nanos >= LEAP_SECOND_NANOS => Err(DateTextError::LeapSecond { wall }),
        _ => Err(DateTextError::FractionOfSecond { wall }),
    }
}

/// Reads a calendar day written `YYYY-MM-DD`, and no other form.
///
/// - `text`: the text as received, for example a JSON string or a path
///   segment; it is not trimmed.
///
/// # Errors
///
/// [`DateTextError::NotDateForm`] when `text` is not in the form,
/// [`DateTextError::YearOutOfRange`] for year 0000 and
/// [`DateTextError::NoSuchDay`] for a day that does not exist.
pub fn parse_date(text: &str) -> Result<NaiveDate, DateTextError> {
    let bytes = text.as_bytes();
    if !has_layout(bytes, DATE_LAYOUT) {
        return Err(DateTextError::NotDateForm);
    }
    date_from_digits(bytes)
}

/// Reads a wall time written `YYYY-MM-DD HH:MM:SS`, and no other form.
///
/// - `text`: the text as received; it is not trimmed.
///
/// Every wall time in the form is read, also one that the spring change
/// skips: whether it exists in Europe/Budapest is for
/// [`super::budapest::wall_to_instant`] to say, so that the admin API can
/// refuse it on its field (ADR 0010 point 6).
///
/// # Errors
///
/// [`DateTextError::NotWallTimeForm`] when `text` is not in the form, and
/// then, for its date and its time in that order,
/// [`DateTextError::YearOutOfRange`], [`DateTextError::NoSuchDay`] and
/// [`DateTextError::NoSuchTime`].
pub fn parse_wall_time(text: &str) -> Result<NaiveDateTime, DateTextError> {
    let bytes = text.as_bytes();
    if !has_layout(bytes, WALL_TIME_LAYOUT) {
        return Err(DateTextError::NotWallTimeForm);
    }
    let date = date_from_digits(&bytes[..DATE_LAYOUT.len()])?;
    let (hour, minute, second) = (
        u32::from(number(&bytes[11..13])),
        u32::from(number(&bytes[14..16])),
        u32::from(number(&bytes[17..19])),
    );
    // `from_hms_opt` refuses seconds 60: chrono builds a leap second only
    // from a nanosecond field of a second or more.
    let time = NaiveTime::from_hms_opt(hour, minute, second).ok_or(DateTextError::NoSuchTime {
        hour,
        minute,
        second,
    })?;
    Ok(date.and_time(time))
}

/// Writes a calendar day as `YYYY-MM-DD`.
///
/// - `date`: the day to write.
///
/// A day that [`check_date`] accepts gets its one text, which
/// [`parse_date`] reads back as the same day. Any other day is written with
/// a sign or with year 0000, which [`parse_date`] refuses.
#[must_use]
pub fn format_date(date: NaiveDate) -> String {
    date.format(DATE_FORMAT).to_string()
}

/// Writes a wall time as `YYYY-MM-DD HH:MM:SS`.
///
/// - `wall`: the wall time to write.
///
/// A wall time that [`check_wall_time`] accepts gets its one text, which
/// [`parse_wall_time`] reads back as the same wall time. Any other one keeps
/// what makes it so (a sign or year 0000, seconds 60, a fraction), and
/// [`parse_wall_time`] refuses the text.
#[must_use]
pub fn format_wall_time(wall: NaiveDateTime) -> String {
    wall.format(WALL_TIME_FORMAT).to_string()
}

/// Refuses a year outside [`FIRST_YEAR`] to [`LAST_YEAR`].
fn check_year(year: i32) -> Result<(), DateTextError> {
    if (FIRST_YEAR..=LAST_YEAR).contains(&year) {
        Ok(())
    } else {
        Err(DateTextError::YearOutOfRange { year })
    }
}

/// Whether `text` matches `layout`, byte for byte: a `d` of the layout takes
/// one ASCII digit, any other byte only itself.
///
/// Comparing bytes is enough: a character outside ASCII never matches, since
/// every byte of the layout is ASCII and every byte of such a character is
/// not.
fn has_layout(text: &[u8], layout: &[u8]) -> bool {
    text.len() == layout.len()
        && text
            .iter()
            .zip(layout)
            .all(|(&byte, &expected)| match expected {
                b'd' => byte.is_ascii_digit(),
                _ => byte == expected,
            })
}

/// The day that the first ten bytes of `bytes` write, after
/// [`has_layout`] matched [`DATE_LAYOUT`] there.
fn date_from_digits(bytes: &[u8]) -> Result<NaiveDate, DateTextError> {
    let (year, month, day) = (
        i32::from(number(&bytes[0..4])),
        u32::from(number(&bytes[5..7])),
        u32::from(number(&bytes[8..10])),
    );
    check_year(year)?;
    NaiveDate::from_ymd_opt(year, month, day).ok_or(DateTextError::NoSuchDay { year, month, day })
}

/// The number that a run of at most four ASCII digits writes.
///
/// `u16` holds 9999, and its `From` conversions to `i32` and `u32` are
/// lossless, so no cast is needed.
fn number(digits: &[u8]) -> u16 {
    digits
        .iter()
        .fold(0, |value, digit| value * 10 + u16::from(digit - b'0'))
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use proptest::prelude::*;

    use super::*;
    use DateTextError as E;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("a test day")
    }

    fn wall(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").expect("a test wall time")
    }

    /// The error for a day that the digits `year`, `month` and `day` name.
    fn no_day(year: i32, month: u32, day: u32) -> DateTextError {
        E::NoSuchDay { year, month, day }
    }

    /// The error for a time that the digits `hour`, `minute` and `second`
    /// name.
    fn no_time(hour: u32, minute: u32, second: u32) -> DateTextError {
        E::NoSuchTime {
            hour,
            minute,
            second,
        }
    }

    #[test]
    fn days_in_the_form_are_read() {
        for (text, expected) in [
            ("2026-09-01", date(2026, 9, 1)),
            ("0001-01-01", date(1, 1, 1)),
            ("9999-12-31", date(9999, 12, 31)),
            ("2028-02-29", date(2028, 2, 29)),
            ("2000-02-29", date(2000, 2, 29)),
            ("1970-01-01", date(1970, 1, 1)),
        ] {
            assert_eq!(parse_date(text), Ok(expected), "{text}");
            assert_eq!(format_date(expected), text, "{text}");
        }
    }

    #[test]
    fn other_forms_of_a_day_are_refused() {
        for text in [
            "",
            "2026-9-1",
            "2026-09-1",
            "26-09-01",
            "02026-09-01",
            "+2026-09-01",
            "-2026-09-01",
            " 2026-09-01",
            "2026-09-01 ",
            "2026-09-01\n",
            "2026/09/01",
            "2026.09.01",
            "20260901",
            "2026-09-01T00:00:00",
            "2026-09-01 00:00:00",
            "2026-09-01Z",
            "2026-09-01+02:00",
            "2026-W36-2",
            "2026-244",
            "2026-09-0a",
            "２０２６-09-01",
            "2026-09-01\u{0}",
        ] {
            assert_eq!(parse_date(text), Err(E::NotDateForm), "{text:?}");
        }
    }

    #[test]
    fn impossible_days_in_the_form_are_named() {
        for (text, error) in [
            ("2026-02-29", no_day(2026, 2, 29)),
            ("1900-02-29", no_day(1900, 2, 29)),
            ("2026-04-31", no_day(2026, 4, 31)),
            ("2026-13-01", no_day(2026, 13, 1)),
            ("2026-00-10", no_day(2026, 0, 10)),
            ("2026-09-00", no_day(2026, 9, 0)),
            ("0000-01-01", E::YearOutOfRange { year: 0 }),
            // The year is checked before the day.
            ("0000-13-45", E::YearOutOfRange { year: 0 }),
        ] {
            assert_eq!(parse_date(text), Err(error), "{text}");
        }
        let message = no_day(2026, 2, 29).to_string();
        assert_eq!(message, "2026-02-29 is not a day of the calendar");
    }

    #[test]
    fn wall_times_in_the_form_are_read() {
        for text in [
            "2026-09-28 08:00:00",
            // Picker seconds of the legacy events (ADR 0010, Context).
            "2019-03-15 00:00:52",
            "2019-07-01 23:59:00",
            "0001-01-01 00:00:00",
            "9999-12-31 23:59:59",
            "2028-02-29 12:00:00",
            // Both changes of 2026: reading does not ask whether the wall
            // time exists in Budapest (`budapest::wall_to_instant` does).
            "2026-03-29 02:30:00",
            "2026-10-25 02:30:00",
        ] {
            assert_eq!(parse_wall_time(text), Ok(wall(text)), "{text}");
            assert_eq!(format_wall_time(wall(text)), text, "{text}");
        }
    }

    #[test]
    fn other_forms_of_a_wall_time_are_refused() {
        for text in [
            "",
            "2026-09-28",
            "2026-09-28T08:00:00",
            "2026-09-28t08:00:00",
            "2026-09-28  08:00:00",
            "2026-09-28 08:00",
            "2026-09-28 8:00:00",
            "2026-09-28 08:00:00.000",
            "2026-09-28 08:00:00,5",
            "2026-09-28 08:00:00Z",
            "2026-09-28 08:00:00+02:00",
            "2026-09-28T06:00:00Z",
            "2026-09-28 08.00.00",
            " 2026-09-28 08:00:00",
            "2026-09-28 08:00:00 ",
            "+2026-09-28 08:00:00",
            "2026-9-28 08:00:00",
            "2026-09-28\t08:00:00",
            "2026-09-28\u{a0}08:00:00",
        ] {
            assert_eq!(parse_wall_time(text), Err(E::NotWallTimeForm), "{text:?}");
        }
    }

    #[test]
    fn impossible_wall_times_in_the_form_are_named() {
        for (text, error) in [
            ("2026-09-28 24:00:00", no_time(24, 0, 0)),
            ("2026-09-28 23:60:00", no_time(23, 60, 0)),
            ("2026-09-28 23:59:60", no_time(23, 59, 60)),
            ("2026-09-28 99:99:99", no_time(99, 99, 99)),
            ("2026-02-30 08:00:00", no_day(2026, 2, 30)),
            ("0000-06-01 08:00:00", E::YearOutOfRange { year: 0 }),
            // The date is checked before the time.
            ("2026-02-30 24:00:00", no_day(2026, 2, 30)),
        ] {
            assert_eq!(parse_wall_time(text), Err(error), "{text}");
        }
        let message = no_time(24, 0, 0).to_string();
        assert!(
            message.starts_with("24:00:00 is not a time of day"),
            "{message}"
        );
    }

    #[test]
    fn errors_about_the_form_do_not_repeat_the_text() {
        let long = "<script>".repeat(1_000);
        for error in [
            parse_date(&long),
            parse_wall_time(&long).map(|wall| wall.date()),
        ] {
            let message = error.expect_err("refused").to_string();
            assert!(
                message.len() < 120 && !message.contains("<script>"),
                "{message}"
            );
        }
        assert_eq!(
            E::NotDateForm.to_string(),
            "expected a calendar day in the form YYYY-MM-DD, such as 2026-09-01"
        );
        assert_eq!(
            E::NotWallTimeForm.to_string(),
            "expected a Europe/Budapest wall time in the form YYYY-MM-DD HH:MM:SS, such as \
             2026-09-28 08:00:00"
        );
    }

    #[test]
    fn the_checks_take_years_0001_to_9999_and_whole_seconds() {
        for (day, expected) in [
            (date(1, 1, 1), Ok(date(1, 1, 1))),
            (date(9999, 12, 31), Ok(date(9999, 12, 31))),
            (date(0, 12, 31), Err(E::YearOutOfRange { year: 0 })),
            (date(10_000, 1, 1), Err(E::YearOutOfRange { year: 10_000 })),
            (date(-1, 1, 1), Err(E::YearOutOfRange { year: -1 })),
            (
                NaiveDate::MIN,
                Err(E::YearOutOfRange {
                    year: NaiveDate::MIN.year(),
                }),
            ),
            (
                NaiveDate::MAX,
                Err(E::YearOutOfRange {
                    year: NaiveDate::MAX.year(),
                }),
            ),
        ] {
            assert_eq!(check_date(day), expected, "{day}");
        }

        let whole = wall("2026-09-28 08:00:00");
        assert_eq!(check_wall_time(whole), Ok(whole));
        let fraction = wall("2026-09-28 08:00:00.000001");
        assert_eq!(
            check_wall_time(fraction),
            Err(E::FractionOfSecond { wall: fraction })
        );
        let parsed = NaiveDateTime::parse_from_str("2026-06-30 23:59:60", "%Y-%m-%d %H:%M:%S");
        let leap = parsed.expect("chrono parses seconds 60 as a leap second");
        assert_eq!(check_wall_time(leap), Err(E::LeapSecond { wall: leap }));
        let late = date(10_000, 1, 1).and_time(NaiveTime::MIN) + TimeDelta::milliseconds(5);
        assert_eq!(
            check_wall_time(late),
            Err(E::YearOutOfRange { year: 10_000 })
        );
    }

    #[test]
    fn values_without_a_form_are_written_in_texts_that_are_refused() {
        for (day, text) in [
            (date(0, 12, 31), "0000-12-31"),
            (date(10_000, 1, 1), "+10000-01-01"),
            (date(-1, 1, 1), "-0001-01-01"),
        ] {
            assert_eq!(format_date(day), text);
            assert!(parse_date(text).is_err(), "{text}");
        }
        let fraction = wall("2026-09-28 08:00:00.25");
        assert_eq!(format_wall_time(fraction), "2026-09-28 08:00:00.250");
        let parsed = NaiveDateTime::parse_from_str("2026-06-30 23:59:60", "%Y-%m-%d %H:%M:%S");
        let leap = parsed.expect("a leap second");
        assert_eq!(format_wall_time(leap), "2026-06-30 23:59:60");
        for value in [fraction, leap] {
            assert!(
                parse_wall_time(&format_wall_time(value)).is_err(),
                "{value}"
            );
        }
    }

    /// Any day chrono can represent, or one of the years the forms write.
    fn days() -> impl Strategy<Value = NaiveDate> {
        let any_day = NaiveDate::MIN.num_days_from_ce()..=NaiveDate::MAX.num_days_from_ce();
        let in_range = date(1, 1, 1).num_days_from_ce()..=date(9999, 12, 31).num_days_from_ce();
        prop_oneof![any_day, in_range]
            .prop_map(|days| NaiveDate::from_num_days_from_ce_opt(days).expect("a day in range"))
    }

    /// Wall times: whole seconds, fractions, and leap seconds (chrono has
    /// them only at seconds 59).
    fn walls() -> impl Strategy<Value = NaiveDateTime> {
        let time = prop_oneof![
            4 => (0u32..86_400, Just(0u32)),
            1 => (0u32..86_400, 1..LEAP_SECOND_NANOS),
            1 => ((0u32..1_440).prop_map(|minute| minute * 60 + 59), LEAP_SECOND_NANOS..2 * LEAP_SECOND_NANOS),
        ];
        (days(), time).prop_map(|(day, (seconds, nanos))| {
            let time = NaiveTime::from_num_seconds_from_midnight_opt(seconds, nanos);
            day.and_time(time.expect("a time"))
        })
    }

    /// Texts near the forms: arbitrary text, the layouts filled with any
    /// digits, and the layouts with one byte replaced or with text around.
    fn texts() -> impl Strategy<Value = String> {
        prop_oneof![
            any::<String>(),
            "[0-9]{4}-[0-9]{2}-[0-9]{2}",
            "[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}",
            "[0-9 :+T.-]{8,21}",
            "[0-9]{4}-[01][0-9]-[0-3][0-9]( [0-2][0-9]:[0-6][0-9]:[0-6][0-9])?.?",
        ]
    }

    proptest! {
        // Writing is honest: a day the check accepts reads back as itself,
        // and any other day is written in a text that reading refuses.
        #[test]
        fn a_written_day_reads_back_exactly_when_it_has_the_form(day in days()) {
            let text = format_date(day);
            match check_date(day) {
                Ok(checked) => {
                    prop_assert_eq!(checked, day);
                    prop_assert_eq!(parse_date(&text), Ok(day));
                    prop_assert!(has_layout(text.as_bytes(), DATE_LAYOUT), "{}", text);
                }
                Err(error) => {
                    prop_assert_eq!(error, E::YearOutOfRange { year: day.year() });
                    prop_assert!(parse_date(&text).is_err(), "{}", text);
                }
            }
        }

        #[test]
        fn a_written_wall_time_reads_back_exactly_when_it_has_the_form(w in walls()) {
            let text = format_wall_time(w);
            match check_wall_time(w) {
                Ok(checked) => {
                    prop_assert_eq!(checked, w);
                    prop_assert_eq!(parse_wall_time(&text), Ok(w));
                    prop_assert!(has_layout(text.as_bytes(), WALL_TIME_LAYOUT), "{}", text);
                    prop_assert_eq!(&text[..10], format_date(w.date()));
                }
                Err(error) => {
                    let expected = if !(FIRST_YEAR..=LAST_YEAR).contains(&w.year()) {
                        E::YearOutOfRange { year: w.year() }
                    } else if w.nanosecond() >= LEAP_SECOND_NANOS {
                        E::LeapSecond { wall: w }
                    } else {
                        E::FractionOfSecond { wall: w }
                    };
                    prop_assert_eq!(error, expected);
                    prop_assert!(parse_wall_time(&text).is_err(), "{}", text);
                }
            }
        }

        // Reading is canonical: whatever text reads as a value is the one
        // text that value is written as. So no other form, padding or case
        // gets through, and an error about the form is given exactly when
        // the layout does not match.
        #[test]
        fn only_the_canonical_text_of_a_value_is_read(text in texts()) {
            match parse_date(&text) {
                Ok(day) => {
                    prop_assert_eq!(format_date(day), text.clone());
                    prop_assert_eq!(check_date(day), Ok(day));
                }
                Err(error) => prop_assert_eq!(
                    error == E::NotDateForm,
                    !has_layout(text.as_bytes(), DATE_LAYOUT),
                    "{:?}: {:?}", text, error
                ),
            }
            match parse_wall_time(&text) {
                Ok(w) => {
                    prop_assert_eq!(format_wall_time(w), text.clone());
                    prop_assert_eq!(check_wall_time(w), Ok(w));
                }
                Err(error) => prop_assert_eq!(
                    error == E::NotWallTimeForm,
                    !has_layout(text.as_bytes(), WALL_TIME_LAYOUT),
                    "{:?}: {:?}", text, error
                ),
            }
        }
    }
}
