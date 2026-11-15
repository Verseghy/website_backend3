//! The uploads volume: how an uploaded image gets onto it (ADR 0015 §3, §6
//! and §7). A model module without an entity (ADR 0008 rule 5): every
//! upload endpoint shares it, and no table owns it.
//!
//! An upload takes two calls, so that the endpoint can check the rest of the
//! request between them:
//!
//! 1. [`receive_image`] streams the request body into a new file in
//!    `media.tmp_dir`, chunk by chunk, so the upload is never held whole in
//!    memory while it arrives and several uploads can arrive at once. Before
//!    it reads a byte, it checks the file name against the entity image
//!    allow-list and opens `media.root`, so an upload is refused before its
//!    body is read while the volume is not mounted. From the first 12 bytes
//!    it checks that the content is an image of the format the extension
//!    names, so an SVG renamed `.png` is refused before the rest is read. It
//!    holds the start of the body in memory, at most [`HEADER_WINDOW`], until
//!    the image header has arrived, and refuses an image above the pixel
//!    budget from that header ([`inspect_start`]). Only then does it create
//!    the staging file and write to it. All along it refuses to go past the
//!    free-space reserve of `media.min_free_bytes` ([`SpaceWatch`]). The file
//!    is flushed with `fsync` before it is handed back as a [`ReceivedImage`].
//! 2. [`store_entity_image`] waits for the process's one optimiser permit
//!    ([`UploadGate`], a semaphore of one: ADR 0015 §7), then, on a blocking
//!    thread, reads the staged upload, optimises it
//!    (`domain::image_optimise::optimise_image`), writes the result to a
//!    second staging file, `fsync`s it, renames it atomically into
//!    `<folder>/<32 hex>.<ext>` under `media.root`, and `fsync`s the folder,
//!    so the new name survives a crash. Only then does the URL exist.
//!
//! What it guarantees:
//!
//! - **No partial file is ever visible**, because everything is written in
//!   the staging directory, outside the served tree, and enters it by one
//!   rename on the same file system. Every staging file is removed when its
//!   upload ends, also when it fails or the request is dropped: the
//!   staging guard unlinks the file when it is dropped.
//! - **An early refusal writes nothing.** An upload refused for its name,
//!   its first bytes, its header's pixels or a missing root leaves no file
//!   and no directory on the volume, because the staging file is created
//!   only once the header has passed (ADR 0015 §7, "Image tests").
//! - **Nothing is overwritten.** The staging files are created with
//!   `O_EXCL` and the rename uses `RENAME_NOREPLACE`, so an existing name,
//!   a symbolic link included, fails the upload instead of being replaced.
//! - **No symbolic link under the root is followed.** Each folder below
//!   `media.root` is opened with `O_NOFOLLOW` relative to its parent, and the
//!   final name is a rename target, which a link cannot redirect. The three
//!   configured directories themselves are the operator's and are opened as
//!   configured.
//! - **The free-space reserve holds**, checked before the first byte, at
//!   every chunk, and before the optimised file is written. While the body
//!   arrives, the free space is read again every
//!   [`SPACE_RECHECK_BYTES`](crate::domain::media_rules::SPACE_RECHECK_BYTES), so
//!   concurrent uploads and other writers can undercut the reserve by at
//!   most that much each between two readings ([`SpaceWatch`]).
//! - **Modes do not depend on the umask.** A staging file is `0600`, a
//!   stored file `0644` and a folder that an upload creates `0755`, set with
//!   `fchmod` after it is created, because the media server reads them with
//!   the volume's group (ADR 0015 §3).
//! - **One optimisation at a time per process**, on a blocking thread, so
//!   the async runtime keeps serving other requests. The permit and the
//!   blocking work outlive a request that the client drops: the optimiser
//!   cannot be stopped half way, and its result still lands, as a file
//!   nothing refers to yet, which ADR 0011 §6 counts as harmless.
//! - **The directories are checked per upload**, never at boot, so the
//!   server starts before its volume is mounted (ADR 0015 §3). Without
//!   `media.root` every upload is refused before its body is read, with a
//!   message that names the key; `media.tmp_dir` and the entity folders
//!   below the root are created when missing. A folder that an upload
//!   creates is flushed into its parent with `fsync`, like the file renamed
//!   into it. Crash safety goes as far as `fsync` does on the volume's file
//!   system; no test can show it.
//!
//! Why not Loco's `ctx.storage` (`.claude/skills/loco/doctrine.md`, P1): its
//! disk driver (opendal's `fs` service) renames over an existing file,
//! follows symbolic links and has no free-space check, and ADR 0015 §6 asks
//! for the opposite of all three. backend3 configures no storage driver.
//!
//! The model writes no activity entry and no row: the endpoint stores the
//! returned [`StoredImage::stored_value`] on its entity, in its own
//! transaction, together with the entity's activity entry (ADR 0011). Files
//! are removed only by the owned-files worker, never here.

