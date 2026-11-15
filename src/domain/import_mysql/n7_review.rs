//! N7's review of the legacy bodies: a body from which the sanitiser
//! removed what no normaliser step planned stops the import, unless an
//! `N7-accept` entry of the resolution file accepts that row
//! (data-migration.md §7.1 N7, §8.2, §4.2; ADR 0017 §3).
//!
//! The normaliser (`domain::normalise`) is the same pure function for the
//! import, `content_normalize` and the history view, so it only reports
//! N7's unplanned removals ([`NormaliseReport::unplanned_removals`]). This
//! module is the import's decision about them, per body:
//!
//! - **No unplanned removal**: the body goes on ([`Verdict::Clean`]).
//! - **Unplanned removals, no entry**: a refusal that names the row and
//!   each removal with the sanitiser's rule ([`N7Refusal::Unplanned`]). The
//!   editors look at the body in the dry run's diff and either fix it in
//!   the current admin or sign an entry for it.
//! - **An entry for the row**: its precondition, the SHA-256 of the source
//!   `content` the editors saw, is compared first, on every run: a body
//!   that changed since the sign-off is refused whatever N7 did
//!   ([`N7Refusal::PreconditionMismatch`]), and the entry is signed again
//!   (§8.2, "the importer stops and names the entry"). With a matching
//!   precondition the unplanned removals are accepted ([`Verdict::Accepted`]);
//!   if there are none, because a later normaliser rule now plans them, the
//!   entry is no longer needed ([`Verdict::EntryUnused`]) and the run goes
//!   on.
//! - **An entry for a row that has no body** (no such row, or a NULL body,
//!   which is never normalised, D4) accepts nothing: [`N7Review::finish`]
//!   refuses it ([`N7Refusal::NoSuchBody`]).
//!
//! **The maintainer's answer.** The maintainer confirmed these verdicts as
//! built on 2026-11-15 (research: DECISIONS R38 (D161)), the entry that is
//! no longer needed included. They are N7's own entry policy in the
//! import's generic defect detector
//! ([`EntryPolicy::Acceptance`](super::defects::EntryPolicy::Acceptance) in
//! [`super::defects`], ADR 0047 §4 as that answer corrects it), and a
//! property test checks that the detector under that policy judges every
//! body and every entry as this review does. An entry accepts whatever N7
//! removes from the bytes it was signed for: a later change to the
//! sanitiser, the normaliser or a dependency that removes more from an
//! accepted body goes through without a new review, and only the dry run's
//! per-body diff and V5(c)'s check of the text show it (data-migration.md
//! §10).
//!
//! **Dry run and apply** (data-migration.md §4.2, §11.1). [`N7Review`] follows
//! the rule the maintainer accepted for the import's defects (ADR 0047;
//! research: DECISIONS R9 (D49)): `apply` stops at the first refusal, and a
//! dry run collects every refusal, goes on, and lists them all, so that one
//! nightly report shows every row the editors must look at; the task then
//! exits non-zero ([`N7Summary::passed`]). It also adds up N7's planned
//! changes per rule over the bodies, for the run report (§7.2).
//!
//! **A body the normaliser refuses** (`NormaliseError`) is the import's
//! to report and stops `apply` on its own (D140). The caller still names
//! its row here ([`N7Review::refused`]), so that the row's entry is not
//! taken for one without a body and its precondition is still compared.
//!
//! The module holds no body text: a refusal names the row, the sanitiser's
//! rules and the SHA-256 of the source body, which the run report may show
//! and the resolution file may hold (§8.2).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::options::Mode;
use super::resolutions::{BodyRow, FileDigest, N7Accept};
use crate::domain::normalise::{NormaliseReport, Rule, Step, UnplannedRemoval};

/// What the review decided for a body that may be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// N7 removed nothing that no step planned, and no entry names the row.
    Clean,
    /// N7's unplanned removals are accepted by the row's `N7-accept` entry,
    /// whose precondition matches.
    Accepted,
    /// The row's entry matches, but N7 removed nothing unplanned: a later
    /// normaliser rule now plans what the editors accepted, so the entry can
    /// go at the next sign-off.
    EntryUnused,
}

