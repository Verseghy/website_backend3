//! The error envelope of the admin API (ADR 0021 §4).
//!
//! Every refused or invalid admin API request is answered with one envelope:
//! a stable [`ErrorCode`], an English `description` for script clients and
//! logs, the request `field` the error belongs to, and named `params` for the
//! Hungarian message template. The admin UI shows `errors.<code>` from its
//! Hungarian message file and falls back to `description` only for a code it
//! does not know. Its message table is declared
//! `satisfies Record<ErrorCode, string>`, so a code added here without a
//! Hungarian message fails the admin type check.
//!
//! Codes are added by the slice that first returns them: `not_found` for
//! unknown `/api/admin` paths, `invalid_credentials` for every failed
//! sign-in (ADR 0006 §4), `invalid_body` and `body_too_large` for a
//! request body that cannot be read as the endpoint's JSON DTO (the
//! maintainer's answer of 2026-10-25, research: DECISIONS R15 (D74)), and
//! `unauthorized` for a request without a valid admin session (ADR 0006 §1,
//! ADR 0021 §4's 401 row), and `forbidden` for a signed-in account that the
//! route's requirement refuses (the route table's guard, ADR 0007 §5,
//! ADR 0021 §4's 403 row). `origin_mismatch` answers a write that carries
//! the session cookie but not the admin's own origin in its `Origin` header
//! (ADR 0006 §3, as amended by ADR 0044), also from the route table's guard,
//! before the session is opened. The seven codes of a refused rich-text body
//! (editor.md §7.6) came with the validator that finds them,
//! `domain::rich_text::validate_content`, before the post and page writes
//! that answer them: each finding is a [`super::content::ContentError`],
//! `{code, index, detail}`, and the codes needed their Hungarian messages
//! first.
//!
//! The shared admin list contract (the first list, labels, parity SHR-02
//! and SHR-03) adds the six refusals of a list request's query, one per
//! parameter kind: `invalid_page`, `invalid_per_page`, `invalid_sort`,
//! `invalid_order`, `unknown_filter` and `invalid_filter`, each a `422` whose
//! `field` is the query parameter (`dtos::admin::list`). Every list has
//! them, so they belong to no resource and are `snake_case`.
//!
//! The shared write contract (the first writes, labels, parity SHR-10 and
//! SHR-13) adds the four refusals of a field that a create or an update
//! cannot store: `field_required`, `field_too_long` (`params`: `max`),
//! `field_invalid_character` and `field_invalid_color`, each a `422` whose
//! `field` is the request member (`dtos::admin::write`). Every resource's
//! fields are refused by the same rules, so they are `snake_case`. The label
//! writes add the first resource rule, `label.name_taken`, the `409` of a
//! name that a live label already has (ADR 0021 §4's 409 row).
//!
//! The upload model adds the image refusals of ADR 0015 §6 and §7 and the
//! four CPU limits of ADR 0033 §4 (`image_*`), and the free-space refusal
//! (`insufficient_storage`, 507 Insufficient Storage: the maintainer's
//! answer of 2026-11-01, research: DECISIONS R24 (D121)).
//! `controllers::admin::media` maps every upload error to them, for every
//! upload endpoint alike. They belong to no single resource (an author's
//! photo and a post's gallery image are refused by the same rules), so they
//! are `snake_case`, not `<resource>.<reason>` (ADR 0021 §4). Their messages
//! ask the editor for what fixes the file: a file saved again, a smaller
//! one, or one of the four formats.

use std::collections::BTreeMap;

use serde::Serialize;
use ts_rs::TS;

