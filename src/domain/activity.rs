//! The pure parts of the activity log (ADR 0011 §5): who acted ([`Actor`]),
//! what they did ([`Action`]), the rules an entry meets before it is written
//! ([`check_entry`]), and the changed-field list of an update entry
//! ([`changed_field_names`], [`changed_fields_details`]).
//!
//! The table is append-only: once written, an entry can never be corrected,
//! and only its `ip` can ever be cleared (ADR 0011 §5, §6). So every rule
//! that keeps a wrong or a personal value out of the log runs before the
//! insert, here, where it is tested without a database. The model
//! (`src/models/activity_log.rs`) calls [`check_entry`] and writes the
//! stored strings of [`Actor::kind`] and [`Action::as_str`]. The table's
//! CHECKs repeat the shape rules and the two personal-data rules (an `ip`
//! only on an account action, a label only about a content record), so an
//! INSERT that bypasses the model cannot store an address or a name either;
//! the migration `m20261031_223930_activity_log` lists which rules it
//! repeats.
//!
//! The action list is extensible: a later ADR adds an action as a new
//! variant here, in [`Action::ALL`], and in a migration that replaces the
//! `action` CHECK of the table (ADR 0009, "Column types"). Nothing else
//! matches on the full list, and the model test that compares the CHECK
//! with [`Action::ALL`] fails until all three agree.

use std::{fmt, str::FromStr};

use serde_json::{Map, Value};

/// Who did what an entry records (ADR 0011 §2, §5): stored as `actor_kind`,
/// with `actor_id` set only for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Actor {
    /// A signed-in admin user, by `users.id`.
    User(i64),
    /// backend3 itself: `CronJob` tasks, queue jobs and the other work no user
    /// started (ADR 0011 §2).
    System,
    /// The legacy importer (ADR 0017).
    Import,
}

/// The `actor_kind` values, in the order of the table's CHECK.
pub const ACTOR_KINDS: [&str; 3] = ["user", "system", "import"];

impl Actor {
    /// The stored `actor_kind`: `user`, `system` or `import`.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::User(_) => ACTOR_KINDS[0],
            Self::System => ACTOR_KINDS[1],
            Self::Import => ACTOR_KINDS[2],
        }
    }

    /// The stored `actor_id`: the user's id, or `None` for the system and
    /// the importer.
    #[must_use]
    pub const fn user_id(self) -> Option<i64> {
        match self {
            Self::User(id) => Some(id),
            Self::System | Self::Import => None,
        }
    }

    /// The actor of a stored entry, read back from its two columns.
    ///
    /// - `kind`: the stored `actor_kind`.
    /// - `user_id`: the stored `actor_id`.
    ///
    /// # Errors
    ///
    /// Returns [`StoredActorError`] for a kind that is not one of
    /// [`ACTOR_KINDS`], a user without an id, or a system or import entry
    /// with one. The table's CHECKs refuse all three, so an error means the
    /// row did not come from this table.
    pub fn from_stored(kind: &str, user_id: Option<i64>) -> Result<Self, StoredActorError> {
        match (kind, user_id) {
            ("user", Some(id)) => Ok(Self::User(id)),
            ("system", None) => Ok(Self::System),
            ("import", None) => Ok(Self::Import),
            _ => Err(StoredActorError {
                kind: kind.to_owned(),
                user_id,
            }),
        }
    }
}

/// Stored actor columns that name no [`Actor`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "the stored actor (actor_kind {kind:?}, actor_id {user_id:?}) is not one of: a user with \
     an id, the system without one, the importer without one (ADR 0011 §5)"
)]
pub struct StoredActorError {
    /// The stored `actor_kind`.
    pub kind: String,
    /// The stored `actor_id`.
    pub user_id: Option<i64>,
}

/// The three groups of ADR 0011 §5's action table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionGroup {
    /// Changes to content records.
    Content,
    /// Sign-in and account events (ADR 0006). Only these entries may hold
    /// the client address (`ip`).
    Account,
    /// Media-library operations (ADR 0015).
    Media,
}

