//! The import's source guard: layer 3 of the guardrails of ADR 0024 §4,
//! applied to the legacy MySQL source (docs/design/data-migration.md §3.3,
//! §4.1).
//!
//! In the environments where `App::load_config` accepts only a local
//! PostgreSQL (`test`, `development` and `e2e`, the local-only class of
//! [`crate::domain::database_target`]), the import accepts only a local
//! MySQL source too: rehearsals there read a throwaway container loaded
//! from a synthetic fixture, never a production database (ADR 0024 §4,
//! "The import source"). In `production` and every other environment the
//! guard does not apply: the staging rehearsals and the cut-over import are
//! operator runs (data-migration.md §3.3, ADR 0003), and the source there is
//! the server the operator names.
//!
//! "Local" is the database guard's own rule, through
//! [`is_local_tcp_host`]: the TCP host `localhost`, `127.0.0.1` or `::1`, or
//! a Unix socket. A container-network name such as `mysql` (the compose
//! service, or a Service in the cluster) is not local, exactly as `postgres`
//! is not for the database guard: a name resolves to whatever the network
//! the process runs in says, and in a cluster pod that may be a production
//! server. Run the import on the host against the published port instead.
//!
//! The guard checks where the MySQL driver will connect, not the URL text:
//! the task passes the host and the socket path of the driver's own parsed
//! options. sqlx-mysql 0.9 connects to the `socket` parameter when the URL
//! has one, and then ignores the host; without it, it connects over TCP to
//! the authority host, as written (not percent-decoded), and to `localhost`
//! when the URL names none. It reads no environment variable for either.
//!
//! The source URL holds the source account's password, so the refusal
//! names the environment, the setting and the variable, and never any part
//! of the value, not even its host (data-migration.md §4.1).

use crate::domain::database_target::{EnvironmentClass, classify_environment, is_local_tcp_host};

use super::source::{SOURCE_DATABASE_URL_ENV, SOURCE_DATABASE_URL_KEY};

/// The source that compose.yaml starts (profile `import`), as the refusal
/// suggests it. Its user and password are the compose services' throwaway
/// `loco`/`loco`, which protect nothing.
pub const COMPOSE_SOURCE_URL: &str = "mysql://loco:loco@127.0.0.1:3306/verseghy";

/// Why the guard refused the source. The message names the environment,
/// the setting and the variable, never the value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "refusing to start: LOCO_ENV={environment}, but {SOURCE_DATABASE_URL_KEY} \
     ({SOURCE_DATABASE_URL_ENV}) points at a MySQL server that is not local. In the test, \
     development and e2e environments the import reads only a local throwaway source: the host \
     localhost, 127.0.0.1 or ::1, or a Unix socket named by the URL's socket parameter, for \
     example the MySQL that compose.yaml starts (podman-compose --profile import up -d mysql) at \
     {COMPOSE_SOURCE_URL}. A container-network name such as mysql is not local; run the import on \
     the host against the published port. Staging and production imports are operator runs \
     (data-migration.md §3.3, ADR 0024 §4). The value is not shown, because it holds the password"
)]
pub struct NonLocalSource {
    /// The Loco environment the task runs in.
    pub environment: String,
}

/// Whether the MySQL driver connects to a local server.
///
/// - `host`: the TCP host of the driver's parsed options, as written in the
///   URL (the driver's default `localhost` when the URL names none).
/// - `socket`: the Unix socket path of the driver's parsed options (the
///   URL's `socket` parameter), if any. A socket replaces the TCP host, so a
///   non-empty path is local whatever the host says; an empty one names no
///   socket and is not local.
#[must_use]
pub fn is_local(host: &str, socket: Option<&str>) -> bool {
    match socket {
        Some(path) => !path.is_empty(),
        None => is_local_tcp_host(host),
    }
}

