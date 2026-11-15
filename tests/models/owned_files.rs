//! `models::owned_files`: the files a record names, what counts as a
//! reference to an owned file, and the removal of the files nothing refers
//! to (ADR 0011 §6, ADR 0016 §3).
//!
//! The references are checked on the test-only `owned_file_probes` entity
//! (`tests/support/owned_file_probes.rs`), whose three columns are
//! registered as an upload slice registers its own. The worker's tests
//! (`tests/workers/delete_owned_files.rs`) run the same removal through the
//! queue.

use std::path::Path;

use backend3::{
    domain::{
        media_rules::EntityImageFolder,
        owned_files::{FILE_OWNERS, FileOwner, FileOwnerError, OwnedPath},
        revision::{RevisionSnapshot, SnapshotObject, UpcastStep, UpcastStepError},
    },
    models::owned_files::{
        DeletionReport, OwnedFilesError, delete_unreferenced, files_of_record, is_referenced,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::support::{
    TempMedia,
    owned_file_probes::{
        self, OWNERS, ProbeRevision, TABLE, append_revision, insert, insert_raw_revision, new_name,
        path, stored,
    },
    with_app,
};

const AUTHORS: EntityImageFolder = EntityImageFolder::AuthorImages;
const GALLERY: EntityImageFolder = EntityImageFolder::PostImages;
const INDEX: EntityImageFolder = EntityImageFolder::PostIndexImages;

/// The probe's snapshot after a second shape: version 1 called the photo
/// `photo`, and version 2 names it like its column, `image`.
#[derive(Debug, Serialize, Deserialize)]
struct ProbeRevisionV2 {
    /// The stored value of `image`.
    image: Option<String>,
    /// The stored value of `index_image`.
    index_image: Option<String>,
    /// The stored values of `images`.
    images: Vec<String>,
}

/// The step from version 1 to 2: `photo` becomes `image`.
fn photo_to_image(mut v1: SnapshotObject) -> Result<SnapshotObject, UpcastStepError> {
    let photo = v1.remove("photo").ok_or(UpcastStepError::MissingField {
        field: "photo".to_owned(),
    })?;
    v1.insert("image".to_owned(), photo);
    Ok(v1)
}

impl RevisionSnapshot for ProbeRevisionV2 {
    const ENTITY_TYPE: &'static str = TABLE;
    const SCHEMA_VERSION: i32 = 2;
    const FIELDS: &'static [&'static str] = &["image", "index_image", "images"];
    const UPCASTS: &'static [UpcastStep] = &[UpcastStep {
        from: 1,
        apply: photo_to_image,
    }];
}

/// A probe snapshot that names the gallery `gallery`, not like its column.
#[derive(Debug, Serialize, Deserialize)]
struct RenamedGalleryRevision {
    /// The stored value of `image`.
    image: Option<String>,
    /// The stored value of `index_image`.
    index_image: Option<String>,
    /// The stored values of `images`, under another name.
    gallery: Vec<String>,
}

impl RevisionSnapshot for RenamedGalleryRevision {
    const ENTITY_TYPE: &'static str = TABLE;
    const SCHEMA_VERSION: i32 = 1;
    const FIELDS: &'static [&'static str] = &["image", "index_image", "gallery"];
}

/// The snapshot of an entity with no registered column.
#[derive(Debug, Serialize, Deserialize)]
struct OtherRevision {
    /// An invented field.
    image: Option<String>,
}

impl RevisionSnapshot for OtherRevision {
    const ENTITY_TYPE: &'static str = "other_entities";
    const SCHEMA_VERSION: i32 = 1;
    const FIELDS: &'static [&'static str] = &["image"];
}

/// A snapshot type that breaks `check_snapshot_type`: version 2 without its
/// upcast step.
#[derive(Debug, Serialize, Deserialize)]
struct NoStepRevision {
    /// The stored value of `image`.
    image: Option<String>,
}

