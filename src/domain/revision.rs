//! The pure parts of revisions (ADR 0011 §4): why a revision was written
//! ([`Reason`]), what a revision snapshot is ([`RevisionSnapshot`], an
//! allow-list type with a `schema_version`), the upcaster registry that
//! reads an older snapshot in the current shape ([`UpcastStep`],
//! [`upcast`], [`read_snapshot`]), and the rules a revision meets before it
//! is written ([`check_revision`], [`next_revision_no`],
//! [`save_entry_details`]).
//!
//! `revisions` is append-only like the activity log: a revision is never
//! corrected, and only a permanent delete may remove one (ADR 0011 §5, §6).
//! So every rule that keeps a wrong revision out runs here, before the
//! insert, where it is tested without a database. The model
//! (`src/models/revisions.rs`) calls these functions and writes what they
//! return.
//!
//! **A snapshot is an allow-list.** Each admin-editable entity has one
//! revision snapshot type, a plain struct that names the record's editable
//! fields and its relation id lists (a post's `label_ids`, a user's roles),
//! never the entity model itself, so a column added to the table does not
//! reach the history by accident, and never a secret (ADR 0011 §4, "Never
//! secrets"). The type lists its fields once more in
//! [`RevisionSnapshot::FIELDS`], and [`snapshot_object`] refuses a snapshot
//! whose JSON object has any other key or lacks one: the list is what a
//! reviewer reads, and what the generic tests compare a stored snapshot
//! with. [`SECRET_FIELDS`] lists the names of the account secrets (their
//! columns, side tables and likely field names) that no snapshot may list.
//!
//! **Shape changes upcast, never rewrite.** Stored snapshots keep the
//! version they were written with. When a snapshot type changes shape, its
//! [`RevisionSnapshot::SCHEMA_VERSION`] goes up by one and the type adds one
//! [`UpcastStep`] from the old version to the new one, so its registry,
//! [`RevisionSnapshot::UPCASTS`], holds exactly one step per version step;
//! [`check_snapshot_type`] refuses a type whose registry has a gap. Reading
//! a stored snapshot runs the steps from its version up to the current one
//! (ADR 0011 §4, "Schema changes"; ADR 0008). Each step has a unit test of
//! its own.
//!
//! These types serialise with serde only: a snapshot is stored JSON, not a
//! wire type, so it derives neither ts-rs nor utoipa (ADR 0008 rule 6). The
//! revisions endpoint converts to its DTO at the controller.

use std::{collections::BTreeSet, fmt, str::FromStr};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use super::activity::{
    Action, Actor, CHANGED_FIELDS_KEY, MAX_REQUEST_ID_CHARS, changed_field_names,
    changed_fields_details, is_snake_case,
};

/// The members of a snapshot: the JSON object a revision stores.
pub type SnapshotObject = Map<String, Value>;

/// Why a revision was written (ADR 0011 §4), stored as `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// The record was created: always its first revision.
    Create,
    /// The record was saved through its update path, including a change of
    /// its relations (a post's labels).
    Update,
    /// The record was saved from an earlier revision through the update path
    /// (ADR 0011 §4, "Revert").
    Revert,
    /// The importer wrote the record (ADR 0017): its first revision, or the
    /// second after [`Reason::ImportOriginal`].
    Import,
    /// The legacy record as MySQL stored it, before the import normalised its
    /// body: always revision 1, followed by [`Reason::Import`] (ADR 0011 §4,
    /// "Import"). The only snapshot that has not been through the sanitiser.
    ImportOriginal,
}

impl Reason {
    /// Every reason, in the order ADR 0011 §4 lists them, which is also the
    /// order of the table's `reason` CHECK.
    pub const ALL: [Self; 5] = [
        Self::Create,
        Self::Update,
        Self::Revert,
        Self::Import,
        Self::ImportOriginal,
    ];

    /// The stored name, as ADR 0011 §4 spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Revert => "revert",
            Self::Import => "import",
            Self::ImportOriginal => "import_original",
        }
    }

    /// Whether only the importer writes revisions of this reason
    /// ([`Reason::Import`] and [`Reason::ImportOriginal`]).
    #[must_use]
    pub const fn is_import(self) -> bool {
        matches!(self, Self::Import | Self::ImportOriginal)
    }

    /// The activity action that records a save of this reason: `create`,
    /// `update` or `revert`, each written with its revision in one
    /// transaction (ADR 0011 §5). `None` for the two import reasons: the
    /// importer writes one `import` entry per table, not one per row.
    #[must_use]
    pub const fn save_action(self) -> Option<Action> {
        match self {
            Self::Create => Some(Action::Create),
            Self::Update => Some(Action::Update),
            Self::Revert => Some(Action::Revert),
            Self::Import | Self::ImportOriginal => None,
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Reason {
    type Err = UnknownReason;

    /// Reads a stored `reason` back.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.as_str() == name)
            .ok_or_else(|| UnknownReason {
                name: name.to_owned(),
            })
    }
}

/// A stored `reason` that names no [`Reason`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{name:?} is not a revision reason (ADR 0011 §4)")]
pub struct UnknownReason {
    /// The refused name.
    pub name: String,
}

/// The field names that no revision snapshot may list, because they name
/// the secrets of ADR 0011 §4 ("Never secrets"): the password hash, the
/// TOTP secret, `token_version`, the reset tokens and the recovery codes.
/// Each secret is listed under the name of its column or side table
/// (ADR 0011 §4, "Account events") and under the shorter name that a
/// snapshot field for it would most likely take (`password`,
/// `totp_secret`, `reset_tokens`, `recovery_codes`).
///
/// [`check_snapshot_type`] refuses a snapshot type that lists any of them.
/// This is a second guard behind the allow-list that a reviewer reads, and
/// behind the users slice's test that the users snapshot holds exactly the
/// allow-listed keys (ADR 0011 "What must be true"). It compares whole
/// names only: a field spelt in another way passes it. It does not refuse
/// every name that contains `password` or `token`, because account columns
/// such as `must_change_password` and `temp_password_expires_at` name no
/// secret.
pub const SECRET_FIELDS: &[&str] = &[
    "password_hash",
    "password",
    "totp_secret_encrypted",
    "totp_secret",
    "token_version",
    "password_reset_tokens",
    "reset_tokens",
    "user_recovery_codes",
    "recovery_codes",
];

/// One step of a snapshot type's upcaster registry: it converts a stored
/// snapshot of version [`UpcastStep::from`] into the shape of the next
/// version.
///
/// A step is a plain function over the snapshot's JSON members, so it reads
/// a snapshot that no Rust type of the current code describes any more. It
/// converts only what changed between the two versions (a renamed field, a
/// new field with the value that the old shape implies) and leaves every
/// other member alone. Each step has a unit test of its own (ADR 0011
/// "What must be true").
///
/// A step that cannot fail (it only adds a field) still returns `Ok`, as
/// the function type asks; Clippy's pedantic `unnecessary_wraps` does not
/// see that the function is used as an `apply`, so such a step carries
/// `#[expect(clippy::unnecessary_wraps, reason = "...")]`.
#[derive(Debug, Clone, Copy)]
pub struct UpcastStep {
    /// The version the step reads; it returns the shape of `from + 1`.
    pub from: i32,
    /// The conversion.
    pub apply: fn(SnapshotObject) -> Result<SnapshotObject, UpcastStepError>,
}

/// Why one [`UpcastStep`] could not convert a stored snapshot: the snapshot
/// does not have the shape its version promises.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpcastStepError {
    /// A member the version always has is missing.
    #[error("the field {field:?} is missing")]
    MissingField {
        /// The missing member.
        field: String,
    },
    /// A member holds a value of the wrong JSON type.
    #[error("the field {field:?} is not {expected}")]
    InvalidField {
        /// The member.
        field: String,
        /// What it should hold, for example "a string".
        expected: &'static str,
    },
}

