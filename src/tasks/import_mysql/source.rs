//! The source connection of the import: the legacy MySQL database named by
//! `settings.import.source_database_url` (data-migration.md §3.3), and the
//! one way the import reads a legacy table from it.
//!
//! The task builds the connection options here and hands the pool to the
//! import; nothing below the task reads the setting (ADR 0008 rule 2).
//! [`connect_options`] refuses an unusable setting and, in the `test`,
//! `development` and `e2e` environments, a source that is not local
//! (`domain::import_mysql::source_guard`, ADR 0024 §4 layer 3), all before
//! any connection exists.
//!
//! It returns them as a [`SourceOptions`], which nothing else can build, and
//! [`pool`] takes only that type, so no pool reaches a source that the guard
//! has not accepted.
//!
//! [`select_rows`] is the source query layer: every read of a legacy table
//! goes through it, and it reads only the tables of the source allow-list
//! (`domain::import_mysql::source_tables`). A read of any other table, the
//! newsletter's subscriber list first of all (NWS-01), is refused before
//! anything is sent to the source. This file and `session.rs`, the locked
//! source session (its settings, `LOCK TABLES`, the schema fingerprint and
//! the `AUTO_INCREMENT` and timestamp reads), are the only files of the
//! import task that build or send a statement to the source or open a
//! connection to it: `tests/architecture.rs` fails when another file of
//! `src/tasks/import_mysql/` names sqlx's statement or connection API, and
//! when a file outside the import task names sqlx at all. A statement of
//! another kind joins that test's list of statement files in its own PR,
//! and names its tables from `SOURCE_TABLES` too.
//!
//! The URL holds the source password, so no message here contains any part
//! of it, and neither do the driver's: sqlx's `MySqlConnectOptions` prints
//! its password in `Debug`, so the options and the pool are never logged or
//! formatted, and [`source_error`] passes on only the parts of a driver
//! error that cannot hold the URL's user name, host or password.

use std::{fmt, io, str::FromStr};

use loco_rs::{Error, Result};
use sqlx::{
    MySqlExecutor, MySqlPool,
    mysql::{MySqlConnectOptions, MySqlDatabaseError, MySqlPoolOptions, MySqlRow},
};

use crate::domain::import_mysql::{
    source::{SOURCE_DATABASE_URL_ENV, SOURCE_DATABASE_URL_KEY, SourceDatabaseUrl},
    source_guard,
    source_tables::SourceSelect,
};

/// The connection options of the source, after every check of
/// [`connect_options`].
///
/// The field is private and only [`connect_options`] builds a value, so a
/// `SourceOptions` always names a source that the setting's check, the
/// driver's parse and the source guard accepted, and [`pool`], which takes
/// nothing else, cannot be handed an unchecked one. `Debug` shows no field:
/// sqlx's own `Debug` of the options prints the password.
pub struct SourceOptions(MySqlConnectOptions);

impl SourceOptions {
    /// The checked options, for a pool with other settings than [`pool`]'s,
    /// such as a test's short acquire timeout. The source guard has already
    /// accepted them; the import task itself always uses [`pool`].
    #[must_use]
    pub fn into_inner(self) -> MySqlConnectOptions {
        self.0
    }
}

impl fmt::Debug for SourceOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceOptions")
            .finish_non_exhaustive()
    }
}

/// The connection options of the source.
///
/// - `setting`: `settings.import.source_database_url`.
/// - `environment`: the Loco environment the task runs in, for the source
///   guard.
///
/// The checks run in this order, and none of them connects: the setting's
/// own refusal (`SourceDatabaseUrl::require`: unset in either spelling, not
/// a URL, not `mysql://`), the MySQL driver's parse of the rest of the URL,
/// then the source guard on the host and socket of the driver's parsed
/// options, so the guard judges exactly where the driver would connect.
///
/// # Errors
///
/// A message that names the setting and never shows the value: the
/// setting's own refusal, the driver's refusal of the URL's other parts, or
/// the guard's refusal of a source that is not local in a local-only
/// environment.
pub fn connect_options(setting: &SourceDatabaseUrl, environment: &str) -> Result<SourceOptions> {
    let url = setting.require().map_err(Error::msg)?;
    // sqlx's reason is not passed on: its parser may name the value of a
    // query parameter, and nothing proves that no future reason names a
    // part of the user name or password.
    let options = MySqlConnectOptions::from_str(url).map_err(|_| {
        Error::Message(format!(
            "refusing to start: {SOURCE_DATABASE_URL_KEY} ({SOURCE_DATABASE_URL_ENV}) is a URL \
             that the MySQL driver refuses: check that the user name, the password and the \
             database are percent-encoded UTF-8 and that the query parameters (ssl-mode, \
             statement-cache-capacity and the others sqlx reads) have valid values; the value is \
             not shown, because it holds the password"
        ))
    })?;
    let socket = options
        .get_socket()
        .map(|path| path.to_string_lossy().into_owned());
    source_guard::check(environment, options.get_host(), socket.as_deref()).map_err(Error::msg)?;
    Ok(SourceOptions(options))
}

