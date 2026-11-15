//! Owned files on the uploads volume: which files a record names, whether
//! anything still refers to one, and the removal of those nothing refers to
//! (ADR 0011 §6 "Files", ADR 0015 §3, ADR 0016 §3). A purge collects its
//! record's files with [`files_of_record`] before it deletes the record, and
//! the `DeleteOwnedFiles` worker (`src/workers/delete_owned_files.rs`) calls
//! [`delete_unreferenced`] after the purge has committed.
//!
//! A model module without an entity (ADR 0008 rule 5): it reads the tables
//! of every entity that owns files and the `revisions` table, and it removes
//! files, so no one Sea-ORM entity owns it. It takes the connection, the
//! media settings and the file-owner registry as arguments (rule 2).
//!
//! **What counts as a reference.** A file is referenced while any row of an
//! owner's table, live or trashed, stores its value in the owner's column,
//! or while any revision of that entity holds the value anywhere in its
//! snapshot. Trashed rows count because a restore brings their files back
//! (ADR 0011 §3), and revisions count because a revert writes an earlier
//! file back into the row (ADR 0011 §4). So a replaced or cleared image
//! stays as long as the revision that shows it, which is until its record is
//! purged (parity SHR-15, MEDIA-01). The snapshot is searched at every depth,
//! not in one named field, so a reference survives a snapshot shape change
//! that renamed or moved its field (ADR 0011 §4, "Schema changes"): an
//! upcaster turns old snapshots into the new shape only when they are read.
//! A false match would need another field of the same entity's history to
//! hold the exact stored value, a random name, and it would only keep the
//! file.
//!
//! **What a record names.** [`files_of_record`] reads a record's files from
//! its row's registered columns and from the member of the same name in
//! each of its revisions, so the entity's snapshot type lists every
//! registered column under its own name (`domain::owned_files::snapshot_owners`
//! refuses one that does not). A revision of an older snapshot shape is
//! upcast to the current one first, as a revert reads it (ADR 0011 §4,
//! "Schema changes"), so a renamed member is still found. A value that names
//! no file of the owner's folder, a revision that cannot be upcast or lacks
//! the member, and an array item that is not a string are each skipped with
//! a warning: the purge then leaves that file as an orphan, which ADR 0011
//! §6 counts as harmless, rather than fail every time on one bad revision.
//!
//! **Why checking, then removing, is safe.** The check and the removal are
//! two steps with no lock between them, and nothing needs one: no new
//! reference to an existing file can appear in between. An upload always
//! makes a new name (128 random bits, `media_rules::entity_image_name`), and
//! the only write that stores an existing name again is a revert, whose
//! revision refers to the name already, so the check has seen it.
//!
//! **What removal means.** Each folder below `media.root` is opened without
//! following a symbolic link, as the upload model does (ADR 0015 §6), and
//! the file is unlinked from its folder. A file that is not there any more
//! counts as removed, so a second run of the same job changes nothing and
//! does not fail (ADR 0016 §3, "Idempotent"). The removal is not flushed
//! with `fsync`: a crash that undoes it leaves an unreferenced file, which
//! ADR 0011 §6 counts as harmless, and the next run removes it.

use std::{io, os::fd::OwnedFd, path::PathBuf, sync::Arc};

use rustix::{
    fs::{self as at, AtFlags, Mode, OFlags},
    io::Errno,
};
use sea_orm::{ConnectionTrait, DbBackend, DbErr, Statement};
use serde_json::Value;

use crate::{
    domain::{
        owned_files::{
            FileOwner, FileOwnerError, NoOwner, OwnedPath, check_file_owners, column_values,
            owners_of, require_owners, snapshot_owners,
        },
        revision::{
            RevisionSnapshot, SnapshotObject, SnapshotTypeError, check_snapshot_type,
            comparable_snapshot,
        },
    },
    settings::MediaSettings,
};

/// The full key of `MediaSettings::root`, for messages.
const ROOT_KEY: &str = "settings.media.root";

/// The jsonpath that finds a string anywhere in a snapshot: every item at
/// every depth (`.**`) that equals the variable `v`. `strict`, because lax
/// mode would also unwrap arrays on the way and visit items twice; items of
/// another type than a string compare as unknown and never match.
const SNAPSHOT_SEARCH: &str = "strict $.** ? (@ == $v)";