use std::{
    fs::File,
    io::{self, Read as _, Seek as _, SeekFrom, Write as _},
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{Stream, StreamExt as _};
use rustix::{
    fs::{self as at, AtFlags, Mode, OFlags, RenameFlags},
    io::Errno,
};
use tokio::{
    io::AsyncWriteExt as _,
    sync::{OwnedSemaphorePermit, Semaphore},
};

use crate::{
    domain::{
        image_optimise::{
            EarlyHeader, ImageError, ImageFormat, ImageHeader, NotReencoded, Treatment,
            inspect_start, optimise_image, sniff,
        },
        media_rules::{
            ENTITY_IMAGE_NAME_BYTES, EntityImageFolder, ImageExtension, MediaRuleError,
            SpaceShortfall, SpaceWatch, check_sniffed_type, entity_image_name, leaves_min_free,
            relative_segments, stored_image_path, stored_value,
        },
    },
    settings::{ImageOptimisationSettings, MEDIA_KEY, MediaSettings},
};

/// How many leading bytes decide the format: the longest signature, a WebP
/// RIFF header, is 12 bytes (`image_optimise::sniff`).
const SNIFF_BYTES: usize = 12;

/// How much of the start of an upload [`receive_image`] holds in memory,
/// unwritten, while the image header has not ended: 1 MiB.
///
/// The PNG, GIF and WebP headers end within their first 40 bytes. A JPEG
/// frame header follows the metadata segments, which cameras and editors
/// keep to some tens of KiB (an EXIF block with its thumbnail, an ICC
/// profile), so 1 MiB holds the header of any ordinary file. An upload whose
/// header has not ended within it is written on and judged whole by the
/// optimiser. Memory stays bounded: this much, plus one chunk, per upload
/// that is arriving.
pub const HEADER_WINDOW: usize = 1024 * 1024;

/// The mode of a staging file while it holds the upload: private to
/// backend3.
const STAGED_UPLOAD_MODE: u32 = 0o600;

/// The mode of the stored file, written before the rename because a rename
/// keeps it: readable by the media server, which runs with backend3's
/// namespace user id and the volume's group (ADR 0015 §3), and by nobody's
/// write but backend3's.
const STORED_FILE_MODE: u32 = 0o644;

/// The mode of an entity folder that an upload creates below the root, for
/// the same reason.
const FOLDER_MODE: u32 = 0o755;

/// The process's one permit to optimise an image (ADR 0015 §7: "A semaphore
/// allows one optimisation at a time per backend3 process").
///
/// `App::after_context` creates one per process and puts it into the shared
/// store; the controllers take it from there and pass it to
/// [`store_entity_image`]. Clones share the permit.
#[derive(Debug, Clone)]
pub struct UploadGate(Arc<Semaphore>);

impl UploadGate {
    /// A new gate with its one permit free.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Semaphore::new(1)))
    }

    /// Waits for the permit and returns it; it is free again when the
    /// returned value is dropped. [`store_entity_image`] takes it itself;
    /// tests take it to hold an optimisation back.
    ///
    /// # Errors
    ///
    /// [`UploadError::Optimiser`] if the semaphore was closed, which nothing
    /// in backend3 does.
    pub async fn acquire(&self) -> Result<UploadPermit, UploadError> {
        Arc::clone(&self.0)
            .acquire_owned()
            .await
            .map(|permit| UploadPermit { _permit: permit })
            .map_err(|_| UploadError::Optimiser("the upload gate is closed".to_owned()))
    }
}

impl Default for UploadGate {
    fn default() -> Self {
        Self::new()
    }
}

/// The optimiser permit of an [`UploadGate`], held while it lives.
#[derive(Debug)]
pub struct UploadPermit {
    /// The permit; returned to the gate when it is dropped.
    _permit: OwnedSemaphorePermit,
}

/// Why a chunk of the request body could not be read. The endpoint maps the
/// extractor's own error to it (`controllers::admin::media`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    /// The body passed the request body limit (`server.middlewares.
    /// limit_payload`, 128 MiB).
    #[error("the request body is larger than the limit")]
    TooLarge,
    /// The body ended early or is not well-formed.
    #[error("the request body could not be read: {0}")]
    Unreadable(String),
}

/// Why an upload is refused because of the file itself. The endpoint shows
/// the editor the Hungarian message of its error code
/// (`controllers::admin::media`, ADR 0021 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UploadRefusal {
    /// The name or the first bytes break a rule of the allow-list.
    #[error(transparent)]
    Rule(MediaRuleError),
    /// The optimiser refused the image: corrupt, above the pixel budget, or
    /// above one of the CPU limits of ADR 0033 §4.
    #[error("{error}")]
    Image {
        /// The extension of the file name, which a type mismatch names.
        extension: ImageExtension,
        /// The optimiser's reason.
        error: ImageError,
    },
}

