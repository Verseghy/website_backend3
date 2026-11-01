//! The arguments of the `import_mysql` task (docs/design/data-migration.md
//! §4.1), parsed and validated into [`ImportOptions`].
//!
//! Loco hands a task its `key:value` arguments as a map, split at the first
//! `:` of each argument (loco-rs 1.2.0 `src/cli.rs`, `parse_key_val`), so
//! values such as RFC 3339 instants may hold colons. It ignores keys that a
//! task does not read and keeps only the last of two arguments with the same
//! key. [`ImportOptions::from_arguments`] refuses every key it does not know,
//! so a misspelt optional argument cannot silently fall back to its default.
//!
//! Every refusal is an [`ArgumentError`] whose message names the argument and
//! says what to write instead. The checks here need neither the clock nor the
//! file system: the task passes "now" in, and checks the paths themselves
//! (that the resolution file exists, that `storage_root` is a directory and
//! that `report` is an empty one) before it calls
//! [`check_report_outside_storage_root`] on their canonical forms.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use chrono::{DateTime, SecondsFormat, Utc};

/// The argument that chooses between a dry run and a committing run.
pub const MODE: &str = "mode";
/// The argument that gives the moment the source was frozen or dumped.
pub const SNAPSHOT_AT: &str = "snapshot_at";
/// The argument that gives the path of the editor-signed resolution file.
pub const RESOLUTIONS: &str = "resolutions";
/// The argument that gives the directory served as `/storage`.
pub const STORAGE_ROOT: &str = "storage_root";
/// The argument that gives the empty directory for the run report.
pub const REPORT: &str = "report";
/// The argument that gives the cut-off of the Budapest wall-time canteen
/// audit values (data-migration.md §5.2).
pub const CANTEEN_AUDIT_LOCAL_BEFORE: &str = "canteen_audit_local_before";

/// Every argument the task takes, in the order of data-migration.md §4.1.
pub const ARGUMENTS: [&str; 6] = [
    MODE,
    SNAPSHOT_AT,
    RESOLUTIONS,
    STORAGE_ROOT,
    REPORT,
    CANTEEN_AUDIT_LOCAL_BEFORE,
];

/// Where the backend3 image keeps the repository's
/// `data/import/resolutions.toml`, the default of [`RESOLUTIONS`]
/// (research: DECISIONS R9, D12).
pub const DEFAULT_RESOLUTIONS: &str = "/app/data/import/resolutions.toml";

/// The default of [`CANTEEN_AUDIT_LOCAL_BEFORE`]: the end of the backend2
/// fix window, after which Laravel wrote the canteen audit values in UTC
/// (data-migration.md §5.2 and §13 item 1; research: DECISIONS R9, D56).
pub const DEFAULT_CANTEEN_AUDIT_LOCAL_BEFORE: &str = "2022-01-21T00:00:00Z";

/// The value of [`CANTEEN_AUDIT_LOCAL_BEFORE`] that reads every canteen audit
/// value as Budapest wall time.
pub const ALL: &str = "all";

/// What the run does after it has computed, written and verified everything
/// (data-migration.md §4.1, §11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// `dry-run`, the default: roll back, and write no file to
    /// `storage_root`.
    DryRun,
    /// `apply`: commit.
    Apply,
}

impl Mode {
    /// The argument value of the mode, as the operator writes it after
    /// `mode:`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DryRun => "dry-run",
            Self::Apply => "apply",
        }
    }

    /// The mode whose argument value is exactly `value`, or `None`.
    ///
    /// - `value`: what followed `mode:`. Case and spaces count: `Apply` and
    ///   ` apply` are refused, so a `CronJob`'s arguments mean exactly what
    ///   they say.
    #[must_use]
    pub fn from_argument(value: &str) -> Option<Self> {
        [Self::DryRun, Self::Apply]
            .into_iter()
            .find(|mode| mode.as_str() == value)
    }
}

/// Which `canteen_data` audit values the import reads as Europe/Budapest
/// wall time; the others are UTC (data-migration.md §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanteenAuditLocalBefore {
    /// Values before this instant are Budapest wall time, values at or after
    /// it are UTC.
    Before(DateTime<Utc>),
    /// `all`: every value is Budapest wall time.
    All,
}

