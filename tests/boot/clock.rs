//! The app clock of ADR 0010 point 7: boot registers the system clock, a
//! replaced clock reaches copies of the context taken before the change, and
//! `with_app_at` and `with_server_at` start a test at a chosen instant that
//! moves only when the test moves it. No route reads the clock yet, so the
//! first time-dependent endpoint adds the request test that shows a handler
//! sees the test clock.

use std::sync::Arc;

use axum::http::StatusCode;
use backend3::{
    app::{clock, set_clock},
    domain::clock::{Clock, FixedClock},
};
use chrono::{DateTime, TimeDelta, Utc};
use proptest::prelude::*;

use crate::support::{TestClock, with_app, with_app_at, with_server_at};

/// Parses an RFC 3339 instant written in a test.
fn instant(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .unwrap_or_else(|error| panic!("{rfc3339}: {error}"))
        .with_timezone(&Utc)
}

#[tokio::test]
async fn the_booted_app_reads_the_system_clock() {
    // One of the two reads of the wall clock that recipe 7 allows in tests:
    // only a reading of the real time shows that a context Loco boots
    // carries the system clock.
    with_app(|_router, ctx| async move {
        let app_clock = clock(&ctx).expect("after_context registers a clock");
        let before = Utc::now();
        let now = app_clock.now();
        let after = Utc::now();
        assert!(
            before <= now && now <= after,
            "{before} <= {now} <= {after}"
        );
        assert_eq!(format!("{app_clock:?}"), "SystemClock");
    })
    .await;
}

#[tokio::test]
async fn a_replaced_clock_reaches_earlier_copies_of_the_context() {
    with_app(|_router, ctx| async move {
        // The router's state, each worker and each task hold such a copy,
        // taken at boot; every copy shares one `Arc<SharedStore>`.
        let earlier_copy = ctx.clone();
        let fixed = instant("2026-10-25T00:30:00Z");
        set_clock(&ctx, Arc::new(FixedClock(fixed)));
        assert_eq!(clock(&earlier_copy).expect("a clock").now(), fixed);
        assert_eq!(clock(&ctx).expect("a clock").now(), fixed);
    })
    .await;
}

#[tokio::test]
async fn with_app_at_starts_at_the_instant_and_moves_only_on_demand() {
    let start = instant("2026-03-29T00:30:00Z");
    with_app_at(start, |_router, ctx, test_clock| async move {
        let app_clock = clock(&ctx).expect("a clock");
        assert_eq!(app_clock.now(), start);
        assert_eq!(app_clock.now(), start, "the clock stands still");

        let later = test_clock.advance(TimeDelta::minutes(110));
        assert_eq!(later, start + TimeDelta::minutes(110));
        assert_eq!(app_clock.now(), later);
        assert_eq!(clock(&ctx).expect("a clock").now(), later);

        let earlier = instant("2025-12-31T23:59:59Z");
        test_clock.set(earlier);
        assert_eq!(app_clock.now(), earlier);
    })
    .await;
}

#[tokio::test]
async fn with_server_at_installs_the_test_clock_behind_a_running_server() {
    let start = instant("2026-09-01T06:00:00Z");
    with_server_at(start, |server, ctx, test_clock| async move {
        assert_eq!(server.get("/_ping").await.status_code(), StatusCode::OK);
        assert_eq!(clock(&ctx).expect("a clock").now(), start);

        test_clock.advance(TimeDelta::hours(2));
        assert_eq!(server.get("/_ping").await.status_code(), StatusCode::OK);
        assert_eq!(
            clock(&ctx).expect("a clock").now(),
            start + TimeDelta::hours(2)
        );
    })
    .await;
}

#[test]
#[should_panic(expected = "use TestClock::set to move the clock back")]
fn advancing_backwards_is_refused() {
    TestClock::new(instant("2026-10-25T01:30:00Z")).advance(TimeDelta::seconds(-1));
}

#[test]
#[should_panic(expected = "is outside chrono's range")]
fn advancing_past_the_end_of_time_is_refused() {
    TestClock::new(DateTime::<Utc>::MAX_UTC).advance(TimeDelta::nanoseconds(1));
}

proptest! {
    #[test]
    fn advancing_adds_exactly_the_step(
        start_seconds in -10_000_000_000i64..10_000_000_000,
        // Zero is a value of its own, so the identity step is always covered.
        step_seconds in prop_oneof![Just(0i64), 0i64..10_000_000_000],
        step_nanos in prop_oneof![Just(0i64), 0i64..1_000_000_000],
    ) {
        let start = DateTime::from_timestamp(start_seconds, 0).expect("in range");
        let step = TimeDelta::seconds(step_seconds) + TimeDelta::nanoseconds(step_nanos);
        let test_clock = TestClock::new(start);
        let moved = test_clock.advance(step);
        prop_assert_eq!(moved, start + step);
        prop_assert_eq!(test_clock.now(), moved);
    }
}