/// Why an upload did not end with a stored file.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The file is refused; nothing was stored.
    #[error("the upload is refused: {0}")]
    Refused(UploadRefusal),
    /// The request body passed the request body limit.
    #[error("the request body is larger than the limit")]
    BodyTooLarge,
    /// The request is not one readable file part; the text says why.
    #[error("the request is not one file upload: {0}")]
    InvalidBody(String),
    /// Writing would leave less than `media.min_free_bytes` free.
    #[error(
        "storing the upload needs {needed} more bytes, and {available} are free; \
         settings.media.min_free_bytes keeps {min_free} free"
    )]
    InsufficientSpace {
        /// The bytes free when the check ran.
        available: u64,
        /// The bytes the upload was about to have written.
        needed: u64,
        /// `media.min_free_bytes`.
        min_free: u64,
    },
    /// A file-system call failed; the message names the step, the setting
    /// and the path, for the operator.
    #[error("{action} ({key}: {}): {source}", .path.display())]
    Storage {
        /// What the model was doing.
        action: &'static str,
        /// The full key of the setting the path comes from.
        key: &'static str,
        /// The path that failed.
        path: PathBuf,
        /// The operating system's error.
        source: io::Error,
    },
    /// The work on a blocking thread (the optimisation, or a file-system
    /// step) did not finish: it panicked.
    #[error("the blocking work of the upload did not finish: {0}")]
    Optimiser(String),
    /// The operating system's random source failed.
    #[error("the operating system's random source failed: {0}")]
    Random(String),
}

impl From<BodyError> for UploadError {
    fn from(error: BodyError) -> Self {
        match error {
            BodyError::TooLarge => Self::BodyTooLarge,
            BodyError::Unreadable(reason) => Self::InvalidBody(reason),
        }
    }
}

impl From<SpaceShortfall> for UploadError {
    fn from(shortfall: SpaceShortfall) -> Self {
        Self::InsufficientSpace {
            available: shortfall.available,
            needed: shortfall.needed,
            min_free: shortfall.min_free,
        }
    }
}

/// The full key of [`MediaSettings::root`], for messages.
const ROOT_KEY: &str = "settings.media.root";
/// The full key of [`MediaSettings::tmp_dir`], for messages.
const TMP_DIR_KEY: &str = "settings.media.tmp_dir";

/// A file in the staging directory that is removed when this guard is
/// dropped, unless it was renamed into place first.
#[derive(Debug)]
struct Staged {
    /// The staging directory, opened once per upload.
    dir: Arc<OwnedFd>,
    /// The file's name in it.
    name: String,
    /// The file's path, for messages only.
    path: PathBuf,
    /// Whether the file still has to be removed.
    armed: bool,
}

impl Staged {
    /// Marks the file as moved away, so the guard leaves its name alone.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Staged {
    /// Removes the staging file. This is one `unlinkat`, which does not
    /// block in practice, so it may run on the async runtime when a request
    /// is dropped.
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        match at::unlinkat(&*self.dir, self.name.as_str(), AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => {}
            Err(error) => tracing::warn!(
                path = %self.path.display(),
                error = %io::Error::from(error),
                "could not remove an upload staging file; it holds no served data"
            ),
        }
    }
}

/// An upload that has arrived completely and was flushed to the staging
/// directory. Its name and first bytes passed the allow-list, and its header
/// is within the pixel budget, or ends beyond [`HEADER_WINDOW`] and is left
/// to the optimiser. Dropping it removes the staging file.
#[derive(Debug)]
pub struct ReceivedImage {
    /// The media root, opened before the body was read; the store step
    /// writes below this directory.
    root: Arc<OwnedFd>,
    /// The staging file's guard.
    staged: Staged,
    /// The staging file, open for reading.
    file: File,
    /// The extension of the client's file name.
    extension: ImageExtension,
    /// The format the first bytes name, which agrees with `extension`.
    format: ImageFormat,
    /// How many bytes arrived.
    received_bytes: u64,
}

impl ReceivedImage {
    /// The extension of the client's file name, lower-cased; the stored name
    /// keeps it.
    #[must_use]
    pub const fn extension(&self) -> ImageExtension {
        self.extension
    }

    /// The format that the upload's magic bytes name.
    #[must_use]
    pub const fn format(&self) -> ImageFormat {
        self.format
    }

    /// How many bytes the upload has.
    #[must_use]
    pub const fn received_bytes(&self) -> u64 {
        self.received_bytes
    }
}

/// An entity image in place under `media.root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredImage {
    /// The value the entity's row stores: the path below the folder's disk,
    /// `<32 hex>.<ext>`, or `index/<32 hex>.<ext>` for a post's index image,
    /// the shapes of the legacy rows (`media_rules::stored_value`,
    /// data-migration.md §6.1, §6.4 and §6.5). The public API's URL is
    /// `{public_base_url}/{disk}/{stored_value}` (public-api.md §2.3).
    pub stored_value: String,
    /// The path below `media.root`, `<disk>/<stored_value>`: where the file
    /// is, and the end of the root-relative `/storage/<path>` that the admin
    /// API sends (ADR 0015 §3). Never stored in a row.
    pub path: String,
    /// The format and the stored width and height, which optimising never
    /// changes (ADR 0015 §7).
    pub header: ImageHeader,
    /// How many bytes were uploaded.
    pub received_bytes: u64,
    /// How many bytes were stored; never more than were uploaded.
    pub stored_bytes: u64,
    /// What the optimiser did.
    pub treatment: Treatment,
    /// How long the optimisation took, not counting the wait for the permit
    /// or the writes; for the upload metrics.
    pub optimise_time: Duration,
}

