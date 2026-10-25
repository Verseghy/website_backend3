//! The fixture tree that the media contract suite expects under `/storage`.
//!
//! One table drives both sides: [`seed`] writes the tree into an empty
//! directory, which then stands in for the volume's `app/public/`, and the
//! suite derives its requests and expected answers from the same entries.
//! The names copy the shapes of the real volume (ADR 0015, "What is on the
//! volume"): 32-hex entity image names, human names with spaces, accents,
//! `(1)` and `[PR]`, upper-case extensions, dot-files and an `index.html`
//! inside a folder, plus symbolic links that must not be followed. Every name
//! is invented. The bodies are synthetic, because both servers type a file by
//! its extension and never read it.
//!
//! The tree is generated rather than committed: a committed `.gitignore`
//! fixture would act on the repository itself.

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    time::{Duration, SystemTime},
};

/// One file the server must serve.
#[derive(Clone, Copy, Debug)]
pub struct Fixture {
    /// The path below the storage root, as the volume stores it (UTF-8,
    /// precomposed accents).
    pub path: &'static str,
    /// The media type the server must send for its extension.
    pub content_type: &'static str,
    /// The size of its synthetic body, in bytes.
    pub size: usize,
}

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

/// Every file the server must serve with `200`.
pub const FILES: &[Fixture] = &[
    file(
        "posts_images/index/0f1e2d3c4b5a69788796a5b4c3d2e1f0.jpg",
        "image/jpeg",
        300,
    ),
    file(
        "posts_images/a1b2c3d4e5f60718293a4b5c6d7e8f90.JPG",
        "image/jpeg",
        300,
    ),
    file(
        "posts_images/b2c3d4e5f60718293a4b5c6d7e8f90a1.jfif",
        "image/jpeg",
        300,
    ),
    file(
        "posts_images/c2d3e4f5061728394a5b6c7d8e9fa0b1.jpeg",
        "image/jpeg",
        300,
    ),
    file(
        "posts_images/c3d4e5f60718293a4b5c6d7e8f90a1b2.png",
        "image/png",
        300,
    ),
    file(
        "posts_images/d4e5f60718293a4b5c6d7e8f90a1b2c3.webp",
        "image/webp",
        300,
    ),
    file(
        "posts_images/e5f60718293a4b5c6d7e8f90a1b2c3d4.pdf",
        "application/pdf",
        300,
    ),
    file(
        "posts_images/f60718293a4b5c6d7e8f90a1b2c3d4e5.mp3",
        "audio/mpeg",
        300,
    ),
    file(
        "authors_images/0718293a4b5c6d7e8f90a1b2c3d4e5f6.png",
        "image/png",
        300,
    ),
    file(
        "colleagues_images/18293a4b5c6d7e8f90a1b2c3d4e5f607.jpg",
        "image/jpeg",
        300,
    ),
    file(
        "uploads/Pictures/Árvíztűrő tükörfúrógép.JPG",
        "image/jpeg",
        300,
    ),
    file(
        "uploads/docs/Felvételi tájékoztató (1) [PR].pdf",
        "application/pdf",
        300,
    ),
    file("uploads/docs/Órarend.PDF", "application/pdf", 300),
    DIAKTOLL,
    file("uploads/docs/beosztás.docx", DOCX, 300),
    file(
        "uploads/docs/jelentkezési lap.doc",
        "application/msword",
        300,
    ),
    file("uploads/posts/12/kép.gif", "image/gif", 300),
    file("uploads/posts/12/rajz.svg", "image/svg+xml", 300),
    file("uploads/pages/3/index.html", "text/html", 300),
    file("uploads/docs/adatok.xml", "text/xml", 300),
    file("uploads/docs/jegyzet.txt", "text/plain", 300),
    file(
        "uploads/docs/ismeretlen.qqq",
        "application/octet-stream",
        300,
    ),
    file("sitemap.xml", "text/xml", 300),
    file("uploads/docs/sitemap.xml", "text/xml", 300),
];

/// The PDF that the Diáktoll viewer loads cross-origin with range requests
/// (ADR 0015, "The URL contract"); large enough for several ranges.
pub const DIAKTOLL: Fixture = file(
    "uploads/diaktoll/ujsag_final_2021osz.pdf",
    "application/pdf",
    4096,
);

/// A file whose type is a download, for the ranged `attachment` checks.
pub const DOWNLOAD: Fixture = file("uploads/docs/beosztás.docx", DOCX, 300);

/// An entity image, for the conditional-request checks.
pub const ENTITY_IMAGE: Fixture = FILES[0];

/// Dot-files and a file in a dot-directory: on the volume, never served.
pub const HIDDEN: &[&str] = &[
    ".gitignore",
    "uploads/.htaccess",
    "uploads/.hidden/titkos.pdf",
];

