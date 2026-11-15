//! The editors' sign-off sheet, `signoff.md` (data-migration.md §8.2,
//! §11.3), which every run writes into its report directory beside
//! `report.md` and `summary.json`.
//!
//! Staging is LAN-only and the run report stays private, so the editors
//! never see a report: the maintainer hands them this sheet, taken from a
//! staging dry-run report, and records in the private runbook who signed
//! and when (§8.2). Because a dry run lists every defect instance instead of
//! stopping at the first (ADR 0047), one sheet holds them all.
//!
//! The sheet lists, per defect whose detector ran, every instance with its
//! legacy row ids, the paths to open on the live site (`/posts/<id>`,
//! `/information/<slug>`) and the resolution the import proposes: the
//! entry's resolution where a valid entry covers the instance, otherwise the
//! defect's proposed default. It also lists the rows the editors fixed in
//! the source as their entry pins (D3), so they see what they signed. An
//! `N7-accept` entry that is no longer needed, because N7 now removes
//! nothing unplanned from the unchanged body it was signed for, is not an
//! instance and asks nothing of the editors: like a stale entry, it is the
//! maintainer's to remove, and only the report lists it (research:
//! DECISIONS R38 (D161)).
//! Nothing else: no person's name, no content, no error text and no path of
//! the machine. Every cell comes from the types of [`super::defects`],
//! which cannot hold free text, or is a fixed phrase of this module, and the
//! tests check every line of every sheet a run can write against that list.
//! A detector slice that needs another column adds it here, with its test.
//! The one column with content that data-migration.md §8.1 and §8.2 plan is
//! D6's two menus side by side: the dish names of the two rows on a date,
//! which the public site already shows and which hold no person's name
//! (ADR 0047 §3). The D6 detector adds it, and the line check learns the
//! column's exact shape.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};

use super::{
    defects::{DefectOutcome, DefectRecord, DefectsPart, id_list},
    options::format_instant,
    report::escape_cell,
};

/// The sheet's first line.
pub const TITLE: &str = "# Defect sign-off sheet";

/// The heading row of every defect's table, and its separator.
pub const TABLE_HEAD: &str =
    "| Rows | Open on the live site | Proposed resolution |\n|---|---|---|\n";

/// What a cell shows when the instance has nothing for it: no row ids, or
/// no page to open on the live site.
pub const NOTHING: &str = "-";

/// The proposed resolution of a defect that is imported unchanged (D4, D7).
pub const IMPORTED_UNCHANGED: &str = "imported unchanged";

/// The proposed resolution of an instance that no valid entry resolves and
/// whose defect has no default.
pub const EDITORS_DECIDE: &str = "no default: the editors decide";

/// The proposed resolution of a row that the editors fixed in the source
/// as its entry pins (D3): the import keeps the source value.
pub const FIXED_IN_SOURCE: &str = "fixed in the source as signed";

/// What a defect's section says when its detector found no instance.
pub const NO_INSTANCE: &str = "No instance found.";

/// The line for a run that reached its end after one or more detectors.
pub const COMPLETE: &str = "The run checked every defect that this build detects.";

/// The line for a run that stopped after one or more detectors.
pub const INCOMPLETE: &str = "The run stopped before it had checked every defect, so the sheet \
                              may miss instances; the run report says why.";

/// The line for a run that reached its end in a build without detectors.
pub const NO_DETECTOR: &str = "This build detects no defect yet, so the sheet lists no instance.";

/// The line for a run that stopped before its first detector.
pub const STOPPED_FIRST: &str = "The run stopped before it checked any defect, so the sheet lists \
                                 no instance; the run report says why.";

/// The introduction before the snapshot instant.
const INTRO_START: &str = "The defect instances that the import found in the source snapshot of ";

/// The introduction after the snapshot instant.
const INTRO_END: &str = ", with the legacy row ids, the pages to open on the live site and the \
                         resolution that the import proposes (data-migration.md §8.1, §8.2). The \
                         sheet holds no names and no content. The editors confirm each instance \
                         or say what to change; the maintainer records who signed and when in \
                         the private runbook only.";

