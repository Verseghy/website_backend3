//! Wire types that both APIs share (ADR 0005 §2, the last row of its table):
//! the calendar day and the event wall time of ADR 0010.
//!
//! Each is a JSON string in exactly one form (ADR 0010, "Wire formats";
//! public-api.md §2.2 and §4):
//!
//! | Type | Form | Example | Stored as |
//! |---|---|---|---|
//! | [`CalendarDate`] | `YYYY-MM-DD` | `2026-09-01` | the instant of the day's Europe/Budapest midnight (ADR 0010 point 2) |
//! | [`EventWallTime`] | `YYYY-MM-DD HH:MM:SS` | `2026-09-28 08:00:00` | the instant of that Europe/Budapest wall time (point 1) |
//!
//! - **The server converts, in both directions.** A handler builds the
//!   wire value from the stored instant ([`CalendarDate::from_midnight`],
//!   [`EventWallTime::from_instant`]) and turns a received one back into an
//!   instant ([`CalendarDate::to_midnight`], [`EventWallTime::to_instant`]),
//!   always through `domain::budapest`, the one conversion module (point 5).
//!   Neither the admin nor the public site converts zones; the admin uses the
//!   same types, so it never converts event times itself (ADR 0005 §3).
//! - **Reading is strict** (`domain::date_text`). Any other form, such as an
//!   RFC 3339 instant, a `T`, a fraction, an offset, a one-digit field or
//!   padding, is refused, never reinterpreted; so is any JSON value that is
//!   not a string. A value can only be built through a check, so every
//!   value has its form and writing one cannot fail.
//! - **Reading does not convert.** A received day or wall time that has the
//!   form is read even if it has no instant: a wall time that the spring
//!   change skips, or one of the few days before 1984 without a Budapest
//!   midnight. The conversion to an instant refuses it, so that the admin
//!   API answers on the field (ADR 0010 point 6) instead of refusing the
//!   whole body.
//!
//! Both derive `ts_rs::TS` as `string` and are exported to
//! `admin/src/bindings/`, so an admin DTO with such a field gets a named
//! `string` type. Their utoipa schemas, a `string` with the form's `pattern`
//! (docs/design/openapi-spike.md §3, rule 3), are added together with the
//! first public DTO that uses each type: that endpoint's response-shape test
//! is what checks a schema against the JSON that serde writes (ADR 0005 §4).
//! The derive also publishes the type's doc comment as the schema's
//! description (rules 3 and 5). Today that comment is written for the admin,
//! whose binding carries it, so the same slice rewords it for API users too.

use std::{fmt, marker::PhantomData, str::FromStr};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
};
use ts_rs::TS;

use crate::domain::{
    budapest::{self, BudapestTimeError},
    date_text::{self, DateTextError},
};

/// A calendar day, written `YYYY-MM-DD`, such as `2026-09-01`: the date of a
/// post, a canteen day or a Diáktoll issue.
///
/// A day of the school's calendar (Europe/Budapest), with no time and no
/// offset. Show it as it is and send it back in the same form. Never turn it
/// into a JavaScript `Date`: `new Date("2026-09-01")` is midnight UTC, which
/// is the day before in a browser west of Greenwich.
// The doc comment above is exported into the TypeScript binding; Rust-only
// notes go here. It is written for the admin. The `ToSchema` derive that the
// first public DTO adds would publish it as the OpenAPI description too
// (openapi-spike.md §3, rules 3 and 5), so that slice rewords it for API users
// as well, or sets `#[schema(description = ...)]`. The field is private: a
// value is built by `new`, `from_str`, serde or `from_midnight`, which all
// check that the day has the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[ts(export, export_to = "../admin/src/bindings/", type = "string")]
pub struct CalendarDate(NaiveDate);

impl CalendarDate {
    /// A calendar day as it goes on the wire.
    ///
    /// - `date`: the day, for example the default post date
    ///   `budapest::today(now)` (ADR 0010 point 7).
    ///
    /// # Errors
    ///
    /// [`DateTextError::YearOutOfRange`] for a year outside 0001 to 9999.
    pub fn new(date: NaiveDate) -> Result<Self, DateTextError> {
        date_text::check_date(date).map(Self)
    }

