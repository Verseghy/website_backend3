//! The run report of the `import_mysql` task (docs/design/data-migration.md
//! §11.3): `summary.json`, the machine-readable part that the nightly
//! rehearsal saves (cutover.md §5.2), and `report.md`, the same run for
//! people to read.
//!
//! This module renders both from a [`RunSummary`]; the task writes them into
//! the run's `report` directory. The summary records the run's start, end
//! and duration (ADR 0003: the report carries the run time). Both instants
//! are readings of the app clock that the task took and passed in; nothing
//! here reads a clock (ADR 0008 rule 1, ADR 0010 point 7). The duration is
//! their difference, so it is consistent with them by construction, and it
//! is negative if the system clock was set back during the run.
//!
//! **What differs between two runs of the same dump.** ADR 0003 asks that
//! the same dump give the same rows and the same report. Rendering is
//! deterministic (equal summaries give equal bytes), but a few fields
//! describe the run rather than the data: `started_at`, `finished_at` and
//! `duration_ms` (in the Markdown: Started, Finished and Duration), and the
//! `report` argument, the directory of this one run, which a second run
//! cannot share. A comparison of two runs leaves exactly these out; every
//! other field is the same when the dump and the other arguments are.
//!
//! The report grows with the import: each step adds its part (defects, rule
//! hits, diffs, files, accounts, row counts and checks), and until a step
//! fills a section, the Markdown says so instead of leaving it blank, so an
//! empty section is never read as "nothing found". [`REPORT_FORMAT`] changes
//! only when a field of `summary.json` is renamed or removed; new fields are
//! added without changing it.
//!
//! The first part is the source session's (data-migration.md §3.3): that the
//! schema fingerprint matched, the latest timestamp in the source, which
//! `snapshot_at` did not precede, and every table's `AUTO_INCREMENT`. A run
//! that stopped before the session ended has none of them, and the report
//! says so; its error says why.
//!
//! The second is the defect detectors' (data-migration.md §8, ADR 0047):
//! every instance of a defect whose detector ran, with its entry, its
//! precondition, the source value and the resolution the import proposes,
//! which a dry run applies, and every entry whose subject the rule does not
//! find ([`super::defects`]). The task also writes the editors' sign-off
//! sheet, [`SIGNOFF_FILE`], from the same part ([`RunSummary::to_signoff`],
//! [`super::signoff`]): the only extract of the report that leaves staging,
//! which holds ids, paths and proposed resolutions only.
//!
//! The report is private: it holds draft content and account counts, and
//! the Markdown starts by saying so (data-migration.md §11.3).

use std::{collections::BTreeMap, fmt::Write as _};

use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;

use super::{
    defects::{DefectOutcome, DefectRecord, DefectsPart, EntryRef, id_list},
    not_migrated::NOT_MIGRATED_TABLES,
    options::{ImportOptions, format_instant},
    signoff,
    source_session::SourceFacts,
    source_tables::SOURCE_TABLES,
};

/// The version of the `summary.json` layout.
pub const REPORT_FORMAT: u32 = 1;

/// The file name of the machine-readable summary in the report directory.
pub const SUMMARY_FILE: &str = "summary.json";

/// The file name of the Markdown report in the report directory.
pub const MARKDOWN_FILE: &str = "report.md";

/// The task whose runs these reports describe.
pub const TASK_NAME: &str = "import_mysql";

/// What the text of a section says while no import step reports there.
pub const NO_STEP_YET: &str = "No import step of this build reports here yet.";

/// The file name of the editors' sign-off sheet in the report directory
/// (data-migration.md §8.2, [`super::signoff`]).
pub const SIGNOFF_FILE: &str = "signoff.md";

/// What the Defects section holds (data-migration.md §11.3), which the
/// defect detector fills ([`super::defects`]).
pub const DEFECTS_HOLDS: &str = "Every detected instance of D1-D10 and N7, with its entry, the \
                                 entry's precondition, the source value and the resolution the \
                                 import proposes (the entry's or the defect's default), which a \
                                 dry run applies, and every entry whose row the rule no longer \
                                 finds; ids, calendar days, instants, SHA-256 digests, index \
                                 image paths and fixed words only. signoff.md beside this report \
                                 is the editors' sign-off sheet (data-migration.md §8.2).";

/// The sections that the import steps fill, each with what it will hold
/// (data-migration.md §11.3), in the order the Markdown report shows them,
/// after the Defects section.
pub const STEP_SECTIONS: [(&str, &str); 4] = [
    (
        "Rule hits",
        "Per row, the hits of N1-N7 (N5b included), the images without alt, the rows exempted \
         from a save-time rule and the values over a validation limit.",
    ),
    (
        "Diffs",
        "For each changed body, a unified diff of the pretty-printed source and target.",
    ),
    (
        "Files",
        "Every extracted image: target path, size, SHA-256 and pixel size.",
    ),
    (
        "Accounts",
        "Counts only: users, roles per role, grants, users without a role and dropped orphans.",
    ),
];

/// How the run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every step ran and every check passed: committed in `apply` mode,
    /// rolled back in `dry-run` mode.
    Completed,
    /// A step or a check refused the import; everything was rolled back.
    Failed {
        /// The refusal, which names what to fix (data-migration.md §4.2).
        message: String,
    },
}

impl Outcome {
    /// The outcome's name in `summary.json` and the Markdown report.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
        }
    }
}

/// One run, as the report shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    /// The run's validated arguments.
    pub options: ImportOptions,
    /// The app clock's reading when the run started.
    pub started_at: DateTime<Utc>,
    /// The app clock's reading when the run ended, just before the report
    /// was written.
    pub finished_at: DateTime<Utc>,
    /// How it ended.
    pub outcome: Outcome,
    /// What the import found, part by part.
    pub parts: ReportParts,
}

/// What the import steps found, for the report. A run that failed has the
/// parts of the steps that ended before it failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportParts {
    /// What the source session established, or `None` when the run stopped
    /// before the session ended.
    pub source: Option<SourceFacts>,
    /// What the defect detectors found (data-migration.md §8, ADR 0047):
    /// every instance and every entry whose subject the rule does not find,
    /// which `report.md`, `summary.json` and the sign-off sheet show.
    pub defects: DefectsPart,
}

/// `summary.json`'s layout. The field order is the order of the file.
#[derive(Serialize)]
struct SummaryJson<'a> {
    report_format: u32,
    task: &'static str,
    outcome: &'static str,
    error: Option<&'a str>,
    started_at: String,
    finished_at: String,
    duration_ms: i64,
    arguments: BTreeMap<String, String>,
    source: Option<SourceJson>,
    defects: DefectsJson,
    not_migrated: Vec<NotMigratedJson>,
}

/// `summary.json`'s `defects` object: the defect detectors' part.
#[derive(Serialize)]
struct DefectsJson {
    /// Whether the run reached its end, so every detector of the build ran.
    complete: bool,
    /// The defects whose detector ran, in the order they first ran.
    checked: Vec<&'static str>,
    /// The instances that no entry covers where the defect needs one.
    uncovered: usize,
    /// The entries whose precondition does not hold.
    refused: usize,
    records: Vec<DefectRecordJson>,
}

