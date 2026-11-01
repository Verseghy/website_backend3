//! The upload model `models::media` against a real file system (ADR 0015
//! §6 and §7, "Upload tests").
//!
//! Every test gets a media volume of its own below `target/tmp`
//! ([`TempMedia`]) and feeds the model a plain stream of chunks, standing in
//! for the multipart field of a request. The tests cover the entity image
//! allow-list, a sniffed type that disagrees with the extension (an SVG
//! renamed `.png`), an image above the pixel budget refused from its header
//! with nothing written, metadata and GPS stripped from the stored file, the
//! stored value of each entity folder, the free-space refusal, a missing
//! root, symbolic links and existing names, file modes under a strict
//! umask, the one-at-a-time optimiser permit, and that no partial file is
//! left anywhere after an error: neither in the staging directory nor under
//! the root.
//!
//! The free space is read again while a body arrives
//! (`media_rules::SpaceWatch`), but no test here makes another writer fill
//! the volume in between: the volume is the developer's or CI's disk, whose
//! free space other processes change all the time, and copy-on-write file
//! systems report a new file's space late. The rule is tested on its own
//! in `src/domain/media_rules.rs`.

use std::{
    future::Future,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
    time::Duration,
};

use backend3::{
    domain::{
        image_optimise::{
            DecodeDefect, Dimensions, ImageError, ImageFormat, PixelBudget, PngEffort, Treatment,
            optimise_image, optimise_png,
        },
        media_rules::{
            ENTITY_IMAGE_NAME_BYTES, EntityImageFolder, ImageExtension, MediaRuleError,
            is_entity_image_name,
        },
    },
    models::media::{
        BodyError, HEADER_WINDOW, ReceivedImage, StoredImage, UploadError, UploadGate,
        UploadRefusal, random_name_bytes, receive_image, store_entity_image,
    },
    settings::{ImageOptimisationSettings, MediaSettings},
};
use futures_util::{Stream, StreamExt as _, stream};

use crate::support::{TempMedia, image_fixture};

/// The image settings of the committed configuration files.
fn images() -> ImageOptimisationSettings {
    ImageOptimisationSettings {
        pixel_budget: PixelBudget::DEFAULT,
        png: PngEffort::DEFAULT,
    }
}

/// `bytes` as a body stream of chunks of at most `chunk` bytes.
fn chunks(bytes: &[u8], chunk: usize) -> impl Stream<Item = Result<Vec<u8>, BodyError>> + use<> {
    let parts: Vec<Result<Vec<u8>, BodyError>> =
        bytes.chunks(chunk).map(|part| Ok(part.to_vec())).collect();
    stream::iter(parts)
}

/// A body stream that fails the test if it is ever read: proof that a
/// refusal came before the body.
fn never_read() -> impl Stream<Item = Result<Vec<u8>, BodyError>> {
    stream::poll_fn(|_| -> std::task::Poll<Option<Result<Vec<u8>, BodyError>>> {
        panic!("the body was read")
    })
}

/// Receives `bytes`, named `file_name`, in chunks of 7 bytes, so the
/// signature check has to collect its 12 bytes over two chunks.
async fn receive(
    media: &MediaSettings,
    file_name: &str,
    bytes: &[u8],
) -> Result<ReceivedImage, UploadError> {
    receive_image(media, &images(), Some(file_name), chunks(bytes, 7)).await
}

/// Receives and stores `bytes` as an author image with a random name.
async fn upload(
    volume: &TempMedia,
    file_name: &str,
    bytes: &[u8],
) -> Result<StoredImage, UploadError> {
    let received = receive(volume.settings(), file_name, bytes).await?;
    store_entity_image(
        received,
        EntityImageFolder::AuthorImages,
        random_name_bytes()?,
        volume.settings(),
        &images(),
        &UploadGate::new(),
    )
    .await
}

/// Asserts that nothing of an upload is left: the staging directory is
/// empty and no file is under the root.
fn assert_nothing_left(volume: &TempMedia, context: &str) {
    let settings = volume.settings();
    assert_eq!(
        TempMedia::files(&settings.tmp_dir),
        Vec::<std::path::PathBuf>::new(),
        "{context}: staging"
    );
    assert_eq!(
        TempMedia::files(&settings.root),
        Vec::<std::path::PathBuf>::new(),
        "{context}: root"
    );
}