    /// The day that a stored calendar-day value stands for (ADR 0010
    /// point 2).
    ///
    /// - `midnight`: the stored instant, for example `posts.date`; it must be
    ///   a Europe/Budapest midnight, which the column's check guarantees.
    ///
    /// # Errors
    ///
    /// [`WireTimeError::Conversion`] with [`BudapestTimeError::NotMidnight`]
    /// when `midnight` is not 00:00:00 in Budapest (a defect, reported and
    /// not rounded), or with the other errors of
    /// [`budapest::midnight_to_date`]; [`WireTimeError::Form`] when the day's
    /// year is outside 0001 to 9999.
    pub fn from_midnight(midnight: DateTime<Utc>) -> Result<Self, WireTimeError> {
        Ok(Self::new(budapest::midnight_to_date(midnight)?)?)
    }

    /// The instant of this day's Europe/Budapest midnight, the form in which
    /// a calendar-day column stores it (ADR 0010 point 2).
    ///
    /// # Errors
    ///
    /// [`BudapestTimeError::NoMidnight`] for a day on which the clocks
    /// skipped 00:00, which happened only on a few days before 1984. The
    /// admin API answers it on the field.
    pub fn to_midnight(self) -> Result<DateTime<Utc>, BudapestTimeError> {
        budapest::date_to_midnight(self.0)
    }

    /// The calendar day.
    #[must_use]
    pub const fn date(self) -> NaiveDate {
        self.0
    }
}

/// A Europe/Budapest wall time, written `YYYY-MM-DD HH:MM:SS`, such as
/// `2026-09-28 08:00:00`: the start or the end of an event.
///
/// The time that the school's clocks show, whatever the browser's time zone,
/// in whole seconds, with no `T`, no fraction and no offset. Show it as it is
/// and send it back in the same form; the server converts it to and from the
/// stored instant. A time that the clocks skip when summer time starts (02:00
/// to 02:59 on the last Sunday of March) is refused on its field, and a time
/// of the repeated autumn hour means its first occurrence, summer time.
// The doc comment above goes into the TypeScript binding and is written for
// the admin ("send it back", "refused on its field"); the public slice that
// derives `ToSchema` rewords it for API users too, or sets
// `#[schema(description = ...)]`, since the derive publishes it (as on
// `CalendarDate`). No `PartialOrd` or `Ord` on purpose: in the repeated autumn
// hour, wall-time order differs from time order (02:45 summer time comes
// before 02:15 winter time), so lists sort by the stored instant, never by
// this value. The field is private: every constructor checks the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TS)]
#[ts(export, export_to = "../admin/src/bindings/", type = "string")]
pub struct EventWallTime(NaiveDateTime);

impl EventWallTime {
    /// A wall time as it goes on the wire.
    ///
    /// - `wall`: the Europe/Budapest wall time.
    ///
    /// # Errors
    ///
    /// [`DateTextError::YearOutOfRange`] for a year outside 0001 to 9999,
    /// [`DateTextError::LeapSecond`] and [`DateTextError::FractionOfSecond`]:
    /// the form carries whole seconds only.
    pub fn new(wall: NaiveDateTime) -> Result<Self, DateTextError> {
        date_text::check_wall_time(wall).map(Self)
    }

    /// The Europe/Budapest wall time of a stored instant, for example
    /// `events.starts_at` (ADR 0010, "Wire formats"). Both instants of the
    /// repeated autumn hour show the same wall time.
    ///
    /// - `instant`: the stored instant.
    ///
    /// # Errors
    ///
    /// [`WireTimeError::Conversion`] with the errors of
    /// [`budapest::instant_to_wall`]; [`WireTimeError::Form`] when the wall
    /// time has a fraction of a second, which no write path stores (it is
    /// reported, not rounded), or a year outside 0001 to 9999.
    pub fn from_instant(instant: DateTime<Utc>) -> Result<Self, WireTimeError> {
        Ok(Self::new(budapest::instant_to_wall(instant)?)?)
    }

