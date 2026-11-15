//! The admin upload path over HTTP, as every upload endpoint runs it
//! (ADR 0015 §6 and §7, ADR 0021 §4, ADR 0023 §5).
//!
//! No upload endpoint exists yet (the first is the author image), so these
//! tests mount a probe route that does exactly what `controllers::admin::
//! media` tells an endpoint to do: read the one file part with
//! `receive_image_part`, store it with `models::media::store_entity_image`,
//! and answer any error with `upload_error_response`. In front of it is the
//! request body limit that `config/test.yaml` gives the server, applied as
//! Loco's `limit_payload` middleware applies it (`DefaultBodyLimit::max`,
//! loco-rs 1.2.0 `src/controller/middleware/limit_payload.rs`), so the
//! 128 MiB boundary and its JSON `413` are tested end to end through axum's
//! multipart reader. The JSON routes' own 4 MiB limit is tested through the
//! real app on the sign-in (`auth_login.rs`).

use std::{path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Multipart, State, multipart::MultipartRejection},
    http::{Request, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse as _, Response},
    routing::post,
};
use backend3::{
    controllers::admin::media::{
        FILE_PART, INVALID_UPLOAD_BODY, media_uploads, receive_image_part, upload_error_response,
    },
    domain::{
        image_optimise::{PixelBudget, PngEffort},
        media_rules::{EntityImageFolder, is_entity_image_name},
    },
    models::media::{UploadGate, random_name_bytes, store_entity_image},
    settings::{ImageOptimisationSettings, MediaSettings, Settings},
};
use futures_util::stream;
use loco_rs::{
    controller::middleware::limit_payload::DefaultBodyLimitKind, environment::Environment,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::support::{TempMedia, image_fixture, with_app};

/// The multipart boundary of every request below.
const BOUNDARY: &str = "backend3-test-boundary";

/// What the probe route needs: the settings and the gate that an endpoint
/// takes from `MediaUploads`.
struct Probe {
    media: MediaSettings,
    images: ImageOptimisationSettings,
    gate: UploadGate,
}

/// The probe endpoint: an author image upload without the author.
async fn upload(
    State(probe): State<Arc<Probe>>,
    multipart: Result<Multipart, MultipartRejection>,
) -> loco_rs::Result<Response> {
    let received = match receive_image_part(multipart, &probe.media, &probe.images).await {
        Ok(received) => received,
        Err(error) => return upload_error_response(&error),
    };
    let name = match random_name_bytes() {
        Ok(name) => name,
        Err(error) => return upload_error_response(&error),
    };
    match store_entity_image(
        received,
        EntityImageFolder::AuthorImages,
        name,
        &probe.media,
        &probe.images,
        &probe.gate,
    )
    .await
    {
        Ok(stored) => Ok(Json(json!({
            "storedValue": stored.stored_value,
            "path": stored.path,
            "width": stored.header.dimensions.width(),
            "height": stored.header.dimensions.height(),
        }))
        .into_response()),
        Err(error) => upload_error_response(&error),
    }
}

/// The request body limit that `config/test.yaml` gives the server.
fn configured_body_limit() -> usize {
    let config = Environment::Test.load().expect("config/test.yaml");
    match config
        .server
        .middlewares
        .limit_payload
        .expect("limit_payload is configured")
        .body_limit
    {
        DefaultBodyLimitKind::Limit(limit) => limit,
        DefaultBodyLimitKind::Disable => panic!("the body limit is on"),
    }
}

/// The probe router on `volume`, behind the configured body limit.
fn router(volume: &TempMedia) -> Router {
    let probe = Probe {
        media: volume.settings().clone(),
        images: ImageOptimisationSettings {
            pixel_budget: PixelBudget::DEFAULT,
            png: PngEffort::DEFAULT,
        },
        gate: UploadGate::new(),
    };
    Router::new()
        .route("/upload", post(upload))
        .layer(axum::extract::DefaultBodyLimit::max(configured_body_limit()))
        .with_state(Arc::new(probe))
}

/// One part of a multipart body, with its leading boundary.
///
/// - `name`: the part's name.
/// - `file_name`: the file name the client sends, if any.
/// - `content`: the part's bytes.
fn part(name: &str, file_name: Option<&str>, content: &[u8]) -> Vec<u8> {
    let file_name = file_name
        .map(|file_name| format!("; filename=\"{file_name}\""))
        .unwrap_or_default();
    let mut part = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"{file_name}\r\n\
         Content-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    part.extend_from_slice(content);
    part.extend_from_slice(b"\r\n");
    part
}

/// A multipart body of `parts`, with the closing boundary.
fn multipart(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = parts.concat();
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// POSTs `body` with `content_type` and returns the status and the JSON body.
async fn send(router: &Router, content_type: &str, body: Body) -> (StatusCode, Value) {
    let request = Request::post("/upload")
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .expect("a request");
    let response = router.clone().oneshot(request).await.expect("infallible");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("a body");
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// POSTs one multipart body.
async fn send_parts(router: &Router, parts: &[Vec<u8>]) -> (StatusCode, Value) {
    send(
        router,
        &format!("multipart/form-data; boundary={BOUNDARY}"),
        Body::from(multipart(parts)),
    )
    .await
}

/// Asserts that no staging file and no stored file is left.
fn assert_nothing_left(volume: &TempMedia, context: &str) {
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<PathBuf>::new(),
        "{context}: staging"
    );
    assert_eq!(
        TempMedia::files(&volume.settings().root),
        Vec::<PathBuf>::new(),
        "{context}: root"
    );
}

#[tokio::test]
async fn one_image_part_is_stored_under_a_generated_name() {
    let volume = TempMedia::new();
    let router = router(&volume);
    let camera = image_fixture("jpeg/camera.jpg");
    let (status, body) =
        send_parts(&router, &[part(FILE_PART, Some("IMG_0001.JPG"), &camera)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["width"].clone(), body["height"].clone()),
        (json!(67), json!(41))
    );
    let path = body["path"].as_str().expect("a path");
    let name = path
        .strip_prefix("authors_images/")
        .expect("the author folder");
    // The author's row would store the bare name (data-migration.md §6.4).
    assert_eq!(body["storedValue"], json!(name));
    assert!(
        is_entity_image_name(name)
            && name.rsplit_once('.').map(|(_, extension)| extension) == Some("jpg"),
        "{name}"
    );
    let stored = std::fs::read(volume.settings().root.join(path)).expect("stored");
    assert!(stored.len() < camera.len());
    assert_eq!(
        TempMedia::files(&volume.settings().tmp_dir),
        Vec::<PathBuf>::new()
    );
}

#[tokio::test]
async fn a_refused_file_answers_its_code_at_the_file_part() {
    let volume = TempMedia::new();
    let router = router(&volume);
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";
    for (file_name, content, expected) in [
        (
            "logo.png",
            svg.to_vec(),
            json!({
                "error": "image_not_allowed",
                "description": "the file content is not a JPEG, PNG, GIF or WebP image",
                "field": "file",
                "params": {}
            }),
        ),
        (
            "logo.svg",
            svg.to_vec(),
            json!({
                "error": "image_not_allowed",
                "description": "only JPEG, PNG, GIF and WebP images with a .jpg, .jpeg, .jfif, \
                                .png, .gif or .webp name are accepted",
                "field": "file",
                "params": {}
            }),
        ),
        (
            "photo.png",
            image_fixture("jpeg/baseline.jpg"),
            json!({
                "error": "image_type_mismatch",
                "description": "the file is named .png but its content is a JPEG image",
                "field": "file",
                "params": { "extension": "png", "format": "JPEG" }
            }),
        ),
        (
            "empty.gif",
            b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec(),
            json!({
                "error": "image_corrupt",
                "description": "the GIF image does not decode: the GIF has no image",
                "field": "file",
                "params": { "format": "GIF" }
            }),
        ),
        (
            "camera.jpg",
            image_fixture("over-budget/camera-50mp.jpg"),
            json!({
                "error": "image_too_many_pixels",
                "description": "the JPEG image is 8688x5792 (50320896 pixels), above the \
                                pixel budget of 50000000 pixels",
                "field": "file",
                "params": { "width": 8688, "height": 5792, "max_megapixels": 50 }
            }),
        ),
    ] {
        let (status, body) =
            send_parts(&router, &[part(FILE_PART, Some(file_name), &content)]).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{file_name}: {body}"
        );
        assert_eq!(body, expected, "{file_name}");
        assert_nothing_left(&volume, file_name);
    }
}

#[tokio::test]
async fn a_request_that_is_not_one_file_part_is_an_invalid_body() {
    let volume = TempMedia::new();
    let router = router(&volume);
    let gif = image_fixture("gif/still.gif");
    let multipart_type = format!("multipart/form-data; boundary={BOUNDARY}");
    let cut_off = {
        let mut body = part(FILE_PART, Some("a.gif"), &gif);
        body.truncate(body.len() - 10);
        body
    };
    // The reason after the description's start: backend3's own text where
    // it decides, and axum's where axum's multipart reader refuses.
    for (case, content_type, body, reason) in [
        (
            "JSON",
            "application/json".to_owned(),
            br#"{"file":"x"}"#.to_vec(),
            None,
        ),
        (
            "no boundary",
            "multipart/form-data".to_owned(),
            multipart(&[part(FILE_PART, Some("a.gif"), &gif)]),
            None,
        ),
        (
            "no part",
            multipart_type.clone(),
            multipart(&[]),
            Some("the request has no part"),
        ),
        (
            "another part first",
            multipart_type.clone(),
            multipart(&[part("image", Some("a.gif"), &gif)]),
            Some("the first part must be the file part `file`"),
        ),
        (
            "a second part",
            multipart_type.clone(),
            multipart(&[
                part(FILE_PART, Some("a.gif"), &gif),
                part("alt", None, "Kép".as_bytes()),
            ]),
            Some("the request has more than one part"),
        ),
        (
            "two files",
            multipart_type.clone(),
            multipart(&[
                part(FILE_PART, Some("a.gif"), &gif),
                part(FILE_PART, Some("b.gif"), &gif),
            ]),
            Some("the request has more than one part"),
        ),
        ("cut off", multipart_type.clone(), cut_off, None),
    ] {
        let (status, body) = send(&router, &content_type, Body::from(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{case}: {body}");
        assert_eq!(
            (&body["error"], &body["field"], &body["params"]),
            (&json!("invalid_body"), &Value::Null, &json!({})),
            "{case}"
        );
        // The description speaks of the upload, not of JSON, and says why.
        let description = body["description"].as_str().expect("a description");
        let why = description
            .strip_prefix(&format!("{INVALID_UPLOAD_BODY}: "))
            .unwrap_or_else(|| panic!("{case}: {description}"));
        match reason {
            Some(reason) => assert_eq!(why, reason, "{case}"),
            None => assert!(!why.is_empty(), "{case}"),
        }
        assert_nothing_left(&volume, case);
    }
}

/// A megabyte of zeros, the filler of the large bodies below.
static ZEROS: [u8; 1 << 20] = [0; 1 << 20];

/// A streamed multipart body of exactly `total` bytes: one `file` part named
/// `big.png` that starts with the PNG signature and is zeros after it. The
/// zeros come from [`ZEROS`], so the test holds no copy of the body.
fn png_body_of(total: usize) -> Body {
    let head = [
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{FILE_PART}\"; \
             filename=\"big.png\"\r\n\r\n"
        )
        .into_bytes(),
        b"\x89PNG\r\n\x1a\n".to_vec(),
    ]
    .concat();
    let tail = format!("\r\n--{BOUNDARY}--\r\n").into_bytes();
    let zeros = total - head.len() - tail.len();
    let mut chunks = vec![Bytes::from(head)];
    chunks.extend((0..zeros / ZEROS.len()).map(|_| Bytes::from_static(&ZEROS)));
    chunks.push(Bytes::from_static(&ZEROS[..zeros % ZEROS.len()]));
    chunks.push(Bytes::from(tail));
    Body::from_stream(stream::iter(
        chunks.into_iter().map(Ok::<_, std::io::Error>),
    ))
}

#[tokio::test]
async fn a_body_above_128_mib_gets_the_json_413_and_leaves_nothing() {
    let limit = configured_body_limit();
    // ADR 0015 §6: `body_limit: 128mib`, which is 134,217,728 bytes.
    assert_eq!(limit, 134_217_728);
    let volume = TempMedia::new();
    let router = router(&volume);
    let multipart_type = format!("multipart/form-data; boundary={BOUNDARY}");

    // At the limit the whole body is read and staged; the image is then
    // refused for what it is, a PNG signature and no image.
    let (status, body) = send(&router, &multipart_type, png_body_of(limit)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "image_corrupt", "{body}");
    assert_nothing_left(&volume, "at the limit");

    // One byte more is refused while it streams in, with the envelope.
    let (status, body) = send(&router, &multipart_type, png_body_of(limit + 1)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        body,
        json!({
            "error": "body_too_large",
            "description": "The request body is larger than the limit",
            "field": null,
            "params": {}
        })
    );
    assert_nothing_left(&volume, "above the limit");
}

#[tokio::test]
async fn the_app_context_carries_the_upload_handles() {
    with_app(|_router, ctx| async move {
        let uploads = media_uploads(&ctx).expect("registered by after_context");
        let settings = Settings::from_section(ctx.config.settings.as_ref()).expect("settings");
        assert_eq!(uploads.media, settings.media);
        assert_eq!(uploads.images, settings.image_optimisation);
        // One permit per process: the context's clones share it.
        let held = uploads.gate.acquire().await.expect("the permit");
        let again = media_uploads(&ctx).expect("registered");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), again.gate.acquire())
                .await
                .is_err(),
            "a second permit while the first is held"
        );
        drop(held);
        drop(again.gate.acquire().await.expect("free again"));
    })
    .await;
}