impl CanteenAuditLocalBefore {
    /// The default, [`DEFAULT_CANTEEN_AUDIT_LOCAL_BEFORE`].
    ///
    /// # Panics
    ///
    /// Never: the constant is a valid instant, which a unit test pins.
    #[must_use]
    pub fn default_cutoff() -> Self {
        Self::Before(
            DateTime::parse_from_rfc3339(DEFAULT_CANTEEN_AUDIT_LOCAL_BEFORE)
                .expect("the default cut-off is an RFC 3339 instant")
                .with_timezone(&Utc),
        )
    }

    /// The argument value that selects this cut-off: [`ALL`], or the instant
    /// in the form of [`format_instant`].
    #[must_use]
    pub fn to_argument(self) -> String {
        match self {
            Self::Before(instant) => format_instant(instant),
            Self::All => ALL.to_owned(),
        }
    }
}

/// The validated arguments of one run, which the task passes down to the
/// import (`run(&source, &target, &ImportOptions)`, ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    /// Dry run or commit.
    pub mode: Mode,
    /// The moment the source was frozen or dumped: the `created_at` and
    /// `updated_at` of rows whose legacy table has no timestamps
    /// (data-migration.md §5.2). Never later than the start of the run.
    pub snapshot_at: DateTime<Utc>,
    /// The editor-signed resolution file (data-migration.md §8.2).
    pub resolutions: PathBuf,
    /// The directory served as `/storage`, the uploads volume's public root.
    pub storage_root: PathBuf,
    /// The empty directory that receives the run report (§11.3).
    pub report: PathBuf,
    /// Which canteen audit values are Budapest wall time (§5.2).
    pub canteen_audit_local_before: CanteenAuditLocalBefore,
}

impl ImportOptions {
    /// Parses and validates the task's arguments.
    ///
    /// - `arguments`: the `key:value` arguments as Loco hands them over
    ///   (`Vars::cli`). Loco has already kept only the last of two equal
    ///   keys, so a repeated argument cannot be detected here.
    /// - `now`: the start of the run, from the app clock; `snapshot_at` must
    ///   not be later.
    ///
    /// `mode` defaults to `dry-run`, `resolutions` to
    /// [`DEFAULT_RESOLUTIONS`] and `canteen_audit_local_before` to
    /// [`DEFAULT_CANTEEN_AUDIT_LOCAL_BEFORE`]; `snapshot_at`, `storage_root`
    /// and `report` are required. A relative path is kept as given and
    /// resolved against the task's working directory.
    ///
    /// # Errors
    ///
    /// The first problem found, as an [`ArgumentError`] that names the
    /// argument: an unknown key (checked first, in key order), then each
    /// argument in the order of [`ARGUMENTS`].
    pub fn from_arguments(
        arguments: &BTreeMap<String, String>,
        now: DateTime<Utc>,
    ) -> Result<Self, ArgumentError> {
        if let Some(name) = arguments
            .keys()
            .find(|name| !ARGUMENTS.contains(&name.as_str()))
        {
            return Err(ArgumentError::Unknown { name: name.clone() });
        }
        let given = |name: &'static str| -> Result<Option<&str>, ArgumentError> {
            match arguments.get(name).map(String::as_str) {
                Some("") => Err(ArgumentError::Empty { name }),
                other => Ok(other),
            }
        };
        let required = |name: &'static str, what: &'static str| {
            given(name)?.ok_or(ArgumentError::Missing { name, what })
        };

