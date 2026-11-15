//! The rules of an entity image upload that need no file system (ADR 0015
//! §3 and §6).
//!
//! An entity image is the picture of an author, a colleague or a post: it is
//! written only through its entity form, never through the media library,
//! and it is stored as `<folder>/<32 hex>.<ext>` under `media.root`, in the
//! folders and with the name shape that backend1 used, so one URL rule
//! serves the legacy files and the new ones (ADR 0015 §6, "Entity image
//! names"). The upload model (`src/models/media.rs`) runs these rules before
//! and between its file-system steps:
//!
//! 1. **The allow-list.** [`ImageExtension::from_file_name`] takes the
//!    extension of the client's file name, in any case, and accepts only the
//!    JPEG (`jpg`, `jpeg`, `jfif`), PNG, GIF and WebP extensions (ADR 0015
//!    §6, "Entity image fields"). The client's content type is never read.
//! 2. **The sniffed type agrees.** [`check_sniffed_type`] compares the
//!    format that the magic bytes name with the extension's, so an SVG or an
//!    HTML file renamed to `.png`, or a JPEG named `.png`, is refused before
//!    anything is decoded.
//! 3. **The name.** [`entity_image_name`] writes 32 lower-case hex digits
//!    from 16 random bytes that the caller draws, plus the lower-cased
//!    extension. The bytes are an argument because this module reads no
//!    random source (ADR 0008 rule 1).
//! 4. **The stored value and the path.** [`stored_value`] is what the
//!    entity's row keeps: the path below the folder's disk, in the shape of
//!    the legacy rows (`<name>`, or `index/<name>` for a post's index
//!    image), so the public API's URL rule `{disk}/{stored value}` serves
//!    both. [`stored_image_path`] is the path below `media.root` that the
//!    file is written to, `<disk>/<stored value>`.
//! 5. **The path stays under the root.** [`relative_segments`] checks every
//!    path before the model touches the file system: relative, with no
//!    empty, `.`, `..` or hidden segment, no backslash or control character,
//!    and no segment longer than a file name may be. A path that passes
//!    cannot leave the directory it is joined to. The model also opens every
//!    segment without following symbolic links, which no text rule can see.
//! 6. **Free space.** [`leaves_min_free`] says whether a write of a given
//!    size keeps `media.min_free_bytes` free (ADR 0015 §6, "Free space"), and
//!    [`SpaceWatch`] applies it to a body while it arrives, from readings of
//!    the free space that the model takes every [`SPACE_RECHECK_BYTES`].
//! 7. **The directories.** [`check_media_dirs`] checks the configured root,
//!    staging and trash directories against each other at boot: the staging
//!    and trash directories hold partial and deleted files, so neither may
//!    lie inside the served tree, nor the tree inside them.
//!
//! The human file names of the media library (`uploads/`, NFC, ` (1)` on a
//! conflict) are not entity image names; their rules come with the media
//! library.

use std::path::{Component, Path};

use core::fmt;

use crate::domain::image_optimise::ImageFormat;

/// The folders under `media.root` that hold entity images, each written only
/// through its entity form (ADR 0015 §5 and §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityImageFolder {
    /// `posts_images/`: the gallery images of posts.
    PostImages,
    /// `posts_images/index/`: the index image of a post.
    PostIndexImages,
    /// `authors_images/`: the photo of an author.
    AuthorImages,
    /// `colleagues_images/`: the photo of a colleague.
    ColleagueImages,
}

impl EntityImageFolder {
    /// Every entity image folder.
    pub const ALL: [Self; 4] = [
        Self::PostImages,
        Self::PostIndexImages,
        Self::AuthorImages,
        Self::ColleagueImages,
    ];

    /// The folder's path segments below `media.root`, outermost first: the
    /// same directories that backend1 wrote and the public API's URLs name
    /// (public-api.md §2.3).
    #[must_use]
    pub const fn segments(self) -> &'static [&'static str] {
        match self {
            Self::PostImages => &["posts_images"],
            Self::PostIndexImages => &["posts_images", "index"],
            Self::AuthorImages => &["authors_images"],
            Self::ColleagueImages => &["colleagues_images"],
        }
    }

    /// The folder's disk: the directory below `media.root` that the public
    /// API's media URLs name between the base and the stored value,
    /// `{public_base_url}/{disk}/{stored value}` (public-api.md §2.3). The
    /// two post folders share `posts_images`, because a post's index image
    /// is stored as `index/<name>` below it ([`stored_value`]).
    #[must_use]
    pub const fn disk(self) -> &'static str {
        match self {
            Self::PostImages | Self::PostIndexImages => "posts_images",
            Self::AuthorImages => "authors_images",
            Self::ColleagueImages => "colleagues_images",
        }
    }

    /// The folder's path below `media.root`, its segments joined with `/`.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::PostImages => "posts_images",
            Self::PostIndexImages => "posts_images/index",
            Self::AuthorImages => "authors_images",
            Self::ColleagueImages => "colleagues_images",
        }
    }
}

/// An extension that an entity image may have (ADR 0015 §6): the three JPEG
/// spellings, PNG, GIF and WebP. The stored name keeps the upload's own
/// spelling, lower-cased, so `.JPEG` stays `.jpeg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageExtension {
    /// `.jpg`.
    Jpg,
    /// `.jpeg`.
    Jpeg,
    /// `.jfif`, which ADR 0015 §6 counts as JPEG; the media server types it
    /// `image/jpeg` (`deploy/media/nginx.conf`).
    Jfif,
    /// `.png`.
    Png,
    /// `.gif`.
    Gif,
    /// `.webp`.
    Webp,
}

impl ImageExtension {
    /// Every allowed extension.
    pub const ALL: [Self; 6] = [
        Self::Jpg,
        Self::Jpeg,
        Self::Jfif,
        Self::Png,
        Self::Gif,
        Self::Webp,
    ];

