//! The CLI pre-check: the second code-level guard of ADR 0024 §4, layer 3.
//!
//! `Hooks::load_config` cannot see which subcommand runs, so the
//! database-target guard cannot stop `db reset` in production. This check can:
//! `src/bin/main.rs` runs it on the raw arguments before it hands control to
//! `loco_rs::cli::main`, so nothing has been loaded or connected yet. Both
//! `backend3-cli ...` and `cargo loco ...` go through it, because the `loco`
//! cargo alias is `run --` and `backend3-cli` is the default binary.
//!
//! In the `production` environment it refuses the three subcommands that wipe
//! data:
//!
//! - `db reset` (drops every table, then migrates up);
//! - `db truncate` (deletes the rows of every table);
//! - `db seed --reset` / `db seed -r` (a reset before seeding).
//!
//! `db down` stays allowed so that an operator can roll back one migration;
//! the agent hook blocks it for agents (ADR 0024 §4, layer 2).
//!
//! The environment resolves as Loco resolves it: the value of the global
//! `-e`/`--environment` option when present, otherwise the ambient
//! environment (`LOCO_ENV`, `RAILS_ENV`, `NODE_ENV`, then `development`),
//! which the caller passes in. The parser knows the option grammar of the
//! loco-rs 1.2.0 CLI for the `db` subcommands (which options take a value)
//! and errs on the side of refusing: when `-e` appears more than once, any
//! `production` value counts.

/// The environment in which the destructive subcommands are refused.
pub const PRODUCTION: &str = "production";

/// A subcommand that wipes data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructiveCommand {
    /// `db reset`.
    DbReset,
    /// `db truncate`.
    DbTruncate,
    /// `db seed --reset` or `db seed -r`.
    DbSeedReset,
}

impl DestructiveCommand {
    /// The command as an operator types it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DbReset => "db reset",
            Self::DbTruncate => "db truncate",
            Self::DbSeedReset => "db seed --reset",
        }
    }
}

/// Where the environment name came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentSource {
    /// The `-e`/`--environment` command-line option.
    Option,
    /// The process environment, as `loco_rs::environment::resolve_from_env`
    /// read it.
    Ambient,
}

/// The refusal of a destructive subcommand in production. The `Display`
/// text is the message the operator sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The refused subcommand.
    pub command: DestructiveCommand,
    /// Where `production` came from.
    pub source: EnvironmentSource,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let source = match self.source {
            EnvironmentSource::Option => "from --environment",
            EnvironmentSource::Ambient => "from LOCO_ENV, RAILS_ENV or NODE_ENV",
        };
        write!(
            formatter,
            "Refusing to run `{}`: the environment is production ({source}), and this command \
             deletes data. backend3 never runs `db reset`, `db truncate` or `db seed --reset` in \
             production (ADR 0024). To roll back one migration, use `db down`; to recover data, \
             follow the restore procedure in the operator runbook.",
            self.command.as_str()
        )
    }
}

impl core::error::Error for Refusal {}

/// Options of the `db seed` subcommand that take a value (loco-rs 1.2.0
/// `DbCommands::Seed`). Their value must not be read as a flag.
const SEED_VALUE_OPTIONS: &[&str] = &["--from", "--dump-tables"];

/// What the parser found in the arguments.
#[derive(Debug, Default)]
struct Parsed {
    /// Positional arguments in order, with option values removed.
    positionals: Vec<String>,
    /// Every value given to `-e`/`--environment`.
    environments: Vec<String>,
    /// Whether `-r` or `--reset` appeared after `seed`.
    seed_reset: bool,
}

/// Splits the arguments into positionals, environment values and the seed
/// reset flag, following clap's rules for the options that matter here.
fn parse(args: &[String]) -> Parsed {
    let mut parsed = Parsed::default();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        index += 1;
        let in_seed = parsed.positionals.len() >= 2
            && parsed.positionals[0] == "db"
            && parsed.positionals[1] == "seed";

        if arg == "--" {
            parsed.positionals.extend(args[index..].iter().cloned());
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (long, None),
            };
            if name == "environment" {
                let value = inline.map(str::to_owned).or_else(|| {
                    let next = args.get(index).cloned();
                    index += 1;
                    next
                });
                parsed.environments.extend(value);
            } else if in_seed && name == "reset" {
                parsed.seed_reset = true;
            } else if in_seed && inline.is_none() && SEED_VALUE_OPTIONS.contains(&arg) {
                index += 1;
            }
            continue;
        }
        if let Some(cluster) = arg.strip_prefix('-').filter(|cluster| !cluster.is_empty()) {
            // A cluster of short flags, such as `-rd`. `-e` takes the rest of
            // the cluster as its value (`-eproduction`, `-e=production`), or
            // the next argument when nothing follows it.
            for (position, short) in cluster.char_indices() {
                match short {
                    'e' => {
                        let rest = &cluster[position + short.len_utf8()..];
                        let rest = rest.strip_prefix('=').unwrap_or(rest);
                        if rest.is_empty() {
                            parsed.environments.extend(args.get(index).cloned());
                            index += 1;
                        } else {
                            parsed.environments.push(rest.to_owned());
                        }
                        break;
                    }
                    'r' if in_seed => parsed.seed_reset = true,
                    _ => {}
                }
            }
            continue;
        }
        parsed.positionals.push(arg.to_owned());
    }
    parsed
}

