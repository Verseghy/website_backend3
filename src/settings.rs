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
//! Validation here is structural, plus the parts of a value that need no
//! external system. Whether the configured admin UI directory exists is
//! checked where the router is built, because tasks and the Kubernetes
//! `CronJob`s never serve it (ADR 0004 §4). Likewise, whether the `/metrics`
//! port is free is only known when `App::serve` binds it. The TOTP
//! encryption key is decoded and checked here, so every command refuses a bad
//! key at boot instead of the first sign-in failing (ADR 0006 §6); the
//! settings keep only the derived cipher key, never the configured value.
//! Production also refuses the public throwaway key of the local
//! environments ([`Settings::refuse_published_keys`], ADR 0034 §3).
//!
//! The PNG encoder effort (`settings.image_optimisation.png`, ADR 0033 §5)
//! is checked here in full, through [`PngEffort::new`], so a value that would
//! break the rule stops the boot with a message that names the key and says
//! what to change, before any upload is optimised with it.

use std::{net::IpAddr, path::PathBuf};

use loco_rs::environment::Environment;
use serde::{Deserialize, Deserializer, de::Error as _};

use crate::domain::{image_optimise::PngEffort, totp_secret::TotpEncryptionKey};

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
    /// Admin sign-in (ADR 0006 §6, §8).
    pub admin_auth: AdminAuthSettings,
    /// How uploaded images are optimised (ADR 0015 §7, ADR 0033).
    pub image_optimisation: ImageOptimisationSettings,
}

/// Settings of admin sign-in (ADR 0006).
///
/// `Debug` never shows the key: [`TotpEncryptionKey`] prints as redacted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminAuthSettings {
    /// The cipher key of the stored TOTP secrets, derived at boot from the
    /// configured value: base64 of at least 64 random bytes (ADR 0006 §6).
    /// Production reads it from `TOTP_ENCRYPTION_KEY`, which has no default,
    /// and refuses the local environments' value; those use a public
    /// throwaway key that is not a secret.
    #[serde(deserialize_with = "totp_encryption_key")]
    pub totp_encryption_key: TotpEncryptionKey,
}

/// The full key of [`AdminAuthSettings::totp_encryption_key`], for error
/// messages.
pub const TOTP_ENCRYPTION_KEY_KEY: &str = "settings.admin_auth.totp_encryption_key";

/// The environment variable that `config/production.yaml` reads
/// [`AdminAuthSettings::totp_encryption_key`] from, named in error messages
/// so the operator knows which secret to fix.
pub const TOTP_ENCRYPTION_KEY_ENV: &str = "TOTP_ENCRYPTION_KEY";

/// Deserializes [`AdminAuthSettings::totp_encryption_key`] from its base64
/// string and derives the cipher key.
///
/// - `deserializer`: the deserializer positioned at the value.
///
/// The error message says what is wrong and which environment variable
/// production reads, but never contains the value. `serde_path_to_error`
/// adds the key path, so [`Settings::from_section`] reports it as
/// `settings.admin_auth.totp_encryption_key`.
///
/// The value is read as any JSON value first, not as a `String`: serde's
/// own type error would quote a value that YAML typed as a number or a
/// boolean, and that value is the secret.
fn totp_encryption_key<'de, D>(deserializer: D) -> Result<TotpEncryptionKey, D::Error>
where
    D: Deserializer<'de>,
{
    let refuse = |problem: &dyn std::fmt::Display| {
        D::Error::custom(format!(
            "the TOTP encryption key ({TOTP_ENCRYPTION_KEY_ENV} in production) {problem}"
        ))
    };
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(configured) => {
            TotpEncryptionKey::from_base64(&configured).map_err(|error| refuse(&error))
        }
        serde_json::Value::Null => Err(refuse(&"is empty")),
        _ => Err(refuse(
            &"is not a string; write it in quotes, as config/production.yaml does",
        )),
    }
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

/// How uploaded images are optimised (ADR 0015 §7, ADR 0033). The upload
/// code passes these values to the optimisers; nothing in
/// `src/domain/image_optimise/` reads them itself (ADR 0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageOptimisationSettings {
    /// The oxipng presets by image size (ADR 0033 §5): `threshold_pixels`,
    /// up to which preset 2 is used, and `large_preset`, the cheaper preset
    /// above it, checked by [`PngEffort::new`]. The production value comes
    /// from a measurement on the production node.
    #[serde(deserialize_with = "png_effort")]
    pub png: PngEffort,
}