/// Whether `bytes` hold the TIFF entry of the EXIF GPS IFD pointer (tag
/// 0x8825, type LONG, count 1), in either byte order.
fn has_gps_pointer(bytes: &[u8]) -> bool {
    let little = [0x25, 0x88, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00];
    let big = [0x88, 0x25, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01];
    bytes
        .windows(8)
        .any(|window| window == little || window == big)
}

/// Runs `future` and fails if it ends within `wait`; returns nothing.
async fn assert_pending<F: Future>(future: F, wait: Duration, context: &str) {
    assert!(
        tokio::time::timeout(wait, future).await.is_err(),
        "{context}: finished although it should wait"
    );
}

#[tokio::test]
async fn an_image_is_optimised_and_renamed_into_its_folder() {
    let volume = TempMedia::new();
    for (fixture, file_name, extension, treatment) in [
        ("jpeg/camera.jpg", "IMG_0001.JPG", ImageExtension::Jpg, None),
        (
            "png/metadata.png",
            "rajz.png",
            ImageExtension::Png,
            Some(Treatment::Recompressed),
        ),
        (
            "webp/lossy-metadata.webp",
            "kép.webp",
            ImageExtension::Webp,
            Some(Treatment::ReEncoded),
        ),
        (
            "gif/metadata.gif",
            "anim.gif",
            ImageExtension::Gif,
            Some(Treatment::Stripped),
        ),
        (
            "jpeg/progressive.jpg",
            "old.jfif",
            ImageExtension::Jfif,
            None,
        ),
    ] {
        let bytes = image_fixture(fixture);
        let stored = upload(&volume, file_name, &bytes).await.expect(fixture);
        let expected = optimise_image(&bytes, PixelBudget::DEFAULT, PngEffort::DEFAULT)
            .expect("the corpus optimises");

        // `authors_images/<32 hex>.<lower-cased extension>`, of which the
        // author's row stores the bare name, as the legacy rows do
        // (data-migration.md §6.4).
        let (folder, name) = stored.path.split_once('/').expect("a folder");
        assert_eq!(folder, "authors_images", "{fixture}");
        assert!(is_entity_image_name(name), "{fixture}: {name}");
        assert_eq!(stored.stored_value, name, "{fixture}");
        assert!(
            name.ends_with(&format!(".{}", extension.as_str())),
            "{fixture}"
        );

        // What is stored is the optimiser's output, byte for byte, with the
        // upload's format and dimensions; never larger than the upload.
        let on_disk = std::fs::read(volume.settings().root.join(&stored.path)).expect("stored");
        assert_eq!(on_disk, expected.bytes, "{fixture}");
        assert_eq!(stored.header, expected.header, "{fixture}");
        assert_eq!(
            stored.header.dimensions,
            Dimensions::new(67, 41).expect("non-zero")
        );
        assert_eq!(stored.treatment, expected.treatment, "{fixture}");
        if let Some(treatment) = treatment {
            assert_eq!(stored.treatment, treatment, "{fixture}");
        }
        assert_eq!(stored.received_bytes, bytes.len() as u64, "{fixture}");
        assert_eq!(stored.stored_bytes, on_disk.len() as u64, "{fixture}");
        assert!(stored.stored_bytes <= stored.received_bytes, "{fixture}");

        // The media server reads it with the volume's group (ADR 0015 §3).
        let mode = std::fs::metadata(volume.settings().root.join(&stored.path))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644, "{fixture}");
    }
    // Every staging file is gone once its upload is stored.
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<std::path::PathBuf>::new()
    );
}

#[tokio::test]
async fn gps_and_the_other_metadata_never_reach_the_volume() {
    // ADR 0015 §7: "Dropping GPS data matters on a school site with photos
    // of students." Each corpus file carries an invented GPS position.
    let volume = TempMedia::new();
    for (fixture, file_name) in [
        ("jpeg/camera.jpg", "camera.jpg"),
        ("png/metadata.png", "metadata.png"),
        ("webp/lossy-metadata.webp", "lossy.webp"),
        ("webp/lossless-metadata.webp", "lossless.webp"),
    ] {
        let bytes = image_fixture(fixture);
        assert!(has_gps_pointer(&bytes), "{fixture} carries a GPS position");
        let stored = upload(&volume, file_name, &bytes).await.expect(fixture);
        let on_disk = std::fs::read(volume.settings().root.join(&stored.path)).expect("stored");
        assert!(!has_gps_pointer(&on_disk), "{fixture}: GPS stored");
    }
}

