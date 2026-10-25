//! Typed application settings: the `settings:` section of `config/<env>.yaml`.
//!
//! Loco hands the section over as untyped JSON (`Config::settings`). backend3
//! deserializes it once into [`Settings`] at boot, from `Hooks::load_config`,
//! so a typo or a missing key stops every command (server, tasks, `db`) with
//! an error that names the full key path, for example
//! `settings.admin_ui.dist_dir` (ADR 0008, "Typed configuration"). Unknown
//! keys are errors too, so a misspelt key cannot silently fall back to a
//! default.
//!
//! Code that talks to an external system receives only the sub-struct it
//! needs, never the whole `Settings` and never `AppContext` (ADR 0008,
//! rule 2). The admin UI router, for example, takes [`AdminUiSettings`].
//!
//! Validation here is structural only. Whether the configured admin UI
//! directory exists is checked where the router is built, because tasks and
//! the Kubernetes `CronJob`s never serve it (ADR 0004 §4). Likewise, whether
//! the `/metrics` port is free is only known when `App::serve` binds it.

use std::{net::IpAddr, path::PathBuf};

use serde::Deserialize;

/// The root key of the section in `config/<env>.yaml`.
pub const SETTINGS_KEY: &str = "settings";

/// All typed settings of backend3.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Where the built admin UI lives (ADR 0004 §4).
    pub admin_ui: AdminUiSettings,
    /// Where the `/metrics` listener listens (ADR 0023 §9).
    pub metrics: MetricsSettings,
}

/// Settings of the `/admin` static router (ADR 0004 §3-§4).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminUiSettings {
    /// The Vite build output directory with `index.html` and `assets/`.
    /// Production uses `/app/admin/dist`; tests use the fixture directory
    /// `tests/fixtures/admin-dist`. A relative path is resolved against the
    /// working directory of the process.
    pub dist_dir: PathBuf,
}

/// The full key of [`AdminUiSettings::dist_dir`], for error messages.
pub const ADMIN_UI_DIST_DIR_KEY: &str = "settings.admin_ui.dist_dir";

/// The listener that serves `/metrics`, on a port of its own that no Route
/// targets, so the metrics are never public (ADR 0014, ADR 0023 §9).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsSettings {
    /// The IP address to bind: `0.0.0.0` in production, where the pod's
    /// `metrics` port is reached through its Service, and `127.0.0.1`
    /// elsewhere. An address, not a host name: `localhost` resolves to `::1`
    /// and `127.0.0.1`, and a bind takes the first free one, so a second
    /// server would start on a port the first already holds.
    pub binding: IpAddr,
    /// The port: 9464 in every committed configuration, never 0 and never
    /// `server.port` ([`Settings::check_server_port`]).
    pub port: u16,
}

/// The full key of [`MetricsSettings::binding`], for error messages.
pub const METRICS_BINDING_KEY: &str = "settings.metrics.binding";
/// The full key of [`MetricsSettings::port`], for error messages.
pub const METRICS_PORT_KEY: &str = "settings.metrics.port";

/// Why the `settings:` section could not be turned into [`Settings`].
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The configuration has no `settings:` section at all.
    #[error(
        "the configuration has no `settings:` section; backend3 needs at least \
         {ADMIN_UI_DIST_DIR_KEY}, {METRICS_BINDING_KEY} and {METRICS_PORT_KEY} \
         (see config/test.yaml for an example)"
    )]
    Missing,
    /// A key is missing, unknown or has the wrong type.
    #[error("invalid configuration at `{path}`: {message}")]
    Invalid {
        /// The full key path, for example `settings.admin_ui.dist_dir`.
        path: String,
        /// What serde found wrong there.
        message: String,
    },
    /// The admin UI directory setting is empty.
    #[error(
        "{ADMIN_UI_DIST_DIR_KEY} is empty; set it to the directory of the built admin UI \
         (for example /app/admin/dist in production)"
    )]
    EmptyDistDir,
    /// Port 0 would make the system pick a random port that nothing knows to
    /// scrape.
    #[error("{METRICS_PORT_KEY} is 0; set a fixed port (9464 in the committed configuration)")]
    MetricsPortZero,
    /// The metrics port is the HTTP server's port.
    #[error(
        "{METRICS_PORT_KEY} and server.port are both {port}; give /metrics a port of its own \
         (9464 in the committed configuration)"
    )]
    MetricsPortIsServerPort {
        /// The port both keys name.
        port: u16,
    },
}

