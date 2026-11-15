//! The pure rules of owned files: which entity column owns the files of
//! which folder, which paths a `DeleteOwnedFiles` job may name, and how a
//! job's paths are split (ADR 0011 §6 "Files", ADR 0015 §6, ADR 0016 §3).
//!
//! An **owned file** is an entity image: the photo of an author or a
//! colleague, a post's index image or gallery image. Its entity's row keeps
//! its stored value (`media_rules::stored_value`), and nothing but that
//! row, its trashed self and its revisions refer to it. Media-library files
//! under `uploads/` are never owned files: the library manages them, and a
//! post's editor uploads stay when the post is purged (ADR 0011 §6).
//!
//! - **The file-owner registry.** [`FILE_OWNERS`] lists every column that
//!   stores the value of an owned file, as a [`FileOwner`]: the entity's
//!   table, the column, and the folder below `media.root` whose files it
//!   names. Each upload slice joins it with one line, in the PR that adds the
//!   column. The worker checks a file against every owner of its folder, so a
//!   column missing from the list would let its files go while rows still
//!   name them; [`require_owners`] therefore refuses a path whose folder has
//!   no owner at all, and the owners' tests in `tests/models/owned_files.rs`
//!   check that every registered column exists. [`check_file_owners`] keeps
//!   the names to plain `snake_case` identifiers, because the model writes
//!   them into its SQL. A registered column is also a field of its entity's
//!   revision snapshot under the same name, as `RevisionSnapshot::FIELDS`
//!   names every column: a purge reads the column's files from every
//!   revision by that name, and [`snapshot_owners`] refuses a snapshot type
//!   that lacks it.
//! - **A column's values.** [`column_values`] reads the stored values from
//!   a column's JSON form, a row's or a revision's: a string, or an array
//!   of strings, with anything else set apart for the caller to warn about.
//! - **The job's paths.** A `DeleteOwnedFiles` job takes paths only, each the
//!   path of one file below `media.root` (`<disk>/<stored value>`, as
//!   `media_rules::stored_image_path` writes it): never row ids, which a
//!   purge has made meaningless, and never file contents (ADR 0016 §3,
//!   "Arguments are ids and small values"). [`OwnedPath::parse`] accepts
//!   exactly one file directly inside an entity image folder, and
//!   [`OwnedPath::from_stored_value`] builds one from a row's stored value.
//!   Legacy names are not all 32 hex digits (data-migration.md §6.1, §6.4,
//!   §6.5), so the name is checked only as a safe path segment.
//! - **Small jobs.** [`job_batches`] drops repeated paths and splits the rest
//!   into jobs of at most [`MAX_PATHS_PER_JOB`], so the arguments stay small
//!   and a job ends well inside the reaper's 30 minutes (ADR 0016 §3);
//!   [`check_job_paths`] refuses a job with more.

use std::{collections::HashSet, fmt};

use serde_json::Value;

use super::{
    activity::is_snake_case,
    media_rules::{EntityImageFolder, PathRuleError, relative_segments},
};

/// One column that stores the value of an owned file (the module
/// documentation, "The file-owner registry").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileOwner {
    /// The entity's table, which is also the `entity_type` of its revisions
    /// (`RevisionSnapshot::ENTITY_TYPE`, ADR 0011 §4), for example `authors`.
    pub entity: &'static str,
    /// The column of that table that stores the file's stored value: a text
    /// column with one value (`authors.image`), or a `jsonb` array of values
    /// (`posts.images`, data-migration.md §6.1). The entity's revision
    /// snapshot type lists it in `RevisionSnapshot::FIELDS` under the same
    /// name, as that list names every column ([`snapshot_owners`]).
    pub column: &'static str,
    /// The folder below `media.root` that holds the files the column names.
    pub folder: EntityImageFolder,
}

/// The file-owner registry: every column that stores the value of an owned
/// file. Each upload slice adds the line of its column here, in the PR that
/// adds the column, for example
/// `FileOwner { entity: "authors", column: "image", folder: EntityImageFolder::AuthorImages }`.
/// `App::after_context` refuses to start on a registry that breaks
/// [`check_file_owners`]. The column is a field of the same name in the
/// entity's revision snapshot type, or the purge's
/// `models::owned_files::files_of_record` refuses to run
/// ([`snapshot_owners`]).
///
/// It is empty until the first upload slice joins: until then no file has an
/// owner, and the worker refuses every path ([`require_owners`]).
pub const FILE_OWNERS: &[FileOwner] = &[];