/// 16 bytes from the operating system's random source, for
/// [`store_entity_image`]'s `name_bytes` (ADR 0015 §6: entity images get
/// 32 hex digits as their name).
///
/// # Errors
///
/// [`UploadError::Random`] when the random source fails.
pub fn random_name_bytes() -> Result<[u8; ENTITY_IMAGE_NAME_BYTES], UploadError> {
    let mut bytes = [0; ENTITY_IMAGE_NAME_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| UploadError::Random(error.to_string()))?;
    Ok(bytes)
}

/// Streams one uploaded file into the staging directory (step 1 of the
/// module documentation).
///
/// - `media`: the media settings; `root` is opened, `tmp_dir` is written to
///   and `min_free_bytes` is kept free.
/// - `images`: the image settings; only the pixel budget is used here, for
///   the refusal from the header.
/// - `file_name`: the name the client sent with the file, `None` when it sent
///   none; only its extension is read.
/// - `body`: the file's bytes, chunk by chunk, as the request delivers them
///   (an axum multipart field, mapped to [`BodyError`] by the endpoint).
///
/// # Errors
///
/// - [`UploadError::Refused`] with [`MediaRuleError::ExtensionNotAllowed`]
///   before anything is read or opened; with
///   [`MediaRuleError::UnrecognisedContent`] or
///   [`MediaRuleError::TypeMismatch`] as soon as the first 12 bytes (or the
///   whole of a shorter file) have arrived; and with
///   [`ImageError::OverPixelBudget`] as soon as the image header has
///   arrived and declares more pixels than the budget;
/// - [`UploadError::Storage`] naming `settings.media.root` before the body
///   is read, when the root does not exist or cannot be opened;
/// - [`UploadError::BodyTooLarge`] or [`UploadError::InvalidBody`] when a
///   chunk cannot be read;
/// - [`UploadError::InsufficientSpace`] before the first byte, or at the
///   chunk that would leave less than `media.min_free_bytes` free;
/// - [`UploadError::Storage`] when the staging directory cannot be created
///   or written or the free space cannot be read, and
///   [`UploadError::Random`] when no staging name can be drawn.
///
/// After any error the staging file is gone, and after a refusal before the
/// header passed none was created.
pub async fn receive_image<S, B>(
    media: &MediaSettings,
    images: &ImageOptimisationSettings,
    file_name: Option<&str>,
    body: S,
) -> Result<ReceivedImage, UploadError>
where
    S: Stream<Item = Result<B, BodyError>>,
    B: AsRef<[u8]>,
{
    let extension = ImageExtension::from_file_name(file_name.unwrap_or(""))
        .map_err(|rule| UploadError::Refused(UploadRefusal::Rule(rule)))?;
    // The root first: while the volume is not mounted, the upload is
    // refused before its body is read, and nothing is created where the
    // volume belongs (ADR 0015 §3). Its file system is the staging
    // directory's (the rename into place needs that), so its free space is
    // the one to keep.
    let settings = media.clone();
    let (root, available) = blocking(move || {
        let root = open_root(&settings)?;
        let available = available_bytes(&root, ROOT_KEY, &settings.root)?;
        Ok((Arc::new(root), available))
    })
    .await?;
    let mut space = SpaceWatch::new(available, media.min_free_bytes)?;

    // The start of the body stays in memory until the header decides, so
    // an upload refused from it writes nothing (`HEADER_WINDOW`).
    let mut head: Vec<u8> = Vec::new();
    let mut format = None;
    let mut staging: Option<Staging> = None;
    let mut body = std::pin::pin!(body);
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        let bytes = chunk.as_ref();
        if bytes.is_empty() {
            continue;
        }
        space.receive(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
        if space.wants_reading() {
            space.record_reading(read_free_space(&root, &media.root).await?)?;
        }
        if let Some(staging) = staging.as_mut() {
            staging.write(bytes).await?;
            continue;
        }
        head.extend_from_slice(bytes);
        if format.is_none() && head.len() >= SNIFF_BYTES {
            format = Some(sniffed(extension, &head[..SNIFF_BYTES])?);
        }
        if format.is_some() {
            match inspect_start(&head, images.pixel_budget) {
                EarlyHeader::OverBudget(error) => {
                    return Err(UploadError::Refused(UploadRefusal::Image {
                        extension,
                        error,
                    }));
                }
                EarlyHeader::Incomplete if head.len() < HEADER_WINDOW => {}
                EarlyHeader::Incomplete | EarlyHeader::Undecided => {
                    staging = Some(Staging::start(media, &head).await?);
                    head = Vec::new();
                }
            }
        }
    }
    // The body has ended. A file shorter than 12 bytes is judged by all of
    // it. A header that is still incomplete was checked with every byte
    // that arrived, and is the optimiser's to refuse as cut short.
    let format = match format {
        Some(format) => format,
        None => sniffed(extension, &head)?,
    };
    let staging = match staging {
        Some(staging) => staging,
        None => Staging::start(media, &head).await?,
    };
    let (file, staged) = staging.finish().await?;
    Ok(ReceivedImage {
        root,
        staged,
        file,
        extension,
        format,
        received_bytes: space.received(),
    })
}