    /// The extension in lower case, without the dot.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jpg => "jpg",
            Self::Jpeg => "jpeg",
            Self::Jfif => "jfif",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }

    /// The image format that a file with this extension must be.
    #[must_use]
    pub const fn format(self) -> ImageFormat {
        match self {
            Self::Jpg | Self::Jpeg | Self::Jfif => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
            Self::Gif => ImageFormat::Gif,
            Self::Webp => ImageFormat::WebP,
        }
    }

    /// The allowed extension of a client's file name.
    ///
    /// - `file_name`: the name the client sent with the file. Only the part
    ///   after its last `.` is read, compared without regard to ASCII case;
    ///   the rest of the name plays no part, because an entity image gets a
    ///   generated name.
    ///
    /// # Errors
    ///
    /// [`MediaRuleError::ExtensionNotAllowed`] when the name has no `.`, ends
    /// with one, or its extension is not on the list (`.svg`, `.html`,
    /// `.pdf`, `.heic`, ...).
    pub fn from_file_name(file_name: &str) -> Result<Self, MediaRuleError> {
        let (_, extension) = file_name
            .rsplit_once('.')
            .ok_or(MediaRuleError::ExtensionNotAllowed)?;
        Self::ALL
            .into_iter()
            .find(|allowed| allowed.as_str().eq_ignore_ascii_case(extension))
            .ok_or(MediaRuleError::ExtensionNotAllowed)
    }
}

impl fmt::Display for ImageExtension {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why an upload is refused before its image is read. The messages are
/// English, for logs and script clients; the editor sees a Hungarian one
/// (ADR 0021 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MediaRuleError {
    /// The file name has no extension, or one that is not an allowed image
    /// extension.
    #[error(
        "only JPEG, PNG, GIF and WebP images with a .jpg, .jpeg, .jfif, .png, .gif or .webp name are accepted"
    )]
    ExtensionNotAllowed,
    /// The content starts with none of the four image signatures: an SVG,
    /// HTML, PDF or any other file, whatever its name says.
    #[error("the file content is not a JPEG, PNG, GIF or WebP image")]
    UnrecognisedContent,
    /// The content is an image of another format than its extension names.
    #[error("the file is named .{extension} but its content is a {found} image")]
    TypeMismatch {
        /// The extension of the file name.
        extension: ImageExtension,
        /// The format that the magic bytes name.
        found: ImageFormat,
    },
}

/// Checks that the sniffed format of an upload agrees with its extension
/// (ADR 0015 §6, "Type check").
///
/// - `extension`: the allowed extension of the file name.
/// - `sniffed`: what `image_optimise::sniff` found in the first bytes, `None`
///   when they are no image signature.
///
/// # Errors
///
/// [`MediaRuleError::UnrecognisedContent`] for content that is no image, and
/// [`MediaRuleError::TypeMismatch`] for an image of another format.
pub fn check_sniffed_type(
    extension: ImageExtension,
    sniffed: Option<ImageFormat>,
) -> Result<ImageFormat, MediaRuleError> {
    match sniffed {
        None => Err(MediaRuleError::UnrecognisedContent),
        Some(found) if found == extension.format() => Ok(found),
        Some(found) => Err(MediaRuleError::TypeMismatch { extension, found }),
    }
}

/// How many random bytes an entity image name takes: 16, written as the 32
/// hex digits of the legacy names (ADR 0015 §6).
pub const ENTITY_IMAGE_NAME_BYTES: usize = 16;

/// The file name of a new entity image: 32 lower-case hex digits and the
/// lower-cased extension, for example `0f3a…9c.jpg`.
///
/// - `random`: 16 bytes from the operating system's random source, drawn by
///   the caller. 128 random bits make a collision with any existing name
///   practically impossible; the model still never overwrites a file.
/// - `extension`: the upload's allowed extension.
#[must_use]
pub fn entity_image_name(
    random: [u8; ENTITY_IMAGE_NAME_BYTES],
    extension: ImageExtension,
) -> String {
    use fmt::Write as _;
    let mut name = String::with_capacity(2 * ENTITY_IMAGE_NAME_BYTES + 1 + 4);
    for byte in random {
        // Writing to a String cannot fail.
        let _ = write!(name, "{byte:02x}");
    }
    name.push('.');
    name.push_str(extension.as_str());
    name
}

/// Whether `name` has the shape of an entity image name: 32 lower-case hex
/// digits, a dot and an allowed extension in lower case.
///
/// - `name`: a file name, without any folder.
#[must_use]
pub fn is_entity_image_name(name: &str) -> bool {
    name.split_once('.').is_some_and(|(stem, extension)| {
        stem.len() == 2 * ENTITY_IMAGE_NAME_BYTES
            && stem
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            && ImageExtension::ALL
                .iter()
                .any(|allowed| allowed.as_str() == extension)
    })
}

/// The value that an entity's row stores for a new image: its path below the
/// folder's disk ([`EntityImageFolder::disk`]). It has the shape of the
/// legacy rows, which the importer copies verbatim, so that one URL rule,
/// `{public_base_url}/{disk}/{stored value}`, serves the old images and the
/// new ones (public-api.md §2.3, ADR 0015 §6 "Entity image names", parity
/// AUT-03):
///
/// | Folder | Stored value | Legacy column |
/// |---|---|---|
/// | [`EntityImageFolder::PostImages`] | `<32 hex>.<ext>` | an element of `posts.images` (data-migration.md §6.1) |
/// | [`EntityImageFolder::PostIndexImages`] | `index/<32 hex>.<ext>` | `posts.index_image` (§6.1) |
/// | [`EntityImageFolder::AuthorImages`] | `<32 hex>.<ext>` | `authors.image` (§6.4) |
/// | [`EntityImageFolder::ColleagueImages`] | `<32 hex>.<ext>` | `colleagues.image` (§6.5) |
///
/// - `folder`: the entity's image folder.
/// - `name`: the file name, from [`entity_image_name`].
#[must_use]
pub fn stored_value(folder: EntityImageFolder, name: &str) -> String {
    match folder {
        EntityImageFolder::PostIndexImages => format!("index/{name}"),
        EntityImageFolder::PostImages
        | EntityImageFolder::AuthorImages
        | EntityImageFolder::ColleagueImages => name.to_owned(),
    }
}