/// Defines [`Action`], its stored names and its group, from one table, so
/// that the variant list, [`Action::ALL`] and the two `match`es cannot
/// drift apart.
macro_rules! actions {
    ($($group:ident { $($variant:ident => $name:literal, $doc:literal;)+ })+) => {
        /// What an entry records: every action of ADR 0011 §5, stored as
        /// `action`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Action {
            $($(#[doc = $doc] $variant,)+)+
        }

        impl Action {
            /// Every action, in the order of ADR 0011 §5's table (content,
            /// accounts, media), which is also the order of the table's
            /// `action` CHECK.
            pub const ALL: &'static [Self] = &[$($(Self::$variant,)+)+];

            /// The stored name, in `snake_case`, as ADR 0011 §5 spells it.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($(Self::$variant => $name,)+)+
                }
            }

            /// The group of ADR 0011 §5's table that the action is in.
            #[must_use]
            pub const fn group(self) -> ActionGroup {
                match self {
                    $($(Self::$variant => ActionGroup::$group,)+)+
                }
            }
        }
    };
}

actions! {
    Content {
        Create => "create", "A record was created; a page Clone is a `create` with the source page id in `details`.";
        Update => "update", "A record was changed; `details` names the changed fields.";
        Revert => "revert", "A record was saved from an earlier revision.";
        Delete => "delete", "A record was moved to the trash.";
        Restore => "restore", "A record was restored from the trash.";
        Purge => "purge", "A record was deleted permanently.";
        Publish => "publish", "A record was published.";
        Unpublish => "unpublish", "A record was unpublished.";
        Reorder => "reorder", "Records were put in a new order.";
        PreviewLinkRegenerated => "preview_link_regenerated", "A draft's preview link was replaced.";
        Import => "import", "The importer wrote a table; one entry per table per run, with counts.";
    }
    Account {
        Login => "login", "A user signed in.";
        LoginFailed => "login_failed", "A sign-in of a known account failed.";
        Logout => "logout", "A user signed out.";
        PasswordChanged => "password_changed", "A password was changed.";
        ResetLinkIssued => "reset_link_issued", "An admin issued a password reset link.";
        TemporaryPasswordIssued => "temporary_password_issued", "An admin issued a temporary password.";
        TotpEnabled => "totp_enabled", "Two-factor sign-in was turned on.";
        TotpDisabled => "totp_disabled", "Two-factor sign-in was turned off.";
        TotpReset => "totp_reset", "An admin reset an account's two-factor sign-in.";
        RecoveryCodeUsed => "recovery_code_used", "A recovery code was used to sign in.";
        RecoveryCodesRegenerated => "recovery_codes_regenerated", "A new set of recovery codes replaced the old one.";
        AccountUnlocked => "account_unlocked", "An admin cleared an account's sign-in lock.";
        UserActivated => "user_activated", "An account was activated.";
        UserDeactivated => "user_deactivated", "An account was deactivated.";
        RolesChanged => "roles_changed", "An account's roles were changed.";
        GrantsChanged => "grants_changed", "An account's extra grants were changed.";
    }
    Media {
        MediaUpload => "media_upload", "A file was uploaded to the media library.";
        MediaRename => "media_rename", "A media file was renamed.";
        MediaMove => "media_move", "A media file was moved to another folder.";
        MediaDelete => "media_delete", "A media file was moved to the media trash.";
        MediaRestore => "media_restore", "A media file was restored from the media trash.";
        MediaPurge => "media_purge", "A media file was deleted permanently.";
    }
}

impl fmt::Display for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Action {
    type Err = UnknownAction;

    /// Reads a stored `action` back.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|action| action.as_str() == name)
            .ok_or_else(|| UnknownAction {
                name: name.to_owned(),
            })
    }
}

/// A stored `action` that names no [`Action`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{name:?} is not an action of the activity log (ADR 0011 §5)")]
pub struct UnknownAction {
    /// The refused name.
    pub name: String,
}

/// The entity types whose records are people: their entries keep only the
/// id and never a label, so the log holds no names (ADR 0011 §5, `label`).
pub const PEOPLE_ENTITY_TYPES: [&str; 3] = ["users", "authors", "colleagues"];

/// The longest request id an entry takes, in characters: the limit of the
/// request id that Loco's `request_id` middleware hands to handlers
/// (loco-rs 1.2.0 `src/controller/middleware/request_id.rs`, `MAX_LEN`).
pub const MAX_REQUEST_ID_CHARS: usize = 255;

