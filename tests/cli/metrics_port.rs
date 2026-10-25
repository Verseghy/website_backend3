//! `settings.metrics.port` equal to `server.port` stops every command before
//! it connects anywhere: `App::load_config` checks it (ADR 0023 §9).

use std::time::Duration;

use crate::support::{ConnectionProbe, run_cli};

#[test]
fn a_metrics_port_equal_to_the_server_port_is_refused_before_connecting() {
    // The second run is the positive control: without the clash, the same
    // command reaches the probe.
    for (metrics_port, refused) in [("9464", true), ("9465", false)] {
        let probe = ConnectionProbe::bind("127.0.0.1");
        let url = format!(
            "postgres://loco:loco@127.0.0.1:{}/backend3_test",
            probe.port()
        );
        let run = run_cli(
            &[
                ("LOCO_ENV", "development"),
                ("DATABASE_URL", &url),
                // The probe never answers the PostgreSQL handshake.
                ("DB_CONNECT_TIMEOUT", "300"),
                ("PORT", "9464"),
                ("METRICS_PORT", metrics_port),
            ],
            &["db", "status"],
            Duration::from_secs(60),
        );
        assert!(!run.status.success(), "{}", run.stderr);
        let message = "settings.metrics.port and server.port are both 9464";
        assert_eq!(run.stderr.contains(message), refused, "{}", run.stderr);
        assert_eq!(probe.was_connected(), !refused, "{}", run.stderr);
    }
}
