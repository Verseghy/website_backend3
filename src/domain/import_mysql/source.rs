//! The import's source setting, `settings.import.source_database_url`
//! (docs/design/data-migration.md §3.3, §4.1).
//!
//! The connection URL of the legacy MySQL source reaches backend3 only
//! through the environment variable `IMPORT_SOURCE_DATABASE_URL`, which
//! `config/production.yaml` reads into the typed settings; it is never a
//! command-line argument, so it never lands in shell history or a process
//! list. Only the `import_mysql` task reads the setting, and it passes the
//! connection it builds down to the import (ADR 0008 rules 2 and 3).
//!
//! - **Two spellings of "unset".** The setting's default is the empty
//!   string, and outside the cut-over window the secret behind the variable
//!   (the `backend-secrets` key `import-source-database-url`) holds the
//!   placeholder [`DISABLED`]: the key always exists in that shared secret,
//!   so that the secret's sync never fails for want of it, and only the
//!   cut-over window changes its value. Both mean unset, and the task
//!   refuses to start without a source, naming the setting.
//! - **The value is never shown.** The URL holds the source account's
//!   password. [`SourceDatabaseUrl`]'s `Debug` prints only whether it is set,
//!   and no message here contains any part of the value: a malformed URL is
//!   described by the URL parser's fixed reason, never by quoting it.

use std::fmt;

use url::Url;

/// The full key of the setting, for messages.
pub const SOURCE_DATABASE_URL_KEY: &str = "settings.import.source_database_url";

/// The environment variable that `config/production.yaml` and
/// `config/development.yaml` read the setting from.
pub const SOURCE_DATABASE_URL_ENV: &str = "IMPORT_SOURCE_DATABASE_URL";

/// The placeholder that the secret holds outside the cut-over window. Like
/// the empty string, it means unset.
pub const DISABLED: &str = "disabled";

/// The URL scheme the source must have.
pub const MYSQL_SCHEME: &str = "mysql";

/// The configured value of `settings.import.source_database_url`.
///
/// `Debug` shows `SourceDatabaseUrl(<unset>)` or
/// `SourceDatabaseUrl(<redacted>)`, never the value, so the typed settings
/// can be printed whole.
#[derive(Clone, PartialEq, Eq)]
pub struct SourceDatabaseUrl(String);

impl SourceDatabaseUrl {
    /// Wraps the configured value.
    ///
    /// - `configured`: the setting as the configuration file rendered it:
    ///   the empty string, [`DISABLED`] or a URL.
    #[must_use]
    pub fn new(configured: String) -> Self {
        Self(configured)
    }

    /// Whether the setting names a source: false for the empty string and
    /// for [`DISABLED`], true for anything else, even a malformed URL, which
    /// [`SourceDatabaseUrl::require`] refuses.
    #[must_use]
    pub fn is_set(&self) -> bool {
        !(self.0.is_empty() || self.0 == DISABLED)
    }

    /// The source URL, for the task that connects to it.
    ///
    /// The checks need no network: the value is set, it parses as a URL and
    /// its scheme is `mysql`. Whether the MySQL driver accepts the rest (the
    /// percent-encoding of the user name and password, the query
    /// parameters) the task learns when it builds the connection options.
    ///
    /// # Errors
    ///
    /// A [`SourceError`] that names the setting and the variable and never
    /// contains the value: [`SourceError::Unset`] for both spellings of
    /// unset, [`SourceError::NotAUrl`] and [`SourceError::NotMysql`] for a
    /// value that is not a `mysql://` URL.
    pub fn require(&self) -> Result<&str, SourceError> {
        if !self.is_set() {
            return Err(SourceError::Unset);
        }
        let url = Url::parse(&self.0).map_err(SourceError::NotAUrl)?;
        if url.scheme() != MYSQL_SCHEME {
            return Err(SourceError::NotMysql);
        }
        Ok(&self.0)
    }
}

impl fmt::Debug for SourceDatabaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = if self.is_set() {
            "<redacted>"
        } else {
            "<unset>"
        };
        write!(formatter, "SourceDatabaseUrl({shown})")
    }
}

