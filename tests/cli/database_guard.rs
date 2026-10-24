//! An integration test that a non-local database URI in the test (and
//! development, e2e) environment fails before connecting (ADR 0024 §4,
//! layer 3).
//!
//! The probe listens on 127.0.0.2: a loopback address, so the test needs no
//! network, but not one of the addresses the guard accepts as local.

use std::time::Duration;

use crate::support::{ConnectionProbe, run_cli};

const TIMEOUT: Duration = Duration::from_secs(60);
const NON_LOCAL: &str = "127.0.0.2";

#[test]
fn local_only_environments_refuse_a_non_local_database_before_connecting() {
    for environment in ["test", "development", "e2e"] {
        let probe = ConnectionProbe::bind(NON_LOCAL);
        let url = format!(
            "postgres://loco:loco@{NON_LOCAL}:{}/backend3_test",
            probe.port()
        );

        let run = run_cli(
            &[("LOCO_ENV", environment), ("DATABASE_URL", &url)],
            &["start"],
            TIMEOUT,
        );

        assert_eq!(run.status.code(), Some(1), "{environment}: {}", run.stderr);
        assert!(
            run.stderr.contains(&format!(
                "Refusing to start: LOCO_ENV={environment}, but the database host in \
                 database.uri is '{NON_LOCAL}'."
            )),
            "{environment}: {}",
            run.stderr
        );
        assert!(
            !run.stderr.contains(":loco@"),
            "the URI is never printed: {}",
            run.stderr
        );
        assert!(
            !probe.was_connected(),
            "{environment}: connected to {NON_LOCAL}"
        );
    }
}

/// Positive control: the same address is reachable, so the absence of a
/// connection above is the guard's doing. Production has no host
/// restriction, so `db status` there does connect.
#[test]
fn the_probe_address_is_reachable_when_no_guard_applies() {
    let probe = ConnectionProbe::bind(NON_LOCAL);
    let url = format!(
        "postgres://loco:loco@{NON_LOCAL}:{}/backend3_test",
        probe.port()
    );

    let run = run_cli(
        &[
            ("LOCO_ENV", "production"),
            ("DATABASE_URL", &url),
            ("HOST", "http://localhost"),
            ("DB_CONNECT_TIMEOUT", "300"),
        ],
        &["db", "status"],
        TIMEOUT,
    );

    assert!(!run.status.success());
    assert!(
        probe.was_connected(),
        "expected a connection: {}",
        run.stderr
    );
}