/// A registry that breaks a rule of [`check_file_owners`]: a mistake of the
/// code that defines it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FileOwnerError {
    /// The entity is not a `snake_case` table name.
    #[error(
        "the file owner's entity {entity:?} is not a table name: it starts with a lowercase ASCII \
         letter and holds only lowercase ASCII letters, digits and underscores"
    )]
    EntityNotSnakeCase {
        /// The refused entity.
        entity: &'static str,
    },
    /// The column is not a `snake_case` column name.
    #[error(
        "the file owner {entity}.{column:?} names no column: a column name starts with a \
         lowercase ASCII letter and holds only lowercase ASCII letters, digits and underscores"
    )]
    ColumnNotSnakeCase {
        /// The owner's entity.
        entity: &'static str,
        /// The refused column.
        column: &'static str,
    },
    /// The same column is listed twice.
    #[error("the file owner {entity}.{column} is listed more than once")]
    Duplicate {
        /// The owner's entity.
        entity: &'static str,
        /// The column listed twice.
        column: &'static str,
    },
    /// The entity's revision snapshot type does not list the column.
    #[error(
        "the file owner {entity}.{column} is not a field of the revision snapshot of {entity}: \
         its snapshot type lists the column under the same name in RevisionSnapshot::FIELDS, so \
         that a purge reads the column's files from every revision"
    )]
    NotASnapshotField {
        /// The owner's entity.
        entity: &'static str,
        /// The column the snapshot type lacks.
        column: &'static str,
    },
}

/// Checks a file-owner registry: every entity and column is a `snake_case`
/// name, so the model can write it into SQL as a quoted identifier with no
/// way to carry SQL of its own, and no column is listed twice.
///
/// - `owners`: the registry, [`FILE_OWNERS`] or a test's own.
///
/// # Errors
///
/// The first [`FileOwnerError`] of the list, in its order.
pub fn check_file_owners(owners: &[FileOwner]) -> Result<(), FileOwnerError> {
    let mut seen = HashSet::new();
    for owner in owners {
        if !is_snake_case(owner.entity) {
            return Err(FileOwnerError::EntityNotSnakeCase {
                entity: owner.entity,
            });
        }
        if !is_snake_case(owner.column) {
            return Err(FileOwnerError::ColumnNotSnakeCase {
                entity: owner.entity,
                column: owner.column,
            });
        }
        if !seen.insert((owner.entity, owner.column)) {
            return Err(FileOwnerError::Duplicate {
                entity: owner.entity,
                column: owner.column,
            });
        }
    }
    Ok(())
}

/// The owners whose files are in `folder`, in the registry's order.
///
/// - `owners`: the registry.
/// - `folder`: the folder of a file.
#[must_use]
pub fn owners_of(owners: &[FileOwner], folder: EntityImageFolder) -> Vec<FileOwner> {
    owners
        .iter()
        .filter(|owner| owner.folder == folder)
        .copied()
        .collect()
}

/// The owners of `entity`'s columns, in the registry's order, each checked
/// to be a field of the entity's revision snapshot type. A purge reads a
/// column's files from the row and from the member of the same name in
/// every revision (`models::owned_files::files_of_record`); a snapshot that
/// named the column otherwise would leave the files that only old revisions
/// show on the volume, with no sign of it.
///
/// - `owners`: the registry.
/// - `entity`: the entity's table, its snapshot type's
///   `RevisionSnapshot::ENTITY_TYPE`.
/// - `fields`: the snapshot type's `RevisionSnapshot::FIELDS`.
///
/// # Errors
///
/// [`FileOwnerError::NotASnapshotField`] for the first owner of `entity`
/// whose column `fields` does not list.
pub fn snapshot_owners(
    owners: &[FileOwner],
    entity: &str,
    fields: &[&str],
) -> Result<Vec<FileOwner>, FileOwnerError> {
    owners
        .iter()
        .filter(|owner| owner.entity == entity)
        .map(|owner| {
            if fields.contains(&owner.column) {
                Ok(*owner)
            } else {
                Err(FileOwnerError::NotASnapshotField {
                    entity: owner.entity,
                    column: owner.column,
                })
            }
        })
        .collect()
}

/// The values of an owned column, read from its JSON form by
/// [`column_values`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColumnValues<'a> {
    /// The stored values, in their order: the string itself, or the strings
    /// of an array.
    pub values: Vec<&'a str>,
    /// What is no stored value: a number, a boolean or an object, or an
    /// array item that is not a string, `null` among them. backend3 never
    /// writes one (its columns are text or arrays of text, and the snapshot
    /// types hold `Option<String>` and `Vec<String>`), so the caller warns.
    pub unusable: Vec<&'a Value>,
}

/// The stored values of an owned column in its JSON form: a row's column as
/// PostgreSQL's `to_jsonb` gives it, or a revision snapshot's member.
///
/// - `json`: the column's value. A string is one value (a text column such
///   as `authors.image`), an array holds one value per item (`posts.images`),
///   and `null` holds none (a cleared column); anything else is unusable.
#[must_use]
pub fn column_values(json: &Value) -> ColumnValues<'_> {
    let mut found = ColumnValues::default();
    match json {
        Value::Null => {}
        Value::String(value) => found.values.push(value),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(value) => found.values.push(value),
                    other => found.unusable.push(other),
                }
            }
        }
        other => found.unusable.push(other),
    }
    found
}

/// One file that a `DeleteOwnedFiles` job may name: exactly one file
/// directly inside an entity image folder, checked by [`OwnedPath::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnedPath {
    /// The file's folder.
    folder: EntityImageFolder,
    /// The path below `media.root`, `<folder>/<name>`.
    path: String,
}