/// Why the source setting cannot be used. No message contains any part of
/// the configured value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The setting is the empty string or [`DISABLED`].
    #[error(
        "refusing to start: {SOURCE_DATABASE_URL_KEY} is unset (empty, or the placeholder \
         `{DISABLED}` that the secret holds outside the cut-over window); set \
         {SOURCE_DATABASE_URL_ENV} to the mysql:// URL of the legacy source database \
         (data-migration.md §3.3, §4.1)"
    )]
    Unset,
    /// The value does not parse as a URL.
    #[error(
        "refusing to start: {SOURCE_DATABASE_URL_KEY} ({SOURCE_DATABASE_URL_ENV}) is not a URL \
         ({0}); write it as mysql://<user>:<password>@<host>:<port>/<database>, with the user \
         name and password percent-encoded; the value is not shown, because it holds the \
         password"
    )]
    NotAUrl(url::ParseError),
    /// The value is a URL with a scheme other than `mysql`.
    #[error(
        "refusing to start: {SOURCE_DATABASE_URL_KEY} ({SOURCE_DATABASE_URL_ENV}) is not a \
         mysql:// URL; the import reads the legacy MySQL database (data-migration.md §3.3); the \
         value is not shown, because it holds the password"
    )]
    NotMysql,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The password of the URLs here: a value that no message may contain.
    const PASSWORD: &str = "pw-never-in-output-8f3c";

    fn url(text: &str) -> SourceDatabaseUrl {
        SourceDatabaseUrl::new(text.to_owned())
    }

    #[test]
    fn the_empty_string_and_disabled_both_mean_unset() {
        for unset in ["", "disabled"] {
            let setting = url(unset);
            assert!(!setting.is_set(), "{unset:?}");
            let error = setting.require().expect_err(unset);
            assert_eq!(error, SourceError::Unset);
            let message = error.to_string();
            assert!(message.contains(SOURCE_DATABASE_URL_KEY), "{message}");
            assert!(message.contains(SOURCE_DATABASE_URL_ENV), "{message}");
            assert!(message.contains("`disabled`"), "{message}");
            assert_eq!(format!("{setting:?}"), "SourceDatabaseUrl(<unset>)");
        }
    }

    #[test]
    fn other_spellings_of_disabled_are_values_not_placeholders() {
        // Only the exact placeholder is unset; these are malformed URLs and
        // get the URL's message, so a typo in the secret is not mistaken for
        // a deliberate "off".
        for value in ["Disabled", " disabled", "disabled\n", "off", "none"] {
            let setting = url(value);
            assert!(setting.is_set(), "{value:?}");
            assert!(
                matches!(setting.require(), Err(SourceError::NotAUrl(_))),
                "{value:?}"
            );
        }
    }

    #[test]
    fn a_mysql_url_is_returned_as_configured() {
        for value in [
            format!("mysql://import:{PASSWORD}@mysql:3306/verseghy"),
            "mysql://root@127.0.0.1/verseghy".to_owned(),
            "mysql://localhost/verseghy?socket=%2Frun%2Fmysqld%2Fmysqld.sock".to_owned(),
        ] {
            let setting = SourceDatabaseUrl::new(value.clone());
            assert_eq!(setting.require(), Ok(value.as_str()));
            assert_eq!(format!("{setting:?}"), "SourceDatabaseUrl(<redacted>)");
        }
    }

    #[test]
    fn a_value_that_is_not_a_mysql_url_is_refused_without_showing_it() {
        for (value, expected) in [
            (
                format!("import:{PASSWORD}@mysql/verseghy"),
                "is not a mysql:// URL",
            ),
            (
                format!("//import:{PASSWORD}@mysql/verseghy"),
                "is not a URL",
            ),
            (
                format!("mysql://import:{PASSWORD}@[mysql/verseghy"),
                "is not a URL",
            ),
            (
                format!("postgres://import:{PASSWORD}@mysql/verseghy"),
                "is not a mysql:// URL",
            ),
            (
                format!("mariadb://import:{PASSWORD}@mysql/verseghy"),
                "is not a mysql:// URL",
            ),
        ] {
            let setting = SourceDatabaseUrl::new(value.clone());
            let error = setting.require().expect_err(&value);
            let message = error.to_string();
            assert!(message.contains(expected), "{message}");
            assert!(message.contains(SOURCE_DATABASE_URL_KEY), "{message}");
            assert!(message.contains(SOURCE_DATABASE_URL_ENV), "{message}");
            for shown in [message, format!("{error:?}"), format!("{setting:?}")] {
                assert!(!shown.contains(PASSWORD), "{shown}");
                assert!(!shown.contains("import:"), "{shown}");
            }
        }
    }

    proptest! {
        #[test]
        fn unset_exactly_for_the_two_spellings(value in ".{0,12}") {
            let unset = value.is_empty() || value == DISABLED;
            prop_assert_eq!(url(&value).is_set(), !unset);
            prop_assert_eq!(url(&value).require() == Err(SourceError::Unset), unset);
        }

        #[test]
        fn no_error_or_debug_output_contains_the_password(
            scheme in prop_oneof![Just("mysql"), Just("postgres"), Just(""), Just("my sql")],
            separator in prop_oneof![Just("://"), Just(":"), Just("//"), Just("")],
            password in "pw[A-Za-z0-9]{14}",
            host in prop_oneof![Just("mysql"), Just("[::1"), Just(""), Just("127.0.0.1:99999")],
            rest in prop_oneof![Just("/verseghy"), Just(""), Just("?ssl-mode=x")],
        ) {
            let setting = url(&format!("{scheme}{separator}import:{password}@{host}{rest}"));
            let debug = format!("{setting:?}");
            prop_assert!(!debug.contains(&password));
            if let Err(error) = setting.require() {
                prop_assert!(!error.to_string().contains(&password), "{}", error);
                prop_assert!(!format!("{error:?}").contains(&password), "{:?}", error);
            }
        }

        #[test]
        fn a_set_value_is_returned_unchanged_exactly_when_it_is_a_mysql_url(
            value in "(mysql|postgres)://[a-z]{1,8}(:[a-z0-9]{1,8})?@[a-z0-9.]{1,12}(:[0-9]{1,4})?(/[a-z]{0,8})?",
        ) {
            let setting = url(&value);
            match setting.require() {
                Ok(returned) => prop_assert_eq!(returned, value.as_str()),
                Err(error) => prop_assert!(
                    !value.starts_with("mysql://") || matches!(error, SourceError::NotAUrl(_)),
                    "{}: {:?}", value, error
                ),
            }
            if value.starts_with("postgres://") {
                prop_assert_eq!(setting.require(), Err(SourceError::NotMysql));
            }
        }
    }
}