/// Why [`delete_unreferenced`], [`is_referenced`] or [`files_of_record`]
/// did nothing.
#[derive(Debug, thiserror::Error)]
pub enum OwnedFilesError {
    /// The registry breaks a rule of `check_file_owners`, whose names would
    /// be written into SQL, or names a column that the entity's snapshot
    /// type lacks (`snapshot_owners`); nothing is queried.
    #[error(transparent)]
    Owners(#[from] FileOwnerError),
    /// The revision snapshot type of [`files_of_record`] breaks a rule of
    /// `check_snapshot_type`; nothing is queried.
    #[error(transparent)]
    Snapshot(#[from] SnapshotTypeError),
    /// A path's folder has no owner in the registry.
    #[error(transparent)]
    NoOwner(#[from] NoOwner),
    /// The media root cannot be opened: the volume is not mounted, or the
    /// setting is wrong.
    #[error("{action} ({ROOT_KEY}: {}): {source}", .path.display())]
    Root {
        /// What failed, and what to do about it.
        action: &'static str,
        /// The configured root.
        path: PathBuf,
        /// The operating system's error.
        source: io::Error,
    },
    /// The reference query failed: a table or column that the registry
    /// names and the schema lacks, or a lost connection.
    #[error("cannot read whether anything refers to the file: {0}")]
    Database(#[from] DbErr),
    /// The work on a blocking thread did not finish: it panicked.
    #[error("the file-system work of the job did not finish: {0}")]
    Blocking(String),
}

/// What [`delete_unreferenced`] did with each path, by outcome, each list in
/// the order of the paths it was given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeletionReport {
    /// Unreferenced, and removed by this run.
    pub deleted: Vec<String>,
    /// Still referenced by a row or a revision, so left in place.
    pub kept: Vec<String>,
    /// Unreferenced, and not there any more: removed by an earlier run, or
    /// never written.
    pub absent: Vec<String>,
    /// Neither checked nor removed, with the reason: a database error, or a
    /// file-system error such as a folder that is a symbolic link.
    pub failed: Vec<PathFailure>,
}

impl DeletionReport {
    /// Whether every path ended without a failure, so that no unreferenced
    /// file named by the job is left on the volume.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.failed.is_empty()
    }
}

/// A path that [`delete_unreferenced`] could not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathFailure {
    /// The path below `media.root`.
    pub path: String,
    /// What failed, for the operator.
    pub reason: String,
}

/// Whether a row of an owner of `path`'s folder, live or trashed, or a
/// revision of such an owner's entity refers to the file (the module
/// documentation, "What counts as a reference").
///
/// - `db`: a connection to the application database, or a transaction.
/// - `owners`: the file-owner registry (`domain::owned_files::FILE_OWNERS`,
///   or a test's own).
/// - `path`: the file.
///
/// # Errors
///
/// - [`OwnedFilesError::Owners`] for a registry that breaks
///   `check_file_owners`, and [`OwnedFilesError::NoOwner`] when no owner has
///   the path's folder; nothing is queried then;
/// - [`OwnedFilesError::Database`] when the query fails: a table or column
///   that the registry names and the schema lacks, or a lost connection.
pub async fn is_referenced<C: ConnectionTrait>(
    db: &C,
    owners: &[FileOwner],
    path: &OwnedPath,
) -> Result<bool, OwnedFilesError> {
    check_file_owners(owners)?;
    require_owners(owners, std::slice::from_ref(path))?;
    Ok(references(db, owners, path).await?)
}

/// The query of [`is_referenced`], with the registry checked already.
///
/// - `db`, `owners`, `path`: as [`is_referenced`] takes them.
async fn references<C: ConnectionTrait>(
    db: &C,
    owners: &[FileOwner],
    path: &OwnedPath,
) -> Result<bool, DbErr> {
    for owner in owners_of(owners, path.folder()) {
        // The names were checked to be `snake_case` (`check_file_owners`),
        // so they hold no quote and cannot carry SQL; quoting them keeps a
        // name that is also a keyword usable. `to_jsonb` reads a text
        // column as one JSON string and leaves a `jsonb` array as it is, and
        // `@>` matches a string by equality and an array by holding it as
        // an element, so one query serves both shapes of FileOwner::column.
        // Neither condition filters on `deleted_at`: trashed rows count.
        let sql = format!(
            "SELECT EXISTS (SELECT 1 FROM \"{entity}\" \
                 WHERE to_jsonb(\"{column}\") @> to_jsonb($1::text)) \
             OR EXISTS (SELECT 1 FROM revisions WHERE entity_type = $2 \
                 AND jsonb_path_exists(snapshot, '{SNAPSHOT_SEARCH}', \
                     jsonb_build_object('v', $1::text))) \
             AS referenced",
            entity = owner.entity,
            column = owner.column,
        );
        let statement = Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [path.stored_value().into(), owner.entity.into()],
        );
        let row = db
            .query_one_raw(statement)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound("the reference query returned no row".into()))?;
        if row.try_get::<bool>("", "referenced")? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The owned files that record `entity_id` of the entity of `S` names: the
/// values of the entity's registered columns in its row, live or trashed,
/// and in every one of its revisions, each file once; per registered
/// column, in the registry's order, the row's first and then the
/// revisions' in their order. A purge calls it before it deletes the row
/// and the revisions, and passes the paths to `commit_then_enqueue`: an
/// image replaced long ago is named only by an old revision, and nothing
/// would name it once the purge has run (recipe 2 step 7 of the project
/// skill).
///
/// - `S`: the entity's revision snapshot type. Its
///   `RevisionSnapshot::ENTITY_TYPE` is the record's table, which has an
///   `id` primary key (ADR 0009), and its `FIELDS` list every registered
///   column of that table under the column's name.
/// - `db`: the purge's transaction, or another connection.
/// - `owners`: the file-owner registry; only the owners whose
///   [`FileOwner::entity`] is the entity of `S` are read.
/// - `entity_id`: the record's id.
///
/// A revision's values are read from the member named like the column,
/// after an older snapshot is upcast to the current shape (the module
/// documentation, "What a record names"); a newer one, written by a release
/// this one was rolled back from, is read as stored. What names no file is
/// skipped with a warning, never an error.
///
/// # Errors
///
/// Before anything is read: [`OwnedFilesError::Owners`] for a registry that
/// breaks `check_file_owners` or a registered column that `S::FIELDS` does
/// not list, and [`OwnedFilesError::Snapshot`] for a snapshot type that
/// breaks `check_snapshot_type`. [`OwnedFilesError::Database`] when a query
/// fails.
pub async fn files_of_record<S: RevisionSnapshot>(
    db: &impl ConnectionTrait,
    owners: &[FileOwner],
    entity_id: i64,
) -> Result<Vec<OwnedPath>, OwnedFilesError> {
    check_file_owners(owners)?;
    check_snapshot_type::<S>()?;
    let owners = snapshot_owners(owners, S::ENTITY_TYPE, S::FIELDS)?;
    if owners.is_empty() {
        return Ok(Vec::new());
    }
    let row = row_columns(db, &owners, entity_id).await?;
    let revisions = revision_snapshots::<S>(db, &owners, entity_id).await?;
    let mut files: Vec<OwnedPath> = Vec::new();
    for owner in &owners {
        if let Some(row) = &row {
            // `row_columns` builds the object with every owner's column.
            let value = row.get(owner.column).unwrap_or(&Value::Null);
            collect_files(&mut files, owner, entity_id, None, value);
        }
        for (revision_no, snapshot) in &revisions {
            if let Some(value) = snapshot.get(owner.column) {
                collect_files(&mut files, owner, entity_id, Some(*revision_no), value);
            } else {
                tracing::warn!(
                    entity = owner.entity,
                    entity_id,
                    revision_no = *revision_no,
                    column = owner.column,
                    "a revision has no member for an owned column; the files it showed there, \
                     if any, are left as orphans"
                );
            }
        }
    }
    Ok(files)
}

/// The registered columns of record `entity_id`'s row as one JSON object,
/// each column as PostgreSQL's `to_jsonb` gives it (a text column as a
/// string, a `jsonb` array as it is, SQL `NULL` as `null`), or `None` when
/// there is no such row.
///
/// - `db`, `entity_id`: as [`files_of_record`] takes them.
/// - `owners`: the owners of one entity, checked by `check_file_owners`.
async fn row_columns(
    db: &impl ConnectionTrait,
    owners: &[FileOwner],
    entity_id: i64,
) -> Result<Option<serde_json::Map<String, Value>>, DbErr> {
    let Some(entity) = owners.first().map(|owner| owner.entity) else {
        return Ok(None);
    };
    // The names were checked to be `snake_case` (`check_file_owners`), so
    // they hold no quote and cannot carry SQL, as a quoted identifier or as
    // a string literal.
    let members: Vec<String> = owners
        .iter()
        .map(|owner| {
            format!(
                "'{column}', to_jsonb(e.\"{column}\")",
                column = owner.column
            )
        })
        .collect();
    let sql = format!(
        "SELECT jsonb_build_object({members}) AS owned FROM \"{entity}\" AS e WHERE e.id = $1",
        members = members.join(", "),
    );
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [entity_id.into()],
        ))
        .await?;
    row.map(|row| match row.try_get::<Value>("", "owned")? {
        Value::Object(object) => Ok(object),
        other => Err(DbErr::Type(format!(
            "jsonb_build_object returned {other}, not a JSON object"
        ))),
    })
    .transpose()
}

