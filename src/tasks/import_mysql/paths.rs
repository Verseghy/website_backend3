//! The file-system checks of the import's path arguments, before the run
//! takes the task lock (data-migration.md §4.1).
//!
//! The arguments themselves are parsed by `domain::import_mysql::options`,
//! which cannot touch the file system (ADR 0008 rule 1); this module looks at
//! what they name. A refusal names the argument and the path, and says what
//! to give instead, so a `CronJob`'s log tells the operator what to fix before
//! anything was read or written.
//!
//! The checks run on the paths as given. A path that changes between the
//! check and its use (a directory filled by another process, for example)
//! is caught again when it is used: the report files are created with
//! `create_new`, so a run never overwrites a file it did not write.

use std::{fs, io, path::Path};

use loco_rs::{Error, Result};

use crate::domain::import_mysql::options::{
    DEFAULT_RESOLUTIONS, ImportOptions, check_report_outside_storage_root,
};

/// Checks the paths of `options`: the resolution file can be opened for
/// reading, `storage_root` is a directory that can be listed, and `report`
/// is an empty directory outside `storage_root`.
///
/// - `options`: the run's validated arguments.
///
/// # Errors
///
/// An [`Error::Message`] naming the first argument that fails, its path and
/// the operating system's reason.
pub fn check(options: &ImportOptions) -> Result<()> {
    check_resolutions(&options.resolutions)?;
    check_storage_root(&options.storage_root)?;
    check_report(&options.report)?;
    // Canonical forms, so that `..`, a trailing slash or a symbolic link
    // cannot hide a report directory inside storage_root. Both exist, which
    // the checks above showed.
    let report = canonical("report", &options.report)?;
    let storage_root = canonical("storage_root", &options.storage_root)?;
    check_report_outside_storage_root(&report, &storage_root).map_err(Error::msg)
}

/// The resolution file must be a file that this process can open.
fn check_resolutions(path: &Path) -> Result<()> {
    let refuse = |problem: String| {
        Error::Message(format!(
            "the argument resolutions is {}, {problem}: give the editor-signed resolution file \
             (data-migration.md §8.2); without the argument, the task reads \
             {DEFAULT_RESOLUTIONS}",
            path.display()
        ))
    };
    let metadata =
        fs::metadata(path).map_err(|error| refuse(format!("which cannot be read ({error})")))?;
    if !metadata.is_file() {
        return Err(refuse("which is not a file".to_owned()));
    }
    fs::File::open(path).map_err(|error| refuse(format!("which cannot be opened ({error})")))?;
    Ok(())
}

/// `storage_root` must be a directory that this process can list.
fn check_storage_root(path: &Path) -> Result<()> {
    let refuse = |problem: String| {
        Error::Message(format!(
            "the argument storage_root is {}, {problem}: give the directory that is served as \
             /storage, the uploads volume's public root (data-migration.md §4.1)",
            path.display()
        ))
    };
    let metadata =
        fs::metadata(path).map_err(|error| refuse(format!("which cannot be read ({error})")))?;
    if !metadata.is_dir() {
        return Err(refuse("which is not a directory".to_owned()));
    }
    fs::read_dir(path).map_err(|error| refuse(format!("which cannot be listed ({error})")))?;
    Ok(())
}

/// `report` must be an empty directory, so a run's report never mixes with
/// or overwrites another run's.
fn check_report(path: &Path) -> Result<()> {
    let refuse = |problem: String| {
        Error::Message(format!(
            "the argument report is {}, {problem}: create an empty directory for the run report \
             and give that (data-migration.md §11.3)",
            path.display()
        ))
    };
    let metadata =
        fs::metadata(path).map_err(|error| refuse(format!("which cannot be read ({error})")))?;
    if !metadata.is_dir() {
        return Err(refuse("which is not a directory".to_owned()));
    }
    let mut entries =
        fs::read_dir(path).map_err(|error| refuse(format!("which cannot be listed ({error})")))?;
    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) => Err(refuse(
            "which is not empty, so it may hold the report of another run".to_owned(),
        )),
        Some(Err(error)) => Err(refuse(format!("which cannot be listed ({error})"))),
    }
}

/// The canonical form of an existing path, or a message naming `argument`.
fn canonical(argument: &str, path: &Path) -> Result<std::path::PathBuf> {
    fs::canonicalize(path).map_err(|error: io::Error| {
        Error::Message(format!(
            "the argument {argument} is {}, whose canonical path cannot be resolved ({error})",
            path.display()
        ))
    })
}
