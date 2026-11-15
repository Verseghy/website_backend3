//! The generic defect detector of the legacy import (ADR 0017 §3, as
//! ADR 0047 amends it; data-migration.md §4.2, §8, §11.1).
//!
//! # Why one detector for every defect
//!
//! The legacy data has defects that the import resolves from the
//! editor-signed resolution file (data-migration.md §8,
//! [`super::resolutions`]). ADR 0017 §3 asks the same three things of every
//! one of them, and [`DefectDetector`] does them once, for every defect, so
//! that no table step can leave one out:
//!
//! 1. every instance is detected from its rule on every run, never taken
//!    from the list: the list only says how a detected instance is
//!    resolved, and an instance it does not name is a problem, unless the
//!    defect's entries are optional (D10);
//! 2. an entry's precondition, the source value the editors saw when they
//!    signed, is compared with the source on every run, so a row that
//!    changed since the sign-off is a problem;
//! 3. an entry for a row that the rule no longer finds is a problem too:
//!    the row lost its defect (or the row is gone), and the entry would
//!    resolve something that is not there. The one exception is D3, whose
//!    defect the editors fix in the source before the final dump: its entry
//!    pins that fix, so it holds exactly when the rule no longer finds the
//!    post and the source has the signed value.
//!
//! The defect's [`EntryPolicy`] ([`DefectId::entry_policy`], after
//! data-migration.md §8.1) says which of these rules apply: every instance
//! needs an entry (D1, D2, D5, D6, D8, D9, N7), entries are optional (D10),
//! the entry pins a fix made in the source (D3), or the defect has no
//! entries and is only listed (D4, D7). The policy belongs to the defect,
//! not to the rule, so no table step can choose another one.
//!
//! A table step brings the rule, a [`DefectRule`]: which source rows have
//! the defect ([`DefectRule::detect`]), what the precondition reads for a
//! row that the rule does not find ([`DefectRule::found`]) and what an
//! entry resolves an instance to ([`DefectRule::resolution`]). The detector
//! matches the instances with the entries of the rule's section of the file
//! by their [`Subject`] (the post, page, date, group or menu level they are
//! about) and decides, for each instance, what the step applies
//! ([`Decisions`]). It records every instance and every entry in the run's
//! [`DefectsPart`], which the run report and the editors' sign-off sheet
//! show (data-migration.md §8.2, §11.3).
//!
//! # `apply` stops at the first problem; a dry run lists them all
//!
//! The three problems are an instance that no entry covers where one is
//! needed ([`DefectOutcome::Uncovered`]), an instance whose entry's
//! precondition does not hold ([`DefectOutcome::Changed`]) and an entry
//! that does not hold for a subject the rule no longer finds
//! ([`DefectOutcome::Stale`]). In `mode:apply` the first of them, in subject
//! order, stops the import, with a message that names the defect, the row
//! and, for an entry, the expected and the found value (data-migration.md
//! §4.2): the import never commits a resolution the editors did not sign.
//!
//! A dry run goes on instead (ADR 0047, which records the maintainer's
//! answer D49): an uncovered or changed instance takes the defect's proposed
//! default where the rule has one, and otherwise keeps its source value; a
//! stale entry changes nothing. At the end of the run,
//! [`DefectsPart::conclude`] fails it, naming every problem, so the run
//! still exits non-zero, and one nightly report shows every defect at once
//! instead of one a night. Two outcomes are no problem in either mode: an
//! instance of an optional defect that no entry lists takes the default
//! ([`DefectOutcome::Defaulted`]), and a D3 entry whose post the editors
//! fixed as signed applies nothing ([`DefectOutcome::FixedInSource`]).
//!
//! # What the report and the sign-off sheet may show
//!
//! The sign-off sheet leaves staging and goes to the editors, so it must
//! never hold a person's name or any content (titles, bodies, slugs other
//! than the page's public path), and neither may the report's Defects
//! part, which the sheet is made from (data-migration.md §8.2, §11.3). The
//! types here hold only legacy ids, calendar days, instants, SHA-256
//! digests, index image paths of the legacy shape, the closed set of words
//! of [`Word`] ([`Value`]), and the public paths `/posts/<id>` and
//! `/information/<slug>` ([`PublicPath`]). None of them can hold free text,
//! so what a rule records cannot carry a name or content into either file.
//! A rule's own resolution type stays with its step, and reaches the report
//! only through the [`Value`] it is shown as ([`Proposal::shown`]). The one
//! planned exception is D6's two menus side by side, dish names that the
//! public site already shows, which the D6 detector adds to the sheet with
//! its own column and test (ADR 0047 §3, `super::signoff`).

use std::{
    collections::{BTreeMap, BTreeSet, btree_map},
    fmt,
    num::NonZeroU32,
};

use chrono::{DateTime, NaiveDate, Utc};

use super::{
    options::{Mode, format_instant},
    resolutions::{
        BodyRow, CanteenDuplicate, FileDigest, GENERIC_AUTHOR, LabelDuplicate, MenuUnpositioned,
        N7Accept, PageHidden, PostAuthor, PostDateShift, PostIndexImage, PostSentinelDate,
        ResolutionFile, SENTINEL_DATE, Section, is_index_image_path,
    },
};

/// How many problems the text of [`DefectProblems`] shows; the rest are
/// counted. The 181 D1 rows alone would otherwise fill a log line.
const SHOWN_PROBLEMS: usize = 50;

/// The longest page slug the legacy `pages.slug` column holds
/// (`varchar(191)`, data-migration.md §6.10).
const MAX_SLUG_LENGTH: usize = 191;

/// A defect of data-migration.md §8.1, or the normaliser's N7 (§7.1), whose
/// rows the import resolves from the resolution file or only lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DefectId {
    /// Post dates one day early.
    D1,
    /// Published posts without an author.
    D2,
    /// A published post without an index image.
    D3,
    /// Posts without a body; imported unchanged and listed.
    D4,
    /// The 1970 sentinel date on drafts.
    D5,
    /// Duplicate canteen days.
    D6,
    /// Canteen days that break the 2-3 dish rule; imported unchanged and
    /// listed.
    D7,
    /// Duplicate label names.
    D8,
    /// Menu items without a position.
    D9,
    /// Pages linked from nowhere.
    D10,
    /// A body from which the sanitiser removes something that no normaliser
    /// step planned.
    N7,
}

impl DefectId {
    /// Every defect, in the order of data-migration.md §8.1, N7 last.
    pub const ALL: [Self; 11] = [
        Self::D1,
        Self::D2,
        Self::D3,
        Self::D4,
        Self::D5,
        Self::D6,
        Self::D7,
        Self::D8,
        Self::D9,
        Self::D10,
        Self::N7,
    ];

    /// The defect's id as data-migration.md writes it: `D1` to `D10`, `N7`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::D1 => "D1",
            Self::D2 => "D2",
            Self::D3 => "D3",
            Self::D4 => "D4",
            Self::D5 => "D5",
            Self::D6 => "D6",
            Self::D7 => "D7",
            Self::D8 => "D8",
            Self::D9 => "D9",
            Self::D10 => "D10",
            Self::N7 => "N7",
        }
    }

    /// The section of the resolution file whose entries resolve the defect,
    /// or `None` for D4 and D7: their rows are imported unchanged
    /// (data-migration.md §8.1, ADR 0017 §3), so an instance needs no entry
    /// and is only listed.
    #[must_use]
    pub const fn section(self) -> Option<Section> {
        match self {
            Self::D1 => Some(Section::PostDateShift),
            Self::D2 => Some(Section::PostAuthor),
            Self::D3 => Some(Section::PostIndexImage),
            Self::D5 => Some(Section::PostSentinelDate),
            Self::D6 => Some(Section::CanteenDuplicate),
            Self::D8 => Some(Section::LabelDuplicate),
            Self::D9 => Some(Section::MenuUnpositioned),
            Self::D10 => Some(Section::PageHidden),
            Self::N7 => Some(Section::N7Accept),
            Self::D4 | Self::D7 => None,
        }
    }

    /// The defect in a few words, after the "Defect" column of
    /// data-migration.md §8.1, as the sign-off sheet heads it.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::D1 => "Post dates one day early",
            Self::D2 => "Published posts without an author",
            Self::D3 => "Published posts without an index image",
            Self::D4 => "Posts without a body",
            Self::D5 => "Sentinel dates on drafts",
            Self::D6 => "Duplicate canteen days",
            Self::D7 => "Canteen days that break the 2-3 dish rule",
            Self::D8 => "Duplicate label names",
            Self::D9 => "Menu items without a position",
            Self::D10 => "Pages linked from nowhere",
            Self::N7 => "Unplanned sanitiser removals in a body",
        }
    }

    /// How the entries of the defect's section resolve its instances, after
    /// the "Proposed resolution" column of data-migration.md §8.1.
    #[must_use]
    pub const fn entry_policy(self) -> EntryPolicy {
        match self {
            Self::D1 | Self::D2 | Self::D5 | Self::D6 | Self::D8 | Self::D9 | Self::N7 => {
                EntryPolicy::Required
            }
            Self::D3 => EntryPolicy::SourceFix,
            Self::D10 => EntryPolicy::Optional,
            Self::D4 | Self::D7 => EntryPolicy::Listed,
        }
    }
}

/// How the entries of a defect's section resolve its instances
/// (data-migration.md §8.1, §8.2, ADR 0047 §4): what the detector does with
/// an instance that no entry covers, and with an entry whose subject the
/// rule does not find. An instance that an entry covers is judged the same
/// way under every policy: the entry's resolution when its precondition
/// holds, otherwise a problem ([`DefectOutcome::Changed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryPolicy {
    /// The defect has no section (D4, D7): every instance is imported
    /// unchanged and listed ([`DefectOutcome::Unchanged`]).
    Listed,
    /// Every instance needs a signed entry (D1, D2, D5, D6, D8, D9, N7): an
    /// instance without one is a problem ([`DefectOutcome::Uncovered`]), and
    /// so is an entry for a subject the rule no longer finds
    /// ([`DefectOutcome::Stale`]).
    Required,
    /// The editors may list instances, and an instance that no entry lists
    /// takes the defect's default, which is no problem
    /// ([`DefectOutcome::Defaulted`]): D10, whose pages are imported as
    /// published unless an entry lists one as hidden. An entry for a
    /// subject the rule no longer finds is a problem, as under
    /// [`Self::Required`].
    Optional,
    /// The editors fix every instance in the source before the final dump,
    /// and the entry pins the fixed value (D3: post 70's index image, set in
    /// the current admin; the entry's precondition is that image). An
    /// instance without an entry is a problem: the fix is missing. An entry
    /// whose subject the rule no longer finds holds when the source has the
    /// pinned value ([`DefectOutcome::FixedInSource`]), and is a problem
    /// when the value differs or the row is gone ([`DefectOutcome::Stale`]).
    SourceFix,
}

impl fmt::Display for DefectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What one defect instance, or one entry of the resolution file, is about.
/// The detector pairs an instance with the entry of the same subject.
///
/// Ids are the legacy ids, which the import keeps (ADR 0009).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// A post (`posts_data.id`): D1 to D5, and N7 for a post's body.
    Post(NonZeroU32),
    /// A page (`pages.id`): D10, and N7 for a page's body.
    Page(NonZeroU32),
    /// A canteen day (`canteen_data.id`): D7.
    CanteenDay(NonZeroU32),
    /// A canteen date with more than one day on it: D6.
    CanteenDate(NaiveDate),
    /// A group of labels with equal names, named by its lowest label id: D8.
    LabelGroup(NonZeroU32),
    /// The menu items of one parent (`menu_items.id`), or of the root level
    /// for `None`: D9.
    MenuLevel(Option<NonZeroU32>),
}

impl fmt::Display for Subject {
    /// `post 12`, `page 7`, `canteen day 307`, `canteen date 2022-04-01`,
    /// `label group 4`, `menu parent 15` or `menu root level`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Post(id) => write!(f, "post {id}"),
            Self::Page(id) => write!(f, "page {id}"),
            Self::CanteenDay(id) => write!(f, "canteen day {id}"),
            Self::CanteenDate(date) => write!(f, "canteen date {}", date.format("%Y-%m-%d")),
            Self::LabelGroup(id) => write!(f, "label group {id}"),
            Self::MenuLevel(Some(id)) => write!(f, "menu parent {id}"),
            Self::MenuLevel(None) => f.write_str("menu root level"),
        }
    }
}

/// A legacy index image path, `index/<32 lower-case hex digits>.<extension>`
/// (data-migration.md §6.1), the one kind of path a precondition holds
/// (D3). Only [`Self::new`] makes one, so it never holds other text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndexImage(String);

impl IndexImage {
    /// `path` as an index image path, or `None` when it does not have the
    /// legacy shape ([`is_index_image_path`]).
    ///
    /// - `path`: a source `posts_data.index_image` value, or a D3 entry's
    ///   `expect_index_image`.
    #[must_use]
    pub fn new(path: &str) -> Option<Self> {
        is_index_image_path(path).then(|| Self(path.to_owned()))
    }

    /// The path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A fixed word that a resolution is shown as: the choices of the
/// resolution file and of data-migration.md §8.1 that are neither an id nor
/// a date.
///
/// A closed set, so no text from the source can become one: a `&'static
/// str` would not do, since `String::leak` turns any string into one. A
/// detector that needs another word adds a variant here, to [`Self::ALL`]
/// and to [`Self::as_str`], whose tests (and the sign-off sheet's line
/// check) then cover it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Word {
    /// D2's generic author `Szerkesztőség`, written `generic` as the
    /// resolution file writes it ([`GENERIC_AUTHOR`]).
    Generic,
    /// D10: the page is imported as published, the default.
    Published,
    /// D10: the page is imported as hidden, an entry's resolution.
    Hidden,
    /// N7: the body's unplanned sanitiser removals are accepted.
    Accepted,
}

impl Word {
    /// Every word.
    pub const ALL: [Self; 4] = [Self::Generic, Self::Published, Self::Hidden, Self::Accepted];

    /// The word as the report and the sign-off sheet show it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Generic => GENERIC_AUTHOR,
            Self::Published => "published",
            Self::Hidden => "hidden",
            Self::Accepted => "accepted",
        }
    }
}

/// A value that a precondition compares, and that the report and the
/// sign-off sheet show: a found source value, an entry's expected value, or
/// a resolution.
///
/// It holds only ids, calendar days, instants, digests, index image paths
/// and the words of [`Word`], the kinds of value the resolution file itself
/// may hold (data-migration.md §8.2), so it can never carry a name or
/// content into the report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Value {
    /// SQL NULL, shown `none` as the resolution file writes it.
    Null,
    /// A legacy id.
    Id(NonZeroU32),
    /// Several legacy ids. A precondition compares them as a set
    /// ([`Self::matches`]); they are shown in the order given.
    Ids(Vec<NonZeroU32>),
    /// A calendar day.
    Date(NaiveDate),
    /// An instant.
    Instant(DateTime<Utc>),
    /// The SHA-256 of a source value, such as a body (N7).
    Sha256([u8; 32]),
    /// A legacy index image path (D3).
    IndexImage(IndexImage),
    /// A fixed word, such as the generic author's `generic` (D2); only the
    /// closed set of [`Word`] can be one.
    Choice(Word),
}

impl Value {
    /// Whether this found value meets the precondition `expected`: equal
    /// values, where two [`Self::Ids`] are equal as sets, because a
    /// precondition such as D6's `expect_ids` names every row on a date in
    /// whatever order the editors listed them.
    ///
    /// - `expected`: the entry's precondition.
    #[must_use]
    pub fn matches(&self, expected: &Self) -> bool {
        match (self, expected) {
            (Self::Ids(found), Self::Ids(expected)) => {
                found.iter().collect::<BTreeSet<_>>() == expected.iter().collect::<BTreeSet<_>>()
            }
            (found, expected) => found == expected,
        }
    }
}