/// A stable machine-readable error code of the admin API.
///
/// A code that belongs to no resource is `snake_case` and keeps the public
/// API's spelling where it means the same thing (`not_found`). A rule of one
/// resource uses a dotted `<resource>.<reason>` code, such as
/// `search.term_too_long`.
// The doc comment above is exported into the TypeScript binding; Rust-only
// notes go here. Dotted codes need an explicit `#[serde(rename = "...")]` on
// their variant, because `rename_all` cannot produce a dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../admin/src/bindings/")]
pub enum ErrorCode {
    /// `404`: the resource or the API path does not exist.
    NotFound,
    /// `401`: the sign-in failed. One code for every reason (a wrong
    /// password, an unknown e-mail address, a deactivated or trashed
    /// account), so the answer does not reveal which accounts exist.
    InvalidCredentials,
    /// `422`: the request body is not what the endpoint takes. For a JSON
    /// endpoint: not JSON, not sent as `application/json`, or with a field
    /// that is missing, unknown or of the wrong type. For an upload: not one
    /// `multipart/form-data` part named `file`, and the description says
    /// why. An error on the whole request, so `field` is `null`.
    InvalidBody,
    /// `413`: the request body is larger than the request body limit.
    BodyTooLarge,
    /// `422`: an image of a rich-text body has no alt text, or only white
    /// space (ADR 0013 §2).
    ImageAltMissing,
    /// `422`: the sanitiser removed an image of a rich-text body, because
    /// its source is not an uploaded image (editor.md §3.6) or it had none.
    ImageSrcRejected,
    /// `422`: an embed of a rich-text body is not a `YouTube` or Padlet embed.
    EmbedRejected,
    /// `422`: an embed of a rich-text body holds content.
    IframeContent,
    /// `422`: a rich-text body, or the sanitiser's output for it, is larger
    /// than its limit.
    ContentTooLarge,
    /// `422`: a rich-text body nests elements deeper than the limit.
    ContentTooDeep,
    /// `422`: the sanitiser could not bring a rich-text body to a fixed
    /// point; the server logs it as an error.
    ContentNotNormalisable,
    /// `401`: the request has no valid admin session, so the user signs in
    /// again. One code for every reason: no token, a token that is not
    /// valid or has expired, an account that is trashed, anonymised or
    /// deactivated, a token that an account event revoked, or a session
    /// scope that the route does not accept.
    Unauthorized,
    /// `403`: the session is valid, but the account does not hold what the
    /// route asks for (a permission or the `admin` role). The session stays
    /// valid, so the user is not signed out. A list the account may not see
    /// answers this, never an empty list.
    Forbidden,
    /// `403`: a write that carries the admin session cookie (or the
    /// two-factor step's cookie) did not come with the admin's own origin in
    /// its `Origin` header: it came from another site or a sibling host, or
    /// through a client or extension that drops the header (ADR 0006 §3, as
    /// amended by ADR 0044). Nothing was done and the session stays valid, so
    /// the user is not signed out.
    OriginMismatch,
    /// `422`: the upload is not a JPEG, PNG, GIF or WebP image. Its file
    /// name has another extension or none, or its content is not an image
    /// at all (an SVG or HTML file renamed to `.png`, for example). `field`
    /// is the file part.
    ImageNotAllowed,
    /// `422`: the upload's content is an image of another format than its
    /// extension says. `params`: `extension` (the file name's, without the
    /// dot) and `format` (what the content is, such as `JPEG`).
    ImageTypeMismatch,
    /// `422`: the image is corrupt: its header or data does not decode as
    /// its format, or the file is cut short. `params`: `format`.
    ImageCorrupt,
    /// `422`: the image has more pixels than the pixel budget allows; it is
    /// refused from its header, before any decode. `params`: `width`,
    /// `height` and `max_megapixels`.
    ImageTooManyPixels,
    /// `422`: the animation has more frames, or more pixels in all its
    /// frames, than the optimiser takes. `params`: `max_frames` and
    /// `max_megapixels`.
    ImageAnimationTooLarge,
    /// `422`: the JPEG has more progressive scans than the optimiser takes.
    /// `params`: `max_scans`.
    ImageTooManyScans,
    /// `507`: storing the upload would leave less free space on the media
    /// volume than the configured reserve. An operator has to make room.
    InsufficientStorage,
    /// `422`: a list's `page` is not a whole number from 1 up, or is given
    /// twice. `field`: `page`.
    InvalidPage,
    /// `422`: a list's `perPage` is not 10, 25, 50 or 100, or is given
    /// twice. `field`: `perPage`.
    InvalidPerPage,
    /// `422`: a list's `sort` names a field the list cannot be sorted on,
    /// or is given twice. `field`: `sort`.
    InvalidSort,
    /// `422`: a list's `order` is neither `ASC` nor `DESC`, or is given
    /// twice. `field`: `order`.
    InvalidOrder,
    /// `422`: a list request has a parameter that is no filter of the list.
    /// `field`: the parameter's name.
    UnknownFilter,
    /// `422`: a list filter's value is not what the filter takes, such as
    /// an id that is not a whole number. `field`: the filter's name.
    InvalidFilter,
    /// `422`: a required field of a create or an update is empty, or only
    /// white space once trimmed. `field`: the field.
    FieldRequired,
    /// `422`: a text field of a create or an update is longer than its
    /// limit, counted in characters once trimmed. `field`: the field;
    /// `params`: `max`, the limit.
    FieldTooLong,
    /// `422`: a text field of a create or an update holds the character
    /// U+0000, which the database cannot store. `field`: the field.
    FieldInvalidCharacter,
    /// `422`: a colour field of a create or an update is not `#rrggbb`, a
    /// `#` and six hexadecimal digits. `field`: the field.
    FieldInvalidColor,
    /// `409`: a live label already has the name, compared without regard to
    /// case; a label in the trash does not count. `field`: `name`;
    /// `params`: `name`, the name as sent, trimmed.
    #[serde(rename = "label.name_taken")]
    LabelNameTaken,
}