/// What [`check_entry`] needs to know about an entry: its parts as plain
/// values, without the actor (every actor may write every entry) and without
/// the values the checks do not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryShape<'a> {
    /// What the entry records.
    pub action: Action,
    /// The `entity_type`, if the entry concerns a record or a table.
    pub entity_type: Option<&'a str>,
    /// Whether the entry names one record (`entity_id`).
    pub has_entity_id: bool,
    /// Whether the entry names the revision the action wrote (`revision_id`).
    pub has_revision_id: bool,
    /// The display label.
    pub label: Option<&'a str>,
    /// The request id that groups the entries of one request.
    pub request_id: Option<&'a str>,
    /// Whether the entry holds the client address (`ip`).
    pub has_client_address: bool,
}

/// Why an entry may not be written. Each is a mistake of the code that
/// builds the entry, so the message says which rule it broke and how to
/// build it instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    /// A client address on an entry that is not a sign-in or account event.
    #[error(
        "the activity entry `{action}` holds a client address, but only sign-in and account \
         entries may (ADR 0011 §5, `ip`); leave the address out"
    )]
    ClientAddressNotAllowed {
        /// The entry's action.
        action: Action,
    },
    /// An entity id without an entity type.
    #[error(
        "the activity entry `{action}` names a record id without its entity type; pass both \
         (ADR 0011 §5)"
    )]
    EntityIdWithoutType {
        /// The entry's action.
        action: Action,
    },
    /// A revision id without the record it belongs to.
    #[error(
        "the activity entry `{action}` names a revision but not the record it belongs to; pass \
         the entity type and id with it (ADR 0011 §4, §5)"
    )]
    RevisionWithoutRecord {
        /// The entry's action.
        action: Action,
    },
    /// An entity type that is not a `snake_case` resource name.
    #[error(
        "the entity type {entity_type:?} is not a resource name: it starts with a lowercase \
         ASCII letter and holds only lowercase ASCII letters, digits and underscores, like the \
         table name `posts` (ADR 0011 §4, `entity_type`)"
    )]
    EntityTypeNotSnakeCase {
        /// The refused entity type.
        entity_type: String,
    },
    /// A label on a sign-in or account entry, which is always about a
    /// person.
    #[error(
        "the activity entry `{action}` holds a label, but sign-in and account entries are \
         about a person and keep only the id (ADR 0011 §5, `label`); leave the label out"
    )]
    LabelOnAccountEvent {
        /// The entry's action.
        action: Action,
    },
    /// A label on an entry that names no record or table.
    #[error(
        "the activity entry `{action}` holds a label but no entity type; a label names a \
         content record (ADR 0011 §5, `label`), so pass the record with it or leave the label \
         out (a media path goes in `details`)"
    )]
    LabelWithoutEntity {
        /// The entry's action.
        action: Action,
    },
    /// A label on an entry about a person.
    #[error(
        "the activity entry about {entity_type} holds a label, but entries about people keep \
         only the id (ADR 0011 §5, `label`); leave the label out"
    )]
    LabelOnPerson {
        /// The entity type, one of [`PEOPLE_ENTITY_TYPES`].
        entity_type: String,
    },
    /// A label that is empty or only whitespace.
    #[error("the activity entry `{action}` has a blank label; pass no label instead (ADR 0011 §5)")]
    BlankLabel {
        /// The entry's action.
        action: Action,
    },
    /// A request id that is empty or longer than [`MAX_REQUEST_ID_CHARS`].
    #[error(
        "the activity entry `{action}` has a request id of {chars} characters; a request id has \
         1 to {MAX_REQUEST_ID_CHARS}, as Loco's request_id middleware makes them, or is left out"
    )]
    RequestIdLength {
        /// The entry's action.
        action: Action,
        /// The length of the refused request id, in characters.
        chars: usize,
    },
}

