//! The database-target guard: layer 3 of the guardrails of ADR 0024 §4.
//!
//! Loco's test defaults are destructive: `dangerously_truncate` and
//! `dangerously_recreate` wipe the connected database at boot, and
//! `db reset` drops every table in it. Deny rules and the Bash hook only see
//! command text, so they can be bypassed. This guard cannot: `App` calls it
//! from `Hooks::load_config`, which every Loco entry point runs (the test boot
//! helpers, `cargo loco` and the `backend3-cli` binary) before any connection
//! is made.
//!
//! The rules (ADR 0024 §4, layer 3):
//!
//! 1. In the `test` and `development` environments, every database URI must
//!    name a local host. backend3 also treats `e2e`, the CI end-to-end
//!    environment, as local-only: its configuration only ever points at the
//!    throwaway CI database, so the stricter rule costs nothing there.
//! 2. In any environment, a destructive start-up flag may only be on when the
//!    connection it acts on is local.
//! 3. In `production`, a destructive start-up flag is never allowed.
//!
//! "Local" means `localhost`, `127.0.0.1`, `::1` or a Unix socket (a host
//! that starts with `/`). Anything else is not local, including other
//! loopback spellings such as `127.0.0.2` or `0.0.0.0`: the list is short on
//! purpose, so nobody has to reason about which aliases are safe.
//!
//! The host is read the way the PostgreSQL driver (sqlx 0.9) reads it: from
//! the URI authority and from every `host` and `hostaddr` query parameter,
//! because sqlx lets those parameters override the authority. All of them
//! must be local. A URI without any host is refused in local-only
//! environments, because sqlx would then take the host from the `PGHOST` or
//! `PGHOSTADDR` environment variables, which this pure function cannot see.
//!
//! Error messages name the environment, the configuration key and the host,
//! never the URI itself: it may contain a password.

// `core::net` holds the same address types as `std::net`, without any of the
// socket APIs; `src/domain/` must not name `std::net` (ADR 0008).
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use percent_encoding::percent_decode_str;
use url::Url;

/// The Loco environment whose destructive start-up flags are always refused.
pub const PRODUCTION: &str = "production";

/// The environments in which every database URI must be local.
pub const LOCAL_ONLY_ENVIRONMENTS: &[&str] = &["development", "test", "e2e"];

/// The URI schemes that sqlx accepts for PostgreSQL.
pub const POSTGRES_SCHEMES: &[&str] = &["postgres", "postgresql"];

/// How strictly an environment's database targets are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentClass {
    /// `development`, `test` and `e2e`: every URI must be local (rule 1).
    LocalOnly,
    /// `production`: no destructive start-up flag at all (rule 3).
    Production,
    /// Any other environment name: only rule 2 applies.
    Other,
}

/// Classifies a Loco environment name.
///
/// - `environment`: the environment name as Loco resolved it, for example
///   `test` or `production`. The comparison is exact, as Loco's own is.
#[must_use]
pub fn classify_environment(environment: &str) -> EnvironmentClass {
    if environment == PRODUCTION {
        EnvironmentClass::Production
    } else if LOCAL_ONLY_ENVIRONMENTS.contains(&environment) {
        EnvironmentClass::LocalOnly
    } else {
        EnvironmentClass::Other
    }
}

/// Where a database URI points, as far as this guard is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locality {
    /// Every host in the URI is local.
    Local,
    /// At least one host is not local; `host` is the first such host, as it
    /// appears in the URI.
    NonLocal {
        /// The offending host.
        host: String,
    },
    /// The URI names no host at all, so the driver would choose one from the
    /// process environment.
    NoHost,
    /// The URI is not a PostgreSQL connection URI.
    Invalid {
        /// Why it was not accepted. Never contains the URI itself.
        reason: String,
    },
}

impl Locality {
    /// A short description for error messages, for example `'db.example.org'`.
    fn describe(&self) -> String {
        match self {
            Self::Local => "local".to_owned(),
            Self::NonLocal { host } => format!("'{host}'"),
            Self::NoHost => "not named in the URI".to_owned(),
            Self::Invalid { reason } => format!("unknown (the URI is invalid: {reason})"),
        }
    }
}

