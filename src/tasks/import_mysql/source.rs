//! The source connection of the import: the legacy MySQL database named by
//! `settings.import.source_database_url` (data-migration.md §3.3).
//!
//! The task builds the connection options here and hands the pool to the
//! import; nothing below the task reads the setting (ADR 0008 rule 2). The
//! URL holds the source password, so no message here contains any part of
//! it, and neither do the driver's: sqlx's `MySqlConnectOptions` prints its
//! password in `Debug`, so the options and the pool are never logged or
//! formatted.

use std::str::FromStr;

use loco_rs::{Error, Result};
use sqlx::{
    MySqlPool,
    mysql::{MySqlConnectOptions, MySqlPoolOptions},
};

use crate::domain::import_mysql::source::{
    SOURCE_DATABASE_URL_ENV, SOURCE_DATABASE_URL_KEY, SourceDatabaseUrl,
};

/// The connection options of the source.
///
/// - `setting`: `settings.import.source_database_url`.
///
/// # Errors
///
/// The setting's own refusal (`SourceDatabaseUrl::require`: unset in either
/// spelling, not a URL, not `mysql://`), or a message that the MySQL driver
/// refuses the URL's other parts. None shows the value.
pub fn connect_options(setting: &SourceDatabaseUrl) -> Result<MySqlConnectOptions> {
    let url = setting.require().map_err(Error::msg)?;
    // sqlx's reason is not passed on: its parser may name the value of a
    // query parameter, and nothing proves that no future reason names a
    // part of the user name or password.
    MySqlConnectOptions::from_str(url).map_err(|_| {
        Error::Message(format!(
            "refusing to start: {SOURCE_DATABASE_URL_KEY} ({SOURCE_DATABASE_URL_ENV}) is a URL \
             that the MySQL driver refuses: check that the user name, the password and the \
             database are percent-encoded UTF-8 and that the query parameters (ssl-mode, \
             statement-cache-capacity and the others sqlx reads) have valid values; the value is \
             not shown, because it holds the password"
        ))
    })
}

/// A pool of one connection to the source, opened on first use.
///
/// - `options`: from [`connect_options`].
///
/// One connection, because the import reads the whole source in one locked
/// session (`LOCK TABLES ... READ`, data-migration.md §3.3), which only
/// holds on one connection. Opened on first use, so a run that stops before
/// it reads the source never connects to it.
#[must_use]
pub fn pool(options: MySqlConnectOptions) -> MySqlPool {
    MySqlPoolOptions::new()
        .min_connections(0)
        .max_connections(1)
        .connect_lazy_with(options)
}