impl fmt::Display for Value {
    /// `none`, `55`, `[393, 398]`, `2019-05-02`, `2024-03-01T10:00:00Z`, 64
    /// lower-case hex digits, `index/<hex>.<ext>`, or the word.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("none"),
            Self::Id(id) => write!(f, "{id}"),
            Self::Ids(ids) => write!(f, "[{}]", id_list(ids)),
            Self::Date(date) => write!(f, "{}", date.format("%Y-%m-%d")),
            Self::Instant(instant) => f.write_str(&format_instant(*instant)),
            Self::Sha256(digest) => write!(f, "{}", FileDigest(*digest)),
            Self::IndexImage(path) => f.write_str(path.as_str()),
            Self::Choice(word) => f.write_str(word.as_str()),
        }
    }
}

/// Ids separated by `, `: `393, 398`.
///
/// - `ids`: the ids, in the order to show them.
#[must_use]
pub fn id_list(ids: &[NonZeroU32]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A page slug as the live site's path `/information/<slug>` uses it:
/// 1 to 191 characters of `a-z`, `0-9` and `-`, the shape of all 39 legacy
/// slugs (data-migration.md §6.10). Only [`Self::new`] makes one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageSlug(String);

impl PageSlug {
    /// `slug` as a page slug, or `None` when it has another shape: such a
    /// page gets no path on the sign-off sheet, which then shows its id
    /// only.
    ///
    /// - `slug`: the source `pages.slug`.
    #[must_use]
    pub fn new(slug: &str) -> Option<Self> {
        let shaped = !slug.is_empty()
            && slug.len() <= MAX_SLUG_LENGTH
            && slug
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'));
        shaped.then(|| Self(slug.to_owned()))
    }

    /// The slug.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A path on the live public site at which the editors can see a row, for
/// the sign-off sheet (data-migration.md §8.2; cutover.md §7.2, G9).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PublicPath {
    /// A post: `/posts/<id>`.
    Post(NonZeroU32),
    /// A page: `/information/<slug>`.
    Page(PageSlug),
}

impl fmt::Display for PublicPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Post(id) => write!(f, "/posts/{id}"),
            Self::Page(slug) => write!(f, "/information/{}", slug.as_str()),
        }
    }
}

/// A resolution that a rule proposes or an entry gives, with how the report
/// and the sign-off sheet show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal<R> {
    /// What the step applies, in the step's own type: a post's new date
    /// (D1), the author choice (D2), the row kept (D6), ...
    pub resolution: R,
    /// The same resolution as the report shows it.
    pub shown: Value,
}

/// One instance of a defect, as its rule found it in the source rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance<R> {
    /// What the instance is about; an entry of the same subject covers it.
    pub subject: Subject,
    /// The legacy row ids it touches, as the sign-off sheet lists them: the
    /// post, the rows on a duplicate date, the labels of a group, ...
    pub rows: Vec<NonZeroU32>,
    /// Where the editors can see it on the live site, for the sign-off
    /// sheet; none where the site shows nothing to open.
    pub paths: Vec<PublicPath>,
    /// The source value that an entry's precondition is compared with, in
    /// the form of the entry's `expect_*` field.
    pub found: Value,
    /// The defect's proposed default (data-migration.md §8.1), which a dry
    /// run applies to an instance that no valid entry resolves, and both
    /// modes apply to an instance of an optional defect that no entry lists
    /// ([`EntryPolicy::Optional`]); `None` where the editors must decide, so
    /// the source value is kept.
    pub proposed: Option<Proposal<R>>,
}

/// The rows and the public paths that `subject` itself gives: a post's id
/// and `/posts/<id>`, a page's or canteen day's id; nothing for a date, a
/// group or a menu level, whose rule adds them to its instances.
fn subject_rows_and_paths(subject: &Subject) -> (Vec<NonZeroU32>, Vec<PublicPath>) {
    match subject {
        Subject::Post(id) => (vec![*id], vec![PublicPath::Post(*id)]),
        Subject::Page(id) | Subject::CanteenDay(id) => (vec![*id], Vec::new()),
        Subject::CanteenDate(_) | Subject::LabelGroup(_) | Subject::MenuLevel(_) => {
            (Vec::new(), Vec::new())
        }
    }
}

impl<R> Instance<R> {
    /// An instance of `subject` with the source value `found`, no proposed
    /// default, and the rows and paths that the subject itself gives: a
    /// post's id and `/posts/<id>`, a page's or canteen day's id; nothing
    /// for a date, a group or a menu level, whose rule adds them.
    ///
    /// - `subject`: what the instance is about.
    /// - `found`: the source value that a precondition compares.
    #[must_use]
    pub fn new(subject: Subject, found: Value) -> Self {
        let (rows, paths) = subject_rows_and_paths(&subject);
        Self {
            subject,
            rows,
            paths,
            found,
            proposed: None,
        }
    }

    /// The instance with `rows` as its row ids.
    ///
    /// - `rows`: every legacy row id the instance touches.
    #[must_use]
    pub fn with_rows(self, rows: Vec<NonZeroU32>) -> Self {
        Self { rows, ..self }
    }

    /// The instance with `paths` as its public paths.
    ///
    /// - `paths`: where the editors can see it on the live site.
    #[must_use]
    pub fn with_paths(self, paths: Vec<PublicPath>) -> Self {
        Self { paths, ..self }
    }

    /// The instance with the proposed default `proposed`.
    ///
    /// - `proposed`: the default that a dry run applies when no entry
    ///   resolves the instance.
    #[must_use]
    pub fn with_proposed(self, proposed: Proposal<R>) -> Self {
        Self {
            proposed: Some(proposed),
            ..self
        }
    }
}

/// An entry of a section of the resolution file, as the detector pairs it
/// with an instance: the subject it covers and its precondition.
///
/// The parsed entry types of [`super::resolutions`] implement it, one per
/// section, and [`NoEntry`] stands for the defects that have no section.
pub trait SignedEntry: Sized {
    /// The section whose entries these are, or `None` for [`NoEntry`].
    const SECTION: Option<Section>;

    /// The section's entries in `file`, in file order.
    ///
    /// - `file`: the validated resolution file of the run.
    fn entries(file: &ResolutionFile) -> &[Self];

    /// The subject the entry covers.
    fn subject(&self) -> Subject;

    /// The entry's precondition: the source value the editors saw when they
    /// signed, in its `expect_*` field.
    fn expected(&self) -> Value;
}

impl SignedEntry for PostDateShift {
    const SECTION: Option<Section> = Some(Section::PostDateShift);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.post_date_shift
    }

    fn subject(&self) -> Subject {
        Subject::Post(self.post_id)
    }

    fn expected(&self) -> Value {
        Value::Date(self.expect_date)
    }
}

impl SignedEntry for PostAuthor {
    const SECTION: Option<Section> = Some(Section::PostAuthor);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.post_author
    }

    fn subject(&self) -> Subject {
        Subject::Post(self.post_id)
    }

    /// Always `none`: the file can only write `expect_author_id = "none"`.
    fn expected(&self) -> Value {
        Value::Null
    }
}

impl SignedEntry for PostIndexImage {
    const SECTION: Option<Section> = Some(Section::PostIndexImage);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.post_index_image
    }

    fn subject(&self) -> Subject {
        Subject::Post(self.post_id)
    }

    /// The path, which validation has checked; a value built in code with
    /// another shape is shown as `none`, which no source row matches as an
    /// index image.
    fn expected(&self) -> Value {
        IndexImage::new(&self.expect_index_image).map_or(Value::Null, Value::IndexImage)
    }
}

impl SignedEntry for PostSentinelDate {
    const SECTION: Option<Section> = Some(Section::PostSentinelDate);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.post_sentinel_date
    }

    fn subject(&self) -> Subject {
        Subject::Post(self.post_id)
    }

    /// Always the sentinel, the only value the file can write.
    fn expected(&self) -> Value {
        Value::Date(SENTINEL_DATE)
    }
}

impl SignedEntry for CanteenDuplicate {
    const SECTION: Option<Section> = Some(Section::CanteenDuplicate);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.canteen_duplicate
    }

    fn subject(&self) -> Subject {
        Subject::CanteenDate(self.date)
    }

    fn expected(&self) -> Value {
        Value::Ids(self.expect_ids.clone())
    }
}

impl SignedEntry for LabelDuplicate {
    const SECTION: Option<Section> = Some(Section::LabelDuplicate);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.label_duplicate
    }

    /// The group of the lowest label id the entry lists (`keep` for an
    /// entry built in code without ids). The D8 rule names a group it finds
    /// the same way, so the entry and the group pair up while the lowest id
    /// stays in the group.
    fn subject(&self) -> Subject {
        Subject::LabelGroup(self.expect_ids.iter().min().copied().unwrap_or(self.keep))
    }

    fn expected(&self) -> Value {
        Value::Ids(self.expect_ids.clone())
    }
}

impl SignedEntry for MenuUnpositioned {
    const SECTION: Option<Section> = Some(Section::MenuUnpositioned);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.menu_unpositioned
    }

    fn subject(&self) -> Subject {
        Subject::MenuLevel(self.parent_id)
    }

    fn expected(&self) -> Value {
        Value::Ids(self.expect_null_lft.clone())
    }
}

impl SignedEntry for PageHidden {
    const SECTION: Option<Section> = Some(Section::PageHidden);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.page_hidden
    }

    fn subject(&self) -> Subject {
        Subject::Page(self.page_id)
    }

    fn expected(&self) -> Value {
        Value::Instant(self.expect_updated_at)
    }
}

impl SignedEntry for N7Accept {
    const SECTION: Option<Section> = Some(Section::N7Accept);

    fn entries(file: &ResolutionFile) -> &[Self] {
        &file.n7_accept
    }

    fn subject(&self) -> Subject {
        match self.body {
            BodyRow::Post(id) => Subject::Post(id),
            BodyRow::Page(id) => Subject::Page(id),
        }
    }

    fn expected(&self) -> Value {
        Value::Sha256(self.expect_content_sha256)
    }
}

/// The entries of a defect that has no section of the resolution file (D4
/// and D7, whose rows are imported unchanged): there are none, and the type
/// has no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoEntry {}

impl SignedEntry for NoEntry {
    const SECTION: Option<Section> = None;

    fn entries(_file: &ResolutionFile) -> &[Self] {
        &[]
    }

    fn subject(&self) -> Subject {
        match *self {}
    }

    fn expected(&self) -> Value {
        match *self {}
    }
}

/// The precondition field of `section`, its one `expect_*` field, as the
/// messages name it.
///
/// - `section`: a section of the resolution file.
#[must_use]
pub fn precondition_field(section: Section) -> &'static str {
    section
        .fields()
        .iter()
        .copied()
        .find(|field| field.starts_with("expect_"))
        .unwrap_or("expect")
}

/// A defect's rule, which a table step implements for the defect's source
/// rows and hands to [`DefectDetector::run`].
pub trait DefectRule {
    /// The source rows the rule reads, such as a post's legacy columns.
    type Row;
    /// The entries of the defect's section of the resolution file, or
    /// [`NoEntry`] for a defect that has none.
    type Entry: SignedEntry;
    /// What the step applies to an instance, such as a post's new date.
    type Resolution: Clone;

    /// The defect the rule detects. Its [`DefectId::section`] must be
    /// [`Self::Entry`]'s section; [`DefectDetector::run`] refuses a rule
    /// whose two differ.
    const DEFECT: DefectId;

    /// Every instance of the defect in `rows`, in any order, each subject
    /// once. Called on every run: instances are never taken from the
    /// resolution file.
    ///
    /// - `rows`: every source row the rule reads.
    fn detect(&self, rows: &[Self::Row]) -> Vec<Instance<Self::Resolution>>;

    /// What the precondition of an entry for `subject` reads in `rows`,
    /// when the rule found no instance there: the value in the form of the
    /// entry's `expect_*` field, or `None` when no row is the subject (it
    /// was deleted, say). The report shows it as what was found, and for a
    /// defect whose entries pin a fix made in the source
    /// ([`EntryPolicy::SourceFix`], D3) it decides whether the entry holds.
    ///
    /// - `rows`: the same rows as [`Self::detect`] read.
    /// - `subject`: the subject of an entry that covers no instance.
    fn found(&self, rows: &[Self::Row], subject: &Subject) -> Option<Value>;

    /// Whether the rule's rows can hold `subject`. A rule that reads only
    /// part of a section's subjects, such as N7 over posts while the pages
    /// have their own run, leaves the others out, so their entries are not
    /// taken for stale ones. Every subject, by default.
    ///
    /// - `subject`: the subject of an entry of the rule's section.
    fn covers(&self, subject: &Subject) -> bool {
        let _ = subject;
        true
    }

    /// What `entry` resolves its instance to, once its precondition holds.
    ///
    /// - `entry`: an entry of the rule's section whose precondition the
    ///   source meets.
    fn resolution(&self, entry: &Self::Entry) -> Proposal<Self::Resolution>;
}

/// What happens to one instance, for the step that applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision<R> {
    /// The resolution of the entry that covers the instance and whose
    /// precondition holds.
    Entry(R),
    /// The defect's proposed default, which a dry run applies to an
    /// instance that no valid entry resolves, and both modes to an instance
    /// of an optional defect that no entry lists.
    Default(R),
    /// The source value is kept: the defect is imported unchanged (D4, D7),
    /// or no valid entry resolves the instance and the rule has no default.
    Kept,
}

impl<R> Decision<R> {
    /// The resolution to apply, or `None` when the source value is kept.
    #[must_use]
    pub const fn resolution(&self) -> Option<&R> {
        match self {
            Self::Entry(resolution) | Self::Default(resolution) => Some(resolution),
            Self::Kept => None,
        }
    }
}

/// What a [`DefectDetector::run`] decided for every instance of its defect,
/// by subject. A subject that is not here has no instance, and its rows are
/// imported as the table's rules say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decisions<R>(BTreeMap<Subject, Decision<R>>);

impl<R> Decisions<R> {
    /// The decision for `subject`, or `None` when it is no instance.
    ///
    /// - `subject`: a post, page, date, group or menu level of the step.
    #[must_use]
    pub fn get(&self, subject: &Subject) -> Option<&Decision<R>> {
        self.0.get(subject)
    }

    /// The resolution to apply to `subject`, or `None` when it is no
    /// instance or its source value is kept.
    ///
    /// - `subject`: a post, page, date, group or menu level of the step.
    #[must_use]
    pub fn resolution(&self, subject: &Subject) -> Option<&R> {
        self.get(subject).and_then(Decision::resolution)
    }

    /// Every decision, in subject order.
    pub fn iter(&self) -> btree_map::Iter<'_, Subject, Decision<R>> {
        self.0.iter()
    }

    /// The number of instances.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the rule found no instance.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a, R> IntoIterator for &'a Decisions<R> {
    type Item = (&'a Subject, &'a Decision<R>);
    type IntoIter = btree_map::Iter<'a, Subject, Decision<R>>;

    /// Every decision, in subject order ([`Decisions::iter`]).
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// How the detector judged one instance or one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefectOutcome {
    /// An entry covers the instance and its precondition holds: the entry's
    /// resolution is applied.
    Covered,
    /// A defect without a section (D4, D7): imported unchanged and listed.
    Unchanged,
    /// No entry lists the instance of a defect whose entries are optional
    /// (D10): the defect's default is applied, or the source value kept
    /// where the rule has none. No problem.
    Defaulted,
    /// No entry covers the instance, and the defect needs one. A problem.
    Uncovered,
    /// An entry covers the instance, but its precondition does not hold:
    /// the row changed since the sign-off. A problem.
    Changed,
    /// An entry of a defect whose entries pin a fix made in the source
    /// (D3): the rule no longer finds its subject, and the source has the
    /// pinned value, so the entry holds and nothing is applied. No problem.
    FixedInSource,
    /// An entry that does not hold for a subject the rule no longer finds:
    /// the row no longer has the defect, or is gone, or (D3) its fix in the
    /// source differs from the one signed. A problem.
    Stale,
}

impl DefectOutcome {
    /// Every outcome.
    pub const ALL: [Self; 7] = [
        Self::Covered,
        Self::Unchanged,
        Self::Defaulted,
        Self::Uncovered,
        Self::Changed,
        Self::FixedInSource,
        Self::Stale,
    ];