    /// The instant of this wall time (ADR 0010 point 6): in the repeated
    /// autumn hour the earlier one, `+02:00`.
    ///
    /// # Errors
    ///
    /// [`BudapestTimeError::NonexistentWallTime`] for a wall time that the
    /// spring change skips, such as `2026-03-29 02:30:00`. The admin API
    /// answers it on the field; the importer stops and names the row.
    pub fn to_instant(self) -> Result<DateTime<Utc>, BudapestTimeError> {
        budapest::wall_to_instant(self.0)
    }

    /// The Europe/Budapest wall time.
    #[must_use]
    pub const fn wall(self) -> NaiveDateTime {
        self.0
    }
}

/// Why a stored instant cannot go on the wire as a [`CalendarDate`] or an
/// [`EventWallTime`]. Every case is a defect in the stored data, which the
/// handler reports as a server error, never as the client's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WireTimeError {
    /// The Budapest conversion refused the instant; for a calendar day, the
    /// instant is not a Budapest midnight.
    #[error(transparent)]
    Conversion(#[from] BudapestTimeError),
    /// The converted day or wall time has no text form.
    #[error(transparent)]
    Form(#[from] DateTextError),
}

impl fmt::Display for CalendarDate {
    /// Writes the day in its one form, `YYYY-MM-DD`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&date_text::format_date(self.0))
    }
}

impl fmt::Display for EventWallTime {
    /// Writes the wall time in its one form, `YYYY-MM-DD HH:MM:SS`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&date_text::format_wall_time(self.0))
    }
}

impl FromStr for CalendarDate {
    type Err = DateTextError;

    /// Reads a day written `YYYY-MM-DD`, and no other form, for example
    /// from a JSON string or a path segment.
    ///
    /// - `text`: the text as received; it is not trimmed.
    fn from_str(text: &str) -> Result<Self, DateTextError> {
        // `parse_date` reads only years 0001 to 9999, so the day has the form.
        date_text::parse_date(text).map(Self)
    }
}

impl FromStr for EventWallTime {
    type Err = DateTextError;

    /// Reads a wall time written `YYYY-MM-DD HH:MM:SS`, and no other form.
    ///
    /// - `text`: the text as received; it is not trimmed.
    fn from_str(text: &str) -> Result<Self, DateTextError> {
        // `parse_wall_time` reads whole seconds of years 0001 to 9999 only.
        date_text::parse_wall_time(text).map(Self)
    }
}

impl Serialize for CalendarDate {
    /// Writes the day as a JSON string in its one form.
    ///
    /// - `serializer`: the output format, JSON in both APIs.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Serialize for EventWallTime {
    /// Writes the wall time as a JSON string in its one form.
    ///
    /// - `serializer`: the output format, JSON in both APIs.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CalendarDate {
    /// Reads the day from a JSON string in its one form, and from nothing
    /// else.
    ///
    /// - `deserializer`: the input, a JSON request body in the admin API.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(TextVisitor::<Self>(PhantomData))
    }
}

impl<'de> Deserialize<'de> for EventWallTime {
    /// Reads the wall time from a JSON string in its one form, and from
    /// nothing else.
    ///
    /// - `deserializer`: the input, a JSON request body in the admin API.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(TextVisitor::<Self>(PhantomData))
    }
}

/// A wire type that is read from a string through its `FromStr`.
trait TextForm: FromStr<Err = DateTextError> {
    /// What serde's type errors say was expected, for example when a client
    /// sends a number.
    const EXPECTED: &'static str;
}

impl TextForm for CalendarDate {
    const EXPECTED: &'static str = "a calendar day in the form YYYY-MM-DD";
}

impl TextForm for EventWallTime {
    const EXPECTED: &'static str = "a Europe/Budapest wall time in the form YYYY-MM-DD HH:MM:SS";
}

/// Reads a [`TextForm`] from a string, and from nothing else: serde's
/// defaults refuse every other JSON value with an error that names
/// [`TextForm::EXPECTED`].
struct TextVisitor<T>(PhantomData<T>);