/// One instance, or one entry whose subject the rule does not find.
#[derive(Serialize)]
struct DefectRecordJson {
    defect: &'static str,
    subject: String,
    outcome: &'static str,
    rows: Vec<u32>,
    paths: Vec<String>,
    /// The entry's name, `post_date_shift[2]`, or `null`.
    entry: Option<String>,
    /// The entry's precondition, or `null` without an entry.
    expected: Option<String>,
    /// The source value, or `null` when no row is the subject.
    found: Option<String>,
    /// The resolution the import proposes, which a dry run applies, or
    /// `null` where the source value is kept.
    proposed: Option<String>,
}

impl DefectsJson {
    fn new(part: &DefectsPart) -> Self {
        Self {
            complete: part.is_complete(),
            checked: part
                .checked()
                .iter()
                .map(|defect| defect.as_str())
                .collect(),
            uncovered: part.uncovered(),
            refused: part.refused(),
            records: part
                .records()
                .iter()
                .map(|record| DefectRecordJson {
                    defect: record.defect.as_str(),
                    subject: record.subject.to_string(),
                    outcome: record.outcome.as_str(),
                    rows: record.rows.iter().map(|row| row.get()).collect(),
                    paths: record.paths.iter().map(ToString::to_string).collect(),
                    entry: record.entry.as_ref().map(EntryRef::name),
                    expected: record
                        .entry
                        .as_ref()
                        .map(|entry| entry.expected.to_string()),
                    found: record.found.as_ref().map(ToString::to_string),
                    proposed: record.proposed.as_ref().map(ToString::to_string),
                })
                .collect(),
        }
    }
}

/// `summary.json`'s `source` object: the source session's part.
#[derive(Serialize)]
struct SourceJson {
    /// Always `matched`: a session that ends has checked the fingerprint.
    fingerprint: &'static str,
    latest_timestamp: Option<LatestTimestampJson>,
    /// Each allow-listed table's `AUTO_INCREMENT`, `null` for a table
    /// without one, by table name.
    auto_increment: BTreeMap<&'static str, Option<u64>>,
}

/// The latest timestamp in the source.
#[derive(Serialize)]
struct LatestTimestampJson {
    table: &'static str,
    column: &'static str,
    at: String,
}

impl SourceJson {
    fn new(facts: &SourceFacts) -> Self {
        Self {
            fingerprint: "matched",
            latest_timestamp: facts.latest_timestamp().map(|latest| LatestTimestampJson {
                table: latest.table,
                column: latest.column,
                at: format_instant(latest.at),
            }),
            auto_increment: facts
                .auto_increments()
                .iter()
                .map(|entry| (entry.table.name(), entry.next_id))
                .collect(),
        }
    }
}

/// One row of `summary.json`'s `not_migrated` list.
#[derive(Serialize)]
struct NotMigratedJson {
    table: &'static str,
    reason: &'static str,
}

impl RunSummary {
    /// How long the run took: `finished_at - started_at`.
    #[must_use]
    pub fn duration(&self) -> TimeDelta {
        self.finished_at - self.started_at
    }

    /// The contents of `summary.json`: pretty-printed JSON with a final
    /// newline. Every instant is RFC 3339 in UTC ([`format_instant`]), the
    /// duration is in whole milliseconds (`duration_ms`, rounded towards
    /// zero), `arguments` spells out every argument of the run with its
    /// defaults ([`ImportOptions::to_arguments`]), `error` is `null`
    /// unless the run failed, and `source` is the source session's part,
    /// `null` when the run stopped before the session ended: `fingerprint`
    /// (`matched`), `latest_timestamp` (`table`, `column` and `at`, or
    /// `null` for a source without one) and `auto_increment` by table.
    /// `defects` is the defect detectors' part: `complete` (the run reached
    /// its end), `checked` (the defects whose detector ran), the counts
    /// `uncovered` and `refused`, and `records`, one per instance and per
    /// entry whose subject the rule does not find (`defect`, `subject`,
    /// `outcome`, `rows`, `paths`, and `entry`, `expected`, `found` and
    /// `proposed`, each `null` where there is none, `found` when no row is
    /// the subject and `proposed` where the source value is kept). Equal
    /// summaries give equal bytes.
    ///
    /// # Panics
    ///
    /// Never: the layout holds only strings, integers, a map with string keys
    /// and lists of them, which `serde_json` always writes.
    #[must_use]
    pub fn to_json(&self) -> String {
        let error = match &self.outcome {
            Outcome::Completed => None,
            Outcome::Failed { message } => Some(message.as_str()),
        };
        let summary = SummaryJson {
            report_format: REPORT_FORMAT,
            task: TASK_NAME,
            outcome: self.outcome.as_str(),
            error,
            started_at: format_instant(self.started_at),
            finished_at: format_instant(self.finished_at),
            duration_ms: self.duration().num_milliseconds(),
            arguments: self.options.to_arguments(),
            source: self.parts.source.as_ref().map(SourceJson::new),
            defects: DefectsJson::new(&self.parts.defects),
            not_migrated: NOT_MIGRATED_TABLES
                .iter()
                .map(|row| NotMigratedJson {
                    table: row.table,
                    reason: row.reason,
                })
                .collect(),
        };
        let mut json = serde_json::to_string_pretty(&summary)
            .expect("the summary layout always serialises to JSON");
        json.push('\n');
        json
    }

    /// The contents of `report.md`: the privacy note, the summary table,
    /// the error of a failed run, the source session's part (`## Source`),
    /// the defect detectors' part (`## Defects`), one section per entry of
    /// [`STEP_SECTIONS`] and the not-migrated tables (data-migration.md
    /// §9.1). Values from the arguments and the error are escaped, so no
    /// path or message can break the Markdown structure.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        // The arguments are labelled with their names, so a reader can copy
        // them back into a command line.
        let mut rows = vec![
            ("Outcome".to_owned(), self.outcome.as_str().to_owned()),
            ("Started".to_owned(), format_instant(self.started_at)),
            ("Finished".to_owned(), format_instant(self.finished_at)),
            ("Duration".to_owned(), format_duration(self.duration())),
        ];
        rows.extend(self.options.to_arguments());