    /// The outcome's name in `summary.json` and `report.md`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Covered => "covered",
            Self::Unchanged => "unchanged",
            Self::Defaulted => "defaulted",
            Self::Uncovered => "uncovered",
            Self::Changed => "changed",
            Self::FixedInSource => "fixed_in_source",
            Self::Stale => "stale",
        }
    }

    /// Whether the outcome is a problem, which stops `apply` and fails a
    /// dry run at its end.
    #[must_use]
    pub const fn is_problem(self) -> bool {
        matches!(self, Self::Uncovered | Self::Changed | Self::Stale)
    }

    /// Whether the record is an instance the rule found, rather than an
    /// entry whose subject the rule no longer finds
    /// ([`Self::FixedInSource`], [`Self::Stale`]).
    #[must_use]
    pub const fn is_instance(self) -> bool {
        !matches!(self, Self::FixedInSource | Self::Stale)
    }
}

/// The entry that a [`DefectRecord`] is about, with its precondition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRef {
    /// The entry's section.
    pub section: Section,
    /// Its place among the section's entries, counted from 1.
    pub index: usize,
    /// Its precondition.
    pub expected: Value,
}

impl EntryRef {
    /// The entry as the resolution file's messages name it:
    /// `post_date_shift[2]`.
    #[must_use]
    pub fn name(&self) -> String {
        format!("{}[{}]", self.section.key(), self.index)
    }
}

/// One line of the report's Defects part: an instance, or an entry whose
/// subject the rule does not find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefectRecord {
    /// The defect.
    pub defect: DefectId,
    /// What the instance or the entry is about.
    pub subject: Subject,
    /// How the detector judged it.
    pub outcome: DefectOutcome,
    /// The legacy row ids it touches: an instance's, or the subject's own
    /// for an entry that holds for a row fixed in the source; none for a
    /// stale entry, whose row may be gone.
    pub rows: Vec<NonZeroU32>,
    /// Where the editors can see it on the live site, given as the rows
    /// are.
    pub paths: Vec<PublicPath>,
    /// The source value, or `None` when no row is the subject (a stale
    /// entry's deleted row).
    pub found: Option<Value>,
    /// The entry that covers the subject, if one does.
    pub entry: Option<EntryRef>,
    /// The resolution the import proposes for the instance, as shown: the
    /// valid entry's, otherwise the defect's default; `None` where the
    /// source value is kept (no default, a defect imported unchanged, or an
    /// entry whose subject the rule does not find). A dry run applies it,
    /// and so does `apply` up to the record that stops it; the sign-off
    /// sheet shows it.
    pub proposed: Option<Value>,
}

impl DefectRecord {
    /// The problem's message, which names the defect, the row, and for an
    /// entry, the expected and the found value, and says what to do; `None`
    /// for an outcome that is no problem.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let defect = self.defect;
        let subject = &self.subject;
        let found = self
            .found
            .as_ref()
            .map_or_else(|| "no such row".to_owned(), ToString::to_string);
        match (self.outcome, &self.entry) {
            (
                DefectOutcome::Covered
                | DefectOutcome::Unchanged
                | DefectOutcome::Defaulted
                | DefectOutcome::FixedInSource,
                _,
            ) => None,
            (DefectOutcome::Uncovered, _) => {
                let section = defect
                    .section()
                    .map_or_else(|| "an".to_owned(), |section| format!("a {}", section.key()));
                Some(if defect.entry_policy() == EntryPolicy::SourceFix {
                    format!(
                        "{defect} {subject} is not covered by the resolution file (its source value \
                         is {found}): the editors fix it in the current admin before the final \
                         dump, then add {section} entry with the fixed value once they have signed \
                         it (data-migration.md §8.1, §8.2)"
                    )
                } else {
                    format!(
                        "{defect} {subject} is not covered by the resolution file (its source value \
                         is {found}): add {section} entry for it once the editors have signed it \
                         (data-migration.md §8.2)"
                    )
                })
            }
            (DefectOutcome::Changed, Some(entry)) => Some(format!(
                "{defect} {subject}: the precondition of {} does not hold: {} is {}, but the source \
                 has {found}. The row changed since the sign-off: sign the entry again \
                 (data-migration.md §8.2)",
                entry.name(),
                precondition_field(entry.section),
                entry.expected
            )),
            // An entry that pins a fix made in the source, for a row that
            // is still there: the fix changed after the sign-off.
            (DefectOutcome::Stale, Some(entry))
                if defect.entry_policy() == EntryPolicy::SourceFix && self.found.is_some() =>
            {
                Some(format!(
                    "{defect} {subject}: the precondition of {}, which pins the fix made in the \
                     source, does not hold: {} is {}, but the source has {found}. The row changed \
                     since the sign-off: sign the entry again (data-migration.md §8.1, §8.2)",
                    entry.name(),
                    precondition_field(entry.section),
                    entry.expected
                ))
            }
            (DefectOutcome::Stale, Some(entry)) => Some(format!(
                "{defect} {subject}: {} covers it, but the {defect} rule no longer finds it ({} is \
                 {}, the source has {found}): {} (data-migration.md §8.2)",
                entry.name(),
                precondition_field(entry.section),
                entry.expected,
                if self.found.is_some() {
                    "the row no longer has the defect, so remove the entry or sign it again"
                } else {
                    "the row is gone, so remove the entry"
                }
            )),
            // A record without its entry cannot be changed or stale; the
            // detector never makes one, but it is still a problem.
            (DefectOutcome::Changed | DefectOutcome::Stale, None) => Some(format!(
                "{defect} {subject}: an entry of the resolution file does not match the source \
                 (the source has {found}) (data-migration.md §8.2)"
            )),
        }
    }
}

/// The report's Defects part (data-migration.md §11.3): every defect whose
/// detector ran, every instance and every entry whose subject the rule does
/// not find, and whether the run reached its end.
///
/// Only [`DefectDetector::run`] adds to it, and only
/// [`DefectsPart::conclude`] marks it complete, so it always describes what
/// the run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefectsPart {
    checked: Vec<DefectId>,
    records: Vec<DefectRecord>,
    complete: bool,
}

impl DefectsPart {
    /// The defects whose detector ran, each once, in the order they first
    /// ran.
    #[must_use]
    pub fn checked(&self) -> &[DefectId] {
        &self.checked
    }

    /// Every record, in the order the detectors made them: by defect as
    /// they ran, then by subject.
    #[must_use]
    pub fn records(&self) -> &[DefectRecord] {
        &self.records
    }

    /// Whether the run reached its end, so every detector of the build ran
    /// ([`Self::conclude`]).
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// The number of instances the rules found
    /// ([`DefectOutcome::is_instance`]).
    #[must_use]
    pub fn instances(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.outcome.is_instance())
            .count()
    }

    /// The number of instances that no entry covers where the defect needs
    /// one.
    #[must_use]
    pub fn uncovered(&self) -> usize {
        self.count(DefectOutcome::Uncovered)
    }

    /// The number of entries whose precondition does not hold: changed
    /// rows and stale entries.
    #[must_use]
    pub fn refused(&self) -> usize {
        self.count(DefectOutcome::Changed) + self.count(DefectOutcome::Stale)
    }

    /// The number of records with `outcome`.
    ///
    /// - `outcome`: the outcome to count.
    #[must_use]
    pub fn count(&self, outcome: DefectOutcome) -> usize {
        self.records
            .iter()
            .filter(|record| record.outcome == outcome)
            .count()
    }

    /// Ends the defect check at the end of the run: marks the part complete
    /// and fails when it holds a problem, which only a dry run can collect
    /// (ADR 0047).
    ///
    /// # Errors
    ///
    /// [`DefectProblems`], which counts the problems and names each of
    /// them, when any record is a problem.
    pub fn conclude(&mut self) -> Result<(), DefectProblems> {
        self.complete = true;
        let lines: Vec<String> = self
            .records
            .iter()
            .filter_map(DefectRecord::problem)
            .collect();
        if lines.is_empty() {
            Ok(())
        } else {
            Err(DefectProblems {
                uncovered: self.uncovered(),
                refused: self.refused(),
                lines,
            })
        }
    }

    /// Records that the detector of `defect` runs.
    fn start(&mut self, defect: DefectId) {
        if !self.checked.contains(&defect) {
            self.checked.push(defect);
        }
    }
}

/// The problems a run collected, which fail it at its end (ADR 0047): a dry
/// run lists every one instead of stopping at the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefectProblems {
    /// The number of instances that no entry covers.
    pub uncovered: usize,
    /// The number of entries whose precondition does not hold.
    pub refused: usize,
    /// Every problem's message, in record order.
    pub lines: Vec<String>,
}

impl fmt::Display for DefectProblems {
    /// The counts, then one line per problem, the first `SHOWN_PROBLEMS` (50)
    /// of them, and a count of the rest.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.lines.len();
        write!(
            f,
            "the run found {total} defect problem{} (data-migration.md §4.2, §11.1): {} {} that \
             the resolution file does not cover, and {} {} whose precondition does not hold. An \
             apply run stops at the first of them; this dry run went on with the proposed \
             defaults, and the Defects part of the report lists every one, signoff.md every \
             instance for the editors:",
            if total == 1 { "" } else { "s" },
            self.uncovered,
            if self.uncovered == 1 {
                "instance"
            } else {
                "instances"
            },
            self.refused,
            if self.refused == 1 {
                "entry"
            } else {
                "entries"
            },
        )?;
        for line in self.lines.iter().take(SHOWN_PROBLEMS) {
            write!(f, "\n- {line}")?;
        }
        if total > SHOWN_PROBLEMS {
            write!(f, "\n- and {} more", total - SHOWN_PROBLEMS)?;
        }
        Ok(())
    }
}

impl std::error::Error for DefectProblems {}

/// An `apply` run's first problem, or a rule that broke the detector's
/// contract; it stops the import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefectRefusal(pub String);

impl fmt::Display for DefectRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refusing to import: {}", self.0)
    }
}

impl std::error::Error for DefectRefusal {}

/// Runs the defect rules of a run against the validated resolution file.
///
/// The table steps make one per run, from the run's mode and the file the
/// task validated before the task lock, and run each of their rules through
/// it ([`Self::run`]).
#[derive(Debug, Clone, Copy)]
pub struct DefectDetector<'a> {
    mode: Mode,
    file: &'a ResolutionFile,
}

impl<'a> DefectDetector<'a> {
    /// A detector for a run in `mode` with the resolution file `file`.
    ///
    /// - `mode`: the run's mode: `apply` stops at the first problem, a dry
    ///   run collects them.
    /// - `file`: the run's validated resolution file.
    #[must_use]
    pub const fn new(mode: Mode, file: &'a ResolutionFile) -> Self {
        Self { mode, file }
    }

    /// Detects the instances of `rule`'s defect in `rows`, pairs them with
    /// the entries of its section, records every instance and every entry
    /// whose subject the rule does not find in `part`, and returns what to
    /// apply to each instance.
    ///
    /// - `rule`: the defect's rule.
    /// - `rows`: every source row the rule reads.
    /// - `part`: the run's Defects part, which keeps what this call
    ///   recorded also when it fails.
    ///
    /// Subjects are judged in order, by the defect's [`EntryPolicy`]. An
    /// instance that a valid entry covers gets the entry's resolution; an
    /// instance of a defect without a section is imported unchanged; an
    /// instance of a defect whose entries are optional, and that no entry
    /// lists, gets the default (or keeps the source value); an entry that
    /// pins a fix made in the source holds when the rule no longer finds its
    /// subject and the source has the pinned value. The rest are problems
    /// ([`DefectOutcome::is_problem`]): `apply` returns the first, while a
    /// dry run records it, gives the instance the rule's proposed default
    /// (or keeps the source value) and goes on.
    ///
    /// # Errors
    ///
    /// [`DefectRefusal`] for the first problem in `apply` mode, naming the
    /// defect and the row and, for an entry, the expected and the found
    /// value; in either mode, for a rule that breaks the contract of
    /// [`DefectRule`]: its section is not its defect's, it finds a subject
    /// twice, or two entries cover one subject (which validation of the
    /// file rules out).
    pub fn run<D: DefectRule>(
        &self,
        rule: &D,
        rows: &[D::Row],
        part: &mut DefectsPart,
    ) -> Result<Decisions<D::Resolution>, DefectRefusal> {
        let defect = D::DEFECT;
        if D::Entry::SECTION != defect.section() {
            let reads = D::Entry::SECTION.map_or("no section", Section::key);
            let belongs = defect.section().map_or("no section", Section::key);
            return Err(DefectRefusal(format!(
                "the {defect} rule reads the entries of {reads}, but {defect} is resolved by \
                 {belongs}: this is a defect of the import's code, not of the data"
            )));
        }
        let mut instances = BTreeMap::new();
        for instance in rule.detect(rows) {
            let subject = instance.subject.clone();
            if instances.insert(subject.clone(), instance).is_some() {
                return Err(DefectRefusal(format!(
                    "the {defect} rule found {subject} twice: this is a defect of the import's \
                     code, not of the data"
                )));
            }
        }
        let mut entries: BTreeMap<Subject, (EntryRef, &D::Entry)> = BTreeMap::new();
        if let Some(section) = D::Entry::SECTION {
            for (position, entry) in D::Entry::entries(self.file).iter().enumerate() {
                let subject = entry.subject();
                if !rule.covers(&subject) {
                    continue;
                }
                let index = position + 1;
                match entries.entry(subject) {
                    btree_map::Entry::Occupied(earlier) => {
                        return Err(DefectRefusal(format!(
                            "{} and {}[{index}] of the resolution file both cover {}: give each \
                             {defect} instance one entry (data-migration.md §8.2)",
                            earlier.get().0.name(),
                            section.key(),
                            earlier.key()
                        )));
                    }
                    btree_map::Entry::Vacant(vacant) => {
                        let expected = entry.expected();
                        vacant.insert((
                            EntryRef {
                                section,
                                index,
                                expected,
                            },
                            entry,
                        ));
                    }
                }
            }
        }

        part.start(defect);
        let subjects: BTreeSet<Subject> = instances.keys().chain(entries.keys()).cloned().collect();
        let mut decisions = BTreeMap::new();
        for subject in subjects {
            let instance = instances.remove(&subject);
            let entry = entries.remove(&subject);
            let (record, decision) = match (instance, entry) {
                (Some(instance), entry) => {
                    let (record, decision) = judge(rule, instance, entry);
                    (record, Some(decision))
                }
                (None, Some((entry, _))) => (judge_entry(rule, rows, &subject, entry), None),
                // Every subject came from one of the two maps.
                (None, None) => continue,
            };
            let problem = record.problem();
            part.records.push(record);
            if let Some(decision) = decision {
                decisions.insert(subject, decision);
            }
            if self.mode == Mode::Apply
                && let Some(problem) = problem
            {
                return Err(DefectRefusal(problem));
            }
        }
        Ok(Decisions(decisions))
    }
}