/// The path of an entity image below `media.root`: `<disk>/<stored value>`,
/// which is `<folder>/<name>`. The file is written there, and the admin API
/// loads it as `/storage/<path>` (ADR 0015 §3). The row stores
/// [`stored_value`], not this path.
///
/// - `folder`: the entity's image folder.
/// - `name`: the file name, from [`entity_image_name`].
#[must_use]
pub fn stored_image_path(folder: EntityImageFolder, name: &str) -> String {
    format!("{}/{}", folder.disk(), stored_value(folder, name))
}

/// The longest path segment, in bytes: the `NAME_MAX` of Linux file systems,
/// xfs included (ADR 0015 §6).
pub const MAX_SEGMENT_BYTES: usize = 255;

/// Why a path is not a safe path below the media root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathRuleError {
    /// The path is empty.
    #[error("the path is empty")]
    Empty,
    /// The path starts with `/`, so it would not stay below the root.
    #[error("the path is absolute")]
    Absolute,
    /// The path has an empty segment (`a//b`, or a trailing `/`).
    #[error("the path has an empty segment")]
    EmptySegment,
    /// A segment is `.` or `..`, or starts with `.`: the media server never
    /// serves a dot-segment (ADR 0015 §1), and `..` would climb out.
    #[error("a path segment is or starts with a dot")]
    DotSegment,
    /// A segment holds a backslash, NUL or another control character.
    #[error("a path segment holds a backslash or a control character")]
    ForbiddenCharacter,
    /// A segment is longer than [`MAX_SEGMENT_BYTES`].
    #[error("a path segment is longer than 255 bytes")]
    SegmentTooLong,
}

/// The segments of a path that stays below whatever directory it is joined
/// to, or why it would not.
///
/// - `path`: a `/`-separated path relative to `media.root`, such as
///   `authors_images/<name>`.
///
/// A path that passes has only plain segments, so joining it to a directory,
/// segment by segment, never climbs above that directory. Symbolic links are
/// beyond a text rule: the model opens each segment without following one.
///
/// # Errors
///
/// The first [`PathRuleError`] the path breaks: empty, absolute, then per
/// segment an empty one, a dot-segment, a forbidden character or a segment
/// that is too long.
pub fn relative_segments(path: &str) -> Result<Vec<&str>, PathRuleError> {
    if path.is_empty() {
        return Err(PathRuleError::Empty);
    }
    if path.starts_with('/') {
        return Err(PathRuleError::Absolute);
    }
    path.split('/')
        .map(|segment| {
            if segment.is_empty() {
                Err(PathRuleError::EmptySegment)
            } else if segment.starts_with('.') {
                Err(PathRuleError::DotSegment)
            } else if segment
                .chars()
                .any(|character| character == '\\' || character.is_control())
            {
                Err(PathRuleError::ForbiddenCharacter)
            } else if segment.len() > MAX_SEGMENT_BYTES {
                Err(PathRuleError::SegmentTooLong)
            } else {
                Ok(segment)
            }
        })
        .collect()
}

/// Whether writing `to_write` more bytes onto a file system with
/// `available` bytes free leaves at least `min_free` free (ADR 0015 §6:
/// "An upload that would leave less than `media.min_free_bytes` free is
/// refused").
///
/// - `available`: the bytes that an unprivileged process may still write,
///   as `statvfs` reports them (`f_bavail × f_frsize`).
/// - `to_write`: the bytes the upload is about to write.
/// - `min_free`: `media.min_free_bytes`.
#[must_use]
pub fn leaves_min_free(available: u64, to_write: u64, min_free: u64) -> bool {
    available
        .checked_sub(to_write)
        .is_some_and(|left| left >= min_free)
}

/// How many bytes an upload may receive after a reading of the free space
/// before [`SpaceWatch`] wants the next one: 4 MiB. A reading is one
/// `fstatvfs`, so this costs 32 system calls for a body at the 128 MiB
/// limit, and it bounds by how much one upload can undercut the reserve
/// while other writers fill the volume ([`SpaceWatch`]).
pub const SPACE_RECHECK_BYTES: u64 = 4 * 1024 * 1024;

/// An upload that would leave less than `media.min_free_bytes` free: the
/// numbers of the bound that failed, for the operator's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceShortfall {
    /// The free bytes of the reading that the failed bound starts from.
    pub available: u64,
    /// The bytes received since that reading.
    pub needed: u64,
    /// `media.min_free_bytes`.
    pub min_free: u64,
}