impl ErrorCode {
    /// The HTTP status code that goes with this error code (ADR 0021 §4).
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::NotFound => 404,
            // ADR 0021 §4: 401 is "no valid session"; a failed sign-in is one.
            Self::InvalidCredentials | Self::Unauthorized => 401,
            // ADR 0021 §4: 403 is a permission or record-level refusal
            // (ADR 0007 §5). ADR 0006 §3 answers a write from a foreign
            // origin with 403 as well (ADR 0044): the session is valid or
            // not looked at, so it is no 401, which would sign the admin out.
            Self::Forbidden | Self::OriginMismatch => 403,
            // ADR 0021 §4 has no row for it; 413 with the JSON envelope
            // follows ADR 0015 §6, whose upload limit answers it (D74).
            Self::BodyTooLarge => 413,
            // ADR 0021 §4's row for submitted values that fail validation.
            // A body that is not the DTO's JSON is read as such a value
            // (research: DECISIONS R15 (D74)). The size and depth limits of
            // a rich-text body are content rules, checked after the request
            // body limit let the request in, so they are 422 like the other
            // findings of a body (editor.md §7.6), not 413. So is an
            // uploaded file that the image rules refuse.
            Self::InvalidBody
            | Self::ImageAltMissing
            | Self::ImageSrcRejected
            | Self::EmbedRejected
            | Self::IframeContent
            | Self::ContentTooLarge
            | Self::ContentTooDeep
            | Self::ContentNotNormalisable
            | Self::ImageNotAllowed
            | Self::ImageTypeMismatch
            | Self::ImageCorrupt
            | Self::ImageTooManyPixels
            | Self::ImageAnimationTooLarge
            | Self::ImageTooManyScans
            // A list request's query is submitted values too: the parity
            // row SHR-02 and the list contract refuse a page size or a sort
            // field outside the allow-list, and ADR 0021 §4 has no 400 row.
            | Self::InvalidPage
            | Self::InvalidPerPage
            | Self::InvalidSort
            | Self::InvalidOrder
            | Self::UnknownFilter
            | Self::InvalidFilter
            // The write contract: a submitted value that cannot be stored
            // (SHR-13).
            | Self::FieldRequired
            | Self::FieldTooLong
            | Self::FieldInvalidCharacter
            | Self::FieldInvalidColor => 422,
            // ADR 0021 §4's 409 row: a uniqueness rule that the stored
            // labels decide, not the submitted value alone.
            Self::LabelNameTaken => 409,
            // ADR 0021 §4 has no row for it, and ADR 0015 §6 asks only for
            // a clear message. 507 Insufficient Storage (RFC 4918 §11.5):
            // the server cannot store what the request needs, and nothing
            // in the request is wrong. Kept as built by the maintainer
            // (research: DECISIONS R24 (D121)).
            Self::InsufficientStorage => 507,
        }
    }

    /// The English description sent with this code when the caller has no
    /// more specific text.
    #[must_use]
    pub const fn default_description(self) -> &'static str {
        match self {
            // The same text as Loco's `ErrorDetail` and the public API.
            Self::NotFound => "Resource was not found",
            Self::InvalidCredentials => "The e-mail address or the password is wrong",
            Self::InvalidBody => "The request body is not the JSON this endpoint takes",
            Self::BodyTooLarge => "The request body is larger than the limit",
            Self::ImageAltMissing => "An image has no alt text",
            Self::ImageSrcRejected => {
                "An image has no source that is an uploaded image under the upload prefix"
            }
            Self::EmbedRejected => "An embed is not a YouTube or Padlet embed",
            Self::IframeContent => "An embed holds content",
            Self::ContentTooLarge => "The content is larger than the limit",
            Self::ContentTooDeep => "The content nests elements deeper than the limit",
            Self::ContentNotNormalisable => "The content did not reach a fixed point",
            // Not Loco's "You do not have permission to access this
            // resource": that reads as a 403, and a 401 means "sign in".
            Self::Unauthorized => "There is no valid admin session; sign in again",
            // Loco's 403 text, "You do not have permission to access this
            // resource", reads the same; this one names the account, so a
            // script with a Bearer token knows that signing in again does
            // not help.
            Self::Forbidden => "The signed-in account may not perform this operation",
            Self::OriginMismatch => {
                "The request does not come from the admin's origin; a write with the session \
                 cookie must carry it in the Origin header"
            }
            Self::ImageNotAllowed => "Only JPEG, PNG, GIF and WebP images are accepted",
            Self::ImageTypeMismatch => "The file content does not match its extension",
            Self::ImageCorrupt => "The image is corrupt",
            Self::ImageTooManyPixels => "The image has more pixels than the pixel budget",
            Self::ImageAnimationTooLarge => "The animation has too many frames or pixels",
            Self::ImageTooManyScans => "The JPEG image has too many scans",
            Self::InsufficientStorage => "There is not enough free space to store the upload",
            // A refused list request sends its refusal's own text, which
            // names the parameter (`dtos::admin::list`); these are the
            // fallbacks.
            Self::InvalidPage => "The page number is not valid",
            Self::InvalidPerPage => "The page size is not valid",
            Self::InvalidSort => "The list cannot be sorted on this field",
            Self::InvalidOrder => "The sort direction is not valid",
            Self::UnknownFilter => "The list has no such filter",
            Self::InvalidFilter => "The filter value is not valid",
            // A refused field sends its refusal's own text, which names the
            // field and the rule (`dtos::admin::write`); these are the
            // fallbacks.
            Self::FieldRequired => "The field is required",
            Self::FieldTooLong => "The field is longer than its limit",
            Self::FieldInvalidCharacter => "The field holds a character that cannot be stored",
            Self::FieldInvalidColor => "The colour is not of the form #rrggbb",
            Self::LabelNameTaken => {
                "A label that is not in the trash already has this name, compared without regard \
                 to case"
            }
        }
    }
}