/// Judges one instance of `rule`'s defect against the entry that covers its
/// subject, if any, by the defect's [`EntryPolicy`], and returns its record
/// and what to apply. The mode does not change the judgement, only whether
/// a problem stops the run ([`DefectDetector::run`]).
///
/// - `rule`: the defect's rule, which gives a valid entry's resolution.
/// - `instance`: the instance, as the rule found it.
/// - `entry`: the entry of the same subject, with its reference, or `None`.
fn judge<D: DefectRule>(
    rule: &D,
    instance: Instance<D::Resolution>,
    entry: Option<(EntryRef, &D::Entry)>,
) -> (DefectRecord, Decision<D::Resolution>) {
    let defect = D::DEFECT;
    let Instance {
        subject,
        rows,
        paths,
        found,
        proposed,
    } = instance;
    // The default, which applies when no valid entry resolves the instance.
    let fallback = || match &proposed {
        Some(proposal) => (
            Some(proposal.shown.clone()),
            Decision::Default(proposal.resolution.clone()),
        ),
        None => (None, Decision::Kept),
    };
    let (outcome, entry, shown, decision) = match entry {
        None => match defect.entry_policy() {
            EntryPolicy::Listed => (DefectOutcome::Unchanged, None, None, Decision::Kept),
            EntryPolicy::Optional => {
                let (shown, decision) = fallback();
                (DefectOutcome::Defaulted, None, shown, decision)
            }
            EntryPolicy::Required | EntryPolicy::SourceFix => {
                let (shown, decision) = fallback();
                (DefectOutcome::Uncovered, None, shown, decision)
            }
        },
        Some((entry, signed)) if found.matches(&entry.expected) => {
            let proposal = rule.resolution(signed);
            (
                DefectOutcome::Covered,
                Some(entry),
                Some(proposal.shown),
                Decision::Entry(proposal.resolution),
            )
        }
        Some((entry, _)) => {
            let (shown, decision) = fallback();
            (DefectOutcome::Changed, Some(entry), shown, decision)
        }
    };
    (
        DefectRecord {
            defect,
            subject,
            outcome,
            rows,
            paths,
            found: Some(found),
            entry,
            proposed: shown,
        },
        decision,
    )
}

/// Judges an entry of `rule`'s section whose subject the rule did not find,
/// and returns its record; it applies nothing.
///
/// Under [`EntryPolicy::SourceFix`] (D3) that is how a signed entry looks:
/// the editors fixed the row in the source, so the rule no longer finds it,
/// and the entry holds when the source has the value it pins
/// ([`DefectOutcome::FixedInSource`], with the subject's rows and paths, so
/// the sign-off sheet lists it). Every other entry for a subject the rule
/// does not find, and a D3 entry whose value differs or whose row is gone,
/// is stale ([`DefectOutcome::Stale`]).
///
/// - `rule`: the defect's rule, which reads what the precondition finds.
/// - `rows`: every source row the rule reads.
/// - `subject`: the entry's subject.
/// - `entry`: the entry, with its reference.
fn judge_entry<D: DefectRule>(
    rule: &D,
    rows: &[D::Row],
    subject: &Subject,
    entry: EntryRef,
) -> DefectRecord {
    let found = rule.found(rows, subject);
    let fixed = D::DEFECT.entry_policy() == EntryPolicy::SourceFix
        && found
            .as_ref()
            .is_some_and(|value| value.matches(&entry.expected));
    let (outcome, (rows, paths)) = if fixed {
        (
            DefectOutcome::FixedInSource,
            subject_rows_and_paths(subject),
        )
    } else {
        (DefectOutcome::Stale, (Vec::new(), Vec::new()))
    };
    DefectRecord {
        defect: D::DEFECT,
        subject: subject.clone(),
        outcome,
        rows,
        paths,
        found,
        entry: Some(entry),
        proposed: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::import_mysql::resolutions::{AuthorChoice, parse};
    use proptest::{collection, prelude::*};

    fn id(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).expect("a row id")
    }

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a date")
    }

    /// The cut-off of the synthetic D1 rule: posts created before it hold
    /// the early date (data-migration.md §8.1).
    const CUTOFF: &str = "2022-01-21";

    /// A synthetic legacy post: only what the synthetic rules read.
    #[derive(Debug, Clone)]
    struct Post {
        id: u32,
        /// `created_at` before the D1 cut-off.
        created_early: bool,
        date: Option<NaiveDate>,
        author: Option<u32>,
        published: bool,
    }

    fn post(id: u32, created: &str, date_text: Option<&str>) -> Post {
        Post {
            id,
            created_early: date(created) < date(CUTOFF),
            date: date_text.map(date),
            author: Some(1),
            published: true,
        }
    }

    /// A D1 rule over synthetic posts, as posts-import-dates will have it:
    /// a post created before the cut-off with a real date is one day early;
    /// the default and an entry move it one day later.
    struct DateShift;

    impl DefectRule for DateShift {
        type Row = Post;
        type Entry = PostDateShift;
        type Resolution = NaiveDate;
        const DEFECT: DefectId = DefectId::D1;

        fn detect(&self, rows: &[Post]) -> Vec<Instance<NaiveDate>> {
            rows.iter()
                .filter_map(|row| {
                    let found = row
                        .date
                        .filter(|day| row.created_early && *day != SENTINEL_DATE)?;
                    let next = found.succ_opt()?;
                    Some(
                        Instance::new(Subject::Post(id(row.id)), Value::Date(found)).with_proposed(
                            Proposal {
                                resolution: next,
                                shown: Value::Date(next),
                            },
                        ),
                    )
                })
                .collect()
        }

        fn found(&self, rows: &[Post], subject: &Subject) -> Option<Value> {
            let Subject::Post(post_id) = subject else {
                return None;
            };
            rows.iter()
                .find(|row| row.id == post_id.get())
                .map(|row| row.date.map_or(Value::Null, Value::Date))
        }

        fn resolution(&self, entry: &PostDateShift) -> Proposal<NaiveDate> {
            let next = entry.expect_date.succ_opt().unwrap_or(entry.expect_date);
            Proposal {
                resolution: next,
                shown: Value::Date(next),
            }
        }
    }

    /// A D2 rule over synthetic posts: a published post without an author.
    /// Its default is the generic author.
    struct MissingAuthor;

    impl DefectRule for MissingAuthor {
        type Row = Post;
        type Entry = PostAuthor;
        type Resolution = AuthorChoice;
        const DEFECT: DefectId = DefectId::D2;

        fn detect(&self, rows: &[Post]) -> Vec<Instance<AuthorChoice>> {
            rows.iter()
                .filter(|row| row.published && row.author.is_none())
                .map(|row| {
                    Instance::new(Subject::Post(id(row.id)), Value::Null).with_proposed(Proposal {
                        resolution: AuthorChoice::Generic,
                        shown: Value::Choice(Word::Generic),
                    })
                })
                .collect()
        }

        fn found(&self, rows: &[Post], subject: &Subject) -> Option<Value> {
            let Subject::Post(post_id) = subject else {
                return None;
            };
            rows.iter().find(|row| row.id == post_id.get()).map(|row| {
                row.author
                    .map_or(Value::Null, |author| Value::Id(id(author)))
            })
        }

        fn resolution(&self, entry: &PostAuthor) -> Proposal<AuthorChoice> {
            let shown = match entry.set_author {
                AuthorChoice::Generic => Value::Choice(Word::Generic),
                AuthorChoice::Author(author) => Value::Id(author),
            };
            Proposal {
                resolution: entry.set_author,
                shown,
            }
        }
    }

    /// A D4 rule over synthetic posts: a post without a body (here: without
    /// a date, which the synthetic post has instead), imported unchanged.
    struct Listed;

    impl DefectRule for Listed {
        type Row = Post;
        type Entry = NoEntry;
        type Resolution = ();
        const DEFECT: DefectId = DefectId::D4;

        fn detect(&self, rows: &[Post]) -> Vec<Instance<()>> {
            rows.iter()
                .filter(|row| row.date.is_none())
                .map(|row| Instance::new(Subject::Post(id(row.id)), Value::Null))
                .collect()
        }

        fn found(&self, _rows: &[Post], _subject: &Subject) -> Option<Value> {
            None
        }

        fn resolution(&self, entry: &NoEntry) -> Proposal<()> {
            match *entry {}
        }
    }

    /// The synthetic resolution file of these tests, signed for the posts
    /// of [`posts`]: post 12 and post 14 as signed, post 16 signed with a
    /// date the source no longer has.
    const SIGNED: &str = "version = 1
signed_off = 2026-11-20

[[post_date_shift]]
post_id = 12
expect_date = 2019-05-02

[[post_date_shift]]
post_id = 14
expect_date = 2020-02-29