impl OwnedPath {
    /// Reads a job's path.
    ///
    /// - `path`: a `/`-separated path below `media.root`, such as
    ///   `authors_images/<name>` or `posts_images/index/<name>`.
    ///
    /// The path is one plain segment, the file's name, below the segments of
    /// an [`EntityImageFolder`]; `posts_images/index/<name>` is a post's
    /// index image, the deeper of the two folders that match.
    ///
    /// # Errors
    ///
    /// - [`OwnedPathError::Unsafe`] when the path breaks
    ///   `media_rules::relative_segments` (empty, absolute, a dot-segment, a
    ///   backslash or control character, a segment longer than 255 bytes);
    /// - [`OwnedPathError::NamesAFolder`] for the path of a folder itself,
    ///   such as `posts_images/index`;
    /// - [`OwnedPathError::NotInAnEntityImageFolder`] for any other path,
    ///   such as one under `uploads/` or one a folder deeper.
    pub fn parse(path: &str) -> Result<Self, OwnedPathError> {
        let segments = relative_segments(path).map_err(OwnedPathError::Unsafe)?;
        if EntityImageFolder::ALL
            .iter()
            .any(|folder| folder.segments() == segments.as_slice())
        {
            return Err(OwnedPathError::NamesAFolder);
        }
        let (_, parent) = segments
            .split_last()
            .ok_or(OwnedPathError::NotInAnEntityImageFolder)?;
        let folder = EntityImageFolder::ALL
            .into_iter()
            .find(|folder| folder.segments() == parent)
            .ok_or(OwnedPathError::NotInAnEntityImageFolder)?;
        Ok(Self {
            folder,
            path: path.to_owned(),
        })
    }

    /// The path of the file that a row of `folder`'s owner stores as
    /// `stored_value`: `<disk>/<stored value>` (public-api.md §2.3).
    ///
    /// - `folder`: the folder of the column's owner ([`FileOwner::folder`]).
    /// - `stored_value`: the value the row stores, such as `<name>` or, for a
    ///   post's index image, `index/<name>`.
    ///
    /// # Errors
    ///
    /// The errors of [`OwnedPath::parse`] for the joined path, and
    /// [`OwnedPathError::FolderMismatch`] when it lies in another folder than
    /// `folder`, such as `index/<name>` for a gallery image.
    pub fn from_stored_value(
        folder: EntityImageFolder,
        stored_value: &str,
    ) -> Result<Self, OwnedPathError> {
        let parsed = Self::parse(&format!("{}/{stored_value}", folder.disk()))?;
        if parsed.folder == folder {
            Ok(parsed)
        } else {
            Err(OwnedPathError::FolderMismatch {
                expected: folder,
                found: parsed.folder,
            })
        }
    }

    /// The file's folder.
    #[must_use]
    pub const fn folder(&self) -> EntityImageFolder {
        self.folder
    }

    /// The path below `media.root`, as the job names it.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The file's name: the last segment of the path.
    #[must_use]
    pub fn name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(&self.path, |(_, name)| name)
    }

    /// The value an owner's row stores for this file: the path below the
    /// folder's disk (`media_rules::stored_value`), `<name>`, or
    /// `index/<name>` for a post's index image.
    #[must_use]
    pub fn stored_value(&self) -> &str {
        // `parse` found the disk as the path's first segment.
        &self.path[self.folder.disk().len() + 1..]
    }
}

impl fmt::Display for OwnedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.path)
    }
}

/// Why a path is not one owned file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnedPathError {
    /// The path is not a safe path below the media root.
    #[error("the path is not a safe path below the media root: {0}")]
    Unsafe(PathRuleError),
    /// The path is the path of an entity image folder itself.
    #[error("the path names an entity image folder, not a file in it")]
    NamesAFolder,
    /// The path is not one file directly inside an entity image folder.
    #[error(
        "the path is not a file directly inside posts_images/, posts_images/index/, \
         authors_images/ or colleagues_images/"
    )]
    NotInAnEntityImageFolder,
    /// A stored value leads into another folder than its owner's.
    #[error("the stored value belongs in {}/, not in {}/", .found.path(), .expected.path())]
    FolderMismatch {
        /// The owner's folder.
        expected: EntityImageFolder,
        /// The folder the stored value leads to.
        found: EntityImageFolder,
    },
}

/// The most paths one `DeleteOwnedFiles` job takes: 256. At most about 70
/// bytes each (a folder, 32 hex digits and an extension), that keeps a job's
/// arguments below 20 KiB, and 256 files with two short queries each end in
/// seconds, far inside the reaper's 30 minutes (ADR 0016 §3).
pub const MAX_PATHS_PER_JOB: usize = 256;

