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
//! The report is private: it holds draft content and account counts, and
//! the Markdown starts by saying so (data-migration.md §11.3).

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;

use super::{
    not_migrated::NOT_MIGRATED_TABLES,
    options::{ImportOptions, format_instant},
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

/// The sections that the import steps fill, each with what it will hold
/// (data-migration.md §11.3), in the order the Markdown report shows them.
pub const STEP_SECTIONS: [(&str, &str); 5] = [
    (
        "Defects",
        "Every detected instance of D1-D10, with the resolution applied and its precondition.",
    ),
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
    not_migrated: Vec<NotMigratedJson>,
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
    /// defaults ([`ImportOptions::to_arguments`]), and `error` is `null`
    /// unless the run failed. Equal summaries give equal bytes.
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
    /// the error of a failed run, one section per entry of
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
    use crate::domain::import_mysql::options::{CanteenAuditLocalBefore, Mode};
    use proptest::prelude::*;

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
        )
            .prop_map(
                |(
                    mode,
                    snapshot_at,
                    (resolutions, storage_root, report),
                    cutoff,
                    (started_at, finished_at),
                    message,
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
                    }
                },
            )
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
            let run = RunSummary { options: options(), started_at, finished_at, outcome };
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
            prop_assert_eq!(tables.len(), 2);
            prop_assert_eq!(tables[0].len(), 2 + 4 + run.options.to_arguments().len());
            prop_assert_eq!(tables[1].len(), 2 + NOT_MIGRATED_TABLES.len());
            for row in tables.iter().flat_map(|table| table.iter()) {
                prop_assert!(row.starts_with("| ") || *row == "|---|---|", "{}", row);
                prop_assert!(row.ends_with(" |") || *row == "|---|---|", "{}", row);
                prop_assert_eq!(unescaped_pipes(row), 3, "{}", row);
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