#[tokio::test]
async fn every_entity_folder_is_created_below_the_root() {
    let volume = TempMedia::new();
    let gif = image_fixture("gif/still.gif");
    for folder in EntityImageFolder::ALL {
        let received = receive(volume.settings(), "a.gif", &gif)
            .await
            .expect("received");
        let stored = store_entity_image(
            received,
            folder,
            random_name_bytes().expect("random"),
            volume.settings(),
            &images(),
            &UploadGate::new(),
        )
        .await
        .expect("stored");
        assert!(
            stored.path.starts_with(&format!("{}/", folder.path())),
            "{folder:?}"
        );
        assert!(
            volume.settings().root.join(&stored.path).is_file(),
            "{folder:?}"
        );
        // The row's value has the legacy shape of its column
        // (data-migration.md §6.1, §6.4, §6.5), and the public URL rule
        // `{disk}/{stored value}` names the file that was written: the V7
        // check, root/disk/stored value exists, holds for it.
        let bare = match folder {
            EntityImageFolder::PostIndexImages => stored.stored_value.strip_prefix("index/"),
            EntityImageFolder::PostImages
            | EntityImageFolder::AuthorImages
            | EntityImageFolder::ColleagueImages => Some(stored.stored_value.as_str()),
        };
        assert!(
            bare.is_some_and(is_entity_image_name),
            "{folder:?}: {}",
            stored.stored_value
        );
        assert_eq!(
            stored.path,
            format!("{}/{}", folder.disk(), stored.stored_value)
        );
        assert!(
            volume
                .settings()
                .root
                .join(folder.disk())
                .join(&stored.stored_value)
                .is_file(),
            "{folder:?}"
        );
    }
    let mut stored = TempMedia::files(&volume.settings().root);
    for path in &mut stored {
        path.pop();
    }
    assert_eq!(
        stored
            .iter()
            .map(|path| path.to_str().expect("UTF-8"))
            .collect::<Vec<_>>(),
        [
            "authors_images",
            "colleagues_images",
            "posts_images",
            "posts_images/index"
        ]
    );
}

#[tokio::test]
async fn the_allow_list_refuses_by_name_before_the_body_is_read() {
    let volume = TempMedia::new();
    for file_name in [
        None,
        Some(""),
        Some("photo"),
        Some("drawing.svg"),
        Some("page.html"),
        Some("document.pdf"),
        Some("phone.heic"),
        Some("archive.zip"),
    ] {
        let refused = receive_image(volume.settings(), &images(), file_name, never_read())
            .await
            .expect_err("refused");
        assert!(
            matches!(
                refused,
                UploadError::Refused(UploadRefusal::Rule(MediaRuleError::ExtensionNotAllowed))
            ),
            "{file_name:?}: {refused}"
        );
    }
    // Nothing was created, not even the staging directory.
    assert!(!volume.settings().tmp_dir.exists());
    assert_nothing_left(&volume, "allow-list");
}

#[tokio::test]
async fn an_svg_renamed_to_png_is_refused_from_its_first_bytes() {
    let volume = TempMedia::new();
    for (content, file_name) in [
        (
            &b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>"[..],
            "logo.png",
        ),
        (
            b"<!DOCTYPE html><html><script>alert(1)</script></html>",
            "photo.jpg",
        ),
        (
            b"%PDF-1.7\n%\xE2\xE3\xCF\xD3 an invented document",
            "scan.webp",
        ),
    ] {
        // The first chunk holds the 12 bytes that decide; reading on would
        // fail the test.
        let body = stream::iter([Ok(content.to_vec())]).chain(never_read());
        let refused = receive_image(volume.settings(), &images(), Some(file_name), body)
            .await
            .expect_err("refused");
        assert!(
            matches!(
                refused,
                UploadError::Refused(UploadRefusal::Rule(MediaRuleError::UnrecognisedContent))
            ),
            "{file_name}: {refused}"
        );
        // Refused from bytes held in memory: nothing reached the volume.
        assert!(!volume.settings().tmp_dir.exists(), "{file_name}");
        assert_nothing_left(&volume, file_name);
    }
}