        let mode = match given(MODE)? {
            None => Mode::DryRun,
            Some(value) => Mode::from_argument(value).ok_or_else(|| ArgumentError::Mode {
                value: value.to_owned(),
            })?,
        };
        let snapshot_at = parse_snapshot_at(
            required(
                SNAPSHOT_AT,
                "give the moment the source was frozen or dumped, in RFC 3339 and UTC, for \
                 example snapshot_at:2026-10-23T17:01:02Z",
            )?,
            now,
        )?;
        let resolutions = PathBuf::from(given(RESOLUTIONS)?.unwrap_or(DEFAULT_RESOLUTIONS));
        let storage_root = PathBuf::from(required(
            STORAGE_ROOT,
            "give the directory that is served as /storage, the uploads volume's public root, \
             as in storage_root:<directory>",
        )?);
        let report = PathBuf::from(required(
            REPORT,
            "give an empty directory for the run report, as in report:<empty directory>",
        )?);
        let canteen_audit_local_before = match given(CANTEEN_AUDIT_LOCAL_BEFORE)? {
            None => CanteenAuditLocalBefore::default_cutoff(),
            Some(value) => parse_canteen_audit_local_before(value)?,
        };
        Ok(Self {
            mode,
            snapshot_at,
            resolutions,
            storage_root,
            report,
            canteen_audit_local_before,
        })
    }

    /// The arguments that select exactly these options, every one of them
    /// spelt out, defaults included: [`ImportOptions::from_arguments`] reads
    /// them back to the same value. The run report records them, so a run
    /// can be repeated as it was.
    #[must_use]
    pub fn to_arguments(&self) -> BTreeMap<String, String> {
        let path = |path: &Path| path.to_string_lossy().into_owned();
        BTreeMap::from([
            (MODE.to_owned(), self.mode.as_str().to_owned()),
            (SNAPSHOT_AT.to_owned(), format_instant(self.snapshot_at)),
            (RESOLUTIONS.to_owned(), path(&self.resolutions)),
            (STORAGE_ROOT.to_owned(), path(&self.storage_root)),
            (REPORT.to_owned(), path(&self.report)),
            (
                CANTEEN_AUDIT_LOCAL_BEFORE.to_owned(),
                self.canteen_audit_local_before.to_argument(),
            ),
        ])
    }
}

/// Parses `snapshot_at`: an RFC 3339 instant in UTC (`Z` or `+00:00`), not
/// later than `now`.
fn parse_snapshot_at(value: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, ArgumentError> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|problem| ArgumentError::Instant {
        name: SNAPSHOT_AT,
        value: value.to_owned(),
        problem: problem.to_string(),
    })?;
    let instant = parsed.with_timezone(&Utc);
    // data-migration.md §4.1 asks for UTC. Another offset names the same
    // instant, but it is the usual sign that a local wall time was copied
    // from the dump's file name, so the message shows the UTC form to check.
    if parsed.offset().local_minus_utc() != 0 {
        return Err(ArgumentError::SnapshotNotUtc {
            value: value.to_owned(),
            utc: format_instant(instant),
        });
    }
    if instant > now {
        return Err(ArgumentError::SnapshotInFuture {
            snapshot_at: format_instant(instant),
            now: format_instant(now),
        });
    }
    Ok(instant)
}

/// Parses `canteen_audit_local_before`: [`ALL`], or an RFC 3339 instant with
/// any offset (an instant, unlike `snapshot_at`'s dump time, is often
/// written as a Budapest midnight).
fn parse_canteen_audit_local_before(value: &str) -> Result<CanteenAuditLocalBefore, ArgumentError> {
    if value == ALL {
        return Ok(CanteenAuditLocalBefore::All);
    }
    DateTime::parse_from_rfc3339(value)
        .map(|instant| CanteenAuditLocalBefore::Before(instant.with_timezone(&Utc)))
        .map_err(|problem| ArgumentError::CanteenAuditLocalBefore {
            value: value.to_owned(),
            problem: problem.to_string(),
        })
}

/// Refuses a report directory inside `storage_root`.
///
/// - `report`: the canonical path of the report directory (absolute, with
///   every symbolic link resolved, as `std::fs::canonicalize` returns it).
/// - `storage_root`: the canonical path of `storage_root`.
///
/// The media server publishes everything under `storage_root` at
/// `/storage`, and the report holds draft content and account counts, which
/// stay private (data-migration.md §11.3). The comparison is by path
/// component, so a sibling such as `/storage-reports` next to `/storage` is
/// accepted.
///
/// # Errors
///
/// [`ArgumentError::ReportInsideStorageRoot`] when `report` is
/// `storage_root` or lies below it.
pub fn check_report_outside_storage_root(
    report: &Path,
    storage_root: &Path,
) -> Result<(), ArgumentError> {
    if report.starts_with(storage_root) {
        return Err(ArgumentError::ReportInsideStorageRoot {
            report: report.to_string_lossy().into_owned(),
            storage_root: storage_root.to_string_lossy().into_owned(),
        });
    }
    Ok(())
}