/// The revisions of record `entity_id`, in their order, as `(revision_no,
/// snapshot)` with each snapshot in the current shape of `S`. A revision
/// that cannot be read so is left out with a warning.
///
/// - `S`, `db`, `entity_id`: as [`files_of_record`] takes them.
/// - `owners`: the owners of the entity of `S`, checked by
///   `check_file_owners`.
async fn revision_snapshots<S: RevisionSnapshot>(
    db: &impl ConnectionTrait,
    owners: &[FileOwner],
    entity_id: i64,
) -> Result<Vec<(i32, SnapshotObject)>, DbErr> {
    // A snapshot of the current version is cut down to the owned members
    // in the query, so a post's history does not bring every body along; a
    // member it lacks stays absent rather than `null`, so the warning above
    // can tell. An older one is read whole, because an upcast step reads
    // whichever members it needs. The column names are checked
    // `snake_case`, so they cannot carry SQL as string literals.
    let names: Vec<String> = owners
        .iter()
        .map(|owner| format!("'{}'", owner.column))
        .collect();
    let sql = format!(
        "SELECT r.revision_no, r.schema_version, \
             CASE WHEN r.schema_version = $3 THEN \
                 (SELECT COALESCE(jsonb_object_agg(m.key, m.value), '{{}}'::jsonb) \
                  FROM jsonb_each(r.snapshot) AS m WHERE m.key IN ({names})) \
             ELSE r.snapshot END AS snapshot \
         FROM revisions AS r WHERE r.entity_type = $1 AND r.entity_id = $2 \
         ORDER BY r.revision_no",
        names = names.join(", "),
    );
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [
                S::ENTITY_TYPE.into(),
                entity_id.into(),
                S::SCHEMA_VERSION.into(),
            ],
        ))
        .await?;
    let mut snapshots = Vec::with_capacity(rows.len());
    for row in rows {
        let revision_no: i32 = row.try_get("", "revision_no")?;
        let version: i32 = row.try_get("", "schema_version")?;
        let snapshot: Value = row.try_get("", "snapshot")?;
        // `comparable_snapshot` upcasts an older version, keeps the current
        // one, and reads a newer one, which no step converts down, as
        // stored: what the purge needs, as the save's comparison does.
        match comparable_snapshot::<S>(version, snapshot) {
            Ok(object) => snapshots.push((revision_no, object)),
            Err(error) => tracing::warn!(
                entity = S::ENTITY_TYPE,
                entity_id,
                revision_no,
                schema_version = version,
                error = %error,
                "a revision cannot be read in the current snapshot shape; the files it showed, \
                 if any, are left as orphans"
            ),
        }
    }
    Ok(snapshots)
}