impl RevisionSnapshot for NoStepRevision {
    const ENTITY_TYPE: &'static str = TABLE;
    const SCHEMA_VERSION: i32 = 2;
    const FIELDS: &'static [&'static str] = &["image"];
}

/// A registry whose entity name holds a quote, so it could carry SQL.
static BAD: [FileOwner; 1] = [FileOwner {
    entity: "owned_file_probes\"",
    column: "image",
    folder: EntityImageFolder::AuthorImages,
}];

/// A registry that names a column the probe table lacks.
static MISSING: [FileOwner; 1] = [FileOwner {
    entity: TABLE,
    column: "photo",
    folder: EntityImageFolder::AuthorImages,
}];

/// A registry that names a column the probe table lacks and
/// [`RenamedGalleryRevision`] lists, so only the database can refuse it.
static NO_GALLERY_COLUMN: [FileOwner; 1] = [FileOwner {
    entity: TABLE,
    column: "gallery",
    folder: EntityImageFolder::PostImages,
}];

/// Writes an invented file at `path` below `root`, with its folders.
fn write_file(root: &Path, path: &OwnedPath) {
    let file = root.join(path.path());
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("create the folder");
    std::fs::write(&file, b"invented image bytes").expect("write the file");
}

#[tokio::test]
async fn a_live_or_trashed_row_refers_to_its_files_in_every_column_shape() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let (image, index, gallery, trashed) = (new_name(), new_name(), new_name(), new_name());
        insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &image)),
                index_image: Some(stored(INDEX, &index)),
                images: vec![stored(GALLERY, &new_name()), stored(GALLERY, &gallery)],
            },
            false,
        )
        .await;
        insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &trashed)),
                ..ProbeRevision::default()
            },
            true,
        )
        .await;
        for referenced in [
            path(AUTHORS, &image),
            path(INDEX, &index),
            path(GALLERY, &gallery),
            path(AUTHORS, &trashed),
        ] {
            assert!(
                is_referenced(&ctx.db, &OWNERS, &referenced)
                    .await
                    .expect("query"),
                "{referenced}"
            );
        }
        // The same names in another folder are other files, and a gallery
        // name is not an index image: `index/<name>` is what a row stores.
        for unreferenced in [
            path(GALLERY, &image),
            path(GALLERY, &index),
            path(INDEX, &gallery),
            path(AUTHORS, &new_name()),
        ] {
            assert!(
                !is_referenced(&ctx.db, &OWNERS, &unreferenced)
                    .await
                    .expect("query"),
                "{unreferenced}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_revision_refers_to_a_file_wherever_its_snapshot_holds_the_value() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let (replaced, gallery, renamed, nested) = (new_name(), new_name(), new_name(), new_name());
        let id = insert(&ctx.db, &ProbeRevision::default(), false).await;
        // The row was saved with an image and a gallery, then cleared: only
        // its revision still shows them.
        append_revision(
            &ctx.db,
            id,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &replaced)),
                images: vec![stored(GALLERY, &gallery)],
                ..ProbeRevision::default()
            },
        )
        .await;
        append_revision(&ctx.db, id, &ProbeRevision::default()).await;
        // Snapshots of an older shape: the field had another name, or sat
        // in a nested object. Only an upcaster would read them in the
        // current shape, so the search must not depend on the field.
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            1,
            json!({ "photo": stored(AUTHORS, &renamed) }),
        )
        .await;
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            1,
            json!({ "media": { "gallery": [{ "file": stored(GALLERY, &nested) }] } }),
        )
        .await;
        for referenced in [
            path(AUTHORS, &replaced),
            path(GALLERY, &gallery),
            path(AUTHORS, &renamed),
            path(GALLERY, &nested),
        ] {
            assert!(
                is_referenced(&ctx.db, &OWNERS, &referenced)
                    .await
                    .expect("query"),
                "{referenced}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn another_entitys_revision_or_a_partial_match_is_no_reference() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let name = new_name();
        // Another entity's history holds the exact value: its files are in
        // its own folders, so this is not a reference to the probe's file.
        insert_raw_revision(
            &ctx.db,
            "other_entities",
            1,
            1,
            json!({ "image": stored(AUTHORS, &name) }),
        )
        .await;
        // A longer string that contains the value, and the value as an
        // object key, are not the value.
        let id = insert(&ctx.db, &ProbeRevision::default(), false).await;
        let mut snapshot = serde_json::Map::new();
        snapshot.insert("note".to_owned(), json!(format!("see {name}")));
        snapshot.insert(name.clone(), json!(true));
        insert_raw_revision(&ctx.db, TABLE, id, 1, snapshot.into()).await;
        insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(format!("{name}.bak")),
                ..ProbeRevision::default()
            },
            false,
        )
        .await;
        assert!(
            !is_referenced(&ctx.db, &OWNERS, &path(AUTHORS, &name))
                .await
                .expect("query")
        );
    })
    .await;
}