/// The free-space reserve of an upload while its body arrives (ADR 0015 §6,
/// "Free space": an upload that would leave less than `media.min_free_bytes`
/// free is refused).
///
/// The caller reads the volume's free space before the first byte
/// ([`SpaceWatch::new`]), tells the watch the size of every chunk it
/// receives ([`SpaceWatch::receive`]), and reads the free space again
/// whenever [`SpaceWatch::wants_reading`] says so, every
/// [`SPACE_RECHECK_BYTES`] ([`SpaceWatch::record_reading`]). The upload is
/// refused as soon as either of two bounds fails:
///
/// - **The first reading minus everything received.** It holds whatever
///   the file system reports: some (copy-on-write ones such as btrfs, for
///   example) show an upload's own writes in their free space late, so the
///   latest reading alone could let one upload run on unchecked.
/// - **The latest reading minus what arrived since.** It sees what other
///   writers used meanwhile: concurrent uploads, which each start from a
///   reading of their own, and anything else on the volume. Between two
///   readings each of them can still use up to [`SPACE_RECHECK_BYTES`]
///   unseen, so the reserve holds to within that much per concurrent
///   writer, not within a whole body of 128 MiB each.
///
/// The bytes counted are the bytes received, whether they are written to the
/// staging file yet or still held in memory: counting early only refuses
/// sooner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceWatch {
    /// `media.min_free_bytes`.
    min_free: u64,
    /// The free bytes of the reading before the first byte.
    first: u64,
    /// The bytes received in all.
    received: u64,
    /// The free bytes of the latest reading.
    latest: u64,
    /// The bytes received since the latest reading.
    since_latest: u64,
}

impl SpaceWatch {
    /// Starts watching an upload from the reading taken before its first
    /// byte.
    ///
    /// - `available`: the bytes an unprivileged process may still write on
    ///   the volume (`f_bavail × f_frsize`).
    /// - `min_free`: `media.min_free_bytes`.
    ///
    /// # Errors
    ///
    /// [`SpaceShortfall`] when the volume is below the reserve already, so
    /// the upload is refused before its body is read.
    pub fn new(available: u64, min_free: u64) -> Result<Self, SpaceShortfall> {
        let watch = Self {
            min_free,
            first: available,
            received: 0,
            latest: available,
            since_latest: 0,
        };
        watch.check().map(|()| watch)
    }

    /// Counts a chunk of the body.
    ///
    /// - `bytes`: the chunk's size.
    ///
    /// # Errors
    ///
    /// [`SpaceShortfall`] when the bytes received so far would leave less
    /// than the reserve by either bound.
    pub fn receive(&mut self, bytes: u64) -> Result<(), SpaceShortfall> {
        self.received = self.received.saturating_add(bytes);
        self.since_latest = self.since_latest.saturating_add(bytes);
        self.check()
    }

    /// Whether the caller should read the free space again:
    /// [`SPACE_RECHECK_BYTES`] or more have arrived since the latest
    /// reading.
    #[must_use]
    pub const fn wants_reading(&self) -> bool {
        self.since_latest >= SPACE_RECHECK_BYTES
    }

    /// Takes a new reading of the free space; the bytes received from here
    /// on count against it.
    ///
    /// - `available`: the free bytes now, read as for [`SpaceWatch::new`].
    ///
    /// # Errors
    ///
    /// [`SpaceShortfall`] when the reading is below the reserve, or the
    /// first bound fails.
    pub fn record_reading(&mut self, available: u64) -> Result<(), SpaceShortfall> {
        self.latest = available;
        self.since_latest = 0;
        self.check()
    }

    /// The bytes received in all.
    #[must_use]
    pub const fn received(&self) -> u64 {
        self.received
    }

    /// Both bounds of the type documentation, the first one first.
    fn check(&self) -> Result<(), SpaceShortfall> {
        for (available, needed) in [
            (self.first, self.received),
            (self.latest, self.since_latest),
        ] {
            if !leaves_min_free(available, needed, self.min_free) {
                return Err(SpaceShortfall {
                    available,
                    needed,
                    min_free: self.min_free,
                });
            }
        }
        Ok(())
    }
}

/// One of the three directories of the media settings (ADR 0015 §3), named
/// as its key below `settings.media` in the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaDir {
    /// `root`: the served tree.
    Root,
    /// `tmp_dir`: upload staging.
    TmpDir,
    /// `trash_dir`: deleted files.
    TrashDir,
}

impl MediaDir {
    /// The key of the directory below `settings.media`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::TmpDir => "tmp_dir",
            Self::TrashDir => "trash_dir",
        }
    }
}

impl fmt::Display for MediaDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

/// Why the three media directories cannot work together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MediaDirsError {
    /// A directory setting is empty.
    #[error("{0} is empty; set a directory")]
    Empty(MediaDir),
    /// A directory setting has a `..` segment, which the lexical checks
    /// below cannot follow.
    #[error(
        "{0} has a `..` segment; write the directory without it, for example as an absolute path"
    )]
    ParentSegment(MediaDir),
    /// Two settings name the same directory, or one lies inside the other.
    /// The staging and trash directories hold partial and deleted files, so
    /// inside the served tree they would be served, and the served tree
    /// inside either of them could be cleaned away with it.
    #[error(
        "{inner} lies inside {outer} or is the same directory; give each its own directory, none inside another"
    )]
    Nested {
        /// The directory that lies inside the other, or the second of two
        /// equal ones.
        inner: MediaDir,
        /// The directory that holds it.
        outer: MediaDir,
    },
}