/// The staging file of an upload that is arriving, open for writing; created
/// once the upload's header has passed.
#[derive(Debug)]
struct Staging {
    /// The file.
    file: tokio::fs::File,
    /// Its guard, which removes it unless the upload is stored.
    staged: Staged,
}

impl Staging {
    /// Creates a staging file, and the staging directory when it is
    /// missing, and writes the bytes that arrived so far to it.
    ///
    /// - `media`: the media settings; `tmp_dir` is used.
    /// - `head`: the start of the upload, held until now.
    async fn start(media: &MediaSettings, head: &[u8]) -> Result<Self, UploadError> {
        let name = format!("upload-{}.part", hex(random_name_bytes()?));
        let settings = media.clone();
        let (file, staged) = blocking(move || {
            let dir = Arc::new(open_tmp_dir(&settings)?);
            create_staging_file(&dir, &settings.tmp_dir, name, STAGED_UPLOAD_MODE)
        })
        .await?;
        let mut staging = Self {
            file: tokio::fs::File::from_std(file),
            staged,
        };
        staging.write(head).await?;
        Ok(staging)
    }

    /// Appends `bytes` to the file.
    ///
    /// - `bytes`: the next bytes of the upload.
    async fn write(&mut self, bytes: &[u8]) -> Result<(), UploadError> {
        let written = self.file.write_all(bytes).await;
        written.map_err(|source| staging_write_error(&self.staged.path, source))
    }

    /// Flushes the file to the disk and hands it back, open for reading,
    /// with its guard.
    async fn finish(mut self) -> Result<(File, Staged), UploadError> {
        let flushed = match self.file.flush().await {
            Ok(()) => self.file.sync_all().await,
            Err(error) => Err(error),
        };
        flushed.map_err(|source| staging_write_error(&self.staged.path, source))?;
        Ok((self.file.into_std().await, self.staged))
    }
}

/// The error of a failed write to an upload's staging file.
///
/// - `path`: the staging file.
/// - `source`: the operating system's error.
fn staging_write_error(path: &Path, source: io::Error) -> UploadError {
    UploadError::Storage {
        action: "cannot write the upload to the staging directory",
        key: TMP_DIR_KEY,
        path: path.to_owned(),
        source,
    }
}

/// Optimises a received upload and puts it into place as a new entity image
/// (step 2 of the module documentation).
///
/// - `received`: the upload from [`receive_image`]; its staging file is
///   removed whatever happens.
/// - `folder`: the entity's image folder below `media.root`.
/// - `name_bytes`: the 16 random bytes of the new name, from
///   [`random_name_bytes`]. An argument, so tests can choose a name.
/// - `media`: the media settings (`root` and `tmp_dir` for messages, and
///   `min_free_bytes`).
/// - `images`: the pixel budget and the PNG effort for the optimiser.
/// - `gate`: the process's optimiser permit; the call waits for it.
///
/// # Errors
///
/// - [`UploadError::Refused`] with the optimiser's [`ImageError`] when the
///   image is corrupt, above the pixel budget or above a CPU limit;
/// - [`UploadError::InsufficientSpace`] when the optimised file would leave
///   less than `media.min_free_bytes` free;
/// - [`UploadError::Storage`] when a folder below the root is a symbolic
///   link or not a directory, a file of the new name exists already, the
///   staging directory is on another file system than the root, or a read,
///   write or `fsync` fails;
/// - [`UploadError::Optimiser`] when the blocking thread panicked.
///
/// After any error no new file is under `media.root` and no staging file
/// is left, except after a failed `fsync` of the folder: the file is in
/// place then, but its name may not survive a crash, so the call fails and
/// the file stays unreferenced.
pub async fn store_entity_image(
    received: ReceivedImage,
    folder: EntityImageFolder,
    name_bytes: [u8; ENTITY_IMAGE_NAME_BYTES],
    media: &MediaSettings,
    images: &ImageOptimisationSettings,
    gate: &UploadGate,
) -> Result<StoredImage, UploadError> {
    let name = entity_image_name(name_bytes, received.extension);
    let path = stored_image_path(folder, &name);
    // The folder's segments are constants and the name is generated, so the
    // path always passes; the check keeps the rule in one place should a
    // later caller build paths from input.
    relative_segments(&path).map_err(|rule| UploadError::Storage {
        action: "refusing a path that would leave the media root",
        key: ROOT_KEY,
        path: PathBuf::from(&path),
        source: io::Error::new(io::ErrorKind::InvalidInput, rule),
    })?;
    let permit = gate.acquire().await?;
    let media = media.clone();
    let images = *images;
    blocking(move || {
        // Held until the work below ends, also when the request is dropped
        // meanwhile: the optimisation runs to its end either way.
        let _permit = permit;
        store_blocking(received, folder, &name, path, &media, &images)
    })
    .await
}