/// Checks the source the import is about to use against the guard.
///
/// - `environment`: the Loco environment name, as Loco resolved it.
/// - `host` and `socket`: where the driver will connect, as for
///   [`is_local`].
///
/// # Errors
///
/// [`NonLocalSource`] when `environment` is local-only (`test`,
/// `development`, `e2e`) and the source is not local. Every other
/// environment accepts every source.
pub fn check(environment: &str, host: &str, socket: Option<&str>) -> Result<(), NonLocalSource> {
    if classify_environment(environment) != EnvironmentClass::LocalOnly || is_local(host, socket) {
        return Ok(());
    }
    Err(NonLocalSource {
        environment: environment.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::database_target::LOCAL_ONLY_ENVIRONMENTS;
    use proptest::prelude::*;

    #[test]
    fn the_three_local_hosts_and_a_socket_are_local() {
        for host in ["localhost", "LOCALHOST", "127.0.0.1", "::1", "[::1]"] {
            assert!(is_local(host, None), "{host}");
        }
        for socket in [
            "/run/mysqld/mysqld.sock",
            "/var/lib/mysql/mysql.sock",
            "mysql.sock",
        ] {
            assert!(is_local("localhost", Some(socket)), "{socket}");
        }
    }

    #[test]
    fn names_other_loopback_addresses_and_container_services_are_not_local() {
        for host in [
            "mysql",
            "db.example.org",
            "verseghy-gimnazium.net",
            "127.0.0.2",
            "0.0.0.0",
            "[::2]",
            "localhost.",
            "local%68ost",
            "%2Frun%2Fmysqld%2Fmysqld.sock",
            "",
        ] {
            assert!(!is_local(host, None), "{host:?}");
        }
    }

    #[test]
    fn a_socket_replaces_the_host_as_the_driver_does() {
        // sqlx-mysql 0.9 connects to the socket and never to the host when
        // the URL has a socket parameter.
        assert!(is_local("db.example.org", Some("/run/mysqld/mysqld.sock")));
        // An empty socket parameter names no socket.
        assert!(!is_local("localhost", Some("")));
    }

    #[test]
    fn local_only_environments_refuse_a_non_local_source() {
        for environment in LOCAL_ONLY_ENVIRONMENTS {
            assert_eq!(
                check(environment, "mysql", None),
                Err(NonLocalSource {
                    environment: (*environment).to_owned(),
                })
            );
            assert_eq!(check(environment, "127.0.0.1", None), Ok(()));
            assert_eq!(
                check(environment, "mysql", Some("/run/mysqld/mysqld.sock")),
                Ok(())
            );
        }
    }

    #[test]
    fn production_and_other_environments_accept_any_source() {
        for environment in ["production", "staging", "Test", ""] {
            assert_eq!(check(environment, "mysql", None), Ok(()), "{environment}");
            assert_eq!(check(environment, "db.example.org", Some("")), Ok(()));
        }
    }

    #[test]
    fn the_refusal_names_the_environment_and_the_setting_and_says_what_is_local() {
        let message = check("development", "db.example.org", None)
            .expect_err("refused")
            .to_string();
        assert!(
            message.starts_with(
                "refusing to start: LOCO_ENV=development, but settings.import.source_database_url \
                 (IMPORT_SOURCE_DATABASE_URL) points at a MySQL server that is not local."
            ),
            "{message}"
        );
        for part in [
            "localhost, 127.0.0.1 or ::1",
            "socket parameter",
            COMPOSE_SOURCE_URL,
            "operator runs",
            "The value is not shown",
        ] {
            assert!(message.contains(part), "{part}: {message}");
        }
        assert!(!message.contains("db.example.org"), "{message}");
    }

    #[test]
    fn the_compose_source_passes_the_guard() {
        // The example in the message must itself be accepted.
        assert_eq!(check("test", "127.0.0.1", None), Ok(()));
        assert!(COMPOSE_SOURCE_URL.contains("@127.0.0.1:3306/"));
    }

    /// A DNS-style host name that is not `localhost`.
    fn remote_host() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9-]{0,10}(\\.[a-z][a-z0-9-]{0,10}){0,3}".prop_filter("not localhost", |host| {
            !host.eq_ignore_ascii_case("localhost")
        })
    }

    proptest! {
        #[test]
        fn a_non_empty_socket_is_local_whatever_the_host(
            host in any::<String>(),
            socket in ".{1,40}",
        ) {
            prop_assert!(is_local(&host, Some(&socket)));
        }

        #[test]
        fn generated_remote_hosts_are_refused_in_local_only_environments(
            host in remote_host(),
            environment in prop_oneof![Just("test"), Just("development"), Just("e2e")],
        ) {
            prop_assert_eq!(
                check(environment, &host, None),
                Err(NonLocalSource { environment: environment.to_owned() })
            );
        }

        #[test]
        fn other_environments_accept_every_source(
            environment in any::<String>()
                .prop_filter("not local-only", |name| !LOCAL_ONLY_ENVIRONMENTS.contains(&name.as_str())),
            host in any::<String>(),
            socket in proptest::option::of(any::<String>()),
        ) {
            prop_assert_eq!(check(&environment, &host, socket.as_deref()), Ok(()));
        }

        #[test]
        fn the_refusal_never_contains_the_host(
            host in "[a-z]{3,10}\\.[a-z]{2,10}\\.example",
            environment in prop_oneof![Just("test"), Just("development"), Just("e2e")],
        ) {
            let refusal = check(environment, &host, None).expect_err("refused");
            let shown = [refusal.to_string(), format!("{refusal:?}")];
            for text in shown {
                prop_assert!(!text.contains(&host), "{}", text);
            }
        }
    }
}
