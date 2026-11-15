//! The defect resolution file of the legacy import: its TOML schema, its
//! parser and its validation (ADR 0017 §3, data-migration.md §8.2;
//! research: DECISIONS R9, D12).
//!
//! The editors decide, and sign off, how the import resolves the data
//! defects of data-migration.md §8.1 that need a decision. The decisions live
//! in a versioned file in the repository, `data/import/resolutions.toml`,
//! which the image ships as `/app/data/import/resolutions.toml`, the default
//! of the task's `resolutions` argument. The nightly rehearsal, the dress
//! rehearsal and the cut-over window all read the same reviewed bytes, and
//! CI validates them (`tests/image_data/`).
//!
//! # What the file holds, and what it never holds
//!
//! The repository is public, so the file holds only ids, dates, enum choices
//! and source values (or the SHA-256 of one): no names of people, and no
//! content (no titles, bodies, slugs, label or dish names). Who signed and
//! when is recorded in the private runbook; the file only gains the date of
//! the sign-off, [`ResolutionFile::signed_off`].
//!
//! Every entry carries a **precondition**: the source value the editors saw
//! when they signed, in the fields named `expect_*`. The defect detectors
//! compare it with the source on every run, and a row that changed since
//! the sign-off stops the import until its entry is signed again
//! (data-migration.md §4.2). Validation here runs before the source is read
//! (§4.3), so it checks only what the file itself can show: whether a row
//! id exists in the source, or still has its defect, is the detectors'
//! precondition check.
//!
//! # The schema
//!
//! ```toml
//! version = 1                    # required: the schema version, SCHEMA_VERSION
//! signed_off = 2026-11-20        # optional: the date the editors signed this list
//!
//! [[post_date_shift]]            # D1: the post's date moves one day later
//! post_id = 12
//! expect_date = 2019-05-02       # precondition: posts_data.date at sign-off
//!
//! [[post_author]]                # D2: a published post without an author
//! post_id = 55
//! expect_author_id = "none"      # precondition: no author at sign-off
//! set_author = "generic"         # or an author id: set_author = 7
//!
//! [[post_index_image]]           # D3: the index image editors set in the old admin
//! post_id = 70
//! expect_index_image = "index/0123456789abcdef0123456789abcdef.jpg"
//!
//! [[post_sentinel_date]]         # D5: a draft's 1970 sentinel date becomes NULL
//! post_id = 65
//! expect_date = 1970-01-01       # precondition: always the sentinel
//!
//! [[canteen_duplicate]]          # D6: one canteen day kept per duplicate date
//! date = 2022-04-01
//! expect_ids = [393, 398]        # precondition: the rows on that date
//! keep = 398
//!
//! [[label_duplicate]]            # D8: one label kept per duplicate-name group
//! expect_ids = [4, 9]            # precondition: the labels of the group
//! keep = 4
//!
//! [[menu_unpositioned]]          # D9: the served order of the unpositioned items
//! parent_id = 15                 # or "none" for the root level
//! expect_null_lft = [26, 29, 30, 31]   # precondition: the items without lft
//! served_order = [26, 30, 29, 31]
//!
//! [[page_hidden]]                # D10: a page imported as hidden
//! page_id = 12
//! expect_updated_at = 2024-03-01T10:00:00Z   # precondition: pages.updated_at
//!
//! [[N7-accept]]                  # N7: the sanitiser's unplanned removals accepted
//! post_id = 91                   # or page_id = 50
//! expect_content_sha256 = "<64 lower-case hex digits>"
//! ```
//!
//! [`Section`] lists the sections with their defect ids and fields. A
//! section may also be written as an inline array of inline tables. Ids are
//! the legacy rows' ids, which the import keeps: whole numbers from 1 to
//! 4294967295, the range of the source's `int unsigned` ids. Dates are TOML
//! local dates and instants TOML offset date-times in UTC, written without
//! quotes. TOML has no null, so the string `"none"` ([`NONE`]) writes SQL
//! NULL where an entry needs it. D2's `expect_author_id` and D5's
//! `expect_date` can only hold one value, the defect itself, and are still
//! written out, so that every entry states its precondition and a detector
//! names expected and found the same way for every defect.
//!
//! D4 (posts without a body) and D7 (canteen days outside the 2-3 dish
//! rule) have no section: their rows are imported unchanged
//! (data-migration.md §8.1, ADR 0017 §3), so no entry changes how one is
//! imported, and the report lists them. The editors' D4 confirmation that
//! no body is missing is part of the sign-off that `signed_off` dates.
//!
//! # Validation
//!
//! [`parse`] reads the text and validates it in one pass and collects every
//! problem, so that one run names them all, each with the entry and the
//! field ([`ResolutionError`]). It refuses: text that is not TOML; a missing
//! or other `version` (checked first: a file of another schema is not read
//! any further); an unknown key or field; a value of the wrong type or out
//! of range; a precondition that cannot hold for its defect; a `keep` that
//! is not among `expect_ids`; a served order that is not a permutation of
//! the unpositioned items; and an entry for a row, date, group or parent
//! that an earlier entry of its section already covers, or a post in both
//! D1 and D5. [`ResolutionFile::to_toml`] writes a file back in a canonical
//! layout, which [`parse`] reads to the same value.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use sha2::{Digest as _, Sha256};
use toml::{
    Table, Value,
    value::{Datetime, Offset},
};

/// The schema version that this build reads and writes: the value of the
/// file's `version` key.
pub const SCHEMA_VERSION: i64 = 1;

/// The largest resolution file the task reads, in bytes: 1 MiB, far more
/// than a list that covers every defect of the 2026-10-23 dump needs (about
/// two hundred entries, under 20 KB). The task refuses a larger file before
/// it parses anything, so a wrong path (a dump, say) fails at once instead
/// of filling the pod's memory.
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// The sentinel date that backend2's fix 13 wrote into undated drafts, which
/// D5 turns back into NULL (data-migration.md §8.1).
pub const SENTINEL_DATE: NaiveDate =
    NaiveDate::from_ymd_opt(1970, 1, 1).expect("1970-01-01 is a calendar date");

/// The string that writes SQL NULL, which TOML cannot: D2's
/// `expect_author_id` and the root level of D9's `parent_id`.
pub const NONE: &str = "none";

/// D2's `set_author` choice of the generic author (`Szerkesztőség`,
/// data-migration.md §8.1), which the import reuses or creates.
pub const GENERIC_AUTHOR: &str = "generic";

/// How the messages describe a valid row id.
const ROW_ID_RULE: &str = "a whole number from 1 to 4294967295";

/// The keys of the file's top level, in the order [`ResolutionFile::to_toml`]
/// writes them.
const TOP_LEVEL_KEYS: [&str; 11] = [
    "version",
    "signed_off",
    "post_date_shift",
    "post_author",
    "post_index_image",
    "post_sentinel_date",
    "canteen_duplicate",
    "label_duplicate",
    "menu_unpositioned",
    "page_hidden",
    "N7-accept",
];

/// How many problems the text of [`ResolutionErrors`] shows; the rest are
/// counted. A file of the size limit could otherwise fill a log with
/// thousands of lines.
const SHOWN_PROBLEMS: usize = 50;

/// A section of the file: an array of tables, one table per entry, that
/// resolves one defect of data-migration.md §8.1 (or accepts N7 removals,
/// §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// D1: a post dated one day early gets +1 day.
    PostDateShift,
    /// D2: a published post without an author gets one.
    PostAuthor,
    /// D3: the index image editors set on a published post in the old admin.
    PostIndexImage,
    /// D5: a draft's 1970 sentinel date becomes NULL.
    PostSentinelDate,
    /// D6: one canteen day is kept per duplicate date.
    CanteenDuplicate,
    /// D8: one label is kept per group of duplicate names.
    LabelDuplicate,
    /// D9: the served order of the menu items without a position.
    MenuUnpositioned,
    /// D10: a page that is imported as hidden.
    PageHidden,
    /// N7: a body whose unplanned sanitiser removals the editors accept.
    N7Accept,
}

impl Section {
    /// Every section, in the order of data-migration.md §8.1, which is also
    /// the order in which [`parse`] reports and [`ResolutionFile::to_toml`]
    /// writes them.
    pub const ALL: [Self; 9] = [
        Self::PostDateShift,
        Self::PostAuthor,
        Self::PostIndexImage,
        Self::PostSentinelDate,
        Self::CanteenDuplicate,
        Self::LabelDuplicate,
        Self::MenuUnpositioned,
        Self::PageHidden,
        Self::N7Accept,
    ];

    /// The section's key in the file.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::PostDateShift => "post_date_shift",
            Self::PostAuthor => "post_author",
            Self::PostIndexImage => "post_index_image",
            Self::PostSentinelDate => "post_sentinel_date",
            Self::CanteenDuplicate => "canteen_duplicate",
            Self::LabelDuplicate => "label_duplicate",
            Self::MenuUnpositioned => "menu_unpositioned",
            Self::PageHidden => "page_hidden",
            Self::N7Accept => "N7-accept",
        }
    }

    /// The id of the defect (data-migration.md §8.1) or normaliser step
    /// (§7.1) that the section resolves.
    #[must_use]
    pub const fn defect(self) -> &'static str {
        match self {
            Self::PostDateShift => "D1",
            Self::PostAuthor => "D2",
            Self::PostIndexImage => "D3",
            Self::PostSentinelDate => "D5",
            Self::CanteenDuplicate => "D6",
            Self::LabelDuplicate => "D8",
            Self::MenuUnpositioned => "D9",
            Self::PageHidden => "D10",
            Self::N7Accept => "N7",
        }
    }

    /// The fields an entry of the section may have, in the order
    /// [`ResolutionFile::to_toml`] writes them. Every one is required,
    /// except that an `N7-accept` entry has exactly one of `post_id` and
    /// `page_id`.
    #[must_use]
    pub const fn fields(self) -> &'static [&'static str] {
        match self {
            Self::PostDateShift | Self::PostSentinelDate => &["post_id", "expect_date"],
            Self::PostAuthor => &["post_id", "expect_author_id", "set_author"],
            Self::PostIndexImage => &["post_id", "expect_index_image"],
            Self::CanteenDuplicate => &["date", "expect_ids", "keep"],
            Self::LabelDuplicate => &["expect_ids", "keep"],
            Self::MenuUnpositioned => &["parent_id", "expect_null_lft", "served_order"],
            Self::PageHidden => &["page_id", "expect_updated_at"],
            Self::N7Accept => &["post_id", "page_id", "expect_content_sha256"],
        }
    }

    /// The fields of [`Self::fields`] as the messages name them.
    const fn fields_text(self) -> &'static str {
        match self {
            Self::PostDateShift | Self::PostSentinelDate => "post_id and expect_date",
            Self::PostAuthor => "post_id, expect_author_id and set_author",
            Self::PostIndexImage => "post_id and expect_index_image",
            Self::CanteenDuplicate => "date, expect_ids and keep",
            Self::LabelDuplicate => "expect_ids and keep",
            Self::MenuUnpositioned => "parent_id, expect_null_lft and served_order",
            Self::PageHidden => "page_id and expect_updated_at",
            Self::N7Accept => "post_id or page_id, and expect_content_sha256",
        }
    }

    /// The name of the section's `index`-th entry, counted from 1, as the
    /// messages write it: `post_author[2]`.
    fn entry(self, index: usize) -> String {
        format!("{}[{index}]", self.key())
    }
}

/// D1: a post whose date the legacy cast left one day early; the import
/// moves it one day later (data-migration.md §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostDateShift {
    /// The post (`posts_data.id`).
    pub post_id: NonZeroU32,
    /// Precondition: the post's `posts_data.date` at sign-off, before the
    /// shift. Never [`SENTINEL_DATE`], which is D5's.
    pub expect_date: NaiveDate,
}

/// D2: a published post without an author, and the author it gets. The
/// file also writes the precondition, `expect_author_id = "none"`, which for
/// D2 always holds: the post had no author at sign-off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostAuthor {
    /// The post (`posts_data.id`).
    pub post_id: NonZeroU32,
    /// The author the editors chose.
    pub set_author: AuthorChoice,
}

/// The author a D2 entry gives its post.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorChoice {
    /// The generic author `Szerkesztőség`, written `"generic"`: an existing
    /// author of exactly that name, or one the import creates.
    Generic,
    /// An existing author, by `posts_authors.id`.
    Author(NonZeroU32),
}

/// D3: the index image the editors set on a published post in the old admin
/// before the final dump. The entry resolves nothing itself: it pins the
/// image they chose, and the import refuses a published post without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostIndexImage {
    /// The post (`posts_data.id`).
    pub post_id: NonZeroU32,
    /// Precondition: the post's `index_image` at sign-off, the disk-relative
    /// path `index/<32 lower-case hex digits>.<extension>` (§6.1).
    pub expect_index_image: String,
}

/// D5: a draft dated with the 1970 sentinel, whose date becomes NULL. The
/// file also writes the precondition, `expect_date = 1970-01-01`
/// ([`SENTINEL_DATE`]), which for D5 always holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostSentinelDate {
    /// The draft (`posts_data.id`).
    pub post_id: NonZeroU32,
}

/// D6: a canteen date with more than one row, and the row that is kept; the
/// links of the others are not imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanteenDuplicate {
    /// The duplicated day (`canteen_data.date`).
    pub date: NaiveDate,
    /// Precondition: the ids of every row on that date at sign-off, at
    /// least two, as listed.
    pub expect_ids: Vec<NonZeroU32>,
    /// The row that is kept, one of `expect_ids`.
    pub keep: NonZeroU32,
}

/// D8: a group of labels whose names are equal under the legacy collation,
/// merged into the one that is kept (its colour included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDuplicate {
    /// Precondition: the ids of every label of the group at sign-off, at
    /// least two, as listed. The names are not written.
    pub expect_ids: Vec<NonZeroU32>,
    /// The label that is kept, one of `expect_ids`.
    pub keep: NonZeroU32,
}

