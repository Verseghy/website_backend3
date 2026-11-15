//! `rich_text_manifest`: writes the policy manifest of editor.md §3.9,
//! `policy.json` and `css-vectors.jsonl`, into the admin's source tree.
//!
//! The admin's paste filter, its `canonicalStyle` and its custom `TipTap`
//! attributes follow content-v1 in TypeScript. They import these two files,
//! which the task renders from the Rust policy
//! ([`crate::domain::rich_text::manifest`]), so the editor and the
//! sanitiser read one definition. Both files are committed under
//! `admin/src/rich-text/`. A change to the policy runs the task and commits
//! its output in the same PR; the Contract CI job runs it too and fails when
//! a committed copy differs from what it wrote.
//!
//! ```sh
//! cargo loco task rich_text_manifest                 # admin/src/rich-text/, from the repository root
//! cargo loco task rich_text_manifest dir:<directory> # another existing directory
//! ```
//!
//! The task is idempotent: it rewrites a file only when its contents
//! differ, and leaves an equal one in place, untouched. Each file is
//! written to a temporary file in the same directory, flushed to disk and
//! renamed over the old one ([`replace_file`]), so a reader never sees half
//! a file, and after a crash the file holds either its old or its new
//! contents. Like every task, it does its work inside [`with_task_lock`]
//! and exits 0 when another run holds the lock (ADR 0016 §4). The work
//! needs no table, but the lock keeps two runs from renaming over each
//! other's files.
//!
//! The manifest is always rendered from [`SanitizePolicy::content_v1`],
//! never from deployment settings: the committed copy has to be the same in
//! every environment.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use loco_rs::prelude::*;

use crate::{
    domain::rich_text::{
        SanitizePolicy,
        manifest::{ManifestFile, PolicyTables, manifest_files, output_dir},
    },
    models::task_lock::{TaskLockOutcome, with_task_lock},
};

/// The name the task registers and locks under.
pub const TASK_NAME: &str = "rich_text_manifest";

/// The task: `backend3-cli task rich_text_manifest [dir:<directory>]`.
pub struct RichTextManifest;

/// What the task did with one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    /// The file was missing or differed, and now holds the new contents.
    Written,
    /// The file already held exactly the new contents and was not touched.
    Unchanged,
}

impl FileOutcome {
    /// The word the task logs for the outcome.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Written => "written",
            Self::Unchanged => "unchanged",
        }
    }
}

#[async_trait]
impl Task for RichTextManifest {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: TASK_NAME.to_owned(),
            detail: "Writes policy.json and css-vectors.jsonl, the rich-text policy manifest the \
                     admin imports, into admin/src/rich-text/ or dir:<directory> (editor.md §3.9)"
                .to_owned(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        // Every argument is checked, and the files are rendered, before the
        // lock is taken: a run that cannot succeed fails without waiting.
        let dir = output_dir(&vars.cli).map_err(|error| refusal(&error.to_string()))?;
        let dir = Path::new(dir);
        check_output_dir(dir).await?;
        let files = manifest_files(&PolicyTables::CONTENT_V1, &SanitizePolicy::content_v1())
            .map_err(|error| refusal(&error.to_string()))?;

        let outcome =
            with_task_lock(&ctx.db, &self.task().name, || write_manifest(dir, &files)).await?;
        if let TaskLockOutcome::Ran(written) = outcome {
            for (name, outcome) in written {
                tracing::info!(
                    task = TASK_NAME,
                    file = %dir.join(name).display(),
                    outcome = outcome.as_str(),
                    "rich-text manifest file"
                );
            }
        }
        Ok(())
    }
}

/// Writes every file of `files` into `dir`, each only when its contents
/// differ from what the file holds, and returns what happened to each, in
/// the order of `files`.
///
/// - `dir`: an existing directory. Nothing else in it is touched, apart from
///   a temporary file per written file, which is renamed into place.
/// - `files`: the files to write, with names that contain no directory.
///
/// # Errors
///
/// An existing file cannot be read, or a new one cannot be written or
/// renamed into place. Files written before the failure keep their new
/// contents: they are complete, and the next run, which the error message
/// asks for, writes the rest.
pub async fn write_manifest(
    dir: &Path,
    files: &[ManifestFile],
) -> Result<Vec<(&'static str, FileOutcome)>> {
    let mut outcomes = Vec::with_capacity(files.len());
    for file in files {
        let outcome = write_file(dir, file).await?;
        outcomes.push((file.name, outcome));
    }
    Ok(outcomes)
}