        let mut markdown = String::from(
            "# import_mysql run report\n\n\
             This report can hold draft content and account counts. Keep it with the rehearsal \
             tooling, and never attach it to an issue, a pull request or a public CI run \
             (data-migration.md §11.3).\n\n\
             ## Summary\n\n\
             | Field | Value |\n\
             |---|---|\n",
        );
        for (label, value) in rows {
            markdown.push_str(&table_row(&label, &value));
        }
        if let Outcome::Failed { message } = &self.outcome {
            markdown.push_str("\n## Error\n\n");
            markdown.push_str(&fenced(message));
        }
        markdown.push_str(&self.source_section());
        markdown.push_str(&self.defects_section());
        for (title, holds) in STEP_SECTIONS {
            markdown.push_str(&["\n## ", title, "\n\n", holds, "\n\n", NO_STEP_YET, "\n"].concat());
        }
        markdown.push_str(
            "\n## Not migrated\n\n\
             | Legacy table | Why it is not migrated |\n\
             |---|---|\n",
        );
        for row in NOT_MIGRATED_TABLES {
            markdown.push_str(&table_row(row.table, row.reason));
        }
        markdown
    }

    /// The `## Source` section of `report.md`: what the source session
    /// established, with a table of every allow-listed table's
    /// `AUTO_INCREMENT`, or a line that says the session did not end.
    fn source_section(&self) -> String {
        let mut section = String::from("\n## Source\n\n");
        let Some(facts) = &self.parts.source else {
            section.push_str(
                "The run stopped before the source session ended, so nothing was read from the \
                 source; the error above says why.\n",
            );
            return section;
        };
        // The table and column names are the fixed identifiers of
        // `SOURCE_TABLES` and `SOURCE_TIMESTAMPS`, never input, so they go
        // into code spans unescaped. Writing into a `String` cannot fail, so
        // the results of `write!` are ignored.
        let tables = SOURCE_TABLES.len();
        let _ = write!(
            section,
            "The schema fingerprint of the {tables} legacy tables that the import reads matched \
             the expected legacy DDL, and they were read under one LOCK TABLES ... READ \
             (data-migration.md §3.3).\n\n"
        );
        match facts.latest_timestamp() {
            Some(latest) => {
                let _ = write!(
                    section,
                    "The latest timestamp in the source is `{}`.`{}` at {}, which snapshot\\_at \
                     does not precede.\n\n",
                    latest.table,
                    latest.column,
                    format_instant(latest.at)
                );
            }
            None => section.push_str("No timestamp column of the source holds a value.\n\n"),
        }
        section.push_str("| Legacy table | AUTO\\_INCREMENT |\n|---|---|\n");
        for entry in facts.auto_increments() {
            let next_id = entry
                .next_id
                .map_or_else(|| "none".to_owned(), |next_id| next_id.to_string());
            section.push_str(&table_row(entry.table.name(), &next_id));
        }
        section
    }

    /// The `## Defects` section of `report.md`: which defects were checked,
    /// the counts per outcome, and one table row per instance and per entry
    /// whose subject the rule does not find, or a line that says no
    /// detector ran and why.
    fn defects_section(&self) -> String {
        let defects = &self.parts.defects;
        let mut section = ["\n## Defects\n\n", DEFECTS_HOLDS, "\n\n"].concat();
        if defects.checked().is_empty() {
            section.push_str(if defects.is_complete() {
                "No defect detector ran: this build has none yet. They come with the import steps \
                 of the tables they check.\n"
            } else {
                "No defect detector ran before the run stopped; the error above says why.\n"
            });
            return section;
        }
        let checked: Vec<&str> = defects
            .checked()
            .iter()
            .map(|defect| defect.as_str())
            .collect();
        // Writing into a `String` cannot fail, so the result is ignored.
        let _ = write!(
            section,
            "Checked: {}. Instances: {} (covered by a valid entry: {}, imported unchanged: {}, \
             given the default without an optional entry: {}, not covered by the resolution file: \
             {}, the entry's precondition does not hold: {}). Entries for rows that the rules no \
             longer find: {} that hold for a row fixed in the source as signed, {} stale.",
            checked.join(", "),
            defects.instances(),
            defects.count(DefectOutcome::Covered),
            defects.count(DefectOutcome::Unchanged),
            defects.count(DefectOutcome::Defaulted),
            defects.uncovered(),
            defects.count(DefectOutcome::Changed),
            defects.count(DefectOutcome::FixedInSource),
            defects.count(DefectOutcome::Stale),
        );
        if !defects.is_complete() {
            section.push_str(" The run stopped before it had checked every defect.");
        }
        section.push('\n');
        if defects.records().is_empty() {
            return section;
        }
        section.push_str(
            "\n| Defect | Subject | Outcome | Rows | Entry | Expected | Found | Proposed |\n\
             |---|---|---|---|---|---|---|---|\n",
        );
        for record in defects.records() {
            section.push_str(&defect_row(record));
        }
        section
    }

    /// The contents of `signoff.md`, the editors' sign-off sheet
    /// ([`signoff::render`]): the run's `snapshot_at` and every instance of
    /// its Defects part, with nothing else.
    #[must_use]
    pub fn to_signoff(&self) -> String {
        signoff::render(self.options.snapshot_at, &self.parts.defects)
    }
}

/// One row of the Defects table of `report.md`, every cell escaped. A
/// missing entry shows as `-`, a found value without its row as `no such
/// row`, and no proposed resolution as `source value kept`.
fn defect_row(record: &DefectRecord) -> String {
    let rows = if record.rows.is_empty() {
        "-".to_owned()
    } else {
        id_list(&record.rows)
    };
    let cells = [
        record.defect.as_str().to_owned(),
        record.subject.to_string(),
        record.outcome.as_str().to_owned(),
        rows,
        record
            .entry
            .as_ref()
            .map_or_else(|| "-".to_owned(), EntryRef::name),
        record
            .entry
            .as_ref()
            .map_or_else(|| "-".to_owned(), |entry| entry.expected.to_string()),
        record
            .found
            .as_ref()
            .map_or_else(|| "no such row".to_owned(), ToString::to_string),
        record
            .proposed
            .as_ref()
            .map_or_else(|| "source value kept".to_owned(), ToString::to_string),
    ];
    let cells: Vec<String> = cells.iter().map(|cell| escape_cell(cell)).collect();
    format!("| {} |\n", cells.join(" | "))
}

/// One row of a two-column Markdown table, both cells escaped.
fn table_row(left: &str, right: &str) -> String {
    format!("| {} | {} |\n", escape_cell(left), escape_cell(right))
}

/// Writes a duration as seconds with three decimals, for example `1.234 s`
/// or `-0.500 s`, rounded towards zero to the millisecond.
///
/// - `duration`: the duration to write.
#[must_use]
pub fn format_duration(duration: TimeDelta) -> String {
    let milliseconds = duration.num_milliseconds();
    let sign = if milliseconds < 0 { "-" } else { "" };
    let magnitude = milliseconds.unsigned_abs();
    format!("{sign}{}.{:03} s", magnitude / 1000, magnitude % 1000)
}