/// D9: the menu items of one parent that have no position (`lft` is NULL),
/// and the order the live site serves them in, which the import keeps
/// (data-migration.md §6.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuUnpositioned {
    /// The parent item (`menu_items.id`), or `None` for the root level,
    /// written `"none"`.
    pub parent_id: Option<NonZeroU32>,
    /// Precondition: the ids of the parent's items without `lft` at
    /// sign-off, at least one, as listed.
    pub expect_null_lft: Vec<NonZeroU32>,
    /// The same ids in the order the live public API serves them.
    pub served_order: Vec<NonZeroU32>,
}

/// D10: a page the editors chose to import as hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageHidden {
    /// The page (`pages.id`).
    pub page_id: NonZeroU32,
    /// Precondition: the page's `pages.updated_at` at sign-off, which the
    /// legacy admin wrote as UTC with whole seconds (§6.10), so an edit
    /// after the sign-off stops the import.
    pub expect_updated_at: DateTime<Utc>,
}

/// N7: a body whose removals by the sanitiser that no earlier normaliser
/// step planned are accepted, so they do not stop the import (§7.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct N7Accept {
    /// The row whose body it is.
    pub body: BodyRow,
    /// Precondition: the SHA-256 of the row's source `content`, its bytes as
    /// the source session reads them, at sign-off. The body itself is never
    /// written into the file.
    pub expect_content_sha256: [u8; 32],
}

/// The row of a body: a post or a page, written `post_id` or `page_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BodyRow {
    /// A post (`posts_data.id`).
    Post(NonZeroU32),
    /// A page (`pages.id`).
    Page(NonZeroU32),
}

/// A parsed and validated resolution file.
///
/// [`parse`] is the only way a file's text becomes one, and it only returns
/// a value that passed every check. A value built in code is valid when
/// [`parse`] reads its [`Self::to_toml`] back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolutionFile {
    /// The date the editors signed the list, or `None` while it is a
    /// proposal. The nightly rehearsals run with a proposal; the cut-over
    /// runs with the signed list (cutover.md, P2).
    pub signed_off: Option<NaiveDate>,
    /// D1 entries, in file order.
    pub post_date_shift: Vec<PostDateShift>,
    /// D2 entries, in file order.
    pub post_author: Vec<PostAuthor>,
    /// D3 entries, in file order.
    pub post_index_image: Vec<PostIndexImage>,
    /// D5 entries, in file order.
    pub post_sentinel_date: Vec<PostSentinelDate>,
    /// D6 entries, in file order.
    pub canteen_duplicate: Vec<CanteenDuplicate>,
    /// D8 entries, in file order.
    pub label_duplicate: Vec<LabelDuplicate>,
    /// D9 entries, in file order.
    pub menu_unpositioned: Vec<MenuUnpositioned>,
    /// D10 entries, in file order.
    pub page_hidden: Vec<PageHidden>,
    /// `N7-accept` entries, in file order.
    pub n7_accept: Vec<N7Accept>,
}

impl ResolutionFile {
    /// The number of entries in every section together; 0 for the empty
    /// list.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.post_date_shift.len()
            + self.post_author.len()
            + self.post_index_image.len()
            + self.post_sentinel_date.len()
            + self.canteen_duplicate.len()
            + self.label_duplicate.len()
            + self.menu_unpositioned.len()
            + self.page_hidden.len()
            + self.n7_accept.len()
    }

    /// Writes the file in its canonical layout: `version`, `signed_off`
    /// when there is one, then every entry as a `[[section]]` table, the
    /// sections in [`Section::ALL`] order and the fields in
    /// [`Section::fields`] order, with no comments. [`parse`] reads the
    /// result back to `self` when `self` is valid.
    ///
    /// Every value is an integer, a date, an instant, an array of integers
    /// or one of a few fixed ASCII strings, so nothing needs escaping.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut blocks = vec![format!("version = {SCHEMA_VERSION}\n")];
        blocks.extend(
            self.signed_off
                .map(|date| format!("signed_off = {}\n", toml_date(date))),
        );
        blocks.extend(self.post_date_shift.iter().map(|entry| {
            format!(
                "\n[[post_date_shift]]\npost_id = {}\nexpect_date = {}\n",
                entry.post_id,
                toml_date(entry.expect_date)
            )
        }));
        blocks.extend(self.post_author.iter().map(|entry| {
            let choice = match entry.set_author {
                AuthorChoice::Generic => format!("\"{GENERIC_AUTHOR}\""),
                AuthorChoice::Author(id) => id.to_string(),
            };
            format!(
                "\n[[post_author]]\npost_id = {}\nexpect_author_id = \"{NONE}\"\nset_author = \
                 {choice}\n",
                entry.post_id
            )
        }));
        blocks.extend(self.post_index_image.iter().map(|entry| {
            format!(
                "\n[[post_index_image]]\npost_id = {}\nexpect_index_image = \"{}\"\n",
                entry.post_id, entry.expect_index_image
            )
        }));
        blocks.extend(self.post_sentinel_date.iter().map(|entry| {
            format!(
                "\n[[post_sentinel_date]]\npost_id = {}\nexpect_date = {}\n",
                entry.post_id,
                toml_date(SENTINEL_DATE)
            )
        }));
        blocks.extend(self.canteen_duplicate.iter().map(|entry| {
            format!(
                "\n[[canteen_duplicate]]\ndate = {}\nexpect_ids = {}\nkeep = {}\n",
                toml_date(entry.date),
                toml_ids(&entry.expect_ids),
                entry.keep
            )
        }));
        blocks.extend(self.label_duplicate.iter().map(|entry| {
            format!(
                "\n[[label_duplicate]]\nexpect_ids = {}\nkeep = {}\n",
                toml_ids(&entry.expect_ids),
                entry.keep
            )
        }));
        blocks.extend(self.menu_unpositioned.iter().map(|entry| {
            let parent = entry
                .parent_id
                .map_or_else(|| format!("\"{NONE}\""), |id| id.to_string());
            format!(
                "\n[[menu_unpositioned]]\nparent_id = {parent}\nexpect_null_lft = {}\nserved_order \
                 = {}\n",
                toml_ids(&entry.expect_null_lft),
                toml_ids(&entry.served_order)
            )
        }));
        blocks.extend(self.page_hidden.iter().map(|entry| {
            format!(
                "\n[[page_hidden]]\npage_id = {}\nexpect_updated_at = {}\n",
                entry.page_id,
                entry.expect_updated_at.format("%Y-%m-%dT%H:%M:%SZ")
            )
        }));
        blocks.extend(self.n7_accept.iter().map(|entry| {
            let row = match entry.body {
                BodyRow::Post(id) => format!("post_id = {id}"),
                BodyRow::Page(id) => format!("page_id = {id}"),
            };
            format!(
                "\n[[N7-accept]]\n{row}\nexpect_content_sha256 = \"{}\"\n",
                hex(&entry.expect_content_sha256)
            )
        }));
        blocks.concat()
    }
}

/// The SHA-256 of a resolution file's bytes, which the import's `import`
/// activity entries record (data-migration.md §5.4) and the run's start log
/// line shows.
///
/// The task takes it of the very bytes it validated, so what a run records
/// is the list it checked, not a second read of the path that a later
/// change of the file could make differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileDigest(pub [u8; 32]);

impl FileDigest {
    /// The SHA-256 of `bytes`.
    ///
    /// - `bytes`: the whole file as it was read, before it is decoded as
    ///   UTF-8, so the digest is the one `sha256sum` prints for the file.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

impl fmt::Display for FileDigest {
    /// 64 lower-case hex digits, as `sha256sum` writes them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

/// Where in the file a [`ResolutionError`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// The text as a whole: it is not TOML.
    File,
    /// A key of the file's top level: `version`, `signed_off` or an unknown
    /// key, as the file writes it (the message shows a key that is not a
    /// TOML bare key quoted and escaped).
    Key(String),
    /// A section as a whole, which is not an array of tables.
    Section(Section),
    /// One entry of a section.
    Entry {
        /// The section.
        section: Section,
        /// The entry's place among the section's tables, counted from 1.
        index: usize,
        /// The field the problem is in, or `None` for the entry as a whole;
        /// an unknown field as the file writes it, shown like [`Self::Key`].
        field: Option<String>,
    },
}

/// One problem of a resolution file, with where it is.
///
/// Its text names the entry and the field: `post_author[2] (D2), field
/// set_author: ...`, where `post_author[2]` is the second `[[post_author]]`
/// table of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionError {
    /// Where the problem is.
    pub location: Location,
    /// What is wrong, and what to write instead.
    pub problem: String,
}

impl ResolutionError {
    /// A problem in field `field` of entry `index` (from 1) of `section`.
    fn field(section: Section, index: usize, field: &str, problem: String) -> Self {
        Self {
            location: Location::Entry {
                section,
                index,
                field: Some(field.to_owned()),
            },
            problem,
        }
    }

    /// A problem of entry `index` (from 1) of `section` as a whole.
    fn entry(section: Section, index: usize, problem: String) -> Self {
        Self {
            location: Location::Entry {
                section,
                index,
                field: None,
            },
            problem,
        }
    }

    /// A problem at the top-level key `key`.
    fn key(key: &str, problem: String) -> Self {
        Self {
            location: Location::Key(key.to_owned()),
            problem,
        }
    }
}

/// A key or field name as a message shows it: as written when it is a TOML
/// bare key (ASCII letters, digits, `_` and `-`), which every name of the
/// schema is, and otherwise quoted and escaped like a Rust string.
///
/// - `name`: the key of the file's top level or of an entry; for an
///   unknown one, the file's own text.
///
/// An unknown name is the file's text, and a quoted TOML key may hold a
/// line break: shown as written, it would start a line that reads as
/// another problem, and break the one-problem-per-line layout that
/// [`ResolutionErrors`] and the task's log line rely on.
fn shown_name(name: &str) -> Cow<'_, str> {
    let bare = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if bare {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("{name:?}"))
    }
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            Location::File => write!(f, "{}", self.problem),
            Location::Key(key) => write!(f, "{}: {}", shown_name(key), self.problem),
            Location::Section(section) => {
                write!(
                    f,
                    "{} ({}): {}",
                    section.key(),
                    section.defect(),
                    self.problem
                )
            }
            Location::Entry {
                section,
                index,
                field,
            } => {
                write!(f, "{} ({})", section.entry(*index), section.defect())?;
                if let Some(field) = field {
                    write!(f, ", field {}", shown_name(field))?;
                }
                write!(f, ": {}", self.problem)
            }
        }
    }
}

impl std::error::Error for ResolutionError {}

/// Every problem [`parse`] found in a file, at least one, in the order of
/// [`parse`]'s checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionErrors(Vec<ResolutionError>);

impl ResolutionErrors {
    /// The problems, in order.
    #[must_use]
    pub fn problems(&self) -> &[ResolutionError] {
        &self.0
    }
}

impl fmt::Display for ResolutionErrors {
    /// `1 problem:` or `<n> problems:`, then one line per problem, the first
    /// [`SHOWN_PROBLEMS`] of them, and a count of the rest.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.0.len();
        write!(f, "{count} problem{}:", if count == 1 { "" } else { "s" })?;
        for problem in self.0.iter().take(SHOWN_PROBLEMS) {
            write!(f, "\n- {problem}")?;
        }
        if count > SHOWN_PROBLEMS {
            write!(f, "\n- and {} more", count - SHOWN_PROBLEMS)?;
        }
        Ok(())
    }
}

impl std::error::Error for ResolutionErrors {}

/// Parses and validates the text of a resolution file.
///
/// - `text`: the whole file, which the task read from the `resolutions`
///   path (at most [`MAX_FILE_BYTES`]).
///
/// The checks run in this order, and every problem is collected: the TOML
/// syntax (a file that is not TOML gets one problem); `version` (a missing
/// or other version stops there); the unknown top-level keys, in key
/// order; `signed_off`; then every section in [`Section::ALL`] order and
/// its entries in file order, each entry's fields in [`Section::fields`]
/// order, then its checks across fields, its unknown fields in key order
/// and the entries of its section (or of D1, for D5) that already cover
/// what it covers.
///
/// # Errors
///
/// [`ResolutionErrors`] with every problem found, each naming the entry and
/// the field and saying what to write instead.
pub fn parse(text: &str) -> Result<ResolutionFile, ResolutionErrors> {
    let table: Table = text
        .parse()
        .map_err(|error| ResolutionErrors(vec![syntax_error(text, &error)]))?;
    check_version(&table).map_err(|error| ResolutionErrors(vec![error]))?;

    let mut problems = Vec::new();
    let mut unknown: Vec<&String> = table
        .keys()
        .filter(|key| !TOP_LEVEL_KEYS.contains(&key.as_str()))
        .collect();
    unknown.sort();
    for key in unknown {
        problems.push(ResolutionError::key(
            key,
            "unknown key: the file has version, signed_off and the sections post_date_shift, \
             post_author, post_index_image, post_sentinel_date, canteen_duplicate, \
             label_duplicate, menu_unpositioned, page_hidden and N7-accept, and nothing else"
                .to_owned(),
        ));
    }
    let signed_off = table
        .get("signed_off")
        .and_then(|value| match date_value(value) {
            Ok(date) => Some(date),
            Err(problem) => {
                problems.push(ResolutionError::key("signed_off", problem));
                None
            }
        });

    // The sections in Section::ALL order, so the problems come in that
    // order too.
    let mut parser = Parser {
        table: &table,
        problems,
    };
    let post_date_shift = parser.post_date_shift();
    let post_author = parser.post_author();
    let post_index_image = parser.post_index_image();
    let post_sentinel_date = parser.post_sentinel_date(&post_date_shift);
    let canteen_duplicate = parser.canteen_duplicate();
    let label_duplicate = parser.label_duplicate();
    let menu_unpositioned = parser.menu_unpositioned();
    let page_hidden = parser.page_hidden();
    let n7_accept = parser.n7_accept();
    let file = ResolutionFile {
        signed_off,
        post_date_shift,
        post_author,
        post_index_image,
        post_sentinel_date,
        canteen_duplicate,
        label_duplicate,
        menu_unpositioned,
        page_hidden,
        n7_accept,
    };
    if parser.problems.is_empty() {
        Ok(file)
    } else {
        Err(ResolutionErrors(parser.problems))
    }
}