/// Writes one file as [`write_manifest`] describes.
async fn write_file(dir: &Path, file: &ManifestFile) -> Result<FileOutcome> {
    let path = dir.join(file.name);
    match tokio::fs::read(&path).await {
        Ok(current) if current == file.contents.as_bytes() => return Ok(FileOutcome::Unchanged),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(io_failure("read", &path, &error)),
    }
    replace_file(
        &temporary_path(dir, file.name),
        &path,
        file.contents.as_bytes(),
    )
    .await?;
    Ok(FileOutcome::Written)
}

/// The temporary file that [`write_manifest`] writes `name` to before it
/// renames it into place: `.<name>.<process id>.tmp` in `dir`.
///
/// A sibling, so that the rename stays on one file system and replaces the
/// file in one step. The process id keeps two processes that write into the
/// same directory (from two databases, so under two different locks) off
/// each other's temporary file.
///
/// - `dir`: the output directory.
/// - `name`: the file name, without a directory.
#[must_use]
pub fn temporary_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(".{name}.{}.tmp", std::process::id()))
}

/// Writes `contents` to `temporary`, flushes it to disk and renames it over
/// `path`, so that a reader of `path` never sees half a file, and after a
/// crash `path` holds either its old or its new contents. When a step fails,
/// the temporary file is removed again.
///
/// - `temporary`: the temporary file, in the directory of `path`
///   ([`temporary_path`]). Whatever is there is replaced.
/// - `path`: the file to replace or create.
/// - `contents`: the new contents.
///
/// # Errors
///
/// The temporary file cannot be written or flushed (the message names it),
/// or it cannot be renamed over `path` (the message names `path`). Both ask
/// for another run.
pub async fn replace_file(temporary: &Path, path: &Path, contents: &[u8]) -> Result<()> {
    if let Err(error) = write_flushed(temporary, contents).await {
        remove_quietly(temporary).await;
        return Err(io_failure("write", temporary, &error));
    }
    if let Err(error) = tokio::fs::rename(temporary, path).await {
        remove_quietly(temporary).await;
        return Err(io_failure("replace", path, &error));
    }
    Ok(())
}

/// Writes `contents` to `path` and flushes the data to disk before the
/// rename, so that a crash cannot leave the new name pointing at a file
/// whose data never reached the disk. The file is opened again for the
/// flush, because writing to a `tokio::fs::File` directly needs tokio's
/// `io-util` feature, which backend3 itself does not enable.
async fn write_flushed(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    tokio::fs::write(path, contents).await?;
    tokio::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .await?
        .sync_all()
        .await
}

/// Refuses an output directory that does not exist or is not a directory,
/// with what to do instead. The task never creates it: a missing directory
/// usually means the task runs from the wrong working directory.
async fn check_output_dir(dir: &Path) -> Result<()> {
    match tokio::fs::metadata(dir).await {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(refusal(&format!(
            "`{}` is not a directory; pass dir:<directory>",
            dir.display()
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(refusal(&format!(
            "the output directory `{}` does not exist; run the task from the repository root, \
             or pass dir:<an existing directory>",
            dir.display()
        ))),
        Err(error) => Err(io_failure("open", dir, &error)),
    }
}

/// Removes a temporary file after a failed write. A failure to remove it is
/// only logged: the write's own error is the one to report.
async fn remove_quietly(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != ErrorKind::NotFound
    {
        tracing::warn!(task = TASK_NAME, file = %path.display(), %error, "could not remove the temporary file");
    }
}

fn refusal(problem: &str) -> Error {
    Error::Message(format!("task {TASK_NAME}: {problem}"))
}

fn io_failure(action: &str, path: &Path, error: &std::io::Error) -> Error {
    Error::Message(format!(
        "task {TASK_NAME}: could not {action} `{}`: {error}; fix it and run the task again",
        path.display()
    ))
}