#[tokio::test]
async fn a_registry_that_breaks_its_rules_or_misses_the_folder_queries_nothing() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let file = path(AUTHORS, &new_name());
        assert!(matches!(
            is_referenced(&ctx.db, &BAD, &file).await,
            Err(OwnedFilesError::Owners(_))
        ));
        assert!(matches!(
            is_referenced(&ctx.db, &OWNERS[1..], &file).await,
            Err(OwnedFilesError::NoOwner(_))
        ));
        let error = is_referenced(&ctx.db, &MISSING, &file)
            .await
            .expect_err("no such column");
        assert!(matches!(error, OwnedFilesError::Database(_)), "{error:?}");
        assert!(error.to_string().contains("photo"), "{error}");
    })
    .await;
}

#[tokio::test]
async fn every_registered_owner_names_a_column_of_the_migrated_schema() {
    // The registry is written by hand, one line per upload slice; this
    // runs the reference query of each registered column against the
    // schema the migrations build, so a misspelt table or column fails here
    // rather than in a purge's job.
    with_app(|_router, ctx| async move {
        for owner in FILE_OWNERS {
            let file = path(owner.folder, &new_name());
            assert!(
                !is_referenced(&ctx.db, std::slice::from_ref(owner), &file)
                    .await
                    .unwrap_or_else(|error| panic!("{}.{}: {error}", owner.entity, owner.column)),
                "{}.{}",
                owner.entity,
                owner.column
            );
        }
    })
    .await;
}

#[tokio::test]
async fn only_the_unreferenced_files_are_removed_and_each_outcome_is_reported() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        let root = volume.settings().root.clone();
        let (kept, gone, never) = (new_name(), new_name(), new_name());
        insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &kept)),
                ..ProbeRevision::default()
            },
            false,
        )
        .await;
        let paths = [
            path(AUTHORS, &kept),
            path(AUTHORS, &gone),
            path(AUTHORS, &never),
        ];
        write_file(&root, &paths[0]);
        write_file(&root, &paths[1]);

        let report = delete_unreferenced(&ctx.db, volume.settings(), &OWNERS, &paths)
            .await
            .expect("the removal");
        assert_eq!(report.deleted, vec![paths[1].path().to_owned()]);
        assert_eq!(report.kept, vec![paths[0].path().to_owned()]);
        assert_eq!(report.absent, vec![paths[2].path().to_owned()]);
        assert!(report.is_complete(), "{report:?}");
        assert_eq!(
            TempMedia::files(&root),
            vec![Path::new(paths[0].path()).to_owned()]
        );

        // No paths: nothing is opened, so not even a missing root matters.
        std::fs::remove_dir_all(&root).expect("remove the root");
        let report = delete_unreferenced(&ctx.db, volume.settings(), &OWNERS, &[])
            .await
            .expect("nothing to do");
        assert_eq!(report, DeletionReport::default());
    })
    .await;
}