/// The revision snapshot type of one admin-editable entity (ADR 0011 §4):
/// the record after a save, as an explicit allow-list of its editable fields
/// and relation id lists, plus the version of that shape.
///
/// Implement it on a plain struct, never on the entity's model:
///
/// ```
/// use backend3::domain::revision::{
///     RevisionSnapshot, SnapshotObject, UpcastStep, UpcastStepError, check_snapshot_type,
///     read_snapshot, snapshot_object,
/// };
/// use serde::{Deserialize, Serialize};
/// use serde_json::{Value, json};
///
/// /// Version 2 renamed `name` to `title`.
/// #[derive(Debug, PartialEq, Serialize, Deserialize)]
/// struct NoteRevision {
///     title: String,
///     body: Option<String>,
///     label_ids: Vec<i64>,
/// }
///
/// fn rename_name_to_title(mut v1: SnapshotObject) -> Result<SnapshotObject, UpcastStepError> {
///     let name = v1.remove("name").ok_or(UpcastStepError::MissingField {
///         field: "name".to_owned(),
///     })?;
///     v1.insert("title".to_owned(), name);
///     Ok(v1)
/// }
///
/// impl RevisionSnapshot for NoteRevision {
///     const ENTITY_TYPE: &'static str = "notes";
///     const SCHEMA_VERSION: i32 = 2;
///     const FIELDS: &'static [&'static str] = &["title", "body", "label_ids"];
///     const UPCASTS: &'static [UpcastStep] = &[UpcastStep {
///         from: 1,
///         apply: rename_name_to_title,
///     }];
/// }
///
/// check_snapshot_type::<NoteRevision>().expect("a complete registry");
/// let note = NoteRevision { title: "Hírek".to_owned(), body: None, label_ids: vec![3] };
/// let stored = snapshot_object(&note).expect("the allow-list");
/// assert_eq!(Value::Object(stored), json!({"title": "Hírek", "body": null, "label_ids": [3]}));
/// // A snapshot written as version 1 reads in today's shape.
/// let old = json!({"name": "Hírek", "body": null, "label_ids": [3]});
/// assert_eq!(read_snapshot::<NoteRevision>(1, old), Ok(note));
/// ```
///
/// Every field is always serialised: an absent value is `null`, never a
/// missing key (no `skip_serializing_if`), so [`snapshot_object`] can check
/// the keys against [`RevisionSnapshot::FIELDS`].
pub trait RevisionSnapshot: Serialize + DeserializeOwned {
    /// The entity's table name, stored as `entity_type` on its revisions
    /// and activity entries, for example `posts` (ADR 0011 §4, ADR 0009).
    /// The model locks the record's row in this table before it writes a
    /// revision.
    const ENTITY_TYPE: &'static str;

    /// The version of this shape, stored as `schema_version`: 1 for the
    /// first shape, one more for each change of shape.
    const SCHEMA_VERSION: i32;

    /// The allow-list: every key of the serialised snapshot, each exactly
    /// once, in `snake_case` (the column and relation names). None of them
    /// is one of [`SECRET_FIELDS`].
    const FIELDS: &'static [&'static str];

    /// The upcaster registry: one step from each earlier version to the
    /// next, in order, so [`RevisionSnapshot::SCHEMA_VERSION`] `- 1` steps.
    /// A first version has none.
    const UPCASTS: &'static [UpcastStep] = &[];
}

/// A snapshot type whose constants break a rule of [`RevisionSnapshot`]: a
/// mistake of the code that defines it, found before any revision of it is
/// written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotTypeError {
    /// The entity type is not a `snake_case` table name.
    #[error(
        "the revision snapshot type's ENTITY_TYPE {entity_type:?} is not a table name: it starts \
         with a lowercase ASCII letter and holds only lowercase ASCII letters, digits and \
         underscores, like `posts` (ADR 0011 §4, ADR 0009)"
    )]
    EntityTypeNotSnakeCase {
        /// The refused entity type.
        entity_type: String,
    },
    /// The schema version is below 1.
    #[error(
        "the revision snapshot of {entity_type} has SCHEMA_VERSION {version}; versions start at \
         1 (ADR 0011 §4)"
    )]
    SchemaVersionBelowOne {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The refused version.
        version: i32,
    },
    /// The registry does not hold one step per version step.
    #[error(
        "the revision snapshot of {entity_type} is at SCHEMA_VERSION {version}, so its UPCASTS \
         needs {expected} steps (one from each earlier version to the next), but it has {steps} \
         (ADR 0011 §4, \"Schema changes\")"
    )]
    UpcastCount {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The snapshot type's version.
        version: i32,
        /// How many steps the version needs.
        expected: usize,
        /// How many the registry has.
        steps: usize,
    },
    /// A step of the registry is out of order.
    #[error(
        "step {position} of the revision snapshot of {entity_type} reads version {from}, but \
         the steps read versions 1, 2, 3, ... in order, so it must read version {expected}"
    )]
    UpcastOrder {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The step's position in the registry, from 0.
        position: usize,
        /// The version the step reads.
        from: i32,
        /// The version it must read.
        expected: i32,
    },
    /// The allow-list is empty.
    #[error(
        "the revision snapshot of {entity_type} lists no FIELDS; a snapshot holds every editable \
         field of its record (ADR 0011 §4)"
    )]
    NoFields {
        /// The snapshot type's entity type.
        entity_type: &'static str,
    },
    /// A field name is not `snake_case`.
    #[error(
        "the revision snapshot of {entity_type} lists the field {field:?}, which is not \
         snake_case like the column it stands for"
    )]
    FieldNotSnakeCase {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The refused field name.
        field: &'static str,
    },
    /// A field is listed twice.
    #[error("the revision snapshot of {entity_type} lists the field {field:?} twice")]
    DuplicateField {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The repeated field name.
        field: &'static str,
    },
    /// A field names a secret.
    #[error(
        "the revision snapshot of {entity_type} lists {field:?}, but a revision never holds a \
         secret (ADR 0011 §4, \"Never secrets\"); take it out of the snapshot type"
    )]
    SecretField {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The refused field name, one of [`SECRET_FIELDS`].
        field: &'static str,
    },
}

/// Checks the constants of the snapshot type `S` against the rules of
/// [`RevisionSnapshot`]: a `snake_case` entity type, a version from 1, one
/// upcaster step per version step in order, and a non-empty allow-list of
/// distinct `snake_case` names without a secret.
///
/// The model runs it before every write, and each snapshot type's unit test
/// runs it, so a gap in a registry fails the type's own tests first.
///
/// # Errors
///
/// Returns the first [`SnapshotTypeError`] it finds, in the order above.
pub fn check_snapshot_type<S: RevisionSnapshot>() -> Result<(), SnapshotTypeError> {
    let entity_type = S::ENTITY_TYPE;
    if !is_snake_case(entity_type) {
        return Err(SnapshotTypeError::EntityTypeNotSnakeCase {
            entity_type: entity_type.to_owned(),
        });
    }
    let version = S::SCHEMA_VERSION;
    let Some(expected) = version
        .checked_sub(1)
        .and_then(|steps| usize::try_from(steps).ok())
    else {
        return Err(SnapshotTypeError::SchemaVersionBelowOne {
            entity_type,
            version,
        });
    };
    if S::UPCASTS.len() != expected {
        return Err(SnapshotTypeError::UpcastCount {
            entity_type,
            version,
            expected,
            steps: S::UPCASTS.len(),
        });
    }
    // `zip` with `1..` pairs each step with the version it must read; the
    // count above keeps every expected version below SCHEMA_VERSION.
    for ((position, step), expected) in S::UPCASTS.iter().enumerate().zip(1..) {
        if step.from != expected {
            return Err(SnapshotTypeError::UpcastOrder {
                entity_type,
                position,
                from: step.from,
                expected,
            });
        }
    }
    if S::FIELDS.is_empty() {
        return Err(SnapshotTypeError::NoFields { entity_type });
    }
    let mut seen = BTreeSet::new();
    for &field in S::FIELDS {
        if !is_snake_case(field) {
            return Err(SnapshotTypeError::FieldNotSnakeCase { entity_type, field });
        }
        if SECRET_FIELDS.contains(&field) {
            return Err(SnapshotTypeError::SecretField { entity_type, field });
        }
        if !seen.insert(field) {
            return Err(SnapshotTypeError::DuplicateField { entity_type, field });
        }
    }
    Ok(())
}