/// Adds the files that one column value names to `files`, each once, and
/// warns about every part of it that names none.
///
/// - `files`: the files found so far, in order.
/// - `owner`: the column the value is of.
/// - `entity_id`: the record, for the warning.
/// - `revision_no`: the revision the value is of, or `None` for the row.
/// - `json`: the value, as [`column_values`] reads it.
fn collect_files(
    files: &mut Vec<OwnedPath>,
    owner: &FileOwner,
    entity_id: i64,
    revision_no: Option<i32>,
    json: &Value,
) {
    let found = column_values(json);
    for item in found.unusable {
        tracing::warn!(
            entity = owner.entity,
            entity_id,
            revision_no,
            column = owner.column,
            item = %item,
            "an owned column holds an item that is not a stored value; the file it meant, if \
             any, is left as an orphan"
        );
    }
    for value in found.values {
        match OwnedPath::from_stored_value(owner.folder, value) {
            Ok(path) if !files.contains(&path) => files.push(path),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                entity = owner.entity,
                entity_id,
                revision_no,
                column = owner.column,
                value = %value,
                error = %error,
                "a stored value of an owned file is no path of its folder; the file, if any, \
                 is left as an orphan"
            ),
        }
    }
}

/// Removes each file of `paths` that nothing refers to any more, and leaves
/// every other file as it is.
///
/// - `db`: a connection to the application database.
/// - `media`: the media settings; only `root` is used.
/// - `owners`: the file-owner registry.
/// - `paths`: the job's checked paths (`domain::owned_files::check_job_paths`).
///
/// Every path is tried, also after another failed, so one bad path does not
/// keep the others on the volume; the report lists each path's outcome, and
/// the failures with their reasons.
///
/// # Errors
///
/// Before anything is read or removed: [`OwnedFilesError::Owners`] for a
/// registry that breaks `check_file_owners`, [`OwnedFilesError::NoOwner`]
/// for a path whose folder has no owner, and [`OwnedFilesError::Root`] when
/// `media.root` cannot be opened (the volume is not mounted);
/// [`OwnedFilesError::Blocking`] when a blocking thread panicked.
pub async fn delete_unreferenced<C: ConnectionTrait>(
    db: &C,
    media: &MediaSettings,
    owners: &[FileOwner],
    paths: &[OwnedPath],
) -> Result<DeletionReport, OwnedFilesError> {
    check_file_owners(owners)?;
    require_owners(owners, paths)?;
    let mut report = DeletionReport::default();
    if paths.is_empty() {
        return Ok(report);
    }
    let root_path = media.root.clone();
    let root = blocking(move || open_root(&root_path)).await??;
    let root = Arc::new(root);
    for path in paths {
        match references(db, owners, path).await {
            Ok(true) => report.kept.push(path.path().to_owned()),
            Ok(false) => {
                let fd = Arc::clone(&root);
                let target = path.clone();
                match blocking(move || remove_file(&fd, &target)).await? {
                    Ok(Removal::Removed) => report.deleted.push(path.path().to_owned()),
                    Ok(Removal::Absent) => report.absent.push(path.path().to_owned()),
                    Err(reason) => report.failed.push(PathFailure {
                        path: path.path().to_owned(),
                        reason,
                    }),
                }
            }
            Err(error) => report.failed.push(PathFailure {
                path: path.path().to_owned(),
                reason: format!("cannot read whether anything refers to the file: {error}"),
            }),
        }
    }
    Ok(report)
}