/// Paths that are not files, as request paths below `/storage/` (a trailing
/// slash is part of the request). `uploads/pages/3/` holds an `index.html`,
/// which a directory request must not serve.
pub const DIRECTORIES: &[&str] = &[
    "",
    "uploads",
    "uploads/",
    "posts_images/index/",
    "uploads/pages/3/",
];

/// Names that differ from a stored file only in case or in Unicode
/// normalisation, plus one that is simply absent: the mapping is byte for
/// byte (ADR 0015 §1, Path mapping).
pub const MISSING: &[&str] = &[
    "uploads/docs/nincs.pdf",
    "uploads/Pictures/Árvíztűrő tükörfúrógép.jpg",
    "posts_images/A1B2C3D4E5F60718293A4B5C6D7E8F90.JPG",
    // "Órarend.PDF" with the accent as a combining mark (NFD).
    "uploads/docs/O\u{301}rarend.PDF",
];

/// Symbolic links that [`seed`] creates, as (the link's path below the
/// storage root, its target). The public media hold none, and the contract
/// serves regular files only, so none of them is followed (the readings in
/// `mod.rs`).
pub const LINKS: &[(&str, &str)] = &[
    // To a fixture in the same folder.
    ("uploads/docs/hivatkozas.pdf", "Órarend.PDF"),
    // To a file outside the storage root that every Linux system has.
    ("uploads/docs/rendszer.pdf", "/etc/os-release"),
    // To a folder of the tree.
    ("uploads/mappa", "docs"),
];

/// Request paths below `/storage/` that reach a file only through one of
/// [`LINKS`].
pub const THROUGH_LINKS: &[&str] = &[
    "uploads/docs/hivatkozas.pdf",
    "uploads/docs/rendszer.pdf",
    "uploads/mappa/Órarend.PDF",
];

/// The modification time that [`seed`] gives every fixture.
pub const MTIME: Duration = Duration::from_hours(496_736); // 1 788 249 600 s

/// [`MTIME`] as an HTTP date, the exact `Last-Modified` value.
pub const LAST_MODIFIED: &str = "Tue, 01 Sep 2026 08:00:00 GMT";

/// An HTTP date before [`MTIME`].
pub const BEFORE: &str = "Mon, 31 Aug 2026 08:00:00 GMT";

/// An HTTP date after [`MTIME`].
pub const AFTER: &str = "Wed, 02 Sep 2026 08:00:00 GMT";

const fn file(path: &'static str, content_type: &'static str, size: usize) -> Fixture {
    Fixture {
        path,
        content_type,
        size,
    }
}

impl Fixture {
    /// The file's synthetic body: its path and a newline, repeated and cut
    /// to [`Fixture::size`] bytes, so every file and every range of it is
    /// distinct.
    pub fn body(&self) -> Vec<u8> {
        body(self.path, self.size)
    }
}

fn body(path: &str, size: usize) -> Vec<u8> {
    let line = format!("{path}\n");
    line.bytes().cycle().take(size).collect()
}

/// Writes the fixture tree into `root`, which must be an empty directory,
/// with world-readable modes and every file's modification time set to
/// [`MTIME`], and then the symbolic links of [`LINKS`].
///
/// The modes let a container read the tree whatever UID it runs as, as the
/// injected `fsGroup` does on the cluster (ADR 0015 §3). The links are
/// written last, so the folders they sit in exist; their own mode is never
/// set, because changing a link's mode changes its target's.
///
/// - `root`: the directory that becomes the storage root.
///
/// # Errors
///
/// When `root` is missing, not empty, or a write fails; the message names
/// the path.
pub fn seed(root: &Path) -> Result<(), String> {
    let mut entries = fs::read_dir(root)
        .map_err(|error| format!("{}: not a readable directory: {error}", root.display()))?;
    if entries.next().is_some() {
        return Err(format!(
            "{}: not empty; seed writes the fixture tree only into an empty directory",
            root.display()
        ));
    }
    set_mode(root, 0o755)?;
    let hidden = HIDDEN.iter().map(|path| (*path, 64));
    let files = FILES.iter().map(|fixture| (fixture.path, fixture.size));
    for (path, size) in files.chain(hidden) {
        write(root, path, &body(path, size))?;
    }
    for (path, target) in LINKS {
        let link = root.join(path);
        symlink(target, &link)
            .map_err(|error| format!("{}: symlink to {target}: {error}", link.display()))?;
    }
    Ok(())
}

