//! Europe/Budapest wall times, calendar days and instants (ADR 0010).
//!
//! backend3 stores every moment as a UTC `timestamptz`, and a calendar day as
//! the instant of its Budapest midnight (ADR 0010 points 1 and 2), while people
//! work in Budapest wall times and days. This module is the one place that
//! converts between the two (point 5): a hand-rolled conversion is a silent
//! one- or two-hour error, so the rest of backend3 calls these functions, never
//! chrono's local-time API. ADR 0008's illustrative layout calls it `time.rs`.
//!
//! - The zone is the constant [`ZONE`], never the process `TZ` (point 4), and
//!   "now" is an argument of [`today`] (ADR 0008 rule 1).
//! - A wall time ([`NaiveDateTime`]) that the spring change skips is an error.
//!   One that the autumn change repeats takes the earlier instant, `+02:00`
//!   (point 6), so every wall time that exists converts to an instant
//!   ([`DateTime<Utc>`]) and back unchanged.
//! - Fractions of a second are kept: PostgreSQL stores microseconds, and the
//!   wire formats of ADR 0010 carry whole seconds. A leap second, which chrono
//!   parses, is refused both ways: PostgreSQL would store `12:34:60` as
//!   `12:35:00`.
//! - Near the ends of chrono's range the functions return an error, not panic.
//! - A calendar day that is to be stored converts through
//!   [`date_to_stored_midnight`], which refuses every day after
//!   [`CHRONO_TZ_LAST_CHANGE_YEAR`] (ADR 0041 §2). [`date_to_midnight`]
//!   converts every day, for bounds that are compared but never stored.
//!
//! Since 1984 the clocks change at 02:00 and 03:00, so every day has exactly
//! one midnight. Under older rules they changed at midnight on a few days of
//! 1916-1983: such a day has no midnight ([`BudapestTimeError::NoMidnight`]) or
//! two, and [`date_to_midnight`] takes the earlier.

use chrono::{
    DateTime, Datelike, MappedLocalTime, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone,
    Timelike, Utc,
};
use chrono_tz::Tz;

/// The school's time zone (ADR 0010 point 4).
pub const ZONE: Tz = chrono_tz::Europe::Budapest;

/// The IANA name of [`ZONE`], for SQL that derives a Budapest value from a
/// `timestamptz`, such as `(starts_at AT TIME ZONE 'Europe/Budapest')::date`
/// (ADR 0010 point 10).
///
/// SQL never turns a wall time or a day into an instant (point 5): its
/// `timestamp ... AT TIME ZONE` takes the later instant of a repeated wall time
/// and moves a skipped one an hour on, against point 6. Such values go through
/// [`wall_to_instant`] or [`date_to_midnight`] and are bound as instants.
pub const ZONE_NAME: &str = "Europe/Budapest";

/// The IANA tz data version compiled into chrono-tz, which these conversions
/// use (ADR 0010 point 5).
///
/// New tz data can change Hungary's rules and so the Budapest wall time of a
/// stored instant (ADR 0010, Consequences). A unit test compares this with
/// chrono-tz's data, so a bump that brings new data fails until it records the
/// version and follows the tz-data procedure of ADR 0041 §3. PostgreSQL's `AT
/// TIME ZONE` and the midnight check of point 2 use the database's own tz
/// data, which comes with the PostgreSQL image and is versioned separately;
/// `tests/tz_parity/mod.rs` compares the Budapest midnights of the two and pins
/// backend3's in a committed snapshot (ADR 0041 §2).
pub const TZ_DATA_VERSION: &str = "2025b";

/// The last year in which chrono-tz lists a Budapest clock change: 2099 in
/// chrono-tz 0.10.4.
///
/// chrono-tz turns the tz data's rules into a table of offset changes when it
/// is built, and for Budapest that table ends with the change of 2099-10-25.
/// It keeps winter time (`+01:00`) for every later instant, while the tz data
/// goes on changing the clocks each spring and autumn, and so does
/// PostgreSQL, which reads the tz data's rules at run time. From 2100 on, a
/// summer day's midnight from [`date_to_midnight`] is therefore one hour later
/// than PostgreSQL's, and the check of ADR 0010 point 2 would refuse it. The
/// tz-data parity test compares the two sides up to the end of this year, and
/// [`date_to_stored_midnight`] refuses every later day, so backend3 stores
/// only days that the test compares (ADR 0041 §2). A unit test fails when
/// chrono-tz's table ends in another year.
pub const CHRONO_TZ_LAST_CHANGE_YEAR: i32 = 2099;