/// Determines where a PostgreSQL connection URI points.
///
/// - `uri`: the connection URI from the configuration, for example
///   `postgres://loco:loco@localhost:5432/backend3_test`.
#[must_use]
pub fn locality(uri: &str) -> Locality {
    let url = match Url::parse(uri) {
        Ok(url) => url,
        Err(error) => {
            return Locality::Invalid {
                reason: error.to_string(),
            };
        }
    };
    if !POSTGRES_SCHEMES.contains(&url.scheme()) {
        return Locality::Invalid {
            reason: format!(
                "scheme '{}' is not PostgreSQL; use postgres:// or postgresql://",
                url.scheme()
            ),
        };
    }

    // sqlx percent-decodes the authority host only to detect a socket path;
    // a TCP host is used as written. Query values are already decoded by
    // `query_pairs`, exactly as sqlx reads them.
    let mut hosts: Vec<(String, bool)> = Vec::new();
    if let Some(host) = url.host_str().filter(|host| !host.is_empty()) {
        hosts.push((host.to_owned(), true));
    }
    for (key, value) in url.query_pairs() {
        if key == "host" || key == "hostaddr" {
            hosts.push((value.into_owned(), false));
        }
    }

    if hosts.is_empty() {
        return Locality::NoHost;
    }
    match hosts
        .into_iter()
        .find(|(host, from_authority)| !is_local_host(host, *from_authority))
    {
        Some((host, _)) => Locality::NonLocal { host },
        None => Locality::Local,
    }
}

/// Whether one host value counts as local.
///
/// - `host`: the host as written in the URI authority or a query parameter.
/// - `from_authority`: true when it came from the authority, which sqlx
///   percent-decodes before checking for a socket path.
fn is_local_host(host: &str, from_authority: bool) -> bool {
    let socket_candidate = if from_authority {
        percent_decode_str(host).decode_utf8_lossy().into_owned()
    } else {
        host.to_owned()
    };
    if socket_candidate.starts_with('/') {
        return true;
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address == Ipv4Addr::LOCALHOST,
        Ok(IpAddr::V6(address)) => address == Ipv6Addr::LOCALHOST,
        Err(_) => false,
    }
}

/// One database connection that the configuration asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection<'a> {
    /// The configuration key the URI comes from, for example `database.uri`.
    /// Error messages name it.
    pub uri_key: &'a str,
    /// The connection URI.
    pub uri: &'a str,
    /// The configuration keys of the destructive start-up flags that are set
    /// to `true` for this connection, for example
    /// `database.dangerously_truncate`. Empty when none is on.
    pub enabled_destructive_flags: Vec<&'a str>,
}