#[tokio::test]
async fn an_image_of_another_format_than_its_extension_is_refused() {
    let volume = TempMedia::new();
    for (fixture, file_name, extension, found) in [
        (
            "jpeg/baseline.jpg",
            "photo.png",
            ImageExtension::Png,
            ImageFormat::Jpeg,
        ),
        (
            "png/rgb.png",
            "photo.jpg",
            ImageExtension::Jpg,
            ImageFormat::Png,
        ),
        (
            "gif/still.gif",
            "anim.webp",
            ImageExtension::Webp,
            ImageFormat::Gif,
        ),
        (
            "webp/lossy.webp",
            "photo.jpeg",
            ImageExtension::Jpeg,
            ImageFormat::WebP,
        ),
    ] {
        let refused = receive(volume.settings(), file_name, &image_fixture(fixture))
            .await
            .expect_err("refused");
        let expected = MediaRuleError::TypeMismatch { extension, found };
        assert!(
            matches!(refused, UploadError::Refused(UploadRefusal::Rule(rule)) if rule == expected),
            "{fixture}: {refused}"
        );
        assert!(!volume.settings().tmp_dir.exists(), "{fixture}");
        assert_nothing_left(&volume, fixture);
    }
}

#[tokio::test]
async fn a_file_shorter_than_a_signature_is_judged_by_all_of_it() {
    let volume = TempMedia::new();
    // Six bytes of GIF signature: the type agrees, and the optimiser then
    // refuses the image as cut short. Three bytes of PNG signature are no
    // image at all.
    let received = receive(volume.settings(), "a.gif", b"GIF89a")
        .await
        .expect("the type agrees");
    assert_eq!(
        (
            received.format(),
            received.extension(),
            received.received_bytes()
        ),
        (ImageFormat::Gif, ImageExtension::Gif, 6)
    );
    let refused = store_entity_image(
        received,
        EntityImageFolder::PostImages,
        random_name_bytes().expect("random"),
        volume.settings(),
        &images(),
        &UploadGate::new(),
    )
    .await
    .expect_err("cut short");
    assert!(
        matches!(
            refused,
            UploadError::Refused(UploadRefusal::Image {
                error: ImageError::Truncated {
                    format: ImageFormat::Gif
                },
                ..
            })
        ),
        "{refused}"
    );
    let refused = receive(volume.settings(), "a.png", b"\x89PN")
        .await
        .expect_err("no image");
    assert!(matches!(
        refused,
        UploadError::Refused(UploadRefusal::Rule(MediaRuleError::UnrecognisedContent))
    ));
    let refused = receive(volume.settings(), "a.png", b"")
        .await
        .expect_err("empty");
    assert!(matches!(
        refused,
        UploadError::Refused(UploadRefusal::Rule(MediaRuleError::UnrecognisedContent))
    ));
    // The folder that the refused store opened stays empty.
    assert_eq!(
        TempMedia::files(&volume.settings().root),
        Vec::<std::path::PathBuf>::new()
    );
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<std::path::PathBuf>::new()
    );
}

#[tokio::test]
async fn no_partial_file_is_left_after_an_error() {
    let volume = TempMedia::new();
    let camera = image_fixture("jpeg/camera.jpg");

    // The body breaks off, or passes the request body limit, half way.
    for error in [
        BodyError::Unreadable("incomplete multipart stream".to_owned()),
        BodyError::TooLarge,
    ] {
        let too_large = error == BodyError::TooLarge;
        let body = chunks(&camera[..camera.len() / 2], 100).chain(stream::iter([Err(error)]));
        let refused = receive_image(volume.settings(), &images(), Some("camera.jpg"), body)
            .await
            .expect_err("refused");
        if too_large {
            assert!(matches!(refused, UploadError::BodyTooLarge), "{refused}");
        } else {
            assert!(matches!(refused, UploadError::InvalidBody(_)), "{refused}");
        }
        assert_nothing_left(&volume, "a broken body");
    }

    // The body arrives whole, but the image data is cut short inside its
    // scan: refused as corrupt by the optimiser, after the staging file was
    // written.
    let baseline = image_fixture("jpeg/baseline.jpg");
    let refused = upload(&volume, "baseline.jpg", &baseline[..baseline.len() - 50])
        .await
        .expect_err("corrupt");
    assert!(
        matches!(
            refused,
            UploadError::Refused(UploadRefusal::Image {
                error: ImageError::Undecodable {
                    format: ImageFormat::Jpeg,
                    defect: DecodeDefect::JpegCutShort
                },
                ..
            })
        ),
        "{refused}"
    );
    // A GIF with no image (ADR 0033 §3, D104), and a header above the
    // pixel budget, refused before any decode.
    let refused = upload(&volume, "empty.gif", b"GIF89a\x01\x00\x01\x00\x00\x00\x00;")
        .await
        .expect_err("no image");
    assert!(matches!(
        refused,
        UploadError::Refused(UploadRefusal::Image {
            error: ImageError::Undecodable {
                defect: DecodeDefect::GifNoImage,
                ..
            },
            ..
        })
    ));
    let refused = upload(
        &volume,
        "big.png",
        &image_fixture("over-budget/camera-50mp.png"),
    )
    .await
    .expect_err("over budget");
    assert!(matches!(
        refused,
        UploadError::Refused(UploadRefusal::Image {
            error: ImageError::OverPixelBudget { .. },
            ..
        })
    ));
    // The refused stores created the author folder, and nothing in it.
    assert_eq!(
        TempMedia::files(&volume.settings().root),
        Vec::<std::path::PathBuf>::new()
    );
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<std::path::PathBuf>::new()
    );

    // A request dropped while its body arrives leaves no staging file.
    let settings = images();
    let pending = receive_image(
        volume.settings(),
        &settings,
        Some("camera.jpg"),
        chunks(&camera[..100], 100).chain(stream::pending()),
    );
    assert_pending(pending, Duration::from_millis(200), "an endless body").await;
    assert_nothing_left(&volume, "a dropped request");
}