/// Why a job's paths are refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobPathsError {
    /// The job names more than [`MAX_PATHS_PER_JOB`] paths.
    #[error(
        "the job names {count} paths, and one job takes at most {max}; split them with \
         job_batches"
    )]
    TooMany {
        /// How many paths the job names.
        count: usize,
        /// [`MAX_PATHS_PER_JOB`].
        max: usize,
    },
    /// A path is not one owned file.
    #[error("path {index} ({path:?}) is refused: {error}")]
    Path {
        /// The path's position in the job, from 0.
        index: usize,
        /// The refused path.
        path: String,
        /// Why.
        error: OwnedPathError,
    },
}

/// Checks a job's paths, all of them before any is used, so a job with one
/// bad path changes nothing.
///
/// - `paths`: the job's paths, as it was enqueued with them.
///
/// # Errors
///
/// [`JobPathsError::TooMany`] above [`MAX_PATHS_PER_JOB`], otherwise
/// [`JobPathsError::Path`] for the first path that [`OwnedPath::parse`]
/// refuses.
pub fn check_job_paths(paths: &[String]) -> Result<Vec<OwnedPath>, JobPathsError> {
    if paths.len() > MAX_PATHS_PER_JOB {
        return Err(JobPathsError::TooMany {
            count: paths.len(),
            max: MAX_PATHS_PER_JOB,
        });
    }
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            OwnedPath::parse(path).map_err(|error| JobPathsError::Path {
                index,
                path: path.clone(),
                error,
            })
        })
        .collect()
}

/// The paths of the jobs that delete `paths`: each path once, in the order it
/// first appears, in jobs of at most [`MAX_PATHS_PER_JOB`]. No paths make no
/// job.
///
/// - `paths`: the files whose references a transaction removed.
#[must_use]
pub fn job_batches(paths: &[OwnedPath]) -> Vec<Vec<String>> {
    let mut seen = HashSet::new();
    let unique: Vec<String> = paths
        .iter()
        .filter(|path| seen.insert(path.path()))
        .map(|path| path.path().to_owned())
        .collect();
    unique
        .chunks(MAX_PATHS_PER_JOB)
        .map(<[String]>::to_vec)
        .collect()
}

/// A path whose folder no registered column owns.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "no file owner is registered for {}/, so nothing can tell whether {path} is still referenced; \
     the upload slice that stores files there adds its column to FILE_OWNERS \
     (src/domain/owned_files.rs)",
    .folder.path()
)]
pub struct NoOwner {
    /// The path.
    pub path: String,
    /// Its folder.
    pub folder: EntityImageFolder,
}