/// Writes the sign-off sheet of a run.
///
/// - `snapshot_at`: the run's `snapshot_at`, the moment of the source the
///   instances were found in.
/// - `defects`: the run's Defects part.
///
/// The defects come in the order of data-migration.md §8.1, each with one
/// table row per instance and per row fixed in the source as signed, in
/// subject order; a stale entry (its row is gone, no longer has the
/// defect, or holds another fix than the one signed) is the maintainer's
/// to fix, and an N7 entry that is no longer needed the maintainer's to
/// remove, so only the report lists them.
#[must_use]
pub fn render(snapshot_at: DateTime<Utc>, defects: &DefectsPart) -> String {
    let mut sheet = format!(
        "{TITLE}\n\n{INTRO_START}{}{INTRO_END}\n\n",
        format_instant(snapshot_at)
    );
    let mut checked = defects.checked().to_vec();
    checked.sort_unstable();
    sheet.push_str(match (checked.is_empty(), defects.is_complete()) {
        (false, true) => COMPLETE,
        (false, false) => INCOMPLETE,
        (true, true) => NO_DETECTOR,
        (true, false) => STOPPED_FIRST,
    });
    sheet.push('\n');
    for defect in checked {
        // Writing into a `String` cannot fail.
        let _ = write!(sheet, "\n## {defect}: {}\n\n", defect.title());
        let instances: Vec<&DefectRecord> = defects
            .records()
            .iter()
            .filter(|record| record.defect == defect && on_sheet(record))
            .collect();
        if instances.is_empty() {
            sheet.push_str(NO_INSTANCE);
            sheet.push('\n');
            continue;
        }
        sheet.push_str(TABLE_HEAD);
        for record in instances {
            let _ = writeln!(
                sheet,
                "| {} | {} | {} |",
                escape_cell(&rows_cell(record)),
                escape_cell(&paths_cell(record)),
                escape_cell(&proposed_cell(record))
            );
        }
    }
    sheet
}

/// Whether `record` gets a row on the sheet: every instance, and every row
/// fixed in the source as signed; not a stale entry, nor an N7 entry that
/// is no longer needed.
fn on_sheet(record: &DefectRecord) -> bool {
    !matches!(
        record.outcome,
        DefectOutcome::Stale | DefectOutcome::EntryUnused
    )
}

/// The row ids of an instance, `12, 13`, or [`NOTHING`].
fn rows_cell(record: &DefectRecord) -> String {
    if record.rows.is_empty() {
        NOTHING.to_owned()
    } else {
        id_list(&record.rows)
    }
}

