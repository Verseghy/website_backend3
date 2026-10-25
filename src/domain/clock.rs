//! Where "now" comes from: the clock interface (ADR 0010 point 7).
//!
//! Every rule that depends on the current time must be testable at a chosen
//! instant (public-api.md §16.2). So the app context carries one [`Clock`]:
//! the system clock that `src/app.rs` registers at boot, or the settable
//! clock a test installs. Controllers, workers and tasks ask it for the
//! instant (`app::clock(&ctx)`) and pass the instant down; code here takes
//! "now" as a `DateTime<Utc>` argument and never sees a clock (ADR 0008
//! rule 1). The one read of the system time lives in `src/app.rs`, and
//! `tests/architecture.rs` fails on any other in `src/`.

use std::fmt;

use chrono::{DateTime, Utc};

/// A source of the current instant.
///
/// Shared between threads behind an `Arc` by the app context, so it must be
/// `Send + Sync`; `Debug` lets a struct that holds one derive `Debug`. Code
/// that needs "now" once takes the instant; a clock is passed only where one
/// call needs several readings, such as the session extractor of ADR 0006 §1.
pub trait Clock: fmt::Debug + Send + Sync {
    /// The current instant, in UTC, the zone of every stored instant
    /// (ADR 0010 point 1). "Today" is derived from it in Europe/Budapest by
    /// the conversion module of ADR 0010 point 5, never by the clock.
    fn now(&self) -> DateTime<Utc>;
}

/// A clock that always answers its instant, the field. For unit tests
/// inside `src/`, which cannot use the settable clock of `tests/support` and
/// must not read the system time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Any instant chrono can represent, down to the nanosecond.
    fn any_instant() -> impl Strategy<Value = DateTime<Utc>> {
        (
            DateTime::<Utc>::MIN_UTC.timestamp()..=DateTime::<Utc>::MAX_UTC.timestamp(),
            0u32..1_000_000_000,
        )
            .prop_map(|(seconds, nanos)| {
                DateTime::from_timestamp(seconds, nanos).expect("inside chrono's range")
            })
    }

    #[test]
    fn a_fixed_clock_answers_its_instant() {
        // Both 2026 transitions (00:30 UTC is the first 02:30 of the autumn
        // night), a leap day, and chrono's ends.
        for rfc3339 in [
            "2026-03-29T01:00:00Z",
            "2026-10-25T00:30:00Z",
            "2028-02-29T23:59:59.999999999Z",
        ] {
            let value = rfc3339.parse::<DateTime<Utc>>().expect("RFC 3339");
            assert_eq!(FixedClock(value).now(), value, "{value}");
        }
        for value in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
            assert_eq!(FixedClock(value).now(), value, "{value}");
        }
    }

    proptest! {
        #[test]
        fn a_fixed_clock_never_moves(value in any_instant()) {
            let clock = FixedClock(value);
            prop_assert_eq!(clock.now(), value);
            prop_assert_eq!(clock.now(), clock.now());
        }
    }
}