/// Why the review stops the import on a body or an entry. Each names the
/// defect id `N7` and the row, as data-migration.md §4.2 asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum N7Refusal {
    /// N7 removed what no step planned, and no entry accepts the row.
    Unplanned {
        /// The row whose body it is.
        row: BodyRow,
        /// The SHA-256 of the source body, the precondition an entry for
        /// it would carry.
        found: FileDigest,
        /// Each unplanned removal with how many times N7 made it.
        removals: Vec<(UnplannedRemoval, usize)>,
    },
    /// The row's entry was signed for other bytes: the body changed since
    /// the sign-off.
    PreconditionMismatch {
        /// The row the entry names.
        row: BodyRow,
        /// The entry's precondition.
        expected: FileDigest,
        /// The SHA-256 of the source body now.
        found: FileDigest,
        /// What N7 removes unplanned from the body now, each with how many
        /// times, for the editors' new review; `None` when the normaliser
        /// refused the body, which the import reports on its own (D140).
        removals: Option<Vec<(UnplannedRemoval, usize)>>,
    },
    /// The row's entry names no body the run reviewed: the source has no
    /// such row, or its body is NULL.
    NoSuchBody {
        /// The row the entry names.
        row: BodyRow,
        /// The entry's precondition.
        expected: FileDigest,
    },
}

/// The message names the row, the rules or the two digests, and what to
/// do, so that an operator can act on the run report alone.
impl fmt::Display for N7Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unplanned {
                row,
                found,
                removals,
            } => write!(
                f,
                "N7, {}: the sanitiser removed what no normaliser step planned: {}; check the dry \
                 run's diff of the body, then fix it in the current admin or accept it with an \
                 N7-accept entry ({} = {}, expect_content_sha256 = \"{found}\")",
                row_name(*row),
                removals_text(removals),
                row_field(*row),
                row_id(*row)
            ),
            Self::PreconditionMismatch {
                row,
                expected,
                found,
                removals,
            } => {
                write!(
                    f,
                    "N7-accept for {} (N7): expect_content_sha256 is {expected}, but the source \
                     body's SHA-256 is {found}: the body changed since the sign-off; ",
                    row_name(*row)
                )?;
                match removals.as_deref() {
                    Some([]) => f.write_str(
                        "N7 now removes nothing that no normaliser step planned from it, so remove \
                         the entry",
                    ),
                    Some(removals) => write!(
                        f,
                        "review it again and sign the entry again (N7 now removes: {})",
                        removals_text(removals)
                    ),
                    None => f.write_str(
                        "the normaliser refuses the body now (reported on its own); fix the body, \
                         then review it again and sign the entry again",
                    ),
                }
            }
            Self::NoSuchBody { row, expected } => write!(
                f,
                "N7-accept for {} (N7): expect_content_sha256 is {expected}, but the source has no \
                 body for this row (no such row, or its content is NULL), so the entry accepts \
                 nothing: remove it",
                row_name(*row)
            ),
        }
    }
}

impl std::error::Error for N7Refusal {}

/// Reviews one body against the `N7-accept` entries (see the module
/// documentation).
///
/// - `row`: the row whose body it is.
/// - `source`: the row's source `content`, as the source session read it;
///   its SHA-256 is compared with an entry's precondition.
/// - `report`: the normaliser's report for that body.
/// - `accepts`: the resolution file's `N7-accept` entries, which name each
///   row at most once (`resolutions::parse` refuses a second one).
///
/// # Errors
///
/// [`N7Refusal::PreconditionMismatch`] when the row's entry was signed for
/// other bytes, and [`N7Refusal::Unplanned`] when N7 removed what no step
/// planned and no entry names the row.
pub fn review_body(
    row: BodyRow,
    source: &str,
    report: &NormaliseReport,
    accepts: &[N7Accept],
) -> Result<Verdict, N7Refusal> {
    let found = FileDigest::of(source.as_bytes());
    let unplanned = report.has_unplanned_removals();
    match accepts.iter().find(|entry| entry.body == row) {
        Some(entry) if entry.expect_content_sha256 != found.0 => {
            Err(N7Refusal::PreconditionMismatch {
                row,
                expected: FileDigest(entry.expect_content_sha256),
                found,
                removals: Some(removals(report)),
            })
        }
        Some(_) if unplanned => Ok(Verdict::Accepted),
        Some(_) => Ok(Verdict::EntryUnused),
        None if unplanned => Err(N7Refusal::Unplanned {
            row,
            found,
            removals: removals(report),
        }),
        None => Ok(Verdict::Clean),
    }
}

/// The review of every body of one import run, in its mode (see "Dry run
/// and apply" in the module documentation).
#[derive(Debug)]
pub struct N7Review<'a> {
    /// The run's mode.
    mode: Mode,
    /// The resolution file's `N7-accept` entries.
    accepts: &'a [N7Accept],
    /// The rows reviewed so far.
    reviewed: BTreeSet<BodyRow>,
    /// The refusals a dry run collected.
    refusals: Vec<N7Refusal>,
    /// The rows whose unplanned removals an entry accepted.
    accepted: Vec<BodyRow>,
    /// The rows whose entry is no longer needed.
    unused: Vec<BodyRow>,
    /// N7's planned changes, per rule, over every body reviewed.
    planned: BTreeMap<Rule, usize>,
}