/// The problem of a text that is not TOML, with the line and column (from
/// 1, the column in characters) where the TOML parser stopped.
fn syntax_error(text: &str, error: &toml::de::Error) -> ResolutionError {
    let message = error.message().trim_end();
    let problem = match error.span() {
        Some(span) => {
            let before = text.get(..span.start).unwrap_or(text);
            let line = before.matches('\n').count() + 1;
            let column = before
                .rsplit_once('\n')
                .map_or(before, |(_, last)| last)
                .chars()
                .count()
                + 1;
            format!("not valid TOML at line {line}, column {column}: {message}")
        }
        None => format!("not valid TOML: {message}"),
    };
    ResolutionError {
        location: Location::File,
        problem,
    }
}

/// `version` must be [`SCHEMA_VERSION`]. A file of another schema is not
/// read any further, so it gets this one problem only.
fn check_version(table: &Table) -> Result<(), ResolutionError> {
    let problem = match table.get("version") {
        Some(Value::Integer(SCHEMA_VERSION)) => return Ok(()),
        None => format!("missing: the file starts with version = {SCHEMA_VERSION}"),
        Some(Value::Integer(other)) => format!(
            "{other} is not a schema version this build reads: it reads version \
             {SCHEMA_VERSION}"
        ),
        Some(other) => format!(
            "expected the integer {SCHEMA_VERSION}, found {}",
            describe(other)
        ),
    };
    Err(ResolutionError::key("version", problem))
}

/// The sections' parsing, which collects every problem into `problems`.
struct Parser<'a> {
    /// The file's top-level table.
    table: &'a Table,
    /// Every problem so far, in order.
    problems: Vec<ResolutionError>,
}

impl<'a> Parser<'a> {
    /// The tables of `section`, each with its index from 1; a missing
    /// section has none. A value that is not an array, and an element that
    /// is not a table, are problems.
    fn entries(&mut self, section: Section) -> Vec<(usize, &'a Table)> {
        let Some(value) = self.table.get(section.key()) else {
            return Vec::new();
        };
        let Value::Array(elements) = value else {
            self.problems.push(ResolutionError {
                location: Location::Section(section),
                problem: format!(
                    "expected an array of tables, written [[{}]], found {}",
                    section.key(),
                    describe(value)
                ),
            });
            return Vec::new();
        };
        let mut tables = Vec::with_capacity(elements.len());
        for (offset, element) in elements.iter().enumerate() {
            let index = offset + 1;
            match element {
                Value::Table(table) => tables.push((index, table)),
                other => self.problems.push(ResolutionError::entry(
                    section,
                    index,
                    format!("expected a table, found {}", describe(other)),
                )),
            }
        }
        tables
    }

    /// An entry reader for entry `index` of `section`.
    fn entry<'p>(
        &'p mut self,
        section: Section,
        index: usize,
        table: &'a Table,
    ) -> EntryReader<'a, 'p> {
        EntryReader {
            section,
            index,
            table,
            problems: &mut self.problems,
        }
    }

    /// The D1 section.
    fn post_date_shift(&mut self) -> Vec<PostDateShift> {
        let section = Section::PostDateShift;
        let mut seen = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let post_id = entry.row_id("post_id");
            let expect_date = entry.date("expect_date").and_then(|date| {
                if date == SENTINEL_DATE {
                    entry.problem(
                        "expect_date",
                        format!(
                            "{SENTINEL_DATE} is the sentinel date, which post_sentinel_date (D5) \
                             sets to NULL: D1 shifts real dates only"
                        ),
                    );
                    None
                } else {
                    Some(date)
                }
            });
            entry.unknown_fields();
            if let Some(post_id) = post_id {
                entry.once(&mut seen, post_id, "post_id", &format!("post {post_id}"));
            }
            if let (Some(post_id), Some(expect_date)) = (post_id, expect_date) {
                entries.push(PostDateShift {
                    post_id,
                    expect_date,
                });
            }
        }
        entries
    }

    /// The D2 section.
    fn post_author(&mut self) -> Vec<PostAuthor> {
        let section = Section::PostAuthor;
        let mut seen = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let post_id = entry.row_id("post_id");
            let precondition = entry.literal(
                "expect_author_id",
                NONE,
                "D2 covers published posts without an author",
            );
            let set_author = entry.with("set_author", |value| match value {
                Value::String(text) if text == GENERIC_AUTHOR => Ok(AuthorChoice::Generic),
                Value::Integer(number) => {
                    row_id_of(*number).map(AuthorChoice::Author).ok_or_else(|| {
                        format!(
                            "{number} is not an author id: write \"{GENERIC_AUTHOR}\" or \
                             {ROW_ID_RULE}"
                        )
                    })
                }
                other => Err(format!(
                    "expected \"{GENERIC_AUTHOR}\" or an author id, {ROW_ID_RULE}, found {}",
                    describe(other)
                )),
            });
            entry.unknown_fields();
            if let Some(post_id) = post_id {
                entry.once(&mut seen, post_id, "post_id", &format!("post {post_id}"));
            }
            if let (Some(post_id), Some(()), Some(set_author)) = (post_id, precondition, set_author)
            {
                entries.push(PostAuthor {
                    post_id,
                    set_author,
                });
            }
        }
        entries
    }

    /// The D3 section.
    fn post_index_image(&mut self) -> Vec<PostIndexImage> {
        let section = Section::PostIndexImage;
        let mut seen = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let post_id = entry.row_id("post_id");
            let expect_index_image = entry.with("expect_index_image", |value| match value {
                Value::String(path) if is_index_image_path(path) => Ok(path.clone()),
                other => Err(format!(
                    "expected the path index/<32 lower-case hex digits>.<extension>, at most \
                     191 characters, found {}",
                    describe(other)
                )),
            });
            entry.unknown_fields();
            if let Some(post_id) = post_id {
                entry.once(&mut seen, post_id, "post_id", &format!("post {post_id}"));
            }
            if let (Some(post_id), Some(expect_index_image)) = (post_id, expect_index_image) {
                entries.push(PostIndexImage {
                    post_id,
                    expect_index_image,
                });
            }
        }
        entries
    }

    /// The D5 section. `shifted` is the D1 section: a post cannot be in
    /// both, since D1 needs a real date and D5 the sentinel.
    fn post_sentinel_date(&mut self, shifted: &[PostDateShift]) -> Vec<PostSentinelDate> {
        let section = Section::PostSentinelDate;
        // The D1 posts as a set, so that a long D1 section costs one lookup
        // per post rather than a scan of the section.
        let shifted: BTreeSet<NonZeroU32> = shifted.iter().map(|entry| entry.post_id).collect();
        // Where each D1 post is, for the message: its place among the D1
        // entries that parsed is not its index in the file, so the D1
        // section is read again for the indices. A post that D1 lists twice
        // is named by its first entry, as D1's own check names it.
        let mut d1_index: BTreeMap<NonZeroU32, usize> = BTreeMap::new();
        let d1_tables = self
            .table
            .get(Section::PostDateShift.key())
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for (offset, value) in d1_tables.enumerate() {
            let Some(post_id) = value
                .get("post_id")
                .and_then(Value::as_integer)
                .and_then(row_id_of)
            else {
                continue;
            };
            if shifted.contains(&post_id) {
                d1_index.entry(post_id).or_insert(offset + 1);
            }
        }
        let mut seen = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let post_id = entry.row_id("post_id");
            let precondition = entry.with("expect_date", |value| match date_value(value) {
                Ok(date) if date == SENTINEL_DATE => Ok(()),
                Ok(date) => Err(format!(
                    "must be the sentinel {SENTINEL_DATE}, which D5 sets to NULL, found {date}: \
                     a real date that is one day early belongs in post_date_shift (D1)"
                )),
                Err(problem) => Err(problem),
            });
            entry.unknown_fields();
            if let Some(post_id) = post_id {
                if let Some(other) = d1_index.get(&post_id) {
                    entry.problem(
                        "post_id",
                        format!(
                            "post {post_id} is also in {} (D1): a post has either a real date \
                             that D1 shifts or the sentinel that D5 clears",
                            Section::PostDateShift.entry(*other)
                        ),
                    );
                }
                entry.once(&mut seen, post_id, "post_id", &format!("post {post_id}"));
            }
            if let (Some(post_id), Some(())) = (post_id, precondition) {
                entries.push(PostSentinelDate { post_id });
            }
        }
        entries
    }

    /// The D6 section.
    fn canteen_duplicate(&mut self) -> Vec<CanteenDuplicate> {
        let section = Section::CanteenDuplicate;
        let mut dates = Seen::default();
        let mut rows = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let date = entry.date("date");
            let expect_ids = entry.ids("expect_ids", 2, "row", "a duplicate date has");
            let keep = entry.keep(expect_ids.as_deref(), "row");
            entry.unknown_fields();
            if let Some(date) = date {
                entry.once(&mut dates, date, "date", &date.to_string());
            }
            for id in expect_ids.iter().flatten() {
                entry.once(&mut rows, *id, "expect_ids", &format!("row {id}"));
            }
            if let (Some(date), Some(expect_ids), Some(keep)) = (date, expect_ids, keep) {
                entries.push(CanteenDuplicate {
                    date,
                    expect_ids,
                    keep,
                });
            }
        }
        entries
    }

    /// The D8 section.
    fn label_duplicate(&mut self) -> Vec<LabelDuplicate> {
        let section = Section::LabelDuplicate;
        let mut labels = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let expect_ids = entry.ids("expect_ids", 2, "label", "a group of duplicate names has");
            let keep = entry.keep(expect_ids.as_deref(), "label");
            entry.unknown_fields();
            for id in expect_ids.iter().flatten() {
                entry.once(&mut labels, *id, "expect_ids", &format!("label {id}"));
            }
            if let (Some(expect_ids), Some(keep)) = (expect_ids, keep) {
                entries.push(LabelDuplicate { expect_ids, keep });
            }
        }
        entries
    }

    /// The D9 section.
    fn menu_unpositioned(&mut self) -> Vec<MenuUnpositioned> {
        let section = Section::MenuUnpositioned;
        let mut parents = Seen::default();
        let mut items = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let parent_id = entry.with("parent_id", |value| match value {
                Value::String(text) if text == NONE => Ok(None),
                Value::Integer(number) => row_id_of(*number).map(Some).ok_or_else(|| {
                    format!(
                        "{number} is not a menu item id: write {ROW_ID_RULE}, or \"{NONE}\" for \
                         the root level"
                    )
                }),
                other => Err(format!(
                    "expected a menu item id, {ROW_ID_RULE}, or \"{NONE}\" for the root level, \
                     found {}",
                    describe(other)
                )),
            });
            let expect_null_lft = entry.ids("expect_null_lft", 1, "item", "an entry lists");
            let served_order = entry
                .ids("served_order", 1, "item", "an entry lists")
                .and_then(|order| {
                    let expected = expect_null_lft.as_ref()?;
                    let mut sorted_order = order.clone();
                    sorted_order.sort_unstable();
                    let mut sorted_expected = expected.clone();
                    sorted_expected.sort_unstable();
                    if sorted_order == sorted_expected {
                        Some(order)
                    } else {
                        entry.problem(
                            "served_order",
                            format!(
                                "lists {}, which is not an order of expect_null_lft {}: list \
                                 each of those ids once, in the order the live site serves them",
                                toml_ids(&order),
                                toml_ids(expected)
                            ),
                        );
                        None
                    }
                });
            if let (Some(Some(parent)), Some(listed)) = (parent_id, expect_null_lft.as_ref())
                && listed.contains(&parent)
            {
                entry.problem(
                    "parent_id",
                    format!("{parent} is also in expect_null_lft: an item is not its own parent"),
                );
            }
            entry.unknown_fields();
            if let Some(parent) = parent_id {
                let name = parent.map_or_else(
                    || "the root level".to_owned(),
                    |parent| format!("parent {parent}"),
                );
                entry.once(&mut parents, parent, "parent_id", &name);
            }
            for id in expect_null_lft.iter().flatten() {
                entry.once(&mut items, *id, "expect_null_lft", &format!("item {id}"));
            }
            if let (Some(parent_id), Some(expect_null_lft), Some(served_order)) =
                (parent_id, expect_null_lft, served_order)
            {
                entries.push(MenuUnpositioned {
                    parent_id,
                    expect_null_lft,
                    served_order,
                });
            }
        }
        entries
    }

    /// The D10 section.
    fn page_hidden(&mut self) -> Vec<PageHidden> {
        let section = Section::PageHidden;
        let mut seen = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let page_id = entry.row_id("page_id");
            let expect_updated_at = entry.with("expect_updated_at", instant_value);
            entry.unknown_fields();
            if let Some(page_id) = page_id {
                entry.once(&mut seen, page_id, "page_id", &format!("page {page_id}"));
            }
            if let (Some(page_id), Some(expect_updated_at)) = (page_id, expect_updated_at) {
                entries.push(PageHidden {
                    page_id,
                    expect_updated_at,
                });
            }
        }
        entries
    }

    /// The `N7-accept` section.
    fn n7_accept(&mut self) -> Vec<N7Accept> {
        let section = Section::N7Accept;
        let mut seen = Seen::default();
        let mut entries = Vec::new();
        for (index, table) in self.entries(section) {
            let mut entry = self.entry(section, index, table);
            let body = match (table.contains_key("post_id"), table.contains_key("page_id")) {
                (true, false) => entry.row_id("post_id").map(BodyRow::Post),
                (false, true) => entry.row_id("page_id").map(BodyRow::Page),
                (has_post, _) => {
                    entry.problems.push(ResolutionError::entry(
                        section,
                        index,
                        format!(
                            "{}: give exactly one of post_id and page_id, the row whose body it is",
                            if has_post {
                                "has both post_id and page_id"
                            } else {
                                "has neither post_id nor page_id"
                            }
                        ),
                    ));
                    None
                }
            };
            let expect_content_sha256 = entry.with("expect_content_sha256", |value| {
                match value {
                    Value::String(text) => sha256_of(text),
                    _ => None,
                }
                .ok_or_else(|| {
                    format!(
                        "expected 64 lower-case hex digits, the SHA-256 of the source body, found \
                         {}",
                        describe(value)
                    )
                })
            });
            entry.unknown_fields();
            if let Some(body) = body {
                let (field, name) = match body {
                    BodyRow::Post(id) => ("post_id", format!("post {id}")),
                    BodyRow::Page(id) => ("page_id", format!("page {id}")),
                };
                entry.once(&mut seen, body, field, &name);
            }
            if let (Some(body), Some(expect_content_sha256)) = (body, expect_content_sha256) {
                entries.push(N7Accept {
                    body,
                    expect_content_sha256,
                });
            }
        }
        entries
    }
}