/// Writes an instant the way the task's arguments and its report do: RFC
/// 3339 in UTC with `Z`, with as many fractional digits as it needs (none,
/// 3, 6 or 9), so that [`ImportOptions::from_arguments`] reads it back
/// exactly.
///
/// - `instant`: the instant to write.
#[must_use]
pub fn format_instant(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// A refused argument. Each message names the argument and says what to
/// write instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgumentError {
    /// A key the task does not take. Only the key is shown, never the value.
    #[error(
        "unknown argument {name:?}: import_mysql takes mode, snapshot_at, resolutions, \
         storage_root, report and canteen_audit_local_before (data-migration.md §4.1); the \
         source database is not an argument, it comes from IMPORT_SOURCE_DATABASE_URL \
         (settings.import.source_database_url)"
    )]
    Unknown {
        /// The unknown key.
        name: String,
    },
    /// A required argument is not there.
    #[error("the argument {name} is missing: {what} (data-migration.md §4.1)")]
    Missing {
        /// The argument.
        name: &'static str,
        /// What to give.
        what: &'static str,
    },
    /// An argument has nothing after its colon.
    #[error("the argument {name} is empty: write its value after the colon, or leave it out")]
    Empty {
        /// The argument.
        name: &'static str,
    },
    /// `mode` is neither `dry-run` nor `apply`.
    #[error(
        "the argument mode is {value:?}: write mode:dry-run (the default: the whole import, then \
         a rollback) or mode:apply (the whole import, then a commit)"
    )]
    Mode {
        /// The refused value.
        value: String,
    },
    /// An instant that is not RFC 3339.
    #[error(
        "the argument {name} is {value:?}, which is not an RFC 3339 instant ({problem}): write \
         it like 2026-10-23T17:01:02Z"
    )]
    Instant {
        /// The argument.
        name: &'static str,
        /// The refused value.
        value: String,
        /// chrono's reason.
        problem: String,
    },
    /// `snapshot_at` has an offset other than UTC.
    #[error(
        "the argument snapshot_at is {value:?}, which is not in UTC: write the instant in UTC, \
         here snapshot_at:{utc}, after checking that it is the moment of the dump \
         (data-migration.md §4.1)"
    )]
    SnapshotNotUtc {
        /// The refused value.
        value: String,
        /// The same instant in UTC.
        utc: String,
    },
    /// `snapshot_at` is later than the start of the run.
    #[error(
        "the argument snapshot_at is {snapshot_at}, later than the start of this run at {now}: \
         give the moment the source was frozen or dumped, which has passed"
    )]
    SnapshotInFuture {
        /// The refused instant.
        snapshot_at: String,
        /// The start of the run.
        now: String,
    },
    /// `canteen_audit_local_before` is neither `all` nor an instant.
    #[error(
        "the argument canteen_audit_local_before is {value:?}, which is neither all nor an RFC \
         3339 instant ({problem}): write all, or an instant like 2022-01-21T00:00:00Z, the \
         default (data-migration.md §5.2)"
    )]
    CanteenAuditLocalBefore {
        /// The refused value.
        value: String,
        /// chrono's reason.
        problem: String,
    },
    /// The report directory is `storage_root` or lies inside it.
    #[error(
        "the argument report is {report}, inside storage_root {storage_root}, which the media \
         server publishes at /storage: the report holds draft content and account counts and \
         stays private (data-migration.md §11.3), so give a directory outside storage_root"
    )]
    ReportInsideStorageRoot {
        /// The report directory, canonical.
        report: String,
        /// `storage_root`, canonical.
        storage_root: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The start of the runs in these tests.
    fn now() -> DateTime<Utc> {
        instant("2026-10-31T18:00:00Z")
    }

    fn instant(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .expect("RFC 3339")
            .with_timezone(&Utc)
    }

    /// The arguments `pairs`, as Loco hands them over.
    fn arguments(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// The three required arguments, with valid values.
    fn required() -> Vec<(&'static str, &'static str)> {
        vec![
            (SNAPSHOT_AT, "2026-10-23T17:01:02Z"),
            (STORAGE_ROOT, "/storage"),
            (REPORT, "/reports/run-1"),
        ]
    }

    /// The required arguments with `name` replaced by `value`, or removed
    /// for `None`, and `extra` added.
    fn with(
        name: &str,
        value: Option<&str>,
        extra: &[(&'static str, &'static str)],
    ) -> BTreeMap<String, String> {
        let mut pairs: Vec<(&str, &str)> = required()
            .into_iter()
            .filter(|(key, _)| *key != name)
            .collect();
        if let Some(value) = value {
            pairs.push((name, value));
        }
        pairs.extend_from_slice(extra);
        arguments(&pairs)
    }

    fn refused(arguments: &BTreeMap<String, String>) -> ArgumentError {
        ImportOptions::from_arguments(arguments, now()).expect_err("refused")
    }

    #[test]
    fn the_required_arguments_alone_give_the_documented_defaults() {
        let options = ImportOptions::from_arguments(&arguments(&required()), now()).expect("valid");
        assert_eq!(
            options,
            ImportOptions {
                mode: Mode::DryRun,
                snapshot_at: instant("2026-10-23T17:01:02Z"),
                resolutions: PathBuf::from("/app/data/import/resolutions.toml"),
                storage_root: PathBuf::from("/storage"),
                report: PathBuf::from("/reports/run-1"),
                canteen_audit_local_before: CanteenAuditLocalBefore::Before(instant(
                    "2022-01-21T00:00:00Z"
                )),
            }
        );
    }

    #[test]
    fn every_argument_is_read() {
        let pairs = [
            (MODE, "apply"),
            (SNAPSHOT_AT, "2026-10-23T17:01:02.5Z"),
            (RESOLUTIONS, "data/import/resolutions.toml"),
            (STORAGE_ROOT, "/srv/storage"),
            (REPORT, "reports/2026-10-23"),
            (CANTEEN_AUDIT_LOCAL_BEFORE, "all"),
        ];
        let options = ImportOptions::from_arguments(&arguments(&pairs), now()).expect("valid");
        assert_eq!(options.mode, Mode::Apply);
        assert_eq!(options.snapshot_at, instant("2026-10-23T17:01:02.500Z"));
        assert_eq!(
            options.resolutions,
            PathBuf::from("data/import/resolutions.toml")
        );
        assert_eq!(options.storage_root, PathBuf::from("/srv/storage"));
        assert_eq!(options.report, PathBuf::from("reports/2026-10-23"));
        assert_eq!(
            options.canteen_audit_local_before,
            CanteenAuditLocalBefore::All
        );
    }

    #[test]
    fn both_modes_are_read_and_nothing_else() {
        assert_eq!(Mode::from_argument("dry-run"), Some(Mode::DryRun));
        assert_eq!(Mode::from_argument("apply"), Some(Mode::Apply));
        for value in [
            "dry_run", "dryrun", "Apply", " apply", "apply ", "commit", "wet-run",
        ] {
            assert_eq!(Mode::from_argument(value), None, "{value:?}");
            let error = refused(&with(MODE, Some(value), &[]));
            assert_eq!(
                error,
                ArgumentError::Mode {
                    value: value.to_owned()
                }
            );
            let message = error.to_string();
            assert!(
                message.starts_with(&format!("the argument mode is {value:?}: ")),
                "{message}"
            );
            assert!(
                message.contains("mode:dry-run") && message.contains("mode:apply"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_missing_required_argument_is_named_with_an_example() {
        for (name, example) in [
            (SNAPSHOT_AT, "snapshot_at:2026-10-23T17:01:02Z"),
            (STORAGE_ROOT, "storage_root:<directory>"),
            (REPORT, "report:<empty directory>"),
        ] {
            let error = refused(&with(name, None, &[]));
            assert!(
                matches!(error, ArgumentError::Missing { name: missing, .. } if missing == name),
                "{error:?}"
            );
            let message = error.to_string();
            assert!(
                message.starts_with(&format!("the argument {name} is missing: ")),
                "{message}"
            );
            assert!(message.contains(example), "{message}");
        }
    }

    #[test]
    fn an_empty_value_is_refused_for_every_argument() {
        for name in ARGUMENTS {
            let error = refused(&with(name, Some(""), &[]));
            assert_eq!(error, ArgumentError::Empty { name });
            assert!(
                error
                    .to_string()
                    .starts_with(&format!("the argument {name} is empty")),
                "{error}"
            );
        }
    }

    #[test]
    fn an_unknown_argument_is_named_without_its_value() {
        let secret = "mysql://import:pw-not-for-output@legacy/verseghy";
        for name in [
            "snapshot",
            "Mode",
            "source_database_url",
            "dry-run",
            "storage-root",
        ] {
            let mut arguments = arguments(&required());
            arguments.insert(name.to_owned(), secret.to_owned());
            let error = refused(&arguments);
            assert_eq!(
                error,
                ArgumentError::Unknown {
                    name: name.to_owned()
                }
            );
            let message = error.to_string();
            assert!(
                message.starts_with(&format!("unknown argument {name:?}: ")),
                "{message}"
            );
            assert!(message.contains("IMPORT_SOURCE_DATABASE_URL"), "{message}");
            assert!(!message.contains("pw-not-for-output"), "{message}");
            for argument in ARGUMENTS {
                assert!(message.contains(argument), "{message}");
            }
        }
    }

    #[test]
    fn unknown_keys_are_reported_before_any_other_problem() {
        // snapshot_at is missing as well; the first unknown key in key order
        // is named.
        let error = refused(&arguments(&[("zeta", "1"), ("alpha", "2")]));
        assert_eq!(
            error,
            ArgumentError::Unknown {
                name: "alpha".to_owned()
            }
        );
    }

    #[test]
    fn snapshot_at_must_be_an_rfc_3339_instant() {
        for value in [
            "2026-10-23",
            "2026-10-23T17:01:02",
            "2026-10-23 17:01",
            "17:01:02Z",
            "2026-13-01T00:00:00Z",
            "yesterday",
        ] {
            let error = refused(&with(SNAPSHOT_AT, Some(value), &[]));
            assert!(
                matches!(&error, ArgumentError::Instant { name: SNAPSHOT_AT, value: given, .. } if given == value),
                "{value}: {error:?}"
            );
            let message = error.to_string();
            assert!(
                message.starts_with(&format!(
                    "the argument snapshot_at is {value:?}, which is not an RFC 3339 instant ("
                )),
                "{message}"
            );
            assert!(
                message.ends_with("write it like 2026-10-23T17:01:02Z"),
                "{message}"
            );
        }
    }

    #[test]
    fn snapshot_at_must_be_in_utc_and_the_message_gives_the_utc_form() {
        for (value, utc) in [
            ("2026-10-23T19:01:02+02:00", "2026-10-23T17:01:02Z"),
            ("2026-10-23T12:01:02.25-05:00", "2026-10-23T17:01:02.250Z"),
        ] {
            let error = refused(&with(SNAPSHOT_AT, Some(value), &[]));
            assert_eq!(
                error,
                ArgumentError::SnapshotNotUtc {
                    value: value.to_owned(),
                    utc: utc.to_owned()
                }
            );
            assert!(
                error
                    .to_string()
                    .contains(&format!("here snapshot_at:{utc}")),
                "{error}"
            );
        }
        for value in ["2026-10-23T17:01:02+00:00", "2026-10-23t17:01:02z"] {
            let options =
                ImportOptions::from_arguments(&with(SNAPSHOT_AT, Some(value), &[]), now())
                    .expect(value);
            assert_eq!(
                options.snapshot_at,
                instant("2026-10-23T17:01:02Z"),
                "{value}"
            );
        }
    }

    #[test]
    fn snapshot_at_must_not_be_later_than_the_start_of_the_run() {
        let at_start = ImportOptions::from_arguments(
            &with(SNAPSHOT_AT, Some("2026-10-31T18:00:00Z"), &[]),
            now(),
        )
        .expect("the start itself is fine");
        assert_eq!(at_start.snapshot_at, now());
        let error = refused(&with(SNAPSHOT_AT, Some("2026-10-31T18:00:00.001Z"), &[]));
        assert_eq!(
            error,
            ArgumentError::SnapshotInFuture {
                snapshot_at: "2026-10-31T18:00:00.001Z".to_owned(),
                now: "2026-10-31T18:00:00Z".to_owned(),
            }
        );
        assert!(
            error.to_string().starts_with(
                "the argument snapshot_at is 2026-10-31T18:00:00.001Z, later than the start of \
                 this run at 2026-10-31T18:00:00Z: "
            ),
            "{error}"
        );
    }

    #[test]
    fn the_canteen_cut_off_is_all_or_an_instant_with_any_offset() {
        for (value, expected) in [
            ("all", CanteenAuditLocalBefore::All),
            (
                "2022-01-21T00:00:00+01:00",
                CanteenAuditLocalBefore::Before(instant("2022-01-20T23:00:00Z")),
            ),
            (
                "2022-01-21T00:00:00Z",
                CanteenAuditLocalBefore::default_cutoff(),
            ),
        ] {
            let options = ImportOptions::from_arguments(
                &with(CANTEEN_AUDIT_LOCAL_BEFORE, Some(value), &[]),
                now(),
            )
            .expect(value);
            assert_eq!(options.canteen_audit_local_before, expected, "{value}");
        }
        for value in ["ALL", "none", "2022-01-21", "2022-01-21T00:00:00"] {
            let error = refused(&with(CANTEEN_AUDIT_LOCAL_BEFORE, Some(value), &[]));
            assert!(
                matches!(&error, ArgumentError::CanteenAuditLocalBefore { value: given, .. } if given == value),
                "{value}: {error:?}"
            );
            let message = error.to_string();
            assert!(
                message.starts_with(&format!(
                    "the argument canteen_audit_local_before is {value:?}, which is neither all nor"
                )),
                "{message}"
            );
            assert!(
                message.contains("2022-01-21T00:00:00Z, the default"),
                "{message}"
            );
        }
    }

    #[test]
    fn the_default_cut_off_is_the_documented_instant() {
        assert_eq!(
            CanteenAuditLocalBefore::default_cutoff(),
            CanteenAuditLocalBefore::Before(instant("2022-01-21T00:00:00Z"))
        );
        assert_eq!(
            CanteenAuditLocalBefore::default_cutoff().to_argument(),
            DEFAULT_CANTEEN_AUDIT_LOCAL_BEFORE
        );
        assert_eq!(CanteenAuditLocalBefore::All.to_argument(), "all");
    }

    #[test]
    fn to_arguments_spells_out_every_argument() {
        let options = ImportOptions::from_arguments(&arguments(&required()), now()).expect("valid");
        assert_eq!(
            options.to_arguments(),
            arguments(&[
                (MODE, "dry-run"),
                (SNAPSHOT_AT, "2026-10-23T17:01:02Z"),
                (RESOLUTIONS, "/app/data/import/resolutions.toml"),
                (STORAGE_ROOT, "/storage"),
                (REPORT, "/reports/run-1"),
                (CANTEEN_AUDIT_LOCAL_BEFORE, "2022-01-21T00:00:00Z"),
            ])
        );
    }

    #[test]
    fn a_report_inside_storage_root_is_refused() {
        for (report, storage_root) in [
            ("/storage", "/storage"),
            ("/storage/reports", "/storage"),
            ("/srv/storage/uploads/x", "/srv/storage"),
        ] {
            let error =
                check_report_outside_storage_root(Path::new(report), Path::new(storage_root))
                    .expect_err(report);
            assert_eq!(
                error,
                ArgumentError::ReportInsideStorageRoot {
                    report: report.to_owned(),
                    storage_root: storage_root.to_owned(),
                }
            );
            assert!(
                error.to_string().contains("publishes at /storage"),
                "{error}"
            );
        }
        for (report, storage_root) in [
            ("/reports", "/storage"),
            ("/storage-reports", "/storage"),
            ("/srv", "/srv/storage"),
        ] {
            assert_eq!(
                check_report_outside_storage_root(Path::new(report), Path::new(storage_root)),
                Ok(()),
                "{report}"
            );
        }
    }

    #[test]
    fn instants_are_written_in_utc_with_the_digits_they_need() {
        for (value, written) in [
            ("2026-10-23T17:01:02Z", "2026-10-23T17:01:02Z"),
            ("2026-10-23T19:01:02+02:00", "2026-10-23T17:01:02Z"),
            ("2026-10-23T17:01:02.5Z", "2026-10-23T17:01:02.500Z"),
            ("2026-10-23T17:01:02.000001Z", "2026-10-23T17:01:02.000001Z"),
            (
                "2026-10-23T17:01:02.123456789Z",
                "2026-10-23T17:01:02.123456789Z",
            ),
        ] {
            assert_eq!(format_instant(instant(value)), written, "{value}");
        }
    }

    /// Instants from 1970 to the end of year 9999, the years RFC 3339 can
    /// write, down to the nanosecond.
    fn any_rfc3339_instant() -> impl Strategy<Value = DateTime<Utc>> {
        (0_i64..253_402_300_800, 0_u32..1_000_000_000).prop_map(|(seconds, nanos)| {
            DateTime::from_timestamp(seconds, nanos).expect("inside chrono's range")
        })
    }

    /// Path arguments: anything non-empty without a NUL.
    const PATH: &str = "[^\u{0}]{1,40}";

    fn any_options() -> impl Strategy<Value = ImportOptions> {
        (
            prop_oneof![Just(Mode::DryRun), Just(Mode::Apply)],
            any_rfc3339_instant(),
            PATH,
            PATH,
            PATH,
            prop_oneof![
                Just(CanteenAuditLocalBefore::All),
                any_rfc3339_instant().prop_map(CanteenAuditLocalBefore::Before),
            ],
        )
            .prop_map(
                |(mode, snapshot_at, resolutions, storage_root, report, cutoff)| ImportOptions {
                    mode,
                    snapshot_at,
                    resolutions: PathBuf::from(resolutions),
                    storage_root: PathBuf::from(storage_root),
                    report: PathBuf::from(report),
                    canteen_audit_local_before: cutoff,
                },
            )
    }

    proptest! {
        #[test]
        fn options_round_trip_through_their_arguments(options in any_options()) {
            // The latest instant the strategy makes is still not later than
            // this start, so snapshot_at always passes its check.
            let start = instant("9999-12-31T23:59:59.999999999Z");
            let read = ImportOptions::from_arguments(&options.to_arguments(), start);
            prop_assert_eq!(read, Ok(options));
        }

        #[test]
        fn a_snapshot_after_the_start_is_always_refused(
            start in any_rfc3339_instant(),
            later_by in 1_i64..1_000_000_000_000,
        ) {
            let snapshot_at = start + chrono::TimeDelta::nanoseconds(later_by);
            prop_assume!(snapshot_at.timestamp() < 253_402_300_800);
            let value = format_instant(snapshot_at);
            let mut arguments = arguments(&required());
            arguments.insert(SNAPSHOT_AT.to_owned(), value);
            let refused = ImportOptions::from_arguments(&arguments, start);
            let is_in_future = matches!(refused, Err(ArgumentError::SnapshotInFuture { .. }));
            prop_assert!(is_in_future, "{:?}", refused);
        }

        #[test]
        fn any_unknown_key_is_refused_by_name(name in "[a-z_-]{1,24}", value in ".{0,40}") {
            prop_assume!(!ARGUMENTS.contains(&name.as_str()));
            let mut arguments = arguments(&required());
            arguments.insert(name.clone(), value);
            prop_assert_eq!(
                ImportOptions::from_arguments(&arguments, now()),
                Err(ArgumentError::Unknown { name })
            );
        }

        #[test]
        fn a_mode_is_read_exactly_when_it_is_one_of_the_two(value in ".{0,12}") {
            let expected = match value.as_str() {
                "dry-run" => Some(Mode::DryRun),
                "apply" => Some(Mode::Apply),
                _ => None,
            };
            prop_assert_eq!(Mode::from_argument(&value), expected);
        }

        #[test]
        fn parsing_never_panics(
            pairs in proptest::collection::btree_map(
                prop_oneof![Just(MODE.to_owned()), Just(SNAPSHOT_AT.to_owned()), Just(CANTEEN_AUDIT_LOCAL_BEFORE.to_owned()), ".{0,8}"],
                ".{0,32}",
                0..6,
            ),
        ) {
            let _ = ImportOptions::from_arguments(&pairs, now());
        }

        #[test]
        fn a_path_below_storage_root_is_refused_and_a_sibling_is_not(
            base in proptest::collection::vec("[a-z0-9]{1,8}", 1..4),
            below in proptest::collection::vec("[a-z0-9]{1,8}", 0..4),
            suffix in "[a-z0-9-]{1,8}",
        ) {
            let storage_root: PathBuf = std::iter::once("/".to_owned()).chain(base).collect();
            let inside = below.iter().fold(storage_root.clone(), |path, part| path.join(part));
            prop_assert!(check_report_outside_storage_root(&inside, &storage_root).is_err());
            // A sibling whose name only starts with storage_root's last part.
            let mut sibling = storage_root.clone().into_os_string();
            sibling.push(&suffix);
            prop_assert_eq!(
                check_report_outside_storage_root(Path::new(&sibling), &storage_root),
                Ok(())
            );
        }
    }
}