/// Why the guard refused a configuration. The `Display` text is the message
/// the operator sees; it says what was found and how to fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// Rule 1: a non-local host in a local-only environment.
    #[error(
        "Refusing to start: LOCO_ENV={environment}, but the database host in {uri_key} is \
         '{host}'. Tests and development may only use a local database (localhost, 127.0.0.1, \
         ::1 or a Unix socket). Point DATABASE_URL at a local PostgreSQL, for example the one \
         that compose.yaml starts."
    )]
    NonLocalHost {
        /// The environment name.
        environment: String,
        /// The configuration key of the URI.
        uri_key: String,
        /// The first non-local host found.
        host: String,
    },
    /// Rule 1: a URI without a host in a local-only environment.
    #[error(
        "Refusing to start: LOCO_ENV={environment}, but {uri_key} names no database host, so the \
         PostgreSQL driver would take it from PGHOST or PGHOSTADDR. Tests and development may \
         only use a local database (localhost, 127.0.0.1, ::1 or a Unix socket). Write the host \
         into the URI, for example postgres://loco:loco@localhost:5432/backend3_test, or name a \
         socket directory with ?host=/run/postgresql."
    )]
    NoHost {
        /// The environment name.
        environment: String,
        /// The configuration key of the URI.
        uri_key: String,
    },
    /// Rule 1: a URI that is not a PostgreSQL URI in a local-only environment.
    #[error(
        "Refusing to start: LOCO_ENV={environment}, but {uri_key} is not a valid PostgreSQL \
         connection URI ({reason}). Use the form postgres://user:password@localhost:5432/name."
    )]
    InvalidUri {
        /// The environment name.
        environment: String,
        /// The configuration key of the URI.
        uri_key: String,
        /// Why the URI was not accepted.
        reason: String,
    },
    /// Rule 3: a destructive start-up flag in production.
    #[error(
        "Refusing to start: LOCO_ENV=production, but {flag} is true. That flag wipes data at \
         startup and is never allowed in production. Set it to false in config/production.yaml."
    )]
    DestructiveFlagInProduction {
        /// The configuration key of the flag.
        flag: String,
    },
    /// Rule 2: a destructive start-up flag on a connection that is not local.
    #[error(
        "Refusing to start: LOCO_ENV={environment} sets {flag} to true, but the database host in \
         {uri_key} is {target}. Destructive start-up flags may only act on a local database \
         (localhost, 127.0.0.1, ::1 or a Unix socket). Set the flag to false or point the URI at \
         a local PostgreSQL."
    )]
    DestructiveFlagOnNonLocalDatabase {
        /// The environment name.
        environment: String,
        /// The configuration key of the flag.
        flag: String,
        /// The configuration key of the URI.
        uri_key: String,
        /// A description of the host that was found.
        target: String,
    },
}