/// Why a snapshot value cannot be stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    /// The snapshot type itself breaks a rule.
    #[error(transparent)]
    Type(#[from] SnapshotTypeError),
    /// serde could not serialise the value, for example a map with keys
    /// that are not strings.
    #[error("the revision snapshot of {entity_type} cannot be serialised: {message}")]
    Serialize {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// serde's message.
        message: String,
    },
    /// The value serialises to something other than a JSON object.
    #[error(
        "the revision snapshot of {entity_type} serialises to {found}, but a snapshot is a JSON \
         object of its fields (ADR 0011 §4); derive Serialize on a struct with named fields"
    )]
    NotAnObject {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// What it serialised to, for example "an array".
        found: &'static str,
    },
    /// The value has a key that the allow-list does not name.
    #[error(
        "the revision snapshot of {entity_type} holds {field:?}, which its FIELDS do not list; \
         list it there or leave it out of the snapshot type (ADR 0011 §4)"
    )]
    UnlistedField {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The unlisted key.
        field: String,
    },
    /// The value lacks a key that the allow-list names.
    #[error(
        "the revision snapshot of {entity_type} has no {field:?}, which its FIELDS list; a \
         snapshot holds every listed field, as null when it has no value"
    )]
    MissingField {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The missing key.
        field: &'static str,
    },
}

/// The JSON object a revision stores for `snapshot` (ADR 0011 §4).
///
/// - `snapshot`: the record after the save, as its snapshot type.
///
/// It checks the type first ([`check_snapshot_type`]), then that the
/// serialised object has exactly the keys of [`RevisionSnapshot::FIELDS`].
///
/// # Errors
///
/// Returns [`SnapshotError`] when the type breaks a rule, when serde fails,
/// when the value is not a JSON object, or when its keys and the allow-list
/// differ (an unlisted key first, then a missing one).
pub fn snapshot_object<S: RevisionSnapshot>(snapshot: &S) -> Result<SnapshotObject, SnapshotError> {
    check_snapshot_type::<S>()?;
    let entity_type = S::ENTITY_TYPE;
    let value = serde_json::to_value(snapshot).map_err(|error| SnapshotError::Serialize {
        entity_type,
        message: error.to_string(),
    })?;
    let Value::Object(object) = value else {
        return Err(SnapshotError::NotAnObject {
            entity_type,
            found: json_kind(&value),
        });
    };
    if let Some(field) = object.keys().find(|key| !S::FIELDS.contains(&key.as_str())) {
        return Err(SnapshotError::UnlistedField {
            entity_type,
            field: field.clone(),
        });
    }
    if let Some(&field) = S::FIELDS.iter().find(|field| !object.contains_key(**field)) {
        return Err(SnapshotError::MissingField { entity_type, field });
    }
    Ok(object)
}

/// Why a stored snapshot cannot be read in the current shape.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpcastError {
    /// The snapshot type itself breaks a rule.
    #[error(transparent)]
    Type(#[from] SnapshotTypeError),
    /// The stored version is below 1, which the table's CHECK refuses.
    #[error("a revision of {entity_type} has schema_version {version}; versions start at 1")]
    VersionBelowOne {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The stored version.
        version: i32,
    },
    /// The stored version is newer than this code's: a newer release wrote
    /// it, and this one has been rolled back to.
    #[error(
        "a revision of {entity_type} has schema_version {version}, but this release knows \
         versions up to {current}: a newer release wrote it"
    )]
    NewerVersion {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The stored version.
        version: i32,
        /// This code's version.
        current: i32,
    },
    /// The stored snapshot is not a JSON object, which the table's CHECK
    /// refuses.
    #[error("a revision of {entity_type} stores {found} as its snapshot, not a JSON object")]
    NotAnObject {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// What it stores, for example "an array".
        found: &'static str,
    },
    /// One step of the registry refused the snapshot.
    #[error(
        "the upcast of a revision of {entity_type} from version {from} to {to} failed: {source}",
        to = .from + 1
    )]
    Step {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// The version the failing step reads.
        from: i32,
        /// The step's refusal.
        source: UpcastStepError,
    },
    /// The upcast snapshot does not fit the current snapshot type.
    #[error(
        "a revision of {entity_type} does not fit the current snapshot type after its upcast: \
         {message}"
    )]
    Deserialize {
        /// The snapshot type's entity type.
        entity_type: &'static str,
        /// serde's message.
        message: String,
    },
}

/// Converts a stored snapshot of `version` into the shape of the current
/// [`RevisionSnapshot::SCHEMA_VERSION`] of `S`, by running the steps of its
/// registry from `version` up, in order. A snapshot of the current version
/// is returned unchanged.
///
/// - `version`: the revision's stored `schema_version`.
/// - `snapshot`: the revision's stored snapshot.
///
/// # Errors
///
/// Returns [`UpcastError`] when the type breaks a rule, when `version` is
/// below 1 or newer than the current version, or when a step refuses the
/// snapshot.
pub fn upcast<S: RevisionSnapshot>(
    version: i32,
    snapshot: SnapshotObject,
) -> Result<SnapshotObject, UpcastError> {
    check_snapshot_type::<S>()?;
    let entity_type = S::ENTITY_TYPE;
    check_stored_version::<S>(version)?;
    S::UPCASTS
        .iter()
        .filter(|step| step.from >= version)
        .try_fold(snapshot, |snapshot, step| {
            (step.apply)(snapshot).map_err(|source| UpcastError::Step {
                entity_type,
                from: step.from,
                source,
            })
        })
}

/// Reads a stored snapshot as the current snapshot type `S`: its object is
/// upcast from `version` ([`upcast`]) and then deserialised. The history's
/// revert loads a revision this way (ADR 0011 §4, "Revert").
///
/// - `version`: the revision's stored `schema_version`.
/// - `snapshot`: the revision's stored snapshot.
///
/// # Errors
///
/// Returns [`UpcastError`] when the stored value is not an object, when
/// [`upcast`] fails, or when the upcast object does not deserialise into `S`.
pub fn read_snapshot<S: RevisionSnapshot>(version: i32, snapshot: Value) -> Result<S, UpcastError> {
    let object = stored_object::<S>(snapshot)?;
    let current = upcast::<S>(version, object)?;
    serde_json::from_value(Value::Object(current)).map_err(|error| UpcastError::Deserialize {
        entity_type: S::ENTITY_TYPE,
        message: error.to_string(),
    })
}