[[post_author]]
post_id = 55
expect_author_id = \"none\"
set_author = 7
";

    fn file(text: &str) -> ResolutionFile {
        parse(text).unwrap_or_else(|errors| panic!("a valid synthetic file: {errors}"))
    }

    /// Posts 12 and 14 are early and signed; post 55 has no author.
    fn posts() -> Vec<Post> {
        let mut no_author = post(55, "2024-01-01", Some("2024-01-01"));
        no_author.author = None;
        vec![
            post(12, "2019-05-02", Some("2019-05-02")),
            post(14, "2020-02-29", Some("2020-02-29")),
            post(20, "2023-03-01", Some("2023-03-01")),
            no_author,
        ]
    }

    #[test]
    fn instances_that_signed_entries_cover_get_the_entries_resolution() {
        let signed = file(SIGNED);
        for mode in [Mode::DryRun, Mode::Apply] {
            let detector = DefectDetector::new(mode, &signed);
            let mut part = DefectsPart::default();
            let decisions = detector
                .run(&DateShift, &posts(), &mut part)
                .expect("every instance is signed");
            assert_eq!(decisions.len(), 2);
            assert_eq!(
                decisions.get(&Subject::Post(id(12))),
                Some(&Decision::Entry(date("2019-05-03")))
            );
            assert_eq!(
                decisions.resolution(&Subject::Post(id(14))),
                Some(&date("2020-03-01"))
            );
            assert_eq!(decisions.get(&Subject::Post(id(20))), None);
            let author = detector
                .run(&MissingAuthor, &posts(), &mut part)
                .expect("post 55 is signed");
            assert_eq!(
                author.get(&Subject::Post(id(55))),
                Some(&Decision::Entry(AuthorChoice::Author(id(7))))
            );
            assert_eq!(part.checked(), [DefectId::D1, DefectId::D2]);
            assert_eq!(part.count(DefectOutcome::Covered), 3);
            assert_eq!((part.uncovered(), part.refused()), (0, 0));
            let record = &part.records()[0];
            assert_eq!(record.subject, Subject::Post(id(12)));
            assert_eq!(record.rows, [id(12)]);
            assert_eq!(record.paths, [PublicPath::Post(id(12))]);
            assert_eq!(record.found, Some(Value::Date(date("2019-05-02"))));
            assert_eq!(
                record.entry.as_ref().map(EntryRef::name).as_deref(),
                Some("post_date_shift[1]")
            );
            assert_eq!(record.proposed, Some(Value::Date(date("2019-05-03"))));
            assert_eq!(part.conclude(), Ok(()));
            assert!(part.is_complete());
        }
    }

    #[test]
    fn an_uncovered_instance_stops_apply_and_takes_the_default_in_a_dry_run() {
        let unsigned = file("version = 1\n");
        let rows = vec![
            post(12, "2019-05-02", Some("2019-05-02")),
            post(30, "2021-06-01", Some("2021-06-01")),
        ];

        let mut part = DefectsPart::default();
        let refusal = DefectDetector::new(Mode::Apply, &unsigned)
            .run(&DateShift, &rows, &mut part)
            .expect_err("apply stops");
        assert_eq!(
            refusal.to_string(),
            "refusing to import: D1 post 12 is not covered by the resolution file (its source \
             value is 2019-05-02): add a post_date_shift entry for it once the editors have \
             signed it (data-migration.md §8.2)"
        );
        // The first problem stops apply: post 30 was not judged.
        assert_eq!(part.records().len(), 1);
        assert_eq!(part.uncovered(), 1);

        let mut part = DefectsPart::default();
        let decisions = DefectDetector::new(Mode::DryRun, &unsigned)
            .run(&DateShift, &rows, &mut part)
            .expect("a dry run goes on");
        assert_eq!(
            decisions.get(&Subject::Post(id(12))),
            Some(&Decision::Default(date("2019-05-03")))
        );
        assert_eq!(
            decisions.resolution(&Subject::Post(id(30))),
            Some(&date("2021-06-02"))
        );
        assert_eq!(part.uncovered(), 2);
        assert_eq!(
            part.records()[1].proposed,
            Some(Value::Date(date("2021-06-02")))
        );
        let problems = part.conclude().expect_err("the dry run fails at its end");
        assert!(part.is_complete());
        assert_eq!((problems.uncovered, problems.refused), (2, 0));
        assert_eq!(
            problems.to_string(),
            "the run found 2 defect problems (data-migration.md §4.2, §11.1): 2 instances that \
             the resolution file does not cover, and 0 entries whose precondition does not hold. \
             An apply run stops at the first of them; this dry run went on with the proposed \
             defaults, and the Defects part of the report lists every one, signoff.md every \
             instance for the editors:\n- D1 post 12 is not covered by the resolution file (its \
             source value is 2019-05-02): add a post_date_shift entry for it once the editors \
             have signed it (data-migration.md §8.2)\n- D1 post 30 is not covered by the \
             resolution file (its source value is 2021-06-01): add a post_date_shift entry for \
             it once the editors have signed it (data-migration.md §8.2)"
        );
    }

    #[test]
    fn an_uncovered_instance_without_a_default_keeps_its_source_value_in_a_dry_run() {
        // A rule with no default: the D1 rule's instances without their
        // proposal.
        struct NoDefault;
        impl DefectRule for NoDefault {
            type Row = Post;
            type Entry = PostDateShift;
            type Resolution = NaiveDate;
            const DEFECT: DefectId = DefectId::D1;
            fn detect(&self, rows: &[Post]) -> Vec<Instance<NaiveDate>> {
                DateShift
                    .detect(rows)
                    .into_iter()
                    .map(|instance| Instance {
                        proposed: None,
                        ..instance
                    })
                    .collect()
            }
            fn found(&self, rows: &[Post], subject: &Subject) -> Option<Value> {
                DateShift.found(rows, subject)
            }
            fn resolution(&self, entry: &PostDateShift) -> Proposal<NaiveDate> {
                DateShift.resolution(entry)
            }
        }
        let unsigned = file("version = 1\n");
        let mut part = DefectsPart::default();
        let decisions = DefectDetector::new(Mode::DryRun, &unsigned)
            .run(&NoDefault, &posts(), &mut part)
            .expect("a dry run goes on");
        assert_eq!(decisions.get(&Subject::Post(id(12))), Some(&Decision::Kept));
        assert_eq!(decisions.resolution(&Subject::Post(id(12))), None);
        assert_eq!(part.records()[0].proposed, None);
        assert_eq!(part.uncovered(), 2);
    }

    #[test]
    fn a_precondition_mismatch_names_defect_row_expected_and_found() {
        // Post 12's date was corrected in the old admin after the sign-off;
        // it is still a D1 instance, now with another date.
        let signed = file(SIGNED);
        let mut rows = posts();
        rows[0].date = Some(date("2019-05-04"));
        let message = "D1 post 12: the precondition of post_date_shift[1] does not hold: \
                       expect_date is 2019-05-02, but the source has 2019-05-04. The row changed \
                       since the sign-off: sign the entry again (data-migration.md §8.2)";

        let mut part = DefectsPart::default();
        let refusal = DefectDetector::new(Mode::Apply, &signed)
            .run(&DateShift, &rows, &mut part)
            .expect_err("apply stops");
        assert_eq!(refusal, DefectRefusal(message.to_owned()));
        assert_eq!(part.refused(), 1);

        let mut part = DefectsPart::default();
        let decisions = DefectDetector::new(Mode::DryRun, &signed)
            .run(&DateShift, &rows, &mut part)
            .expect("a dry run goes on");
        // The proposed default, from the found date, not the entry's.
        assert_eq!(
            decisions.get(&Subject::Post(id(12))),
            Some(&Decision::Default(date("2019-05-05")))
        );
        assert_eq!(
            decisions.get(&Subject::Post(id(14))),
            Some(&Decision::Entry(date("2020-03-01")))
        );
        let record = &part.records()[0];
        assert_eq!(record.outcome, DefectOutcome::Changed);
        assert_eq!(record.problem().as_deref(), Some(message));
        let problems = part.conclude().expect_err("the dry run fails at its end");
        assert_eq!((problems.uncovered, problems.refused), (0, 1));
        assert_eq!(problems.lines, [message]);
    }

    #[test]
    fn an_entry_for_a_row_that_no_longer_has_the_defect_is_refused() {
        let signed = file(SIGNED);
        // Post 14's creation time no longer puts it before the cut-off (so
        // the rule no longer finds it, although its date is as signed), and
        // post 55 got an author in the old admin.
        let mut rows = posts();
        rows[1].created_early = false;
        rows[3].author = Some(9);

        let mut part = DefectsPart::default();
        let refusal = DefectDetector::new(Mode::Apply, &signed)
            .run(&DateShift, &rows, &mut part)
            .expect_err("apply stops");
        assert_eq!(
            refusal.to_string(),
            "refusing to import: D1 post 14: post_date_shift[2] covers it, but the D1 rule no \
             longer finds it (expect_date is 2020-02-29, the source has 2020-02-29): the row no \
             longer has the defect, so remove the entry or sign it again (data-migration.md §8.2)"
        );

        let mut part = DefectsPart::default();
        let detector = DefectDetector::new(Mode::DryRun, &signed);
        let decisions = detector
            .run(&DateShift, &rows, &mut part)
            .expect("a dry run goes on");
        // Nothing is applied to post 14: it is no instance.
        assert_eq!(decisions.get(&Subject::Post(id(14))), None);
        assert_eq!(decisions.len(), 1);
        let authors = detector
            .run(&MissingAuthor, &rows, &mut part)
            .expect("a dry run goes on");
        assert!(authors.is_empty());
        let stale: Vec<&DefectRecord> = part
            .records()
            .iter()
            .filter(|record| record.outcome == DefectOutcome::Stale)
            .collect();
        assert_eq!(stale.len(), 2);
        assert_eq!(stale[1].found, Some(Value::Id(id(9))));
        assert_eq!(
            stale[1].problem().as_deref(),
            Some(
                "D2 post 55: post_author[1] covers it, but the D2 rule no longer finds it \
                 (expect_author_id is none, the source has 9): the row no longer has the defect, \
                 so remove the entry or sign it again (data-migration.md §8.2)"
            )
        );
        assert!(stale.iter().all(|record| record.rows.is_empty()));
        assert_eq!(part.refused(), 2);
    }

    #[test]
    fn an_entry_for_a_row_that_is_gone_says_so() {
        let signed = file(SIGNED);
        let rows: Vec<Post> = posts().into_iter().filter(|row| row.id != 12).collect();
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::DryRun, &signed)
            .run(&DateShift, &rows, &mut part)
            .expect("a dry run goes on");
        let record = &part.records()[0];
        assert_eq!(
            (record.outcome, record.found.as_ref()),
            (DefectOutcome::Stale, None)
        );
        assert_eq!(
            record.problem().as_deref(),
            Some(
                "D1 post 12: post_date_shift[1] covers it, but the D1 rule no longer finds it \
                 (expect_date is 2019-05-02, the source has no such row): the row is gone, so \
                 remove the entry (data-migration.md §8.2)"
            )
        );
        assert!(record.rows.is_empty() && record.paths.is_empty());
        assert_eq!(record.proposed, None);
    }

    /// A synthetic legacy page as the D10 rule reads it.
    #[derive(Debug, Clone)]
    struct Page {
        id: u32,
        slug: &'static str,
        /// Whether a menu item or another page links to it.
        linked: bool,
        updated_at: DateTime<Utc>,
    }

    fn page(id: u32, slug: &'static str, linked: bool) -> Page {
        Page {
            id,
            slug,
            linked,
            updated_at: DateTime::from_timestamp(1_700_000_000 + i64::from(id), 0)
                .expect("in range"),
        }
    }

    /// A D10 rule over synthetic pages: a page linked from nowhere. Its
    /// default is to import it as published (parity), and an entry hides
    /// it.
    struct Unlinked;

    impl DefectRule for Unlinked {
        type Row = Page;
        type Entry = PageHidden;
        type Resolution = Word;
        const DEFECT: DefectId = DefectId::D10;

        fn detect(&self, rows: &[Page]) -> Vec<Instance<Word>> {
            rows.iter()
                .filter(|row| !row.linked)
                .map(|row| {
                    let instance =
                        Instance::new(Subject::Page(id(row.id)), Value::Instant(row.updated_at))
                            .with_proposed(Proposal {
                                resolution: Word::Published,
                                shown: Value::Choice(Word::Published),
                            });
                    match PageSlug::new(row.slug) {
                        Some(slug) => instance.with_paths(vec![PublicPath::Page(slug)]),
                        None => instance,
                    }
                })
                .collect()
        }

        fn found(&self, rows: &[Page], subject: &Subject) -> Option<Value> {
            let Subject::Page(page_id) = subject else {
                return None;
            };
            rows.iter()
                .find(|row| row.id == page_id.get())
                .map(|row| Value::Instant(row.updated_at))
        }

        fn resolution(&self, _entry: &PageHidden) -> Proposal<Word> {
            Proposal {
                resolution: Word::Hidden,
                shown: Value::Choice(Word::Hidden),
            }
        }
    }

    #[test]
    fn an_optional_defect_takes_its_default_without_an_entry_in_both_modes() {
        // Pages 5 and 6 are linked from nowhere; the editors list page 6 as
        // hidden (data-migration.md §8.1 D10), and page 5 is published.
        let hidden = file(
            "version = 1\n[[page_hidden]]\npage_id = 6\nexpect_updated_at = \
             2023-11-14T22:13:26Z\n",
        );
        let rows = vec![
            page(4, "rolunk", true),
            page(5, "felveteli", false),
            page(6, "archivum", false),
        ];
        for mode in [Mode::DryRun, Mode::Apply] {
            let mut part = DefectsPart::default();
            let decisions = DefectDetector::new(mode, &hidden)
                .run(&Unlinked, &rows, &mut part)
                .expect("an unlisted page is no problem");
            assert_eq!(
                decisions.get(&Subject::Page(id(5))),
                Some(&Decision::Default(Word::Published)),
                "{mode:?}"
            );
            assert_eq!(
                decisions.get(&Subject::Page(id(6))),
                Some(&Decision::Entry(Word::Hidden)),
                "{mode:?}"
            );
            assert_eq!(decisions.get(&Subject::Page(id(4))), None, "{mode:?}");
            let outcomes: Vec<DefectOutcome> =
                part.records().iter().map(|record| record.outcome).collect();
            assert_eq!(
                outcomes,
                [DefectOutcome::Defaulted, DefectOutcome::Covered],
                "{mode:?}"
            );
            let record = &part.records()[0];
            assert_eq!(record.proposed, Some(Value::Choice(Word::Published)));
            assert_eq!(
                record.paths,
                [PublicPath::Page(
                    PageSlug::new("felveteli").expect("a slug")
                )]
            );
            assert_eq!(record.problem(), None);
            assert_eq!((part.uncovered(), part.refused()), (0, 0));
            assert_eq!(part.conclude(), Ok(()), "{mode:?}");
        }

        // The empty list: every unlinked page is published, and the run
        // passes; a page the editors hid but that is now linked is stale.
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::Apply, &file("version = 1\n"))
            .run(&Unlinked, &rows, &mut part)
            .expect("the empty list publishes every page");
        assert_eq!(part.count(DefectOutcome::Defaulted), 2);
        let linked = vec![page(5, "felveteli", false), page(6, "archivum", true)];
        let mut part = DefectsPart::default();
        let refusal = DefectDetector::new(Mode::Apply, &hidden)
            .run(&Unlinked, &linked, &mut part)
            .expect_err("page 6 is linked now");
        assert_eq!(
            refusal.to_string(),
            "refusing to import: D10 page 6: page_hidden[1] covers it, but the D10 rule no longer \
             finds it (expect_updated_at is 2023-11-14T22:13:26Z, the source has \
             2023-11-14T22:13:26Z): the row no longer has the defect, so remove the entry or sign \
             it again (data-migration.md §8.2)"
        );
    }

    /// A synthetic legacy post as the D3 rule reads it.
    #[derive(Debug, Clone)]
    struct Indexed {
        id: u32,
        published: bool,
        index_image: Option<&'static str>,
    }

    /// The index image the editors set on post 70 in the synthetic tests.
    const CHOSEN: &str = "index/0123456789abcdef0123456789abcdef.jpg";

    /// A D3 rule over synthetic posts: a published post without an index
    /// image. It has no default: the editors choose the image and set it in
    /// the current admin before the final dump.
    struct MissingIndexImage;

    impl DefectRule for MissingIndexImage {
        type Row = Indexed;
        type Entry = PostIndexImage;
        type Resolution = ();
        const DEFECT: DefectId = DefectId::D3;

        fn detect(&self, rows: &[Indexed]) -> Vec<Instance<()>> {
            rows.iter()
                .filter(|row| row.published && row.index_image.is_none())
                .map(|row| Instance::new(Subject::Post(id(row.id)), Value::Null))
                .collect()
        }

        fn found(&self, rows: &[Indexed], subject: &Subject) -> Option<Value> {
            let Subject::Post(post_id) = subject else {
                return None;
            };
            rows.iter().find(|row| row.id == post_id.get()).map(|row| {
                row.index_image
                    .and_then(IndexImage::new)
                    .map_or(Value::Null, Value::IndexImage)
            })
        }

        fn resolution(&self, entry: &PostIndexImage) -> Proposal<()> {
            Proposal {
                resolution: (),
                shown: entry.expected(),
            }
        }
    }

    fn indexed(index_image: Option<&'static str>) -> Vec<Indexed> {
        vec![
            Indexed {
                id: 69,
                published: true,
                index_image: Some("index/ffffffffffffffffffffffffffffffff.png"),
            },
            Indexed {
                id: 70,
                published: true,
                index_image,
            },
        ]
    }

    #[test]
    fn a_source_fix_entry_holds_once_the_editors_fixed_the_row_as_signed() {
        // Post 70's entry pins the index image the editors set in the
        // current admin (data-migration.md §8.1 D3).
        let signed = file(&format!(
            "version = 1\n[[post_index_image]]\npost_id = 70\nexpect_index_image = \"{CHOSEN}\"\n"
        ));
        for mode in [Mode::DryRun, Mode::Apply] {
            let mut part = DefectsPart::default();
            let decisions = DefectDetector::new(mode, &signed)
                .run(&MissingIndexImage, &indexed(Some(CHOSEN)), &mut part)
                .expect("the post was fixed as signed");
            // Nothing to apply: the source holds the fix.
            assert!(decisions.is_empty(), "{mode:?}");
            let record = &part.records()[0];
            assert_eq!(record.outcome, DefectOutcome::FixedInSource, "{mode:?}");
            assert_eq!(record.problem(), None);
            assert_eq!(
                (record.rows.as_slice(), record.paths.as_slice()),
                ([id(70)].as_slice(), [PublicPath::Post(id(70))].as_slice())
            );
            assert_eq!(
                record.found,
                Some(Value::IndexImage(IndexImage::new(CHOSEN).expect("a path")))
            );
            assert_eq!(record.proposed, None);
            assert!(!record.outcome.is_instance());
            assert_eq!(part.instances(), 0);
            assert_eq!(part.conclude(), Ok(()), "{mode:?}");
        }
    }

    #[test]
    fn a_source_fix_entry_fails_while_the_fix_is_missing_or_differs() {
        let signed = file(&format!(
            "version = 1\n[[post_index_image]]\npost_id = 70\nexpect_index_image = \"{CHOSEN}\"\n"
        ));
        let other = "index/abcdefabcdefabcdefabcdefabcdef12.webp";
        let cases: [(Vec<Indexed>, &ResolutionFile, DefectOutcome, &str); 4] = [
            // Not fixed yet, nor signed.
            (
                indexed(None),
                &ResolutionFile::default(),
                DefectOutcome::Uncovered,
                "D3 post 70 is not covered by the resolution file (its source value is none): the \
                 editors fix it in the current admin before the final dump, then add a \
                 post_index_image entry with the fixed value once they have signed it \
                 (data-migration.md §8.1, §8.2)",
            ),
            // Signed, but the image was removed again.
            (
                indexed(None),
                &signed,
                DefectOutcome::Changed,
                "D3 post 70: the precondition of post_index_image[1] does not hold: \
                 expect_index_image is index/0123456789abcdef0123456789abcdef.jpg, but the source \
                 has none. The row changed since the sign-off: sign the entry again \
                 (data-migration.md §8.2)",
            ),
            // Signed, and another image set since.
            (
                indexed(Some(other)),
                &signed,
                DefectOutcome::Stale,
                "D3 post 70: the precondition of post_index_image[1], which pins the fix made in \
                 the source, does not hold: expect_index_image is \
                 index/0123456789abcdef0123456789abcdef.jpg, but the source has \
                 index/abcdefabcdefabcdefabcdefabcdef12.webp. The row changed since the sign-off: \
                 sign the entry again (data-migration.md §8.1, §8.2)",
            ),
            // Signed, and the post is gone.
            (
                indexed(None)
                    .into_iter()
                    .filter(|row| row.id != 70)
                    .collect(),
                &signed,
                DefectOutcome::Stale,
                "D3 post 70: post_index_image[1] covers it, but the D3 rule no longer finds it \
                 (expect_index_image is index/0123456789abcdef0123456789abcdef.jpg, the source has \
                 no such row): the row is gone, so remove the entry (data-migration.md §8.2)",
            ),
        ];
        for (rows, list, outcome, message) in cases {
            let mut part = DefectsPart::default();
            let refusal = DefectDetector::new(Mode::Apply, list)
                .run(&MissingIndexImage, &rows, &mut part)
                .expect_err("apply stops");
            assert_eq!(refusal, DefectRefusal(message.to_owned()));
            assert_eq!(part.records()[0].outcome, outcome);

            let mut part = DefectsPart::default();
            let decisions = DefectDetector::new(Mode::DryRun, list)
                .run(&MissingIndexImage, &rows, &mut part)
                .expect("a dry run goes on");
            // D3 has no default: an instance keeps its source value, and
            // the import's constraint refuses the post later.
            assert!(
                decisions
                    .iter()
                    .all(|(_, decision)| *decision == Decision::Kept)
            );
            let problems = part.conclude().expect_err("the dry run fails at its end");
            assert_eq!(problems.lines, [message]);
        }
    }

    #[test]
    fn every_defect_has_the_entry_policy_of_data_migration() {
        for defect in DefectId::ALL {
            let expected = match defect {
                DefectId::D3 => EntryPolicy::SourceFix,
                DefectId::D4 | DefectId::D7 => EntryPolicy::Listed,
                DefectId::D10 => EntryPolicy::Optional,
                DefectId::D1
                | DefectId::D2
                | DefectId::D5
                | DefectId::D6
                | DefectId::D8
                | DefectId::D9
                | DefectId::N7 => EntryPolicy::Required,
            };
            assert_eq!(defect.entry_policy(), expected, "{defect}");
            // Only a defect without a section has no entries.
            assert_eq!(
                defect.entry_policy() == EntryPolicy::Listed,
                defect.section().is_none(),
                "{defect}"
            );
        }
    }

    #[test]
    fn every_word_shows_as_itself() {
        let shown: Vec<&str> = Word::ALL.iter().map(|word| word.as_str()).collect();
        assert_eq!(shown, ["generic", "published", "hidden", "accepted"]);
        assert_eq!(Word::Generic.as_str(), GENERIC_AUTHOR);
        for word in Word::ALL {
            assert_eq!(Value::Choice(word).to_string(), word.as_str());
        }
    }

    #[test]
    fn a_defect_without_a_section_is_listed_and_imported_unchanged() {
        let signed = file(SIGNED);
        let rows = vec![post(40, "2020-01-01", None), post(41, "2020-01-01", None)];
        for mode in [Mode::DryRun, Mode::Apply] {
            let mut part = DefectsPart::default();
            let decisions = DefectDetector::new(mode, &signed)
                .run(&Listed, &rows, &mut part)
                .expect("nothing to refuse");
            assert_eq!(decisions.len(), 2);
            assert!(
                decisions
                    .iter()
                    .all(|(_, decision)| *decision == Decision::Kept)
            );
            assert_eq!(part.count(DefectOutcome::Unchanged), 2);
            assert!(
                part.records()
                    .iter()
                    .all(|record| record.problem().is_none())
            );
            assert_eq!(part.conclude(), Ok(()));
        }
    }

    #[test]
    fn a_rule_leaves_out_the_subjects_it_does_not_cover() {
        // N7 over posts only: the page's entry is not taken for a stale one.
        struct PostBodies;
        impl DefectRule for PostBodies {
            type Row = ();
            type Entry = N7Accept;
            type Resolution = ();
            const DEFECT: DefectId = DefectId::N7;
            fn detect(&self, _rows: &[()]) -> Vec<Instance<()>> {
                Vec::new()
            }
            fn found(&self, _rows: &[()], _subject: &Subject) -> Option<Value> {
                None
            }
            fn covers(&self, subject: &Subject) -> bool {
                matches!(subject, Subject::Post(_))
            }
            fn resolution(&self, _entry: &N7Accept) -> Proposal<()> {
                Proposal {
                    resolution: (),
                    shown: Value::Choice(Word::Accepted),
                }
            }
        }
        let digest = "0".repeat(64);
        let accepted = file(&format!(
            "version = 1\n[[N7-accept]]\npage_id = 50\nexpect_content_sha256 = \"{digest}\"\n"
        ));
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::Apply, &accepted)
            .run(&PostBodies, &[], &mut part)
            .expect("the page's entry is for the pages' run");
        assert_eq!(part.records(), []);
        assert_eq!(part.checked(), [DefectId::N7]);
    }

    #[test]
    fn a_rule_that_breaks_the_contract_is_refused_in_both_modes() {
        struct Twice;
        impl DefectRule for Twice {
            type Row = Post;
            type Entry = PostDateShift;
            type Resolution = NaiveDate;
            const DEFECT: DefectId = DefectId::D1;
            fn detect(&self, rows: &[Post]) -> Vec<Instance<NaiveDate>> {
                let mut found = DateShift.detect(rows);
                found.extend(DateShift.detect(rows));
                found
            }
            fn found(&self, rows: &[Post], subject: &Subject) -> Option<Value> {
                DateShift.found(rows, subject)
            }
            fn resolution(&self, entry: &PostDateShift) -> Proposal<NaiveDate> {
                DateShift.resolution(entry)
            }
        }
        struct WrongSection;
        impl DefectRule for WrongSection {
            type Row = Post;
            type Entry = PostDateShift;
            type Resolution = NaiveDate;
            const DEFECT: DefectId = DefectId::D5;
            fn detect(&self, _rows: &[Post]) -> Vec<Instance<NaiveDate>> {
                Vec::new()
            }
            fn found(&self, _rows: &[Post], _subject: &Subject) -> Option<Value> {
                None
            }
            fn resolution(&self, entry: &PostDateShift) -> Proposal<NaiveDate> {
                DateShift.resolution(entry)
            }
        }
        let signed = file(SIGNED);
        for mode in [Mode::DryRun, Mode::Apply] {
            let detector = DefectDetector::new(mode, &signed);
            let mut part = DefectsPart::default();
            assert_eq!(
                detector
                    .run(&Twice, &posts(), &mut part)
                    .expect_err("a subject twice")
                    .to_string(),
                "refusing to import: the D1 rule found post 12 twice: this is a defect of the \
                 import's code, not of the data"
            );
            assert_eq!(
                detector
                    .run(&WrongSection, &posts(), &mut part)
                    .expect_err("another section")
                    .to_string(),
                "refusing to import: the D5 rule reads the entries of post_date_shift, but D5 is \
                 resolved by post_sentinel_date: this is a defect of the import's code, not of \
                 the data"
            );
            // Neither ran: nothing was recorded.
            assert!(part.checked().is_empty() && part.records().is_empty());
        }
    }

    #[test]
    fn two_entries_for_one_subject_are_refused() {
        // Validation refuses this file; a value built in code can still
        // hold it.
        let mut doubled = file(SIGNED);
        doubled.post_date_shift.push(PostDateShift {
            post_id: id(12),
            expect_date: date("2019-05-02"),
        });
        let mut part = DefectsPart::default();
        assert_eq!(
            DefectDetector::new(Mode::DryRun, &doubled)
                .run(&DateShift, &posts(), &mut part)
                .expect_err("two entries")
                .to_string(),
            "refusing to import: post_date_shift[1] and post_date_shift[3] of the resolution file \
             both cover post 12: give each D1 instance one entry (data-migration.md §8.2)"
        );
    }

    #[test]
    fn every_section_entry_names_its_subject_and_precondition() {
        fn pair<E: SignedEntry>(file: &ResolutionFile) -> (Subject, Value) {
            let entry = &E::entries(file)[0];
            (entry.subject(), entry.expected())
        }
        let digest = "ab".repeat(32);
        let signed = file(&format!(
            "version = 1

[[post_date_shift]]
post_id = 12
expect_date = 2019-05-02

[[post_author]]
post_id = 55
expect_author_id = \"none\"
set_author = \"generic\"

[[post_index_image]]
post_id = 70
expect_index_image = \"index/0123456789abcdef0123456789abcdef.jpg\"

[[post_sentinel_date]]
post_id = 65
expect_date = 1970-01-01

[[canteen_duplicate]]
date = 2022-04-01
expect_ids = [398, 393]
keep = 398

[[label_duplicate]]
expect_ids = [9, 4]
keep = 9

[[menu_unpositioned]]
parent_id = \"none\"
expect_null_lft = [26, 29]
served_order = [29, 26]

[[page_hidden]]
page_id = 12
expect_updated_at = 2024-03-01T10:00:00Z

[[N7-accept]]
page_id = 50
expect_content_sha256 = \"{digest}\"
"
        ));
        let instant = DateTime::parse_from_rfc3339("2024-03-01T10:00:00Z")
            .expect("RFC 3339")
            .with_timezone(&Utc);
        assert_eq!(
            [
                pair::<PostDateShift>(&signed),
                pair::<PostAuthor>(&signed),
                pair::<PostIndexImage>(&signed),
                pair::<PostSentinelDate>(&signed),
                pair::<CanteenDuplicate>(&signed),
                pair::<LabelDuplicate>(&signed),
                pair::<MenuUnpositioned>(&signed),
                pair::<PageHidden>(&signed),
                pair::<N7Accept>(&signed),
            ],
            [
                (Subject::Post(id(12)), Value::Date(date("2019-05-02"))),
                (Subject::Post(id(55)), Value::Null),
                (
                    Subject::Post(id(70)),
                    Value::IndexImage(
                        IndexImage::new("index/0123456789abcdef0123456789abcdef.jpg")
                            .expect("an index image")
                    )
                ),
                (Subject::Post(id(65)), Value::Date(SENTINEL_DATE)),
                (
                    Subject::CanteenDate(date("2022-04-01")),
                    Value::Ids(vec![id(398), id(393)])
                ),
                (Subject::LabelGroup(id(4)), Value::Ids(vec![id(9), id(4)])),
                (Subject::MenuLevel(None), Value::Ids(vec![id(26), id(29)])),
                (Subject::Page(id(12)), Value::Instant(instant)),
                (Subject::Page(id(50)), Value::Sha256([0xab; 32])),
            ]
        );
        assert_eq!(NoEntry::entries(&signed), []);
        // Every section's entry type agrees with its defect.
        assert_eq!(PostDateShift::SECTION, DefectId::D1.section());
        assert_eq!(PostAuthor::SECTION, DefectId::D2.section());
        assert_eq!(PostIndexImage::SECTION, DefectId::D3.section());
        assert_eq!(PostSentinelDate::SECTION, DefectId::D5.section());
        assert_eq!(CanteenDuplicate::SECTION, DefectId::D6.section());
        assert_eq!(LabelDuplicate::SECTION, DefectId::D8.section());
        assert_eq!(MenuUnpositioned::SECTION, DefectId::D9.section());
        assert_eq!(PageHidden::SECTION, DefectId::D10.section());
        assert_eq!(N7Accept::SECTION, DefectId::N7.section());
        assert_eq!(NoEntry::SECTION, DefectId::D4.section());
        assert_eq!(NoEntry::SECTION, DefectId::D7.section());
    }

    #[test]
    fn a_defect_id_names_its_section_and_title() {
        for defect in DefectId::ALL {
            assert_eq!(defect.to_string(), defect.as_str());
            assert_ne!(defect.title(), "");
            if let Some(section) = defect.section() {
                assert_eq!(section.defect(), defect.as_str(), "{defect}");
            }
        }
        // Every section resolves exactly one defect.
        for section in Section::ALL {
            let defects: Vec<DefectId> = DefectId::ALL
                .into_iter()
                .filter(|defect| defect.section() == Some(section))
                .collect();
            assert_eq!(defects.len(), 1, "{section:?}");
        }
        assert_eq!(
            DefectId::ALL
                .iter()
                .filter(|defect| defect.section().is_none())
                .collect::<Vec<_>>(),
            [&DefectId::D4, &DefectId::D7]
        );
    }

    #[test]
    fn every_section_has_one_precondition_field() {
        for section in Section::ALL {
            let fields: Vec<&str> = section
                .fields()
                .iter()
                .copied()
                .filter(|field| field.starts_with("expect_"))
                .collect();
            assert_eq!(fields, [precondition_field(section)], "{section:?}");
        }
        assert_eq!(
            precondition_field(Section::N7Accept),
            "expect_content_sha256"
        );
    }

    #[test]
    fn subjects_values_and_paths_show_as_ids_dates_and_words() {
        assert_eq!(Subject::Post(id(12)).to_string(), "post 12");
        assert_eq!(Subject::Page(id(7)).to_string(), "page 7");
        assert_eq!(Subject::CanteenDay(id(307)).to_string(), "canteen day 307");
        assert_eq!(
            Subject::CanteenDate(date("2022-04-01")).to_string(),
            "canteen date 2022-04-01"
        );
        assert_eq!(Subject::LabelGroup(id(4)).to_string(), "label group 4");
        assert_eq!(
            Subject::MenuLevel(Some(id(15))).to_string(),
            "menu parent 15"
        );
        assert_eq!(Subject::MenuLevel(None).to_string(), "menu root level");

        assert_eq!(Value::Null.to_string(), "none");
        assert_eq!(Value::Id(id(55)).to_string(), "55");
        assert_eq!(Value::Ids(vec![id(398), id(393)]).to_string(), "[398, 393]");
        assert_eq!(Value::Date(date("0999-01-02")).to_string(), "0999-01-02");
        let instant = DateTime::parse_from_rfc3339("2024-03-01T10:00:00.5Z")
            .expect("RFC 3339")
            .with_timezone(&Utc);
        assert_eq!(
            Value::Instant(instant).to_string(),
            "2024-03-01T10:00:00.500Z"
        );
        assert_eq!(Value::Sha256([0x0f; 32]).to_string(), "0f".repeat(32));
        assert_eq!(Value::Choice(Word::Generic).to_string(), "generic");

        assert_eq!(PublicPath::Post(id(12)).to_string(), "/posts/12");
        let slug = PageSlug::new("felveteli-2025").expect("a slug");
        assert_eq!(slug.as_str(), "felveteli-2025");
        assert_eq!(
            PublicPath::Page(slug).to_string(),
            "/information/felveteli-2025"
        );
        for refused in ["", "Felveteli", "a b", "a/b", "ő", "a_b", &"a".repeat(192)] {
            assert_eq!(PageSlug::new(refused), None, "{refused:?}");
        }
        assert!(PageSlug::new(&"a".repeat(191)).is_some());

        let path = "index/0123456789abcdef0123456789abcdef.png";
        assert_eq!(
            IndexImage::new(path).map(|image| image.as_str().to_owned()),
            Some(path.to_owned())
        );
        assert_eq!(IndexImage::new("index/"), None);
        assert_eq!(IndexImage::new("Alice"), None);
    }

    #[test]
    fn ids_meet_a_precondition_as_a_set() {
        let listed = Value::Ids(vec![id(398), id(393)]);
        assert!(Value::Ids(vec![id(393), id(398)]).matches(&listed));
        assert!(!Value::Ids(vec![id(393)]).matches(&listed));
        assert!(!Value::Ids(vec![id(393), id(398), id(400)]).matches(&listed));
        assert!(Value::Null.matches(&Value::Null));
        assert!(!Value::Id(id(7)).matches(&Value::Null));
        assert!(!Value::Id(id(7)).matches(&Value::Ids(vec![id(7)])));
    }

    #[test]
    fn instances_take_their_rows_and_paths_from_the_subject() {
        let post: Instance<()> = Instance::new(Subject::Post(id(3)), Value::Null);
        assert_eq!(
            (post.rows, post.paths),
            (vec![id(3)], vec![PublicPath::Post(id(3))])
        );
        let page: Instance<()> = Instance::new(Subject::Page(id(4)), Value::Null);
        assert_eq!((page.rows, page.paths), (vec![id(4)], Vec::new()));
        let day: Instance<()> = Instance::new(Subject::CanteenDay(id(5)), Value::Null);
        assert_eq!(day.rows, [id(5)]);
        let date_instance: Instance<()> =
            Instance::new(Subject::CanteenDate(date("2022-04-01")), Value::Null)
                .with_rows(vec![id(393), id(398)])
                .with_proposed(Proposal {
                    resolution: (),
                    shown: Value::Id(id(398)),
                });
        assert_eq!(date_instance.rows, [id(393), id(398)]);
        assert_eq!(date_instance.paths, Vec::<PublicPath>::new());
        assert_eq!(
            date_instance.proposed.map(|proposal| proposal.shown),
            Some(Value::Id(id(398)))
        );
        let slug = PageSlug::new("rolunk").expect("a slug");
        let page: Instance<()> = Instance::new(Subject::Page(id(4)), Value::Null)
            .with_paths(vec![PublicPath::Page(slug.clone())]);
        assert_eq!(page.paths, [PublicPath::Page(slug)]);
        assert_eq!(id_list(&[id(1), id(22)]), "1, 22");
        assert_eq!(id_list(&[]), "");
    }

    #[test]
    fn outcomes_name_themselves_and_which_are_problems() {
        let table = [
            (DefectOutcome::Covered, "covered", false, true),
            (DefectOutcome::Unchanged, "unchanged", false, true),
            (DefectOutcome::Defaulted, "defaulted", false, true),
            (DefectOutcome::Uncovered, "uncovered", true, true),
            (DefectOutcome::Changed, "changed", true, true),
            (
                DefectOutcome::FixedInSource,
                "fixed_in_source",
                false,
                false,
            ),
            (DefectOutcome::Stale, "stale", true, false),
        ];
        assert_eq!(
            table.map(|(outcome, ..)| outcome),
            DefectOutcome::ALL,
            "every outcome, once"
        );
        for (outcome, name, problem, instance) in table {
            assert_eq!(outcome.as_str(), name);
            assert_eq!(outcome.is_problem(), problem, "{name}");
            assert_eq!(outcome.is_instance(), instance, "{name}");
        }
    }

    #[test]
    fn a_long_problem_list_is_cut_after_fifty_lines() {
        let unsigned = file("version = 1\n");
        let rows: Vec<Post> = (1..=60)
            .map(|number| post(number, "2020-01-01", Some("2020-01-01")))
            .collect();
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::DryRun, &unsigned)
            .run(&DateShift, &rows, &mut part)
            .expect("a dry run goes on");
        let text = part.conclude().expect_err("60 problems").to_string();
        assert!(
            text.starts_with("the run found 60 defect problems"),
            "{text}"
        );
        assert_eq!(text.matches("\n- D1 post ").count(), SHOWN_PROBLEMS);
        assert!(text.ends_with("\n- and 10 more"), "{text}");

        let rows = vec![post(1, "2020-01-01", Some("2020-01-01"))];
        let mut part = DefectsPart::default();
        DefectDetector::new(Mode::DryRun, &unsigned)
            .run(&DateShift, &rows, &mut part)
            .expect("a dry run goes on");
        let text = part.conclude().expect_err("one problem").to_string();
        assert!(
            text.starts_with(
                "the run found 1 defect problem (data-migration.md §4.2, §11.1): 1 instance that"
            ),
            "{text}"
        );
    }

    /// Synthetic posts with ids from 1 to 40: early or not, with or
    /// without a date and an author.
    fn any_posts() -> impl Strategy<Value = Vec<Post>> {
        collection::btree_map(
            1_u32..=40,
            (
                any::<bool>(),
                proptest::option::of(0_i64..20_000),
                proptest::option::of(1_u32..5),
                any::<bool>(),
            ),
            0..20,
        )
        .prop_map(|rows| {
            rows.into_iter()
                .map(|(id, (created_early, day, author, published))| Post {
                    id,
                    created_early,
                    date: day.map(|days| {
                        NaiveDate::from_ymd_opt(1990, 1, 1).expect("a date")
                            + chrono::TimeDelta::days(days)
                    }),
                    author,
                    published,
                })
                .collect()
        })
    }

    /// D1 and D2 entries for ids 1 to 40, with any dates: some match the
    /// posts, some do not, some cover posts without the defect.
    fn any_file() -> impl Strategy<Value = ResolutionFile> {
        (
            collection::btree_map(1_u32..=40, 0_i64..20_000, 0..15),
            collection::btree_set(1_u32..=40, 0..10),
        )
            .prop_map(|(shifts, authors)| ResolutionFile {
                post_date_shift: shifts
                    .into_iter()
                    .map(|(post_id, days)| PostDateShift {
                        post_id: id(post_id),
                        expect_date: NaiveDate::from_ymd_opt(1990, 1, 1).expect("a date")
                            + chrono::TimeDelta::days(days),
                    })
                    .collect(),
                post_author: authors
                    .into_iter()
                    .map(|post_id| PostAuthor {
                        post_id: id(post_id),
                        set_author: AuthorChoice::Generic,
                    })
                    .collect(),
                ..ResolutionFile::default()
            })
    }

    proptest! {
        #[test]
        fn every_instance_and_entry_is_judged_once_and_apply_stops_at_the_first_problem(
            rows in any_posts(),
            signed in any_file(),
        ) {
            let instances = DateShift.detect(&rows);
            let mut dry = DefectsPart::default();
            let decisions = DefectDetector::new(Mode::DryRun, &signed)
                .run(&DateShift, &rows, &mut dry)
                .expect("a dry run never stops on the data");

            // Every instance gets one decision, and nothing else does.
            prop_assert_eq!(decisions.len(), instances.len());
            for instance in &instances {
                prop_assert!(decisions.get(&instance.subject).is_some());
            }
            // One record per instance and per entry without an instance,
            // in subject order.
            let entry_subjects: BTreeSet<Subject> =
                signed.post_date_shift.iter().map(SignedEntry::subject).collect();
            let instance_subjects: BTreeSet<Subject> =
                instances.iter().map(|instance| instance.subject.clone()).collect();
            let expected: Vec<Subject> =
                instance_subjects.union(&entry_subjects).cloned().collect();
            let recorded: Vec<Subject> =
                dry.records().iter().map(|record| record.subject.clone()).collect();
            prop_assert_eq!(&recorded, &expected);
            prop_assert_eq!(
                dry.uncovered(),
                instance_subjects.difference(&entry_subjects).count()
            );
            prop_assert_eq!(
                dry.count(DefectOutcome::Stale),
                entry_subjects.difference(&instance_subjects).count()
            );
            // A covered instance applies its entry, every other one the
            // default, which the synthetic rule always has.
            for record in dry.records() {
                let decision = decisions.get(&record.subject);
                match record.outcome {
                    DefectOutcome::Covered => {
                        prop_assert!(matches!(decision, Some(Decision::Entry(_))));
                    }
                    DefectOutcome::Uncovered | DefectOutcome::Changed => {
                        prop_assert!(matches!(decision, Some(Decision::Default(_))));
                    }
                    DefectOutcome::Stale => prop_assert!(decision.is_none()),
                    DefectOutcome::Unchanged
                    | DefectOutcome::Defaulted
                    | DefectOutcome::FixedInSource => {
                        prop_assert!(false, "D1 needs an entry for every instance");
                    }
                }
                prop_assert_eq!(
                    record.problem().is_some(),
                    record.outcome.is_problem()
                );
            }

            // apply: the same records up to the first problem, which it
            // returns; or the same decisions when there is none.
            let mut applied = DefectsPart::default();
            let result = DefectDetector::new(Mode::Apply, &signed)
                .run(&DateShift, &rows, &mut applied);
            let first = dry.records().iter().position(|record| record.outcome.is_problem());
            match (first, result) {
                (None, Ok(apply_decisions)) => {
                    prop_assert_eq!(apply_decisions, decisions);
                    prop_assert_eq!(applied.records(), dry.records());
                }
                (Some(at), Err(refusal)) => {
                    prop_assert_eq!(applied.records(), &dry.records()[..=at]);
                    let message = dry.records()[at].problem().expect("a problem");
                    prop_assert_eq!(refusal, DefectRefusal(message));
                }
                (first, result) => prop_assert!(false, "{:?} {:?}", first, result.map(|d| d.len())),
            }

            // The dry run fails at its end exactly when it saw a problem.
            let problems = dry.records().iter().filter(|r| r.outcome.is_problem()).count();
            let concluded = dry.conclude();
            prop_assert!(dry.is_complete());
            match concluded {
                Ok(()) => prop_assert_eq!(problems, 0),
                Err(found) => {
                    prop_assert_eq!(found.lines.len(), problems);
                    prop_assert_eq!(found.uncovered + found.refused, problems);
                }
            }
        }

        #[test]
        fn a_slug_is_accepted_exactly_when_it_has_the_legacy_shape(slug in "[a-z0-9_ A-Z/-]{0,200}") {
            let shaped = !slug.is_empty()
                && slug.len() <= 191
                && slug.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
            let made = PageSlug::new(&slug);
            prop_assert_eq!(made.is_some(), shaped);
            if let Some(made) = made {
                let path = PublicPath::Page(made).to_string();
                prop_assert_eq!(path.strip_prefix("/information/"), Some(slug.as_str()));
            }
        }

        #[test]
        fn ids_match_exactly_their_own_set(
            left in collection::vec(1_u32..20, 0..8),
            right in collection::vec(1_u32..20, 0..8),
        ) {
            let as_value = |ids: &[u32]| Value::Ids(ids.iter().map(|number| id(*number)).collect());
            let same_set = left.iter().collect::<BTreeSet<_>>() == right.iter().collect::<BTreeSet<_>>();
            prop_assert_eq!(as_value(&left).matches(&as_value(&right)), same_set);
            prop_assert!(as_value(&left).matches(&as_value(&left)));
        }

        #[test]
        fn a_value_shows_without_line_breaks_or_markdown_structure(
            number in 1_u32..,
            numbers in collection::vec(1_u32.., 0..5),
            days in -100_000_i64..100_000,
            seconds in 0_i64..253_402_300_799,
            digest in any::<[u8; 32]>(),
            image in "index/[0-9a-f]{32}\\.[a-zA-Z0-9]{1,8}",
        ) {
            let day = NaiveDate::from_ymd_opt(2000, 1, 1).expect("a date") + chrono::TimeDelta::days(days);
            let instant = DateTime::from_timestamp(seconds, 0).expect("in range");
            let mut values = vec![
                Value::Null,
                Value::Id(id(number)),
                Value::Ids(numbers.iter().map(|n| id(*n)).collect()),
                Value::Date(day),
                Value::Instant(instant),
                Value::Sha256(digest),
                Value::IndexImage(IndexImage::new(&image).expect("the legacy shape")),
            ];
            values.extend(Word::ALL.map(Value::Choice));
            for value in values {
                let shown = value.to_string();
                prop_assert!(!shown.is_empty());
                prop_assert!(
                    shown.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"[], :.-+/".contains(&byte)),
                    "{}", shown
                );
            }
        }

        #[test]
        fn an_index_image_is_made_exactly_from_a_legacy_index_path(
            path in prop_oneof![
                "index/[0-9a-f]{32}\\.[a-zA-Z0-9]{1,8}",
                "(index/)?[0-9a-fA-F]{30,34}(\\.[a-zA-Z0-9./ ]{0,8})?",
                "[ -~]{0,48}",
                "index/[0-9a-f]{32}\\.[a-z0-9]{150,170}",
            ],
        ) {
            let made = IndexImage::new(&path);
            prop_assert_eq!(made.is_some(), is_index_image_path(&path));
            if let Some(made) = made {
                prop_assert_eq!(made.as_str(), path.as_str());
                prop_assert_eq!(Value::IndexImage(made).to_string(), path);
            }
        }

        #[test]
        fn every_policy_judges_instances_and_entries_and_apply_stops_at_the_dry_runs_first_problem(
            dates in testing::any_dates(),
            listed in testing::any_listed(),
            images in testing::any_index_images(),
            hidden in testing::any_hidden(),
            bodies in testing::any_bodies(),
            signed in testing::any_file(),
        ) {
            // One rule per policy: D1 and N7 need an entry for every
            // instance, D4 has none, D10's are optional, D3's pin a fix
            // made in the source.
            judged_by_policy(&dates, &[], &signed)?;
            judged_by_policy(&listed, &[], &signed)?;
            judged_by_policy(&images, &[], &signed)?;
            judged_by_policy(&hidden, &[], &signed)?;
            judged_by_policy(&bodies, &[], &signed)?;
        }
    }

    /// The kind of a value, to count which kinds a strategy reaches.
    fn kind(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Id(_) => "id",
            Value::Ids(_) => "ids",
            Value::Date(_) => "date",
            Value::Instant(_) => "instant",
            Value::Sha256(_) => "sha256",
            Value::IndexImage(_) => "index image",
            Value::Choice(word) => word.as_str(),
        }
    }

    /// The shared strategy of the report's and the sign-off sheet's
    /// property tests reaches every outcome and every kind of value, so
    /// their checks of every line are not vacuous.
    #[test]
    fn the_shared_part_strategy_reaches_every_outcome_and_every_kind_of_value() {
        use proptest::{
            strategy::ValueTree,
            test_runner::{Config, TestRunner},
        };
        let mut runner = TestRunner::new_with_rng(
            Config::default(),
            proptest::test_runner::TestRng::deterministic_rng(
                proptest::test_runner::RngAlgorithm::ChaCha,
            ),
        );
        let strategy = testing::any_part();
        let mut outcomes = BTreeSet::new();
        let mut kinds = BTreeSet::new();
        let mut defects = BTreeSet::new();
        for _ in 0..512 {
            let part = strategy.new_tree(&mut runner).expect("a part").current();
            for record in part.records() {
                outcomes.insert(record.outcome.as_str());
                defects.insert(record.defect);
                let values = [
                    record.found.as_ref(),
                    record.proposed.as_ref(),
                    record.entry.as_ref().map(|entry| &entry.expected),
                ];
                kinds.extend(values.into_iter().flatten().map(kind));
            }
        }
        let every_outcome: BTreeSet<&str> = DefectOutcome::ALL.iter().map(|o| o.as_str()).collect();
        assert_eq!(outcomes, every_outcome);
        let mut every_kind: BTreeSet<&str> = [
            "null",
            "id",
            "ids",
            "date",
            "instant",
            "sha256",
            "index image",
        ]
        .into_iter()
        .collect();
        every_kind.extend(Word::ALL.map(Word::as_str));
        assert_eq!(kinds, every_kind);
        assert_eq!(
            defects.into_iter().collect::<Vec<_>>(),
            [
                DefectId::D1,
                DefectId::D3,
                DefectId::D4,
                DefectId::D6,
                DefectId::D10,
                DefectId::N7
            ]
        );
    }

    /// Checks a run of `rule` over `rows` with `file` against the rules of
    /// the defect's [`EntryPolicy`], written out here independently of
    /// [`judge`] and [`judge_entry`] ([`expected_outcome`]): in a dry run,
    /// every instance and every entry gets one record of the expected
    /// outcome, in subject order, and every instance the expected decision
    /// ([`decided_as_expected`]); `apply` records the same up to the dry
    /// run's first problem and stops there with its message, or returns the
    /// same decisions when there is none; the dry run fails at its end
    /// exactly when it saw a problem ([`apply_stops_where_the_dry_run_first_failed`]).
    ///
    /// - `rule`: a synthetic rule.
    /// - `rows`: the rows it reads.
    /// - `file`: any resolution file.
    fn judged_by_policy<D>(
        rule: &D,
        rows: &[D::Row],
        file: &ResolutionFile,
    ) -> Result<(), TestCaseError>
    where
        D: DefectRule,
        D::Resolution: PartialEq + fmt::Debug,
    {
        let instances: BTreeMap<Subject, Instance<D::Resolution>> = rule
            .detect(rows)
            .into_iter()
            .map(|instance| (instance.subject.clone(), instance))
            .collect();
        let entries: BTreeMap<Subject, Value> = D::Entry::entries(file)
            .iter()
            .filter(|entry| rule.covers(&entry.subject()))
            .map(|entry| (entry.subject(), entry.expected()))
            .collect();
        let mut dry = DefectsPart::default();
        let decisions = DefectDetector::new(Mode::DryRun, file)
            .run(rule, rows, &mut dry)
            .map_err(|refusal| TestCaseError::fail(refusal.to_string()))?;
        let subjects: BTreeSet<&Subject> = instances.keys().chain(entries.keys()).collect();
        prop_assert_eq!(dry.records().len(), subjects.len());
        prop_assert_eq!(decisions.len(), instances.len());
        for (record, subject) in dry.records().iter().zip(subjects) {
            prop_assert_eq!(&record.subject, subject);
            prop_assert_eq!(record.defect, D::DEFECT);
            let instance = instances.get(subject);
            let outcome = expected_outcome(rule, rows, subject, instance, entries.get(subject));
            prop_assert_eq!(record.outcome, outcome, "{}", subject);
            prop_assert_eq!(record.problem().is_some(), outcome.is_problem());
            decided_as_expected(record, instance, decisions.get(subject))?;
        }
        apply_stops_where_the_dry_run_first_failed(rule, rows, file, dry, &decisions)
    }

    /// The outcome of `subject` under the defect's [`EntryPolicy`], as
    /// ADR 0047 §4's table states it.
    ///
    /// - `rule`, `rows`: the rule and its rows, for what an entry's
    ///   precondition finds where the rule found no instance.
    /// - `subject`: the subject.
    /// - `instance`: the rule's instance of it, if any.
    /// - `expected`: the precondition of its entry, if any.
    fn expected_outcome<D: DefectRule>(
        rule: &D,
        rows: &[D::Row],
        subject: &Subject,
        instance: Option<&Instance<D::Resolution>>,
        expected: Option<&Value>,
    ) -> DefectOutcome {
        let policy = D::DEFECT.entry_policy();
        match (instance, expected) {
            (Some(_), None) => match policy {
                EntryPolicy::Listed => DefectOutcome::Unchanged,
                EntryPolicy::Optional => DefectOutcome::Defaulted,
                EntryPolicy::Required | EntryPolicy::SourceFix => DefectOutcome::Uncovered,
            },
            (Some(instance), Some(expected)) if instance.found.matches(expected) => {
                DefectOutcome::Covered
            }
            (Some(_), Some(_)) => DefectOutcome::Changed,
            (None, Some(expected)) => {
                let holds = rule
                    .found(rows, subject)
                    .is_some_and(|found| found.matches(expected));
                if policy == EntryPolicy::SourceFix && holds {
                    DefectOutcome::FixedInSource
                } else {
                    DefectOutcome::Stale
                }
            }
            (None, None) => unreachable!("every subject has an instance or an entry"),
        }
    }

    /// Checks what `record` proposes and what the run decided for its
    /// subject: the entry's resolution for a covered instance, the default
    /// (or the source value) for one that no valid entry resolves, the
    /// source value for a listed one, and nothing for an entry without an
    /// instance.
    ///
    /// - `record`: the dry run's record.
    /// - `instance`: the rule's instance of the record's subject, if any.
    /// - `decision`: the run's decision for the subject, if any.
    fn decided_as_expected<R>(
        record: &DefectRecord,
        instance: Option<&Instance<R>>,
        decision: Option<&Decision<R>>,
    ) -> Result<(), TestCaseError>
    where
        R: Clone + PartialEq + fmt::Debug,
    {
        match (record.outcome, instance) {
            (DefectOutcome::Covered, Some(_)) => {
                prop_assert!(matches!(decision, Some(Decision::Entry(_))));
                prop_assert!(record.proposed.is_some());
            }
            (DefectOutcome::Unchanged, Some(_)) => {
                prop_assert_eq!(decision, Some(&Decision::Kept));
                prop_assert_eq!(&record.proposed, &None);
            }
            (
                DefectOutcome::Defaulted | DefectOutcome::Uncovered | DefectOutcome::Changed,
                Some(instance),
            ) => {
                let (shown, expected) =
                    instance
                        .proposed
                        .as_ref()
                        .map_or((None, Decision::Kept), |proposal| {
                            (
                                Some(proposal.shown.clone()),
                                Decision::Default(proposal.resolution.clone()),
                            )
                        });
                prop_assert_eq!(decision, Some(&expected));
                prop_assert_eq!(&record.proposed, &shown);
            }
            (DefectOutcome::FixedInSource | DefectOutcome::Stale, None) => {
                prop_assert!(decision.is_none());
                prop_assert_eq!(&record.proposed, &None);
            }
            (outcome, instance) => prop_assert!(
                false,
                "{} for an instance: {}",
                outcome.as_str(),
                instance.is_some()
            ),
        }
        Ok(())
    }

    /// Checks that `apply` over the same rule, rows and file records what
    /// the dry run recorded up to its first problem and stops there with
    /// that problem's message, or returns the dry run's decisions when it
    /// had none; and that the dry run fails at its end exactly when it saw
    /// a problem, naming each.
    ///
    /// - `rule`, `rows`, `file`: what the dry run ran.
    /// - `dry`: the dry run's part, which this concludes.
    /// - `decisions`: the dry run's decisions.
    fn apply_stops_where_the_dry_run_first_failed<D>(
        rule: &D,
        rows: &[D::Row],
        file: &ResolutionFile,
        mut dry: DefectsPart,
        decisions: &Decisions<D::Resolution>,
    ) -> Result<(), TestCaseError>
    where
        D: DefectRule,
        D::Resolution: PartialEq + fmt::Debug,
    {
        let mut applied = DefectsPart::default();
        let result = DefectDetector::new(Mode::Apply, file).run(rule, rows, &mut applied);
        let first = dry
            .records()
            .iter()
            .position(|record| record.outcome.is_problem());
        match (first, result) {
            (None, Ok(apply_decisions)) => {
                prop_assert_eq!(&apply_decisions, decisions);
                prop_assert_eq!(applied.records(), dry.records());
            }
            (Some(at), Err(refusal)) => {
                prop_assert_eq!(applied.records(), &dry.records()[..=at]);
                let message = dry.records()[at].problem().expect("a problem");
                prop_assert_eq!(refusal, DefectRefusal(message));
            }
            (first, result) => {
                prop_assert!(
                    false,
                    "{:?} {:?}",
                    first,
                    result.map(|decisions| decisions.len())
                );
            }
        }
        let problems = dry
            .records()
            .iter()
            .filter(|record| record.outcome.is_problem())
            .count();
        match dry.conclude() {
            Ok(()) => prop_assert_eq!(problems, 0),
            Err(found) => prop_assert_eq!(found.lines.len(), problems),
        }
        Ok(())
    }
}