fn write(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), String> {
    let path = root.join(relative);
    let error = |error: io::Error| format!("{}: {error}", path.display());
    let mut directory = root.to_path_buf();
    if let Some(parent) = Path::new(relative).parent() {
        for component in parent.components() {
            directory.push(component);
            if !directory.is_dir() {
                fs::create_dir(&directory).map_err(error)?;
                set_mode(&directory, 0o755)?;
            }
        }
    }
    let mut file = fs::File::create_new(&path).map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.set_modified(SystemTime::UNIX_EPOCH + MTIME)
        .map_err(error)?;
    drop(file);
    set_mode(&path, 0o644)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("{}: chmod {mode:o}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// U+0300 to U+036F, the combining diacritical marks of NFD.
    fn has_combining_mark(text: &str) -> bool {
        text.chars().any(|c| ('\u{300}'..='\u{36f}').contains(&c))
    }

    #[test]
    fn stored_names_are_precomposed_and_the_nfd_probe_is_not() {
        for fixture in FILES {
            assert!(!has_combining_mark(fixture.path), "{}", fixture.path);
        }
        assert!(MISSING.iter().any(|path| has_combining_mark(path)));
    }

    #[test]
    fn missing_names_are_not_files_and_paths_are_unique() {
        let files: HashSet<&str> = FILES.iter().map(|fixture| fixture.path).collect();
        assert_eq!(files.len(), FILES.len(), "duplicate fixture path");
        let links = LINKS.iter().map(|(path, _)| path);
        for path in MISSING
            .iter()
            .chain(HIDDEN)
            .chain(THROUGH_LINKS)
            .chain(links)
        {
            assert!(!files.contains(path), "{path}");
        }
        assert!(files.contains(DIAKTOLL.path) && files.contains(DOWNLOAD.path));
    }

    #[test]
    fn bodies_have_the_size_and_differ_per_file() {
        let bodies: HashSet<Vec<u8>> = FILES.iter().map(Fixture::body).collect();
        assert_eq!(bodies.len(), FILES.len());
        assert_eq!(DIAKTOLL.body().len(), 4096);
        assert!(
            DIAKTOLL
                .body()
                .starts_with(b"uploads/diaktoll/ujsag_final_2021osz.pdf\n")
        );
    }

    #[test]
    fn seed_writes_the_tree_with_modes_and_mtime() {
        let root = scratch("seed");
        seed(&root).expect("seed");
        for fixture in FILES {
            let path = root.join(fixture.path);
            assert_eq!(
                fs::read(&path).expect("read"),
                fixture.body(),
                "{}",
                fixture.path
            );
            let metadata = fs::metadata(&path).expect("metadata");
            assert_eq!(metadata.permissions().mode() & 0o777, 0o644);
            assert_eq!(
                metadata.modified().expect("mtime"),
                SystemTime::UNIX_EPOCH + MTIME
            );
        }
        for path in HIDDEN {
            assert!(root.join(path).is_file(), "{path}");
        }
        for (path, target) in LINKS {
            let link = root.join(path);
            let kind = fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type();
            assert!(kind.is_symlink(), "{path}");
            assert_eq!(fs::read_link(&link).expect("read link"), Path::new(target));
        }
        // Each in-tree link resolves, so a server that followed it would
        // find a file: the refusal is the server's, not a dangling link.
        assert!(root.join("uploads/docs/hivatkozas.pdf").is_file());
        assert!(root.join("uploads/mappa/Órarend.PDF").is_file());
        let folder = fs::metadata(root.join("uploads/docs")).expect("folder");
        assert_eq!(folder.permissions().mode() & 0o777, 0o755);
        fs::remove_dir_all(&root).expect("clean up");
    }

    #[test]
    fn seed_refuses_a_directory_that_is_not_empty_or_missing() {
        let root = scratch("refuse");
        fs::write(root.join("real-media.jpg"), b"x").expect("write");
        let error = seed(&root).expect_err("not empty");
        assert!(error.contains("not empty"), "{error}");
        fs::remove_dir_all(&root).expect("clean up");
        let error = seed(&root).expect_err("missing");
        assert!(error.contains("not a readable directory"), "{error}");
    }

    #[test]
    fn last_modified_is_mtime_as_an_http_date() {
        // 1_788_249_600 s after the epoch is day 20 697 plus 8 hours; day 0,
        // 1970-01-01, was a Thursday, so day 20 697 is a Tuesday. The
        // calendar date itself is checked by every Last-Modified assertion
        // of the nginx run.
        assert_eq!(MTIME.as_secs() / 86_400, 20_697);
        assert_eq!(MTIME.as_secs() % 86_400, 8 * 3_600);
        assert_eq!((20_697 + 4) % 7, 2, "Tuesday");
        assert!(LAST_MODIFIED.starts_with("Tue, 01 Sep 2026 08:00:00"));
    }

    /// A fresh, empty directory below Cargo's per-target scratch directory.
    fn scratch(name: &str) -> std::path::PathBuf {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("media-fixtures-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        root
    }
}