/// What [`remove_file`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Removal {
    /// The file was there and is gone now.
    Removed,
    /// The file, or its folder, was not there.
    Absent,
}

/// Opens the media root, which must exist: without it the volume is not
/// mounted, and a missing file would say nothing about the volume's files.
///
/// - `root`: `media.root`.
fn open_root(root: &std::path::Path) -> Result<OwnedFd, OwnedFilesError> {
    at::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| OwnedFilesError::Root {
        action: if errno == Errno::NOENT {
            "the media root does not exist; mount the media volume there before the job is \
             retried"
        } else {
            "cannot open the media root"
        },
        path: root.to_owned(),
        source: errno.into(),
    })
}

/// Unlinks one file below the root, opening each folder on the way without
/// following a symbolic link.
///
/// - `root`: the open media root.
/// - `path`: the file.
///
/// # Errors
///
/// What failed, for the operator, with the operating system's error where
/// there is one: a folder that is a symbolic link or not a directory, a
/// folder that cannot be opened, a name that is a directory, or a file that
/// cannot be removed.
fn remove_file(root: &OwnedFd, path: &OwnedPath) -> Result<Removal, String> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut folder: Option<OwnedFd> = None;
    for segment in path.folder().segments() {
        let parent = folder.as_ref().unwrap_or(root);
        folder = Some(match at::openat(parent, *segment, flags, Mode::empty()) {
            Ok(opened) => opened,
            // A folder that does not exist yet, such as colleagues_images/
            // before the first colleague photo, holds no file either.
            Err(Errno::NOENT) => return Ok(Removal::Absent),
            Err(Errno::LOOP | Errno::NOTDIR) => {
                return Err(format!(
                    "the entity image folder {}/ is a symbolic link or not a directory; owned \
                     files are never removed through a symbolic link",
                    path.folder().path()
                ));
            }
            Err(errno) => {
                return Err(format!(
                    "cannot open the entity image folder {}/: {}",
                    path.folder().path(),
                    io::Error::from(errno)
                ));
            }
        });
    }
    let folder = folder.ok_or_else(|| "an entity image folder has no path".to_owned())?;
    match at::unlinkat(&folder, path.name(), AtFlags::empty()) {
        Ok(()) => Ok(Removal::Removed),
        Err(Errno::NOENT) => Ok(Removal::Absent),
        // Linux, where backend3 runs, answers EISDIR for a directory
        // (unlink(2)); its EPERM means something else (`removal_failure`).
        Err(Errno::ISDIR) => {
            Err("the path names a directory, not a file; a directory is never removed".to_owned())
        }
        Err(errno) => Err(removal_failure(errno)),
    }
}