/// The blocking part of [`store_entity_image`], on its thread, with the
/// permit held. The parameters are those of [`store_entity_image`], with
/// the new file's `name` and its `path` below the root.
fn store_blocking(
    mut received: ReceivedImage,
    folder: EntityImageFolder,
    name: &str,
    path: String,
    media: &MediaSettings,
    images: &ImageOptimisationSettings,
) -> Result<StoredImage, UploadError> {
    let root = Arc::clone(&received.root);
    let folder_dir = open_folder(&root, folder, media)?;

    // The optimisers take the whole image in memory; ADR 0015 §7 bounds
    // that by holding one at a time.
    let read_error = |source| UploadError::Storage {
        action: "cannot read the staged upload back",
        key: TMP_DIR_KEY,
        path: received.staged.path.clone(),
        source,
    };
    let mut bytes = Vec::with_capacity(usize::try_from(received.received_bytes).unwrap_or(0));
    received.file.seek(SeekFrom::Start(0)).map_err(read_error)?;
    received.file.read_to_end(&mut bytes).map_err(read_error)?;

    let started = Instant::now();
    let optimised = optimise_image(&bytes, images.pixel_budget, images.png).map_err(|error| {
        UploadError::Refused(UploadRefusal::Image {
            extension: received.extension,
            error,
        })
    })?;
    let optimise_time = started.elapsed();
    drop(bytes);
    if optimised.treatment == Treatment::ReEncodeDiscarded(NotReencoded::EncoderFailed) {
        // The stripped original is stored, which is a correct result
        // (ADR 0015 §7); the encoder's reason (libwebp gives one, MozJPEG
        // none) is only for the operator.
        tracing::warn!(
            format = %optimised.header.format,
            failure = optimised.re_encode_failure.as_deref().unwrap_or("no reason given"),
            "the re-encode of an upload failed; its stripped original is stored"
        );
    }

    let stored_bytes = u64::try_from(optimised.bytes.len()).unwrap_or(u64::MAX);
    let available = available_bytes(&folder_dir, ROOT_KEY, &media.root)?;
    check_space(available, stored_bytes, media.min_free_bytes)?;

    let staging_dir = Arc::clone(&received.staged.dir);
    let (mut out, mut out_staged) = create_staging_file(
        &staging_dir,
        &media.tmp_dir,
        format!("store-{name}"),
        STORED_FILE_MODE,
    )?;
    let write_error = |source| UploadError::Storage {
        action: "cannot write the optimised image to the staging directory",
        key: TMP_DIR_KEY,
        path: out_staged.path.clone(),
        source,
    };
    out.write_all(&optimised.bytes).map_err(write_error)?;
    out.sync_all().map_err(write_error)?;
    drop(out);

    let target = media.root.join(&path);
    at::renameat_with(
        &*staging_dir,
        out_staged.name.as_str(),
        &folder_dir,
        name,
        RenameFlags::NOREPLACE,
    )
    .map_err(|errno| UploadError::Storage {
        action: rename_failure(errno),
        key: ROOT_KEY,
        path: target.clone(),
        source: errno.into(),
    })?;
    out_staged.disarm();
    at::fsync(&folder_dir).map_err(|errno| UploadError::Storage {
        action: "the image is in place, but its folder could not be flushed",
        key: ROOT_KEY,
        path: target,
        source: errno.into(),
    })?;

    Ok(StoredImage {
        stored_value: stored_value(folder, name),
        path,
        header: optimised.header,
        received_bytes: received.received_bytes,
        stored_bytes,
        treatment: optimised.treatment,
        optimise_time,
    })
}

/// What a failed rename into place means, for the operator.
///
/// - `errno`: the error of `renameat2`.
fn rename_failure(errno: Errno) -> &'static str {
    match errno {
        Errno::EXIST => "a file of the new name exists already, and an upload never replaces one",
        Errno::XDEV => {
            "settings.media.tmp_dir is on another file system than settings.media.root, so the \
             image cannot be renamed into place atomically; put both on the volume"
        }
        Errno::INVAL => {
            "the media file system does not support renaming without replacing \
             (RENAME_NOREPLACE)"
        }
        _ => "cannot rename the optimised image into place",
    }
}

/// Runs `work` on a blocking thread and returns its result (ADR 0015 §7:
/// "on a blocking thread"). `tokio::task::spawn_blocking` is the runtime's
/// own pool for blocking calls; it is not the `tokio::spawn` that ADR 0016
/// keeps background work away from, because the request awaits it.
///
/// - `work`: the blocking calls.
///
/// # Errors
///
/// The error of `work`, or [`UploadError::Optimiser`] when it panicked.
async fn blocking<T, F>(work: F) -> Result<T, UploadError>
where
    F: FnOnce() -> Result<T, UploadError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| UploadError::Optimiser(error.to_string()))?
}