#[tokio::test]
async fn an_upload_is_refused_below_the_free_space_reserve() {
    let volume = TempMedia::new();
    let camera = image_fixture("jpeg/camera.jpg");

    // No reserve can be met: refused before the body is read.
    let refused = receive_image(
        &volume.settings_with_reserve(u64::MAX),
        &images(),
        Some("camera.jpg"),
        never_read(),
    )
    .await
    .expect_err("no space");
    assert!(
        matches!(
            refused,
            UploadError::InsufficientSpace {
                needed: 0,
                min_free: u64::MAX,
                ..
            }
        ),
        "{refused}"
    );
    assert!(
        refused
            .to_string()
            .contains("settings.media.min_free_bytes"),
        "{refused}"
    );
    assert_nothing_left(&volume, "before the body");

    // A reserve 4 MiB below what is free now: an endless body is refused at
    // the chunk that would cross it, whatever other tests write meanwhile.
    let free = {
        let stat = rustix::fs::statvfs(volume.base()).expect("statvfs");
        stat.f_bavail * stat.f_frsize
    };
    let reserve = free.saturating_sub(4 * 1024 * 1024);
    let endless = stream::iter([Ok(camera[..64].to_vec())])
        .chain(stream::repeat_with(|| Ok(vec![0_u8; 256 * 1024])));
    let refused = receive_image(
        &volume.settings_with_reserve(reserve),
        &images(),
        Some("camera.jpg"),
        endless,
    )
    .await
    .expect_err("no space");
    assert!(
        matches!(refused, UploadError::InsufficientSpace { min_free, .. } if min_free == reserve),
        "{refused}"
    );
    assert_nothing_left(&volume, "during the body");

    // The reserve is checked again before the optimised file is written.
    let received = receive(volume.settings(), "camera.jpg", &camera)
        .await
        .expect("received");
    let refused = store_entity_image(
        received,
        EntityImageFolder::AuthorImages,
        random_name_bytes().expect("random"),
        &volume.settings_with_reserve(u64::MAX),
        &images(),
        &UploadGate::new(),
    )
    .await
    .expect_err("no space");
    assert!(
        matches!(refused, UploadError::InsufficientSpace { .. }),
        "{refused}"
    );
    assert_eq!(
        TempMedia::files(&volume.settings().root),
        Vec::<std::path::PathBuf>::new()
    );
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<std::path::PathBuf>::new()
    );
}

#[tokio::test]
async fn a_symbolic_link_below_the_root_is_never_followed() {
    let volume = TempMedia::new();
    let outside = volume.base().join("outside");
    std::fs::create_dir(&outside).expect("outside");
    symlink(&outside, volume.settings().root.join("authors_images")).expect("symlink");

    let refused = upload(&volume, "camera.jpg", &image_fixture("jpeg/camera.jpg"))
        .await
        .expect_err("refused");
    match &refused {
        UploadError::Storage { key, .. } => assert_eq!(*key, "settings.media.root"),
        other => panic!("expected a storage error, got {other}"),
    }
    assert!(refused.to_string().contains("symbolic link"), "{refused}");
    assert_eq!(TempMedia::files(&outside), Vec::<std::path::PathBuf>::new());
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<std::path::PathBuf>::new()
    );
}