/// Refuses a path whose folder has no owner in the registry. Without one the
/// worker would find no reference and delete a file that rows of an
/// unregistered column may still name.
///
/// - `owners`: the registry.
/// - `paths`: the job's checked paths.
///
/// # Errors
///
/// [`NoOwner`] for the first such path.
pub fn require_owners(owners: &[FileOwner], paths: &[OwnedPath]) -> Result<(), NoOwner> {
    match paths
        .iter()
        .find(|path| !owners.iter().any(|owner| owner.folder == path.folder()))
    {
        Some(path) => Err(NoOwner {
            path: path.path().to_owned(),
            folder: path.folder(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::media_rules::{
        ENTITY_IMAGE_NAME_BYTES, ImageExtension, entity_image_name, stored_image_path, stored_value,
    };
    use proptest::{collection::vec, prelude::*, sample::select};
    use serde_json::json;

    /// An invented entity image name.
    const NAME: &str = "0123456789abcdef0123456789abcdef.jpg";

    /// A registry of the four folders, as the upload slices will write it.
    const OWNERS: &[FileOwner] = &[
        FileOwner {
            entity: "authors",
            column: "image",
            folder: EntityImageFolder::AuthorImages,
        },
        FileOwner {
            entity: "colleagues",
            column: "image",
            folder: EntityImageFolder::ColleagueImages,
        },
        FileOwner {
            entity: "posts",
            column: "index_image",
            folder: EntityImageFolder::PostIndexImages,
        },
        FileOwner {
            entity: "posts",
            column: "images",
            folder: EntityImageFolder::PostImages,
        },
    ];

    #[test]
    fn the_production_registry_passes_its_checks() {
        assert_eq!(check_file_owners(FILE_OWNERS), Ok(()));
    }

    #[test]
    fn a_registry_of_plain_names_passes() {
        assert_eq!(check_file_owners(OWNERS), Ok(()));
        assert_eq!(check_file_owners(&[]), Ok(()));
    }

    #[test]
    fn a_registry_with_a_name_that_could_carry_sql_is_refused() {
        let owner = |entity, column| FileOwner {
            entity,
            column,
            folder: EntityImageFolder::AuthorImages,
        };
        for entity in [
            "",
            "Authors",
            "authors\"; DROP TABLE x; --",
            "1authors",
            "a-b",
        ] {
            assert_eq!(
                check_file_owners(&[owner(entity, "image")]),
                Err(FileOwnerError::EntityNotSnakeCase { entity }),
                "{entity:?}"
            );
        }
        for column in ["", "Image", "image\"", "_image", "image name"] {
            assert_eq!(
                check_file_owners(&[owner("authors", column)]),
                Err(FileOwnerError::ColumnNotSnakeCase {
                    entity: "authors",
                    column
                }),
                "{column:?}"
            );
        }
    }

    #[test]
    fn a_column_listed_twice_is_refused_even_with_another_folder() {
        let twice = [
            OWNERS[0],
            FileOwner {
                folder: EntityImageFolder::ColleagueImages,
                ..OWNERS[0]
            },
        ];
        assert_eq!(
            check_file_owners(&twice),
            Err(FileOwnerError::Duplicate {
                entity: "authors",
                column: "image"
            })
        );
        // The same column name in two tables is two owners.
        assert_eq!(check_file_owners(&OWNERS[..2]), Ok(()));
    }

    #[test]
    fn each_folder_has_its_owners() {
        assert_eq!(
            owners_of(OWNERS, EntityImageFolder::AuthorImages),
            vec![OWNERS[0]]
        );
        assert_eq!(
            owners_of(OWNERS, EntityImageFolder::PostImages),
            vec![OWNERS[3]]
        );
        assert_eq!(
            owners_of(&OWNERS[..2], EntityImageFolder::PostImages),
            Vec::new()
        );
    }

    #[test]
    fn an_entitys_owners_are_fields_of_its_snapshot() {
        assert_eq!(
            snapshot_owners(OWNERS, "posts", &["title", "index_image", "images"]),
            Ok(vec![OWNERS[2], OWNERS[3]])
        );
        // An entity with no registered column has no owners to check.
        assert_eq!(snapshot_owners(OWNERS, "labels", &["name"]), Ok(Vec::new()));
        // A snapshot that names the gallery `gallery` would hide the files
        // that only old revisions show.
        let error = snapshot_owners(OWNERS, "posts", &["index_image", "gallery"])
            .expect_err("no images field");
        assert_eq!(
            error,
            FileOwnerError::NotASnapshotField {
                entity: "posts",
                column: "images"
            }
        );
        assert!(
            error.to_string().contains("RevisionSnapshot::FIELDS"),
            "{error}"
        );
    }

    #[test]
    fn a_column_holds_a_string_an_array_of_strings_or_nothing() {
        let one = json!("a.jpg");
        assert_eq!(
            column_values(&one),
            ColumnValues {
                values: vec!["a.jpg"],
                unusable: Vec::new()
            }
        );
        let many = json!(["a.jpg", "b.jpg"]);
        assert_eq!(column_values(&many).values, vec!["a.jpg", "b.jpg"]);
        assert_eq!(column_values(&json!(null)), ColumnValues::default());
        assert_eq!(column_values(&json!([])), ColumnValues::default());
    }

    #[test]
    fn what_is_no_stored_value_is_set_apart_in_order() {
        // A `null` item, a number and an object in an array: backend3 never
        // writes one, and none of them names a file.
        let mixed = json!([null, "a.jpg", 5, {"file": "b.jpg"}, "c.jpg"]);
        assert_eq!(
            column_values(&mixed),
            ColumnValues {
                values: vec!["a.jpg", "c.jpg"],
                unusable: vec![&json!(null), &json!(5), &json!({"file": "b.jpg"})],
            }
        );
        for scalar in [json!(5), json!(true), json!({"image": "a.jpg"})] {
            assert_eq!(
                column_values(&scalar),
                ColumnValues {
                    values: Vec::new(),
                    unusable: vec![&scalar]
                },
                "{scalar}"
            );
        }
    }

    #[test]
    fn a_file_in_each_folder_parses_with_its_stored_value() {
        for (path, folder, stored) in [
            (
                format!("authors_images/{NAME}"),
                EntityImageFolder::AuthorImages,
                NAME.to_owned(),
            ),
            (
                format!("colleagues_images/{NAME}"),
                EntityImageFolder::ColleagueImages,
                NAME.to_owned(),
            ),
            (
                format!("posts_images/{NAME}"),
                EntityImageFolder::PostImages,
                NAME.to_owned(),
            ),
            (
                format!("posts_images/index/{NAME}"),
                EntityImageFolder::PostIndexImages,
                format!("index/{NAME}"),
            ),
        ] {
            let parsed = OwnedPath::parse(&path).expect(&path);
            assert_eq!(parsed.folder(), folder, "{path}");
            assert_eq!(parsed.path(), path);
            assert_eq!(parsed.to_string(), path);
            assert_eq!(parsed.name(), NAME);
            assert_eq!(parsed.stored_value(), stored, "{path}");
            assert_eq!(OwnedPath::from_stored_value(folder, &stored), Ok(parsed));
        }
    }

    #[test]
    fn legacy_names_that_are_not_hex_still_parse() {
        // data-migration.md §6.1 keeps the five non-image gallery entries,
        // and §6.4 keeps author file names verbatim.
        for name in [
            "0123456789abcdef0123456789abcdef.PDF",
            "Kovács Anna (1).JPG",
            "a",
        ] {
            let path = format!("posts_images/{name}");
            assert_eq!(
                OwnedPath::parse(&path).map(|p| p.name().to_owned()),
                Ok(name.to_owned())
            );
        }
    }

    #[test]
    fn paths_outside_the_entity_image_folders_are_refused() {
        for path in [
            "uploads/posts/12/photo.jpg",
            "sitemap.xml",
            "authors_images/sub/file.jpg",
            "posts_images/index/sub/file.jpg",
            "other_images/file.jpg",
            "authors_images",
            "colleagues_images",
            "posts_images",
            "posts_images/index",
        ] {
            let error = OwnedPath::parse(path).expect_err(path);
            assert!(
                matches!(
                    error,
                    OwnedPathError::NotInAnEntityImageFolder | OwnedPathError::NamesAFolder
                ),
                "{path}: {error:?}"
            );
        }
        assert_eq!(
            OwnedPath::parse("posts_images/index"),
            Err(OwnedPathError::NamesAFolder)
        );
        assert_eq!(
            OwnedPath::parse("authors_images"),
            Err(OwnedPathError::NamesAFolder)
        );
        assert_eq!(
            OwnedPath::parse("uploads/x.jpg"),
            Err(OwnedPathError::NotInAnEntityImageFolder)
        );
    }

    #[test]
    fn unsafe_paths_are_refused_with_the_path_rule() {
        for (path, rule) in [
            ("", PathRuleError::Empty),
            ("/authors_images/a.jpg", PathRuleError::Absolute),
            ("authors_images//a.jpg", PathRuleError::EmptySegment),
            ("authors_images/../etc", PathRuleError::DotSegment),
            ("authors_images/.hidden", PathRuleError::DotSegment),
            ("authors_images/a\\b.jpg", PathRuleError::ForbiddenCharacter),
            ("authors_images/a\0.jpg", PathRuleError::ForbiddenCharacter),
        ] {
            assert_eq!(
                OwnedPath::parse(path),
                Err(OwnedPathError::Unsafe(rule)),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_stored_value_must_lead_into_its_owners_folder() {
        assert_eq!(
            OwnedPath::from_stored_value(EntityImageFolder::PostImages, &format!("index/{NAME}")),
            Err(OwnedPathError::FolderMismatch {
                expected: EntityImageFolder::PostImages,
                found: EntityImageFolder::PostIndexImages,
            })
        );
        assert_eq!(
            OwnedPath::from_stored_value(EntityImageFolder::PostIndexImages, NAME),
            Err(OwnedPathError::FolderMismatch {
                expected: EntityImageFolder::PostIndexImages,
                found: EntityImageFolder::PostImages,
            })
        );
        assert_eq!(
            OwnedPath::from_stored_value(EntityImageFolder::AuthorImages, "../x.jpg"),
            Err(OwnedPathError::Unsafe(PathRuleError::DotSegment))
        );
        assert_eq!(
            OwnedPath::from_stored_value(EntityImageFolder::AuthorImages, ""),
            Err(OwnedPathError::Unsafe(PathRuleError::EmptySegment))
        );
        let error = OwnedPath::from_stored_value(EntityImageFolder::PostImages, "index/a.jpg")
            .expect_err("index image");
        assert_eq!(
            error.to_string(),
            "the stored value belongs in posts_images/index/, not in posts_images/"
        );
    }

    #[test]
    fn a_job_is_checked_whole_and_names_the_first_bad_path() {
        let good = format!("authors_images/{NAME}");
        assert_eq!(check_job_paths(&[]), Ok(Vec::new()));
        assert_eq!(
            check_job_paths(std::slice::from_ref(&good)),
            Ok(vec![OwnedPath::parse(&good).expect("good")])
        );
        let error = check_job_paths(&[good.clone(), "uploads/a.pdf".to_owned(), "/x".to_owned()])
            .expect_err("a bad path");
        assert_eq!(
            error,
            JobPathsError::Path {
                index: 1,
                path: "uploads/a.pdf".to_owned(),
                error: OwnedPathError::NotInAnEntityImageFolder,
            }
        );
        assert!(
            error.to_string().contains("path 1 (\"uploads/a.pdf\")"),
            "{error}"
        );
    }

    #[test]
    fn a_job_takes_at_most_the_maximum() {
        let path = format!("authors_images/{NAME}");
        let full = vec![path.clone(); MAX_PATHS_PER_JOB];
        assert_eq!(
            check_job_paths(&full).map(|paths| paths.len()),
            Ok(MAX_PATHS_PER_JOB)
        );
        let over = vec![path; MAX_PATHS_PER_JOB + 1];
        let error = check_job_paths(&over).expect_err("one too many");
        assert_eq!(
            error,
            JobPathsError::TooMany {
                count: MAX_PATHS_PER_JOB + 1,
                max: MAX_PATHS_PER_JOB
            }
        );
        assert!(error.to_string().contains("job_batches"), "{error}");
    }

    #[test]
    fn batches_drop_repeats_and_split_at_the_maximum() {
        assert_eq!(job_batches(&[]), Vec::<Vec<String>>::new());
        let paths: Vec<OwnedPath> = (0..=MAX_PATHS_PER_JOB)
            .map(|n| OwnedPath::parse(&format!("authors_images/{n}.jpg")).expect("a path"))
            .collect();
        let mut with_repeats = paths.clone();
        with_repeats.insert(1, paths[0].clone());
        with_repeats.push(paths[5].clone());
        let batches = job_batches(&with_repeats);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), MAX_PATHS_PER_JOB);
        assert_eq!(
            batches[1],
            vec![format!("authors_images/{MAX_PATHS_PER_JOB}.jpg")]
        );
        assert_eq!(batches[0][0], "authors_images/0.jpg");
        assert_eq!(batches[0][1], "authors_images/1.jpg");
    }

    #[test]
    fn a_path_without_an_owner_is_refused() {
        let author = OwnedPath::parse(&format!("authors_images/{NAME}")).expect("author");
        let gallery = OwnedPath::parse(&format!("posts_images/{NAME}")).expect("gallery");
        assert_eq!(
            require_owners(OWNERS, &[author.clone(), gallery.clone()]),
            Ok(())
        );
        assert_eq!(require_owners(&[], &[]), Ok(()));
        let error = require_owners(&OWNERS[..1], &[author, gallery]).expect_err("no owner");
        assert_eq!(
            error,
            NoOwner {
                path: format!("posts_images/{NAME}"),
                folder: EntityImageFolder::PostImages,
            }
        );
        assert!(error.to_string().contains("FILE_OWNERS"), "{error}");
        // The empty production registry owns nothing yet.
        let colleague = OwnedPath::parse(&format!("colleagues_images/{NAME}")).expect("colleague");
        assert!(require_owners(FILE_OWNERS, &[colleague]).is_err());
    }

    /// A plain path segment: no dot first, no slash, backslash or control
    /// character.
    fn segment() -> impl Strategy<Value = String> {
        "[^./\\\\\\p{Cc}][^/\\\\\\p{Cc}]{0,20}"
    }

    /// A `snake_case` name.
    fn snake() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,12}"
    }

    /// Any JSON value, nested up to three levels.
    fn json_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(Value::from),
            "\\PC{0,12}".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 24, 6, |inner| {
            prop_oneof![
                vec(inner.clone(), 0..6).prop_map(Value::Array),
                vec(("[a-z]{1,6}", inner), 0..4)
                    .prop_map(|members| Value::Object(members.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn every_stored_image_path_parses_back_to_its_folder_and_value(
            random in any::<[u8; ENTITY_IMAGE_NAME_BYTES]>(),
            extension in select(ImageExtension::ALL.to_vec()),
            folder in select(EntityImageFolder::ALL.to_vec()),
        ) {
            let name = entity_image_name(random, extension);
            let path = stored_image_path(folder, &name);
            let parsed = OwnedPath::parse(&path).expect("a stored image path");
            prop_assert_eq!(parsed.folder(), folder);
            prop_assert_eq!(parsed.name(), name.as_str());
            prop_assert_eq!(parsed.stored_value(), stored_value(folder, &name));
            prop_assert_eq!(
                OwnedPath::from_stored_value(folder, &stored_value(folder, &name)),
                Ok(parsed)
            );
        }

        #[test]
        fn any_plain_name_in_a_folder_parses_unless_it_is_the_index_folder(
            name in segment(),
            folder in select(EntityImageFolder::ALL.to_vec()),
        ) {
            let path = format!("{}/{name}", folder.path());
            let parsed = OwnedPath::parse(&path);
            if folder == EntityImageFolder::PostImages && name == "index" {
                prop_assert_eq!(parsed, Err(OwnedPathError::NamesAFolder));
            } else {
                let parsed = parsed.expect("a plain name");
                prop_assert_eq!(parsed.folder(), folder);
                prop_assert_eq!(parsed.name(), name.as_str());
                prop_assert_eq!(format!("{}/{}", folder.disk(), parsed.stored_value()), path);
            }
        }

        #[test]
        fn arbitrary_paths_never_panic_and_accepted_ones_are_one_file_in_a_folder(
            path in "\\PC{0,60}",
        ) {
            if let Ok(parsed) = OwnedPath::parse(&path) {
                let segments = relative_segments(&path).expect("a safe path");
                prop_assert_eq!(&segments[..segments.len() - 1], parsed.folder().segments());
                prop_assert!(path.starts_with(parsed.folder().disk()));
                prop_assert!(!EntityImageFolder::ALL.iter().any(|f| f.path() == path));
            }
        }

        #[test]
        fn a_job_is_accepted_exactly_when_every_path_is(
            paths in vec(prop_oneof![
                segment().prop_map(|name| format!("authors_images/{name}")),
                segment().prop_map(|name| format!("uploads/{name}")),
                "\\PC{0,20}",
            ], 0..8),
        ) {
            let checked = check_job_paths(&paths);
            let all_good = paths.iter().all(|path| OwnedPath::parse(path).is_ok());
            prop_assert_eq!(checked.is_ok(), all_good);
            if let Ok(checked) = checked {
                let back: Vec<&str> = checked.iter().map(OwnedPath::path).collect();
                prop_assert_eq!(back, paths.iter().map(String::as_str).collect::<Vec<_>>());
            }
        }

        #[test]
        fn batches_keep_each_path_once_in_order_and_stay_small(
            names in vec(0..400_usize, 0..700),
        ) {
            let paths: Vec<OwnedPath> = names
                .iter()
                .map(|n| OwnedPath::parse(&format!("posts_images/{n}.jpg")).expect("a path"))
                .collect();
            let batches = job_batches(&paths);
            prop_assert!(batches.iter().all(|batch| !batch.is_empty() && batch.len() <= MAX_PATHS_PER_JOB));
            let flat: Vec<String> = batches.concat();
            let mut seen = HashSet::new();
            let expected: Vec<String> = paths
                .iter()
                .map(|path| path.path().to_owned())
                .filter(|path| seen.insert(path.clone()))
                .collect();
            prop_assert_eq!(flat, expected);
            for batch in &batches {
                prop_assert!(check_job_paths(batch).is_ok());
            }
        }

        #[test]
        fn registries_of_distinct_snake_case_columns_pass(
            columns in vec((snake(), snake(), select(EntityImageFolder::ALL.to_vec())), 0..6),
        ) {
            let mut seen = HashSet::new();
            let owners: Vec<FileOwner> = columns
                .into_iter()
                .filter(|(entity, column, _)| seen.insert((entity.clone(), column.clone())))
                .map(|(entity, column, folder)| FileOwner {
                    entity: Box::leak(entity.into_boxed_str()),
                    column: Box::leak(column.into_boxed_str()),
                    folder,
                })
                .collect();
            prop_assert_eq!(check_file_owners(&owners), Ok(()));
            if let Some(first) = owners.first() {
                let mut twice = owners.clone();
                twice.push(*first);
                prop_assert_eq!(
                    check_file_owners(&twice),
                    Err(FileOwnerError::Duplicate { entity: first.entity, column: first.column })
                );
            }
        }

        #[test]
        fn every_item_of_a_column_is_a_value_or_unusable_and_only_strings_are_values(
            json in json_value(),
        ) {
            let found = column_values(&json);
            match &json {
                Value::Null => prop_assert_eq!(found, ColumnValues::default()),
                Value::String(value) => prop_assert_eq!(found.values, vec![value.as_str()]),
                Value::Array(items) => {
                    let strings: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
                    let others: Vec<&Value> = items.iter().filter(|item| !item.is_string()).collect();
                    prop_assert_eq!(found.values, strings);
                    prop_assert_eq!(found.unusable, others);
                }
                other => {
                    prop_assert!(found.values.is_empty());
                    prop_assert_eq!(found.unusable, vec![other]);
                }
            }
        }

        #[test]
        fn an_entitys_owners_pass_exactly_when_the_snapshot_lists_each_column(
            registered in vec((select(vec!["authors", "posts"]), snake()), 0..6),
            fields in vec(snake(), 0..6),
            entity in select(vec!["authors", "posts", "labels"]),
        ) {
            let owners: Vec<FileOwner> = registered
                .into_iter()
                .map(|(entity, column)| FileOwner {
                    entity,
                    column: Box::leak(column.into_boxed_str()),
                    folder: EntityImageFolder::AuthorImages,
                })
                .collect();
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            let of_entity: Vec<FileOwner> =
                owners.iter().filter(|owner| owner.entity == entity).copied().collect();
            let listed = of_entity.iter().all(|owner| fields.contains(&owner.column));
            match snapshot_owners(&owners, entity, &fields) {
                Ok(found) => {
                    prop_assert!(listed);
                    prop_assert_eq!(found, of_entity);
                }
                Err(FileOwnerError::NotASnapshotField { entity: refused, column }) => {
                    prop_assert!(!listed);
                    prop_assert_eq!(refused, entity);
                    let first = of_entity.iter().find(|owner| !fields.contains(&owner.column));
                    prop_assert_eq!(Some(column), first.map(|owner| owner.column));
                }
                Err(other) => prop_assert!(false, "unexpected {other:?}"),
            }
        }

        #[test]
        fn a_path_has_an_owner_exactly_when_its_folder_is_registered(
            registered in vec(select(EntityImageFolder::ALL.to_vec()), 0..4),
            folder in select(EntityImageFolder::ALL.to_vec()),
            name in "[0-9a-f]{32}\\.jpg",
        ) {
            let owners: Vec<FileOwner> = registered
                .iter()
                .enumerate()
                .map(|(n, &folder)| FileOwner {
                    entity: "probes",
                    column: ["c0", "c1", "c2", "c3"][n],
                    folder,
                })
                .collect();
            let path = OwnedPath::parse(&format!("{}/{name}", folder.path())).expect("a path");
            prop_assert_eq!(
                require_owners(&owners, std::slice::from_ref(&path)).is_ok(),
                registered.contains(&folder)
            );
            prop_assert_eq!(owners_of(&owners, folder).len(), registered.iter().filter(|f| **f == folder).count());
        }
    }
}