#[tokio::test]
async fn a_folder_that_does_not_exist_yet_holds_no_file_to_remove() {
    // A fresh volume has the root and none of the entity image folders,
    // such as colleagues_images/ before the first colleague photo; the job
    // must not fail there, or its retry would never succeed.
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let volume = TempMedia::new();
        let root = volume.settings().root.clone();
        let author = path(AUTHORS, &new_name());
        let index = path(INDEX, &new_name());
        let report = delete_unreferenced(
            &ctx.db,
            volume.settings(),
            &OWNERS,
            &[author.clone(), index.clone()],
        )
        .await
        .expect("the removal");
        assert_eq!(
            report.absent,
            vec![author.path().to_owned(), index.path().to_owned()]
        );
        assert!(report.is_complete(), "{report:?}");

        // posts_images/ is there, with a gallery image, and its index/ is
        // not.
        let gallery = path(GALLERY, &new_name());
        write_file(&root, &gallery);
        let report = delete_unreferenced(
            &ctx.db,
            volume.settings(),
            &OWNERS,
            std::slice::from_ref(&index),
        )
        .await
        .expect("the removal");
        assert_eq!(report.absent, vec![index.path().to_owned()]);
        assert!(report.is_complete(), "{report:?}");
        assert_eq!(
            TempMedia::files(&root),
            vec![Path::new(gallery.path()).to_owned()]
        );
    })
    .await;
}

#[tokio::test]
async fn a_record_names_the_files_of_its_row_and_of_every_revision_once_in_order() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let (old, current, index, first, second) =
            (new_name(), new_name(), new_name(), new_name(), new_name());
        let id = insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &current)),
                index_image: Some(stored(INDEX, &index)),
                images: vec![stored(GALLERY, &first), stored(GALLERY, &second)],
            },
            true,
        )
        .await;
        // Revision 1 had another image and the gallery in the other order;
        // revision 2 is the row as it is.
        append_revision(
            &ctx.db,
            id,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &old)),
                images: vec![stored(GALLERY, &second), stored(GALLERY, &first)],
                ..ProbeRevision::default()
            },
        )
        .await;
        append_revision(
            &ctx.db,
            id,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &current)),
                index_image: Some(stored(INDEX, &index)),
                images: vec![stored(GALLERY, &first), stored(GALLERY, &second)],
            },
        )
        .await;
        // Another record's files are not this record's.
        insert(
            &ctx.db,
            &ProbeRevision {
                image: Some(stored(AUTHORS, &new_name())),
                ..ProbeRevision::default()
            },
            false,
        )
        .await;

        let files = files_of_record::<ProbeRevision>(&ctx.db, &OWNERS, id)
            .await
            .expect("the files");
        // Per registered column, in the registry's order: the row first,
        // then the revisions' files it does not name.
        assert_eq!(
            files,
            vec![
                path(AUTHORS, &current),
                path(AUTHORS, &old),
                path(INDEX, &index),
                path(GALLERY, &first),
                path(GALLERY, &second),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn a_record_with_no_files_or_unusable_values_names_only_its_usable_files() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let empty = insert(&ctx.db, &ProbeRevision::default(), false).await;
        assert_eq!(
            files_of_record::<ProbeRevision>(&ctx.db, &OWNERS, empty)
                .await
                .expect("none"),
            Vec::new()
        );
        // A row that does not exist, and an entity with no registered
        // column, name nothing.
        assert_eq!(
            files_of_record::<ProbeRevision>(&ctx.db, &OWNERS, empty + 1000)
                .await
                .expect("none"),
            Vec::new()
        );
        assert_eq!(
            files_of_record::<OtherRevision>(&ctx.db, &OWNERS, empty)
                .await
                .expect("none"),
            Vec::new()
        );

        // A value that leaves its folder and an index value in the gallery
        // column are skipped.
        let (good, older, kept) = (new_name(), new_name(), new_name());
        let id = insert(
            &ctx.db,
            &ProbeRevision {
                image: Some("../uploads/a.pdf".to_owned()),
                images: vec![format!("index/{}", new_name()), stored(GALLERY, &good)],
                ..ProbeRevision::default()
            },
            false,
        )
        .await;
        // A revision of the current version that lacks the members, written
        // by hand: its `photo` is no registered column.
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            1,
            json!({ "photo": stored(AUTHORS, &older) }),
        )
        .await;
        // Items of a gallery that are no stored value: `null`, a number and
        // an object. jsonb keeps them, though backend3 never writes one;
        // the strings beside them still count.
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            1,
            json!({
                "image": null,
                "index_image": 5,
                "images": [null, 7, { "file": stored(GALLERY, &new_name()) }, stored(GALLERY, &kept)],
            }),
        )
        .await;
        assert_eq!(
            files_of_record::<ProbeRevision>(&ctx.db, &OWNERS, id)
                .await
                .expect("the files"),
            vec![path(GALLERY, &good), path(GALLERY, &kept)]
        );

        // A registry that could carry SQL reads nothing; a registered column
        // that the table lacks is the database's error.
        assert!(matches!(
            files_of_record::<ProbeRevision>(&ctx.db, &BAD, id).await,
            Err(OwnedFilesError::Owners(_))
        ));
        let error = files_of_record::<RenamedGalleryRevision>(&ctx.db, &NO_GALLERY_COLUMN, id)
            .await
            .expect_err("no such column");
        assert!(matches!(error, OwnedFilesError::Database(_)), "{error:?}");
        assert!(error.to_string().contains("gallery"), "{error}");
    })
    .await;
}