/// What the review of a whole run found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct N7Summary {
    /// Every refusal, in the order the bodies came, then the entries
    /// without a body; empty after an `apply`, which stops at the first.
    pub refusals: Vec<N7Refusal>,
    /// The rows whose unplanned removals an entry accepted.
    pub accepted: Vec<BodyRow>,
    /// The rows whose entry matched but was no longer needed.
    pub unused_entries: Vec<BodyRow>,
    /// N7's planned changes per rule, added up over every body (§7.2).
    pub planned: BTreeMap<Rule, usize>,
}

impl N7Summary {
    /// Whether the run may succeed as far as N7 is concerned: no refusal.
    /// A dry run with refusals exits non-zero (ADR 0047 §2).
    #[must_use]
    pub fn passed(&self) -> bool {
        self.refusals.is_empty()
    }
}

impl<'a> N7Review<'a> {
    /// A review for one run.
    ///
    /// - `mode`: the run's mode: `apply` stops at the first refusal, a dry
    ///   run collects them.
    /// - `accepts`: the validated resolution file's `N7-accept` entries.
    #[must_use]
    pub fn new(mode: Mode, accepts: &'a [N7Accept]) -> Self {
        Self {
            mode,
            accepts,
            reviewed: BTreeSet::new(),
            refusals: Vec::new(),
            accepted: Vec::new(),
            unused: Vec::new(),
            planned: BTreeMap::new(),
        }
    }

    /// Reviews one body ([`review_body`]) and counts its planned N7
    /// changes. The caller passes every body it normalised, once each, a
    /// body the normaliser refused to [`Self::refused`] instead, and none for
    /// a NULL body.
    ///
    /// - `row`: the row whose body it is.
    /// - `source`: the row's source `content`.
    /// - `report`: the normaliser's report for that body.
    ///
    /// # Errors
    ///
    /// In `apply`, the body's refusal: the import stops and rolls back. A
    /// dry run records the refusal and returns `Ok`.
    pub fn body(
        &mut self,
        row: BodyRow,
        source: &str,
        report: &NormaliseReport,
    ) -> Result<(), N7Refusal> {
        self.reviewed.insert(row);
        for (rule, count) in report.hits().filter(|(rule, _)| rule.step() == Step::N7) {
            *self.planned.entry(rule).or_insert(0) += count;
        }
        match review_body(row, source, report, self.accepts) {
            Ok(Verdict::Clean) => {}
            Ok(Verdict::Accepted) => self.accepted.push(row),
            Ok(Verdict::EntryUnused) => self.unused.push(row),
            Err(refusal) => self.refuse(refusal)?,
        }
        Ok(())
    }

    /// Records a row whose body the normaliser refused (`NormaliseError`):
    /// the import reports that refusal on its own and stops `apply` on it
    /// (D140). The row counts as reviewed, so that [`Self::finish`] does not
    /// take its entry for one without a body, and the entry's precondition
    /// is still compared: a body that changed since the sign-off is named
    /// as such, whatever else happened to it. The caller passes each such
    /// row once, instead of [`Self::body`].
    ///
    /// - `row`: the row whose body the normaliser refused.
    /// - `source`: the row's source `content`.
    ///
    /// # Errors
    ///
    /// In `apply`, [`N7Refusal::PreconditionMismatch`] when the row's entry
    /// was signed for other bytes. A dry run records it and returns `Ok`.
    pub fn refused(&mut self, row: BodyRow, source: &str) -> Result<(), N7Refusal> {
        self.reviewed.insert(row);
        let found = FileDigest::of(source.as_bytes());
        match self.accepts.iter().find(|entry| entry.body == row) {
            Some(entry) if entry.expect_content_sha256 != found.0 => {
                self.refuse(N7Refusal::PreconditionMismatch {
                    row,
                    expected: FileDigest(entry.expect_content_sha256),
                    found,
                    removals: None,
                })
            }
            _ => Ok(()),
        }
    }

    /// Ends the review: an entry whose row had no body is refused, then the
    /// summary is returned.
    ///
    /// # Errors
    ///
    /// In `apply`, the first entry without a body. A dry run lists them in
    /// [`N7Summary::refusals`].
    pub fn finish(mut self) -> Result<N7Summary, N7Refusal> {
        for entry in self.accepts {
            if !self.reviewed.contains(&entry.body) {
                self.refuse(N7Refusal::NoSuchBody {
                    row: entry.body,
                    expected: FileDigest(entry.expect_content_sha256),
                })?;
            }
        }
        Ok(N7Summary {
            refusals: self.refusals,
            accepted: self.accepted,
            unused_entries: self.unused,
            planned: self.planned,
        })
    }