/// The format of an upload from its first bytes, refused unless it agrees
/// with `extension`.
///
/// - `extension`: the file name's extension.
/// - `prefix`: the first bytes of the upload, at most [`SNIFF_BYTES`].
fn sniffed(extension: ImageExtension, prefix: &[u8]) -> Result<ImageFormat, UploadError> {
    check_sniffed_type(extension, sniff(prefix))
        .map_err(|rule| UploadError::Refused(UploadRefusal::Rule(rule)))
}

/// Refuses a write of `needed` bytes that would leave less than `min_free`
/// of `available` free.
///
/// - `available`, `needed`, `min_free`: as `leaves_min_free` takes them.
fn check_space(available: u64, needed: u64, min_free: u64) -> Result<(), UploadError> {
    if leaves_min_free(available, needed, min_free) {
        Ok(())
    } else {
        Err(UploadError::InsufficientSpace {
            available,
            needed,
            min_free,
        })
    }
}

/// The bytes an unprivileged process may still write on the file system of
/// `dir`.
///
/// - `dir`: an open directory on that file system.
/// - `key`, `path`: the setting and path it was opened from, for messages.
fn available_bytes(dir: &OwnedFd, key: &'static str, path: &Path) -> Result<u64, UploadError> {
    let stat = at::fstatvfs(dir).map_err(|errno| UploadError::Storage {
        action: "cannot read the free space of the media volume",
        key,
        path: path.to_owned(),
        source: errno.into(),
    })?;
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// Reads the free space of the media volume again, while a body arrives, on
/// a blocking thread like every other file-system call of an upload.
///
/// - `root`: the open media root.
/// - `root_path`: its configured path, for messages.
async fn read_free_space(root: &Arc<OwnedFd>, root_path: &Path) -> Result<u64, UploadError> {
    let root = Arc::clone(root);
    let path = root_path.to_owned();
    blocking(move || available_bytes(&root, ROOT_KEY, &path)).await
}

/// Opens the staging directory, creating it and its parents when missing.
///
/// - `media`: the media settings; `tmp_dir` is used.
fn open_tmp_dir(media: &MediaSettings) -> Result<OwnedFd, UploadError> {
    let failed = |action| {
        move |source| UploadError::Storage {
            action,
            key: TMP_DIR_KEY,
            path: media.tmp_dir.clone(),
            source,
        }
    };
    std::fs::create_dir_all(&media.tmp_dir)
        .map_err(failed("cannot create the upload staging directory"))?;
    at::open(
        &media.tmp_dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| failed("cannot open the upload staging directory")(errno.into()))
}

/// Opens the media root, which must exist: without it the volume is not
/// mounted, and nothing may be written in its place.
///
/// - `media`: the media settings; `root` is used.
fn open_root(media: &MediaSettings) -> Result<OwnedFd, UploadError> {
    at::open(
        &media.root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| UploadError::Storage {
        action: if errno == Errno::NOENT {
            "the media root does not exist; mount the media volume there, or create the \
             directory in a local environment"
        } else {
            "cannot open the media root"
        },
        key: ROOT_KEY,
        path: media.root.clone(),
        source: errno.into(),
    })
}

/// Opens an entity folder below the root, one segment at a time, never
/// following a symbolic link, and creates the segments that are missing.
///
/// - `root`: the open media root.
/// - `folder`: the entity folder.
/// - `media`: the media settings, for messages.
fn open_folder(
    root: &OwnedFd,
    folder: EntityImageFolder,
    media: &MediaSettings,
) -> Result<OwnedFd, UploadError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut path = media.root.clone();
    let mut dir: Option<OwnedFd> = None;
    for segment in folder.segments() {
        path.push(segment);
        let failed = |action, errno: Errno| UploadError::Storage {
            action,
            key: ROOT_KEY,
            path: path.clone(),
            source: errno.into(),
        };
        let parent = dir.as_ref().unwrap_or(root);
        let opened = match at::openat(parent, *segment, flags, Mode::empty()) {
            Err(Errno::NOENT) => {
                match at::mkdirat(parent, *segment, Mode::from_raw_mode(FOLDER_MODE)) {
                    Ok(()) => {
                        let created = at::openat(parent, *segment, flags, Mode::empty());
                        if let Ok(created) = &created {
                            settle_new_folder(parent, created)
                                .map_err(|(action, errno)| failed(action, errno))?;
                        }
                        created
                    }
                    // Another upload created it meanwhile, and settles it.
                    Err(Errno::EXIST) => at::openat(parent, *segment, flags, Mode::empty()),
                    Err(errno) => {
                        return Err(failed("cannot create an entity image folder", errno));
                    }
                }
            }
            other => other,
        };
        dir = Some(opened.map_err(|errno| {
            failed(
                if matches!(errno, Errno::LOOP | Errno::NOTDIR) {
                    "an entity image folder is a symbolic link or not a directory; uploads \
                     never follow a symbolic link below the media root"
                } else {
                    "cannot open an entity image folder"
                },
                errno,
            )
        })?);
    }
    dir.ok_or_else(|| UploadError::Storage {
        action: "an entity image folder has no path",
        key: MEDIA_KEY,
        path,
        source: io::Error::from(io::ErrorKind::InvalidInput),
    })
}