#[tokio::test]
async fn a_registered_column_that_the_snapshot_names_otherwise_is_refused() {
    // The snapshot calls the gallery `gallery`: its files in old revisions
    // would be missed without a sign, so nothing is read.
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let id = insert(
            &ctx.db,
            &ProbeRevision {
                images: vec![stored(GALLERY, &new_name())],
                ..ProbeRevision::default()
            },
            false,
        )
        .await;
        let error = files_of_record::<RenamedGalleryRevision>(&ctx.db, &OWNERS, id)
            .await
            .expect_err("no images field");
        assert!(
            matches!(
                error,
                OwnedFilesError::Owners(FileOwnerError::NotASnapshotField {
                    entity: TABLE,
                    column: "images"
                })
            ),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("RevisionSnapshot::FIELDS"),
            "{error}"
        );
        // A snapshot type that breaks its own rules reads nothing either.
        assert!(matches!(
            files_of_record::<NoStepRevision>(&ctx.db, &OWNERS[..1], id).await,
            Err(OwnedFilesError::Snapshot(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn a_revision_of_an_older_shape_is_upcast_before_its_files_are_read() {
    with_app(|_router, ctx| async move {
        owned_file_probes::create_table(&ctx.db).await;
        let (photo, current, newer, unreadable) = (new_name(), new_name(), new_name(), new_name());
        let id = insert(&ctx.db, &ProbeRevision::default(), false).await;
        // Version 1 called the photo `photo`; the upcast names it `image`.
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            1,
            json!({ "photo": stored(AUTHORS, &photo), "index_image": null, "images": [] }),
        )
        .await;
        // A version 1 snapshot without `photo`: its upcast step refuses it,
        // so it is skipped, and the other revisions are still read.
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            1,
            json!({ "image": stored(AUTHORS, &unreadable), "index_image": null, "images": [] }),
        )
        .await;
        // The current version, with a member that is no owned column.
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            2,
            json!({
                "image": stored(AUTHORS, &current),
                "index_image": null,
                "images": [],
                "body": "<p>invented</p>",
            }),
        )
        .await;
        // Version 3, written by a newer release this one was rolled back
        // from: read as stored.
        insert_raw_revision(
            &ctx.db,
            TABLE,
            id,
            3,
            json!({ "image": stored(AUTHORS, &newer), "index_image": null, "images": [] }),
        )
        .await;
        assert_eq!(
            files_of_record::<ProbeRevisionV2>(&ctx.db, &OWNERS, id)
                .await
                .expect("the files"),
            vec![
                path(AUTHORS, &photo),
                path(AUTHORS, &current),
                path(AUTHORS, &newer)
            ]
        );
    })
    .await;
}