impl<T: TextForm> Visitor<'_> for TextVisitor<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(T::EXPECTED)
    }

    /// Parses the string, and refuses it with the form's error.
    ///
    /// - `text`: the string as received; serde's defaults also send
    ///   borrowed and owned strings here.
    ///
    /// The error is `custom` rather than serde's `invalid_value`, which
    /// would quote the whole string: [`DateTextError`] names short values
    /// only.
    fn visit_str<E: de::Error>(self, text: &str) -> Result<T, E> {
        text.parse().map_err(E::custom)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, NaiveTime, TimeDelta, Timelike};
    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::*;
    use BudapestTimeError as B;
    use DateTextError as D;

    fn day(text: &str) -> CalendarDate {
        text.parse().expect("a test day")
    }

    fn wall(text: &str) -> EventWallTime {
        text.parse().expect("a test wall time")
    }

    fn utc(text: &str) -> DateTime<Utc> {
        text.parse().expect("a test instant")
    }

    fn json_of(value: impl Serialize) -> Value {
        serde_json::to_value(value).expect("serializable")
    }

    #[test]
    fn a_calendar_date_is_written_as_yyyy_mm_dd() {
        for text in [
            "2026-09-01",
            "0001-01-01",
            "9999-12-31",
            "2028-02-29",
            "1970-01-01",
        ] {
            assert_eq!(json_of(day(text)), json!(text));
            assert_eq!(day(text).to_string(), text);
        }
        let first = CalendarDate::new(NaiveDate::from_ymd_opt(1, 1, 1).expect("a day"));
        assert_eq!(
            serde_json::to_string(&first.expect("year 1")).ok(),
            Some(r#""0001-01-01""#.to_owned())
        );
    }

    #[test]
    fn an_event_wall_time_is_written_as_yyyy_mm_dd_hh_mm_ss() {
        for text in [
            "2026-09-28 08:00:00",
            "2019-03-15 00:00:52",
            "2019-07-01 23:59:00",
            "2019-12-01 00:01:00",
            "0001-01-01 00:00:00",
            "9999-12-31 23:59:59",
        ] {
            assert_eq!(json_of(wall(text)), json!(text));
            assert_eq!(wall(text).to_string(), text);
        }
    }

    #[test]
    fn both_types_are_read_back_from_their_json() {
        let date: CalendarDate = serde_json::from_str(r#""2026-09-01""#).expect("a day");
        assert_eq!(
            date.date(),
            NaiveDate::from_ymd_opt(2026, 9, 1).expect("a day")
        );
        let start: EventWallTime =
            serde_json::from_str(r#""2026-09-28 08:00:00""#).expect("a time");
        let expected = NaiveDate::from_ymd_opt(2026, 9, 28).and_then(|d| d.and_hms_opt(8, 0, 0));
        assert_eq!(Some(start.wall()), expected);
        // `null` is a missing value only where the field is an `Option`.
        let none: Option<CalendarDate> = serde_json::from_value(Value::Null).expect("null");
        assert_eq!(none, None);
        let some: Option<EventWallTime> =
            serde_json::from_value(json!("2026-09-28 08:00:00")).expect("a time");
        assert_eq!(some, Some(start));
    }

    /// The message of serde's refusal of `value` as a `T`; panics when it is
    /// accepted.
    fn refusal<T: for<'de> Deserialize<'de> + fmt::Debug>(value: &Value) -> String {
        match serde_json::from_value::<T>(value.clone()) {
            Ok(accepted) => panic!("accepted {value}: {accepted:?}"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn other_forms_of_a_calendar_date_are_refused() {
        let form = "expected a calendar day in the form YYYY-MM-DD, such as 2026-09-01";
        for text in [
            "2026-9-1",
            "02026-09-01",
            "+2026-09-01",
            " 2026-09-01",
            "2026-09-01 ",
            "2026/09/01",
            "20260901",
            "2026-09-01T00:00:00Z",
            "2026-09-01T00:00:00+02:00",
            "2026-09-01 00:00:00",
            "2026-08-31T22:00:00.000Z",
            "",
        ] {
            assert_eq!(refusal::<CalendarDate>(&json!(text)), form, "{text:?}");
        }
        for (text, message) in [
            ("2026-02-29", "2026-02-29 is not a day of the calendar"),
            ("2026-13-01", "2026-13-01 is not a day of the calendar"),
            (
                "0000-01-01",
                "year 0 is outside 0001 to 9999, the years that the four-digit form can write",
            ),
        ] {
            assert_eq!(refusal::<CalendarDate>(&json!(text)), message, "{text}");
        }
    }

    #[test]
    fn other_forms_of_an_event_wall_time_are_refused() {
        let form = "expected a Europe/Budapest wall time in the form YYYY-MM-DD HH:MM:SS, such as \
                    2026-09-28 08:00:00";
        for text in [
            // An RFC 3339 instant: the form of every other instant (ADR 0010).
            "2026-09-28T06:00:00Z",
            "2026-09-28T08:00:00",
            "2026-09-28 08:00:00Z",
            "2026-09-28 08:00:00+02:00",
            "2026-09-28 08:00:00.000",
            "2026-09-28 08:00",
            "2026-09-28 8:00:00",
            "2026-09-28",
            "",
        ] {
            assert_eq!(refusal::<EventWallTime>(&json!(text)), form, "{text:?}");
        }
        for (text, start) in [
            ("2026-09-28 24:00:00", "24:00:00 is not a time of day"),
            ("2026-09-28 23:59:60", "23:59:60 is not a time of day"),
            (
                "2026-02-30 08:00:00",
                "2026-02-30 is not a day of the calendar",
            ),
        ] {
            let message = refusal::<EventWallTime>(&json!(text));
            assert!(message.starts_with(start), "{text}: {message}");
        }
    }

    #[test]
    fn values_that_are_not_strings_are_refused() {
        for value in [
            json!(20_260_901),
            json!(1.5),
            json!(true),
            Value::Null,
            json!(["2026-09-01"]),
            json!({ "date": "2026-09-01" }),
        ] {
            let message = refusal::<CalendarDate>(&value);
            assert!(
                message.ends_with("expected a calendar day in the form YYYY-MM-DD"),
                "{message}"
            );
            let message = refusal::<EventWallTime>(&value);
            let expected = "expected a Europe/Budapest wall time in the form YYYY-MM-DD HH:MM:SS";
            assert!(message.ends_with(expected), "{message}");
        }
    }

    #[test]
    fn a_refused_text_is_not_quoted_in_the_error() {
        let long = format!("2026-09-01{}", "<b>".repeat(1_000));
        for message in [
            refusal::<CalendarDate>(&json!(long)),
            refusal::<EventWallTime>(&json!(long)),
        ] {
            assert!(message.len() < 120 && !message.contains("<b>"), "{message}");
        }
    }

    #[test]
    fn the_constructors_check_the_form() {
        let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("a day");
        assert_eq!(
            CalendarDate::new(date(2026, 9, 1)).map(CalendarDate::date),
            Ok(date(2026, 9, 1))
        );
        assert_eq!(
            CalendarDate::new(date(0, 1, 1)),
            Err(D::YearOutOfRange { year: 0 })
        );
        assert_eq!(
            CalendarDate::new(date(10_000, 1, 1)),
            Err(D::YearOutOfRange { year: 10_000 })
        );

        let at = |seconds, nanos| {
            let time = NaiveTime::from_num_seconds_from_midnight_opt(seconds, nanos);
            date(2026, 6, 30).and_time(time.expect("a time"))
        };
        assert_eq!(
            EventWallTime::new(at(28_800, 0)).map(EventWallTime::wall),
            Ok(at(28_800, 0))
        );
        let fraction = at(28_800, 500_000_000);
        assert_eq!(
            EventWallTime::new(fraction),
            Err(D::FractionOfSecond { wall: fraction })
        );
        let leap = at(86_399, 1_000_000_000);
        assert_eq!(EventWallTime::new(leap), Err(D::LeapSecond { wall: leap }));
    }

    #[test]
    fn a_calendar_date_converts_to_and_from_its_budapest_midnight() {
        for (text, midnight) in [
            ("2026-09-01", "2026-08-31T22:00:00Z"),
            ("2026-01-15", "2026-01-14T23:00:00Z"),
            // The change days of 2026 and the year boundary.
            ("2026-03-29", "2026-03-28T23:00:00Z"),
            ("2026-10-25", "2026-10-24T22:00:00Z"),
            ("2027-01-01", "2026-12-31T23:00:00Z"),
            ("2028-02-29", "2028-02-28T23:00:00Z"),
        ] {
            assert_eq!(day(text).to_midnight(), Ok(utc(midnight)), "{text}");
            assert_eq!(
                CalendarDate::from_midnight(utc(midnight)),
                Ok(day(text)),
                "{midnight}"
            );
            assert_eq!(
                json_of(CalendarDate::from_midnight(utc(midnight)).expect("a day")),
                json!(text)
            );
        }
    }

    #[test]
    fn a_stored_value_that_is_no_budapest_midnight_is_reported() {
        // A UTC midnight: the shape that dated the legacy posts a day early
        // (ADR 0010, Context).
        let instant = utc("2026-09-01T00:00:00Z");
        let wall = budapest::instant_to_wall(instant).expect("a wall time");
        let expected = WireTimeError::Conversion(B::NotMidnight { instant, wall });
        assert_eq!(CalendarDate::from_midnight(instant), Err(expected));
        // A Budapest midnight in year 10000 has no four-digit form.
        let year_10000 = NaiveDate::from_ymd_opt(10_000, 1, 1).expect("a day chrono holds");
        let late = budapest::date_to_midnight(year_10000).expect("a midnight");
        let error = WireTimeError::Form(D::YearOutOfRange { year: 10_000 });
        assert_eq!(CalendarDate::from_midnight(late), Err(error));
        // A day without a Budapest midnight is read, and only its conversion
        // fails.
        let skipped = day("1981-03-29");
        assert_eq!(
            skipped.to_midnight(),
            Err(B::NoMidnight {
                date: skipped.date()
            })
        );
    }

    #[test]
    fn an_event_wall_time_converts_to_and_from_its_instant() {
        for (text, instant) in [
            // 23:30 in winter and in summer (ADR 0010, "What must be true").
            ("2026-01-15 23:30:00", "2026-01-15T22:30:00Z"),
            ("2026-07-15 23:30:00", "2026-07-15T21:30:00Z"),
            ("2026-09-28 08:00:00", "2026-09-28T06:00:00Z"),
            ("2019-03-15 00:00:52", "2019-03-14T23:00:52Z"),
            ("2026-03-29 03:00:00", "2026-03-29T01:00:00Z"),
            ("2026-12-31 23:59:59", "2026-12-31T22:59:59Z"),
            // The repeated autumn hour converts to its first occurrence.
            ("2026-10-25 02:30:00", "2026-10-25T00:30:00Z"),
        ] {
            assert_eq!(wall(text).to_instant(), Ok(utc(instant)), "{text}");
            assert_eq!(
                EventWallTime::from_instant(utc(instant)),
                Ok(wall(text)),
                "{instant}"
            );
        }
        // Both instants of the repeated hour show the same wall time.
        let later = EventWallTime::from_instant(utc("2026-10-25T01:30:00Z"));
        assert_eq!(later, Ok(wall("2026-10-25 02:30:00")));
    }

    #[test]
    fn a_wall_time_in_the_spring_gap_is_read_but_has_no_instant() {
        let skipped: EventWallTime =
            serde_json::from_value(json!("2026-03-29 02:30:00")).expect("in the form");
        let expected = B::NonexistentWallTime {
            wall: skipped.wall(),
        };
        assert_eq!(skipped.to_instant(), Err(expected));
    }

    #[test]
    fn a_stored_instant_with_a_fraction_of_a_second_is_reported() {
        let instant = utc("2026-09-28T06:00:00.25Z");
        let wall = budapest::instant_to_wall(instant).expect("a wall time");
        let expected = WireTimeError::Form(D::FractionOfSecond { wall });
        assert_eq!(EventWallTime::from_instant(instant), Err(expected));
        let leap = utc("2016-12-31T23:59:60Z");
        let value = leap.naive_utc();
        let expected = WireTimeError::Conversion(B::LeapSecond { value });
        assert_eq!(EventWallTime::from_instant(leap), Err(expected));
    }

    #[test]
    fn the_typescript_type_of_both_is_string() {
        let config = ts_rs::Config::default();
        assert_eq!(CalendarDate::decl(&config), "type CalendarDate = string;");
        assert_eq!(EventWallTime::decl(&config), "type EventWallTime = string;");
        assert_eq!(CalendarDate::inline(&config), "string");
        assert_eq!(EventWallTime::inline(&config), "string");
    }

    /// Days of the years that the form writes.
    fn days() -> impl Strategy<Value = CalendarDate> {
        let first = NaiveDate::from_ymd_opt(1, 1, 1)
            .expect("a day")
            .num_days_from_ce();
        let last = NaiveDate::from_ymd_opt(9999, 12, 31)
            .expect("a day")
            .num_days_from_ce();
        (first..=last).prop_map(|days| {
            let date = NaiveDate::from_num_days_from_ce_opt(days).expect("a day in range");
            CalendarDate::new(date).expect("a year in range")
        })
    }

    /// Whole-second wall times of the years that the form writes.
    fn walls() -> impl Strategy<Value = EventWallTime> {
        (days(), 0u32..86_400).prop_map(|(day, seconds)| {
            let time = NaiveTime::from_num_seconds_from_midnight_opt(seconds, 0).expect("a time");
            EventWallTime::new(day.date().and_time(time)).expect("a whole second")
        })
    }

    proptest! {
        // ADR 0005 §3: the serialized form is pinned, and reading it back
        // gives the same value.
        #[test]
        fn every_calendar_date_round_trips_through_json(date in days()) {
            let value = json_of(date);
            let text = value.as_str().expect("a JSON string");
            prop_assert_eq!(text.len(), 10);
            prop_assert_eq!(text, format!("{:04}-{:02}-{:02}", date.date().year(), date.date().month(), date.date().day()));
            prop_assert_eq!(serde_json::from_value::<CalendarDate>(value.clone()).ok(), Some(date));
        }

        #[test]
        fn every_event_wall_time_round_trips_through_json(w in walls()) {
            let value = json_of(w);
            let text = value.as_str().expect("a JSON string");
            let n = w.wall();
            prop_assert_eq!(text, format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                n.year(), n.month(), n.day(), n.hour(), n.minute(), n.second()
            ));
            prop_assert_eq!(serde_json::from_value::<EventWallTime>(value.clone()).ok(), Some(w));
        }

        // Only the one text of a value is read: any string that is read is
        // written back unchanged.
        #[test]
        fn only_canonical_strings_are_read(
            text in prop_oneof![
                any::<String>(),
                "[0-9]{4}-[0-9]{2}-[0-9]{2}( [0-9]{2}:[0-9]{2}:[0-9]{2})?",
                "[0-9T:+. Z-]{10,25}",
            ]
        ) {
            if let Ok(date) = serde_json::from_value::<CalendarDate>(json!(text)) {
                prop_assert_eq!(json_of(date), json!(text));
            }
            if let Ok(w) = serde_json::from_value::<EventWallTime>(json!(text)) {
                prop_assert_eq!(json_of(w), json!(text));
            }
        }

        // Every day converts to its Budapest midnight and back, except the
        // few days before 1984 on which the clocks skipped 00:00.
        #[test]
        fn every_calendar_date_round_trips_through_its_midnight(date in days()) {
            match date.to_midnight() {
                Ok(midnight) => prop_assert_eq!(CalendarDate::from_midnight(midnight), Ok(date)),
                Err(error) => prop_assert!(
                    matches!(error, B::NoMidnight { .. }) && date.date().year() < 1984,
                    "{}: {}", date, error
                ),
            }
        }

        // ADR 0010 point 6: every wall time outside the spring gap converts
        // to an instant and back to itself; a gap time is refused, and the
        // same time an hour later exists.
        #[test]
        fn every_existing_event_wall_time_round_trips_through_its_instant(w in walls()) {
            match w.to_instant() {
                Ok(instant) => prop_assert_eq!(EventWallTime::from_instant(instant), Ok(w)),
                Err(B::NonexistentWallTime { wall }) => {
                    prop_assert_eq!(wall, w.wall());
                    let later = EventWallTime::new(w.wall() + TimeDelta::hours(1));
                    prop_assert!(later.is_ok_and(|later| later.to_instant().is_ok()), "{}", w);
                }
                Err(other) => return Err(TestCaseError::fail(format!("{w}: {other:?}"))),
            }
        }
    }
}