/// Checks the database connections of one configuration against the rules of
/// this module and returns the first violation.
///
/// - `environment`: the Loco environment name the configuration was loaded
///   for.
/// - `connections`: every database connection the configuration describes,
///   with the destructive flags that act on each.
///
/// # Errors
///
/// Returns a [`Refusal`] that names the environment, the configuration key
/// and the host involved. Production flags are checked first, then each
/// connection in the given order.
pub fn check(environment: &str, connections: &[Connection<'_>]) -> Result<(), Refusal> {
    let class = classify_environment(environment);

    if class == EnvironmentClass::Production
        && let Some(flag) = connections
            .iter()
            .flat_map(|connection| connection.enabled_destructive_flags.iter())
            .next()
    {
        return Err(Refusal::DestructiveFlagInProduction {
            flag: (*flag).to_owned(),
        });
    }

    for connection in connections {
        let found = locality(connection.uri);
        if class == EnvironmentClass::LocalOnly {
            let refusal = match &found {
                Locality::Local => None,
                Locality::NonLocal { host } => Some(Refusal::NonLocalHost {
                    environment: environment.to_owned(),
                    uri_key: connection.uri_key.to_owned(),
                    host: host.clone(),
                }),
                Locality::NoHost => Some(Refusal::NoHost {
                    environment: environment.to_owned(),
                    uri_key: connection.uri_key.to_owned(),
                }),
                Locality::Invalid { reason } => Some(Refusal::InvalidUri {
                    environment: environment.to_owned(),
                    uri_key: connection.uri_key.to_owned(),
                    reason: reason.clone(),
                }),
            };
            if let Some(refusal) = refusal {
                return Err(refusal);
            }
        }
        if found != Locality::Local
            && let Some(flag) = connection.enabled_destructive_flags.first()
        {
            return Err(Refusal::DestructiveFlagOnNonLocalDatabase {
                environment: environment.to_owned(),
                flag: (*flag).to_owned(),
                uri_key: connection.uri_key.to_owned(),
                target: found.describe(),
            });
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const TRUNCATE: &str = "database.dangerously_truncate";
    const RECREATE: &str = "database.dangerously_recreate";

    fn connection<'a>(uri: &'a str, flags: &[&'a str]) -> Connection<'a> {
        Connection {
            uri_key: "database.uri",
            uri,
            enabled_destructive_flags: flags.to_vec(),
        }
    }

    #[test]
    fn environments_are_classified_exactly() {
        assert_eq!(
            classify_environment("production"),
            EnvironmentClass::Production
        );
        assert_eq!(
            classify_environment("development"),
            EnvironmentClass::LocalOnly
        );
        assert_eq!(classify_environment("test"), EnvironmentClass::LocalOnly);
        assert_eq!(classify_environment("e2e"), EnvironmentClass::LocalOnly);
        assert_eq!(classify_environment("staging"), EnvironmentClass::Other);
        assert_eq!(classify_environment("Production"), EnvironmentClass::Other);
        assert_eq!(classify_environment(""), EnvironmentClass::Other);
    }

    #[test]
    fn local_hosts() {
        for uri in [
            "postgres://loco:loco@localhost:5432/backend3_test",
            "postgresql://loco:loco@localhost/backend3_test",
            "postgres://LocalHost/db",
            "postgres://127.0.0.1:5432/db",
            "postgres://[::1]:5432/db",
            "postgres://[0:0:0:0:0:0:0:1]/db",
            "postgres://%2Frun%2Fpostgresql/db",
            "postgres:///db?host=/run/postgresql",
            "postgres://localhost/db?host=/tmp&hostaddr=127.0.0.1",
            "  postgres://localhost/db  ",
        ] {
            assert_eq!(locality(uri), Locality::Local, "{uri}");
        }
    }

    #[test]
    fn non_local_hosts() {
        for (uri, host) in [
            (
                "postgres://loco:loco@db.example.org:5432/x",
                "db.example.org",
            ),
            ("postgres://postgres:5432/x", "postgres"),
            ("postgres://127.0.0.2/x", "127.0.0.2"),
            ("postgres://0.0.0.0/x", "0.0.0.0"),
            ("postgres://127.1/x", "127.1"),
            ("postgres://[::ffff:127.0.0.1]/x", "[::ffff:7f00:1]"),
            ("postgres://local%68ost/x", "local%68ost"),
            ("postgres://localhost./x", "localhost."),
            ("postgres://h1,h2/x", "h1,h2"),
            (
                "postgres://localhost/x?host=db.example.org",
                "db.example.org",
            ),
            ("postgres://localhost/x?hostaddr=10.0.0.5", "10.0.0.5"),
            ("postgres://%2Ftmp/x?host=remote", "remote"),
            ("postgres:///x?host=", ""),
        ] {
            assert_eq!(
                locality(uri),
                Locality::NonLocal {
                    host: host.to_owned()
                },
                "{uri}"
            );
        }
    }

    #[test]
    fn uris_without_a_host() {
        assert_eq!(locality("postgres:///backend3_test"), Locality::NoHost);
        assert_eq!(locality("postgres:backend3_test"), Locality::NoHost);
    }

    #[test]
    fn invalid_uris_never_echo_the_uri() {
        for uri in [
            "",
            "localhost:5432",
            "not a uri",
            "postgres://u:secret@/db",
            "mysql://u:secret@localhost/db",
            "sqlite://secret.sqlite",
        ] {
            match locality(uri) {
                Locality::Invalid { reason } => {
                    assert!(!reason.contains("secret"), "{uri}: {reason}");
                }
                other => panic!("{uri}: expected Invalid, got {other:?}"),
            }
        }
    }

    #[test]
    fn local_only_environments_refuse_non_local_hosts() {
        for environment in LOCAL_ONLY_ENVIRONMENTS {
            let refusal = check(
                environment,
                &[connection("postgres://db.example.org/x", &[])],
            )
            .expect_err("non-local host must be refused");
            assert_eq!(
                refusal,
                Refusal::NonLocalHost {
                    environment: (*environment).to_owned(),
                    uri_key: "database.uri".to_owned(),
                    host: "db.example.org".to_owned(),
                }
            );
            let message = refusal.to_string();
            assert!(message.starts_with(&format!("Refusing to start: LOCO_ENV={environment}")));
            assert!(message.contains("'db.example.org'"), "{message}");
            assert!(
                message.contains("Point DATABASE_URL at a local PostgreSQL"),
                "{message}"
            );
        }
    }

    #[test]
    fn local_only_environments_refuse_missing_and_invalid_hosts() {
        assert_eq!(
            check("test", &[connection("postgres:///x", &[])]),
            Err(Refusal::NoHost {
                environment: "test".to_owned(),
                uri_key: "database.uri".to_owned(),
            })
        );
        assert!(matches!(
            check("development", &[connection("mysql://localhost/x", &[])]),
            Err(Refusal::InvalidUri { .. })
        ));
    }

    #[test]
    fn local_only_environments_accept_local_hosts_and_flags() {
        assert_eq!(
            check(
                "test",
                &[connection(
                    "postgres://loco:loco@localhost:5432/backend3_test",
                    &[TRUNCATE, RECREATE]
                )]
            ),
            Ok(())
        );
    }

    #[test]
    fn every_connection_is_checked() {
        let connections = [
            connection("postgres://localhost/x", &[]),
            Connection {
                uri_key: "queue.uri",
                uri: "postgres://queue.example.org/x",
                enabled_destructive_flags: vec![],
            },
        ];
        assert_eq!(
            check("development", &connections),
            Err(Refusal::NonLocalHost {
                environment: "development".to_owned(),
                uri_key: "queue.uri".to_owned(),
                host: "queue.example.org".to_owned(),
            })
        );
    }

    #[test]
    fn production_refuses_destructive_flags_even_on_local_hosts() {
        assert_eq!(
            check(
                "production",
                &[connection("postgres://localhost/x", &[RECREATE])]
            ),
            Err(Refusal::DestructiveFlagInProduction {
                flag: RECREATE.to_owned()
            })
        );
        let message = check("production", &[connection("postgres://db/x", &[TRUNCATE])])
            .expect_err("refused")
            .to_string();
        assert!(
            message.contains("database.dangerously_truncate is true"),
            "{message}"
        );
    }

    #[test]
    fn production_accepts_remote_hosts_without_flags() {
        assert_eq!(
            check(
                "production",
                &[connection("postgres://u:p@postgres:5432/verseghy", &[])]
            ),
            Ok(())
        );
    }

    #[test]
    fn other_environments_refuse_flags_on_non_local_hosts_only() {
        assert_eq!(
            check("staging", &[connection("postgres://postgres/x", &[])]),
            Ok(())
        );
        assert_eq!(
            check(
                "staging",
                &[connection("postgres://localhost/x", &[TRUNCATE])]
            ),
            Ok(())
        );
        assert_eq!(
            check(
                "staging",
                &[connection("postgres://postgres/x", &[TRUNCATE])]
            ),
            Err(Refusal::DestructiveFlagOnNonLocalDatabase {
                environment: "staging".to_owned(),
                flag: TRUNCATE.to_owned(),
                uri_key: "database.uri".to_owned(),
                target: "'postgres'".to_owned(),
            })
        );
        assert!(matches!(
            check("staging", &[connection("postgres:///x", &[RECREATE])]),
            Err(Refusal::DestructiveFlagOnNonLocalDatabase { .. })
        ));
    }

    #[test]
    fn no_connections_is_fine() {
        assert_eq!(check("test", &[]), Ok(()));
        assert_eq!(check("production", &[]), Ok(()));
    }

    /// A DNS-style host name that is never `localhost`.
    fn remote_host() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9-]{0,10}(\\.[a-z][a-z0-9-]{0,10}){0,3}".prop_filter("not localhost", |host| {
            !host.eq_ignore_ascii_case("localhost")
        })
    }

    fn local_host() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("localhost".to_owned()),
            Just("LOCALHOST".to_owned()),
            Just("127.0.0.1".to_owned()),
            Just("[::1]".to_owned()),
            Just("%2Frun%2Fpostgresql".to_owned()),
        ]
    }

    fn credentials() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(String::new()),
            "[a-z]{1,8}@",
            "[a-z]{1,8}:[a-zA-Z0-9]{1,12}@",
        ]
    }

    fn port() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(String::new()),
            (1u16..).prop_map(|port| format!(":{port}"))
        ]
    }

    proptest! {
        #[test]
        fn only_the_exact_environment_names_are_classified(environment in any::<String>()) {
            let expected = match environment.as_str() {
                "production" => EnvironmentClass::Production,
                "development" | "test" | "e2e" => EnvironmentClass::LocalOnly,
                _ => EnvironmentClass::Other,
            };
            prop_assert_eq!(classify_environment(&environment), expected);
        }

        #[test]
        fn decorated_environment_names_are_other(
            name in prop_oneof![
                Just("production"), Just("development"), Just("test"), Just("e2e")
            ],
            prefix in "[ A-Za-z0-9_.-]{0,3}",
            suffix in "[ A-Za-z0-9_.-]{0,3}",
            uppercase in any::<bool>(),
        ) {
            let mut decorated = format!("{prefix}{name}{suffix}");
            if uppercase {
                decorated = decorated.to_uppercase();
            }
            prop_assume!(decorated != name);
            prop_assert_eq!(classify_environment(&decorated), EnvironmentClass::Other);
        }

        #[test]
        fn locality_never_panics(uri in any::<String>()) {
            let _ = locality(&uri);
        }

        #[test]
        fn generated_local_uris_pass_everywhere_without_flags(
            scheme in prop_oneof![Just("postgres"), Just("postgresql")],
            auth in credentials(),
            host in local_host(),
            port in port(),
            database in "[a-z_]{1,16}",
            environment in prop_oneof![
                Just("test"), Just("development"), Just("e2e"), Just("production"), Just("staging")
            ],
        ) {
            let uri = format!("{scheme}://{auth}{host}{port}/{database}");
            prop_assert_eq!(locality(&uri), Locality::Local);
            prop_assert_eq!(check(environment, &[connection(&uri, &[])]), Ok(()));
        }

        #[test]
        fn generated_remote_uris_are_refused_in_local_only_environments(
            auth in credentials(),
            host in remote_host(),
            port in port(),
            database in "[a-z_]{1,16}",
            environment in prop_oneof![Just("test"), Just("development"), Just("e2e")],
        ) {
            let uri = format!("postgres://{auth}{host}{port}/{database}");
            let refusal = check(environment, &[connection(&uri, &[])]);
            prop_assert_eq!(
                refusal,
                Err(Refusal::NonLocalHost {
                    environment: environment.to_owned(),
                    uri_key: "database.uri".to_owned(),
                    host,
                })
            );
        }

        #[test]
        fn a_remote_host_parameter_overrides_a_local_authority(
            local in local_host(),
            remote in remote_host(),
            key in prop_oneof![Just("host"), Just("hostaddr")],
        ) {
            let uri = format!("postgres://{local}/db?{key}={remote}");
            prop_assert_eq!(locality(&uri), Locality::NonLocal { host: remote });
        }

        #[test]
        fn production_refuses_any_destructive_flag(
            host in prop_oneof![local_host(), remote_host()],
            flags in proptest::sample::subsequence(vec![TRUNCATE, RECREATE], 1..=2),
        ) {
            let uri = format!("postgres://{host}/db");
            let result = check("production", &[connection(&uri, &flags)]);
            prop_assert_eq!(
                result,
                Err(Refusal::DestructiveFlagInProduction { flag: flags[0].to_owned() })
            );
        }

        #[test]
        fn messages_never_contain_the_password(
            password in "[a-zA-Z0-9]{12,20}",
            host in remote_host(),
            environment in prop_oneof![Just("test"), Just("staging"), Just("production")],
        ) {
            let uri = format!("postgres://user:{password}@{host}/db");
            if let Err(refusal) = check(environment, &[connection(&uri, &[TRUNCATE])]) {
                prop_assert!(!refusal.to_string().contains(&password));
            }
        }
    }
}