/// A pool of one connection to the source, opened on first use.
///
/// - `options`: from [`connect_options`], the only way to a
///   [`SourceOptions`], so the source guard has always run.
///
/// One connection, because the import reads the whole source in one locked
/// session (`LOCK TABLES ... READ`, data-migration.md §3.3), which only
/// holds on one connection. Opened on first use, so a run that stops before
/// it reads the source never connects to it.
#[must_use]
pub fn pool(options: SourceOptions) -> MySqlPool {
    MySqlPoolOptions::new()
        .min_connections(0)
        .max_connections(1)
        .connect_lazy_with(options.0)
}

/// Reads `columns` of every row of the legacy table `table`, ordered by
/// `order_by`, from the source.
///
/// - `executor`: the source pool, or the one connection of the locked
///   source session.
/// - `table`: the legacy table; it must be on the source allow-list
///   (`domain::import_mysql::source_tables::SOURCE_TABLES`).
/// - `columns`: the columns to read, in the order each row holds them.
/// - `order_by`: the columns that order the rows, normally the primary key.
///
/// The statement is checked and built by `SourceSelect::new` first, so a
/// table off the list (such as `newsletter`) or a malformed column name is
/// refused before anything is sent: a lazy pool does not even connect.
///
/// # Errors
///
/// The allow-list's refusal, which names the table, or a failed read,
/// described by [`source_error`].
pub async fn select_rows<'e, E>(
    executor: E,
    table: &str,
    columns: &[&str],
    order_by: &[&str],
) -> Result<Vec<MySqlRow>>
where
    E: MySqlExecutor<'e>,
{
    let select = SourceSelect::new(table, columns, order_by).map_err(Error::msg)?;
    // sqlx 0.9 takes only `&'static str` as SQL unless the caller asserts
    // that a built string is safe. This one is: `SourceSelect::new` writes
    // only an allow-listed table name and plain identifiers (letters,
    // digits, underscores), each in backticks, and no value at all.
    sqlx::query(sqlx::AssertSqlSafe(select.sql()))
        .fetch_all(executor)
        .await
        .map_err(|error| {
            source_error(
                &format!("reading the legacy table `{}`", select.table().name()),
                &error,
            )
        })
}

/// The task's error for a failed exchange with the source.
///
/// - `doing`: what the import was doing, for example "reading the legacy
///   table `users`"; it starts the message.
/// - `error`: the driver's error.
///
/// Only parts that cannot hold the source URL's user name, host or password
/// are passed on (data-migration.md §4.1):
///
/// - a server error keeps its MySQL error number and SQLSTATE, never the
///   server's text, which may name the account (`Access denied for user
///   '<user>'@'<host>'`) or the database;
/// - an I/O error keeps its text only when it is the operating system's (an
///   error number) or the standard library's fixed text of its kind, which
///   describe the socket and never the URL; an I/O error that carries a
///   library's own text gets a fixed description instead (`describe_io`);
/// - every other error gets a fixed description of its kind, because the
///   driver's own text (a TLS failure that names the host, a configuration
///   error) may quote the URL.
#[must_use]
pub fn source_error(doing: &str, error: &sqlx::Error) -> Error {
    Error::Message(format!(
        "{doing} from the source failed: {}",
        describe(error)
    ))
}

/// The description of a failed TLS connection to the source. It is fixed,
/// because rustls's text of a certificate that does not match names the
/// server name it expected (the URL's host) and the names the certificate
/// holds.
const TLS_FAILED: &str = "the TLS connection to the source failed: check the URL's ssl-mode and \
                          ssl-ca and the server's certificate (the driver's text is not shown, \
                          because it may name the host)";