/// chrono marks a leap second by a nanosecond field of one second or more.
const LEAP_SECOND_NANOS: u32 = 1_000_000_000;

/// Why a conversion failed.
///
/// The `Display` text names the value and, where someone can act on it, what
/// to do. Callers map the variants to their own errors: the admin API to a
/// validation error on the field, the importer to a refusal that names the row
/// (ADR 0010 point 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BudapestTimeError {
    /// The wall time lies in a spring gap: the clocks skip it.
    #[error(
        "{wall} does not exist in Europe/Budapest: the clocks skip it when summer time starts \
         (today 02:00 to 02:59 on the last Sunday of March). Enter a time outside that hour."
    )]
    NonexistentWallTime {
        /// The wall time as given.
        wall: NaiveDateTime,
    },
    /// The day has no Budapest midnight, so it cannot be stored (before 1984
    /// only).
    #[error("{date} has no Budapest midnight (the clocks skipped 00:00), so it cannot be stored.")]
    NoMidnight {
        /// The calendar day as given.
        date: NaiveDate,
    },
    /// The day lies after 31 December of [`CHRONO_TZ_LAST_CHANGE_YEAR`],
    /// where chrono-tz's Budapest table ends, so it is not stored (ADR 0041
    /// §2). Winter days are refused too, so the bound is one year boundary.
    #[error(
        "{date} is after {last}-12-31, the last calendar day that can be stored. Check the year.",
        last = CHRONO_TZ_LAST_CHANGE_YEAR
    )]
    AfterTzTable {
        /// The calendar day as given.
        date: NaiveDate,
    },
    /// The instant is not a Budapest midnight, so it stands for no calendar
    /// day.
    #[error("{instant} is {wall} in Budapest, not the midnight that a calendar-day value holds.")]
    NotMidnight {
        /// The instant as given.
        instant: DateTime<Utc>,
        /// The Budapest wall time of `instant`.
        wall: NaiveDateTime,
    },
    /// The value is a leap second (seconds 60), which backend3 does not store.
    #[error("{value} has seconds 60, a leap second, which backend3 does not store. Use 00 to 59.")]
    LeapSecond {
        /// The wall time as given, or the UTC date and time of the instant.
        value: NaiveDateTime,
    },
    /// The value is so close to an end of chrono's range (years -262143 and
    /// 262142) that its conversion cannot be represented.
    #[error("{value} is too close to an end of the representable range to convert.")]
    OutOfRange {
        /// The wall time as given, or the UTC date and time of the instant.
        value: NaiveDateTime,
    },
}

/// Converts a Europe/Budapest wall time to its instant (ADR 0010 point 6).
///
/// - `wall`: the local date and time, for example an event's start as an
///   editor typed it or as the legacy database holds it.
///
/// # Errors
///
/// [`BudapestTimeError::NonexistentWallTime`] when the spring change skips
/// `wall` (`2026-03-29 02:30:00`), [`BudapestTimeError::LeapSecond`], and
/// [`BudapestTimeError::OutOfRange`] at the start of chrono's range.
pub fn wall_to_instant(wall: NaiveDateTime) -> Result<DateTime<Utc>, BudapestTimeError> {
    refuse_leap_second(wall)?;
    // chrono-tz lists the instants of a repeated wall time in time order, so
    // `earliest` is the CEST one, `+02:00`; the 2026-10-25 rows pin it.
    if let Some(instant) = ZONE.from_local_datetime(&wall).earliest() {
        return Ok(instant.to_utc());
    }
    // `None` also means that the instant would precede chrono's first date;
    // only the offset lookup tells that from a real gap.
    Err(match ZONE.offset_from_local_datetime(&wall) {
        MappedLocalTime::None => BudapestTimeError::NonexistentWallTime { wall },
        _ => BudapestTimeError::OutOfRange { value: wall },
    })
}