/// Checks every rule of ADR 0011 §5 for an entry before it is written,
/// including those the table's CHECKs repeat, so that a refusal names the
/// rule and how to keep it instead of a constraint.
///
/// - `shape`: the entry's parts.
///
/// A label is allowed only on an entry about a content record: an
/// account entry is always about a person, an entry without an entity type
/// has no record to name, and the entries about [`PEOPLE_ENTITY_TYPES`] keep
/// only the id. The label is the one free text the log keeps for good, so
/// these rules keep names out of it (ADR 0011 §5, §6).
///
/// # Errors
///
/// Returns the first [`EntryError`] of: a client address on an action that
/// is not an account event; an entity id without an entity type; a revision
/// id without an entity id; an entity type that is not `snake_case`; a label
/// on an account event, on an entry without an entity type, or on an entry
/// about one of [`PEOPLE_ENTITY_TYPES`]; a blank label; a request id that is
/// empty or longer than [`MAX_REQUEST_ID_CHARS`] characters.
pub fn check_entry(shape: &EntryShape<'_>) -> Result<(), EntryError> {
    let action = shape.action;
    if shape.has_client_address && action.group() != ActionGroup::Account {
        return Err(EntryError::ClientAddressNotAllowed { action });
    }
    if shape.has_entity_id && shape.entity_type.is_none() {
        return Err(EntryError::EntityIdWithoutType { action });
    }
    if shape.has_revision_id && !shape.has_entity_id {
        return Err(EntryError::RevisionWithoutRecord { action });
    }
    if let Some(entity_type) = shape.entity_type
        && !is_snake_case(entity_type)
    {
        return Err(EntryError::EntityTypeNotSnakeCase {
            entity_type: entity_type.to_owned(),
        });
    }
    if let Some(label) = shape.label {
        if action.group() == ActionGroup::Account {
            return Err(EntryError::LabelOnAccountEvent { action });
        }
        match shape.entity_type {
            None => return Err(EntryError::LabelWithoutEntity { action }),
            Some(entity_type) if PEOPLE_ENTITY_TYPES.contains(&entity_type) => {
                return Err(EntryError::LabelOnPerson {
                    entity_type: entity_type.to_owned(),
                });
            }
            Some(_) => {}
        }
        if label.trim().is_empty() {
            return Err(EntryError::BlankLabel { action });
        }
    }
    if let Some(request_id) = shape.request_id {
        let chars = request_id.chars().count();
        if chars == 0 || chars > MAX_REQUEST_ID_CHARS {
            return Err(EntryError::RequestIdLength { action, chars });
        }
    }
    Ok(())
}

/// Whether `name` starts with a lowercase ASCII letter and holds only
/// lowercase ASCII letters, digits and underscores.
fn is_snake_case(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|rest| rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '_')
}

