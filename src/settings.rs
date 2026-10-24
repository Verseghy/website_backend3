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
//! the Kubernetes `CronJob`s never serve it (ADR 0004 §4).

use std::path::PathBuf;

use serde::Deserialize;

/// The root key of the section in `config/<env>.yaml`.
pub const SETTINGS_KEY: &str = "settings";

/// All typed settings of backend3.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Where the built admin UI lives (ADR 0004 §4).
    pub admin_ui: AdminUiSettings,
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

/// Why the `settings:` section could not be turned into [`Settings`].
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The configuration has no `settings:` section at all.
    #[error(
        "the configuration has no `settings:` section; backend3 needs at least \
         {ADMIN_UI_DIST_DIR_KEY} (see config/test.yaml for an example)"
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
    /// is missing, has an unknown or missing key, a value of the wrong type,
    /// or an empty `dist_dir`.
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
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn parses_a_complete_section() {
        let section = json!({ "admin_ui": { "dist_dir": "/app/admin/dist" } });
        assert_eq!(
            Settings::from_section(Some(&section)).expect("valid"),
            Settings {
                admin_ui: AdminUiSettings {
                    dist_dir: PathBuf::from("/app/admin/dist"),
                },
            }
        );
    }

    #[test]
    fn a_missing_section_is_an_error() {
        let error = Settings::from_section(None).expect_err("missing");
        assert!(matches!(error, SettingsError::Missing));
        assert!(error.to_string().contains(ADMIN_UI_DIST_DIR_KEY));
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
        let section = json!({ "admin_ui": { "dist_dir": "" } });
        assert!(matches!(
            Settings::from_section(Some(&section)),
            Err(SettingsError::EmptyDistDir)
        ));
    }

    #[test]
    fn structure_only_a_missing_directory_is_fine_here() {
        let section = json!({ "admin_ui": { "dist_dir": "/does/not/exist" } });
        assert!(Settings::from_section(Some(&section)).is_ok());
    }

    proptest! {
        #[test]
        fn any_non_empty_dist_dir_round_trips(dir in "[^\u{0}]{1,64}") {
            let section = json!({ "admin_ui": { "dist_dir": dir } });
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