/// A stored snapshot in the shape to compare with a new snapshot of `S`, to
/// name the fields an update changed ([`save_entry_details`]).
///
/// - `version`: the previous revision's stored `schema_version`.
/// - `snapshot`: its stored snapshot.
///
/// An older version is upcast ([`upcast`]). A newer one, written by a newer
/// release that this one was rolled back from, is compared as stored: no
/// step can convert it down, and refusing it would stop every save of the
/// record until the newer release returns. The fields that only one of the
/// two shapes has then count as changed, which is what happened to the
/// stored history.
///
/// # Errors
///
/// Returns [`UpcastError`] when the type breaks a rule, when the stored
/// value is not an object, when `version` is below 1, or when a step
/// refuses the snapshot.
pub fn comparable_snapshot<S: RevisionSnapshot>(
    version: i32,
    snapshot: Value,
) -> Result<SnapshotObject, UpcastError> {
    check_snapshot_type::<S>()?;
    let object = stored_object::<S>(snapshot)?;
    if version > S::SCHEMA_VERSION {
        return Ok(object);
    }
    upcast::<S>(version, object)
}

/// The object of a stored snapshot of `S`, or [`UpcastError::NotAnObject`].
fn stored_object<S: RevisionSnapshot>(snapshot: Value) -> Result<SnapshotObject, UpcastError> {
    match snapshot {
        Value::Object(object) => Ok(object),
        other => Err(UpcastError::NotAnObject {
            entity_type: S::ENTITY_TYPE,
            found: json_kind(&other),
        }),
    }
}

/// Refuses a stored version below 1 or above the current version of `S`.
fn check_stored_version<S: RevisionSnapshot>(version: i32) -> Result<(), UpcastError> {
    let entity_type = S::ENTITY_TYPE;
    if version < 1 {
        return Err(UpcastError::VersionBelowOne {
            entity_type,
            version,
        });
    }
    if version > S::SCHEMA_VERSION {
        return Err(UpcastError::NewerVersion {
            entity_type,
            version,
            current: S::SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// The kind of a JSON value, for an error message.
const fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Why a revision may not be written. Each is a mistake of the code that
/// writes it, so the message says which rule it broke.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RevisionError {
    /// An import reason from an actor that is not the importer.
    #[error(
        "a revision with reason `{reason}` is written only by the importer (Actor::Import), not \
         by the {actor} actor (ADR 0011 §4, \"Import\")",
        actor = .actor.kind()
    )]
    ImportReasonNeedsImporter {
        /// The refused reason.
        reason: Reason,
        /// The actor that tried.
        actor: Actor,
    },
    /// The importer with a reason other than `import` or `import_original`.
    #[error(
        "the importer writes only `import` and `import_original` revisions, not `{reason}` \
         (ADR 0011 §4, \"Import\")"
    )]
    ImporterWritesOnlyImport {
        /// The refused reason.
        reason: Reason,
    },
    /// A reason that only a record's first revision may have, on a record
    /// that already has revisions.
    #[error(
        "a `{reason}` revision is the first revision of its record, but the record already has \
         a `{previous}` revision (ADR 0011 §4)"
    )]
    NotFirst {
        /// The refused reason.
        reason: Reason,
        /// The reason of the record's latest revision.
        previous: Reason,
    },
    /// An `import` revision after anything but `import_original`.
    #[error(
        "an `import` revision is the record's first revision or follows its `import_original`, \
         but the latest revision is `{previous}` (ADR 0011 §4, \"Import\")"
    )]
    ImportAfter {
        /// The reason of the record's latest revision.
        previous: Reason,
    },
    /// A request id that is empty or longer than [`MAX_REQUEST_ID_CHARS`].
    #[error(
        "the revision has a request id of {chars} characters; a request id has 1 to \
         {MAX_REQUEST_ID_CHARS}, as Loco's request_id middleware makes them, or is left out"
    )]
    RequestIdLength {
        /// The length of the refused request id, in characters.
        chars: usize,
    },
    /// The record already has the highest revision number `integer` holds.
    #[error("the record already has revision {previous}, the highest number a revision can have")]
    NumberExhausted {
        /// The latest revision number.
        previous: i32,
    },
    /// An import reason given to the save path.
    #[error(
        "`{reason}` is not a save: the importer appends its revisions on their own and writes \
         one `import` entry per table (ADR 0011 §5), so record a save only for `create`, \
         `update` or `revert`"
    )]
    NotASave {
        /// The refused reason.
        reason: Reason,
    },
    /// Extra entry details that use the key of the changed field names.
    #[error(
        "the extra details of a save use the key {key:?}, which the entry's changed field names \
         take (ADR 0011 §5); use another key"
    )]
    ReservedDetailKey {
        /// The refused key, [`CHANGED_FIELDS_KEY`].
        key: &'static str,
    },
}

/// Checks the rules a revision meets before it is written, apart from its
/// snapshot ([`snapshot_object`]) and its number ([`next_revision_no`]).
///
/// - `actor`: who writes it.
/// - `reason`: why.
/// - `previous`: the reason of the record's latest revision, or `None` when
///   the record has none yet.
/// - `request_id`: the request id that groups the request's writes, if any.
///
/// The rules, from ADR 0011 §2 and §4:
///
/// - the two import reasons are the importer's and only the importer's
///   ([`Actor::Import`]);
/// - `create` and `import_original` are a record's first revision;
/// - `import` is the first revision or follows `import_original`;
/// - a request id has 1 to [`MAX_REQUEST_ID_CHARS`] characters.
///
/// `update` and `revert` may follow anything, also nothing: a record that
/// was written before its entity joined the safety net (an account made by
/// a task or a seed) gets its first revision at its first save.
///
/// # Errors
///
/// Returns the first [`RevisionError`] of the rules above, in that order.
pub fn check_revision(
    actor: Actor,
    reason: Reason,
    previous: Option<Reason>,
    request_id: Option<&str>,
) -> Result<(), RevisionError> {
    match (actor, reason.is_import()) {
        (Actor::Import, false) => return Err(RevisionError::ImporterWritesOnlyImport { reason }),
        (Actor::User(_) | Actor::System, true) => {
            return Err(RevisionError::ImportReasonNeedsImporter { reason, actor });
        }
        _ => {}
    }
    match (reason, previous) {
        (Reason::Create | Reason::ImportOriginal, Some(previous)) => {
            return Err(RevisionError::NotFirst { reason, previous });
        }
        (Reason::Import, Some(previous)) if previous != Reason::ImportOriginal => {
            return Err(RevisionError::ImportAfter { previous });
        }
        _ => {}
    }
    if let Some(request_id) = request_id {
        let chars = request_id.chars().count();
        if chars == 0 || chars > MAX_REQUEST_ID_CHARS {
            return Err(RevisionError::RequestIdLength { chars });
        }
    }
    Ok(())
}

/// The number of a record's next revision: 1 for its first, otherwise one
/// more than its latest (ADR 0011 §4).
///
/// - `previous`: the record's latest revision number, or `None`.
///
/// # Errors
///
/// Returns [`RevisionError::NumberExhausted`] when `previous` is already the
/// highest `integer`.
pub fn next_revision_no(previous: Option<i32>) -> Result<i32, RevisionError> {
    match previous {
        None => Ok(1),
        Some(previous) => previous
            .checked_add(1)
            .ok_or(RevisionError::NumberExhausted { previous }),
    }
}