/// Synthetic rules and strategies for any instances, any resolution file
/// and any Defects part, which the tests of the detector, the run report
/// and the sign-off sheet share: a [`DefectsPart`] is only ever made by
/// [`DefectDetector::run`], so they make theirs the same way.
///
/// The rules cover every [`super::EntryPolicy`] and every kind of
/// [`Value`]: D1 (required entries; dates), D3 (a fix made in the source;
/// index image paths), D4 (no entries), D6 (required; ids), D10 (optional;
/// instants and every [`Word`]) and N7 (required; SHA-256 digests).
#[cfg(test)]
pub(crate) mod testing {
    use std::num::NonZeroU32;

    use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
    use proptest::{collection, prelude::*, sample};

    use super::{
        DefectDetector, DefectId, DefectRule, DefectsPart, IndexImage, Instance, NoEntry, PageSlug,
        Proposal, PublicPath, SignedEntry, Subject, Value, Word,
    };
    use crate::domain::import_mysql::{
        options::Mode,
        resolutions::{
            BodyRow, CanteenDuplicate, N7Accept, PageHidden, PostDateShift, PostIndexImage,
            ResolutionFile,
        },
    };

    /// A rule that finds exactly the instances it was given, for the
    /// defect whose entries are `E` (D1, D3, D4, D6, D10 or N7); an entry
    /// resolves its instance to its own precondition, and is shown so.
    #[derive(Debug)]
    pub struct Given<E> {
        /// The instances [`DefectRule::detect`] returns.
        pub instances: Vec<Instance<Value>>,
        /// What [`DefectRule::found`] reads for every subject.
        pub found: Option<Value>,
        marker: std::marker::PhantomData<E>,
    }