/// Checks the three media directories against each other, from their text
/// alone (no file system is read: production starts before its volume is
/// mounted, ADR 0015 §3).
///
/// - `root`: `media.root`, the served tree.
/// - `tmp_dir`: `media.tmp_dir`, upload staging.
/// - `trash_dir`: `media.trash_dir`, deleted files.
///
/// The comparison is lexical, after dropping `.` segments: a relative and
/// an absolute path never count as nested, and neither do two paths that
/// only a symbolic link joins. That is the operator's configuration, which
/// backend3 trusts; the paths that uploads build below the root are checked
/// per upload.
///
/// # Errors
///
/// [`MediaDirsError::Empty`], [`MediaDirsError::ParentSegment`] or
/// [`MediaDirsError::Nested`] for the first directory, in the order root,
/// staging, trash, that breaks a rule.
pub fn check_media_dirs(
    root: &Path,
    tmp_dir: &Path,
    trash_dir: &Path,
) -> Result<(), MediaDirsError> {
    let dirs = [
        (MediaDir::Root, root),
        (MediaDir::TmpDir, tmp_dir),
        (MediaDir::TrashDir, trash_dir),
    ];
    let mut normalised = Vec::with_capacity(dirs.len());
    for (dir, path) in dirs {
        if path.as_os_str().is_empty() {
            return Err(MediaDirsError::Empty(dir));
        }
        let components: Vec<Component<'_>> = path
            .components()
            .filter(|component| *component != Component::CurDir)
            .collect();
        if components.contains(&Component::ParentDir) {
            return Err(MediaDirsError::ParentSegment(dir));
        }
        normalised.push((dir, components));
    }
    for (index, (inner, inner_path)) in normalised.iter().enumerate() {
        for (outer, outer_path) in &normalised[..index] {
            if inner_path.starts_with(outer_path) {
                return Err(MediaDirsError::Nested {
                    inner: *inner,
                    outer: *outer,
                });
            }
            if outer_path.starts_with(inner_path) {
                return Err(MediaDirsError::Nested {
                    inner: *outer,
                    outer: *inner,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{collection::vec, prelude::*, sample::select};
    use std::path::PathBuf;

    #[test]
    fn folders_are_the_legacy_directories() {
        let paths: Vec<&str> = EntityImageFolder::ALL
            .iter()
            .map(|folder| folder.path())
            .collect();
        assert_eq!(
            paths,
            [
                "posts_images",
                "posts_images/index",
                "authors_images",
                "colleagues_images"
            ]
        );
        for folder in EntityImageFolder::ALL {
            assert_eq!(folder.segments().join("/"), folder.path(), "{folder:?}");
            assert_eq!(
                relative_segments(folder.path()).as_deref(),
                Ok(folder.segments())
            );
        }
    }

    #[test]
    fn the_stored_value_has_the_shape_of_the_legacy_rows() {
        // public-api.md §2.3's table and data-migration.md §6.1, §6.4 and
        // §6.5: the disk, then the value the row keeps.
        let name = "000123456789abcdefff102030405060.jpg";
        for (folder, disk, stored) in [
            (
                EntityImageFolder::PostImages,
                "posts_images",
                name.to_owned(),
            ),
            (
                EntityImageFolder::PostIndexImages,
                "posts_images",
                format!("index/{name}"),
            ),
            (
                EntityImageFolder::AuthorImages,
                "authors_images",
                name.to_owned(),
            ),
            (
                EntityImageFolder::ColleagueImages,
                "colleagues_images",
                name.to_owned(),
            ),
        ] {
            assert_eq!(folder.disk(), disk, "{folder:?}");
            assert_eq!(stored_value(folder, name), stored, "{folder:?}");
            assert_eq!(
                stored_image_path(folder, name),
                format!("{}/{name}", folder.path()),
                "{folder:?}"
            );
            assert_eq!(folder.segments().first(), Some(&disk), "{folder:?}");
        }
    }

    #[test]
    fn the_space_watch_refuses_at_the_byte_that_crosses_the_reserve() {
        // 100 free, 60 kept: 40 bytes may arrive, the 41st is refused.
        let mut watch = SpaceWatch::new(100, 60).expect("above the reserve");
        assert_eq!(watch.receive(40), Ok(()));
        assert_eq!(
            watch.receive(1),
            Err(SpaceShortfall {
                available: 100,
                needed: 41,
                min_free: 60
            })
        );
        assert_eq!(watch.received(), 41);
        // Below the reserve before the first byte, or exactly at it.
        assert_eq!(
            SpaceWatch::new(59, 60),
            Err(SpaceShortfall {
                available: 59,
                needed: 0,
                min_free: 60
            })
        );
        assert!(SpaceWatch::new(60, 60).is_ok());
    }

    #[test]
    fn a_new_reading_sees_what_other_writers_used() {
        let mut watch = SpaceWatch::new(1_000, 100).expect("above the reserve");
        assert_eq!(watch.receive(10), Ok(()));
        // Another writer used 850 bytes meanwhile: 140 are left, so 40 more
        // may arrive, and the 41st is refused against the new reading.
        assert_eq!(watch.record_reading(140), Ok(()));
        assert_eq!(watch.receive(40), Ok(()));
        assert_eq!(
            watch.receive(1),
            Err(SpaceShortfall {
                available: 140,
                needed: 41,
                min_free: 100
            })
        );
        // A reading below the reserve refuses at once.
        let mut watch = SpaceWatch::new(1_000, 100).expect("above the reserve");
        assert_eq!(
            watch.record_reading(99),
            Err(SpaceShortfall {
                available: 99,
                needed: 0,
                min_free: 100
            })
        );
        // More free space later never lifts the first bound.
        let mut watch = SpaceWatch::new(1_000, 100).expect("above the reserve");
        assert_eq!(watch.record_reading(u64::MAX), Ok(()));
        assert_eq!(
            watch.receive(901),
            Err(SpaceShortfall {
                available: 1_000,
                needed: 901,
                min_free: 100
            })
        );
    }

    #[test]
    fn a_reading_is_wanted_every_four_mebibytes() {
        assert_eq!(SPACE_RECHECK_BYTES, 4 * 1024 * 1024);
        let mut watch = SpaceWatch::new(u64::MAX, 0).expect("no reserve");
        assert!(!watch.wants_reading());
        watch.receive(SPACE_RECHECK_BYTES - 1).expect("room");
        assert!(!watch.wants_reading());
        watch.receive(1).expect("room");
        assert!(watch.wants_reading());
        watch.record_reading(u64::MAX).expect("room");
        assert!(!watch.wants_reading());
        assert_eq!(watch.received(), SPACE_RECHECK_BYTES);
    }

    #[test]
    fn the_allow_list_takes_the_image_extensions_in_any_case() {
        for (name, extension) in [
            ("photo.jpg", ImageExtension::Jpg),
            ("IMG_0001.JPG", ImageExtension::Jpg),
            ("scan.Jpeg", ImageExtension::Jpeg),
            ("old.jfif", ImageExtension::Jfif),
            ("logo.png", ImageExtension::Png),
            ("anim.GIF", ImageExtension::Gif),
            ("kép ékezettel.webp", ImageExtension::Webp),
            ("archive.tar.png", ImageExtension::Png),
            (".png", ImageExtension::Png),
        ] {
            assert_eq!(
                ImageExtension::from_file_name(name),
                Ok(extension),
                "{name}"
            );
        }
        for extension in ImageExtension::ALL {
            assert_eq!(extension.to_string(), extension.as_str());
        }
    }

    #[test]
    fn everything_else_is_refused_by_name() {
        for name in [
            "",
            "photo",
            "photo.",
            "drawing.svg",
            "page.html",
            "page.htm",
            "doc.pdf",
            "phone.heic",
            "photo.avif",
            "photo.jpg.exe",
            "photo.jpg ",
            "photo. jpg",
            "photo.ｊｐｇ",
            "archive.zip",
            "photo.bmp",
            "photo.tiff",
        ] {
            assert_eq!(
                ImageExtension::from_file_name(name),
                Err(MediaRuleError::ExtensionNotAllowed),
                "{name:?}"
            );
        }
    }

    #[test]
    fn each_extension_names_its_format() {
        let formats: Vec<ImageFormat> = ImageExtension::ALL.iter().map(|e| e.format()).collect();
        assert_eq!(
            formats,
            [
                ImageFormat::Jpeg,
                ImageFormat::Jpeg,
                ImageFormat::Jpeg,
                ImageFormat::Png,
                ImageFormat::Gif,
                ImageFormat::WebP,
            ]
        );
    }

    #[test]
    fn the_sniffed_type_must_agree_with_the_extension() {
        // Every pair of extension and sniffed format, exhaustively.
        for extension in ImageExtension::ALL {
            assert_eq!(
                check_sniffed_type(extension, None),
                Err(MediaRuleError::UnrecognisedContent)
            );
            for found in ImageFormat::ALL {
                let expected = if found == extension.format() {
                    Ok(found)
                } else {
                    Err(MediaRuleError::TypeMismatch { extension, found })
                };
                assert_eq!(check_sniffed_type(extension, Some(found)), expected);
            }
        }
        assert_eq!(
            MediaRuleError::TypeMismatch {
                extension: ImageExtension::Png,
                found: ImageFormat::Jpeg
            }
            .to_string(),
            "the file is named .png but its content is a JPEG image"
        );
    }

    #[test]
    fn a_name_is_32_hex_digits_and_the_extension() {
        let bytes = [
            0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xff, 0x10, 0x20, 0x30, 0x40,
            0x50, 0x60,
        ];
        assert_eq!(
            entity_image_name(bytes, ImageExtension::Jpeg),
            "000123456789abcdefff102030405060.jpeg"
        );
        assert_eq!(
            stored_image_path(EntityImageFolder::PostIndexImages, "ab.png"),
            "posts_images/index/ab.png"
        );
        for name in [
            "000123456789abcdefff102030405060.JPG",
            "000123456789ABCDEFFF102030405060.jpg",
            "000123456789abcdefff10203040506.jpg",
            "000123456789abcdefff102030405060",
            "000123456789abcdefff102030405060.svg",
            "000123456789abcdefff102030405060.jpg.png",
        ] {
            assert!(!is_entity_image_name(name), "{name}");
        }
    }

    #[test]
    fn unsafe_paths_are_refused_with_their_reason() {
        let long = "a".repeat(MAX_SEGMENT_BYTES + 1);
        for (path, expected) in [
            ("", PathRuleError::Empty),
            ("/etc/passwd", PathRuleError::Absolute),
            ("authors_images//a.png", PathRuleError::EmptySegment),
            ("authors_images/", PathRuleError::EmptySegment),
            ("authors_images/../a.png", PathRuleError::DotSegment),
            ("..", PathRuleError::DotSegment),
            ("./a.png", PathRuleError::DotSegment),
            ("authors_images/.htaccess", PathRuleError::DotSegment),
            (
                "authors_images\\..\\a.png",
                PathRuleError::ForbiddenCharacter,
            ),
            ("a\0b", PathRuleError::ForbiddenCharacter),
            ("a\nb", PathRuleError::ForbiddenCharacter),
            (long.as_str(), PathRuleError::SegmentTooLong),
        ] {
            assert_eq!(relative_segments(path), Err(expected), "{path:?}");
        }
        let longest = "é".repeat(MAX_SEGMENT_BYTES / 2);
        assert_eq!(relative_segments(&longest), Ok(vec![longest.as_str()]));
    }

    #[test]
    fn free_space_rule_at_its_boundaries() {
        assert!(leaves_min_free(100, 40, 60));
        assert!(!leaves_min_free(100, 41, 60));
        assert!(leaves_min_free(100, 0, 100));
        assert!(!leaves_min_free(99, 0, 100));
        assert!(!leaves_min_free(10, 11, 0));
        assert!(leaves_min_free(u64::MAX, u64::MAX, 0));
        assert!(leaves_min_free(u64::MAX, 0, u64::MAX));
        assert!(!leaves_min_free(u64::MAX, 1, u64::MAX));
    }

    #[test]
    fn the_media_directories_must_not_nest() {
        let check = |root: &str, tmp: &str, trash: &str| {
            check_media_dirs(Path::new(root), Path::new(tmp), Path::new(trash))
        };
        // The production layout of ADR 0015 §3.
        assert_eq!(
            check(
                "/data/app/public",
                "/data/backend3/tmp",
                "/data/backend3/trash"
            ),
            Ok(())
        );
        assert_eq!(
            check(
                "target/media/public",
                "./target/media/tmp",
                "target/media/trash/"
            ),
            Ok(())
        );
        // Siblings that share a prefix of characters are not nested.
        assert_eq!(
            check("/data/public", "/data/public-tmp", "/data/trash"),
            Ok(())
        );
        for (root, tmp, trash, expected) in [
            ("", "/t", "/x", MediaDirsError::Empty(MediaDir::Root)),
            ("/r", "", "/x", MediaDirsError::Empty(MediaDir::TmpDir)),
            ("/r", "/t", "", MediaDirsError::Empty(MediaDir::TrashDir)),
            (
                "/data/../r",
                "/t",
                "/x",
                MediaDirsError::ParentSegment(MediaDir::Root),
            ),
            (
                "/r",
                "../t",
                "/x",
                MediaDirsError::ParentSegment(MediaDir::TmpDir),
            ),
            (
                "/data/app/public",
                "/data/app/public/tmp",
                "/x",
                MediaDirsError::Nested {
                    inner: MediaDir::TmpDir,
                    outer: MediaDir::Root,
                },
            ),
            (
                "/data/app/public",
                "/t",
                "/data/app/public/.trash",
                MediaDirsError::Nested {
                    inner: MediaDir::TrashDir,
                    outer: MediaDir::Root,
                },
            ),
            (
                "/data/tmp/public",
                "/data/tmp",
                "/x",
                MediaDirsError::Nested {
                    inner: MediaDir::Root,
                    outer: MediaDir::TmpDir,
                },
            ),
            (
                "/r",
                "/data/same",
                "/data/./same/",
                MediaDirsError::Nested {
                    inner: MediaDir::TrashDir,
                    outer: MediaDir::TmpDir,
                },
            ),
        ] {
            assert_eq!(
                check(root, tmp, trash),
                Err(expected),
                "{root} {tmp} {trash}"
            );
        }
        assert_eq!(
            MediaDirsError::Nested {
                inner: MediaDir::TmpDir,
                outer: MediaDir::Root
            }
            .to_string(),
            "tmp_dir lies inside root or is the same directory; give each its own directory, none \
             inside another"
        );
    }

    /// A plain path segment: no dot first, no slash, backslash or control
    /// character, at most 255 bytes.
    fn segment() -> impl Strategy<Value = String> {
        "[^./\\\\\\p{Cc}][^/\\\\\\p{Cc}]{0,20}"
    }

    proptest! {
        #[test]
        fn every_generated_name_is_an_entity_image_name_and_a_safe_path(
            random in any::<[u8; ENTITY_IMAGE_NAME_BYTES]>(),
            extension in select(ImageExtension::ALL.to_vec()),
            folder in select(EntityImageFolder::ALL.to_vec()),
        ) {
            let name = entity_image_name(random, extension);
            prop_assert!(is_entity_image_name(&name));
            prop_assert_eq!(ImageExtension::from_file_name(&name), Ok(extension));
            let path = stored_image_path(folder, &name);
            let segments = relative_segments(&path).expect("safe");
            prop_assert_eq!(segments.last().copied(), Some(name.as_str()));
            prop_assert_eq!(&segments[..segments.len() - 1], folder.segments());
        }

        #[test]
        fn the_stored_value_is_the_path_below_the_disk_in_the_legacy_shape(
            random in any::<[u8; ENTITY_IMAGE_NAME_BYTES]>(),
            extension in select(ImageExtension::ALL.to_vec()),
            folder in select(EntityImageFolder::ALL.to_vec()),
        ) {
            let name = entity_image_name(random, extension);
            let stored = stored_value(folder, &name);
            // The public URL rule `{disk}/{stored value}` names the file that
            // the model writes (public-api.md §2.3).
            prop_assert_eq!(
                stored_image_path(folder, &name),
                format!("{}/{stored}", folder.disk())
            );
            prop_assert!(relative_segments(&stored).is_ok());
            // data-migration.md §6.1, §6.4, §6.5: a bare name, or
            // `index/<name>` for a post's index image.
            let bare = match folder {
                EntityImageFolder::PostIndexImages => stored.strip_prefix("index/"),
                EntityImageFolder::PostImages
                | EntityImageFolder::AuthorImages
                | EntityImageFolder::ColleagueImages => Some(stored.as_str()),
            };
            prop_assert_eq!(bare, Some(name.as_str()));
            prop_assert!(is_entity_image_name(&name));
        }

        #[test]
        fn the_space_watch_is_never_weaker_than_the_first_reading(
            first in any::<u64>(),
            min_free in any::<u64>(),
            steps in vec((any::<bool>(), any::<u64>()), 0..20),
        ) {
            // Any mix of chunks and readings: while the watch admits the
            // body, the first reading minus everything received keeps the
            // reserve, and so does the latest reading minus what came since.
            let Ok(mut watch) = SpaceWatch::new(first, min_free) else {
                prop_assert!(first < min_free);
                return Ok(());
            };
            let (mut received, mut latest, mut since) = (0_u64, first, 0_u64);
            for (is_reading, value) in steps {
                let outcome = if is_reading {
                    latest = value;
                    since = 0;
                    watch.record_reading(value)
                } else {
                    let bytes = value % (2 * SPACE_RECHECK_BYTES);
                    received = received.saturating_add(bytes);
                    since = since.saturating_add(bytes);
                    watch.receive(bytes)
                };
                let both = leaves_min_free(first, received, min_free)
                    && leaves_min_free(latest, since, min_free);
                prop_assert_eq!(outcome.is_ok(), both);
                prop_assert_eq!(watch.received(), received);
                prop_assert_eq!(watch.wants_reading(), since >= SPACE_RECHECK_BYTES);
                if let Err(shortfall) = outcome {
                    prop_assert_eq!(shortfall.min_free, min_free);
                    prop_assert!(!leaves_min_free(shortfall.available, shortfall.needed, min_free));
                    return Ok(());
                }
            }
        }

        #[test]
        fn a_lone_upload_is_judged_as_by_one_reading(
            first in 0_u64..1 << 40,
            min_free in 0_u64..1 << 40,
            chunks in vec(0_u64..1 << 24, 0..40),
        ) {
            // With no other writer, every reading is the first one minus what
            // arrived (the file system shows the writes at once): the watch
            // refuses at exactly the chunk that the first bound refuses.
            let Ok(mut watch) = SpaceWatch::new(first, min_free) else {
                prop_assert!(first < min_free);
                return Ok(());
            };
            let mut received = 0_u64;
            for bytes in chunks {
                received += bytes;
                let mut outcome = watch.receive(bytes);
                if outcome.is_ok() && watch.wants_reading() {
                    outcome = watch.record_reading(first - received);
                }
                prop_assert_eq!(outcome.is_ok(), leaves_min_free(first, received, min_free));
                if outcome.is_err() {
                    return Ok(());
                }
            }
        }

        #[test]
        fn different_bytes_give_different_names(
            first in any::<[u8; ENTITY_IMAGE_NAME_BYTES]>(),
            second in any::<[u8; ENTITY_IMAGE_NAME_BYTES]>(),
            extension in select(ImageExtension::ALL.to_vec()),
        ) {
            prop_assert_eq!(
                entity_image_name(first, extension) == entity_image_name(second, extension),
                first == second
            );
        }

        #[test]
        fn the_extension_alone_decides_and_case_does_not_matter(
            stem in "\\PC{0,20}",
            extension in select(ImageExtension::ALL.to_vec()),
            upper in vec(any::<bool>(), 4),
        ) {
            let spelled: String = extension
                .as_str()
                .chars()
                .zip(upper.iter().cycle())
                .map(|(c, up)| if *up { c.to_ascii_uppercase() } else { c })
                .collect();
            prop_assert_eq!(
                ImageExtension::from_file_name(&format!("{stem}.{spelled}")),
                Ok(extension)
            );
        }

        #[test]
        fn a_name_whose_extension_is_not_listed_is_refused(name in "\\PC{0,30}") {
            let listed = name.rsplit_once('.').is_some_and(|(_, extension)| {
                ImageExtension::ALL.iter().any(|e| e.as_str().eq_ignore_ascii_case(extension))
            });
            prop_assert_eq!(ImageExtension::from_file_name(&name).is_ok(), listed);
        }

        #[test]
        fn a_safe_path_never_leaves_its_root(segments in vec(segment(), 1..5)) {
            let path = segments.join("/");
            let checked = relative_segments(&path).expect("plain segments");
            let root = PathBuf::from("/srv/media");
            let joined = checked.iter().fold(root.clone(), |dir, segment| dir.join(segment));
            prop_assert!(joined.starts_with(&root));
            prop_assert_eq!(joined.components().count(), root.components().count() + checked.len());
            prop_assert!(joined.components().all(|c| c != Component::ParentDir));
        }

        #[test]
        fn a_dot_dot_segment_is_refused_wherever_it_stands(
            before in vec(segment(), 0..3),
            after in vec(segment(), 0..3),
        ) {
            let path = [before, vec!["..".to_owned()], after].concat().join("/");
            prop_assert_eq!(relative_segments(&path), Err(PathRuleError::DotSegment));
        }

        #[test]
        fn arbitrary_paths_never_panic_and_accepted_ones_are_plain(path in "\\PC{0,40}") {
            if let Ok(segments) = relative_segments(&path) {
                prop_assert_eq!(segments.join("/"), path.as_str());
                for segment in segments {
                    prop_assert!(!segment.is_empty() && !segment.starts_with('.'));
                    prop_assert!(Path::new(segment).components().count() == 1);
                }
            }
        }

        #[test]
        fn the_free_space_rule_is_the_arithmetic(
            available in any::<u64>(),
            to_write in any::<u64>(),
            min_free in any::<u64>(),
        ) {
            let expected = u128::from(available) >= u128::from(to_write) + u128::from(min_free);
            prop_assert_eq!(leaves_min_free(available, to_write, min_free), expected);
        }

        #[test]
        fn nesting_is_refused_whichever_way_round(
            base in vec(segment(), 1..3),
            below in vec(segment(), 0..3),
            order in 0..6_usize,
        ) {
            let outer = PathBuf::from("/").join(base.join("/"));
            let inner = outer.join(below.join("/"));
            let other = PathBuf::from("/unrelated-media-directory");
            // Put the nested pair at two of the three places.
            let dirs = match order {
                0 => [outer.clone(), inner.clone(), other.clone()],
                1 => [inner.clone(), outer.clone(), other.clone()],
                2 => [outer.clone(), other.clone(), inner.clone()],
                3 => [inner.clone(), other.clone(), outer.clone()],
                4 => [other.clone(), outer.clone(), inner.clone()],
                _ => [other.clone(), inner.clone(), outer.clone()],
            };
            let nested = matches!(
                check_media_dirs(&dirs[0], &dirs[1], &dirs[2]),
                Err(MediaDirsError::Nested { .. })
            );
            prop_assert!(nested);
        }

        #[test]
        fn three_distinct_siblings_pass(names in vec(segment(), 3)) {
            prop_assume!(names[0] != names[1] && names[1] != names[2] && names[0] != names[2]);
            let dirs: Vec<PathBuf> = names.iter().map(|name| Path::new("/data").join(name)).collect();
            prop_assert_eq!(check_media_dirs(&dirs[0], &dirs[1], &dirs[2]), Ok(()));
        }
    }
}