/// The `details` of the activity entry that records a save (ADR 0011 §5).
///
/// - `reason`: the save's reason: `create`, `update` or `revert`.
/// - `before`: the record's latest earlier snapshot in the current shape
///   ([`comparable_snapshot`]), or `None` when it has none.
/// - `after`: the snapshot the save writes ([`snapshot_object`]).
/// - `extra`: facts the caller adds, such as a clone's source page id or
///   the revision a revert loaded.
///
/// A `create` entry holds only `extra`. An `update` or `revert` entry holds
/// the changed field names under [`CHANGED_FIELDS_KEY`]
/// ([`changed_field_names`], the names and never the values), with `extra`
/// beside them. Without an earlier snapshot, every field counts as changed.
/// `None` when there is nothing to hold.
///
/// # Errors
///
/// Returns [`RevisionError::NotASave`] for an import reason, and
/// [`RevisionError::ReservedDetailKey`] when `extra` uses
/// [`CHANGED_FIELDS_KEY`].
pub fn save_entry_details(
    reason: Reason,
    before: Option<&SnapshotObject>,
    after: &SnapshotObject,
    extra: Option<SnapshotObject>,
) -> Result<Option<SnapshotObject>, RevisionError> {
    if reason.save_action().is_none() {
        return Err(RevisionError::NotASave { reason });
    }
    if extra
        .as_ref()
        .is_some_and(|extra| extra.contains_key(CHANGED_FIELDS_KEY))
    {
        return Err(RevisionError::ReservedDetailKey {
            key: CHANGED_FIELDS_KEY,
        });
    }
    if reason == Reason::Create {
        return Ok(extra);
    }
    let empty = SnapshotObject::new();
    let mut details = changed_fields_details(&changed_field_names(before.unwrap_or(&empty), after));
    details.extend(extra.unwrap_or_default());
    Ok(Some(details))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    /// A first version: no upcasters.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Note {
        title: String,
        body: Option<String>,
        label_ids: Vec<i64>,
    }

    impl RevisionSnapshot for Note {
        const ENTITY_TYPE: &'static str = "notes";
        const SCHEMA_VERSION: i32 = 1;
        const FIELDS: &'static [&'static str] = &["title", "body", "label_ids"];
    }

    /// A third version. Version 1 was `{name, body}`; version 2 renamed
    /// `name` to `title`; version 3 added `pinned`, false for every older
    /// snapshot.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Card {
        title: String,
        body: Option<String>,
        pinned: bool,
    }

    fn card_v1_to_v2(mut v1: SnapshotObject) -> Result<SnapshotObject, UpcastStepError> {
        let name = v1.remove("name").ok_or(UpcastStepError::MissingField {
            field: "name".to_owned(),
        })?;
        if !name.is_string() {
            return Err(UpcastStepError::InvalidField {
                field: "name".to_owned(),
                expected: "a string",
            });
        }
        v1.insert("title".to_owned(), name);
        Ok(v1)
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "UpcastStep::apply has this signature, also for a step that cannot fail"
    )]
    fn card_v2_to_v3(mut v2: SnapshotObject) -> Result<SnapshotObject, UpcastStepError> {
        v2.insert("pinned".to_owned(), Value::Bool(false));
        Ok(v2)
    }

    impl RevisionSnapshot for Card {
        const ENTITY_TYPE: &'static str = "cards";
        const SCHEMA_VERSION: i32 = 3;
        const FIELDS: &'static [&'static str] = &["title", "body", "pinned"];
        const UPCASTS: &'static [UpcastStep] = &[
            UpcastStep {
                from: 1,
                apply: card_v1_to_v2,
            },
            UpcastStep {
                from: 2,
                apply: card_v2_to_v3,
            },
        ];
    }

    /// Defines a snapshot type that is a [`Note`] in every way but the
    /// constants given, for the refusals of [`check_snapshot_type`].
    macro_rules! note_type {
        ($name:ident, $entity:literal, $version:expr, $fields:expr, $upcasts:expr) => {
            #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
            struct $name {
                title: String,
                body: Option<String>,
                label_ids: Vec<i64>,
            }

            impl RevisionSnapshot for $name {
                const ENTITY_TYPE: &'static str = $entity;
                const SCHEMA_VERSION: i32 = $version;
                const FIELDS: &'static [&'static str] = $fields;
                const UPCASTS: &'static [UpcastStep] = $upcasts;
            }
        };
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "UpcastStep::apply has this signature, also for a step that cannot fail"
    )]
    fn identity(snapshot: SnapshotObject) -> Result<SnapshotObject, UpcastStepError> {
        Ok(snapshot)
    }

    const NOTE_FIELDS: &[&str] = &["title", "body", "label_ids"];
    const ONE_STEP: &[UpcastStep] = &[UpcastStep {
        from: 1,
        apply: identity,
    }];

    note_type!(BadEntity, "Notes", 1, NOTE_FIELDS, &[]);
    note_type!(BlankEntity, "", 1, NOTE_FIELDS, &[]);
    note_type!(VersionZero, "notes", 0, NOTE_FIELDS, &[]);
    note_type!(VersionNegative, "notes", -3, NOTE_FIELDS, &[]);
    note_type!(MissingStep, "notes", 2, NOTE_FIELDS, &[]);
    note_type!(ExtraStep, "notes", 1, NOTE_FIELDS, ONE_STEP);
    note_type!(
        StepOutOfOrder,
        "notes",
        3,
        NOTE_FIELDS,
        &[
            UpcastStep {
                from: 2,
                apply: identity,
            },
            UpcastStep {
                from: 1,
                apply: identity,
            },
        ]
    );
    note_type!(NoFields, "notes", 1, &[], &[]);
    note_type!(CamelField, "notes", 1, &["title", "body", "labelIds"], &[]);
    note_type!(
        DuplicateField,
        "notes",
        1,
        &["title", "body", "label_ids", "title"],
        &[]
    );
    note_type!(
        SecretField,
        "notes",
        1,
        &["title", "body", "label_ids", "password_hash"],
        &[]
    );
    // The allow-list and the struct disagree.
    note_type!(ListsTooFew, "notes", 1, &["title", "body"], &[]);
    note_type!(
        ListsTooMany,
        "notes",
        1,
        &["title", "body", "label_ids", "pinned"],
        &[]
    );

    /// A snapshot type whose value is not an object.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Bare(String);

    impl RevisionSnapshot for Bare {
        const ENTITY_TYPE: &'static str = "bares";
        const SCHEMA_VERSION: i32 = 1;
        const FIELDS: &'static [&'static str] = &["value"];
    }

    /// A snapshot type that serde cannot serialise: a map with keys that
    /// are not strings.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Keyed {
        positions: std::collections::BTreeMap<(i64, i64), i64>,
    }

    impl RevisionSnapshot for Keyed {
        const ENTITY_TYPE: &'static str = "keyeds";
        const SCHEMA_VERSION: i32 = 1;
        const FIELDS: &'static [&'static str] = &["positions"];
    }

    fn object(value: &Value) -> SnapshotObject {
        value.as_object().expect("an object").clone()
    }

    fn note() -> Note {
        Note {
            title: "Hírek".to_owned(),
            body: Some("<p>Első</p>".to_owned()),
            label_ids: vec![3, 1],
        }
    }

    fn card_v1() -> Value {
        json!({"name": "Kártya", "body": null})
    }

    fn arb_note() -> impl Strategy<Value = Note> {
        (
            any::<String>(),
            proptest::option::of(any::<String>()),
            proptest::collection::vec(any::<i64>(), 0..8),
        )
            .prop_map(|(title, body, label_ids)| Note {
                title,
                body,
                label_ids,
            })
    }

    fn arb_actor() -> impl Strategy<Value = Actor> {
        prop_oneof![
            any::<i64>().prop_map(Actor::User),
            Just(Actor::System),
            Just(Actor::Import),
        ]
    }

    fn arb_reason() -> impl Strategy<Value = Reason> {
        proptest::sample::select(Reason::ALL.to_vec())
    }

    /// Small JSON objects with `snake_case` keys.
    fn arb_object() -> impl Strategy<Value = SnapshotObject> {
        proptest::collection::btree_map(
            "[a-z][a-z_]{0,6}",
            prop_oneof![
                Just(Value::Null),
                any::<bool>().prop_map(Value::Bool),
                any::<i64>().prop_map(Value::from),
                "[a-zA-Z ]{0,8}".prop_map(Value::String),
            ],
            0..6,
        )
        .prop_map(|map| map.into_iter().collect())
    }

    #[test]
    fn the_reasons_are_those_of_adr_0011_in_order() {
        let names: Vec<&str> = Reason::ALL.iter().map(|reason| reason.as_str()).collect();
        assert_eq!(
            names,
            ["create", "update", "revert", "import", "import_original"]
        );
        for reason in Reason::ALL {
            assert_eq!(reason.to_string(), reason.as_str());
            assert_eq!(reason.as_str().parse(), Ok(reason));
        }
    }

    #[test]
    fn only_the_two_import_reasons_are_import_reasons() {
        for reason in Reason::ALL {
            assert_eq!(
                reason.is_import(),
                matches!(reason, Reason::Import | Reason::ImportOriginal),
                "{reason}"
            );
        }
    }

    #[test]
    fn each_save_reason_has_its_action_and_an_import_has_none() {
        assert_eq!(Reason::Create.save_action(), Some(Action::Create));
        assert_eq!(Reason::Update.save_action(), Some(Action::Update));
        assert_eq!(Reason::Revert.save_action(), Some(Action::Revert));
        assert_eq!(Reason::Import.save_action(), None);
        assert_eq!(Reason::ImportOriginal.save_action(), None);
    }

    #[test]
    fn an_unknown_reason_is_refused_by_name() {
        for name in ["", "Create", "delete", "import-original", " update"] {
            assert_eq!(
                name.parse::<Reason>(),
                Err(UnknownReason {
                    name: name.to_owned()
                })
            );
        }
        assert_eq!(
            "purge".parse::<Reason>().unwrap_err().to_string(),
            "\"purge\" is not a revision reason (ADR 0011 §4)"
        );
    }

    #[test]
    fn well_formed_snapshot_types_pass() {
        assert_eq!(check_snapshot_type::<Note>(), Ok(()));
        assert_eq!(check_snapshot_type::<Card>(), Ok(()));
    }

    #[test]
    fn each_broken_snapshot_type_is_refused_with_its_rule() {
        assert_eq!(
            check_snapshot_type::<BadEntity>(),
            Err(SnapshotTypeError::EntityTypeNotSnakeCase {
                entity_type: "Notes".to_owned()
            })
        );
        assert_eq!(
            check_snapshot_type::<BlankEntity>(),
            Err(SnapshotTypeError::EntityTypeNotSnakeCase {
                entity_type: String::new()
            })
        );
        for (refusal, version) in [
            (check_snapshot_type::<VersionZero>(), 0),
            (check_snapshot_type::<VersionNegative>(), -3),
        ] {
            assert_eq!(
                refusal,
                Err(SnapshotTypeError::SchemaVersionBelowOne {
                    entity_type: "notes",
                    version
                })
            );
        }
        assert_eq!(
            check_snapshot_type::<MissingStep>(),
            Err(SnapshotTypeError::UpcastCount {
                entity_type: "notes",
                version: 2,
                expected: 1,
                steps: 0
            })
        );
        assert_eq!(
            check_snapshot_type::<ExtraStep>(),
            Err(SnapshotTypeError::UpcastCount {
                entity_type: "notes",
                version: 1,
                expected: 0,
                steps: 1
            })
        );
        assert_eq!(
            check_snapshot_type::<StepOutOfOrder>(),
            Err(SnapshotTypeError::UpcastOrder {
                entity_type: "notes",
                position: 0,
                from: 2,
                expected: 1
            })
        );
        assert_eq!(
            check_snapshot_type::<NoFields>(),
            Err(SnapshotTypeError::NoFields {
                entity_type: "notes"
            })
        );
        assert_eq!(
            check_snapshot_type::<CamelField>(),
            Err(SnapshotTypeError::FieldNotSnakeCase {
                entity_type: "notes",
                field: "labelIds"
            })
        );
        assert_eq!(
            check_snapshot_type::<DuplicateField>(),
            Err(SnapshotTypeError::DuplicateField {
                entity_type: "notes",
                field: "title"
            })
        );
        assert_eq!(
            check_snapshot_type::<SecretField>(),
            Err(SnapshotTypeError::SecretField {
                entity_type: "notes",
                field: "password_hash"
            })
        );
    }

    #[test]
    fn the_refusals_say_what_to_fix() {
        assert_eq!(
            check_snapshot_type::<MissingStep>()
                .unwrap_err()
                .to_string(),
            "the revision snapshot of notes is at SCHEMA_VERSION 2, so its UPCASTS needs 1 steps \
             (one from each earlier version to the next), but it has 0 (ADR 0011 §4, \"Schema \
             changes\")"
        );
        assert_eq!(
            check_snapshot_type::<SecretField>()
                .unwrap_err()
                .to_string(),
            "the revision snapshot of notes lists \"password_hash\", but a revision never holds \
             a secret (ADR 0011 §4, \"Never secrets\"); take it out of the snapshot type"
        );
    }

    /// Checks a type that is a [`Note`] in every way but its allow-list,
    /// `["title", <field>]`.
    macro_rules! one_field_type {
        ($name:ident, $field:literal) => {{
            note_type!($name, "notes", 1, &["title", $field], &[]);
            check_snapshot_type::<$name>()
        }};
    }

    #[test]
    fn every_secret_field_is_refused() {
        // Each name of SECRET_FIELDS, in its order, listed in an otherwise
        // valid type: the column or table name, then the shorter spelling.
        let refusals = [
            one_field_type!(Hash, "password_hash"),
            one_field_type!(Password, "password"),
            one_field_type!(TotpColumn, "totp_secret_encrypted"),
            one_field_type!(Totp, "totp_secret"),
            one_field_type!(Version, "token_version"),
            one_field_type!(ResetTable, "password_reset_tokens"),
            one_field_type!(Reset, "reset_tokens"),
            one_field_type!(CodesTable, "user_recovery_codes"),
            one_field_type!(Codes, "recovery_codes"),
        ];
        assert_eq!(
            refusals.len(),
            SECRET_FIELDS.len(),
            "one case for each name of SECRET_FIELDS"
        );
        for (refusal, &field) in refusals.into_iter().zip(SECRET_FIELDS) {
            assert_eq!(
                refusal,
                Err(SnapshotTypeError::SecretField {
                    entity_type: "notes",
                    field
                })
            );
        }
    }

    #[test]
    fn account_columns_that_name_no_secret_pass_the_secret_guard() {
        // The guard compares whole names, so a column that only contains a
        // secret's word is not refused (SECRET_FIELDS says why).
        assert_eq!(one_field_type!(MustChange, "must_change_password"), Ok(()));
        assert_eq!(
            one_field_type!(TempExpiry, "temp_password_expires_at"),
            Ok(())
        );
        assert_eq!(one_field_type!(TotpSince, "totp_enabled_at"), Ok(()));
    }

    #[test]
    fn a_snapshot_object_holds_exactly_the_listed_fields() {
        assert_eq!(
            Value::Object(snapshot_object(&note()).expect("valid")),
            json!({"title": "Hírek", "body": "<p>Első</p>", "label_ids": [3, 1]})
        );
        // A field without a value is null, never left out.
        let empty = Note {
            body: None,
            ..note()
        };
        assert_eq!(
            snapshot_object(&empty).expect("valid").get("body"),
            Some(&Value::Null)
        );
    }

    #[test]
    fn a_snapshot_that_differs_from_its_allow_list_is_refused() {
        let value = ListsTooFew {
            title: "Hírek".to_owned(),
            body: None,
            label_ids: vec![],
        };
        assert_eq!(
            snapshot_object(&value),
            Err(SnapshotError::UnlistedField {
                entity_type: "notes",
                field: "label_ids".to_owned()
            })
        );
        let value = ListsTooMany {
            title: "Hírek".to_owned(),
            body: None,
            label_ids: vec![],
        };
        assert_eq!(
            snapshot_object(&value),
            Err(SnapshotError::MissingField {
                entity_type: "notes",
                field: "pinned"
            })
        );
        assert_eq!(
            snapshot_object(&Bare("x".to_owned())),
            Err(SnapshotError::NotAnObject {
                entity_type: "bares",
                found: "a string"
            })
        );
        let keyed = Keyed {
            positions: [((1, 2), 3)].into_iter().collect(),
        };
        assert!(matches!(
            snapshot_object(&keyed),
            Err(SnapshotError::Serialize {
                entity_type: "keyeds",
                ..
            })
        ));
        // A broken type is refused before its value is looked at.
        let value = SecretField {
            title: "Hírek".to_owned(),
            body: None,
            label_ids: vec![],
        };
        assert_eq!(
            snapshot_object(&value),
            Err(SnapshotError::Type(SnapshotTypeError::SecretField {
                entity_type: "notes",
                field: "password_hash"
            }))
        );
    }

    #[test]
    fn card_step_1_renames_name_to_title() {
        assert_eq!(
            card_v1_to_v2(object(&card_v1())),
            Ok(object(&json!({"title": "Kártya", "body": null})))
        );
        assert_eq!(
            card_v1_to_v2(object(&json!({"body": null}))),
            Err(UpcastStepError::MissingField {
                field: "name".to_owned()
            })
        );
        assert_eq!(
            card_v1_to_v2(object(&json!({"name": 7, "body": null}))),
            Err(UpcastStepError::InvalidField {
                field: "name".to_owned(),
                expected: "a string"
            })
        );
    }

    #[test]
    fn card_step_2_adds_pinned_as_false() {
        assert_eq!(
            card_v2_to_v3(object(&json!({"title": "Kártya", "body": "x"}))),
            Ok(object(
                &json!({"title": "Kártya", "body": "x", "pinned": false})
            ))
        );
    }

    #[test]
    fn upcast_runs_the_steps_from_the_stored_version_up() {
        let current = json!({"title": "Kártya", "body": null, "pinned": false});
        assert_eq!(upcast::<Card>(1, object(&card_v1())), Ok(object(&current)));
        assert_eq!(
            upcast::<Card>(2, object(&json!({"title": "Kártya", "body": null}))),
            Ok(object(&current))
        );
        // The current version is returned as stored.
        let pinned = json!({"title": "Kártya", "body": null, "pinned": true});
        assert_eq!(upcast::<Card>(3, object(&pinned)), Ok(object(&pinned)));
        assert_eq!(
            read_snapshot::<Card>(1, card_v1()),
            Ok(Card {
                title: "Kártya".to_owned(),
                body: None,
                pinned: false
            })
        );
    }

    #[test]
    fn upcast_refuses_what_no_step_can_read() {
        for version in [0, -1, i32::MIN] {
            assert_eq!(
                upcast::<Card>(version, object(&card_v1())),
                Err(UpcastError::VersionBelowOne {
                    entity_type: "cards",
                    version
                })
            );
        }
        assert_eq!(
            upcast::<Card>(4, object(&card_v1())),
            Err(UpcastError::NewerVersion {
                entity_type: "cards",
                version: 4,
                current: 3
            })
        );
        let error = upcast::<Card>(1, object(&json!({"body": null}))).unwrap_err();
        assert_eq!(
            error,
            UpcastError::Step {
                entity_type: "cards",
                from: 1,
                source: UpcastStepError::MissingField {
                    field: "name".to_owned()
                }
            }
        );
        assert_eq!(
            error.to_string(),
            "the upcast of a revision of cards from version 1 to 2 failed: the field \"name\" is \
             missing"
        );
        assert_eq!(
            upcast::<MissingStep>(1, object(&card_v1())),
            Err(UpcastError::Type(SnapshotTypeError::UpcastCount {
                entity_type: "notes",
                version: 2,
                expected: 1,
                steps: 0
            }))
        );
    }

    #[test]
    fn read_snapshot_refuses_a_value_that_is_not_its_type() {
        assert_eq!(
            read_snapshot::<Card>(3, json!(["Kártya"])),
            Err(UpcastError::NotAnObject {
                entity_type: "cards",
                found: "an array"
            })
        );
        assert!(matches!(
            read_snapshot::<Card>(3, json!({"title": 7, "body": null, "pinned": false})),
            Err(UpcastError::Deserialize {
                entity_type: "cards",
                ..
            })
        ));
    }

    #[test]
    fn a_newer_snapshot_is_compared_as_stored() {
        let newer = json!({"title": "Kártya", "body": null, "pinned": true, "color": "#ff0000"});
        assert_eq!(
            comparable_snapshot::<Card>(4, newer.clone()),
            Ok(object(&newer))
        );
        assert_eq!(
            comparable_snapshot::<Card>(1, card_v1()),
            Ok(object(
                &json!({"title": "Kártya", "body": null, "pinned": false})
            ))
        );
        assert_eq!(
            comparable_snapshot::<Card>(0, card_v1()),
            Err(UpcastError::VersionBelowOne {
                entity_type: "cards",
                version: 0
            })
        );
        assert_eq!(
            comparable_snapshot::<Card>(5, json!(null)),
            Err(UpcastError::NotAnObject {
                entity_type: "cards",
                found: "null"
            })
        );
    }

    #[test]
    fn the_import_reasons_are_the_importers_alone() {
        for reason in [Reason::Import, Reason::ImportOriginal] {
            assert_eq!(check_revision(Actor::Import, reason, None, None), Ok(()));
            for actor in [Actor::User(4), Actor::System] {
                assert_eq!(
                    check_revision(actor, reason, None, None),
                    Err(RevisionError::ImportReasonNeedsImporter { reason, actor })
                );
            }
        }
        for reason in [Reason::Create, Reason::Update, Reason::Revert] {
            assert_eq!(
                check_revision(Actor::Import, reason, None, None),
                Err(RevisionError::ImporterWritesOnlyImport { reason })
            );
            for actor in [Actor::User(4), Actor::System] {
                assert_eq!(check_revision(actor, reason, None, None), Ok(()));
            }
        }
        assert_eq!(
            check_revision(Actor::System, Reason::Import, None, None)
                .unwrap_err()
                .to_string(),
            "a revision with reason `import` is written only by the importer (Actor::Import), \
             not by the system actor (ADR 0011 §4, \"Import\")"
        );
    }

    #[test]
    fn create_and_import_original_come_first_and_import_follows_its_original() {
        for previous in Reason::ALL {
            assert_eq!(
                check_revision(Actor::System, Reason::Create, Some(previous), None),
                Err(RevisionError::NotFirst {
                    reason: Reason::Create,
                    previous
                })
            );
            assert_eq!(
                check_revision(Actor::Import, Reason::ImportOriginal, Some(previous), None),
                Err(RevisionError::NotFirst {
                    reason: Reason::ImportOriginal,
                    previous
                })
            );
            let import = check_revision(Actor::Import, Reason::Import, Some(previous), None);
            if previous == Reason::ImportOriginal {
                assert_eq!(import, Ok(()));
            } else {
                assert_eq!(import, Err(RevisionError::ImportAfter { previous }));
            }
            for reason in [Reason::Update, Reason::Revert] {
                assert_eq!(
                    check_revision(Actor::User(1), reason, Some(previous), None),
                    Ok(())
                );
            }
        }
    }

    #[test]
    fn a_request_id_has_1_to_255_characters() {
        let at_limit = "é".repeat(MAX_REQUEST_ID_CHARS);
        assert_eq!(
            check_revision(Actor::System, Reason::Update, None, Some(&at_limit)),
            Ok(())
        );
        for (request_id, chars) in [(String::new(), 0), (format!("{at_limit}x"), 256)] {
            assert_eq!(
                check_revision(Actor::System, Reason::Update, None, Some(&request_id)),
                Err(RevisionError::RequestIdLength { chars })
            );
        }
    }

    #[test]
    fn revision_numbers_count_from_one() {
        assert_eq!(next_revision_no(None), Ok(1));
        assert_eq!(next_revision_no(Some(1)), Ok(2));
        assert_eq!(
            next_revision_no(Some(i32::MAX)),
            Err(RevisionError::NumberExhausted { previous: i32::MAX })
        );
    }

    #[test]
    fn a_create_entry_holds_only_the_extra_details() {
        let after = snapshot_object(&note()).expect("valid");
        assert_eq!(
            save_entry_details(Reason::Create, None, &after, None),
            Ok(None)
        );
        let extra = object(&json!({"cloned_from": 12}));
        assert_eq!(
            save_entry_details(Reason::Create, None, &after, Some(extra.clone())),
            Ok(Some(extra))
        );
    }

    #[test]
    fn an_update_entry_names_the_changed_fields() {
        let before = snapshot_object(&note()).expect("valid");
        let after = snapshot_object(&Note {
            title: "Más".to_owned(),
            label_ids: vec![1, 3],
            ..note()
        })
        .expect("valid");
        for reason in [Reason::Update, Reason::Revert] {
            assert_eq!(
                save_entry_details(reason, Some(&before), &after, None),
                Ok(Some(object(
                    &json!({"changed_fields": ["label_ids", "title"]})
                )))
            );
        }
        // Without an earlier snapshot every field counts, and the extra
        // details sit beside the names.
        assert_eq!(
            save_entry_details(
                Reason::Revert,
                None,
                &after,
                Some(object(&json!({"revert_of": 2})))
            ),
            Ok(Some(object(&json!({
                "changed_fields": ["body", "label_ids", "title"],
                "revert_of": 2
            }))))
        );
        // An unchanged save lists no field.
        assert_eq!(
            save_entry_details(Reason::Update, Some(&before), &before, None),
            Ok(Some(object(&json!({"changed_fields": []}))))
        );
    }

    #[test]
    fn save_entry_details_refuses_an_import_and_the_reserved_key() {
        let after = snapshot_object(&note()).expect("valid");
        for reason in [Reason::Import, Reason::ImportOriginal] {
            assert_eq!(
                save_entry_details(reason, None, &after, None),
                Err(RevisionError::NotASave { reason })
            );
        }
        for reason in [Reason::Create, Reason::Update] {
            assert_eq!(
                save_entry_details(
                    reason,
                    None,
                    &after,
                    Some(object(&json!({"changed_fields": ["x"]})))
                ),
                Err(RevisionError::ReservedDetailKey {
                    key: CHANGED_FIELDS_KEY
                })
            );
        }
    }

    proptest! {
        #[test]
        fn a_reason_reads_back_from_its_name(reason in arb_reason()) {
            prop_assert_eq!(reason.as_str().parse::<Reason>(), Ok(reason));
        }

        #[test]
        fn any_other_name_is_not_a_reason(name in ".{0,24}") {
            let known = Reason::ALL.iter().any(|reason| reason.as_str() == name);
            prop_assert_eq!(name.parse::<Reason>().is_ok(), known);
        }

        #[test]
        fn a_snapshot_round_trips_through_its_object(note in arb_note()) {
            let object = snapshot_object(&note).expect("valid");
            let keys: Vec<&str> = object.keys().map(String::as_str).collect();
            let mut listed = Note::FIELDS.to_vec();
            listed.sort_unstable();
            let mut keys_sorted = keys.clone();
            keys_sorted.sort_unstable();
            prop_assert_eq!(keys_sorted, listed);
            prop_assert_eq!(read_snapshot::<Note>(1, Value::Object(object)), Ok(note));
        }

        #[test]
        fn upcasting_the_current_version_changes_nothing(object in arb_object()) {
            prop_assert_eq!(upcast::<Card>(3, object.clone()), Ok(object.clone()));
            prop_assert_eq!(upcast::<Note>(1, object.clone()), Ok(object));
        }

        #[test]
        fn a_version_1_card_reads_as_version_3(name in any::<String>(), body in proptest::option::of(any::<String>())) {
            let v1 = json!({"name": name, "body": body});
            prop_assert_eq!(
                read_snapshot::<Card>(1, v1),
                Ok(Card { title: name, body, pinned: false })
            );
        }

        #[test]
        fn a_version_outside_the_registry_is_refused(version in any::<i32>(), object in arb_object()) {
            let outcome = upcast::<Card>(version, object);
            if (1..=3).contains(&version) {
                // A known version may still fail a step, but never on its
                // version. (A binding, because prop_assert! would read the
                // braces of the pattern as a format string.)
                let refused_version = matches!(
                    outcome,
                    Err(UpcastError::VersionBelowOne { .. } | UpcastError::NewerVersion { .. })
                );
                prop_assert!(!refused_version, "version {} was refused", version);
            } else if version < 1 {
                prop_assert_eq!(
                    outcome,
                    Err(UpcastError::VersionBelowOne { entity_type: "cards", version })
                );
            } else {
                prop_assert_eq!(
                    outcome,
                    Err(UpcastError::NewerVersion { entity_type: "cards", version, current: 3 })
                );
            }
        }

        #[test]
        fn a_newer_snapshot_compares_as_stored(version in 4..=i32::MAX, object in arb_object()) {
            prop_assert_eq!(
                comparable_snapshot::<Card>(version, Value::Object(object.clone())),
                Ok(object)
            );
        }

        #[test]
        fn check_revision_pairs_the_importer_with_the_import_reasons(
            actor in arb_actor(),
            reason in arb_reason(),
        ) {
            let paired = (actor == Actor::Import) == reason.is_import();
            prop_assert_eq!(check_revision(actor, reason, None, None).is_ok(), paired);
        }

        #[test]
        fn check_revision_accepts_every_request_id_of_1_to_255_characters(
            request_id in ".{1,255}",
            actor in arb_actor(),
        ) {
            let reason = if actor == Actor::Import { Reason::Import } else { Reason::Update };
            prop_assert_eq!(check_revision(actor, reason, None, Some(&request_id)), Ok(()));
        }

        #[test]
        fn the_next_number_is_one_more(previous in 0..i32::MAX) {
            prop_assert_eq!(next_revision_no(Some(previous)), Ok(previous + 1));
        }

        #[test]
        fn an_update_entry_lists_exactly_the_changed_names(
            before in arb_object(),
            after in arb_object(),
        ) {
            let details = save_entry_details(Reason::Update, Some(&before), &after, None)
                .expect("a save")
                .expect("details");
            prop_assert_eq!(details.len(), 1);
            let expected = changed_fields_details(&changed_field_names(&before, &after));
            prop_assert_eq!(details, expected);
        }

        #[test]
        fn extra_details_survive_beside_the_changed_names(
            after in arb_object(),
            extra in arb_object(),
        ) {
            prop_assume!(!extra.contains_key(CHANGED_FIELDS_KEY));
            let details = save_entry_details(Reason::Update, None, &after, Some(extra.clone()))
                .expect("a save")
                .expect("details");
            for (key, value) in &extra {
                prop_assert_eq!(details.get(key), Some(value));
            }
            prop_assert!(details.contains_key(CHANGED_FIELDS_KEY));
        }
    }
}