/// The reason an `unlinkat` failed with `errno`: the operating system's
/// error, and, for the errors an operator can act on, what to look at.
///
/// - `errno`: the error, never `NOENT` or `ISDIR`, which `remove_file`
///   answers itself.
fn removal_failure(errno: Errno) -> String {
    // unlink(2) on Linux: EPERM is a file marked immutable or append-only, a
    // sticky folder whose file another account owns, or a file system that
    // forbids unlinking; EACCES is a folder the process may not write to.
    let hint = match errno {
        Errno::PERM => {
            "; the file may be immutable or append-only, its folder sticky with the file owned by \
             another account, or its file system one that does not remove files"
        }
        Errno::ACCESS => "; the process may not write to the file's folder",
        Errno::ROFS => "; the media volume is mounted read-only",
        _ => "",
    };
    format!("cannot remove the file: {}{hint}", io::Error::from(errno))
}

/// Runs `work` on the runtime's pool for blocking calls, so the file-system
/// calls do not stall the async worker; the job awaits it, so it is not the
/// background `tokio::spawn` that ADR 0016 rules out.
///
/// - `work`: the blocking calls.
///
/// # Errors
///
/// [`OwnedFilesError::Blocking`] when `work` panicked.
async fn blocking<T, F>(work: F) -> Result<T, OwnedFilesError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| OwnedFilesError::Blocking(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_complete_without_failures() {
        let mut report = DeletionReport {
            deleted: vec!["authors_images/a.jpg".to_owned()],
            kept: vec!["authors_images/b.jpg".to_owned()],
            absent: vec!["authors_images/c.jpg".to_owned()],
            failed: Vec::new(),
        };
        assert!(report.is_complete());
        report.failed.push(PathFailure {
            path: "authors_images/d.jpg".to_owned(),
            reason: "cannot remove the file".to_owned(),
        });
        assert!(!report.is_complete());
    }

    #[test]
    fn a_failed_removal_keeps_the_systems_error_and_says_where_to_look() {
        let refused = removal_failure(Errno::PERM);
        assert!(refused.contains("Operation not permitted"), "{refused}");
        assert!(refused.contains("immutable or append-only"), "{refused}");
        assert!(!refused.contains("directory"), "{refused}");
        let denied = removal_failure(Errno::ACCESS);
        assert!(denied.contains("Permission denied"), "{denied}");
        assert!(denied.contains("may not write"), "{denied}");
        let read_only = removal_failure(Errno::ROFS);
        assert!(read_only.contains("mounted read-only"), "{read_only}");
        let other = removal_failure(Errno::IO);
        assert!(other.starts_with("cannot remove the file: "), "{other}");
        assert!(other.contains("os error"), "{other}");
    }

    #[test]
    fn a_missing_root_says_to_mount_the_volume() {
        let error = open_root(std::path::Path::new("/nonexistent-backend3-media-root"))
            .expect_err("no such directory");
        let text = error.to_string();
        assert!(text.contains("settings.media.root"), "{text}");
        assert!(text.contains("mount the media volume"), "{text}");
    }
}