/// A value for a placeholder of the Hungarian message template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ErrorParam {
    /// A text value, for example a slug.
    Text(String),
    /// A number, for example a count of days. Values stay below 2^53, the
    /// largest integer JSON readers handle exactly (ADR 0005 §3).
    Number(i64),
}

/// The admin API error body: `{ error, description, field, params }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct ErrorEnvelope {
    /// The stable error code.
    pub error: ErrorCode,
    /// English free text for script clients and logs.
    pub description: String,
    /// The request field the error belongs to, spelt as the DTO serializes
    /// it, or `null` for an error on the whole request.
    pub field: Option<String>,
    /// Named values for the placeholders of the message; `{}` when none.
    #[ts(type = "Record<string, string | number>")]
    pub params: BTreeMap<String, ErrorParam>,
}

impl ErrorEnvelope {
    /// An envelope for `code` with its default description, no field and no
    /// parameters.
    ///
    /// - `code`: the error code to report.
    #[must_use]
    pub fn new(code: ErrorCode) -> Self {
        Self {
            error: code,
            description: code.default_description().to_owned(),
            field: None,
            params: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_found_envelope_serializes_exactly() {
        let envelope = ErrorEnvelope::new(ErrorCode::NotFound);
        assert_eq!(
            serde_json::to_value(&envelope).expect("serializes"),
            json!({
                "error": "not_found",
                "description": "Resource was not found",
                "field": null,
                "params": {}
            })
        );
    }

    #[test]
    fn field_and_params_serialize_in_key_order() {
        let mut envelope = ErrorEnvelope::new(ErrorCode::NotFound);
        envelope.field = Some("parentId".to_owned());
        envelope
            .params
            .insert("slug".to_owned(), ErrorParam::Text("hirek".to_owned()));
        envelope
            .params
            .insert("days".to_owned(), ErrorParam::Number(12));
        assert_eq!(
            serde_json::to_string(&envelope).expect("serializes"),
            r#"{"error":"not_found","description":"Resource was not found","field":"parentId","params":{"days":12,"slug":"hirek"}}"#
        );
    }

    #[test]
    fn codes_map_to_statuses() {
        for (code, status, wire) in [
            (ErrorCode::NotFound, 404, "not_found"),
            (ErrorCode::InvalidCredentials, 401, "invalid_credentials"),
            (ErrorCode::InvalidBody, 422, "invalid_body"),
            (ErrorCode::BodyTooLarge, 413, "body_too_large"),
            (ErrorCode::ImageAltMissing, 422, "image_alt_missing"),
            (ErrorCode::ImageSrcRejected, 422, "image_src_rejected"),
            (ErrorCode::EmbedRejected, 422, "embed_rejected"),
            (ErrorCode::IframeContent, 422, "iframe_content"),
            (ErrorCode::ContentTooLarge, 422, "content_too_large"),
            (ErrorCode::ContentTooDeep, 422, "content_too_deep"),
            (
                ErrorCode::ContentNotNormalisable,
                422,
                "content_not_normalisable",
            ),
            (ErrorCode::Unauthorized, 401, "unauthorized"),
            (ErrorCode::Forbidden, 403, "forbidden"),
            (ErrorCode::OriginMismatch, 403, "origin_mismatch"),
            (ErrorCode::ImageNotAllowed, 422, "image_not_allowed"),
            (ErrorCode::ImageTypeMismatch, 422, "image_type_mismatch"),
            (ErrorCode::ImageCorrupt, 422, "image_corrupt"),
            (ErrorCode::ImageTooManyPixels, 422, "image_too_many_pixels"),
            (
                ErrorCode::ImageAnimationTooLarge,
                422,
                "image_animation_too_large",
            ),
            (ErrorCode::ImageTooManyScans, 422, "image_too_many_scans"),
            (ErrorCode::InsufficientStorage, 507, "insufficient_storage"),
            (ErrorCode::InvalidPage, 422, "invalid_page"),
            (ErrorCode::InvalidPerPage, 422, "invalid_per_page"),
            (ErrorCode::InvalidSort, 422, "invalid_sort"),
            (ErrorCode::InvalidOrder, 422, "invalid_order"),
            (ErrorCode::UnknownFilter, 422, "unknown_filter"),
            (ErrorCode::InvalidFilter, 422, "invalid_filter"),
            (ErrorCode::FieldRequired, 422, "field_required"),
            (ErrorCode::FieldTooLong, 422, "field_too_long"),
            (
                ErrorCode::FieldInvalidCharacter,
                422,
                "field_invalid_character",
            ),
            (ErrorCode::FieldInvalidColor, 422, "field_invalid_color"),
            (ErrorCode::LabelNameTaken, 409, "label.name_taken"),
        ] {
            assert_eq!(code.status(), status, "{code:?}");
            assert_eq!(serde_json::to_value(code).expect("serializes"), json!(wire));
            assert!(!code.default_description().is_empty(), "{code:?}");
        }
    }

    #[test]
    fn the_sign_in_failure_envelope_serializes_exactly() {
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::InvalidCredentials))
                .expect("serializes"),
            r#"{"error":"invalid_credentials","description":"The e-mail address or the password is wrong","field":null,"params":{}}"#
        );
    }

    #[test]
    fn the_body_envelopes_serialize_exactly() {
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::InvalidBody)).expect("serializes"),
            r#"{"error":"invalid_body","description":"The request body is not the JSON this endpoint takes","field":null,"params":{}}"#
        );
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::BodyTooLarge))
                .expect("serializes"),
            r#"{"error":"body_too_large","description":"The request body is larger than the limit","field":null,"params":{}}"#
        );
    }

    #[test]
    fn the_session_envelope_serializes_exactly() {
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::Unauthorized))
                .expect("serializes"),
            r#"{"error":"unauthorized","description":"There is no valid admin session; sign in again","field":null,"params":{}}"#
        );
    }

    #[test]
    fn the_permission_envelope_serializes_exactly() {
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::Forbidden)).expect("serializes"),
            r#"{"error":"forbidden","description":"The signed-in account may not perform this operation","field":null,"params":{}}"#
        );
    }

    #[test]
    fn the_origin_envelope_serializes_exactly() {
        assert_eq!(
            serde_json::to_string(&ErrorEnvelope::new(ErrorCode::OriginMismatch))
                .expect("serializes"),
            r#"{"error":"origin_mismatch","description":"The request does not come from the admin's origin; a write with the session cookie must carry it in the Origin header","field":null,"params":{}}"#
        );
    }

    #[test]
    fn typescript_declarations() {
        assert_eq!(
            ErrorCode::decl(&ts_rs::Config::default()),
            r#"type ErrorCode = "not_found" | "invalid_credentials" | "invalid_body" | "body_too_large" | "image_alt_missing" | "image_src_rejected" | "embed_rejected" | "iframe_content" | "content_too_large" | "content_too_deep" | "content_not_normalisable" | "unauthorized" | "forbidden" | "origin_mismatch" | "image_not_allowed" | "image_type_mismatch" | "image_corrupt" | "image_too_many_pixels" | "image_animation_too_large" | "image_too_many_scans" | "insufficient_storage" | "invalid_page" | "invalid_per_page" | "invalid_sort" | "invalid_order" | "unknown_filter" | "invalid_filter" | "field_required" | "field_too_long" | "field_invalid_character" | "field_invalid_color" | "label.name_taken";"#
        );
        let envelope = ErrorEnvelope::decl(&ts_rs::Config::default());
        assert!(envelope.contains("error: ErrorCode"), "{envelope}");
        assert!(envelope.contains("field: string | null"), "{envelope}");
        assert!(
            envelope.contains("params: Record<string, string | number>"),
            "{envelope}"
        );
    }
}