#[tokio::test]
async fn an_existing_name_is_never_overwritten() {
    let volume = TempMedia::new();
    let name = [0x5a_u8; ENTITY_IMAGE_NAME_BYTES];
    let store = |fixture: &'static str| {
        let settings = volume.settings().clone();
        async move {
            let received = receive(&settings, "photo.jpg", &image_fixture(fixture)).await?;
            store_entity_image(
                received,
                EntityImageFolder::ColleagueImages,
                name,
                &settings,
                &images(),
                &UploadGate::new(),
            )
            .await
        }
    };
    let first = store("jpeg/baseline.jpg").await.expect("stored");
    let path = volume.settings().root.join(&first.path);
    let before = std::fs::read(&path).expect("stored");

    let refused = store("jpeg/progressive.jpg")
        .await
        .expect_err("the name is taken");
    assert!(refused.to_string().contains("never replaces"), "{refused}");
    assert_eq!(std::fs::read(&path).expect("kept"), before);

    // A symbolic link of that name is not written through either.
    std::fs::remove_file(&path).expect("remove");
    let target = volume.base().join("target.jpg");
    symlink(&target, &path).expect("symlink");
    store("jpeg/progressive.jpg")
        .await
        .expect_err("the name is taken");
    assert!(!target.exists());
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<std::path::PathBuf>::new()
    );
}

/// Asserts that `refused` is the missing-root refusal, which names the key.
fn assert_no_root(refused: &UploadError) {
    match refused {
        UploadError::Storage { key, .. } => assert_eq!(*key, "settings.media.root"),
        other => panic!("expected the missing-root error, got {other}"),
    }
    let message = refused.to_string();
    assert!(message.contains("settings.media.root"), "{message}");
    assert!(message.contains("does not exist"), "{message}");
}

#[tokio::test]
async fn a_missing_root_refuses_every_upload_before_its_body_is_read() {
    let volume = TempMedia::new();

    // Production's layout before the volume is mounted (ADR 0015 §3): the
    // root and the staging directory both lie below the missing mount point.
    let mount = volume.base().join("data");
    let unmounted = MediaSettings {
        root: mount.join("app/public"),
        tmp_dir: mount.join("backend3/tmp"),
        trash_dir: mount.join("backend3/trash"),
        ..volume.settings().clone()
    };
    let refused = receive_image(&unmounted, &images(), Some("camera.jpg"), never_read())
        .await
        .expect_err("no root");
    assert_no_root(&refused);
    // Nothing was created where the volume belongs.
    assert!(!mount.exists());

    // The local layout, three siblings, with the root removed.
    std::fs::remove_dir(&volume.settings().root).expect("remove the root");
    let refused = receive_image(
        volume.settings(),
        &images(),
        Some("camera.jpg"),
        never_read(),
    )
    .await
    .expect_err("no root");
    assert_no_root(&refused);
    assert!(!volume.settings().root.exists());
    assert!(!volume.settings().tmp_dir.exists());
}