/// The names of the fields that an update changed, for the `details` of its
/// `update` entry (ADR 0011 §5: the names, not the values).
///
/// - `before`: the record's revision snapshot before the update, as a JSON
///   object (ADR 0011 §4: the editable fields and relation id lists).
/// - `after`: the snapshot after the update.
///
/// A field counts as changed when its value differs, compared as JSON, or
/// when it is in only one of the two snapshots (a field that a newer
/// snapshot version added or dropped). Nested values are compared whole: a
/// changed element of `label_ids` names `label_ids`. The names are sorted
/// and unique, so the same update always gives the same list.
#[must_use]
pub fn changed_field_names(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<String> {
    let mut names: Vec<String> = before
        .iter()
        .filter(|(name, value)| after.get(*name) != Some(value))
        .map(|(name, _)| name.clone())
        .chain(
            after
                .keys()
                .filter(|name| !before.contains_key(*name))
                .cloned(),
        )
        .collect();
    // serde_json's `Map` keeps insertion order when a crate in the graph
    // turns on its `preserve_order` feature, so the order is set here.
    names.sort_unstable();
    names.dedup();
    names
}

/// The key under which an `update` entry's `details` lists the changed
/// field names.
pub const CHANGED_FIELDS_KEY: &str = "changed_fields";

/// The `details` of an `update` entry: `{"changed_fields": [...]}`.
///
/// - `names`: the changed field names, as [`changed_field_names`] returns
///   them.
#[must_use]
pub fn changed_fields_details(names: &[String]) -> Map<String, Value> {
    let list = names.iter().cloned().map(Value::String).collect();
    [(CHANGED_FIELDS_KEY.to_owned(), Value::Array(list))]
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    /// ADR 0011 §5's action table, group by group, as written there.
    const CONTENT: &[&str] = &[
        "create",
        "update",
        "revert",
        "delete",
        "restore",
        "purge",
        "publish",
        "unpublish",
        "reorder",
        "preview_link_regenerated",
        "import",
    ];
    const ACCOUNT: &[&str] = &[
        "login",
        "login_failed",
        "logout",
        "password_changed",
        "reset_link_issued",
        "temporary_password_issued",
        "totp_enabled",
        "totp_disabled",
        "totp_reset",
        "recovery_code_used",
        "recovery_codes_regenerated",
        "account_unlocked",
        "user_activated",
        "user_deactivated",
        "roles_changed",
        "grants_changed",
    ];
    const MEDIA: &[&str] = &[
        "media_upload",
        "media_rename",
        "media_move",
        "media_delete",
        "media_restore",
        "media_purge",
    ];

    /// An entry shape that passes every check: an update of post 7.
    fn valid() -> EntryShape<'static> {
        EntryShape {
            action: Action::Update,
            entity_type: Some("posts"),
            has_entity_id: true,
            has_revision_id: true,
            label: Some("Szalagavató"),
            request_id: Some("0b1c2d3e-request"),
            has_client_address: false,
        }
    }

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn the_actions_are_adr_0011_s_table_in_order_and_group() {
        let names: Vec<&str> = Action::ALL.iter().map(|action| action.as_str()).collect();
        assert_eq!(names, [CONTENT, ACCOUNT, MEDIA].concat());
        for (group, expected) in [
            (ActionGroup::Content, CONTENT),
            (ActionGroup::Account, ACCOUNT),
            (ActionGroup::Media, MEDIA),
        ] {
            let members: Vec<&str> = Action::ALL
                .iter()
                .filter(|action| action.group() == group)
                .map(|action| action.as_str())
                .collect();
            assert_eq!(members, expected, "{group:?}");
        }
    }

    #[test]
    fn every_action_name_is_snake_case_and_unique() {
        let names: BTreeSet<&str> = Action::ALL.iter().map(|action| action.as_str()).collect();
        assert_eq!(names.len(), Action::ALL.len());
        for name in names {
            assert!(is_snake_case(name), "{name}");
        }
    }

    #[test]
    fn every_action_reads_back_from_its_name_and_displays_as_it() {
        for &action in Action::ALL {
            assert_eq!(action.as_str().parse::<Action>(), Ok(action));
            assert_eq!(action.to_string(), action.as_str());
        }
    }

    #[test]
    fn names_that_are_not_actions_are_refused_with_the_name() {
        for name in ["", "Create", "create ", "archive", "media-upload", "logins"] {
            let error = name.parse::<Action>().expect_err(name);
            assert_eq!(error.name, name);
            assert_eq!(
                error.to_string(),
                format!("{name:?} is not an action of the activity log (ADR 0011 §5)")
            );
        }
    }

    #[test]
    fn each_actor_has_its_stored_kind_and_id() {
        assert_eq!(Actor::User(42).kind(), "user");
        assert_eq!(Actor::User(42).user_id(), Some(42));
        assert_eq!(Actor::System.kind(), "system");
        assert_eq!(Actor::System.user_id(), None);
        assert_eq!(Actor::Import.kind(), "import");
        assert_eq!(Actor::Import.user_id(), None);
    }

    #[test]
    fn stored_actor_columns_that_name_no_actor_are_refused() {
        for (kind, user_id) in [
            ("user", None),
            ("system", Some(1)),
            ("import", Some(1)),
            ("User", Some(1)),
            ("", None),
            ("admin", Some(1)),
        ] {
            let error = Actor::from_stored(kind, user_id).expect_err(kind);
            assert_eq!(
                error,
                StoredActorError {
                    kind: kind.to_owned(),
                    user_id
                }
            );
            assert!(error.to_string().contains("(ADR 0011 §5)"), "{error}");
        }
        assert_eq!(Actor::from_stored("system", None), Ok(Actor::System));
        assert_eq!(Actor::from_stored("import", None), Ok(Actor::Import));
    }

    #[test]
    fn a_complete_valid_entry_passes() {
        assert_eq!(check_entry(&valid()), Ok(()));
        // The smallest entry: an action and nothing else.
        let bare = EntryShape {
            entity_type: None,
            has_entity_id: false,
            has_revision_id: false,
            label: None,
            request_id: None,
            ..valid()
        };
        assert_eq!(check_entry(&bare), Ok(()));
    }

    #[test]
    fn only_account_entries_may_hold_the_client_address() {
        for &action in Action::ALL {
            // No label, which an account entry may not hold either.
            let shape = EntryShape {
                action,
                has_client_address: true,
                label: None,
                ..valid()
            };
            let result = check_entry(&shape);
            if action.group() == ActionGroup::Account {
                assert_eq!(result, Ok(()), "{action}");
            } else {
                assert_eq!(
                    result,
                    Err(EntryError::ClientAddressNotAllowed { action }),
                    "{action}"
                );
            }
        }
        let message = EntryError::ClientAddressNotAllowed {
            action: Action::MediaUpload,
        }
        .to_string();
        assert_eq!(
            message,
            "the activity entry `media_upload` holds a client address, but only sign-in and \
             account entries may (ADR 0011 §5, `ip`); leave the address out"
        );
    }

    #[test]
    fn a_record_id_needs_its_type_and_a_revision_needs_its_record() {
        let without_type = EntryShape {
            entity_type: None,
            label: None,
            ..valid()
        };
        assert_eq!(
            check_entry(&without_type),
            Err(EntryError::EntityIdWithoutType {
                action: Action::Update
            })
        );
        let without_record = EntryShape {
            has_entity_id: false,
            ..valid()
        };
        assert_eq!(
            check_entry(&without_record),
            Err(EntryError::RevisionWithoutRecord {
                action: Action::Update
            })
        );
        // A table without a record id, as the importer's per-table entry.
        let table = EntryShape {
            action: Action::Import,
            has_entity_id: false,
            has_revision_id: false,
            label: None,
            ..valid()
        };
        assert_eq!(check_entry(&table), Ok(()));
    }

    #[test]
    fn entity_types_must_be_resource_names() {
        for entity_type in ["posts", "canteen_days", "menu_items", "v1", "a"] {
            let shape = EntryShape {
                entity_type: Some(entity_type),
                ..valid()
            };
            assert_eq!(check_entry(&shape), Ok(()), "{entity_type}");
        }
        for entity_type in [
            "",
            "Posts",
            "menu-items",
            "_posts",
            "1posts",
            "posts ",
            "címkék",
        ] {
            let shape = EntryShape {
                entity_type: Some(entity_type),
                ..valid()
            };
            let error = check_entry(&shape).expect_err(entity_type);
            assert_eq!(
                error,
                EntryError::EntityTypeNotSnakeCase {
                    entity_type: entity_type.to_owned()
                }
            );
            assert!(
                error
                    .to_string()
                    .starts_with(&format!("the entity type {entity_type:?} is not")),
                "{error}"
            );
        }
    }

    #[test]
    fn entries_about_people_keep_no_label() {
        for entity_type in PEOPLE_ENTITY_TYPES {
            let labelled = EntryShape {
                entity_type: Some(entity_type),
                ..valid()
            };
            assert_eq!(
                check_entry(&labelled),
                Err(EntryError::LabelOnPerson {
                    entity_type: entity_type.to_owned()
                })
            );
            let unlabelled = EntryShape {
                label: None,
                ..labelled
            };
            assert_eq!(check_entry(&unlabelled), Ok(()), "{entity_type}");
        }
        assert_eq!(
            EntryError::LabelOnPerson {
                entity_type: "authors".to_owned()
            }
            .to_string(),
            "the activity entry about authors holds a label, but entries about people keep only \
             the id (ADR 0011 §5, `label`); leave the label out"
        );
    }

    #[test]
    fn account_entries_keep_no_label_whatever_they_name() {
        // About the account itself, about another record, and about nothing:
        // an account entry is about a person in every case.
        for &action in Action::ALL {
            if action.group() != ActionGroup::Account {
                continue;
            }
            for (entity_type, has_entity_id) in
                [(Some("users"), true), (Some("posts"), true), (None, false)]
            {
                let shape = EntryShape {
                    action,
                    entity_type,
                    has_entity_id,
                    has_revision_id: false,
                    ..valid()
                };
                assert_eq!(
                    check_entry(&shape),
                    Err(EntryError::LabelOnAccountEvent { action }),
                    "{action} {entity_type:?}"
                );
                let unlabelled = EntryShape {
                    label: None,
                    ..shape
                };
                assert_eq!(check_entry(&unlabelled), Ok(()), "{action} {entity_type:?}");
            }
        }
        assert_eq!(
            EntryError::LabelOnAccountEvent {
                action: Action::Login
            }
            .to_string(),
            "the activity entry `login` holds a label, but sign-in and account entries are about \
             a person and keep only the id (ADR 0011 §5, `label`); leave the label out"
        );
    }

    #[test]
    fn a_label_needs_the_record_it_names() {
        for &action in Action::ALL {
            if action.group() == ActionGroup::Account {
                continue;
            }
            let without_entity = EntryShape {
                action,
                entity_type: None,
                has_entity_id: false,
                has_revision_id: false,
                ..valid()
            };
            assert_eq!(
                check_entry(&without_entity),
                Err(EntryError::LabelWithoutEntity { action }),
                "{action}"
            );
            // A whole table is enough: it names the entity type.
            let table = EntryShape {
                entity_type: Some("labels"),
                ..without_entity
            };
            assert_eq!(check_entry(&table), Ok(()), "{action}");
        }
        assert_eq!(
            EntryError::LabelWithoutEntity {
                action: Action::MediaDelete
            }
            .to_string(),
            "the activity entry `media_delete` holds a label but no entity type; a label names a \
             content record (ADR 0011 §5, `label`), so pass the record with it or leave the label \
             out (a media path goes in `details`)"
        );
    }

    #[test]
    fn blank_labels_are_refused() {
        for label in ["", " ", "\t\n", "\u{3000}"] {
            let shape = EntryShape {
                label: Some(label),
                ..valid()
            };
            assert_eq!(
                check_entry(&shape),
                Err(EntryError::BlankLabel {
                    action: Action::Update
                }),
                "{label:?}"
            );
        }
    }

    #[test]
    fn request_ids_have_one_to_255_characters() {
        let longest = "é".repeat(MAX_REQUEST_ID_CHARS);
        for request_id in ["x", longest.as_str()] {
            let shape = EntryShape {
                request_id: Some(request_id),
                ..valid()
            };
            assert_eq!(check_entry(&shape), Ok(()), "{}", request_id.len());
        }
        let too_long = "x".repeat(MAX_REQUEST_ID_CHARS + 1);
        for (request_id, chars) in [("", 0), (too_long.as_str(), 256)] {
            let shape = EntryShape {
                request_id: Some(request_id),
                ..valid()
            };
            assert_eq!(
                check_entry(&shape),
                Err(EntryError::RequestIdLength {
                    action: Action::Update,
                    chars
                })
            );
        }
        assert_eq!(
            EntryError::RequestIdLength {
                action: Action::Login,
                chars: 0
            }
            .to_string(),
            "the activity entry `login` has a request id of 0 characters; a request id has 1 to \
             255, as Loco's request_id middleware makes them, or is left out"
        );
    }

    #[test]
    fn changed_field_names_lists_changed_added_and_removed_fields_sorted() {
        let before = object(json!({
            "name": "Hírek",
            "color": "#AABBCC",
            "label_ids": [1, 2],
            "note": null,
            "dropped": 1,
        }));
        let after = object(json!({
            "name": "Hírek",
            "color": "#AABBCD",
            "label_ids": [2, 1],
            "note": null,
            "added": false,
        }));
        assert_eq!(
            changed_field_names(&before, &after),
            ["added", "color", "dropped", "label_ids"]
        );
        assert_eq!(changed_field_names(&before, &before), Vec::<String>::new());
        assert_eq!(
            changed_field_names(&Map::new(), &Map::new()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_null_and_a_missing_field_differ() {
        let before = object(json!({ "description": null }));
        assert_eq!(changed_field_names(&before, &Map::new()), ["description"]);
        assert_eq!(changed_field_names(&Map::new(), &before), ["description"]);
    }

    #[test]
    fn the_details_of_an_update_name_the_fields_under_one_key() {
        let names = vec!["color".to_owned(), "name".to_owned()];
        assert_eq!(
            Value::Object(changed_fields_details(&names)),
            json!({ "changed_fields": ["color", "name"] })
        );
        assert_eq!(
            Value::Object(changed_fields_details(&[])),
            json!({ "changed_fields": [] })
        );
    }

    /// JSON values up to two levels deep, with few distinct leaves, so that
    /// two generated values are often equal.
    fn json_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            (0_i64..4).prop_map(Value::from),
            "[ab]{0,2}".prop_map(Value::String),
        ];
        leaf.prop_recursive(2, 8, 3, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
                prop::collection::btree_map("[a-c]", inner, 0..3)
                    .prop_map(|map| Value::Object(map.into_iter().collect())),
            ]
        })
    }

    /// Snapshots over a small key alphabet, so that two of them share keys.
    fn snapshot() -> impl Strategy<Value = Map<String, Value>> {
        prop::collection::btree_map("[a-e]{1,2}", json_value(), 0..6)
            .prop_map(|map| map.into_iter().collect())
    }

    fn action() -> impl Strategy<Value = Action> {
        prop::sample::select(Action::ALL)
    }

    proptest! {
        #[test]
        fn a_field_is_named_exactly_when_it_differs(before in snapshot(), after in snapshot()) {
            let names = changed_field_names(&before, &after);
            let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
            let expected: Vec<String> = keys
                .into_iter()
                .filter(|key| before.get(*key) != after.get(*key))
                .cloned()
                .collect();
            prop_assert_eq!(names, expected);
        }

        #[test]
        fn the_names_do_not_depend_on_the_direction(before in snapshot(), after in snapshot()) {
            prop_assert_eq!(
                changed_field_names(&before, &after),
                changed_field_names(&after, &before)
            );
        }

        #[test]
        fn an_unchanged_snapshot_names_nothing(snapshot in snapshot()) {
            prop_assert!(changed_field_names(&snapshot, &snapshot.clone()).is_empty());
        }

        #[test]
        fn changing_one_field_names_only_that_field(
            snapshot in snapshot(),
            name in "[a-e]{1,2}",
            value in json_value(),
        ) {
            let mut after = snapshot.clone();
            let changed = after.insert(name.clone(), value.clone()) != Some(value);
            let expected = if changed { vec![name] } else { Vec::new() };
            prop_assert_eq!(changed_field_names(&snapshot, &after), expected);
        }

        #[test]
        fn the_details_list_the_names_in_order(names in prop::collection::vec("[a-z_]{1,8}", 0..6)) {
            let details = changed_fields_details(&names);
            prop_assert_eq!(details.len(), 1);
            prop_assert_eq!(&details[CHANGED_FIELDS_KEY], &json!(names));
        }

        #[test]
        fn every_name_reads_back_and_every_other_string_is_refused(name in "\\PC{0,30}") {
            match name.parse::<Action>() {
                Ok(action) => prop_assert_eq!(action.as_str(), name.as_str()),
                Err(error) => {
                    prop_assert!(!Action::ALL.iter().any(|action| action.as_str() == name));
                    prop_assert_eq!(error.name, name);
                }
            }
        }

        #[test]
        fn every_actor_reads_back_from_its_columns(id in any::<i64>(), pick in 0_usize..3) {
            let actor = [Actor::User(id), Actor::System, Actor::Import][pick];
            prop_assert_eq!(Actor::from_stored(actor.kind(), actor.user_id()), Ok(actor));
        }

        #[test]
        fn a_kind_that_is_not_stored_is_refused(kind in "\\PC{0,12}", id in any::<Option<i64>>()) {
            prop_assume!(!ACTOR_KINDS.contains(&kind.as_str()));
            prop_assert!(Actor::from_stored(&kind, id).is_err());
        }

        #[test]
        fn snake_case_entity_types_pass_and_others_fail(entity_type in "\\PC{0,20}", action in action()) {
            let shape = EntryShape {
                action,
                entity_type: Some(&entity_type),
                has_entity_id: true,
                has_revision_id: false,
                label: None,
                request_id: None,
                has_client_address: false,
            };
            let snake = entity_type.chars().next().is_some_and(|first| first.is_ascii_lowercase())
                && entity_type
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            prop_assert_eq!(check_entry(&shape).is_ok(), snake);
        }

        #[test]
        fn a_request_id_passes_exactly_when_its_length_is_in_range(
            request_id in "\\PC{0,300}",
            action in action(),
        ) {
            let shape = EntryShape {
                action,
                entity_type: None,
                has_entity_id: false,
                has_revision_id: false,
                label: None,
                request_id: Some(&request_id),
                has_client_address: false,
            };
            let chars = request_id.chars().count();
            prop_assert_eq!(
                check_entry(&shape).is_ok(),
                (1..=MAX_REQUEST_ID_CHARS).contains(&chars)
            );
        }

        #[test]
        fn a_label_passes_exactly_when_it_names_a_content_record_and_is_not_blank(
            label in "\\PC{0,20}",
            entity_type in prop::option::of(prop::sample::select(
                vec!["posts", "labels", "users", "authors", "colleagues"],
            )),
            has_entity_id in any::<bool>(),
            action in action(),
        ) {
            let shape = EntryShape {
                action,
                entity_type,
                has_entity_id: has_entity_id && entity_type.is_some(),
                has_revision_id: false,
                label: Some(&label),
                request_id: None,
                has_client_address: false,
            };
            let expected = action.group() != ActionGroup::Account
                && entity_type.is_some_and(|entity_type| !PEOPLE_ENTITY_TYPES.contains(&entity_type))
                && !label.trim().is_empty();
            prop_assert_eq!(check_entry(&shape).is_ok(), expected);
            // Without the label, the same entry always passes.
            prop_assert_eq!(check_entry(&EntryShape { label: None, ..shape }), Ok(()));
        }
    }
}