/// Converts an instant to its Europe/Budapest wall time. In the repeated
/// autumn hour two instants show one wall time.
///
/// - `instant`: any instant, for example a stored `events.starts_at`.
///
/// # Errors
///
/// [`BudapestTimeError::LeapSecond`], and [`BudapestTimeError::OutOfRange`] in
/// the last hours of chrono's range.
pub fn instant_to_wall(instant: DateTime<Utc>) -> Result<NaiveDateTime, BudapestTimeError> {
    let utc = instant.naive_utc();
    refuse_leap_second(utc)?;
    // `instant.with_timezone(&ZONE).naive_local()` would panic at the end of
    // chrono's range; adding the offset by hand reports it instead.
    utc.checked_add_offset(ZONE.offset_from_utc_datetime(&utc).fix())
        .ok_or(BudapestTimeError::OutOfRange { value: utc })
}

/// Converts a calendar day to the instant of its Budapest midnight, the form in
/// which every calendar-day column stores it (ADR 0010 point 2).
///
/// - `date`: the calendar day, for example the first day of a public month.
///
/// A day that is to be stored goes through [`date_to_stored_midnight`]
/// instead; this function converts every year, for bounds that are compared
/// with stored instants, such as the public month and grid bounds.
///
/// # Errors
///
/// [`BudapestTimeError::NoMidnight`] for a day on which the clocks skipped
/// 00:00, and [`BudapestTimeError::OutOfRange`] for chrono's first day.
pub fn date_to_midnight(date: NaiveDate) -> Result<DateTime<Utc>, BudapestTimeError> {
    wall_to_instant(date.and_time(NaiveTime::MIN)).map_err(|error| match error {
        BudapestTimeError::NonexistentWallTime { .. } => BudapestTimeError::NoMidnight { date },
        other => other,
    })
}

/// Converts a calendar day that is to be stored to the instant of its
/// Budapest midnight: [`date_to_midnight`], for the days up to 31 December
/// of [`CHRONO_TZ_LAST_CHANGE_YEAR`] (ADR 0041 §2).
///
/// - `date`: the day to store, for example a post's date from the admin
///   form or a legacy `posts_data.date` in the importer.
///
/// chrono-tz's Budapest table ends in that year, and after it chrono-tz
/// keeps winter time while PostgreSQL goes on applying the summer-time rule,
/// so the check of ADR 0010 point 2 would refuse the midnight of each later
/// summer day as a database error. Every later day is refused here instead,
/// winter days included, so the rule is one year boundary, and the admin
/// answers it on the field and the importer refuses the row (point 6).
///
/// # Errors
///
/// [`BudapestTimeError::AfterTzTable`] for a day after that year, and the
/// errors of [`date_to_midnight`].
pub fn date_to_stored_midnight(date: NaiveDate) -> Result<DateTime<Utc>, BudapestTimeError> {
    if date.year() > CHRONO_TZ_LAST_CHANGE_YEAR {
        return Err(BudapestTimeError::AfterTzTable { date });
    }
    date_to_midnight(date)
}

/// Converts the stored Budapest midnight of a calendar day back to the day.
///
/// - `instant`: a calendar-day value as stored, for example `posts.date`.
///
/// The database refuses any other value in a calendar-day column (ADR 0010
/// point 2), so another instant here is a defect: it is reported, not rounded.
///
/// # Errors
///
/// [`BudapestTimeError::NotMidnight`] when `instant` is not 00:00:00 in
/// Budapest, and the errors of [`instant_to_wall`].
pub fn midnight_to_date(instant: DateTime<Utc>) -> Result<NaiveDate, BudapestTimeError> {
    let wall = instant_to_wall(instant)?;
    if wall.time() != NaiveTime::MIN {
        return Err(BudapestTimeError::NotMidnight { instant, wall });
    }
    Ok(wall.date())
}

/// The Europe/Budapest calendar day that contains `now` (ADR 0010 point 7).
///
/// - `now`: the current instant from the caller's injected clock, never read
///   here (ADR 0008 rule 1); any other instant gives its Budapest day too.
///
/// # Errors
///
/// The errors of [`instant_to_wall`].
pub fn today(now: DateTime<Utc>) -> Result<NaiveDate, BudapestTimeError> {
    instant_to_wall(now).map(|wall| wall.date())
}

/// Whether `instant` is exactly 00:00:00 in Budapest: the check that every
/// stored calendar-day value must pass (ADR 0010 point 2).
///
/// - `instant`: the instant to check.
#[must_use]
pub fn is_budapest_midnight(instant: DateTime<Utc>) -> bool {
    midnight_to_date(instant).is_ok()
}