/// Gives a folder that an upload has just created its mode, and flushes its
/// entry in the parent to the disk, so the folder survives a crash together
/// with the file that is renamed into it (whose own entry the folder's
/// `fsync` flushes).
///
/// - `parent`: the open parent directory.
/// - `created`: the new folder, opened without following a link.
///
/// # Errors
///
/// What failed and the operating system's error.
fn settle_new_folder(parent: &OwnedFd, created: &OwnedFd) -> Result<(), (&'static str, Errno)> {
    // `mkdirat` applies the process's umask, as `openat` does to files.
    at::fchmod(created, Mode::from_raw_mode(FOLDER_MODE))
        .map_err(|errno| ("cannot set the mode of a new entity image folder", errno))?;
    at::fsync(parent).map_err(|errno| {
        (
            "a new entity image folder could not be flushed into its parent",
            errno,
        )
    })
}

/// Creates a new file in the staging directory, never over an existing name
/// or through a symbolic link, and returns it with its guard.
///
/// - `dir`: the open staging directory.
/// - `tmp_dir`: its configured path, for messages.
/// - `name`: the file's name, unique per upload.
/// - `mode`: its permission bits.
fn create_staging_file(
    dir: &Arc<OwnedFd>,
    tmp_dir: &Path,
    name: String,
    mode: u32,
) -> Result<(File, Staged), UploadError> {
    let path = tmp_dir.join(&name);
    let fd = at::openat(
        &**dir,
        name.as_str(),
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(mode),
    )
    .map_err(|errno| UploadError::Storage {
        action: "cannot create a file in the upload staging directory",
        key: TMP_DIR_KEY,
        path: path.clone(),
        source: errno.into(),
    })?;
    let staged = Staged {
        dir: Arc::clone(dir),
        name,
        path,
        armed: true,
    };
    // `openat` applies the process's umask to the mode; set it exactly, so
    // the stored file's mode does not depend on how the process started.
    // The guard exists already, so a failure here removes the file.
    at::fchmod(&fd, Mode::from_raw_mode(mode)).map_err(|errno| UploadError::Storage {
        action: "cannot set the mode of a file in the upload staging directory",
        key: TMP_DIR_KEY,
        path: staged.path.clone(),
        source: errno.into(),
    })?;
    Ok((File::from(fd), staged))
}

/// The bytes as lower-case hex digits.
///
/// - `bytes`: the bytes to write.
fn hex(bytes: [u8; ENTITY_IMAGE_NAME_BYTES]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(2 * bytes.len()), |mut text, byte| {
            // Writing to a String cannot fail.
            let _ = write!(text, "{byte:02x}");
            text
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_writes_two_lower_case_digits_per_byte() {
        let mut bytes = [0; ENTITY_IMAGE_NAME_BYTES];
        bytes[0] = 0xAB;
        bytes[15] = 0x01;
        assert_eq!(hex(bytes), format!("ab{}01", "0".repeat(28)));
    }

    #[test]
    fn a_body_error_maps_to_its_upload_error() {
        assert!(matches!(
            UploadError::from(BodyError::TooLarge),
            UploadError::BodyTooLarge
        ));
        assert!(matches!(
            UploadError::from(BodyError::Unreadable("cut".to_owned())),
            UploadError::InvalidBody(reason) if reason == "cut"
        ));
    }

    #[test]
    fn a_shortfall_maps_to_the_space_error_with_its_numbers() {
        let error = UploadError::from(SpaceShortfall {
            available: 10,
            needed: 5,
            min_free: 6,
        });
        assert!(matches!(
            error,
            UploadError::InsufficientSpace {
                available: 10,
                needed: 5,
                min_free: 6
            }
        ));
    }

    #[test]
    fn each_rename_failure_says_what_to_fix() {
        assert!(rename_failure(Errno::EXIST).contains("never replaces"));
        assert!(rename_failure(Errno::XDEV).contains("settings.media.tmp_dir"));
        assert!(rename_failure(Errno::INVAL).contains("RENAME_NOREPLACE"));
        assert_eq!(
            rename_failure(Errno::IO),
            "cannot rename the optimised image into place"
        );
    }

    #[test]
    fn the_space_check_reports_its_numbers() {
        assert!(check_space(10, 4, 6).is_ok());
        let error = check_space(10, 5, 6).expect_err("one byte short");
        assert!(matches!(
            error,
            UploadError::InsufficientSpace {
                available: 10,
                needed: 5,
                min_free: 6
            }
        ));
        assert!(
            error.to_string().contains("settings.media.min_free_bytes"),
            "{error}"
        );
    }

    #[test]
    fn random_names_differ() {
        assert_ne!(random_name_bytes().unwrap(), random_name_bytes().unwrap());
    }
}