/// Checks the command-line arguments of `backend3-cli` before Loco runs them.
///
/// - `args`: the arguments after the program name, as the process received
///   them.
/// - `ambient_environment`: the environment name Loco would use when no
///   `-e`/`--environment` option is given, from
///   `loco_rs::environment::resolve_from_env()`.
///
/// # Errors
///
/// Returns a [`Refusal`] when the environment resolves to `production` and
/// the arguments name `db reset`, `db truncate` or `db seed` with `--reset`.
pub fn check(args: &[String], ambient_environment: &str) -> Result<(), Refusal> {
    let parsed = parse(args);

    let command = match parsed
        .positionals
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["db", "reset", ..] => DestructiveCommand::DbReset,
        ["db", "truncate", ..] => DestructiveCommand::DbTruncate,
        ["db", "seed", ..] if parsed.seed_reset => DestructiveCommand::DbSeedReset,
        _ => return Ok(()),
    };

    let source = if parsed.environments.is_empty() {
        (ambient_environment == PRODUCTION).then_some(EnvironmentSource::Ambient)
    } else {
        parsed
            .environments
            .iter()
            .any(|environment| environment == PRODUCTION)
            .then_some(EnvironmentSource::Option)
    };

    match source {
        Some(source) => Err(Refusal { command, source }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn refused(line: &str, ambient: &str) -> Option<DestructiveCommand> {
        check(&args(line), ambient)
            .err()
            .map(|refusal| refusal.command)
    }

    #[test]
    fn refuses_the_three_commands_in_ambient_production() {
        assert_eq!(
            refused("db reset", "production"),
            Some(DestructiveCommand::DbReset)
        );
        assert_eq!(
            refused("db truncate", "production"),
            Some(DestructiveCommand::DbTruncate)
        );
        assert_eq!(
            refused("db seed --reset", "production"),
            Some(DestructiveCommand::DbSeedReset)
        );
        assert_eq!(
            refused("db seed -r", "production"),
            Some(DestructiveCommand::DbSeedReset)
        );
        assert_eq!(
            refused("db seed -dr", "production"),
            Some(DestructiveCommand::DbSeedReset)
        );
        assert_eq!(
            refused("db seed --from src/fixtures --reset", "production"),
            Some(DestructiveCommand::DbSeedReset)
        );
    }

    #[test]
    fn allows_everything_else_in_production() {
        for line in [
            "",
            "start",
            "start --server-and-worker",
            "db migrate",
            "db down",
            "db down 2",
            "db status",
            "db seed",
            "db seed --dump",
            "db seed --from -r",
            "db seed --from=-r",
            "db seed --dump-tables r",
            "task reset",
            "jobs tidy",
            "routes",
            "reset",
            "db",
        ] {
            assert_eq!(check(&args(line), "production"), Ok(()), "{line}");
        }
    }

    #[test]
    fn arguments_after_a_double_dash_still_count() {
        // clap would not treat `db` after `--` as a subcommand and would fail
        // instead, so refusing here costs nothing and keeps the check simple
        // and fail-closed.
        assert_eq!(
            refused("-- db reset", "production"),
            Some(DestructiveCommand::DbReset)
        );
    }

    #[test]
    fn allows_the_commands_outside_production() {
        for ambient in ["development", "test", "e2e", "staging", "Production", ""] {
            for line in ["db reset", "db truncate", "db seed --reset"] {
                assert_eq!(check(&args(line), ambient), Ok(()), "{ambient}: {line}");
            }
        }
    }

    #[test]
    fn the_environment_option_wins_over_the_ambient_environment() {
        let source =
            |line: &str, ambient: &str| check(&args(line), ambient).err().map(|r| r.source);
        assert_eq!(
            source("-e production db reset", "development"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("--environment production db reset", "test"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("--environment=production db reset", "test"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("-eproduction db reset", "test"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("-e=production db reset", "test"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("db reset -e production", "test"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("db -e production reset", "test"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("db seed -re production", "test"),
            Some(EnvironmentSource::Option)
        );
        assert_eq!(
            source("db reset", "production"),
            Some(EnvironmentSource::Ambient)
        );
        // An explicit non-production environment overrides an ambient production.
        assert_eq!(source("-e development db reset", "production"), None);
        assert_eq!(source("db reset --environment test", "production"), None);
    }

    #[test]
    fn repeated_environment_options_refuse_if_any_is_production() {
        assert!(check(&args("-e test -e production db reset"), "test").is_err());
        assert!(check(&args("-e production -e test db reset"), "test").is_err());
    }

    #[test]
    fn option_values_are_not_mistaken_for_subcommands() {
        // `production` is the value of -e here, so the subcommand is `db reset`.
        assert_eq!(
            refused("-e production db reset", "test"),
            Some(DestructiveCommand::DbReset)
        );
        // `db` is the value of -e, so the positionals are `reset` only.
        assert_eq!(check(&args("-e db reset"), "production"), Ok(()));
    }

    #[test]
    fn the_message_says_what_was_refused_and_what_to_do() {
        let message = check(&args("db reset"), "production")
            .expect_err("refused")
            .to_string();
        assert!(
            message.starts_with("Refusing to run `db reset`"),
            "{message}"
        );
        assert!(message.contains("from LOCO_ENV"), "{message}");
        assert!(message.contains("use `db down`"), "{message}");
        let message = check(&args("-e production db seed -r"), "test")
            .expect_err("refused")
            .to_string();
        assert!(
            message.starts_with("Refusing to run `db seed --reset`"),
            "{message}"
        );
        assert!(message.contains("from --environment"), "{message}");
    }

    #[test]
    fn command_names() {
        assert_eq!(DestructiveCommand::DbReset.as_str(), "db reset");
        assert_eq!(DestructiveCommand::DbTruncate.as_str(), "db truncate");
        assert_eq!(DestructiveCommand::DbSeedReset.as_str(), "db seed --reset");
    }

    /// Arguments that can never form a refused command or set the
    /// environment: no `db`, no option starting with `-e` or `--environment`.
    fn harmless_arg() -> impl Strategy<Value = String> {
        "[a-z0-9_.:/-]{0,12}".prop_filter("no db and no -e", |arg| {
            arg != "db" && !arg.starts_with("-e") && !arg.starts_with("--env") && arg != "--"
        })
    }

    fn environment_option() -> impl Strategy<Value = Vec<String>> {
        prop_oneof![
            Just(vec!["-e".to_owned(), "production".to_owned()]),
            Just(vec!["--environment".to_owned(), "production".to_owned()]),
            Just(vec!["--environment=production".to_owned()]),
            Just(vec!["-eproduction".to_owned()]),
        ]
    }

    proptest! {
        #[test]
        fn never_panics(args in proptest::collection::vec(any::<String>(), 0..8), ambient in any::<String>()) {
            let _ = check(&args, &ambient);
        }

        #[test]
        fn nothing_is_refused_outside_production(
            args in proptest::collection::vec(any::<String>(), 0..8),
            ambient in "[a-z]{0,12}",
        ) {
            prop_assume!(ambient != PRODUCTION);
            prop_assume!(!args.iter().any(|arg| arg.contains(PRODUCTION)));
            prop_assert_eq!(check(&args, &ambient), Ok(()));
        }

        #[test]
        fn commands_without_db_are_never_refused(
            args in proptest::collection::vec(harmless_arg(), 0..8),
        ) {
            prop_assert_eq!(check(&args, PRODUCTION), Ok(()));
        }

        #[test]
        fn the_environment_option_is_found_anywhere(
            command in prop_oneof![
                Just(vec!["db", "reset"]),
                Just(vec!["db", "truncate"]),
                Just(vec!["db", "seed", "--reset"]),
            ],
            option in environment_option(),
            position in 0usize..4,
        ) {
            let mut args: Vec<String> = command.iter().map(|part| (*part).to_owned()).collect();
            let position = position.min(args.len());
            for (offset, part) in option.into_iter().enumerate() {
                args.insert(position + offset, part);
            }
            let refusal = check(&args, "development");
            prop_assert!(refusal.is_err(), "{args:?}");
            prop_assert_eq!(refusal.unwrap_err().source, EnvironmentSource::Option);
        }
    }
}