/// Refuses a leap second, which chrono can hold but backend3 does not store.
fn refuse_leap_second(value: NaiveDateTime) -> Result<(), BudapestTimeError> {
    if value.nanosecond() >= LEAP_SECOND_NANOS {
        return Err(BudapestTimeError::LeapSecond { value });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use proptest::prelude::*;

    use super::*;
    use BudapestTimeError as E;

    fn wall(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").expect("a test wall time")
    }

    fn utc(text: &str) -> DateTime<Utc> {
        text.parse().expect("a test instant")
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    #[test]
    fn the_zone_and_the_recorded_tz_data() {
        assert_eq!(ZONE.name(), ZONE_NAME);
        assert_eq!(
            chrono_tz::IANA_TZDB_VERSION,
            TZ_DATA_VERSION,
            "chrono-tz compiles in other tz data than TZ_DATA_VERSION records: follow the tz-data \
             procedure of ADR 0041 §3, then update TZ_DATA_VERSION and the chrono-tz comment in \
             Cargo.toml"
        );
    }

    #[test]
    fn chrono_tz_lists_clock_changes_until_the_recorded_year() {
        // Mid-July is summer time in every year whose changes chrono-tz
        // lists, from 1980 on, and winter time after its table ends.
        let summer_time_in = |year: i32| {
            let noon = NaiveDate::from_ymd_opt(year, 7, 15)
                .and_then(|day| day.and_hms_opt(12, 0, 0))
                .expect("mid-July noon");
            ZONE.offset_from_utc_datetime(&noon).fix().local_minus_utc() == 7_200
        };
        let last = (1980..=crate::domain::date_text::LAST_YEAR).rfind(|&year| summer_time_in(year));
        assert_eq!(
            last,
            Some(CHRONO_TZ_LAST_CHANGE_YEAR),
            "the last year with Budapest summer time in chrono-tz is no longer \
             CHRONO_TZ_LAST_CHANGE_YEAR: chrono-tz's table ends in another year, or new tz data \
             changed Hungary's rules. Follow the tz-data procedure of ADR 0041 §3 and set the \
             constant, which also moves the end of the parity test in tests/tz_parity/mod.rs"
        );
        let without = (1980..=CHRONO_TZ_LAST_CHANGE_YEAR).find(|&year| !summer_time_in(year));
        assert_eq!(
            without, None,
            "a year without summer time: Hungary's rules changed"
        );
    }

    #[test]
    fn wall_times_convert_to_their_instants() {
        for (w, instant) in [
            // Evenings in winter and summer, picker seconds and fractions.
            ("2026-01-15 23:30:00", "2026-01-15T22:30:00Z"),
            ("2026-07-15 23:30:00", "2026-07-15T21:30:00Z"),
            ("2019-05-06 00:00:52", "2019-05-05T22:00:52Z"),
            ("2026-07-15 08:00:00.25", "2026-07-15T06:00:00.25Z"),
            // 2026-03-29: the clocks skip 02:00 to 02:59 (an empty instant).
            ("2026-03-29 01:59:59", "2026-03-29T00:59:59Z"),
            ("2026-03-29 02:00:00", ""),
            ("2026-03-29 02:59:59.999999999", ""),
            ("2026-03-29 03:00:00", "2026-03-29T01:00:00Z"),
            // 2026-10-25: 02:00 to 02:59 repeats and takes the earlier instant.
            ("2026-10-25 02:00:00", "2026-10-25T00:00:00Z"),
            ("2026-10-25 02:59:59", "2026-10-25T00:59:59Z"),
            ("2026-10-25 03:00:00", "2026-10-25T02:00:00Z"),
            // The year boundary and a leap day.
            ("2026-12-31 23:59:59", "2026-12-31T22:59:59Z"),
            ("2028-02-29 23:59:59", "2028-02-29T22:59:59Z"),
        ] {
            let expected = match instant {
                "" => Err(E::NonexistentWallTime { wall: wall(w) }),
                _ => Ok(utc(instant)),
            };
            assert_eq!(wall_to_instant(wall(w)), expected, "{w}");
        }
        let wall = wall("2026-03-29 02:30:00");
        let message = E::NonexistentWallTime { wall }.to_string();
        assert!(message.starts_with("2026-03-29 02:30:00 does not exist"));
    }

    #[test]
    fn instants_convert_to_their_wall_times_and_days() {
        for (instant, w) in [
            ("2026-01-15T22:30:00Z", "2026-01-15 23:30:00"),
            ("2026-07-15T21:30:00Z", "2026-07-15 23:30:00"),
            ("2026-03-29T01:00:00Z", "2026-03-29 03:00:00"),
            // Both instants of the repeated hour show the same wall time.
            ("2026-10-25T00:30:00Z", "2026-10-25 02:30:00"),
            ("2026-10-25T01:30:00Z", "2026-10-25 02:30:00"),
            ("2026-12-31T22:59:59Z", "2026-12-31 23:59:59"),
            // Budapest is already in 2027 while UTC is still in 2026.
            ("2026-12-31T23:00:00Z", "2027-01-01 00:00:00"),
            ("2028-02-29T22:59:59Z", "2028-02-29 23:59:59"),
        ] {
            assert_eq!(instant_to_wall(utc(instant)), Ok(wall(w)), "{instant}");
            assert_eq!(today(utc(instant)), Ok(wall(w).date()), "{instant}");
        }
    }

    #[test]
    fn days_convert_to_their_midnights_and_back() {
        for (day, midnight) in [
            // The change days of 2026 are 23 and 25 hours long.
            ("2026-03-29", "2026-03-28T23:00:00Z"),
            ("2026-03-30", "2026-03-29T22:00:00Z"),
            ("2026-10-25", "2026-10-24T22:00:00Z"),
            ("2026-10-26", "2026-10-25T23:00:00Z"),
            ("2027-01-01", "2026-12-31T23:00:00Z"),
            ("2000-02-29", "2000-02-28T23:00:00Z"),
            ("2024-02-29", "2024-02-28T23:00:00Z"),
            ("2028-02-29", "2028-02-28T23:00:00Z"),
            ("2028-03-01", "2028-02-29T23:00:00Z"),
            // A day with two midnights converts to the earlier.
            ("1981-09-27", "1981-09-26T22:00:00Z"),
        ] {
            assert_eq!(date_to_midnight(date(day)), Ok(utc(midnight)), "{day}");
            assert_eq!(midnight_to_date(utc(midnight)), Ok(date(day)), "{midnight}");
            assert!(is_budapest_midnight(utc(midnight)), "{midnight}");
        }
        let later_twin = utc("1981-09-26T23:00:00Z");
        assert_eq!(midnight_to_date(later_twin), Ok(date("1981-09-27")));
    }

    #[test]
    fn values_that_are_no_midnights() {
        // A UTC midnight: the shape that dated legacy posts one day early
        // (ADR 0010, Context).
        let (instant, wall) = (utc("2026-03-29T00:00:00Z"), wall("2026-03-29 01:00:00"));
        let error = E::NotMidnight { instant, wall };
        assert_eq!(midnight_to_date(instant), Err(error));
        assert!(!is_budapest_midnight(instant));
        assert!(!is_budapest_midnight(utc("2026-03-28T23:00:00.000001Z")));
        let date = date("1981-03-29");
        assert_eq!(date_to_midnight(date), Err(E::NoMidnight { date }));
    }

    #[test]
    fn a_stored_day_ends_with_the_last_year_of_chrono_tzs_table() {
        let year = CHRONO_TZ_LAST_CHANGE_YEAR;
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("a day");
        let at = |d: NaiveDate, h| d.and_hms_opt(h, 0, 0).expect("an hour").and_utc();
        // The last day stored, in winter time, and a summer day of its year.
        let last = day(year, 12, 31);
        assert_eq!(date_to_stored_midnight(last), Ok(at(day(year, 12, 30), 23)));
        let summer = day(year, 7, 1);
        assert_eq!(
            date_to_stored_midnight(summer),
            Ok(at(day(year, 6, 30), 22))
        );
        // The first day refused, a winter day after it (on which chrono-tz
        // and PostgreSQL agree), a summer day and the last year of the forms.
        for date in [
            day(year + 1, 1, 1),
            day(year + 1, 1, 15),
            day(year + 1, 7, 1),
            day(9999, 12, 31),
        ] {
            assert_eq!(
                date_to_stored_midnight(date),
                Err(E::AfterTzTable { date }),
                "{date}"
            );
            assert!(date_to_midnight(date).is_ok(), "{date}");
        }
        // The other refusals stay those of date_to_midnight.
        let skipped = date("1981-03-29");
        assert_eq!(
            date_to_stored_midnight(skipped),
            Err(E::NoMidnight { date: skipped })
        );
        let first = day(year + 1, 1, 1);
        assert_eq!(
            E::AfterTzTable { date: first }.to_string(),
            format!(
                "{first} is after {year}-12-31, the last calendar day that can be stored. Check the year."
            )
        );
    }

    #[test]
    fn leap_seconds_are_refused_both_ways() {
        let parsed = NaiveDateTime::parse_from_str("2026-03-15 12:34:60", "%Y-%m-%d %H:%M:%S");
        let value = parsed.expect("chrono parses seconds 60 as a leap second");
        assert_eq!(wall_to_instant(value), Err(E::LeapSecond { value }));
        let instant = utc("2026-03-28T22:59:60Z");
        let value = instant.naive_utc();
        assert_eq!(instant_to_wall(instant), Err(E::LeapSecond { value }));
        assert_eq!(today(instant), Err(E::LeapSecond { value }));
    }

    #[test]
    fn the_ends_of_chronos_range_are_errors_not_panics() {
        let value = NaiveDateTime::MIN;
        assert_eq!(wall_to_instant(value), Err(E::OutOfRange { value }));
        assert_eq!(date_to_midnight(value.date()), Err(E::OutOfRange { value }));
        assert!(wall_to_instant(NaiveDateTime::MAX).is_ok());
        assert!(instant_to_wall(DateTime::<Utc>::MIN_UTC).is_ok());
        let (last, value) = (DateTime::<Utc>::MAX_UTC, NaiveDateTime::MAX);
        assert_eq!(instant_to_wall(last), Err(E::OutOfRange { value }));
        assert_eq!(midnight_to_date(last), Err(E::OutOfRange { value }));
        assert!(!is_budapest_midnight(last));
    }

    /// A day from `first` to `last`.
    fn date_between(first: NaiveDate, last: NaiveDate) -> impl Strategy<Value = NaiveDate> {
        (first.num_days_from_ce()..=last.num_days_from_ce())
            .prop_map(|days| NaiveDate::from_num_days_from_ce_opt(days).expect("a day in range"))
    }

    /// The last Sunday of March or October, when the clocks change today.
    fn change_day() -> impl Strategy<Value = NaiveDate> {
        (1996i32..=2099, prop_oneof![Just(3u32), Just(10u32)]).prop_map(|(year, month)| {
            let last = NaiveDate::from_ymd_opt(year, month, 31).expect("a 31-day month");
            last - TimeDelta::days(i64::from(last.weekday().num_days_from_sunday()))
        })
    }

    /// Any day chrono can represent, a day of the years that the tz data
    /// details, or a change day.
    fn dates() -> impl Strategy<Value = NaiveDate> {
        prop_oneof![
            date_between(NaiveDate::MIN, NaiveDate::MAX),
            date_between(date("1880-01-01"), date("2110-12-31")),
            change_day(),
        ]
    }

    /// The date and time `seconds` and `nanos` after the midnight of `day`.
    fn at((day, seconds, nanos): (NaiveDate, u32, u32)) -> NaiveDateTime {
        day.and_time(NaiveTime::from_num_seconds_from_midnight_opt(seconds, nanos).expect("a time"))
    }

    /// Wall times: any time of `dates`, one from 01:00 to 04:00 on a change
    /// day, or a leap second (chrono has one only at seconds 59).
    fn walls() -> impl Strategy<Value = NaiveDateTime> {
        let leap_second = (0u32..1_440).prop_map(|minute| minute * 60 + 59);
        let leap_nanos = LEAP_SECOND_NANOS..2 * LEAP_SECOND_NANOS;
        prop_oneof![
            (dates(), 0u32..86_400, 0..LEAP_SECOND_NANOS).prop_map(at),
            (change_day(), 3_600u32..14_400, 0..LEAP_SECOND_NANOS).prop_map(at),
            (dates(), leap_second, leap_nanos).prop_map(at),
        ]
    }

    /// Instants: any of `walls` read as UTC, or one from 00:00 to 03:00 UTC on
    /// a change day (the change itself is at 01:00 UTC).
    fn instants() -> impl Strategy<Value = DateTime<Utc>> {
        let around_the_change = (change_day(), 0u32..10_800, 0..LEAP_SECOND_NANOS);
        prop_oneof![walls(), around_the_change.prop_map(at)].prop_map(|utc| utc.and_utc())
    }

    /// The failure of a property that met an error it does not expect.
    fn unexpected(error: BudapestTimeError, input: impl std::fmt::Display) -> TestCaseError {
        TestCaseError::fail(format!("unexpected {error:?} for {input}"))
    }

    proptest! {
        // ADR 0010: instant_to_wall(wall_to_instant(w)) == w for every wall
        // time outside the spring gap.
        #[test]
        fn every_existing_wall_time_round_trips(w in walls()) {
            match wall_to_instant(w) {
                Ok(instant) => prop_assert_eq!(instant_to_wall(instant), Ok(w)),
                // Every gap of the zone is one hour long.
                Err(E::NonexistentWallTime { wall }) => prop_assert!(
                    wall == w && wall_to_instant(w + TimeDelta::hours(1)).is_ok(), "{w}"
                ),
                Err(E::LeapSecond { value }) => prop_assert!(
                    value == w && w.nanosecond() >= LEAP_SECOND_NANOS, "{w}"
                ),
                Err(E::OutOfRange { value }) => prop_assert!(
                    value == w && w < NaiveDateTime::MIN + TimeDelta::hours(2), "{w}"
                ),
                Err(other) => return Err(unexpected(other, w)),
            }
        }

        // The other way: no wall time that an instant shows is refused, and a
        // repeated one converts to the earlier of its two instants.
        #[test]
        fn an_instant_round_trips_or_moves_to_the_earlier_twin(i in instants()) {
            match instant_to_wall(i) {
                Ok(w) => {
                    let back = wall_to_instant(w).map_err(|error| unexpected(error, w))?;
                    prop_assert_eq!(instant_to_wall(back), Ok(w));
                    // The twins of a repeated wall time are at most an hour apart.
                    prop_assert!(back <= i && i - back <= TimeDelta::hours(1), "{i}: {back}");
                }
                Err(E::LeapSecond { value }) => prop_assert!(
                    value == i.naive_utc() && i.nanosecond() >= LEAP_SECOND_NANOS, "{i}"
                ),
                Err(E::OutOfRange { value }) => prop_assert!(
                    value == i.naive_utc() && i > DateTime::<Utc>::MAX_UTC - TimeDelta::hours(2),
                    "{i}"
                ),
                Err(other) => return Err(unexpected(other, i)),
            }
        }

        // ADR 0010: the same round trip for every calendar day through its
        // Budapest midnight, where `today` turns from the day before to it.
        #[test]
        fn every_date_round_trips_through_its_midnight(d in dates()) {
            match date_to_midnight(d) {
                Ok(midnight) => {
                    prop_assert_eq!(midnight_to_date(midnight), Ok(d));
                    prop_assert!(is_budapest_midnight(midnight));
                    prop_assert_eq!(today(midnight), Ok(d));
                    let just_before = midnight - TimeDelta::nanoseconds(1);
                    prop_assert_eq!(today(just_before).ok(), d.pred_opt());
                }
                Err(E::NoMidnight { date }) => prop_assert!(date == d && d.year() < 1984, "{d}"),
                Err(E::OutOfRange { .. }) => prop_assert_eq!(d, NaiveDate::MIN),
                Err(other) => return Err(unexpected(other, d)),
            }
        }

        // ADR 0041 §2: a day to be stored converts as date_to_midnight
        // converts it up to the end of chrono-tz's table, and every later
        // day is refused.
        #[test]
        fn a_stored_day_converts_as_any_day_until_the_table_ends(d in dates()) {
            let expected = if d.year() > CHRONO_TZ_LAST_CHANGE_YEAR {
                Err(E::AfterTzTable { date: d })
            } else {
                date_to_midnight(d)
            };
            prop_assert_eq!(date_to_stored_midnight(d), expected);
        }

        #[test]
        fn a_midnight_moved_by_less_than_a_minute_is_not_a_midnight(
            d in dates(),
            nanos in 1i64..60_000_000_000,
            earlier in any::<bool>(),
        ) {
            let shift = TimeDelta::nanoseconds(if earlier { -nanos } else { nanos });
            if let Some(moved) = date_to_midnight(d).ok().and_then(|m| m.checked_add_signed(shift)) {
                let refused = matches!(midnight_to_date(moved), Err(E::NotMidnight { .. }));
                prop_assert!(refused && !is_budapest_midnight(moved), "{moved}");
            }
        }
    }
}