    impl<E> Given<E> {
        /// A rule that finds `instances` and reads `found` for every
        /// subject it does not find.
        #[must_use]
        pub fn new(instances: Vec<Instance<Value>>, found: Option<Value>) -> Self {
            Self {
                instances,
                found,
                marker: std::marker::PhantomData,
            }
        }
    }

    macro_rules! given_rule {
        ($entry:ty, $defect:expr) => {
            impl DefectRule for Given<$entry> {
                type Row = ();
                type Entry = $entry;
                type Resolution = Value;
                const DEFECT: DefectId = $defect;

                fn detect(&self, _rows: &[()]) -> Vec<Instance<Value>> {
                    self.instances.clone()
                }

                fn found(&self, _rows: &[()], _subject: &Subject) -> Option<Value> {
                    self.found.clone()
                }

                fn resolution(&self, entry: &$entry) -> Proposal<Value> {
                    Proposal {
                        resolution: entry.expected(),
                        shown: entry.expected(),
                    }
                }
            }
        };
    }

    given_rule!(PostDateShift, DefectId::D1);
    given_rule!(PostIndexImage, DefectId::D3);
    given_rule!(NoEntry, DefectId::D4);
    given_rule!(CanteenDuplicate, DefectId::D6);
    given_rule!(PageHidden, DefectId::D10);
    given_rule!(N7Accept, DefectId::N7);

