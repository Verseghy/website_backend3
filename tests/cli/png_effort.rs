//! `settings.image_optimisation.png` is checked before any command connects
//! (ADR 0033 §5): in production its threshold comes from the
//! `PNG_PRESET_THRESHOLD_PIXELS` variable, which the Deployment sets from the
//! measurement on the production node, and a value that breaks the rule
//! stops the process with the key and what to change.

use std::time::Duration;

use crate::support::{CLI_JWT_SECRET, ConnectionProbe, cli_totp_key, run_cli};

#[test]
fn a_png_threshold_of_zero_is_refused_before_connecting() {
    // The last two runs are the positive controls: a measured value and the
    // default reach the probe.
    for (threshold, refused) in [(Some("0"), true), (Some("8000000"), false), (None, false)] {
        let probe = ConnectionProbe::bind("127.0.0.1");
        let url = format!(
            "postgres://loco:loco@127.0.0.1:{}/backend3_test",
            probe.port()
        );
        // config/production.yaml also requires both keys, so that only the
        // PNG threshold decides the outcome. Production refuses a committed
        // TOTP key, so that one is made when the test runs (ADR 0034 §3).
        let totp_key = cli_totp_key();
        let mut env = vec![
            ("LOCO_ENV", "production"),
            ("DATABASE_URL", url.as_str()),
            ("HOST", "http://localhost"),
            ("JWT_SECRET", CLI_JWT_SECRET),
            ("TOTP_ENCRYPTION_KEY", totp_key.as_str()),
            // The probe never answers the PostgreSQL handshake.
            ("DB_CONNECT_TIMEOUT", "300"),
        ];
        if let Some(threshold) = threshold {
            env.push(("PNG_PRESET_THRESHOLD_PIXELS", threshold));
        }
        let run = run_cli(&env, &["db", "status"], Duration::from_secs(60));
        assert!(!run.status.success(), "{}", run.stderr);
        let message = "invalid configuration at `settings.image_optimisation.png`: \
                       threshold_pixels is 0";
        assert_eq!(run.stderr.contains(message), refused, "{}", run.stderr);
        assert_eq!(probe.was_connected(), !refused, "{}", run.stderr);
    }
}