impl Settings {
    /// Deserializes and validates the `settings:` section.
    ///
    /// - `section`: the value of `Config::settings`, `None` when the section
    ///   is absent.
    ///
    /// # Errors
    ///
    /// Returns a [`SettingsError`] naming the offending key when the section
    /// is missing, has an unknown or missing key, a value of the wrong type
    /// (a metrics binding that is not an IP address and a metrics port above
    /// 65535 included), an empty `dist_dir`, or metrics port 0.
    pub fn from_section(section: Option<&serde_json::Value>) -> Result<Self, SettingsError> {
        let section = section.ok_or(SettingsError::Missing)?;
        let settings: Self =
            serde_path_to_error::deserialize(section).map_err(|error| SettingsError::Invalid {
                path: match error.path().to_string() {
                    path if path == "." => SETTINGS_KEY.to_owned(),
                    path => format!("{SETTINGS_KEY}.{path}"),
                },
                message: error.inner().to_string(),
            })?;
        if settings.admin_ui.dist_dir.as_os_str().is_empty() {
            return Err(SettingsError::EmptyDistDir);
        }
        if settings.metrics.port == 0 {
            return Err(SettingsError::MetricsPortZero);
        }
        Ok(settings)
    }

    /// Checks the one rule that spans sections: `settings.metrics.port` is
    /// not `server.port`. `Hooks::load_config` runs it.
    ///
    /// - `server_port`: `server.port` of the same configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::MetricsPortIsServerPort`] when the ports are
    /// equal. The bind cannot be relied on to catch it: with
    /// `server.binding: localhost` the HTTP server binds `::1` in
    /// development, so a metrics bind on `127.0.0.1` and the same port
    /// would succeed.
    pub fn check_server_port(&self, server_port: i32) -> Result<(), SettingsError> {
        if i32::from(self.metrics.port) == server_port {
            return Err(SettingsError::MetricsPortIsServerPort {
                port: self.metrics.port,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{Value, json};
    use std::net::Ipv4Addr;

    /// A section with the given admin UI directory and metrics sub-section.
    fn section(dist_dir: &str, metrics: &Value) -> Value {
        json!({ "admin_ui": { "dist_dir": dist_dir }, "metrics": metrics })
    }

    /// A valid metrics sub-section.
    fn metrics() -> Value {
        json!({ "binding": "127.0.0.1", "port": 9464 })
    }

    #[test]
    fn parses_a_complete_section() {
        let section = section("/app/admin/dist", &metrics());
        assert_eq!(
            Settings::from_section(Some(&section)).expect("valid"),
            Settings {
                admin_ui: AdminUiSettings {
                    dist_dir: PathBuf::from("/app/admin/dist"),
                },
                metrics: MetricsSettings {
                    binding: Ipv4Addr::LOCALHOST.into(),
                    port: 9464,
                },
            }
        );
    }

    #[test]
    fn a_missing_section_is_an_error() {
        let error = Settings::from_section(None).expect_err("missing");
        assert!(matches!(error, SettingsError::Missing));
        let message = error.to_string();
        for key in [ADMIN_UI_DIST_DIR_KEY, METRICS_BINDING_KEY, METRICS_PORT_KEY] {
            assert!(message.contains(key), "{message}");
        }
    }

    #[test]
    fn errors_name_the_full_key_path() {
        let cases = [
            (json!({}), "settings"),
            (json!({ "admin_ui": {} }), "settings.admin_ui"),
            (
                json!({ "admin_ui": { "dist_dir": 5 } }),
                "settings.admin_ui.dist_dir",
            ),
            (
                json!({ "admin_ui": { "dist_dir": "x", "extra": 1 } }),
                "settings.admin_ui.extra",
            ),
            (
                json!({ "admin_ui": { "dist_dir": "x" }, "admin_uii": {} }),
                "settings.admin_uii",
            ),
            (json!("text"), "settings"),
            (json!({ "admin_ui": { "dist_dir": "x" } }), "settings"),
            (section("x", &json!({ "port": 9464 })), "settings.metrics"),
            // A host name is refused: it can resolve to several addresses.
            (
                section("x", &json!({ "binding": "localhost", "port": 9464 })),
                "settings.metrics.binding",
            ),
            (
                section("x", &json!({ "binding": "127.0.0.1", "port": "9464" })),
                "settings.metrics.port",
            ),
            (
                section("x", &json!({ "binding": "127.0.0.1", "port": 70000 })),
                "settings.metrics.port",
            ),
            (
                section("x", &json!({ "binding": "::1", "port": 1, "path": "/" })),
                "settings.metrics.path",
            ),
        ];
        for (section, expected_path) in cases {
            match Settings::from_section(Some(&section)) {
                Err(SettingsError::Invalid { path, message }) => {
                    assert_eq!(path, expected_path, "{section}: {message}");
                }
                other => panic!("{section}: expected Invalid, got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_keys_are_reported_by_name() {
        let section = json!({ "admin_ui": { "dist_dir": "x" }, "admin_uii": {} });
        let message = Settings::from_section(Some(&section))
            .expect_err("unknown key")
            .to_string();
        assert!(message.contains("admin_uii"), "{message}");
    }

    #[test]
    fn an_empty_dist_dir_is_an_error() {
        assert!(matches!(
            Settings::from_section(Some(&section("", &metrics()))),
            Err(SettingsError::EmptyDistDir)
        ));
    }

    #[test]
    fn structure_only_a_missing_directory_is_fine_here() {
        let section = section("/does/not/exist", &metrics());
        assert!(Settings::from_section(Some(&section)).is_ok());
    }

    #[test]
    fn metrics_port_zero_is_an_error() {
        let section = section("x", &json!({ "binding": "127.0.0.1", "port": 0 }));
        let error = Settings::from_section(Some(&section)).expect_err("port 0");
        assert!(matches!(error, SettingsError::MetricsPortZero));
        assert!(error.to_string().contains(METRICS_PORT_KEY), "{error}");
    }

    #[test]
    fn the_metrics_port_must_differ_from_the_server_port() {
        let settings = Settings::from_section(Some(&section("x", &metrics()))).expect("valid");
        assert!(settings.check_server_port(5150).is_ok());
        let error = settings.check_server_port(9464).expect_err("same port");
        assert!(matches!(
            error,
            SettingsError::MetricsPortIsServerPort { port: 9464 }
        ));
        let message = error.to_string();
        assert!(message.contains(METRICS_PORT_KEY) && message.contains("server.port"));
    }

    proptest! {
        #[test]
        fn any_ip_address_and_non_zero_port_round_trip(
            binding in any::<IpAddr>(),
            port in 1u16..,
        ) {
            let metrics = json!({ "binding": binding.to_string(), "port": port });
            let settings = Settings::from_section(Some(&section("x", &metrics))).expect("valid");
            prop_assert_eq!(&settings.metrics, &MetricsSettings { binding, port });
            prop_assert!(settings.check_server_port(i32::from(port)).is_err());
            prop_assert!(settings.check_server_port(i32::from(port) + 1).is_ok());
        }

        #[test]
        fn any_non_empty_dist_dir_round_trips(dir in "[^\u{0}]{1,64}") {
            let section = section(&dir, &metrics());
            let settings = Settings::from_section(Some(&section)).expect("valid");
            prop_assert_eq!(settings.admin_ui.dist_dir, PathBuf::from(&dir));
        }

        #[test]
        fn arbitrary_json_never_panics(key in "[a-z_]{0,10}", value in any::<i64>()) {
            let section = json!({ key: value });
            let _ = Settings::from_section(Some(&section));
        }
    }
}