    /// The index image paths of the synthetic D3 posts and entries: few, so
    /// that a found image often equals an entry's.
    const IMAGES: [&str; 2] = [
        "index/0123456789abcdef0123456789abcdef.jpg",
        "index/fedcba9876543210fedcba9876543210.png",
    ];

    fn id(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).expect("a row id")
    }

    fn day(days: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2015, 1, 1).expect("a date") + TimeDelta::days(days)
    }

    fn instant(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_400_000_000 + seconds, 0).expect("in range")
    }

    fn image(index: usize) -> IndexImage {
        IndexImage::new(IMAGES[index]).expect("the legacy shape")
    }

    /// A digest of one of four values, so that a found digest often equals
    /// an entry's.
    fn digest(seed: u8) -> [u8; 32] {
        [seed % 4; 32]
    }

    /// The body row `seed` names: a post for an even seed, a page for an
    /// odd one.
    fn body(seed: u32) -> BodyRow {
        if seed.is_multiple_of(2) {
            BodyRow::Post(id(seed / 2 + 1))
        } else {
            BodyRow::Page(id(seed / 2 + 1))
        }
    }

    /// The proposal that shows `value` and resolves to it.
    fn proposal(value: Value) -> Proposal<Value> {
        Proposal {
            resolution: value.clone(),
            shown: value,
        }
    }

    /// Any D1 instances (posts with a date, with or without a default),
    /// and what the rule reads for an entry that covers none.
    pub fn any_dates() -> impl Strategy<Value = Given<PostDateShift>> {
        (
            collection::btree_map(
                1_u32..30,
                (0_i64..40, proptest::option::of(0_i64..40)),
                0..6,
            ),
            proptest::option::of(0_i64..40),
        )
            .prop_map(|(posts, found)| {
                let instances = posts
                    .into_iter()
                    .map(|(post, (days, proposed))| {
                        let instance =
                            Instance::new(Subject::Post(id(post)), Value::Date(day(days)));
                        match proposed {
                            Some(next) => instance.with_proposed(proposal(Value::Date(day(next)))),
                            None => instance,
                        }
                    })
                    .collect();
                Given::new(instances, found.map(|days| Value::Date(day(days))))
            })
    }

    /// Any D3 instances: published posts without an index image, with no
    /// default, as D3 has none; and what the rule reads for a post it does
    /// not find: one of the synthetic images, no image, or no row.
    pub fn any_index_images() -> impl Strategy<Value = Given<PostIndexImage>> {
        (
            collection::btree_set(1_u32..30, 0..4),
            proptest::option::of(proptest::option::of(0_usize..IMAGES.len())),
        )
            .prop_map(|(posts, found)| {
                let instances = posts
                    .into_iter()
                    .map(|post| Instance::new(Subject::Post(id(post)), Value::Null))
                    .collect();
                let found = found.map(|image_index| {
                    image_index.map_or(Value::Null, |at| Value::IndexImage(image(at)))
                });
                Given::new(instances, found)
            })
    }

    /// Any D4 instances, which have no section.
    pub fn any_listed() -> impl Strategy<Value = Given<NoEntry>> {
        collection::btree_set(1_u32..30, 0..4).prop_map(|posts| {
            let instances = posts
                .into_iter()
                .map(|post| Instance::new(Subject::Post(id(post)), Value::Null))
                .collect();
            Given::new(instances, None)
        })
    }

    /// Any D6 instances: dates with their rows, with or without a default.
    pub fn any_duplicates() -> impl Strategy<Value = Given<CanteenDuplicate>> {
        collection::btree_map(
            0_i64..20,
            (
                collection::btree_set(1_u32..2_000, 2..4),
                proptest::option::of(1_u32..2_000),
            ),
            0..4,
        )
        .prop_map(|dates| {
            let instances = dates
                .into_iter()
                .map(|(days, (ids, kept))| {
                    let ids: Vec<NonZeroU32> = ids.into_iter().map(id).collect();
                    let instance =
                        Instance::new(Subject::CanteenDate(day(days)), Value::Ids(ids.clone()))
                            .with_rows(ids);
                    match kept {
                        Some(kept) => instance.with_proposed(proposal(Value::Id(id(kept)))),
                        None => instance,
                    }
                })
                .collect();
            Given::new(instances, None)
        })
    }

    /// Any D10 instances: pages with or without a slug of the legacy shape,
    /// each with any word as its default (D10's is `published`; the others
    /// put every word on the synthetic sheets), and what the rule reads for
    /// a page it does not find.
    pub fn any_hidden() -> impl Strategy<Value = Given<PageHidden>> {
        (
            collection::btree_map(
                1_u32..60,
                (
                    proptest::option::of("[a-z0-9-]{1,20}"),
                    0_i64..1_000,
                    sample::select(Word::ALL.as_slice()),
                ),
                0..4,
            ),
            proptest::option::of(0_i64..1_000),
        )
            .prop_map(|(pages, found)| {
                let instances = pages
                    .into_iter()
                    .map(|(page, (slug, seconds, word))| {
                        let instance = Instance::new(
                            Subject::Page(id(page)),
                            Value::Instant(instant(seconds)),
                        )
                        .with_proposed(proposal(Value::Choice(word)));
                        match slug.as_deref().and_then(PageSlug::new) {
                            Some(slug) => instance.with_paths(vec![PublicPath::Page(slug)]),
                            None => instance,
                        }
                    })
                    .collect();
                Given::new(
                    instances,
                    found.map(|seconds| Value::Instant(instant(seconds))),
                )
            })
    }

    /// Any N7 instances: post and page bodies with a digest, with or
    /// without the default `accepted`.
    pub fn any_bodies() -> impl Strategy<Value = Given<N7Accept>> {
        (
            collection::btree_map(0_u32..20, (any::<u8>(), any::<bool>()), 0..4),
            proptest::option::of(any::<u8>()),
        )
            .prop_map(|(bodies, found)| {
                let instances = bodies
                    .into_iter()
                    .map(|(seed, (sha, accepted))| {
                        let subject = match body(seed) {
                            BodyRow::Post(post) => Subject::Post(post),
                            BodyRow::Page(page) => Subject::Page(page),
                        };
                        let instance = Instance::new(subject, Value::Sha256(digest(sha)));
                        if accepted {
                            instance.with_proposed(proposal(Value::Choice(Word::Accepted)))
                        } else {
                            instance
                        }
                    })
                    .collect();
                Given::new(instances, found.map(|sha| Value::Sha256(digest(sha))))
            })
    }

    /// Any D1, D3, D6, D10 and N7 entries over the same ids, dates,
    /// instants, images and digests as the instances, so some match, some
    /// do not and some cover nothing.
    pub fn any_file() -> impl Strategy<Value = ResolutionFile> {
        (
            collection::btree_map(1_u32..30, 0_i64..40, 0..6),
            collection::btree_map(1_u32..30, 0_usize..IMAGES.len(), 0..4),
            collection::btree_map(0_i64..20, collection::btree_set(1_u32..2_000, 2..4), 0..3),
            collection::btree_map(1_u32..60, 0_i64..1_000, 0..3),
            collection::btree_map(0_u32..20, any::<u8>(), 0..4),
        )
            .prop_map(
                |(shifts, images, duplicates, hidden, accepted)| ResolutionFile {
                    post_date_shift: shifts
                        .into_iter()
                        .map(|(post, days)| PostDateShift {
                            post_id: id(post),
                            expect_date: day(days),
                        })
                        .collect(),
                    post_index_image: images
                        .into_iter()
                        .map(|(post, at)| PostIndexImage {
                            post_id: id(post),
                            expect_index_image: IMAGES[at].to_owned(),
                        })
                        .collect(),
                    canteen_duplicate: duplicates
                        .into_iter()
                        .map(|(days, ids)| {
                            let ids: Vec<NonZeroU32> = ids.into_iter().map(id).collect();
                            CanteenDuplicate {
                                date: day(days),
                                keep: ids[0],
                                expect_ids: ids,
                            }
                        })
                        .collect(),
                    page_hidden: hidden
                        .into_iter()
                        .map(|(page, seconds)| PageHidden {
                            page_id: id(page),
                            expect_updated_at: instant(seconds),
                        })
                        .collect(),
                    n7_accept: accepted
                        .into_iter()
                        .map(|(seed, sha)| N7Accept {
                            body: body(seed),
                            expect_content_sha256: digest(sha),
                        })
                        .collect(),
                    ..ResolutionFile::default()
                },
            )
    }

    /// Any part that a run of the six synthetic rules can leave: any
    /// instances of D1 (posts), D3 (posts), D4 (posts, no section), D6
    /// (canteen dates), D10 (pages, with or without a slug) and N7 (post
    /// and page bodies), any entries of their sections that match some and
    /// miss others, either mode, and either a run that reached its end or
    /// one that stopped.
    pub fn any_part() -> impl Strategy<Value = DefectsPart> {
        (
            (any_dates(), any_index_images(), any_listed()),
            (any_duplicates(), any_hidden(), any_bodies()),
            any_file(),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(
                |((dates, images, listed), (duplicates, hidden, bodies), file, apply, ends)| {
                    let mode = if apply { Mode::Apply } else { Mode::DryRun };
                    let detector = DefectDetector::new(mode, &file);
                    let mut part = DefectsPart::default();
                    // An apply run stops at its first problem, as the import
                    // does; the part keeps what it recorded.
                    let ran = detector
                        .run(&dates, &[], &mut part)
                        .and_then(|_| detector.run(&images, &[], &mut part))
                        .and_then(|_| detector.run(&listed, &[], &mut part))
                        .and_then(|_| detector.run(&duplicates, &[], &mut part))
                        .and_then(|_| detector.run(&hidden, &[], &mut part))
                        .and_then(|_| detector.run(&bodies, &[], &mut part));
                    if ran.is_ok() && ends {
                        let _ = part.conclude();
                    }
                    part
                },
            )
    }
}