    /// Stops on `refusal` in `apply`, records it in a dry run.
    fn refuse(&mut self, refusal: N7Refusal) -> Result<(), N7Refusal> {
        match self.mode {
            Mode::Apply => Err(refusal),
            Mode::DryRun => {
                self.refusals.push(refusal);
                Ok(())
            }
        }
    }
}

/// `post 91` or `page 50`, as the run report names a row.
fn row_name(row: BodyRow) -> String {
    format!("{} {}", row_kind(row), row_id(row))
}

/// The kind of row: `post` or `page`.
const fn row_kind(row: BodyRow) -> &'static str {
    match row {
        BodyRow::Post(_) => "post",
        BodyRow::Page(_) => "page",
    }
}

/// The row's legacy id.
const fn row_id(row: BodyRow) -> u32 {
    match row {
        BodyRow::Post(id) | BodyRow::Page(id) => id.get(),
    }
}

/// The field of an `N7-accept` entry that names the row: `post_id` or
/// `page_id`.
const fn row_field(row: BodyRow) -> &'static str {
    match row {
        BodyRow::Post(_) => "post_id",
        BodyRow::Page(_) => "page_id",
    }
}

/// The unplanned removals of `report`, each with how many times N7 made
/// them, in the report's order.
fn removals(report: &NormaliseReport) -> Vec<(UnplannedRemoval, usize)> {
    report
        .unplanned_removals()
        .map(|(removal, count)| (removal.clone(), count))
        .collect()
}

