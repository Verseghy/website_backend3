//! Reads the resolution file and validates it, before the run takes the
//! task lock (data-migration.md §4.1-§4.3, §8.2).
//!
//! The parser and the schema are pure (`domain::import_mysql::resolutions`,
//! ADR 0008 rule 1); this module is the file-system half. It runs after
//! `paths::check` has shown that the `resolutions` argument names a file
//! the task can open, so a refusal here is about what the file holds: more
//! than `MAX_FILE_BYTES`, text that is not UTF-8, or a file that fails
//! validation, with every problem named by entry and field. Every refusal
//! names the argument and the path, like the path checks, so a `CronJob`'s
//! log tells the operator what to fix before anything was read from the
//! source or written anywhere.
//!
//! The file is read once: the list and its SHA-256 come from the same
//! bytes ([`LoadedResolutions`]), so the digest that a run records is that
//! of the list it validated.

use std::{fs::File, io::Read, path::Path};

use loco_rs::{Error, Result};

use crate::domain::import_mysql::resolutions::{self, FileDigest, MAX_FILE_BYTES, ResolutionFile};

/// A resolution file as [`load`] read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedResolutions {
    /// The list, parsed and validated.
    pub file: ResolutionFile,
    /// The SHA-256 of the bytes that were read and validated, which the
    /// import's `import` activity entries record (data-migration.md §5.4)
    /// and the run's start log line shows.
    pub digest: FileDigest,
}

/// Reads the resolution file at `path` and returns it parsed and validated,
/// with the digest of the bytes it read.
///
/// - `path`: the `resolutions` argument (`ImportOptions::resolutions`),
///   which `paths::check` has already checked.
///
/// It reads at most one byte more than [`MAX_FILE_BYTES`], so a wrong path
/// (a dump, say) is refused without being read into memory.
///
/// # Errors
///
/// An [`Error::Message`] naming the argument and the path: the file cannot
/// be opened or read, is larger than [`MAX_FILE_BYTES`], is not UTF-8, or
/// fails validation, in which case the message lists every problem.
pub fn load(path: &Path) -> Result<LoadedResolutions> {
    let refuse = |problem: String| {
        Error::Message(format!(
            "the argument resolutions is {}, {problem}",
            path.display()
        ))
    };
    let file =
        File::open(path).map_err(|error| refuse(format!("which cannot be opened ({error})")))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| refuse(format!("which cannot be read ({error})")))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_FILE_BYTES) {
        return Err(refuse(format!(
            "which is larger than {MAX_FILE_BYTES} bytes: give the editor-signed resolution file \
             (data-migration.md §8.2), which is far smaller"
        )));
    }
    let digest = FileDigest::of(&bytes);
    let text = String::from_utf8(bytes).map_err(|error| {
        refuse(format!(
            "which is not UTF-8 text (the first invalid byte is at offset {}): give the \
             editor-signed resolution file (data-migration.md §8.2), a UTF-8 TOML file",
            error.utf8_error().valid_up_to()
        ))
    })?;
    let file = resolutions::parse(&text).map_err(|errors| {
        refuse(format!(
            "which is not a valid resolution file (data-migration.md §8.2; its schema is \
             documented in src/domain/import_mysql/resolutions.rs): {errors}"
        ))
    })?;
    Ok(LoadedResolutions { file, digest })
}