/// Where each key of a section was first seen: the index of its entry.
type Seen<K> = BTreeMap<K, usize>;

/// Reads the fields of one entry, recording a problem for each one that is
/// missing or wrong, and returning `None` for it.
struct EntryReader<'a, 'p> {
    /// The entry's section.
    section: Section,
    /// The entry's index in its section, from 1.
    index: usize,
    /// The entry's table.
    table: &'a Table,
    /// Where the problems go.
    problems: &'p mut Vec<ResolutionError>,
}

impl EntryReader<'_, '_> {
    /// Records `problem` in `field` of this entry.
    fn problem(&mut self, field: &str, problem: String) {
        self.problems.push(ResolutionError::field(
            self.section,
            self.index,
            field,
            problem,
        ));
    }

    /// Reads the required field `field` with `read`, which returns the value
    /// or the problem.
    fn with<T>(
        &mut self,
        field: &str,
        read: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Option<T> {
        let Some(value) = self.table.get(field) else {
            let problem = format!(
                "missing: every {} entry has {}",
                self.section.key(),
                self.section.fields_text()
            );
            self.problem(field, problem);
            return None;
        };
        match read(value) {
            Ok(read) => Some(read),
            Err(problem) => {
                self.problem(field, problem);
                None
            }
        }
    }

    /// The required row id `field`.
    fn row_id(&mut self, field: &str) -> Option<NonZeroU32> {
        self.with(field, row_id_value)
    }

    /// The required date `field`.
    fn date(&mut self, field: &str) -> Option<NaiveDate> {
        self.with(field, date_value)
    }

    /// The required field `field`, which must be the string `expected`;
    /// `why` says why it can only be that.
    fn literal(&mut self, field: &str, expected: &str, why: &str) -> Option<()> {
        self.with(field, |value| match value {
            Value::String(text) if text == expected => Ok(()),
            other => Err(format!(
                "must be \"{expected}\": {why}, found {}",
                describe(other)
            )),
        })
    }

    /// The required list of distinct row ids `field`, at least `minimum`
    /// long. `noun` names one id (`row`), and `rule` starts the sentence
    /// that states the minimum (`a duplicate date has`).
    fn ids(
        &mut self,
        field: &str,
        minimum: usize,
        noun: &str,
        rule: &str,
    ) -> Option<Vec<NonZeroU32>> {
        self.with(field, |value| {
            let Value::Array(elements) = value else {
                return Err(format!(
                    "expected an array of ids, each {ROW_ID_RULE}, found {}",
                    describe(value)
                ));
            };
            let mut ids = Vec::with_capacity(elements.len());
            // The ids already listed, beside the Vec that keeps their order:
            // a search of the Vec for every element would be quadratic, and
            // one array within MAX_FILE_BYTES can list about 150,000 ids.
            let mut listed = BTreeSet::new();
            for (offset, element) in elements.iter().enumerate() {
                let id = row_id_value(element)
                    .map_err(|problem| format!("element {}: {problem}", offset + 1))?;
                if !listed.insert(id) {
                    return Err(format!("lists {noun} {id} twice"));
                }
                ids.push(id);
            }
            if ids.len() < minimum {
                let plural = if ids.len() == 1 { "" } else { "s" };
                let minimum_plural = if minimum == 1 { "" } else { "s" };
                return Err(format!(
                    "lists {} {noun}{plural}: {rule} at least {minimum} {noun}{minimum_plural}",
                    ids.len()
                ));
            }
            Ok(ids)
        })
    }

    /// The required `keep` field: one of `listed`, the entry's `expect_ids`
    /// (`None` when they did not parse, which leaves only the id's own
    /// check). `noun` names one id.
    fn keep(&mut self, listed: Option<&[NonZeroU32]>, noun: &str) -> Option<NonZeroU32> {
        let keep = self.row_id("keep")?;
        let listed = listed?;
        if listed.contains(&keep) {
            Some(keep)
        } else {
            self.problem(
                "keep",
                format!(
                    "{noun} {keep} is not in expect_ids {}: keep one of the listed {noun}s",
                    toml_ids(listed)
                ),
            );
            None
        }
    }

    /// Records every key of the entry that is not one of its section's
    /// fields, in key order.
    fn unknown_fields(&mut self) {
        let mut unknown: Vec<&String> = self
            .table
            .keys()
            .filter(|key| !self.section.fields().contains(&key.as_str()))
            .collect();
        unknown.sort();
        for key in unknown {
            let problem = format!(
                "unknown field: every {} entry has {}, and nothing else",
                self.section.key(),
                self.section.fields_text()
            );
            self.problems.push(ResolutionError::field(
                self.section,
                self.index,
                key,
                problem,
            ));
        }
    }

    /// Records `key` as covered by this entry, or, when an earlier entry of
    /// the section covers it already, a problem in `field` that names it as
    /// `name` (`post 55`) and names that entry.
    fn once<K: Ord>(&mut self, seen: &mut Seen<K>, key: K, field: &str, name: &str) {
        match seen.get(&key) {
            Some(&earlier) if earlier != self.index => {
                let problem = format!(
                    "{name} is already in {}: one entry each",
                    self.section.entry(earlier)
                );
                self.problem(field, problem);
            }
            Some(_) => {}
            None => {
                seen.insert(key, self.index);
            }
        }
    }
}

/// A row id from a TOML integer: 1 to `u32::MAX`, the range of the
/// source's `int unsigned` ids.
fn row_id_of(number: i64) -> Option<NonZeroU32> {
    u32::try_from(number).ok().and_then(NonZeroU32::new)
}

/// A row id from a TOML value, or the problem.
fn row_id_value(value: &Value) -> Result<NonZeroU32, String> {
    match value {
        Value::Integer(number) => row_id_of(*number)
            .ok_or_else(|| format!("{number} is not a row id: write {ROW_ID_RULE}")),
        other => Err(format!(
            "expected a row id, {ROW_ID_RULE}, found {}",
            describe(other)
        )),
    }
}

/// A calendar day from a TOML local date (`2022-04-01`, no time, no
/// offset), or the problem.
fn date_value(value: &Value) -> Result<NaiveDate, String> {
    match value {
        Value::Datetime(Datetime {
            date: Some(date),
            time: None,
            offset: None,
        }) => NaiveDate::from_ymd_opt(
            i32::from(date.year),
            u32::from(date.month),
            u32::from(date.day),
        )
        .ok_or_else(|| format!("{date} is not a calendar date")),
        other => Err(format!(
            "expected a date such as 2022-04-01, without a time and without quotes, found {}",
            describe(other)
        )),
    }
}

/// An instant from a TOML offset date-time in UTC (`2024-03-01T10:00:00Z`,
/// or with `+00:00`), with whole seconds, or the problem.
fn instant_value(value: &Value) -> Result<DateTime<Utc>, String> {
    let expected = || {
        format!(
            "expected an instant in UTC such as 2024-03-01T10:00:00Z, without quotes, found {}",
            describe(value)
        )
    };
    let Value::Datetime(datetime) = value else {
        return Err(expected());
    };
    let (Some(date), Some(time)) = (datetime.date, datetime.time) else {
        return Err(expected());
    };
    let Some(offset) = datetime.offset else {
        return Err(format!(
            "{datetime} has no offset: write the instant in UTC, as in {datetime}Z"
        ));
    };
    if time.nanosecond.is_some_and(|nanosecond| nanosecond != 0) {
        return Err(format!(
            "{datetime} has a fraction of a second: the legacy updated_at holds whole seconds"
        ));
    }
    // TOML allows a leap second (:60), which no stored instant has.
    let local = NaiveDate::from_ymd_opt(
        i32::from(date.year),
        u32::from(date.month),
        u32::from(date.day),
    )
    .and_then(|day| {
        day.and_hms_opt(
            u32::from(time.hour),
            u32::from(time.minute),
            u32::from(time.second.unwrap_or(0)),
        )
    })
    .ok_or_else(|| format!("{datetime} is not an instant the source can hold"))?;
    let minutes = match offset {
        Offset::Z => 0,
        Offset::Custom { minutes } => minutes,
    };
    let utc = (local - TimeDelta::minutes(i64::from(minutes))).and_utc();
    if minutes == 0 {
        Ok(utc)
    } else {
        Err(format!(
            "{datetime} is not in UTC: write the same instant in UTC, {}",
            utc.format("%Y-%m-%dT%H:%M:%SZ")
        ))
    }
}

/// Whether `path` is a legacy index image path: `index/`, 32 lower-case hex
/// digits, `.` and an extension of ASCII letters and digits, at most 191
/// characters, the length of the `index_image` column (§6.1).
fn is_index_image_path(path: &str) -> bool {
    let Some(name) = path.strip_prefix("index/") else {
        return false;
    };
    let Some((stem, extension)) = name.split_once('.') else {
        return false;
    };
    path.len() <= 191
        && stem.len() == 32
        && stem
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && !extension.is_empty()
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// The SHA-256 written as 64 lower-case hex digits, or `None`.
fn sha256_of(text: &str) -> Option<[u8; 32]> {
    let digits = text.as_bytes();
    if digits.len() != 64 {
        return None;
    }
    let nibble = |digit: u8| match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    };
    let mut digest = [0_u8; 32];
    let (pairs, _) = digits.as_chunks::<2>();
    for (byte, &[high, low]) in digest.iter_mut().zip(pairs) {
        *byte = nibble(high)? << 4 | nibble(low)?;
    }
    Some(digest)
}

/// `bytes` as lower-case hex digits.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                DIGITS[usize::from(byte >> 4)],
                DIGITS[usize::from(byte & 0x0f)],
            ]
        })
        .map(char::from)
        .collect()
}

/// A date as a TOML local date.
fn toml_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Ids as a TOML array: `[393, 398]`.
fn toml_ids(ids: &[NonZeroU32]) -> String {
    let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
    format!("[{}]", ids.join(", "))
}