/// The URL-free description of a driver error, as [`source_error`] says.
fn describe(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(database) => {
            let number = database
                .try_downcast_ref::<MySqlDatabaseError>()
                .map_or_else(|| "unknown".to_owned(), |mysql| mysql.number().to_string());
            let state = database
                .code()
                .map_or_else(|| "unknown".to_owned(), std::borrow::Cow::into_owned);
            format!(
                "the MySQL server answered with error {number} (SQLSTATE {state}); look the \
                 number up in the MySQL error reference. The server's text is not shown, \
                 because it may name the source account"
            )
        }
        sqlx::Error::Io(io) => describe_io(io),
        sqlx::Error::Tls(_) => TLS_FAILED.to_owned(),
        sqlx::Error::PoolTimedOut => "no connection to the source could be opened in time: \
                                      check that the source is running and reachable"
            .to_owned(),
        sqlx::Error::Protocol(_) => "the source broke the MySQL protocol: check that the URL \
                                     names a MySQL server"
            .to_owned(),
        sqlx::Error::ColumnNotFound(column) => {
            format!("the result has no column {column:?}")
        }
        sqlx::Error::ColumnDecode { index, .. } => {
            format!("the value of column {index} could not be decoded")
        }
        _ => "the MySQL driver failed; its text is not shown, because it may quote the source \
              URL"
        .to_owned(),
    }
}