/// The public paths of an instance, `/posts/12, /posts/13`, or
/// [`NOTHING`].
fn paths_cell(record: &DefectRecord) -> String {
    if record.paths.is_empty() {
        NOTHING.to_owned()
    } else {
        record
            .paths
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The resolution the import proposes for an instance (the entry's
/// resolution or the defect's default, [`DefectRecord::proposed`]), or a
/// fixed phrase when it keeps the source value.
fn proposed_cell(record: &DefectRecord) -> String {
    match (&record.proposed, record.outcome) {
        (_, DefectOutcome::Unchanged) => IMPORTED_UNCHANGED.to_owned(),
        (_, DefectOutcome::FixedInSource) => FIXED_IN_SOURCE.to_owned(),
        (Some(proposed), _) => proposed.to_string(),
        (None, _) => EDITORS_DECIDE.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use chrono::NaiveDate;
    use proptest::prelude::*;

    use super::*;
    use crate::domain::import_mysql::{
        defects::{
            DefectDetector, DefectId, DefectRule, Instance, PageSlug, Proposal, PublicPath,
            Subject, Value, Word,
            testing::{N7Body, UnplannedRemovals, any_part},
        },
        options::Mode,
        resolutions::{
            BodyRow, FileDigest, PostAuthor, PostIndexImage, ResolutionFile, is_index_image_path,
            parse,
        },
    };

    fn snapshot() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-23T17:01:02Z")
            .expect("RFC 3339")
            .with_timezone(&Utc)
    }

    fn id(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).expect("a row id")
    }

    /// A legacy post as the D2 rule reads it, with the content and the
    /// names a real row holds: none of them may reach the sheet.
    struct Post {
        id: u32,
        title: &'static str,
        slug_of_a_page: &'static str,
        author: Option<&'static str>,
    }

    /// A D2 rule that reads posts with titles and author names.
    struct MissingAuthor;

    impl DefectRule for MissingAuthor {
        type Row = Post;
        type Entry = PostAuthor;
        type Resolution = ();
        const DEFECT: DefectId = DefectId::D2;

        fn detect(&self, rows: &[Post]) -> Vec<Instance<()>> {
            rows.iter()
                .filter(|row| row.author.is_none())
                .map(|row| {
                    // The rule sees the title, but an instance has no place
                    // for it.
                    assert_ne!(row.title, "");
                    let mut paths = vec![PublicPath::Post(id(row.id))];
                    paths.extend(PageSlug::new(row.slug_of_a_page).map(PublicPath::Page));
                    Instance::new(Subject::Post(id(row.id)), Value::Null)
                        .with_paths(paths)
                        .with_proposed(Proposal {
                            resolution: (),
                            shown: Value::Choice(Word::Generic),
                        })
                })
                .collect()
        }

        fn found(&self, _rows: &[Post], _subject: &Subject) -> Option<Value> {
            None
        }

        fn resolution(&self, entry: &PostAuthor) -> Proposal<()> {
            let _ = entry;
            Proposal {
                resolution: (),
                shown: Value::Id(id(7)),
            }
        }
    }

    fn rows() -> Vec<Post> {
        vec![
            Post {
                id: 55,
                title: "Synthetic title of post 55",
                slug_of_a_page: "felveteli",
                author: None,
            },
            Post {
                id: 61,
                title: "Another synthetic title",
                slug_of_a_page: "Not A Slug",
                author: None,
            },
            Post {
                id: 62,
                title: "A post that has its author",
                slug_of_a_page: "rolunk",
                author: Some("Próba Szerző"),
            },
        ]
    }

    #[test]
    fn the_sheet_lists_each_instance_with_rows_paths_and_proposed_resolution() {
        let signed = parse(
            "version = 1\n[[post_author]]\npost_id = 55\nexpect_author_id = \"none\"\nset_author \
             = 7\n",
        )
        .expect("a valid synthetic file");
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::DryRun, &signed)
            .run(&MissingAuthor, &rows(), &mut part)
            .expect("a dry run goes on");
        let _ = part.conclude();
        let sheet = render(snapshot(), &part);
        assert_eq!(
            sheet,
            "# Defect sign-off sheet\n\nThe defect instances that the import found in the source \
             snapshot of 2026-10-23T17:01:02Z, with the legacy row ids, the pages to open on the \
             live site and the resolution that the import proposes (data-migration.md §8.1, \
             §8.2). The sheet holds no names and no content. The editors confirm each instance \
             or say what to change; the maintainer records who signed and when in the private \
             runbook only.\n\nThe run checked every defect that this build detects.\n\n## D2: \
             Published posts without an author\n\n| Rows | Open on the live site | Proposed \
             resolution |\n|---|---|---|\n| 55 | /posts/55, /information/felveteli | 7 |\n| 61 \
             | /posts/61 | generic |\n"
        );
        for row in rows() {
            assert!(!sheet.contains(row.title), "{sheet}");
        }
        assert!(!sheet.contains("Próba"), "{sheet}");
        assert!(!sheet.contains("Not A Slug"), "{sheet}");
    }

    #[test]
    fn the_sheet_says_how_far_the_run_got() {
        let empty = ResolutionFile::default();
        let mut part = DefectsPart::default();
        assert!(render(snapshot(), &part).ends_with(&format!("\n\n{STOPPED_FIRST}\n")));
        part.conclude().expect("no problem");
        assert!(render(snapshot(), &part).ends_with(&format!("\n\n{NO_DETECTOR}\n")));

        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::Apply, &empty)
            .run(&MissingAuthor, &[], &mut part)
            .expect("no instance");
        let sheet = render(snapshot(), &part);
        assert!(
            sheet.ends_with(&format!(
                "\n\n{INCOMPLETE}\n\n## D2: Published posts without an author\n\n{NO_INSTANCE}\n"
            )),
            "{sheet}"
        );
        part.conclude().expect("no problem");
        assert!(render(snapshot(), &part).contains(&format!("\n\n{COMPLETE}\n\n## D2: ")));
    }

    #[test]
    fn an_instance_without_a_default_and_an_unchanged_one_say_so() {
        // The synthetic part of the shared strategy covers both; here they
        // are named.
        struct NoDefault;
        impl DefectRule for NoDefault {
            type Row = Post;
            type Entry = PostAuthor;
            type Resolution = ();
            const DEFECT: DefectId = DefectId::D2;
            fn detect(&self, rows: &[Post]) -> Vec<Instance<()>> {
                MissingAuthor
                    .detect(rows)
                    .into_iter()
                    .map(|instance| Instance {
                        proposed: None,
                        ..instance
                    })
                    .collect()
            }
            fn found(&self, _rows: &[Post], _subject: &Subject) -> Option<Value> {
                None
            }
            fn resolution(&self, entry: &PostAuthor) -> Proposal<()> {
                MissingAuthor.resolution(entry)
            }
        }
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::DryRun, &ResolutionFile::default())
            .run(&NoDefault, &rows(), &mut part)
            .expect("a dry run goes on");
        let sheet = render(snapshot(), &part);
        assert!(
            sheet.contains(&format!("| 61 | /posts/61 | {EDITORS_DECIDE} |\n")),
            "{sheet}"
        );
    }

    #[test]
    fn a_row_fixed_in_the_source_is_listed_and_a_stale_entry_is_not() {
        // D3: post 70 got its index image in the current admin, as its
        // entry pins; post 71's entry pins an image the post no longer has.
        struct MissingIndexImage;
        impl DefectRule for MissingIndexImage {
            type Row = (u32, Option<&'static str>);
            type Entry = PostIndexImage;
            type Resolution = ();
            const DEFECT: DefectId = DefectId::D3;
            fn detect(&self, _rows: &[Self::Row]) -> Vec<Instance<()>> {
                Vec::new()
            }
            fn found(&self, rows: &[Self::Row], subject: &Subject) -> Option<Value> {
                let Subject::Post(post) = subject else {
                    return None;
                };
                rows.iter()
                    .find(|(id, _)| *id == post.get())
                    .map(|(_, image)| {
                        image
                            .and_then(crate::domain::import_mysql::defects::IndexImage::new)
                            .map_or(Value::Null, Value::IndexImage)
                    })
            }
            fn resolution(&self, _entry: &PostIndexImage) -> Proposal<()> {
                Proposal {
                    resolution: (),
                    shown: Value::Null,
                }
            }
        }
        let image = "index/0123456789abcdef0123456789abcdef.jpg";
        let signed = parse(&format!(
            "version = 1\n[[post_index_image]]\npost_id = 70\nexpect_index_image = \
             \"{image}\"\n[[post_index_image]]\npost_id = 71\nexpect_index_image = \"{image}\"\n"
        ))
        .expect("a valid synthetic file");
        let rows = [(70, Some(image)), (71, None)];
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::DryRun, &signed)
            .run(&MissingIndexImage, &rows, &mut part)
            .expect("a dry run goes on");
        let sheet = render(snapshot(), &part);
        assert!(
            sheet.ends_with(&format!(
                "## D3: Published posts without an index image\n\n{TABLE_HEAD}| 70 | /posts/70 | \
                 {FIXED_IN_SOURCE} |\n"
            )),
            "{sheet}"
        );
        assert!(!sheet.contains("| 71 |"), "{sheet}");
    }

    #[test]
    fn an_n7_entry_no_longer_needed_is_not_on_the_sheet() {
        // Post 91's body is the one its entry was signed for, but N7 no
        // longer removes anything unplanned from it: the entry is no longer
        // needed (research: DECISIONS R38 (D161)), which asks nothing of
        // the editors. Post 92's body still has an unplanned removal that
        // its entry accepts, so the editors see it.
        let rows = [
            N7Body {
                row: BodyRow::Post(id(91)),
                source: "<p>91</p>",
                unplanned: false,
            },
            N7Body {
                row: BodyRow::Post(id(92)),
                source: "<p>92</p>",
                unplanned: true,
            },
        ];
        let signed = parse(&format!(
            "version = 1\n[[N7-accept]]\npost_id = 91\nexpect_content_sha256 = \"{}\"\n\
             [[N7-accept]]\npost_id = 92\nexpect_content_sha256 = \"{}\"\n",
            FileDigest::of(rows[0].source.as_bytes()),
            FileDigest::of(rows[1].source.as_bytes())
        ))
        .expect("a valid synthetic file");
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::Apply, &signed)
            .run(&UnplannedRemovals, &rows, &mut part)
            .expect("neither entry is a problem");
        assert_eq!(
            part.records()
                .iter()
                .map(|record| record.outcome)
                .collect::<Vec<_>>(),
            [DefectOutcome::EntryUnused, DefectOutcome::Covered]
        );
        let sheet = render(snapshot(), &part);
        assert!(
            sheet.ends_with(&format!(
                "## N7: Unplanned sanitiser removals in a body\n\n{TABLE_HEAD}| 92 | /posts/92 | \
                 accepted |\n"
            )),
            "{sheet}"
        );
        assert!(!sheet.contains("| 91 |"), "{sheet}");
    }

    /// Whether `cell` is a list of legacy ids, `12` or `12, 13`.
    fn is_id_list(cell: &str) -> bool {
        cell.split(", ").all(|id| {
            !id.is_empty()
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && id.parse::<u32>().is_ok_and(|n| n > 0)
        })
    }

    /// Whether `cell` is a list of public paths.
    fn is_path_list(cell: &str) -> bool {
        cell.split(", ").all(|path| {
            path.strip_prefix("/posts/").is_some_and(is_id_list)
                || path
                    .strip_prefix("/information/")
                    .is_some_and(|slug| PageSlug::new(slug).is_some())
        })
    }

    /// Whether `cell`, unescaped, is a value as [`Value`] shows it.
    fn is_shown_value(cell: &str) -> bool {
        let text = cell.replace("\\[", "[").replace("\\]", "]");
        let hex = text.len() == 64
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        text == "none"
            || is_id_list(&text) && !text.contains(',')
            || text
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .is_some_and(|ids| ids.is_empty() || is_id_list(ids))
            || (text.len() == 10 && NaiveDate::parse_from_str(&text, "%Y-%m-%d").is_ok())
            || (text.ends_with('Z') && DateTime::parse_from_rfc3339(&text).is_ok())
            || hex
            || is_index_image_path(&text)
            || Word::ALL.iter().any(|word| word.as_str() == text)
    }

    /// Whether `line` of a sheet is one of the lines the sheet may hold.
    fn allowed(line: &str) -> bool {
        let fixed = [
            TITLE,
            "",
            COMPLETE,
            INCOMPLETE,
            NO_DETECTOR,
            STOPPED_FIRST,
            NO_INSTANCE,
            "| Rows | Open on the live site | Proposed resolution |",
            "|---|---|---|",
        ];
        if fixed.contains(&line) {
            return true;
        }
        if DefectId::ALL
            .iter()
            .any(|defect| line == format!("## {defect}: {}", defect.title()))
        {
            return true;
        }
        if let Some(instant) = line
            .strip_prefix(INTRO_START)
            .and_then(|rest| rest.strip_suffix(INTRO_END))
        {
            return instant.ends_with('Z') && DateTime::parse_from_rfc3339(instant).is_ok();
        }
        let Some(cells) = line
            .strip_prefix("| ")
            .and_then(|rest| rest.strip_suffix(" |"))
        else {
            return false;
        };
        let cells: Vec<&str> = cells.split(" | ").collect();
        let [rows, paths, proposed] = cells.as_slice() else {
            return false;
        };
        (*rows == NOTHING || is_id_list(rows))
            && (*paths == NOTHING || is_path_list(paths))
            && ([IMPORTED_UNCHANGED, EDITORS_DECIDE, FIXED_IN_SOURCE].contains(proposed)
                || is_shown_value(proposed))
    }

    #[test]
    fn the_line_check_refuses_content() {
        for line in [
            "| 55 | /posts/55 | Synthetic title |",
            "| Próba Szerző | /posts/55 | 7 |",
            "| 55 | /information/Not A Slug | 7 |",
            "| 55 | https://example.org/x | 7 |",
            "| 55 | /posts/55 | 7 | extra |",
            "Some other sentence.",
            "## D2: Something else",
        ] {
            assert!(!allowed(line), "{line}");
        }
        assert!(allowed("| 393, 398 | - | 398 |"));
        assert!(allowed("| - | - | \\[26, 29\\] |"));
    }

    proptest! {
        #[test]
        fn a_sheet_holds_only_the_allowed_fields(part in any_part(), seconds in 0_i64..4_000_000_000) {
            let snapshot_at = DateTime::from_timestamp(seconds, 0).expect("in range");
            let sheet = render(snapshot_at, &part);
            prop_assert!(sheet.ends_with('\n'));
            prop_assert!(!sheet.contains('\r'));
            for line in sheet.lines() {
                prop_assert!(allowed(line), "{:?} in\n{}", line, sheet);
            }
            // One table row per instance and per row fixed in the source,
            // none for a stale entry or an N7 entry no longer needed.
            let instances = part
                .records()
                .iter()
                .filter(|record| on_sheet(record))
                .count();
            let rows = sheet
                .lines()
                .filter(|line| line.starts_with("| ") && !line.starts_with("| Rows |"))
                .count();
            prop_assert_eq!(rows, instances);
            prop_assert_eq!(
                sheet.lines().filter(|line| line.starts_with("## ")).count(),
                part.checked().len()
            );
        }
    }
}