/// How the messages describe a value they refuse: its type, and the value
/// itself where it is short. A string is quoted and cut after 40
/// characters.
fn describe(value: &Value) -> String {
    match value {
        Value::String(text) => {
            let shown: String = text.chars().take(40).collect();
            if shown.len() < text.len() {
                format!("the string {shown:?} (cut)")
            } else {
                format!("the string {text:?}")
            }
        }
        Value::Integer(number) => format!("the integer {number}"),
        Value::Float(number) => format!("the float {number}"),
        Value::Boolean(flag) => format!("the boolean {flag}"),
        Value::Datetime(datetime) => match (datetime.date, datetime.time) {
            (Some(_), Some(_)) => format!("the date-time {datetime}"),
            (Some(_), None) => format!("the date {datetime}"),
            _ => format!("the time {datetime}"),
        },
        Value::Array(_) => "an array".to_owned(),
        Value::Table(_) => "a table".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection, prelude::*, sample::Index};

    fn id(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).expect("a row id")
    }

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a date")
    }

    fn instant(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("RFC 3339")
            .with_timezone(&Utc)
    }

    /// A digest with every byte equal to `byte`.
    fn digest(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    /// The problems of `text`, as their messages.
    fn problems(text: &str) -> Vec<String> {
        parse(text)
            .expect_err("the file is refused")
            .problems()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// The one problem of `text`.
    fn problem(text: &str) -> String {
        let mut problems = problems(text);
        assert_eq!(problems.len(), 1, "{problems:#?}");
        problems.remove(0)
    }

    /// `body` after `version = 1`.
    fn v1(body: &str) -> String {
        format!("version = 1\n{body}")
    }

    /// A file with one entry of every section, as an editor writes it.
    const FULL: &str = r#"
# A list with one entry of every section.
version = 1
signed_off = 2026-11-20

[[post_date_shift]]
post_id = 12
expect_date = 2019-05-02

[[post_author]]
post_id = 55
expect_author_id = "none"
set_author = "generic"

[[post_author]]
post_id = 61
expect_author_id = "none"
set_author = 7

[[post_index_image]]
post_id = 70
expect_index_image = "index/0123456789abcdef0123456789abcdef.JPG"

[[post_sentinel_date]]
post_id = 65
expect_date = 1970-01-01

[[canteen_duplicate]]
date = 2022-04-01
expect_ids = [393, 398]
keep = 398

[[label_duplicate]]
expect_ids = [4, 9, 31]
keep = 4

[[menu_unpositioned]]
parent_id = 15
expect_null_lft = [26, 29, 30, 31]
served_order = [26, 30, 29, 31]

[[menu_unpositioned]]
parent_id = "none"
expect_null_lft = [40]
served_order = [40]

[[page_hidden]]
page_id = 12
expect_updated_at = 2024-03-01T10:00:00Z

[[N7-accept]]
post_id = 91
expect_content_sha256 = "0101010101010101010101010101010101010101010101010101010101010101"

[[N7-accept]]
page_id = 91
expect_content_sha256 = "abababababababababababababababababababababababababababababababab"
"#;

    /// [`FULL`] as a value.
    fn full() -> ResolutionFile {
        ResolutionFile {
            signed_off: Some(day("2026-11-20")),
            post_date_shift: vec![PostDateShift {
                post_id: id(12),
                expect_date: day("2019-05-02"),
            }],
            post_author: vec![
                PostAuthor {
                    post_id: id(55),
                    set_author: AuthorChoice::Generic,
                },
                PostAuthor {
                    post_id: id(61),
                    set_author: AuthorChoice::Author(id(7)),
                },
            ],
            post_index_image: vec![PostIndexImage {
                post_id: id(70),
                expect_index_image: "index/0123456789abcdef0123456789abcdef.JPG".to_owned(),
            }],
            post_sentinel_date: vec![PostSentinelDate { post_id: id(65) }],
            canteen_duplicate: vec![CanteenDuplicate {
                date: day("2022-04-01"),
                expect_ids: vec![id(393), id(398)],
                keep: id(398),
            }],
            label_duplicate: vec![LabelDuplicate {
                expect_ids: vec![id(4), id(9), id(31)],
                keep: id(4),
            }],
            menu_unpositioned: vec![
                MenuUnpositioned {
                    parent_id: Some(id(15)),
                    expect_null_lft: vec![id(26), id(29), id(30), id(31)],
                    served_order: vec![id(26), id(30), id(29), id(31)],
                },
                MenuUnpositioned {
                    parent_id: None,
                    expect_null_lft: vec![id(40)],
                    served_order: vec![id(40)],
                },
            ],
            page_hidden: vec![PageHidden {
                page_id: id(12),
                expect_updated_at: instant("2024-03-01T10:00:00Z"),
            }],
            n7_accept: vec![
                N7Accept {
                    body: BodyRow::Post(id(91)),
                    expect_content_sha256: digest(0x01),
                },
                N7Accept {
                    body: BodyRow::Page(id(91)),
                    expect_content_sha256: digest(0xab),
                },
            ],
        }
    }

    // ----- Valid files -----

    #[test]
    fn the_empty_list_is_the_version_alone() {
        let file = parse("version = 1\n").expect("valid");
        assert_eq!(file, ResolutionFile::default());
        assert_eq!(file.entry_count(), 0);
        assert_eq!(file.signed_off, None);
    }

    #[test]
    fn a_file_with_every_section_parses_to_its_entries() {
        let file = parse(FULL).expect("valid");
        assert_eq!(file, full());
        assert_eq!(file.entry_count(), 12);
    }

    #[test]
    fn a_section_may_be_an_inline_array_and_an_instant_may_say_plus_zero() {
        let file = parse(&v1(
            "post_author = [{ post_id = 55, expect_author_id = \"none\", set_author = \"generic\" \
             }]\npage_hidden = [{ page_id = 3, expect_updated_at = 2024-03-01T10:00:00+00:00 }]\n",
        ))
        .expect("valid");
        assert_eq!(
            file.post_author,
            vec![PostAuthor {
                post_id: id(55),
                set_author: AuthorChoice::Generic
            }]
        );
        assert_eq!(
            file.page_hidden[0].expect_updated_at,
            instant("2024-03-01T10:00:00Z")
        );
    }

    #[test]
    fn an_instant_without_seconds_or_with_a_zero_fraction_is_read_as_whole_seconds() {
        for written in ["2024-03-01T10:00Z", "2024-03-01T10:00:00.000Z"] {
            let file = parse(&v1(&format!(
                "[[page_hidden]]\npage_id = 3\nexpect_updated_at = {written}\n"
            )))
            .unwrap_or_else(|errors| panic!("{written}: {errors}"));
            assert_eq!(
                file.page_hidden[0].expect_updated_at,
                instant("2024-03-01T10:00:00Z"),
                "{written}"
            );
        }
    }

    #[test]
    fn the_canonical_layout_of_the_full_file() {
        let expected = "version = 1
signed_off = 2026-11-20

[[post_date_shift]]
post_id = 12
expect_date = 2019-05-02

[[post_author]]
post_id = 55
expect_author_id = \"none\"
set_author = \"generic\"

[[post_author]]
post_id = 61
expect_author_id = \"none\"
set_author = 7

[[post_index_image]]
post_id = 70
expect_index_image = \"index/0123456789abcdef0123456789abcdef.JPG\"

[[post_sentinel_date]]
post_id = 65
expect_date = 1970-01-01

[[canteen_duplicate]]
date = 2022-04-01
expect_ids = [393, 398]
keep = 398

[[label_duplicate]]
expect_ids = [4, 9, 31]
keep = 4

[[menu_unpositioned]]
parent_id = 15
expect_null_lft = [26, 29, 30, 31]
served_order = [26, 30, 29, 31]

[[menu_unpositioned]]
parent_id = \"none\"
expect_null_lft = [40]
served_order = [40]

[[page_hidden]]
page_id = 12
expect_updated_at = 2024-03-01T10:00:00Z

[[N7-accept]]
post_id = 91
expect_content_sha256 = \"0101010101010101010101010101010101010101010101010101010101010101\"

[[N7-accept]]
page_id = 91
expect_content_sha256 = \"abababababababababababababababababababababababababababababababab\"
";
        assert_eq!(full().to_toml(), expected);
        assert_eq!(ResolutionFile::default().to_toml(), "version = 1\n");
    }

    #[test]
    fn the_sections_name_their_defects_keys_and_fields() {
        let keys: Vec<&str> = Section::ALL.iter().map(|section| section.key()).collect();
        assert_eq!(keys, TOP_LEVEL_KEYS[2..]);
        let defects: Vec<&str> = Section::ALL
            .iter()
            .map(|section| section.defect())
            .collect();
        assert_eq!(
            defects,
            ["D1", "D2", "D3", "D5", "D6", "D8", "D9", "D10", "N7"]
        );
        for section in Section::ALL {
            let fields = section.fields();
            assert!(!fields.is_empty(), "{section:?}");
            for field in fields {
                assert!(
                    section.fields_text().contains(field),
                    "{section:?}: {field}"
                );
            }
        }
        assert_eq!(Section::PostAuthor.entry(2), "post_author[2]");
    }

    #[test]
    fn the_constants_hold_what_their_documentation_says() {
        assert_eq!(SENTINEL_DATE, day("1970-01-01"));
        assert_eq!(MAX_FILE_BYTES, 1_048_576);
        assert_eq!(SCHEMA_VERSION, 1);
    }

    // ----- The file as a whole -----

    #[test]
    fn text_that_is_not_toml_names_the_line_and_column() {
        let message = problem("version = 1\npost_author = [\n  { post_id = 55\n");
        assert!(message.starts_with("not valid TOML at line "), "{message}");
        // The parser's own text follows the place; its wording belongs to
        // the toml crate, so only the place is pinned here.
        assert_eq!(
            problem("version = 1\nfoo = = 2\n").split(':').next(),
            Some("not valid TOML at line 2, column 7")
        );
        assert_eq!(
            problem("version = 1\nversion = 1\n").split(':').next(),
            Some("not valid TOML at line 2, column 1")
        );
    }

    #[test]
    fn the_version_is_checked_first_and_alone() {
        assert_eq!(
            problem("post_author = 3\n"),
            "version: missing: the file starts with version = 1"
        );
        assert_eq!(
            problem("version = 2\npost_author = 3\n"),
            "version: 2 is not a schema version this build reads: it reads version 1"
        );
        assert_eq!(
            problem("version = \"1\"\n"),
            "version: expected the integer 1, found the string \"1\""
        );
    }

    #[test]
    fn an_unknown_top_level_key_is_named() {
        assert_eq!(
            problem(&v1("[[post_without_body]]\npost_id = 3\n")),
            "post_without_body: unknown key: the file has version, signed_off and the sections \
             post_date_shift, post_author, post_index_image, post_sentinel_date, \
             canteen_duplicate, label_duplicate, menu_unpositioned, page_hidden and N7-accept, \
             and nothing else"
        );
    }

    #[test]
    fn the_sign_off_date_must_be_a_date() {
        assert_eq!(
            problem(&v1("signed_off = \"2026-11-20\"\n")),
            "signed_off: expected a date such as 2022-04-01, without a time and without quotes, \
             found the string \"2026-11-20\""
        );
        assert_eq!(
            problem(&v1("signed_off = 2026-11-20T10:00:00Z\n")),
            "signed_off: expected a date such as 2022-04-01, without a time and without quotes, \
             found the date-time 2026-11-20T10:00:00Z"
        );
    }

    #[test]
    fn a_section_must_be_an_array_of_tables() {
        assert_eq!(
            problem(&v1("post_author = 3\n")),
            "post_author (D2): expected an array of tables, written [[post_author]], found the \
             integer 3"
        );
        assert_eq!(
            problem(&v1("[post_author]\npost_id = 3\n")),
            "post_author (D2): expected an array of tables, written [[post_author]], found a table"
        );
        assert_eq!(
            problems(&v1(
                "label_duplicate = [1, { expect_ids = [1, 2], keep = 1 }, \"x\"]\n"
            ))[0],
            "label_duplicate[1] (D8): expected a table, found the integer 1"
        );
        assert_eq!(
            problems(&v1(
                "label_duplicate = [1, { expect_ids = [1, 2], keep = 1 }, \"x\"]\n"
            ))[1],
            "label_duplicate[3] (D8): expected a table, found the string \"x\""
        );
    }

    #[test]
    fn a_missing_and_an_unknown_field_are_named() {
        assert_eq!(
            problems(&v1(
                "[[post_author]]\npost_id = 55\nauthor = 7\nset_autor = 7\n"
            )),
            [
                "post_author[1] (D2), field expect_author_id: missing: every post_author entry \
                 has post_id, expect_author_id and set_author",
                "post_author[1] (D2), field set_author: missing: every post_author entry has \
                 post_id, expect_author_id and set_author",
                "post_author[1] (D2), field author: unknown field: every post_author entry has \
                 post_id, expect_author_id and set_author, and nothing else",
                "post_author[1] (D2), field set_autor: unknown field: every post_author entry has \
                 post_id, expect_author_id and set_author, and nothing else",
            ]
        );
    }

    #[test]
    fn a_row_id_is_a_whole_number_in_the_source_range() {
        let entry = |value: &str| {
            problem(&v1(&format!(
                "[[post_date_shift]]\npost_id = {value}\nexpect_date = 2019-05-02\n"
            )))
        };
        assert_eq!(
            entry("\"12\""),
            "post_date_shift[1] (D1), field post_id: expected a row id, a whole number from 1 to \
             4294967295, found the string \"12\""
        );
        assert_eq!(
            entry("12.0"),
            "post_date_shift[1] (D1), field post_id: expected a row id, a whole number from 1 to \
             4294967295, found the float 12"
        );
        for number in ["0", "-3", "4294967296"] {
            assert_eq!(
                entry(number),
                format!(
                    "post_date_shift[1] (D1), field post_id: {number} is not a row id: write a \
                     whole number from 1 to 4294967295"
                )
            );
        }
        let largest = parse(&v1(
            "[[post_date_shift]]\npost_id = 4294967295\nexpect_date = 2019-05-02\n",
        ))
        .expect("the largest id");
        assert_eq!(largest.post_date_shift[0].post_id.get(), u32::MAX);
    }

    #[test]
    fn a_date_is_a_toml_local_date() {
        let entry = |value: &str| {
            problem(&v1(&format!(
                "[[post_date_shift]]\npost_id = 12\nexpect_date = {value}\n"
            )))
        };
        assert_eq!(
            entry("\"2019-05-02\""),
            "post_date_shift[1] (D1), field expect_date: expected a date such as 2022-04-01, \
             without a time and without quotes, found the string \"2019-05-02\""
        );
        assert_eq!(
            entry("2019-05-02T00:00:00"),
            "post_date_shift[1] (D1), field expect_date: expected a date such as 2022-04-01, \
             without a time and without quotes, found the date-time 2019-05-02T00:00:00"
        );
        assert_eq!(
            entry("10:00:00"),
            "post_date_shift[1] (D1), field expect_date: expected a date such as 2022-04-01, \
             without a time and without quotes, found the time 10:00:00"
        );
    }

    #[test]
    fn a_date_outside_the_calendar_is_refused() {
        // The TOML parser already refuses such a date in a file, so the
        // value is built by hand.
        let value = Value::Datetime(Datetime {
            date: Some(toml::value::Date {
                year: 2021,
                month: 2,
                day: 30,
            }),
            time: None,
            offset: None,
        });
        assert_eq!(
            date_value(&value),
            Err("2021-02-30 is not a calendar date".to_owned())
        );
    }

    #[test]
    fn the_values_of_every_type_are_described() {
        let cases = [
            (Value::Boolean(true), "the boolean true"),
            (Value::Array(Vec::new()), "an array"),
            (Value::Table(Table::new()), "a table"),
            (Value::Float(1.5), "the float 1.5"),
            (
                Value::String("a".repeat(41)),
                "the string \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" (cut)",
            ),
            (Value::String("ő\"".to_owned()), "the string \"ő\\\"\""),
        ];
        for (value, described) in cases {
            assert_eq!(describe(&value), described);
        }
    }

    // ----- D1 -----

    #[test]
    fn d1_shifts_real_dates_only_and_one_entry_per_post() {
        assert_eq!(
            problem(&v1(
                "[[post_date_shift]]\npost_id = 12\nexpect_date = 1970-01-01\n"
            )),
            "post_date_shift[1] (D1), field expect_date: 1970-01-01 is the sentinel date, which \
             post_sentinel_date (D5) sets to NULL: D1 shifts real dates only"
        );
        assert_eq!(
            problem(&v1(
                "[[post_date_shift]]\npost_id = 12\nexpect_date = 2019-05-02\n\
                 [[post_date_shift]]\npost_id = 13\nexpect_date = 2019-05-02\n\
                 [[post_date_shift]]\npost_id = 12\nexpect_date = 2019-05-03\n"
            )),
            "post_date_shift[3] (D1), field post_id: post 12 is already in post_date_shift[1]: \
             one entry each"
        );
    }

    // ----- D2 -----

    #[test]
    fn d2_states_the_missing_author_and_names_a_choice() {
        let entry = |precondition: &str, choice: &str| {
            problem(&v1(&format!(
                "[[post_author]]\npost_id = 55\nexpect_author_id = {precondition}\nset_author = \
                 {choice}\n"
            )))
        };
        assert_eq!(
            entry("7", "\"generic\""),
            "post_author[1] (D2), field expect_author_id: must be \"none\": D2 covers published \
             posts without an author, found the integer 7"
        );
        assert_eq!(
            entry("\"none\"", "\"Generic\""),
            "post_author[1] (D2), field set_author: expected \"generic\" or an author id, a whole \
             number from 1 to 4294967295, found the string \"Generic\""
        );
        assert_eq!(
            entry("\"none\"", "0"),
            "post_author[1] (D2), field set_author: 0 is not an author id: write \"generic\" or a \
             whole number from 1 to 4294967295"
        );
        assert_eq!(
            problem(&v1(
                "[[post_author]]\npost_id = 55\nexpect_author_id = \"none\"\nset_author = 3\n\
                 [[post_author]]\npost_id = 55\nexpect_author_id = \"none\"\nset_author = \
                 \"generic\"\n"
            )),
            "post_author[2] (D2), field post_id: post 55 is already in post_author[1]: one entry \
             each"
        );
    }

    // ----- D3 -----

    #[test]
    fn d3_pins_an_index_image_path() {
        let expected = |found: &str| {
            format!(
                "post_index_image[1] (D3), field expect_index_image: expected the path \
                 index/<32 lower-case hex digits>.<extension>, at most 191 characters, found \
                 {found}"
            )
        };
        let entry = |value: &str| {
            problem(&v1(&format!(
                "[[post_index_image]]\npost_id = 70\nexpect_index_image = {value}\n"
            )))
        };
        let hex32 = "0123456789abcdef0123456789abcdef";
        let long = format!("\"index/{hex32}.{}\"", "a".repeat(153));
        for value in [
            format!("\"index/{}.jpg\"", hex32.to_uppercase()),
            format!("\"index/{hex32}\""),
            format!("\"index/{hex32}.\""),
            format!("\"index/{hex32}.j-g\""),
            format!("\"uploads/{hex32}.jpg\""),
            format!("\"index/{}.jpg\"", &hex32[1..]),
            long.clone(),
        ] {
            let found = describe(&Value::String(value.trim_matches('"').to_owned()));
            assert_eq!(entry(&value), expected(&found), "{value}");
        }
        assert_eq!(entry("70"), expected("the integer 70"));
        // 191 characters is the column's length, and allowed.
        let longest = format!("\"index/{hex32}.{}\"", "a".repeat(152));
        assert!(
            parse(&v1(&format!(
                "[[post_index_image]]\npost_id = 70\nexpect_index_image = {longest}\n"
            )))
            .is_ok()
        );
        assert_eq!(
            problem(&v1(&format!(
                "[[post_index_image]]\npost_id = 70\nexpect_index_image = \"index/{hex32}.png\"\n\
                 [[post_index_image]]\npost_id = 70\nexpect_index_image = \"index/{hex32}.jpg\"\n"
            ))),
            "post_index_image[2] (D3), field post_id: post 70 is already in post_index_image[1]: \
             one entry each"
        );
    }

    // ----- D5 -----

    #[test]
    fn d5_states_the_sentinel_and_never_shares_a_post_with_d1() {
        assert_eq!(
            problem(&v1(
                "[[post_sentinel_date]]\npost_id = 65\nexpect_date = 2019-05-02\n"
            )),
            "post_sentinel_date[1] (D5), field expect_date: must be the sentinel 1970-01-01, \
             which D5 sets to NULL, found 2019-05-02: a real date that is one day early belongs \
             in post_date_shift (D1)"
        );
        assert_eq!(
            problem(&v1(
                "[[post_sentinel_date]]\npost_id = 65\nexpect_date = \"1970-01-01\"\n"
            )),
            "post_sentinel_date[1] (D5), field expect_date: expected a date such as 2022-04-01, \
             without a time and without quotes, found the string \"1970-01-01\""
        );
        assert_eq!(
            problem(&v1(
                "[[post_date_shift]]\npost_id = 11\nexpect_date = 2019-05-02\n\
                 [[post_date_shift]]\npost_id = 65\nexpect_date = 2019-05-02\n\
                 [[post_sentinel_date]]\npost_id = 65\nexpect_date = 1970-01-01\n"
            )),
            "post_sentinel_date[1] (D5), field post_id: post 65 is also in post_date_shift[2] \
             (D1): a post has either a real date that D1 shifts or the sentinel that D5 clears"
        );
        assert_eq!(
            problem(&v1(
                "[[post_sentinel_date]]\npost_id = 65\nexpect_date = 1970-01-01\n\
                 [[post_sentinel_date]]\npost_id = 65\nexpect_date = 1970-01-01\n"
            )),
            "post_sentinel_date[2] (D5), field post_id: post 65 is already in \
             post_sentinel_date[1]: one entry each"
        );
    }

    #[test]
    fn d5_names_the_d1_entry_of_the_file_even_after_a_refused_one() {
        // The first D1 entry is refused, so the shared post is the second
        // D1 table of the file, not the first that parsed.
        assert_eq!(
            problems(&v1(
                "[[post_date_shift]]\npost_id = 0\nexpect_date = 2019-05-02\n\
                 [[post_date_shift]]\npost_id = 65\nexpect_date = 2019-05-02\n\
                 [[post_sentinel_date]]\npost_id = 65\nexpect_date = 1970-01-01\n"
            ))[1],
            "post_sentinel_date[1] (D5), field post_id: post 65 is also in post_date_shift[2] \
             (D1): a post has either a real date that D1 shifts or the sentinel that D5 clears"
        );
    }

    // ----- D6 -----

    #[test]
    fn d6_keeps_one_of_at_least_two_rows_of_a_date() {
        let entry = |ids: &str, keep: &str| {
            problem(&v1(&format!(
                "[[canteen_duplicate]]\ndate = 2022-04-01\nexpect_ids = {ids}\nkeep = {keep}\n"
            )))
        };
        assert_eq!(
            entry("[393]", "393"),
            "canteen_duplicate[1] (D6), field expect_ids: lists 1 row: a duplicate date has at \
             least 2 rows"
        );
        assert_eq!(
            entry("[]", "393"),
            "canteen_duplicate[1] (D6), field expect_ids: lists 0 rows: a duplicate date has at \
             least 2 rows"
        );
        assert_eq!(
            entry("[393, 393]", "393"),
            "canteen_duplicate[1] (D6), field expect_ids: lists row 393 twice"
        );
        assert_eq!(
            entry("[393, \"398\"]", "393"),
            "canteen_duplicate[1] (D6), field expect_ids: element 2: expected a row id, a whole \
             number from 1 to 4294967295, found the string \"398\""
        );
        assert_eq!(
            entry("393", "393"),
            "canteen_duplicate[1] (D6), field expect_ids: expected an array of ids, each a whole \
             number from 1 to 4294967295, found the integer 393"
        );
        assert_eq!(
            entry("[393, 398]", "400"),
            "canteen_duplicate[1] (D6), field keep: row 400 is not in expect_ids [393, 398]: \
             keep one of the listed rows"
        );
        assert_eq!(
            entry("[393, 398]", "0"),
            "canteen_duplicate[1] (D6), field keep: 0 is not a row id: write a whole number from \
             1 to 4294967295"
        );
    }

    #[test]
    fn d6_covers_a_date_and_a_row_once() {
        assert_eq!(
            problems(&v1(
                "[[canteen_duplicate]]\ndate = 2022-04-01\nexpect_ids = [393, 398]\nkeep = 398\n\
                 [[canteen_duplicate]]\ndate = 2022-04-01\nexpect_ids = [400, 401]\nkeep = 400\n\
                 [[canteen_duplicate]]\ndate = 2022-04-02\nexpect_ids = [398, 402]\nkeep = 402\n"
            )),
            [
                "canteen_duplicate[2] (D6), field date: 2022-04-01 is already in \
                 canteen_duplicate[1]: one entry each",
                "canteen_duplicate[3] (D6), field expect_ids: row 398 is already in \
                 canteen_duplicate[1]: one entry each",
            ]
        );
    }

    // ----- D8 -----

    #[test]
    fn d8_keeps_one_label_of_a_group_and_each_label_is_in_one_group() {
        assert_eq!(
            problem(&v1("[[label_duplicate]]\nexpect_ids = [4]\nkeep = 4\n")),
            "label_duplicate[1] (D8), field expect_ids: lists 1 label: a group of duplicate names \
             has at least 2 labels"
        );
        assert_eq!(
            problem(&v1("[[label_duplicate]]\nexpect_ids = [4, 9]\nkeep = 31\n")),
            "label_duplicate[1] (D8), field keep: label 31 is not in expect_ids [4, 9]: keep one \
             of the listed labels"
        );
        assert_eq!(
            problem(&v1("[[label_duplicate]]\nexpect_ids = [4, 9]\nkeep = 4\n\
                 [[label_duplicate]]\nexpect_ids = [31, 9]\nkeep = 31\n")),
            "label_duplicate[2] (D8), field expect_ids: label 9 is already in \
             label_duplicate[1]: one entry each"
        );
    }

    // ----- D9 -----

    #[test]
    fn d9_orders_exactly_the_unpositioned_items_of_a_parent() {
        let entry = |parent: &str, listed: &str, order: &str| {
            problem(&v1(&format!(
                "[[menu_unpositioned]]\nparent_id = {parent}\nexpect_null_lft = \
                 {listed}\nserved_order = {order}\n"
            )))
        };
        assert_eq!(
            entry("\"root\"", "[25]", "[25]"),
            "menu_unpositioned[1] (D9), field parent_id: expected a menu item id, a whole number \
             from 1 to 4294967295, or \"none\" for the root level, found the string \"root\""
        );
        assert_eq!(
            entry("0", "[25]", "[25]"),
            "menu_unpositioned[1] (D9), field parent_id: 0 is not a menu item id: write a whole \
             number from 1 to 4294967295, or \"none\" for the root level"
        );
        assert_eq!(
            entry("15", "[]", "[26]"),
            "menu_unpositioned[1] (D9), field expect_null_lft: lists 0 items: an entry lists at \
             least 1 item"
        );
        for order in ["[26, 30, 29]", "[26, 30, 29, 31, 32]", "[26, 30, 29, 25]"] {
            assert_eq!(
                entry("15", "[26, 29, 30, 31]", order),
                format!(
                    "menu_unpositioned[1] (D9), field served_order: lists {order}, which is not \
                     an order of expect_null_lft [26, 29, 30, 31]: list each of those ids once, \
                     in the order the live site serves them"
                )
            );
        }
        assert_eq!(
            entry("15", "[26, 29]", "[26, 26]"),
            "menu_unpositioned[1] (D9), field served_order: lists item 26 twice"
        );
        assert_eq!(
            entry("26", "[26, 29]", "[29, 26]"),
            "menu_unpositioned[1] (D9), field parent_id: 26 is also in expect_null_lft: an item \
             is not its own parent"
        );
    }

    #[test]
    fn d9_covers_a_parent_and_an_item_once() {
        assert_eq!(
            problems(&v1(
                "[[menu_unpositioned]]\nparent_id = \"none\"\nexpect_null_lft = [1]\nserved_order \
                 = [1]\n\
                 [[menu_unpositioned]]\nparent_id = 7\nexpect_null_lft = [25]\nserved_order = \
                 [25]\n\
                 [[menu_unpositioned]]\nparent_id = 7\nexpect_null_lft = [26]\nserved_order = \
                 [26]\n\
                 [[menu_unpositioned]]\nparent_id = \"none\"\nexpect_null_lft = [25]\nserved_order \
                 = [25]\n"
            )),
            [
                "menu_unpositioned[3] (D9), field parent_id: parent 7 is already in \
                 menu_unpositioned[2]: one entry each",
                "menu_unpositioned[4] (D9), field parent_id: the root level is already in \
                 menu_unpositioned[1]: one entry each",
                "menu_unpositioned[4] (D9), field expect_null_lft: item 25 is already in \
                 menu_unpositioned[2]: one entry each",
            ]
        );
    }

    // ----- D10 -----

    #[test]
    fn d10_pins_the_page_s_last_change_as_a_utc_instant() {
        let entry = |value: &str| {
            problem(&v1(&format!(
                "[[page_hidden]]\npage_id = 12\nexpect_updated_at = {value}\n"
            )))
        };
        let expected = |found: &str| {
            format!(
                "page_hidden[1] (D10), field expect_updated_at: expected an instant in UTC such \
                 as 2024-03-01T10:00:00Z, without quotes, found {found}"
            )
        };
        assert_eq!(
            entry("\"2024-03-01T10:00:00Z\""),
            expected("the string \"2024-03-01T10:00:00Z\"")
        );
        assert_eq!(entry("2024-03-01"), expected("the date 2024-03-01"));
        assert_eq!(entry("10:00:00"), expected("the time 10:00:00"));
        assert_eq!(
            entry("2024-03-01T10:00:00"),
            "page_hidden[1] (D10), field expect_updated_at: 2024-03-01T10:00:00 has no offset: \
             write the instant in UTC, as in 2024-03-01T10:00:00Z"
        );
        assert_eq!(
            entry("2024-03-01T12:00:00+02:00"),
            "page_hidden[1] (D10), field expect_updated_at: 2024-03-01T12:00:00+02:00 is not in \
             UTC: write the same instant in UTC, 2024-03-01T10:00:00Z"
        );
        assert_eq!(
            entry("2024-03-01T10:00:00.5Z"),
            "page_hidden[1] (D10), field expect_updated_at: 2024-03-01T10:00:00.5Z has a fraction \
             of a second: the legacy updated_at holds whole seconds"
        );
        assert_eq!(
            entry("2016-12-31T23:59:60Z"),
            "page_hidden[1] (D10), field expect_updated_at: 2016-12-31T23:59:60Z is not an \
             instant the source can hold"
        );
        assert_eq!(
            problem(&v1(
                "[[page_hidden]]\npage_id = 12\nexpect_updated_at = 2024-03-01T10:00:00Z\n\
                 [[page_hidden]]\npage_id = 12\nexpect_updated_at = 2024-03-02T10:00:00Z\n"
            )),
            "page_hidden[2] (D10), field page_id: page 12 is already in page_hidden[1]: one \
             entry each"
        );
    }

    // ----- N7-accept -----

    #[test]
    fn n7_accept_names_one_body_row_and_its_digest() {
        let digest = "\"0101010101010101010101010101010101010101010101010101010101010101\"";
        assert_eq!(
            problem(&v1(&format!(
                "[[N7-accept]]\nexpect_content_sha256 = {digest}\n"
            ))),
            "N7-accept[1] (N7): has neither post_id nor page_id: give exactly one of post_id and \
             page_id, the row whose body it is"
        );
        assert_eq!(
            problem(&v1(&format!(
                "[[N7-accept]]\npost_id = 1\npage_id = 2\nexpect_content_sha256 = {digest}\n"
            ))),
            "N7-accept[1] (N7): has both post_id and page_id: give exactly one of post_id and \
             page_id, the row whose body it is"
        );
        assert_eq!(
            problem(&v1("[[N7-accept]]\npage_id = 50\n")),
            "N7-accept[1] (N7), field expect_content_sha256: missing: every N7-accept entry has \
             post_id or page_id, and expect_content_sha256"
        );
        // A string longer than 40 characters is shown cut after 40.
        for (value, found) in [
            ("\"0101\"".to_owned(), "the string \"0101\"".to_owned()),
            (
                digest.to_uppercase().replace('0', "A"),
                format!("the string \"{}\" (cut)", "A1".repeat(20)),
            ),
            (
                format!("\"sha256:{}\"", "01".repeat(32)),
                format!("the string \"sha256:{}0\" (cut)", "01".repeat(16)),
            ),
            ("7".to_owned(), "the integer 7".to_owned()),
        ] {
            assert_eq!(
                problem(&v1(&format!(
                    "[[N7-accept]]\npost_id = 91\nexpect_content_sha256 = {value}\n"
                ))),
                format!(
                    "N7-accept[1] (N7), field expect_content_sha256: expected 64 lower-case hex \
                     digits, the SHA-256 of the source body, found {found}"
                ),
                "{value}"
            );
        }
        assert_eq!(
            problem(&v1(&format!(
                "[[N7-accept]]\npost_id = 91\nexpect_content_sha256 = {digest}\n\
                 [[N7-accept]]\npost_id = 91\nexpect_content_sha256 = {digest}\n"
            ))),
            "N7-accept[2] (N7), field post_id: post 91 is already in N7-accept[1]: one entry each"
        );
    }

    #[test]
    fn digests_read_and_write_as_lower_case_hex() {
        let written = "00ff10a0".repeat(8);
        let read = sha256_of(&written).expect("a digest");
        assert_eq!(hex(&read), written);
        assert_eq!(sha256_of(&"g".repeat(64)), None);
        assert_eq!(sha256_of(&"0".repeat(63)), None);
    }

    // ----- The file's digest -----

    #[test]
    fn the_file_digest_is_the_sha256_that_sha256sum_prints() {
        // The values sha256sum prints for these bytes; "abc" is the test
        // vector of FIPS 180-2.
        for (bytes, printed) in [
            (
                &b""[..],
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"version = 1\n",
                "dbab12665d98aef021ba64953c61b0ed8a908cfb56a1c01e2fcb4b052b71a2a1",
            ),
        ] {
            assert_eq!(FileDigest::of(bytes).to_string(), printed);
        }
        // The bytes count, not the list they parse to.
        assert_ne!(
            FileDigest::of(b"version = 1\n"),
            FileDigest::of(b"version = 1\n# a comment\n")
        );
    }

    // ----- Names the file writes -----

    #[test]
    fn an_unknown_name_that_is_not_a_bare_key_is_shown_quoted_and_escaped() {
        let unknown_key = "unknown key: the file has version, signed_off and the sections \
                           post_date_shift, post_author, post_index_image, post_sentinel_date, \
                           canteen_duplicate, label_duplicate, menu_unpositioned, page_hidden and \
                           N7-accept, and nothing else";
        // A quoted key with a line break would otherwise start a line that
        // reads as a second problem.
        let errors = parse(&v1("\"x\\n- version: fake problem\" = 1\n")).expect_err("refused");
        assert_eq!(
            errors.to_string(),
            format!("1 problem:\n- \"x\\n- version: fake problem\": {unknown_key}")
        );
        assert_eq!(
            problem(&v1(
                "[[post_author]]\npost_id = 55\nexpect_author_id = \"none\"\nset_author = \
                 \"generic\"\n\"y\\n- fake\" = 1\n"
            )),
            "post_author[1] (D2), field \"y\\n- fake\": unknown field: every post_author entry \
             has post_id, expect_author_id and set_author, and nothing else"
        );
        // Every other character outside a bare key is quoted too, and the
        // line breaks beyond \n and \r are escaped.
        for (written, shown) in [
            ("\"\"", "\"\""),
            ("\"a b\"", "\"a b\""),
            ("\"é\"", "\"é\""),
            ("\"tab\\there\"", "\"tab\\there\""),
            ("\"q\\\"uote\"", "\"q\\\"uote\""),
            ("\"cr\\r\"", "\"cr\\r\""),
            ("\"nel\\u0085\"", "\"nel\\u{85}\""),
            ("\"ls\\u2028\"", "\"ls\\u{2028}\""),
            ("\"ps\\u2029\"", "\"ps\\u{2029}\""),
            // A dotted key is a table: the top-level key is its first part.
            ("x.y", "x"),
        ] {
            let text = v1(&format!("{written} = 1\n"));
            let shown_problem = problem(&text);
            assert_eq!(
                shown_problem,
                format!("{shown}: {unknown_key}"),
                "{written}"
            );
        }
        // A bare key is shown as written.
        assert_eq!(shown_name("post_id"), "post_id");
        assert_eq!(shown_name("N7-accept"), "N7-accept");
        assert_eq!(shown_name("Ab9_-"), "Ab9_-");
    }

    // ----- Every problem at once -----

    #[test]
    fn every_problem_of_a_file_is_listed_in_order() {
        let errors = parse(&v1("colour = 1\nsigned_off = 3\n\
             [[post_date_shift]]\npost_id = 0\nexpect_date = 1970-01-01\n\
             [[label_duplicate]]\nexpect_ids = [4]\nkeep = 9\n"))
        .expect_err("refused");
        let lines: Vec<String> = errors.problems().iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "colour: unknown key: the file has version, signed_off and the sections \
                 post_date_shift, post_author, post_index_image, post_sentinel_date, \
                 canteen_duplicate, label_duplicate, menu_unpositioned, page_hidden and \
                 N7-accept, and nothing else",
                "signed_off: expected a date such as 2022-04-01, without a time and without \
                 quotes, found the integer 3",
                "post_date_shift[1] (D1), field post_id: 0 is not a row id: write a whole number \
                 from 1 to 4294967295",
                "post_date_shift[1] (D1), field expect_date: 1970-01-01 is the sentinel date, \
                 which post_sentinel_date (D5) sets to NULL: D1 shifts real dates only",
                // keep = 9 is a row id; whether it is among expect_ids
                // cannot be checked while expect_ids is refused.
                "label_duplicate[1] (D8), field expect_ids: lists 1 label: a group of duplicate \
                 names has at least 2 labels",
            ]
        );
        assert_eq!(
            errors.to_string(),
            format!("5 problems:\n- {}", lines.join("\n- "))
        );
    }

    #[test]
    fn the_text_of_many_problems_shows_fifty_and_counts_the_rest() {
        let body = (1..=60)
            .map(|n| format!("[[page_hidden]]\npage_id = {n}\n"))
            .collect::<Vec<_>>()
            .concat();
        let errors = parse(&v1(&body)).expect_err("refused");
        assert_eq!(errors.problems().len(), 60);
        let text = errors.to_string();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "60 problems:");
        assert_eq!(lines.len(), 1 + 50 + 1);
        assert_eq!(lines[51], "- and 10 more");
        assert_eq!(
            parse(&v1("[[page_hidden]]\npage_id = 1\n"))
                .expect_err("refused")
                .to_string(),
            "1 problem:\n- page_hidden[1] (D10), field expect_updated_at: missing: every \
             page_hidden entry has page_id and expect_updated_at"
        );
    }

    // ----- Properties -----

    /// Row ids over their whole range.
    fn any_id() -> impl Strategy<Value = NonZeroU32> {
        (1_u32..=u32::MAX).prop_map(id)
    }

    /// Ids from `range`, distinct, up to `most` of them.
    fn distinct_ids(
        range: std::ops::RangeInclusive<u32>,
        most: usize,
    ) -> impl Strategy<Value = Vec<NonZeroU32>> {
        collection::btree_set(range, 0..=most).prop_map(|ids| ids.into_iter().map(id).collect())
    }

    /// Calendar days that TOML can write (years 1 to 9999).
    fn any_day() -> impl Strategy<Value = NaiveDate> {
        (1_i32..=9999, 1_u32..=365)
            .prop_map(|(year, ordinal)| NaiveDate::from_yo_opt(year, ordinal).expect("a day"))
    }

    /// Instants with whole seconds, years 1 to 9999.
    fn any_instant() -> impl Strategy<Value = DateTime<Utc>> {
        (-62_135_596_800_i64..=253_402_300_799)
            .prop_map(|seconds| DateTime::from_timestamp(seconds, 0).expect("in range"))
    }

    /// `ids` cut into groups of the given sizes, each with the index of the
    /// one it keeps.
    fn grouped(
        ids: &[NonZeroU32],
        sizes: &[usize],
        keeps: &[Index],
    ) -> Vec<(Vec<NonZeroU32>, NonZeroU32)> {
        let mut start = 0;
        sizes
            .iter()
            .zip(keeps)
            .map(|(&size, keep)| {
                let group = ids[start..start + size].to_vec();
                start += size;
                let kept = group[keep.index(size)];
                (group, kept)
            })
            .collect()
    }

    /// Groups of distinct ids (no id in two groups) of `minimum` to 4 ids
    /// each, with a kept one; up to 3 groups.
    fn any_groups(
        range: std::ops::RangeInclusive<u32>,
        minimum: usize,
    ) -> impl Strategy<Value = Vec<(Vec<NonZeroU32>, NonZeroU32)>> {
        collection::vec(minimum..=4_usize, 0..=3).prop_flat_map(move |sizes| {
            let total: usize = sizes.iter().sum();
            let count = sizes.len();
            (
                Just(sizes),
                collection::btree_set(range.clone(), total..=total),
                collection::vec(any::<Index>(), count..=count),
            )
                .prop_map(|(sizes, ids, keeps)| {
                    let ids: Vec<NonZeroU32> = ids.into_iter().map(id).collect();
                    grouped(&ids, &sizes, &keeps)
                })
        })
    }

    /// `items` in an order chosen by `picks`.
    fn permuted(items: &[NonZeroU32], picks: &[Index]) -> Vec<NonZeroU32> {
        let mut left = items.to_vec();
        let mut order = Vec::with_capacity(items.len());
        for pick in picks.iter().take(items.len()) {
            order.push(left.remove(pick.index(left.len())));
        }
        order.extend(left);
        order
    }

    fn any_post_date_shift() -> impl Strategy<Value = Vec<PostDateShift>> {
        // Posts 1 to 1,000,000; D5 takes the ids above, so the two never
        // share a post.
        distinct_ids(1..=1_000_000, 5).prop_flat_map(|ids| {
            let count = ids.len();
            (Just(ids), collection::vec(any_day(), count..=count)).prop_map(|(ids, days)| {
                ids.into_iter()
                    .zip(days)
                    .map(|(post_id, day)| PostDateShift {
                        post_id,
                        expect_date: if day == SENTINEL_DATE {
                            day.succ_opt().expect("1970-01-02")
                        } else {
                            day
                        },
                    })
                    .collect()
            })
        })
    }

    fn any_post_author() -> impl Strategy<Value = Vec<PostAuthor>> {
        distinct_ids(1..=u32::MAX, 4).prop_flat_map(|ids| {
            let count = ids.len();
            let choice = prop_oneof![
                Just(AuthorChoice::Generic),
                any_id().prop_map(AuthorChoice::Author)
            ];
            (Just(ids), collection::vec(choice, count..=count)).prop_map(|(ids, choices)| {
                ids.into_iter()
                    .zip(choices)
                    .map(|(post_id, set_author)| PostAuthor {
                        post_id,
                        set_author,
                    })
                    .collect()
            })
        })
    }

    fn any_post_index_image() -> impl Strategy<Value = Vec<PostIndexImage>> {
        distinct_ids(1..=u32::MAX, 3).prop_flat_map(|ids| {
            let count = ids.len();
            let path = ("[0-9a-f]{32}", "[A-Za-z0-9]{1,6}")
                .prop_map(|(stem, extension)| format!("index/{stem}.{extension}"));
            (Just(ids), collection::vec(path, count..=count)).prop_map(|(ids, paths)| {
                ids.into_iter()
                    .zip(paths)
                    .map(|(post_id, expect_index_image)| PostIndexImage {
                        post_id,
                        expect_index_image,
                    })
                    .collect()
            })
        })
    }

    fn any_post_sentinel_date() -> impl Strategy<Value = Vec<PostSentinelDate>> {
        distinct_ids(1_000_001..=u32::MAX, 4).prop_map(|ids| {
            ids.into_iter()
                .map(|post_id| PostSentinelDate { post_id })
                .collect()
        })
    }

    fn any_canteen_duplicate() -> impl Strategy<Value = Vec<CanteenDuplicate>> {
        any_groups(1..=u32::MAX, 2).prop_flat_map(|groups| {
            let count = groups.len();
            (
                Just(groups),
                collection::btree_set(any_day(), count..=count),
            )
                .prop_map(|(groups, days)| {
                    groups
                        .into_iter()
                        .zip(days)
                        .map(|((expect_ids, keep), date)| CanteenDuplicate {
                            date,
                            expect_ids,
                            keep,
                        })
                        .collect()
                })
        })
    }

    fn any_label_duplicate() -> impl Strategy<Value = Vec<LabelDuplicate>> {
        any_groups(1..=u32::MAX, 2).prop_map(|groups| {
            groups
                .into_iter()
                .map(|(expect_ids, keep)| LabelDuplicate { expect_ids, keep })
                .collect()
        })
    }

    fn any_menu_unpositioned() -> impl Strategy<Value = Vec<MenuUnpositioned>> {
        // Parents up to 1,000 (or the root level) and items above, so no
        // item is its own parent.
        (
            any_groups(1_001..=u32::MAX, 1),
            collection::btree_set(prop::option::of(1_u32..=1_000), 3..=3),
            collection::vec(any::<Index>(), 4..=4),
        )
            .prop_map(|(groups, parents, picks)| {
                groups
                    .into_iter()
                    .zip(parents)
                    .map(|((expect_null_lft, _), parent)| MenuUnpositioned {
                        parent_id: parent.map(id),
                        served_order: permuted(&expect_null_lft, &picks),
                        expect_null_lft,
                    })
                    .collect()
            })
    }

    fn any_page_hidden() -> impl Strategy<Value = Vec<PageHidden>> {
        distinct_ids(1..=u32::MAX, 3).prop_flat_map(|ids| {
            let count = ids.len();
            (Just(ids), collection::vec(any_instant(), count..=count)).prop_map(
                |(ids, instants)| {
                    ids.into_iter()
                        .zip(instants)
                        .map(|(page_id, expect_updated_at)| PageHidden {
                            page_id,
                            expect_updated_at,
                        })
                        .collect()
                },
            )
        })
    }

    fn any_n7_accept() -> impl Strategy<Value = Vec<N7Accept>> {
        collection::btree_set(
            prop_oneof![
                any_id().prop_map(BodyRow::Post),
                any_id().prop_map(BodyRow::Page)
            ],
            0..=3,
        )
        .prop_flat_map(|rows| {
            let count = rows.len();
            (
                Just(rows),
                collection::vec(any::<[u8; 32]>(), count..=count),
            )
                .prop_map(|(rows, digests)| {
                    rows.into_iter()
                        .zip(digests)
                        .map(|(body, expect_content_sha256)| N7Accept {
                            body,
                            expect_content_sha256,
                        })
                        .collect()
                })
        })
    }

    /// Valid files: every section with up to a few entries.
    fn any_file() -> impl Strategy<Value = ResolutionFile> {
        (
            prop::option::of(any_day()),
            any_post_date_shift(),
            any_post_author(),
            any_post_index_image(),
            any_post_sentinel_date(),
            any_canteen_duplicate(),
            any_label_duplicate(),
            any_menu_unpositioned(),
            any_page_hidden(),
            any_n7_accept(),
        )
            .prop_map(
                |(
                    signed_off,
                    post_date_shift,
                    post_author,
                    post_index_image,
                    post_sentinel_date,
                    canteen_duplicate,
                    label_duplicate,
                    menu_unpositioned,
                    page_hidden,
                    n7_accept,
                )| ResolutionFile {
                    signed_off,
                    post_date_shift,
                    post_author,
                    post_index_image,
                    post_sentinel_date,
                    canteen_duplicate,
                    label_duplicate,
                    menu_unpositioned,
                    page_hidden,
                    n7_accept,
                },
            )
    }

    /// The canonical text of `file` as a TOML table, to change.
    fn as_table(file: &ResolutionFile) -> Table {
        file.to_toml()
            .parse()
            .expect("the canonical layout is TOML")
    }

    /// A line of the kind a resolution file holds: a field and a value of
    /// one of the kinds the fields take, often of the wrong kind or range.
    const TOML_LIKE_LINE: &str = concat!(
        "(post_id|page_id|keep|parent_id|set_author|expect_ids|served_order",
        "|expect_null_lft|expect_date|expect_updated_at) = ",
        "(-?[0-9]{1,11}|\"[a-z]{0,6}\"|\\[[0-9, ]{0,12}\\]",
        "|19[0-9]{2}-0[1-9]-[0-2][0-9](T10:00:00(Z|\\+01:00)?)?)",
    );

    /// The entries of `section` in `table`.
    fn section_entries(table: &mut Table, section: Section) -> Option<&mut Vec<Value>> {
        match table.get_mut(section.key())? {
            Value::Array(entries) if !entries.is_empty() => Some(entries),
            _ => None,
        }
    }

    proptest! {
        #[test]
        fn a_valid_file_round_trips_through_its_canonical_text(file in any_file()) {
            prop_assert_eq!(parse(&file.to_toml()), Ok(file));
        }

        #[test]
        fn parsing_never_panics_on_any_text(text in "\\PC{0,200}") {
            let _ = parse(&text);
        }

        #[test]
        fn parsing_never_panics_on_toml_like_text(
            lines in collection::vec(
                prop_oneof![
                    Just("version = 1".to_owned()),
                    Just("[[post_author]]".to_owned()),
                    Just("[[N7-accept]]".to_owned()),
                    Just("[[menu_unpositioned]]".to_owned()),
                    TOML_LIKE_LINE,
                ],
                0..20,
            ),
        ) {
            let _ = parse(&lines.join("\n"));
        }

        #[test]
        fn a_removed_field_is_named_as_missing(
            file in any_file(),
            section in prop::sample::select(Section::ALL.to_vec()),
            entry in any::<Index>(),
            field in any::<Index>(),
        ) {
            let mut table = as_table(&file);
            let Some(entries) = section_entries(&mut table, section) else {
                return Ok(());
            };
            let index = entry.index(entries.len());
            let Value::Table(fields) = &mut entries[index] else {
                unreachable!("the canonical layout writes tables");
            };
            let names: Vec<String> = fields.keys().cloned().collect();
            let removed = names[field.index(names.len())].clone();
            fields.remove(&removed);
            let errors = parse(&toml::to_string(&table).expect("TOML")).expect_err("refused");
            let expected = if section == Section::N7Accept && removed != "expect_content_sha256" {
                ResolutionError::entry(
                    section,
                    index + 1,
                    "has neither post_id nor page_id: give exactly one of post_id and page_id, \
                     the row whose body it is"
                        .to_owned(),
                )
            } else {
                ResolutionError::field(
                    section,
                    index + 1,
                    &removed,
                    format!(
                        "missing: every {} entry has {}",
                        section.key(),
                        section.fields_text()
                    ),
                )
            };
            prop_assert_eq!(errors.problems(), &[expected][..]);
        }

        #[test]
        fn an_added_field_is_named_as_unknown(
            file in any_file(),
            section in prop::sample::select(Section::ALL.to_vec()),
            entry in any::<Index>(),
            name in "[a-z][a-z_]{0,12}",
        ) {
            prop_assume!(!section.fields().contains(&name.as_str()));
            let mut table = as_table(&file);
            let Some(entries) = section_entries(&mut table, section) else {
                return Ok(());
            };
            let index = entry.index(entries.len());
            let Value::Table(fields) = &mut entries[index] else {
                unreachable!("the canonical layout writes tables");
            };
            fields.insert(name.clone(), Value::Integer(1));
            let errors = parse(&toml::to_string(&table).expect("TOML")).expect_err("refused");
            let expected = ResolutionError::field(
                section,
                index + 1,
                &name,
                format!(
                    "unknown field: every {} entry has {}, and nothing else",
                    section.key(),
                    section.fields_text()
                ),
            );
            prop_assert_eq!(errors.problems(), &[expected][..]);
        }

        #[test]
        fn a_repeated_entry_is_named_with_the_entry_it_repeats(
            file in any_file(),
            section in prop::sample::select(Section::ALL.to_vec()),
            entry in any::<Index>(),
        ) {
            let mut table = as_table(&file);
            let Some(entries) = section_entries(&mut table, section) else {
                return Ok(());
            };
            let index = entry.index(entries.len());
            let copy = entries[index].clone();
            entries.push(copy);
            let last = entries.len();
            let errors = parse(&toml::to_string(&table).expect("TOML")).expect_err("refused");
            // The copy repeats every key of its entry; the first one its
            // section checks is enough to name it.
            let first = &errors.problems()[0];
            let is_at_copy = matches!(
                &first.location,
                Location::Entry { section: found, index: at, field: Some(_) }
                    if *found == section && *at == last
            );
            prop_assert!(is_at_copy, "{}", first);
            prop_assert!(
                first.problem.ends_with(&format!(
                    "is already in {}: one entry each",
                    section.entry(index + 1)
                )),
                "{}",
                first
            );
            let all_at_copy = errors.problems().iter().all(|problem| matches!(
                &problem.location,
                Location::Entry { index: at, .. } if *at == last
            ));
            prop_assert!(all_at_copy, "{}", errors);
        }

        #[test]
        fn an_id_outside_the_source_range_is_named(
            file in any_file(),
            section in prop::sample::select(Section::ALL.to_vec()),
            entry in any::<Index>(),
            number in prop_oneof![i64::MIN..=0, i64::from(u32::MAX) + 1..=i64::MAX],
        ) {
            let field = match section {
                Section::CanteenDuplicate | Section::LabelDuplicate => "keep",
                Section::MenuUnpositioned => "parent_id",
                Section::PageHidden => "page_id",
                Section::N7Accept => return Ok(()),
                _ => "post_id",
            };
            let mut table = as_table(&file);
            let Some(entries) = section_entries(&mut table, section) else {
                return Ok(());
            };
            let index = entry.index(entries.len());
            let Value::Table(fields) = &mut entries[index] else {
                unreachable!("the canonical layout writes tables");
            };
            fields.insert(field.to_owned(), Value::Integer(number));
            let errors = parse(&toml::to_string(&table).expect("TOML")).expect_err("refused");
            let problem = &errors.problems()[0];
            prop_assert_eq!(
                &problem.location,
                &Location::Entry { section, index: index + 1, field: Some(field.to_owned()) }
            );
            let expected = if section == Section::MenuUnpositioned {
                format!(
                    "{number} is not a menu item id: write {ROW_ID_RULE}, or \"{NONE}\" for the \
                     root level"
                )
            } else {
                format!("{number} is not a row id: write {ROW_ID_RULE}")
            };
            prop_assert_eq!(&problem.problem, &expected);
        }

        #[test]
        fn the_entry_count_is_the_number_of_entry_tables(file in any_file()) {
            // The canonical text writes one `[[section]]` header per entry,
            // and nothing else starts a line with `[[`.
            let text = file.to_toml();
            prop_assert_eq!(file.entry_count(), text.matches("\n[[").count());
            let sections = [
                file.post_date_shift.len(),
                file.post_author.len(),
                file.post_index_image.len(),
                file.post_sentinel_date.len(),
                file.canteen_duplicate.len(),
                file.label_duplicate.len(),
                file.menu_unpositioned.len(),
                file.page_hidden.len(),
                file.n7_accept.len(),
            ];
            prop_assert_eq!(file.entry_count(), sections.iter().sum::<usize>());
            prop_assert_eq!(parse(&text).map(|read| read.entry_count()), Ok(file.entry_count()));
        }

        #[test]
        fn an_unknown_key_or_field_never_breaks_the_one_line_layout(
            name in collection::vec(any::<char>(), 0..16).prop_map(String::from_iter),
            section in prop::sample::select(Section::ALL.to_vec()),
        ) {
            prop_assume!(!TOP_LEVEL_KEYS.contains(&name.as_str()));
            prop_assume!(!section.fields().contains(&name.as_str()));
            // The name as an unknown top-level key, and as an unknown field
            // of an entry of `section` that is otherwise the first entry of
            // that section in the full file.
            let mut table = as_table(&full());
            table.insert(name.clone(), Value::Integer(1));
            let entries = section_entries(&mut table, section).expect("every section is in FULL");
            let Value::Table(fields) = &mut entries[0] else {
                unreachable!("the canonical layout writes tables");
            };
            fields.insert(name.clone(), Value::Integer(1));
            let errors = parse(&toml::to_string(&table).expect("TOML")).expect_err("refused");
            prop_assert_eq!(errors.problems().len(), 2, "{}", errors);
            let shown = shown_name(&name);
            let line_breaks = ['\n', '\r', '\u{0b}', '\u{0c}', '\u{85}', '\u{2028}', '\u{2029}'];
            for problem in errors.problems() {
                let text = problem.to_string();
                prop_assert!(!text.contains(line_breaks), "{:?}", text);
                prop_assert!(text.contains(shown.as_ref()), "{:?}", text);
            }
            // The whole message is the count and one line per problem.
            prop_assert_eq!(errors.to_string().lines().count(), 3, "{}", errors);
        }

        #[test]
        fn index_image_paths_are_exactly_the_documented_shape(
            stem in "[0-9a-f]{32}",
            extension in "[A-Za-z0-9]{1,10}",
            other in "\\PC{0,60}",
        ) {
            let path = format!("index/{stem}.{extension}");
            prop_assert!(is_index_image_path(&path));
            let lower_hex = |byte: u8| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase();
            let parts = other.strip_prefix("index/").and_then(|name| name.split_once('.'));
            let shaped = parts.is_some_and(|(stem, extension)| {
                stem.len() == 32
                    && stem.bytes().all(lower_hex)
                    && !extension.is_empty()
                    && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
            });
            prop_assert_eq!(is_index_image_path(&other), shaped);
        }

        #[test]
        fn digests_round_trip_through_hex(bytes in any::<[u8; 32]>()) {
            prop_assert_eq!(sha256_of(&hex(&bytes)), Some(bytes));
        }

        #[test]
        fn a_file_digest_is_written_as_64_lower_case_hex_digits(
            bytes in collection::vec(any::<u8>(), 0..512),
        ) {
            let digest = FileDigest::of(&bytes);
            let written = digest.to_string();
            prop_assert_eq!(written.len(), 64);
            // The written form is the one N7-accept reads, so it reads back
            // to the same 32 bytes.
            prop_assert_eq!(sha256_of(&written), Some(digest.0));
            prop_assert_eq!(FileDigest::of(&bytes), digest);
        }

        #[test]
        fn a_utc_offset_other_than_zero_is_refused_with_the_utc_instant(
            at in any_instant(),
            minutes in prop_oneof![-1_439_i16..=-1, 1_i16..=1_439],
        ) {
            let local = at.naive_utc() + TimeDelta::minutes(i64::from(minutes));
            prop_assume!((1..=9999).contains(&chrono::Datelike::year(&local)));
            let sign = if minutes < 0 { '-' } else { '+' };
            let written = format!(
                "{}{sign}{:02}:{:02}",
                local.format("%Y-%m-%dT%H:%M:%S"),
                minutes.unsigned_abs() / 60,
                minutes.unsigned_abs() % 60
            );
            let value: Table = format!("at = {written}\n").parse().expect("TOML");
            let refused = instant_value(&value["at"]).expect_err("not UTC");
            prop_assert!(
                refused.ends_with(&format!(", {}", at.format("%Y-%m-%dT%H:%M:%SZ"))),
                "{}",
                refused
            );
        }
    }
}