/// The unplanned removals of a refusal, `rule element ×count`, joined with
/// commas.
fn removals_text(removals: &[(UnplannedRemoval, usize)]) -> String {
    removals
        .iter()
        .map(|(removal, count)| format!("{removal} ×{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for Verdict {
    /// `clean`, `accepted` or `entry_unused`, as the run report writes it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Clean => "clean",
            Self::Accepted => "accepted",
            Self::EntryUnused => "entry_unused",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::import_mysql::defects::{
        DefectDetector, DefectOutcome, DefectsPart, Subject,
        testing::{N7Body, UnplannedRemovals},
    };
    use crate::domain::import_mysql::resolutions::{ResolutionFile, parse};
    use crate::domain::normalise::{Normalised, normalise};
    use crate::domain::rich_text::{SanitizePolicy, Sanitizer};
    use proptest::prelude::*;
    use std::num::NonZeroU32;
    use std::sync::LazyLock;

    static SANITIZER: LazyLock<Sanitizer> = LazyLock::new(|| {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
    });

    /// A legacy body whose N7 changes are all planned: a table border, a
    /// black cell border, and the locked styles of the table and its
    /// `tbody`.
    const PLANNED: &str = r#"<table border="1"><tbody><tr><td style="border-top: 1px solid black">x</td></tr></tbody></table>"#;
    /// A legacy body with an unplanned removal: a `div` and a class.
    const UNPLANNED: &str = r#"<div class="doboz"><p>Szöveg</p></div>"#;

    fn normalised(body: &str) -> Normalised {
        normalise(body, &SANITIZER).expect("a small body settles")
    }

    fn post(id: u32) -> BodyRow {
        BodyRow::Post(NonZeroU32::new(id).expect("a row id"))
    }

    fn page(id: u32) -> BodyRow {
        BodyRow::Page(NonZeroU32::new(id).expect("a row id"))
    }

    fn accept(row: BodyRow, source: &str) -> N7Accept {
        N7Accept {
            body: row,
            expect_content_sha256: FileDigest::of(source.as_bytes()).0,
        }
    }

    #[test]
    fn a_body_with_only_planned_changes_passes() {
        let out = normalised(PLANNED);
        assert_eq!(
            review_body(post(1), PLANNED, &out.report, &[]),
            Ok(Verdict::Clean)
        );
    }

    #[test]
    fn an_unplanned_removal_is_refused_naming_row_and_rule() {
        let out = normalised(UNPLANNED);
        let refusal = review_body(post(91), UNPLANNED, &out.report, &[])
            .expect_err("an unplanned removal without an entry");
        let found = FileDigest::of(UNPLANNED.as_bytes());
        assert_eq!(
            refusal.to_string(),
            format!(
                "N7, post 91: the sanitiser removed what no normaliser step planned: \
                 element_unwrapped div ×1; check the dry run's diff of the body, then fix it in \
                 the current admin or accept it with an N7-accept entry (post_id = 91, \
                 expect_content_sha256 = \"{found}\")"
            )
        );
        let N7Refusal::Unplanned { row, removals, .. } = refusal else {
            panic!("{refusal:?}");
        };
        assert_eq!(row, post(91));
        assert_eq!(removals.len(), 1);
        let page_refusal = review_body(page(50), UNPLANNED, &out.report, &[])
            .expect_err("pages are reviewed alike");
        assert!(
            page_refusal.to_string().starts_with("N7, page 50: ")
                && page_refusal.to_string().contains("(page_id = 50, "),
            "{page_refusal}"
        );
    }

    #[test]
    fn an_accepted_body_passes() {
        let out = normalised(UNPLANNED);
        let accepts = [accept(post(7), "<p>más</p>"), accept(post(91), UNPLANNED)];
        assert_eq!(
            review_body(post(91), UNPLANNED, &out.report, &accepts),
            Ok(Verdict::Accepted)
        );
        // An entry for another row does not cover this one.
        assert!(review_body(post(8), UNPLANNED, &out.report, &accepts).is_err());
        // Nor does an entry for the page with the same id.
        assert!(review_body(page(91), UNPLANNED, &out.report, &accepts).is_err());
    }

    #[test]
    fn a_precondition_mismatch_is_refused_whatever_n7_did() {
        // The refusal names what N7 removes now, so that the editors know
        // what to look at, or that the entry can go.
        let accepts = [accept(post(91), "<div>the body the editors saw</div>")];
        for (body, now) in [
            (
                UNPLANNED,
                "review it again and sign the entry again (N7 now removes: element_unwrapped div \
                 ×1)",
            ),
            (
                PLANNED,
                "N7 now removes nothing that no normaliser step planned from it, so remove the \
                 entry",
            ),
        ] {
            let out = normalised(body);
            let refusal = review_body(post(91), body, &out.report, &accepts)
                .expect_err("the body changed since the sign-off");
            let expected = FileDigest::of("<div>the body the editors saw</div>".as_bytes());
            let found = FileDigest::of(body.as_bytes());
            assert_eq!(
                refusal,
                N7Refusal::PreconditionMismatch {
                    row: post(91),
                    expected,
                    found,
                    removals: Some(removals(&out.report)),
                }
            );
            assert_eq!(
                refusal.to_string(),
                format!(
                    "N7-accept for post 91 (N7): expect_content_sha256 is {expected}, but the \
                     source body's SHA-256 is {found}: the body changed since the sign-off; {now}"
                )
            );
        }
    }

    #[test]
    fn a_row_the_normaliser_refused_keeps_its_entry_and_its_precondition() {
        // A signed row whose source was later edited past the sanitiser's
        // depth limit: the normaliser refuses it, and the import reports
        // that on its own. The review must neither call the entry one
        // without a body nor skip its precondition.
        let deep = format!("{}x", "<div>".repeat(300));
        assert!(normalise(&deep, &SANITIZER).is_err(), "the body is refused");
        let signed = [accept(post(5), &deep)];
        let mut dry = N7Review::new(Mode::DryRun, &signed);
        dry.refused(post(5), &deep).expect("a dry run goes on");
        let summary = dry.finish().expect("a dry run goes on");
        assert!(summary.passed(), "{:?}", summary.refusals);

        let stale = [accept(post(5), "<p>the body the editors saw</p>")];
        let mut dry = N7Review::new(Mode::DryRun, &stale);
        dry.refused(post(5), &deep).expect("a dry run goes on");
        let summary = dry.finish().expect("a dry run goes on");
        let [refusal] = &summary.refusals[..] else {
            panic!("{:?}", summary.refusals);
        };
        assert!(
            matches!(refusal, N7Refusal::PreconditionMismatch { row, removals: None, .. } if *row == post(5)),
            "{refusal:?}"
        );
        assert!(
            refusal
                .to_string()
                .ends_with("the normaliser refuses the body now (reported on its own); fix the body, then review it again and sign the entry again"),
            "{refusal}"
        );
        let mut apply = N7Review::new(Mode::Apply, &stale);
        assert_eq!(apply.refused(post(5), &deep), Err(refusal.clone()));
        // Without an entry, there is nothing to compare.
        let mut apply = N7Review::new(Mode::Apply, &[]);
        assert_eq!(apply.refused(post(5), &deep), Ok(()));
        assert!(apply.finish().expect("nothing to refuse").passed());
    }

    #[test]
    fn only_n7_rules_are_added_up_for_the_report() {
        // A body with N3 and N5 hits beside N7's: the review adds up N7's
        // planned changes only, the other steps' hits are the normaliser's
        // report.
        let body = r#"<center><h6>Cím</h6></center><table border="1"><tbody><tr><td>x</td></tr></tbody></table>"#;
        let out = normalised(body);
        assert!(out.report.count(Rule::CenterUnwrapped) > 0);
        assert!(out.report.count(Rule::HeadingToH4) > 0);
        let mut review = N7Review::new(Mode::Apply, &[]);
        review
            .body(post(1), body, &out.report)
            .expect("only planned changes");
        let planned = review.finish().expect("nothing to refuse").planned;
        assert!(!planned.is_empty());
        assert!(
            planned.keys().all(|rule| rule.step() == Step::N7),
            "{planned:?}"
        );
        assert_eq!(planned.get(&Rule::TableBorderDropped), Some(&1));
    }

    #[test]
    fn an_entry_that_is_no_longer_needed_lets_the_body_pass() {
        let out = normalised(PLANNED);
        assert_eq!(
            review_body(page(3), PLANNED, &out.report, &[accept(page(3), PLANNED)]),
            Ok(Verdict::EntryUnused)
        );
    }

    #[test]
    fn verdicts_have_report_names() {
        assert_eq!(Verdict::Clean.to_string(), "clean");
        assert_eq!(Verdict::Accepted.to_string(), "accepted");
        assert_eq!(Verdict::EntryUnused.to_string(), "entry_unused");
    }

    #[test]
    fn a_dry_run_collects_every_refusal_and_apply_stops_at_the_first() {
        let unplanned = normalised(UNPLANNED);
        let planned = normalised(PLANNED);
        let accepts = [
            accept(post(2), UNPLANNED),
            accept(post(4), "<p>other bytes</p>"),
            accept(page(9), "<p>a NULL body</p>"),
        ];
        let bodies = [
            (post(1), PLANNED, &planned),
            (post(2), UNPLANNED, &unplanned),
            (post(3), UNPLANNED, &unplanned),
            (post(4), PLANNED, &planned),
        ];

        let mut dry = N7Review::new(Mode::DryRun, &accepts);
        for (row, source, out) in bodies {
            dry.body(row, source, &out.report)
                .expect("a dry run goes on");
        }
        let summary = dry.finish().expect("a dry run goes on");
        assert!(!summary.passed());
        let refused: Vec<_> = summary
            .refusals
            .iter()
            .map(|refusal| match refusal {
                N7Refusal::Unplanned { row, .. } => ("unplanned", *row),
                N7Refusal::PreconditionMismatch { row, .. } => ("mismatch", *row),
                N7Refusal::NoSuchBody { row, .. } => ("no body", *row),
            })
            .collect();
        assert_eq!(
            refused,
            [
                ("unplanned", post(3)),
                ("mismatch", post(4)),
                ("no body", page(9))
            ]
        );
        assert_eq!(summary.accepted, [post(2)]);
        assert_eq!(summary.unused_entries, []);
        assert_eq!(
            summary.planned,
            BTreeMap::from([
                (Rule::NamedColourToHex, 2),
                (Rule::LockedMarkupAdded, 4),
                (Rule::TableBorderDropped, 2),
            ]),
            "the per-rule counts of the two planned bodies, refused or not"
        );

        let mut apply = N7Review::new(Mode::Apply, &accepts);
        let mut stopped = None;
        for (row, source, out) in bodies {
            if let Err(refusal) = apply.body(row, source, &out.report) {
                stopped = Some(refusal);
                break;
            }
        }
        assert!(
            matches!(stopped, Some(N7Refusal::Unplanned { row, .. }) if row == post(3)),
            "{stopped:?}"
        );
    }

    #[test]
    fn apply_refuses_an_entry_without_a_body_at_the_end() {
        let accepts = [accept(page(9), "<p>x</p>")];
        let review = N7Review::new(Mode::Apply, &accepts);
        let refusal = review.finish().expect_err("no body for the entry");
        assert_eq!(
            refusal.to_string(),
            format!(
                "N7-accept for page 9 (N7): expect_content_sha256 is {}, but the source has no \
                 body for this row (no such row, or its content is NULL), so the entry accepts \
                 nothing: remove it",
                FileDigest::of(b"<p>x</p>")
            )
        );
        let summary = N7Review::new(Mode::Apply, &[])
            .finish()
            .expect("nothing to refuse");
        assert!(summary.passed());
        assert_eq!(summary, N7Summary::default());
    }

    #[test]
    fn an_entry_read_from_the_resolution_file_accepts_its_body() {
        // The whole path: the editors sign the SHA-256 of the source body,
        // the file is parsed and validated, and the review accepts the body.
        let digest = FileDigest::of(UNPLANNED.as_bytes());
        let file = parse(&format!(
            "version = 1\n\n[[N7-accept]]\npost_id = 91\nexpect_content_sha256 = \"{digest}\"\n"
        ))
        .expect("a valid resolution file");
        let out = normalised(UNPLANNED);
        let mut review = N7Review::new(Mode::Apply, &file.n7_accept);
        review
            .body(post(91), UNPLANNED, &out.report)
            .expect("the entry accepts the body");
        assert_eq!(
            review.finish().expect("every entry met its body").accepted,
            [post(91)]
        );
    }

    /// One body of a generated run: its row, whether N7 removed something
    /// unplanned, and whether an entry names it, signed for its bytes or
    /// for others.
    fn arb_run() -> impl Strategy<Value = Vec<(u32, bool, Option<bool>)>> {
        prop::collection::btree_map(1_u32..50, (any::<bool>(), any::<Option<bool>>()), 0..12)
            .prop_map(|bodies| {
                bodies
                    .into_iter()
                    .map(|(id, (unplanned, entry))| (id, unplanned, entry))
                    .collect()
            })
    }

    /// The row a refusal names.
    fn refused_row(refusal: &N7Refusal) -> BodyRow {
        match refusal {
            N7Refusal::Unplanned { row, .. }
            | N7Refusal::PreconditionMismatch { row, .. }
            | N7Refusal::NoSuchBody { row, .. } => *row,
        }
    }

    proptest! {
        // N7's own entry policy in the defect detector
        // (`EntryPolicy::Acceptance`) gives every body and every entry the
        // verdict this review gives, which the maintainer confirmed as
        // built (research: DECISIONS R38 (D161)): the review's refusals
        // are the detector's problems, its accepted rows the covered
        // instances and its unused entries the entries no longer needed,
        // and in `apply` both stop on the same row. Pages without a body
        // stand for entries whose row has none.
        #[test]
        fn n7s_entry_policy_in_the_detector_gives_the_reviews_verdicts(
            run in arb_run(),
            bodiless in prop::collection::btree_set(1_u32..20, 0..3),
        ) {
            let unplanned = normalised(UNPLANNED);
            let planned = normalised(PLANNED);
            let source = |has_unplanned: bool| if has_unplanned { UNPLANNED } else { PLANNED };
            let mut accepts: Vec<N7Accept> = run
                .iter()
                .filter_map(|&(id, has_unplanned, entry)| {
                    entry.map(|signed_for_these| {
                        accept(post(id), if signed_for_these { source(has_unplanned) } else { "<p>old</p>" })
                    })
                })
                .collect();
            accepts.extend(bodiless.iter().map(|&id| accept(page(id), "<p>a NULL body</p>")));
            let file = ResolutionFile {
                n7_accept: accepts.clone(),
                ..ResolutionFile::default()
            };
            let rows: Vec<N7Body> = run
                .iter()
                .map(|&(id, has_unplanned, _)| N7Body {
                    row: post(id),
                    source: source(has_unplanned),
                    unplanned: has_unplanned,
                })
                .collect();
            let report = |has_unplanned: bool| if has_unplanned { &unplanned.report } else { &planned.report };

            let mut review = N7Review::new(Mode::DryRun, &accepts);
            for body in &rows {
                review.body(body.row, body.source, report(body.unplanned)).expect("a dry run goes on");
            }
            let summary = review.finish().expect("a dry run goes on");
            let mut part = DefectsPart::default();
            DefectDetector::new(Mode::DryRun, &file)
                .run(&UnplannedRemovals, &rows, &mut part)
                .map_err(|refusal| TestCaseError::fail(refusal.to_string()))?;
            let judged = |keep: &dyn Fn(DefectOutcome) -> bool| -> Vec<Subject> {
                part.records()
                    .iter()
                    .filter(|record| keep(record.outcome))
                    .map(|record| record.subject.clone())
                    .collect()
            };
            let subjects = |rows: &[BodyRow]| -> Vec<Subject> { rows.iter().copied().map(Subject::from).collect() };
            let refused: Vec<BodyRow> = summary.refusals.iter().map(refused_row).collect();
            prop_assert_eq!(judged(&DefectOutcome::is_problem), subjects(&refused));
            prop_assert_eq!(
                judged(&|outcome| outcome == DefectOutcome::Covered),
                subjects(&summary.accepted)
            );
            prop_assert_eq!(
                judged(&|outcome| outcome == DefectOutcome::EntryUnused),
                subjects(&summary.unused_entries)
            );

            let mut apply = N7Review::new(Mode::Apply, &accepts);
            let mut review_stop = None;
            for body in &rows {
                if let Err(refusal) = apply.body(body.row, body.source, report(body.unplanned)) {
                    review_stop = Some(refused_row(&refusal));
                    break;
                }
            }
            if review_stop.is_none()
                && let Err(refusal) = apply.finish()
            {
                review_stop = Some(refused_row(&refusal));
            }
            let mut applied = DefectsPart::default();
            let detector_stop = DefectDetector::new(Mode::Apply, &file)
                .run(&UnplannedRemovals, &rows, &mut applied)
                .err()
                .and_then(|_| applied.records().last().map(|record| record.subject.clone()));
            prop_assert_eq!(detector_stop, review_stop.map(Subject::from));
        }

        // Every refusal names the defect id and the row as §4.2 asks, and
        // the digests it compares, for any row and any bytes.
        #[test]
        fn every_refusal_names_n7_and_its_row(
            id in 1_u32..=u32::MAX,
            is_page in any::<bool>(),
            expected in any::<[u8; 32]>(),
            source in "\\PC{0,40}",
        ) {
            let row = if is_page { page(id) } else { post(id) };
            let name = format!("{} {id}", if is_page { "page" } else { "post" });
            let found = FileDigest::of(source.as_bytes());
            let refusals = [
                N7Refusal::Unplanned {
                    row,
                    found,
                    removals: normalised(UNPLANNED).report.unplanned_removals()
                        .map(|(removal, count)| (removal.clone(), count))
                        .collect(),
                },
                N7Refusal::PreconditionMismatch { row, expected: FileDigest(expected), found, removals: None },
                N7Refusal::PreconditionMismatch { row, expected: FileDigest(expected), found, removals: Some(Vec::new()) },
                N7Refusal::NoSuchBody { row, expected: FileDigest(expected) },
            ];
            for refusal in refusals {
                let text = refusal.to_string();
                prop_assert!(text.contains("N7"), "{}", text);
                prop_assert!(text.contains(&format!("{name}:")) || text.contains(&format!("{name} (N7)")), "{}", text);
                match refusal {
                    N7Refusal::Unplanned { .. } => prop_assert!(text.contains(&found.to_string())),
                    N7Refusal::PreconditionMismatch { .. } => {
                        prop_assert!(text.contains(&found.to_string()));
                        prop_assert!(text.contains(&FileDigest(expected).to_string()));
                    }
                    N7Refusal::NoSuchBody { .. } => {
                        prop_assert!(text.contains(&FileDigest(expected).to_string()));
                    }
                }
            }
        }

        // The verdict follows the precondition first, then the removals,
        // and a dry run refuses exactly the bodies whose `apply` would stop
        // on them, of which `apply` stops at the first.
        #[test]
        fn the_modes_refuse_the_same_bodies(run in arb_run()) {
            let unplanned = normalised(UNPLANNED);
            let planned = normalised(PLANNED);
            let source = |has_unplanned: bool| if has_unplanned { UNPLANNED } else { PLANNED };
            let accepts: Vec<_> = run
                .iter()
                .filter_map(|&(id, has_unplanned, entry)| {
                    entry.map(|signed_for_these| {
                        accept(post(id), if signed_for_these { source(has_unplanned) } else { "<p>old</p>" })
                    })
                })
                .collect();
            let expected: Vec<_> = run
                .iter()
                .filter(|&&(_, has_unplanned, entry)| match entry {
                    Some(signed_for_these) => !signed_for_these,
                    None => has_unplanned,
                })
                .map(|&(id, _, _)| post(id))
                .collect();

            let mut dry = N7Review::new(Mode::DryRun, &accepts);
            let mut apply = N7Review::new(Mode::Apply, &accepts);
            let mut first_stop = None;
            for &(id, has_unplanned, _) in &run {
                let out = if has_unplanned { &unplanned } else { &planned };
                dry.body(post(id), source(has_unplanned), &out.report).expect("a dry run goes on");
                if first_stop.is_none()
                    && let Err(refusal) = apply.body(post(id), source(has_unplanned), &out.report)
                {
                    first_stop = Some(refusal);
                }
            }
            let summary = dry.finish().expect("a dry run goes on");
            let refused: Vec<_> = summary
                .refusals
                .iter()
                .map(|refusal| match refusal {
                    N7Refusal::Unplanned { row, .. }
                    | N7Refusal::PreconditionMismatch { row, .. }
                    | N7Refusal::NoSuchBody { row, .. } => *row,
                })
                .collect();
            prop_assert_eq!(&refused, &expected);
            prop_assert_eq!(summary.passed(), expected.is_empty());
            let stopped_at = first_stop.map(|refusal| match refusal {
                N7Refusal::Unplanned { row, .. }
                | N7Refusal::PreconditionMismatch { row, .. }
                | N7Refusal::NoSuchBody { row, .. } => row,
            });
            prop_assert_eq!(stopped_at, expected.first().copied());
        }
    }
}