/// The full key of [`ImageOptimisationSettings::png`], for error messages.
pub const IMAGE_OPTIMISATION_PNG_KEY: &str = "settings.image_optimisation.png";

/// The keys of `settings.image_optimisation.png`, as written in the YAML.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PngEffortFields {
    /// [`PngEffort::threshold_pixels`].
    threshold_pixels: u64,
    /// [`PngEffort::large_preset`].
    large_preset: u8,
}

/// Reads `settings.image_optimisation.png` and checks it with
/// [`PngEffort::new`]. The error goes back through serde, so
/// [`Settings::from_section`] reports it at the key's full path, with
/// [`PngEffortError`](crate::domain::image_optimise::PngEffortError)'s
/// message on what to change.
///
/// - `deserializer`: the deserializer at that key.
fn png_effort<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PngEffort, D::Error> {
    let fields = PngEffortFields::deserialize(deserializer)?;
    PngEffort::new(fields.threshold_pixels, fields.large_preset).map_err(D::Error::custom)
}

/// Why the `settings:` section could not be turned into [`Settings`].
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The configuration has no `settings:` section at all.
    #[error(
        "the configuration has no `settings:` section; backend3 needs at least \
         {ADMIN_UI_DIST_DIR_KEY}, {METRICS_BINDING_KEY}, {METRICS_PORT_KEY}, \
         {TOTP_ENCRYPTION_KEY_KEY} and {IMAGE_OPTIMISATION_PNG_KEY} (see config/test.yaml for \
         an example)"
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
    /// Production was given the public throwaway TOTP key of the local
    /// environments, which protects nothing (ADR 0034 §3). The message says
    /// where the real key comes from, as ADR 0034 §3 asks; the name of the
    /// `backend-secrets` key is the one ADR 0023 §11 gives, not a value.
    #[error(
        "refusing to start: {TOTP_ENCRYPTION_KEY_KEY} ({TOTP_ENCRYPTION_KEY_ENV}) is the public \
         throwaway key of config/development.yaml, config/test.yaml and config/e2e.yaml, which \
         production refuses; set {TOTP_ENCRYPTION_KEY_ENV} to the key from the secret store \
         (the backend-secrets key backend3-totp-key): base64 of at least 64 random bytes \
         (ADR 0006 §6, ADR 0034 §3)"
    )]
    PublishedTotpKey,
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
    /// 65535 included), an empty `dist_dir`, metrics port 0, a TOTP
    /// encryption key that is not base64 of at least 64 bytes, or a PNG
    /// effort that [`PngEffort::new`] refuses.
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

    /// Refuses, in production, the TOTP key that this public repository
    /// commits for development, test and e2e (ADR 0034 §2 item 3 allows it
    /// there, §3 refuses it in production). `Hooks::load_config` runs it, so
    /// every command stops before it connects anywhere.
    ///
    /// This is the one refusal of published keys that ADR 0034 §3 asks for.
    /// The `secret-guards` slice adds the published JWT secrets to it rather
    /// than adding a second check (ADR 0034 §3, "Who"), and a later slice
    /// that publishes a throwaway key adds that key here in the same PR.
    ///
    /// - `environment`: the Loco environment the configuration was loaded
    ///   for. Only [`Environment::Production`] refuses the key; staging runs
    ///   that environment too. The local environments keep it, because a
    ///   fixed value keeps their runs reproducible and protects nothing.
    ///
    /// The configuration cannot bring the key into production by itself:
    /// `config/production.yaml` reads `TOTP_ENCRYPTION_KEY` with no default.
    /// This check stops the one remaining way, the public value copied into
    /// that variable.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::PublishedTotpKey`] in production when the
    /// configured key is the published one. The message names the key and
    /// the variable, not the value.
    pub fn refuse_published_keys(&self, environment: &Environment) -> Result<(), SettingsError> {
        if *environment == Environment::Production
            && self.admin_auth.totp_encryption_key.is_published_local_key()
        {
            return Err(SettingsError::PublishedTotpKey);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::totp_secret::{PUBLISHED_LOCAL_KEY, PUBLISHED_LOCAL_KEY_TEXT};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use proptest::prelude::*;
    use serde_json::{Value, json};
    use std::net::Ipv4Addr;

    /// A valid metrics sub-section.
    fn metrics() -> Value {
        json!({ "binding": "127.0.0.1", "port": 9464 })
    }

    /// A valid `admin_auth` value, with the local environments' key.
    fn admin_auth() -> Value {
        json!({ "totp_encryption_key": PUBLISHED_LOCAL_KEY })
    }

    /// The `image_optimisation` value of the committed configuration files.
    fn image_optimisation() -> Value {
        json!({ "png": png() })
    }

    /// The PNG effort of the committed configuration files.
    fn png() -> Value {
        json!({ "threshold_pixels": 6_000_000, "large_preset": 0 })
    }

    /// A complete section with the given admin UI directory and metrics
    /// sub-section, the local environments' TOTP key and the committed
    /// image optimisation values.
    fn section(dist_dir: &str, metrics: &Value) -> Value {
        with_png(dist_dir, metrics, &png())
    }

    /// A complete section with the given admin UI directory, metrics
    /// sub-section and PNG effort.
    fn with_png(dist_dir: &str, metrics: &Value, png: &Value) -> Value {
        json!({
            "admin_ui": { "dist_dir": dist_dir },
            "metrics": metrics,
            "admin_auth": admin_auth(),
            "image_optimisation": { "png": png },
        })
    }

    /// A complete section whose `admin_ui` value is `admin_ui`.
    fn with_admin_ui(admin_ui: &Value) -> Value {
        json!({
            "admin_ui": admin_ui,
            "metrics": metrics(),
            "admin_auth": admin_auth(),
            "image_optimisation": image_optimisation(),
        })
    }

    /// A complete section with the TOTP key replaced by `key`.
    fn with_totp_key(key: &Value) -> Value {
        json!({
            "admin_ui": { "dist_dir": "x" },
            "metrics": metrics(),
            "admin_auth": { "totp_encryption_key": key },
            "image_optimisation": image_optimisation(),
        })
    }

    /// The `Invalid` error of `section`, as (path, message).
    fn invalid(section: &Value) -> (String, String) {
        match Settings::from_section(Some(section)) {
            Err(SettingsError::Invalid { path, message }) => (path, message),
            other => panic!("{section}: expected Invalid, got {other:?}"),
        }
    }

    /// The settings of a complete section with the TOTP key `key`.
    fn settings_with_totp_key(key: &str) -> Settings {
        Settings::from_section(Some(&with_totp_key(&json!(key)))).expect("valid")
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
                admin_auth: AdminAuthSettings {
                    totp_encryption_key: TotpEncryptionKey::from_base64(PUBLISHED_LOCAL_KEY)
                        .expect("valid"),
                },
                image_optimisation: ImageOptimisationSettings {
                    png: PngEffort::DEFAULT,
                },
            }
        );
    }

    #[test]
    fn a_missing_section_is_an_error() {
        let error = Settings::from_section(None).expect_err("missing");
        assert!(matches!(error, SettingsError::Missing));
        let message = error.to_string();
        for key in [
            ADMIN_UI_DIST_DIR_KEY,
            METRICS_BINDING_KEY,
            METRICS_PORT_KEY,
            TOTP_ENCRYPTION_KEY_KEY,
            IMAGE_OPTIMISATION_PNG_KEY,
        ] {
            assert!(message.contains(key), "{message}");
        }
    }

    #[test]
    fn errors_name_the_full_key_path() {
        let cases = [
            (json!({}), "settings"),
            (with_admin_ui(&json!({})), "settings.admin_ui"),
            (
                with_admin_ui(&json!({ "dist_dir": 5 })),
                "settings.admin_ui.dist_dir",
            ),
            (
                with_admin_ui(&json!({ "dist_dir": "x", "extra": 1 })),
                "settings.admin_ui.extra",
            ),
            (
                json!({
                    "admin_ui": { "dist_dir": "x" },
                    "metrics": metrics(),
                    "admin_auth": admin_auth(),
                    "image_optimisation": image_optimisation(),
                    "admin_uii": {},
                }),
                "settings.admin_uii",
            ),
            (json!("text"), "settings"),
            (
                json!({
                    "admin_ui": { "dist_dir": "x" },
                    "admin_auth": admin_auth(),
                    "image_optimisation": image_optimisation(),
                }),
                "settings",
            ),
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
            // No admin_auth.
            (
                json!({
                    "admin_ui": { "dist_dir": "x" },
                    "metrics": metrics(),
                    "image_optimisation": image_optimisation(),
                }),
                "settings",
            ),
            (
                json!({
                    "admin_ui": { "dist_dir": "x" },
                    "metrics": metrics(),
                    "admin_auth": {},
                    "image_optimisation": image_optimisation(),
                }),
                "settings.admin_auth",
            ),
            (
                json!({
                    "admin_ui": { "dist_dir": "x" },
                    "metrics": metrics(),
                    "admin_auth": { "totp_encryption_key": PUBLISHED_LOCAL_KEY, "extra": 1 },
                    "image_optimisation": image_optimisation(),
                }),
                "settings.admin_auth.extra",
            ),
            (with_totp_key(&json!("")), TOTP_ENCRYPTION_KEY_KEY),
        ];
        for (section, expected_path) in cases {
            let (path, message) = invalid(&section);
            assert_eq!(path, expected_path, "{section}: {message}");
        }
    }

    #[test]
    fn image_optimisation_errors_name_the_full_key_path() {
        let cases = [
            // No image_optimisation.
            (
                json!({
                    "admin_ui": { "dist_dir": "x" },
                    "metrics": metrics(),
                    "admin_auth": admin_auth(),
                }),
                "settings",
            ),
            (
                with_png("x", &metrics(), &json!({ "threshold_pixels": 1 })),
                "settings.image_optimisation.png",
            ),
            (
                with_png(
                    "x",
                    &metrics(),
                    &json!({ "threshold_pixels": 1, "large_preset": 0, "level": 3 }),
                ),
                "settings.image_optimisation.png.level",
            ),
            (
                with_png(
                    "x",
                    &metrics(),
                    &json!({ "threshold_pixels": -1, "large_preset": 0 }),
                ),
                "settings.image_optimisation.png.threshold_pixels",
            ),
            (
                with_png(
                    "x",
                    &metrics(),
                    &json!({ "threshold_pixels": 1, "large_preset": 300 }),
                ),
                "settings.image_optimisation.png.large_preset",
            ),
            // Preset 2 below the threshold is fixed (ADR 0033 §5), so the
            // key that set it in earlier drafts is unknown.
            (
                with_png(
                    "x",
                    &metrics(),
                    &json!({ "threshold_pixels": 1, "preset": 3, "large_preset": 0 }),
                ),
                "settings.image_optimisation.png.preset",
            ),
        ];
        for (section, expected_path) in cases {
            let (path, message) = invalid(&section);
            assert_eq!(path, expected_path, "{section}: {message}");
        }
    }

    #[test]
    fn bad_totp_keys_name_the_key_and_the_problem_but_not_the_value() {
        let short = STANDARD.encode([0x5a_u8; 40]);
        let cases = [
            (json!(""), "is empty"),
            (Value::Null, "is empty"),
            (
                json!("not base64, but a pasted password"),
                "is not valid base64",
            ),
            (
                json!(format!("{PUBLISHED_LOCAL_KEY}\n")),
                "is not valid base64",
            ),
            (json!(short), "decodes to 40 bytes"),
            (json!(1_234_567_890_123_u64), "is not a string"),
            (json!(true), "is not a string"),
            (json!(["a"]), "is not a string"),
        ];
        for (key, problem) in cases {
            let section = with_totp_key(&key);
            let (path, message) = invalid(&section);
            assert_eq!(path, TOTP_ENCRYPTION_KEY_KEY, "{key}: {message}");
            assert!(message.contains(problem), "{key}: {message}");
            assert!(
                message.contains(TOTP_ENCRYPTION_KEY_ENV),
                "{key}: {message}"
            );
            for fragment in ["pasted password", "1234567890123", short.as_str()] {
                assert!(!message.contains(fragment), "{key}: {message}");
            }
            let shown = SettingsError::Invalid { path, message }.to_string();
            assert!(
                shown.starts_with(&format!(
                    "invalid configuration at `{TOTP_ENCRYPTION_KEY_KEY}`"
                )),
                "{shown}"
            );
        }
    }

    #[test]
    fn debug_never_shows_the_totp_key() {
        let section = with_admin_ui(&json!({ "dist_dir": "x" }));
        let printed = format!("{:?}", Settings::from_section(Some(&section)));
        assert!(
            printed.contains("TotpEncryptionKey(<redacted>)")
                && !printed.contains(PUBLISHED_LOCAL_KEY)
        );
    }

    #[test]
    fn production_refuses_the_published_totp_key() {
        let settings = settings_with_totp_key(PUBLISHED_LOCAL_KEY);
        let error = settings
            .refuse_published_keys(&Environment::Production)
            .expect_err("the published key in production");
        assert!(matches!(error, SettingsError::PublishedTotpKey));
        let message = error.to_string();
        assert!(
            message.contains(TOTP_ENCRYPTION_KEY_KEY) && message.contains(TOTP_ENCRYPTION_KEY_ENV),
            "{message}"
        );
        assert!(message.contains("public throwaway key"), "{message}");
        // ADR 0034 §3: the message says to set a key from the secret store.
        assert!(
            message.contains("from the secret store (the backend-secrets key backend3-totp-key)"),
            "{message}"
        );
        assert!(!message.contains(PUBLISHED_LOCAL_KEY), "{message}");
    }

    #[test]
    fn the_local_environments_keep_the_published_totp_key() {
        let settings = settings_with_totp_key(PUBLISHED_LOCAL_KEY);
        for environment in [
            Environment::Development,
            Environment::Test,
            Environment::Any("e2e".to_owned()),
        ] {
            assert!(
                settings.refuse_published_keys(&environment).is_ok(),
                "{environment}"
            );
        }
    }

    #[test]
    fn production_accepts_a_key_of_its_own() {
        // The published text with one byte changed is another key: the
        // refusal compares the whole derived key, not a prefix. These are
        // fixed inputs of the comparison alone; a test that starts the
        // production configuration uses a key made when it runs
        // (tests/support `cli_totp_key`, ADR 0034 §3).
        let mut near = PUBLISHED_LOCAL_KEY_TEXT.to_vec();
        if let Some(last) = near.last_mut() {
            *last ^= 1;
        }
        for key in [
            STANDARD.encode([0x42_u8; 64]),
            STANDARD.encode(&near),
            STANDARD.encode(&PUBLISHED_LOCAL_KEY_TEXT[..64]),
        ] {
            let settings = settings_with_totp_key(&key);
            assert!(
                settings
                    .refuse_published_keys(&Environment::Production)
                    .is_ok(),
                "{key}"
            );
        }
    }

    #[test]
    fn unknown_keys_are_reported_by_name() {
        let section = json!({
            "admin_ui": { "dist_dir": "x" },
            "metrics": metrics(),
            "admin_auth": admin_auth(),
            "image_optimisation": image_optimisation(),
            "admin_uii": {},
        });
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
    fn the_png_effort_is_checked_at_its_key() {
        // Each rule of PngEffort::new, reported at the key with its message.
        for (png, message) in [
            (
                json!({ "threshold_pixels": 0, "large_preset": 0 }),
                "threshold_pixels is 0",
            ),
            (
                json!({ "threshold_pixels": 1, "large_preset": 2 }),
                "large_preset is 2",
            ),
        ] {
            let (path, found) = invalid(&with_png("x", &metrics(), &png));
            assert_eq!(path, IMAGE_OPTIMISATION_PNG_KEY, "{png}");
            assert!(found.starts_with(message), "{png}: {found}");
        }
        // A valid value other than the default is taken as it is.
        let png = json!({ "threshold_pixels": 8_000_000, "large_preset": 1 });
        let settings = Settings::from_section(Some(&with_png("x", &metrics(), &png))).unwrap();
        assert_eq!(
            settings.image_optimisation.png,
            PngEffort::new(8_000_000, 1).unwrap()
        );
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
        fn any_key_of_64_bytes_or_more_is_accepted_and_derived(
            bytes in proptest::collection::vec(any::<u8>(), 64..=160),
        ) {
            let section = with_totp_key(&json!(STANDARD.encode(&bytes)));
            let settings = Settings::from_section(Some(&section)).expect("valid");
            prop_assert_eq!(
                &settings.admin_auth.totp_encryption_key,
                &TotpEncryptionKey::derive(&bytes).expect("valid")
            );
            // Any key but the published one starts in production too.
            prop_assume!(bytes != PUBLISHED_LOCAL_KEY_TEXT);
            prop_assert!(settings.refuse_published_keys(&Environment::Production).is_ok());
        }

        #[test]
        fn the_png_effort_is_taken_exactly_when_png_effort_new_takes_it(
            threshold in any::<u64>(),
            large_preset in any::<u8>(),
        ) {
            let png = json!({ "threshold_pixels": threshold, "large_preset": large_preset });
            let parsed = Settings::from_section(Some(&with_png("x", &metrics(), &png)))
                .map(|settings| settings.image_optimisation.png);
            match PngEffort::new(threshold, large_preset) {
                Ok(effort) => prop_assert_eq!(parsed.ok(), Some(effort)),
                Err(error) => {
                    let message = parsed.expect_err("refused").to_string();
                    prop_assert!(message.contains(IMAGE_OPTIMISATION_PNG_KEY), "{}", message);
                    prop_assert!(message.contains(&error.to_string()), "{}", message);
                }
            }
        }

        #[test]
        fn arbitrary_json_never_panics(key in "[a-z_]{0,10}", value in any::<i64>()) {
            let section = json!({ key: value });
            let _ = Settings::from_section(Some(&section));
        }
    }
}