/// Makes `text` safe inside one cell of a Markdown table: backslashes,
/// pipes, backticks, `*`, `_`, `~`, `<`, `&`, `$`, `[` and `]` are escaped
/// with a backslash, and line breaks become spaces, so the text cannot end
/// the cell, the row or the table, and shows as written apart from the line
/// breaks.
///
/// Those are the characters that start an inline construct inside a table
/// cell in Markdown, GitHub's dialect included: code spans, emphasis,
/// strikethrough (`~`), raw HTML and autolinks (`<`), entity references
/// (`&lt;` would show as `<`) and links, and `$`, which GitHub reads as the
/// start of inline math. Markdown accepts a backslash before any ASCII
/// punctuation, so each escape shows as the character alone. Other
/// punctuation is left as it is, so that paths stay readable in the
/// Markdown source.
///
/// - `text`: the cell's text.
#[must_use]
pub fn escape_cell(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' | '|' | '`' | '*' | '_' | '~' | '<' | '&' | '$' | '[' | ']' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '\r' | '\n' => escaped.push(' '),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Puts `text` in a fenced code block whose fence is longer than any run of
/// backticks in `text`, so the text shows verbatim and cannot close the
/// block early. The block ends with a newline.
///
/// - `text`: the block's content, for example an error message.
#[must_use]
pub fn fenced(text: &str) -> String {
    let longest_run = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run.max(2) + 1);
    let newline = if text.ends_with('\n') { "" } else { "\n" };
    format!("{fence}text\n{text}{newline}{fence}\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::import_mysql::{
        defects::{
            DefectDetector, IndexImage, Instance, NoEntry, Proposal, Subject, Value, Word,
            testing::{Given, any_part},
        },
        options::{CanteenAuditLocalBefore, Mode},
        resolutions::{PageHidden, PostDateShift, PostIndexImage, parse},
        source_session::{AutoIncrement, SOURCE_TIMESTAMPS, SourceTimestamp},
    };
    use proptest::prelude::*;
    use std::num::NonZeroU32;

    fn instant(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .expect("RFC 3339")
            .with_timezone(&Utc)
    }

    fn options() -> ImportOptions {
        ImportOptions {
            mode: Mode::DryRun,
            snapshot_at: instant("2026-10-23T17:01:02Z"),
            resolutions: PathBuf::from("/app/data/import/resolutions.toml"),
            storage_root: PathBuf::from("/storage"),
            report: PathBuf::from("/reports/run-1"),
            canteen_audit_local_before: CanteenAuditLocalBefore::default_cutoff(),
        }
    }

    fn summary(outcome: Outcome) -> RunSummary {
        RunSummary {
            options: options(),
            started_at: instant("2026-10-31T18:00:00Z"),
            finished_at: instant("2026-10-31T18:00:04.25Z"),
            outcome,
            parts: ReportParts::default(),
        }
    }

    #[test]
    fn the_summary_records_the_run_time_and_the_arguments() {
        let json: serde_json::Value =
            serde_json::from_str(&summary(Outcome::Completed).to_json()).expect("JSON");
        assert_eq!(json["report_format"], 1);
        assert_eq!(json["task"], "import_mysql");
        assert_eq!(json["outcome"], "completed");
        assert_eq!(json["error"], serde_json::Value::Null);
        assert_eq!(json["started_at"], "2026-10-31T18:00:00Z");
        assert_eq!(json["finished_at"], "2026-10-31T18:00:04.250Z");
        assert_eq!(json["duration_ms"], 4250);
        assert_eq!(
            json["arguments"],
            serde_json::json!({
                "canteen_audit_local_before": "2022-01-21T00:00:00Z",
                "mode": "dry-run",
                "report": "/reports/run-1",
                "resolutions": "/app/data/import/resolutions.toml",
                "snapshot_at": "2026-10-23T17:01:02Z",
                "storage_root": "/storage",
            })
        );
        let not_migrated = json["not_migrated"].as_array().expect("a list");
        assert_eq!(not_migrated.len(), NOT_MIGRATED_TABLES.len());
        for (row, expected) in not_migrated.iter().zip(NOT_MIGRATED_TABLES) {
            assert_eq!(row["table"], expected.table);
            assert_eq!(row["reason"], expected.reason);
        }
    }

    #[test]
    fn the_summary_fields_come_in_a_fixed_order_and_end_with_a_newline() {
        let text = summary(Outcome::Completed).to_json();
        assert!(text.ends_with("}\n"), "{text}");
        let positions: Vec<usize> = [
            "\"report_format\"",
            "\"task\"",
            "\"outcome\"",
            "\"error\"",
            "\"started_at\"",
            "\"finished_at\"",
            "\"duration_ms\"",
            "\"arguments\"",
            "\"source\"",
            "\"defects\"",
            "\"not_migrated\"",
        ]
        .iter()
        .map(|key| text.find(key).unwrap_or_else(|| panic!("{key} in {text}")))
        .collect();
        assert!(positions.is_sorted(), "{text}");
    }

    #[test]
    fn a_failed_run_records_its_error() {
        let failed = summary(Outcome::Failed {
            message: "V8 failed: duplicate live canteen dates".to_owned(),
        });
        let json: serde_json::Value = serde_json::from_str(&failed.to_json()).expect("JSON");
        assert_eq!(json["outcome"], "failed");
        assert_eq!(json["error"], "V8 failed: duplicate live canteen dates");
        let markdown = failed.to_markdown();
        assert!(markdown.contains("| Outcome | failed |"), "{markdown}");
        assert!(
            markdown
                .contains("\n## Error\n\n```text\nV8 failed: duplicate live canteen dates\n```\n"),
            "{markdown}"
        );
        assert!(
            !summary(Outcome::Completed)
                .to_markdown()
                .contains("## Error")
        );
    }

    /// Facts of a source whose first table has an `AUTO_INCREMENT` of 621
    /// and whose pivot tables report none.
    fn source_facts(latest: Option<SourceTimestamp>) -> SourceFacts {
        let auto_increments = SOURCE_TABLES
            .iter()
            .enumerate()
            .map(|(index, table)| AutoIncrement {
                table: *table,
                next_id: (!table.name().contains("pivot"))
                    .then(|| 621 + u64::try_from(index).expect("small")),
            })
            .collect();
        // The latest instant chrono has: no timestamp lies after it.
        SourceFacts::checked(auto_increments, latest, DateTime::<Utc>::MAX_UTC)
            .expect("snapshot_at after every timestamp")
    }

    #[test]
    fn the_source_part_shows_the_fingerprint_the_latest_timestamp_and_auto_increment() {
        let mut run = summary(Outcome::Completed);
        run.parts.source = Some(source_facts(Some(SourceTimestamp {
            table: "posts_data",
            column: "updated_at",
            at: instant("2026-10-23T17:00:59Z"),
        })));
        let json: serde_json::Value = serde_json::from_str(&run.to_json()).expect("JSON");
        assert_eq!(json["source"]["fingerprint"], "matched");
        assert_eq!(
            json["source"]["latest_timestamp"],
            serde_json::json!({
                "table": "posts_data",
                "column": "updated_at",
                "at": "2026-10-23T17:00:59Z",
            })
        );
        let auto_increment = json["source"]["auto_increment"]
            .as_object()
            .expect("an object");
        assert_eq!(auto_increment.len(), SOURCE_TABLES.len());
        assert_eq!(auto_increment["posts_data"], 621);
        assert_eq!(
            auto_increment["posts_pivot_labels_data"],
            serde_json::Value::Null
        );
        assert!(!auto_increment.contains_key("newsletter"));

        let markdown = run.to_markdown();
        assert!(
            markdown.contains(
                "\n## Source\n\nThe schema fingerprint of the 17 legacy tables that the import \
                 reads matched the expected legacy DDL, and they were read under one LOCK TABLES \
                 ... READ (data-migration.md §3.3).\n\nThe latest timestamp in the source is \
                 `posts_data`.`updated_at` at 2026-10-23T17:00:59Z, which snapshot\\_at does not \
                 precede.\n\n| Legacy table | AUTO\\_INCREMENT |\n|---|---|\n| posts\\_data | \
                 621 |\n"
            ),
            "{markdown}"
        );
        assert!(
            markdown.contains("| posts\\_pivot\\_labels\\_data | none |\n"),
            "{markdown}"
        );
        // The source part lists only the tables the import reads; the
        // newsletter appears only in the not-migrated table at the end.
        let source_part = markdown
            .split_once("\n## Source\n")
            .and_then(|(_, rest)| rest.split_once("\n## Defects\n"))
            .expect("a Source section before Defects")
            .0;
        assert!(!source_part.contains("newsletter"), "{source_part}");
        assert_eq!(
            source_part.matches("\n| ").count(),
            1 + SOURCE_TABLES.len(),
            "{source_part}"
        );

        // A source without a timestamp, and a run that stopped before the
        // session ended.
        run.parts.source = Some(source_facts(None));
        let json: serde_json::Value = serde_json::from_str(&run.to_json()).expect("JSON");
        assert_eq!(json["source"]["latest_timestamp"], serde_json::Value::Null);
        assert!(
            run.to_markdown()
                .contains("No timestamp column of the source holds a value.\n"),
        );
        let stopped = summary(Outcome::Failed {
            message: "refusing to import: the target table `users` already holds a row".to_owned(),
        });
        let json: serde_json::Value = serde_json::from_str(&stopped.to_json()).expect("JSON");
        assert_eq!(json["source"], serde_json::Value::Null);
        assert!(stopped.to_markdown().contains(
            "\n## Source\n\nThe run stopped before the source session ended, so nothing was \
                 read from the source; the error above says why.\n"
        ));
    }

    #[test]
    fn the_markdown_report_has_every_part_of_section_11_3() {
        let markdown = summary(Outcome::Completed).to_markdown();
        assert!(
            markdown.starts_with("# import_mysql run report\n\n"),
            "{markdown}"
        );
        assert!(
            markdown.contains("never attach it to an issue"),
            "{markdown}"
        );
        for row in [
            "| Outcome | completed |",
            "| Started | 2026-10-31T18:00:00Z |",
            "| Finished | 2026-10-31T18:00:04.250Z |",
            "| Duration | 4.250 s |",
            "| mode | dry-run |",
            "| snapshot\\_at | 2026-10-23T17:01:02Z |",
            "| canteen\\_audit\\_local\\_before | 2022-01-21T00:00:00Z |",
            "| storage\\_root | /storage |",
        ] {
            assert!(markdown.contains(row), "{row} in {markdown}");
        }
        let mut last = 0;
        for title in [
            "Summary",
            "Source",
            "Defects",
            "Rule hits",
            "Diffs",
            "Files",
            "Accounts",
            "Not migrated",
        ] {
            let at = markdown
                .find(&format!("\n## {title}\n"))
                .unwrap_or_else(|| panic!("section {title} in {markdown}"));
            assert!(at > last, "{title} out of order");
            last = at;
        }
        assert_eq!(markdown.matches(NO_STEP_YET).count(), STEP_SECTIONS.len());
        for row in NOT_MIGRATED_TABLES {
            let line = format!(
                "| {} | {} |",
                escape_cell(row.table),
                escape_cell(row.reason)
            );
            assert!(markdown.contains(&line), "{line} in {markdown}");
        }
    }

    fn id(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).expect("a row id")
    }

    fn day(text: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a date")
    }

    /// The index image of the synthetic D3 post 70.
    const IMAGE: &str = "index/0123456789abcdef0123456789abcdef.jpg";

    /// A dry run's Defects part with one record of each outcome: post 12
    /// covered, post 14 changed, post 16 stale (its row gone), post 30
    /// uncovered (D1); post 70 fixed in the source as its entry pins (D3);
    /// a D4 post imported unchanged; and a D10 page that no entry lists,
    /// which takes the default.
    fn dry_run_defects() -> DefectsPart {
        let signed = parse(&format!(
            "version = 1\n\n[[post_date_shift]]\npost_id = 12\nexpect_date = \
             2019-05-02\n\n[[post_date_shift]]\npost_id = 14\nexpect_date = \
             2020-02-29\n\n[[post_date_shift]]\npost_id = 16\nexpect_date = \
             2020-03-01\n\n[[post_index_image]]\npost_id = 70\nexpect_index_image = \
             \"{IMAGE}\"\n"
        ))
        .expect("a valid synthetic file");
        let early = |post: u32, found: &str, next: &str| {
            Instance::new(Subject::Post(id(post)), Value::Date(day(found))).with_proposed(
                Proposal {
                    resolution: Value::Date(day(next)),
                    shown: Value::Date(day(next)),
                },
            )
        };
        let dates = Given::<PostDateShift>::new(
            vec![
                early(12, "2019-05-02", "2019-05-03"),
                early(14, "2020-03-02", "2020-03-03"),
                early(30, "2021-06-01", "2021-06-02"),
            ],
            None,
        );
        let images = Given::<PostIndexImage>::new(
            Vec::new(),
            Some(Value::IndexImage(IndexImage::new(IMAGE).expect("a path"))),
        );
        let listed = Given::<NoEntry>::new(
            vec![Instance::new(Subject::Post(id(40)), Value::Null)],
            None,
        );
        let published = Proposal {
            resolution: Value::Choice(Word::Published),
            shown: Value::Choice(Word::Published),
        };
        let pages = Given::<PageHidden>::new(
            vec![
                Instance::new(
                    Subject::Page(id(5)),
                    Value::Instant(instant("2023-11-14T22:13:20Z")),
                )
                .with_proposed(published),
            ],
            None,
        );
        let detector = DefectDetector::new(Mode::DryRun, &signed);
        let mut part = DefectsPart::default();
        detector
            .run(&dates, &[], &mut part)
            .expect("a dry run goes on");
        detector
            .run(&images, &[], &mut part)
            .expect("post 70 was fixed as signed");
        detector
            .run(&listed, &[], &mut part)
            .expect("nothing to refuse");
        detector
            .run(&pages, &[], &mut part)
            .expect("D10's entries are optional");
        part
    }

    #[test]
    fn the_defects_part_lists_every_instance_and_every_entry_without_one() {
        let run = concluded_dry_run();
        let json: serde_json::Value = serde_json::from_str(&run.to_json()).expect("JSON");
        let defects = &json["defects"];
        assert_eq!(defects["complete"], true);
        assert_eq!(
            defects["checked"],
            serde_json::json!(["D1", "D3", "D4", "D10"])
        );
        assert_eq!(
            (defects["uncovered"].clone(), defects["refused"].clone()),
            (1.into(), 2.into())
        );
        assert_eq!(
            defects["records"],
            serde_json::json!([
                {
                    "defect": "D1", "subject": "post 12", "outcome": "covered", "rows": [12],
                    "paths": ["/posts/12"], "entry": "post_date_shift[1]",
                    "expected": "2019-05-02", "found": "2019-05-02", "proposed": "2019-05-02",
                },
                {
                    "defect": "D1", "subject": "post 14", "outcome": "changed", "rows": [14],
                    "paths": ["/posts/14"], "entry": "post_date_shift[2]",
                    "expected": "2020-02-29", "found": "2020-03-02", "proposed": "2020-03-03",
                },
                {
                    "defect": "D1", "subject": "post 16", "outcome": "stale", "rows": [],
                    "paths": [], "entry": "post_date_shift[3]", "expected": "2020-03-01",
                    "found": null, "proposed": null,
                },
                {
                    "defect": "D1", "subject": "post 30", "outcome": "uncovered", "rows": [30],
                    "paths": ["/posts/30"], "entry": null, "expected": null,
                    "found": "2021-06-01", "proposed": "2021-06-02",
                },
                {
                    "defect": "D3", "subject": "post 70", "outcome": "fixed_in_source",
                    "rows": [70], "paths": ["/posts/70"], "entry": "post_index_image[1]",
                    "expected": IMAGE, "found": IMAGE, "proposed": null,
                },
                {
                    "defect": "D4", "subject": "post 40", "outcome": "unchanged", "rows": [40],
                    "paths": ["/posts/40"], "entry": null, "expected": null, "found": "none",
                    "proposed": null,
                },
                {
                    "defect": "D10", "subject": "page 5", "outcome": "defaulted", "rows": [5],
                    "paths": [], "entry": null, "expected": null,
                    "found": "2023-11-14T22:13:20Z", "proposed": "published",
                },
            ])
        );
    }

    /// The run of [`dry_run_defects`], concluded: a failed dry run.
    fn concluded_dry_run() -> RunSummary {
        let mut run = summary(Outcome::Completed);
        run.parts.defects = dry_run_defects();
        let failure = run.parts.defects.conclude().expect_err("three problems");
        run.outcome = Outcome::Failed {
            message: failure.to_string(),
        };
        run
    }

    #[test]
    fn the_defects_section_and_the_sheet_show_every_record() {
        let run = concluded_dry_run();
        let markdown = run.to_markdown();
        let section = markdown
            .split_once("\n## Defects\n\n")
            .and_then(|(_, rest)| rest.split_once("\n## Rule hits\n"))
            .expect("a Defects section before Rule hits")
            .0;
        assert_eq!(
            section,
            [
                DEFECTS_HOLDS,
                "\n\nChecked: D1, D3, D4, D10. Instances: 5 (covered by a valid entry: 1, imported \
                 unchanged: 1, given the default without an optional entry: 1, not covered by the \
                 resolution file: 1, the entry's precondition does not hold: 1). Entries for rows \
                 that the rules no longer find: 1 that hold for a row fixed in the source as \
                 signed, 1 stale.\n\n| Defect | Subject | Outcome | Rows | Entry | Expected | \
                 Found | Proposed |\n\
                 |---|---|---|---|---|---|---|---|\n\
                 | D1 | post 12 | covered | 12 | post\\_date\\_shift\\[1\\] | 2019-05-02 | \
                 2019-05-02 | 2019-05-02 |\n\
                 | D1 | post 14 | changed | 14 | post\\_date\\_shift\\[2\\] | 2020-02-29 | \
                 2020-03-02 | 2020-03-03 |\n\
                 | D1 | post 16 | stale | - | post\\_date\\_shift\\[3\\] | 2020-03-01 | no such \
                 row | source value kept |\n\
                 | D1 | post 30 | uncovered | 30 | - | - | 2021-06-01 | 2021-06-02 |\n\
                 | D3 | post 70 | fixed\\_in\\_source | 70 | post\\_index\\_image\\[1\\] | \
                 index/0123456789abcdef0123456789abcdef.jpg | \
                 index/0123456789abcdef0123456789abcdef.jpg | source value kept |\n\
                 | D4 | post 40 | unchanged | 40 | - | - | none | source value kept |\n\
                 | D10 | page 5 | defaulted | 5 | - | - | 2023-11-14T22:13:20Z | published |\n",
            ]
            .concat()
        );
        // The error names every problem.
        assert!(
            markdown.contains("- D1 post 30 is not covered"),
            "{markdown}"
        );

        // The sign-off sheet lists the five instances and the row fixed in
        // the source, not the stale entry.
        let sheet = run.to_signoff();
        assert!(
            sheet.contains("snapshot of 2026-10-23T17:01:02Z"),
            "{sheet}"
        );
        for row in [
            "| 14 | /posts/14 | 2020-03-03 |\n",
            "| 70 | /posts/70 | fixed in the source as signed |\n",
            "| 40 | /posts/40 | imported unchanged |\n",
            "| 5 | - | published |\n",
        ] {
            assert!(sheet.contains(row), "{row} in {sheet}");
        }
        assert!(!sheet.contains("| 16 |"), "{sheet}");
    }

    #[test]
    fn a_run_without_a_detector_says_why() {
        let mut run = summary(Outcome::Failed {
            message: "refusing to import: the target table `users` already holds a row".to_owned(),
        });
        assert!(run.to_markdown().contains(&format!(
            "\n## Defects\n\n{DEFECTS_HOLDS}\n\nNo defect detector ran before the run stopped; \
             the error above says why.\n"
        )));
        let json: serde_json::Value = serde_json::from_str(&run.to_json()).expect("JSON");
        assert_eq!(
            json["defects"],
            serde_json::json!({
                "complete": false, "checked": [], "uncovered": 0, "refused": 0, "records": [],
            })
        );
        run.outcome = Outcome::Completed;
        run.parts.defects.conclude().expect("nothing found");
        assert!(run.to_markdown().contains(
            "\nNo defect detector ran: this build has none yet. They come with the import steps \
             of the tables they check.\n"
        ));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&run.to_json()).expect("JSON")["defects"]["complete"],
            true
        );
        // A run that stopped after a detector says so after the counts.
        let mut run = summary(Outcome::Completed);
        run.parts.defects = dry_run_defects();
        assert!(run.to_markdown().contains(
            "fixed in the source as signed, 1 stale. The run stopped before it had checked every \
             defect.\n"
        ),);
    }

    #[test]
    fn the_duration_is_the_difference_of_the_two_readings() {
        let mut run = summary(Outcome::Completed);
        assert_eq!(run.duration(), TimeDelta::milliseconds(4250));
        run.finished_at = run.started_at;
        assert_eq!(run.duration(), TimeDelta::zero());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&run.to_json()).unwrap()["duration_ms"],
            0
        );
        // The system clock set back during the run.
        run.finished_at = run.started_at - TimeDelta::milliseconds(500);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&run.to_json()).unwrap()["duration_ms"],
            -500
        );
        assert!(run.to_markdown().contains("| Duration | -0.500 s |"));
    }

    #[test]
    fn durations_are_written_in_seconds_to_the_millisecond() {
        for (duration, written) in [
            (TimeDelta::zero(), "0.000 s"),
            (TimeDelta::milliseconds(7), "0.007 s"),
            (TimeDelta::milliseconds(61_250), "61.250 s"),
            (TimeDelta::microseconds(1_999), "0.001 s"),
            (TimeDelta::milliseconds(-1_005), "-1.005 s"),
        ] {
            assert_eq!(format_duration(duration), written, "{duration}");
        }
    }

    #[test]
    fn cells_and_fences_keep_the_markdown_intact() {
        assert_eq!(escape_cell("a|b"), "a\\|b");
        assert_eq!(escape_cell("C:\\x"), "C:\\\\x");
        assert_eq!(escape_cell("/srv/my_dir\nnext"), "/srv/my\\_dir next");
        assert_eq!(
            escape_cell("`code` *em* <b> [l]"),
            "\\`code\\` \\*em\\* \\<b> \\[l\\]"
        );
        // An entity reference, strikethrough and GitHub's inline math would
        // otherwise show something other than the text.
        assert_eq!(
            escape_cell("&lt; ~~x~~ $y$"),
            "\\&lt; \\~\\~x\\~\\~ \\$y\\$"
        );
        // Other punctuation stays readable.
        assert_eq!(escape_cell("/srv/run-1/a.toml#x!"), "/srv/run-1/a.toml#x!");
        assert_eq!(fenced("plain"), "```text\nplain\n```\n");
        assert_eq!(fenced("ends\n"), "```text\nends\n```\n");
        assert_eq!(fenced("has ``` inside"), "````text\nhas ``` inside\n````\n");
        assert_eq!(fenced("``````"), "```````text\n``````\n```````\n");
    }

    fn any_instant() -> impl Strategy<Value = DateTime<Utc>> {
        (0_i64..253_402_300_800, 0_u32..1_000_000_000).prop_map(|(seconds, nanos)| {
            DateTime::from_timestamp(seconds, nanos).expect("inside chrono's range")
        })
    }

    /// Text for paths and error messages. Besides arbitrary text it makes,
    /// often, the characters and lines that Markdown reads as structure:
    /// pipes, backticks, emphasis, entities, line breaks of both kinds, and
    /// lines that would start a heading or a fenced block.
    fn any_text() -> impl Strategy<Value = String> {
        prop_oneof![
            "[^\u{0}]{0,40}",
            "[a-z /|`*_~<>&$#\\[\\]\\\\\r\n-]{0,40}",
            "(\n## [A-Za-z ]{0,10}|\n`{3,7}[a-z]{0,4}|\r# x|\n\\| a \\| b \\||[a-z]{0,5}){0,6}",
        ]
    }

    /// Runs with any arguments, instants and outcome.
    fn any_run() -> impl Strategy<Value = RunSummary> {
        (
            prop_oneof![Just(Mode::DryRun), Just(Mode::Apply)],
            any_instant(),
            (any_text(), any_text(), any_text()),
            prop_oneof![
                Just(CanteenAuditLocalBefore::All),
                any_instant().prop_map(CanteenAuditLocalBefore::Before),
            ],
            (any_instant(), any_instant()),
            proptest::option::of(any_text()),
            (proptest::option::of(any_source()), any_part()),
        )
            .prop_map(
                |(
                    mode,
                    snapshot_at,
                    (resolutions, storage_root, report),
                    cutoff,
                    (started_at, finished_at),
                    message,
                    (source, defects),
                )| {
                    RunSummary {
                        options: ImportOptions {
                            mode,
                            snapshot_at,
                            resolutions: PathBuf::from(resolutions),
                            storage_root: PathBuf::from(storage_root),
                            report: PathBuf::from(report),
                            canteen_audit_local_before: cutoff,
                        },
                        started_at,
                        finished_at,
                        outcome: message
                            .map_or(Outcome::Completed, |message| Outcome::Failed { message }),
                        parts: ReportParts { source, defects },
                    }
                },
            )
    }

    /// What any source session could establish.
    fn any_source() -> impl Strategy<Value = SourceFacts> {
        (
            proptest::collection::vec(proptest::option::of(any::<u64>()), SOURCE_TABLES.len()),
            proptest::option::of((
                proptest::sample::select(SOURCE_TIMESTAMPS.to_vec()),
                any_instant(),
            )),
        )
            .prop_map(|(next_ids, latest)| {
                let auto_increments = SOURCE_TABLES
                    .iter()
                    .zip(next_ids)
                    .map(|(table, next_id)| AutoIncrement {
                        table: *table,
                        next_id,
                    })
                    .collect();
                let latest = latest.map(|(columns, at)| SourceTimestamp {
                    table: columns.table,
                    column: columns.columns[0],
                    at,
                });
                SourceFacts::checked(auto_increments, latest, DateTime::<Utc>::MAX_UTC)
                    .expect("snapshot_at after every timestamp")
            })
    }

    /// Splits `markdown` into the lines outside fenced code blocks and the
    /// content of each fenced block. A block opens with a line of three or
    /// more backticks; the report closes it with a line that is exactly the
    /// same fence. Panics if the report could close a block earlier by
    /// the Markdown rule (a line, ended by `\n` or `\r`, that starts, after
    /// spaces, with at least as many backticks as the fence), or leaves one
    /// open, or has a `\r` outside one.
    fn outside_fences(markdown: &str) -> (Vec<&str>, Vec<String>) {
        let mut lines = Vec::new();
        let mut blocks = Vec::new();
        let mut open: Option<(&str, Vec<&str>)> = None;
        for line in markdown.split('\n') {
            match &mut open {
                Some((fence, content)) if line == *fence => {
                    blocks.push(content.join("\n"));
                    open = None;
                }
                Some((fence, content)) => {
                    for piece in line.split('\r') {
                        let run = piece
                            .trim_start_matches(' ')
                            .chars()
                            .take_while(|c| *c == '`')
                            .count();
                        assert!(run < fence.len(), "{line:?} closes {fence} early");
                    }
                    content.push(line);
                }
                None if line.starts_with("```") => {
                    let fence_length = line.chars().take_while(|c| *c == '`').count();
                    open = Some((&line[..fence_length], Vec::new()));
                }
                None => {
                    assert!(!line.contains('\r'), "a line break inside {line:?}");
                    lines.push(line);
                }
            }
        }
        assert!(open.is_none(), "a fenced block is never closed: {markdown}");
        (lines, blocks)
    }

    /// How many pipes of `line` are not escaped, that is, are preceded by
    /// an even number of backslashes.
    fn unescaped_pipes(line: &str) -> usize {
        let characters: Vec<char> = line.chars().collect();
        characters
            .iter()
            .enumerate()
            .filter(|(at, character)| {
                **character == '|'
                    && characters[..*at]
                        .iter()
                        .rev()
                        .take_while(|c| **c == '\\')
                        .count()
                        % 2
                        == 0
            })
            .count()
    }

    proptest! {
        #[test]
        fn the_summary_parses_back_to_the_run_it_describes(
            started_at in any_instant(),
            finished_at in any_instant(),
            message in proptest::option::of(".{0,80}"),
        ) {
            let outcome = message.clone().map_or(Outcome::Completed, |message| Outcome::Failed { message });
            let run = RunSummary { options: options(), started_at, finished_at, outcome, parts: ReportParts::default() };
            let json: serde_json::Value = serde_json::from_str(&run.to_json()).expect("JSON");
            let (started, finished) = (format_instant(started_at), format_instant(finished_at));
            prop_assert_eq!(json["started_at"].as_str(), Some(started.as_str()));
            prop_assert_eq!(json["finished_at"].as_str(), Some(finished.as_str()));
            prop_assert_eq!(json["duration_ms"].as_i64(), Some((finished_at - started_at).num_milliseconds()));
            prop_assert_eq!(json["error"].as_str(), message.as_deref());
            prop_assert_eq!(json["outcome"].as_str(), Some(run.outcome.as_str()));
            let expected_outcome = if message.is_some() { "failed" } else { "completed" };
            prop_assert_eq!(run.outcome.as_str(), expected_outcome);
            prop_assert_eq!(run.to_json(), run.clone().to_json());
        }

        #[test]
        fn no_argument_or_error_breaks_the_markdown_structure(run in any_run()) {
            let markdown = run.to_markdown();
            prop_assert_eq!(&markdown, &run.clone().to_markdown());
            let (lines, blocks) = outside_fences(&markdown);

            // One title, first, and the sections in their order; Error only
            // for a failed run.
            prop_assert_eq!(lines[0], "# import_mysql run report");
            prop_assert_eq!(lines.iter().filter(|line| line.starts_with("# ")).count(), 1);
            let headings: Vec<&str> = lines.iter().filter_map(|line| line.strip_prefix("## ")).collect();
            let mut expected = vec!["Summary"];
            if matches!(run.outcome, Outcome::Failed { .. }) {
                expected.push("Error");
            }
            expected.push("Source");
            expected.push("Defects");
            expected.extend(STEP_SECTIONS.iter().map(|(title, _)| *title));
            expected.push("Not migrated");
            prop_assert_eq!(headings, expected);
            prop_assert_eq!(
                lines.iter().filter(|line| **line == NO_STEP_YET).count(),
                STEP_SECTIONS.len()
            );

            // The error, verbatim, in the only fenced block.
            match &run.outcome {
                Outcome::Failed { message } => {
                    prop_assert_eq!(blocks.len(), 1);
                    prop_assert_eq!(blocks[0].as_str(), message.strip_suffix('\n').unwrap_or(message));
                }
                Outcome::Completed => prop_assert!(blocks.is_empty()),
            }

            // Both tables: every row one line of exactly two cells.
            let tables: Vec<&[&str]> = lines
                .split(|line| line.is_empty())
                .filter(|paragraph| paragraph.get(1) == Some(&"|---|---|"))
                .collect();
            let source_tables = usize::from(run.parts.source.is_some());
            prop_assert_eq!(tables.len(), 2 + source_tables);
            prop_assert_eq!(tables[0].len(), 2 + 4 + run.options.to_arguments().len());
            if source_tables == 1 {
                prop_assert_eq!(tables[1].len(), 2 + SOURCE_TABLES.len());
            }
            prop_assert_eq!(tables[1 + source_tables].len(), 2 + NOT_MIGRATED_TABLES.len());
            for row in tables.iter().flat_map(|table| table.iter()) {
                prop_assert!(row.starts_with("| ") || *row == "|---|---|", "{}", row);
                prop_assert!(row.ends_with(" |") || *row == "|---|---|", "{}", row);
                prop_assert_eq!(unescaped_pipes(row), 3, "{}", row);
            }

            // The Defects table, when the detectors recorded anything:
            // every row one line of exactly eight cells.
            let separator = "|---|---|---|---|---|---|---|---|";
            let defect_tables: Vec<&[&str]> = lines
                .split(|line| line.is_empty())
                .filter(|paragraph| paragraph.get(1) == Some(&separator))
                .collect();
            let records = run.parts.defects.records().len();
            prop_assert_eq!(defect_tables.len(), usize::from(records > 0));
            for table in defect_tables {
                prop_assert_eq!(table.len(), 2 + records);
                for row in table {
                    prop_assert!(row.starts_with("| ") || *row == separator, "{}", row);
                    prop_assert!(row.ends_with(" |") || *row == separator, "{}", row);
                    prop_assert_eq!(unescaped_pipes(row), 9, "{}", row);
                }
            }
        }

        #[test]
        fn an_escaped_cell_never_ends_its_cell_or_row(text in ".{0,60}") {
            let cell = escape_cell(&text);
            prop_assert!(!cell.contains(['\n', '\r']));
            // Every pipe is preceded by an odd number of backslashes, so it
            // is a literal pipe.
            let characters: Vec<char> = cell.chars().collect();
            for (at, character) in characters.iter().enumerate() {
                if *character == '|' {
                    let backslashes = characters[..at].iter().rev().take_while(|c| **c == '\\').count();
                    prop_assert_eq!(backslashes % 2, 1, "{}", cell);
                }
            }
        }

        #[test]
        fn a_fenced_block_is_closed_only_by_its_last_line(text in "[a-z`\n]{0,40}") {
            let block = fenced(&text);
            let fence: String = block.chars().take_while(|c| *c == '`').collect();
            prop_assert!(fence.len() >= 3);
            let closing = format!("\n{fence}\n");
            prop_assert!(block.ends_with(&closing));
            let body = &block[fence.len() + "text\n".len()..block.len() - fence.len() - 1];
            prop_assert!(!body.contains(&fence), "{}", block);
        }

        #[test]
        fn a_duration_is_written_as_its_milliseconds(milliseconds in -10_000_000_000_i64..10_000_000_000) {
            let written = format_duration(TimeDelta::milliseconds(milliseconds));
            let number = written.strip_suffix(" s").expect("unit");
            let (whole, fraction) = number.split_once('.').expect("decimals");
            prop_assert_eq!(fraction.len(), 3);
            let negative = whole.starts_with('-');
            let magnitude = whole.trim_start_matches('-').parse::<i64>().unwrap() * 1000
                + fraction.parse::<i64>().unwrap();
            prop_assert_eq!(if negative { -magnitude } else { magnitude }, milliseconds);
        }
    }
}