#[tokio::test]
async fn the_optimiser_settings_are_the_ones_passed() {
    let volume = TempMedia::new();
    // A budget below the corpus's 67x41 refuses it from its header.
    let tiny = ImageOptimisationSettings {
        pixel_budget: PixelBudget::new(2_000).expect("a budget"),
        png: PngEffort::DEFAULT,
    };
    let is_tiny_refusal = |refused: &UploadError| {
        matches!(
            refused,
            UploadError::Refused(UploadRefusal::Image {
                error: ImageError::OverPixelBudget { budget, .. },
                ..
            }) if *budget == tiny.pixel_budget
        )
    };
    // While the body arrives, the budget passed to the receiver refuses it.
    let refused = receive_image(
        volume.settings(),
        &tiny,
        Some("a.png"),
        chunks(&image_fixture("png/rgb.png"), 7),
    )
    .await
    .expect_err("over the budget");
    assert!(is_tiny_refusal(&refused), "{refused}");
    // Once it has arrived, the budget passed to the store refuses it.
    let received = receive(volume.settings(), "a.png", &image_fixture("png/rgb.png"))
        .await
        .expect("received");
    let refused = store_entity_image(
        received,
        EntityImageFolder::AuthorImages,
        random_name_bytes().expect("random"),
        volume.settings(),
        &tiny,
        &UploadGate::new(),
    )
    .await
    .expect_err("over the budget");
    assert!(is_tiny_refusal(&refused), "{refused}");

    // The PNG effort reaches oxipng: a threshold of one pixel selects the
    // cheaper preset for the corpus file, as `optimise_png` does with it.
    let cheap = ImageOptimisationSettings {
        pixel_budget: PixelBudget::DEFAULT,
        png: PngEffort::new(1, 0).expect("an effort"),
    };
    let png = image_fixture("png/rgba-interlaced.png");
    let received = receive(volume.settings(), "a.png", &png)
        .await
        .expect("received");
    let stored = store_entity_image(
        received,
        EntityImageFolder::AuthorImages,
        random_name_bytes().expect("random"),
        volume.settings(),
        &cheap,
        &UploadGate::new(),
    )
    .await
    .expect("stored");
    let on_disk = std::fs::read(volume.settings().root.join(&stored.path)).expect("stored");
    assert_eq!(
        on_disk,
        optimise_png(&png, PixelBudget::DEFAULT, cheap.png).expect("optimises")
    );
}

#[tokio::test]
async fn one_optimisation_runs_at_a_time() {
    let volume = TempMedia::new();
    let gate = UploadGate::new();
    let gif = image_fixture("gif/still.gif");
    let images = images();
    let store = |received| {
        store_entity_image(
            received,
            EntityImageFolder::PostImages,
            random_name_bytes().expect("random"),
            volume.settings(),
            &images,
            &gate,
        )
    };

    // While another upload holds the permit, a store waits for it; a
    // request dropped while waiting leaves nothing behind.
    let held = gate.acquire().await.expect("the permit");
    let received = receive(volume.settings(), "a.gif", &gif)
        .await
        .expect("received");
    assert_pending(
        store(received),
        Duration::from_millis(300),
        "a second optimisation",
    )
    .await;
    assert_nothing_left(&volume, "a store that waited");

    // Once the permit is free, the store runs, and holds the permit for as
    // long as its blocking work runs: polled once, it has taken the permit
    // and handed the work to a blocking thread, so another acquire waits,
    // and gets the permit only once the file is in place. (The work ends
    // with two `fsync`s, which take far longer than the two polls here.)
    drop(held);
    let received = receive(volume.settings(), "a.gif", &gif)
        .await
        .expect("received");
    let mut storing = std::pin::pin!(store(received));
    assert!(
        pending_after_one_poll(storing.as_mut()),
        "the store ended without waiting for its blocking work"
    );
    let mut waiting = std::pin::pin!(gate.acquire());
    assert!(
        pending_after_one_poll(waiting.as_mut()),
        "the permit is free while an optimisation runs"
    );
    let stored = storing.await.expect("stored");
    assert!(volume.settings().root.join(&stored.path).is_file());
    // The permit is free again right afterwards.
    let permit = tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .expect("the permit is free once the store has ended")
        .expect("the permit");
    drop(permit);
}

/// Polls `future` once, with a waker that does nothing, and says whether it
/// is still pending. A later `.await` polls it again with the task's waker.
///
/// - `future`: the pinned future to poll.
fn pending_after_one_poll<F: Future>(future: std::pin::Pin<&mut F>) -> bool {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    future.poll(&mut context).is_pending()
}

#[tokio::test]
async fn an_image_above_the_budget_is_refused_from_its_header_and_nothing_is_written() {
    // ADR 0015 §7, "Image tests": "rejected from its header dimensions,
    // before any decode, ... and nothing is written to the volume". Each
    // corpus file is a 50 MP camera's header, sent 7 bytes at a time; the
    // body after it must never be read.
    let volume = TempMedia::new();
    for (fixture, file_name, format) in [
        (
            "over-budget/camera-50mp.jpg",
            "camera.jpg",
            ImageFormat::Jpeg,
        ),
        (
            "over-budget/camera-50mp.png",
            "camera.png",
            ImageFormat::Png,
        ),
        (
            "over-budget/camera-50mp.gif",
            "camera.gif",
            ImageFormat::Gif,
        ),
        (
            "over-budget/camera-50mp.webp",
            "camera.webp",
            ImageFormat::WebP,
        ),
    ] {
        let body = chunks(&image_fixture(fixture), 7).chain(never_read());
        let refused = receive_image(volume.settings(), &images(), Some(file_name), body)
            .await
            .expect_err("over the budget");
        assert!(
            matches!(
                refused,
                UploadError::Refused(UploadRefusal::Image {
                    error: ImageError::OverPixelBudget { format: found, .. },
                    ..
                }) if found == format
            ),
            "{fixture}: {refused}"
        );
        // Not even the staging directory was created.
        assert!(!volume.settings().tmp_dir.exists(), "{fixture}");
        assert_nothing_left(&volume, fixture);
    }
}