/// The URL-free description of an I/O error of the source connection.
///
/// - `io`: the I/O error inside the driver's error.
///
/// An error without a payload keeps its text: that text is the operating
/// system's for an error number ("Connection refused (os error 111)") or
/// the standard library's fixed text of the error's kind. An error with a
/// payload carries text that some library wrote, and that text may name the
/// host. sqlx-core 0.9 returns a failed rustls handshake as one: its
/// `handshake` ends with `socket.complete_io().await?`
/// (`src/net/tls/tls_rustls.rs`), and rustls 0.23 wraps its error in an
/// `io::ErrorKind::InvalidData` error, whose text for a certificate that
/// does not match names the expected server name. So `InvalidData` gets
/// the fixed TLS description, and any other payload only its kind (a failed
/// name lookup, for example, is the standard library's "failed to lookup
/// address information" with a payload).
fn describe_io(io: &io::Error) -> String {
    if io.get_ref().is_none() {
        return format!("the connection to the source failed: {io}");
    }
    match io.kind() {
        io::ErrorKind::InvalidData => TLS_FAILED.to_owned(),
        kind => format!(
            "the connection to the source failed ({kind}): check the URL's host and port and \
             that the source is running (the driver's text is not shown, because it may name \
             the host)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    /// A user name, a host and a password that no message may contain.
    const SECRET_PARTS: [&str; 3] = ["user-c0ffee", "host-5ec2e7.example", "pw-9a1f"];

    fn message(error: &sqlx::Error) -> String {
        source_error("reading the legacy table `users`", error).to_string()
    }

    #[test]
    fn errors_that_may_quote_the_url_get_a_fixed_description() {
        let quoting = format!(
            "mysql://{}:{}@{}/verseghy",
            SECRET_PARTS[0], SECRET_PARTS[2], SECRET_PARTS[1]
        );
        for (error, expected) in [
            (
                sqlx::Error::Tls(quoting.clone().into()),
                "the TLS connection to the source failed: check the URL's ssl-mode and ssl-ca",
            ),
            (
                sqlx::Error::Configuration(quoting.clone().into()),
                "the MySQL driver failed; its text is not shown",
            ),
            (
                sqlx::Error::Protocol(quoting.clone()),
                "the source broke the MySQL protocol",
            ),
            (
                sqlx::Error::InvalidArgument(quoting.clone()),
                "the MySQL driver failed; its text is not shown",
            ),
            (
                sqlx::Error::PoolTimedOut,
                "no connection to the source could be opened in time",
            ),
        ] {
            let text = message(&error);
            assert!(
                text.starts_with(&format!(
                    "reading the legacy table `users` from the source failed: {expected}"
                )),
                "{text}"
            );
            for part in SECRET_PARTS {
                assert!(!text.contains(part), "{part}: {text}");
            }
        }
    }

    #[test]
    fn an_io_error_keeps_the_operating_system_text() {
        let error = sqlx::Error::Io(io::Error::from(io::ErrorKind::ConnectionRefused));
        let text = message(&error);
        assert!(
            text.starts_with(
                "reading the legacy table `users` from the source failed: the connection to the \
                 source failed: "
            ),
            "{text}"
        );
        assert!(text.to_lowercase().contains("refused"), "{text}");

        // An error number keeps the operating system's own text.
        let os = io::Error::from_raw_os_error(2);
        let text = message(&sqlx::Error::Io(io::Error::from_raw_os_error(2)));
        assert!(text.ends_with(&os.to_string()), "{text}");
        assert!(text.contains("(os error 2)"), "{text}");
    }

    #[test]
    fn a_failed_tls_handshake_does_not_show_rustls_s_text() {
        // sqlx-core 0.9 passes a failed rustls handshake on as an I/O error
        // of the kind InvalidData that holds rustls's error; this is its
        // text for a certificate that does not match the URL's host.
        let handshake = io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "invalid peer certificate: certificate not valid for name \"{}\"; certificate \
                 is only valid for DnsName(\"{}\")",
                SECRET_PARTS[1], SECRET_PARTS[0]
            ),
        );
        let text = message(&sqlx::Error::Io(handshake));
        assert_eq!(
            text,
            format!("reading the legacy table `users` from the source failed: {TLS_FAILED}")
        );
        for part in SECRET_PARTS {
            assert!(!text.contains(part), "{part}: {text}");
        }
    }

    #[test]
    fn an_io_error_with_a_library_s_text_shows_only_its_kind() {
        for (kind, kind_text) in [
            (io::ErrorKind::UnexpectedEof, "unexpected end of file"),
            (io::ErrorKind::AddrNotAvailable, "address not available"),
            (io::ErrorKind::Other, "other error"),
        ] {
            let error = io::Error::new(
                kind,
                format!(
                    "{} at {} with {}",
                    SECRET_PARTS[0], SECRET_PARTS[1], SECRET_PARTS[2]
                ),
            );
            let text = message(&sqlx::Error::Io(error));
            assert!(
                text.starts_with(&format!(
                    "reading the legacy table `users` from the source failed: the connection to \
                     the source failed ({kind_text}): check the URL's host and port"
                )),
                "{text}"
            );
            for part in SECRET_PARTS {
                assert!(!text.contains(part), "{part}: {text}");
            }
        }
    }

    #[test]
    fn column_errors_name_the_column() {
        assert!(
            message(&sqlx::Error::ColumnNotFound("previewToken".to_owned()))
                .ends_with("the result has no column \"previewToken\"")
        );
        let decode = sqlx::Error::ColumnDecode {
            index: "\"date\"".to_owned(),
            source: "not a date".into(),
        };
        assert!(message(&decode).ends_with("the value of column \"date\" could not be decoded"));
    }

    #[test]
    fn checked_options_show_no_field_and_keep_the_url_s_parts() {
        let url = format!(
            "mysql://{}:{}@localhost:3307/verseghy",
            SECRET_PARTS[0], SECRET_PARTS[2]
        );
        let options =
            connect_options(&SourceDatabaseUrl::new(url), "test").expect("a local source");
        let shown = format!("{options:?}");
        assert_eq!(shown, "SourceOptions { .. }");
        let inner = options.into_inner();
        assert_eq!(inner.get_host(), "localhost");
        assert_eq!(inner.get_port(), 3307);
        assert_eq!(inner.get_username(), SECRET_PARTS[0]);
        assert_eq!(inner.get_database(), Some("verseghy"));
    }

    #[test]
    fn the_guard_runs_on_the_driver_s_own_reading_of_the_url() {
        let refused = |url: &str, environment: &str| {
            connect_options(&SourceDatabaseUrl::new(url.to_owned()), environment)
                .err()
                .map(|error| error.to_string())
        };
        // A socket parameter replaces the host, as in the driver.
        assert_eq!(
            refused(
                "mysql://u:p@db.example.org/verseghy?socket=%2Frun%2Fmysqld%2Fmysqld.sock",
                "test"
            ),
            None
        );
        // Without one, the host as written counts.
        for url in [
            "mysql://u:p@db.example.org/verseghy",
            "mysql://u:p@mysql:3306/verseghy",
            "mysql://u:p@127.0.0.2/verseghy",
            "mysql://u:p@%6Cocalhost/verseghy",
        ] {
            let message = refused(url, "development").expect(url);
            assert!(message.contains("is not local"), "{url}: {message}");
            assert!(!message.contains("u:p"), "{message}");
        }
        // No host at all: the driver connects to localhost.
        assert_eq!(refused("mysql:verseghy", "test"), None);
        for url in [
            "mysql://u:p@localhost/verseghy",
            "mysql://u:p@127.0.0.1:3306/verseghy",
            "mysql://u:p@[::1]:3306/verseghy",
        ] {
            assert_eq!(refused(url, "e2e"), None, "{url}");
        }
        assert_eq!(
            refused("mysql://u:p@mysql:3306/verseghy", "production"),
            None
        );
    }
}