#[tokio::test]
async fn a_header_beyond_the_window_is_judged_once_the_upload_is_in() {
    // A JPEG whose frame header follows more than HEADER_WINDOW of metadata
    // (17 APP15 segments of 64 KiB): the start is written on once the window
    // is full, the whole upload arrives, and the store refuses it from the
    // same header.
    let volume = TempMedia::new();
    let camera = image_fixture("over-budget/camera-50mp.jpg");
    let (soi, rest) = camera.split_at(2);
    assert_eq!(soi, [0xFF, 0xD8]);
    let mut jpeg = soi.to_vec();
    for _ in 0..17 {
        jpeg.extend([0xFF, 0xEF, 0xFF, 0xFF]);
        jpeg.extend(std::iter::repeat_n(0_u8, 0xFFFF - 2));
    }
    assert!(jpeg.len() > HEADER_WINDOW);
    jpeg.extend_from_slice(rest);

    let received = receive_image(
        volume.settings(),
        &images(),
        Some("camera.jpg"),
        chunks(&jpeg, 64 * 1024),
    )
    .await
    .expect("received: the header is beyond the window");
    assert_eq!(received.received_bytes(), jpeg.len() as u64);
    let refused = store_entity_image(
        received,
        EntityImageFolder::PostImages,
        random_name_bytes().expect("random"),
        volume.settings(),
        &images(),
        &UploadGate::new(),
    )
    .await
    .expect_err("over the budget");
    assert!(
        matches!(
            refused,
            UploadError::Refused(UploadRefusal::Image {
                error: ImageError::OverPixelBudget { .. },
                ..
            })
        ),
        "{refused}"
    );
    assert_nothing_left(&volume, "a header beyond the window");
}

/// Restores the process's umask when it is dropped.
struct RestoreUmask(rustix::fs::Mode);

impl Drop for RestoreUmask {
    fn drop(&mut self) {
        rustix::process::umask(self.0);
    }
}

/// The permission bits of `path`.
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[tokio::test]
async fn the_modes_do_not_depend_on_the_umask() {
    // A process started with a strict umask: without the `fchmod`s, the
    // stored file and the new folders would be private to backend3, and the
    // media server, which reads them with the volume's group, could not.
    // nextest runs each test in a process of its own, so no other test sees
    // this umask; it is restored at the end all the same.
    let _restore = RestoreUmask(rustix::process::umask(rustix::fs::Mode::from_raw_mode(
        0o077,
    )));
    let volume = TempMedia::new();
    let received = receive(volume.settings(), "a.gif", &image_fixture("gif/still.gif"))
        .await
        .expect("received");
    let staged = TempMedia::files(&volume.settings().tmp_dir);
    assert_eq!(staged.len(), 1, "{staged:?}");
    assert_eq!(mode(&volume.settings().tmp_dir.join(&staged[0])), 0o600);
    let stored = store_entity_image(
        received,
        EntityImageFolder::PostIndexImages,
        random_name_bytes().expect("random"),
        volume.settings(),
        &images(),
        &UploadGate::new(),
    )
    .await
    .expect("stored");
    let root = &volume.settings().root;
    assert_eq!(mode(&root.join(&stored.path)), 0o644);
    assert_eq!(mode(&root.join("posts_images")), 0o755);
    assert_eq!(mode(&root.join("posts_images/index")), 0o755);
}

#[test]
fn the_staging_directory_is_not_inside_the_root() {
    // TempMedia mirrors the production layout: three siblings.
    let volume = TempMedia::new();
    let settings = volume.settings();
    for dir in [&settings.tmp_dir, &settings.trash_dir] {
        assert!(!dir.starts_with(&settings.root), "{}", dir.display());
        assert_eq!(dir.parent(), Some(volume.base()));
    }
    assert!(Path::new(&settings.root).starts_with(volume.base()));
}
