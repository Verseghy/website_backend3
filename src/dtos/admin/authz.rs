//! Wire types of roles, permissions and access actions (ADR 0007 §1, §2,
//! §7).
//!
//! The admin UI receives an account's roles, effective permissions and
//! access map in `/auth/me` ([`super::auth::MeResponse`]), and later the
//! roles and permissions lists and the user editor. The values are the
//! identifiers of `crate::domain::authz`, in English exactly as stored
//! (`edit posts`, `writer`) or as ra-core names them (`list`), because
//! identifiers stay in English and the admin shows Hungarian labels for them
//! (ADR 0021 §6).
//!
//! Domain types never derive ts-rs (ADR 0008 rule 6), so these enums mirror
//! the domain's and convert from them with an exhaustive `match`: a new
//! domain variant does not compile until it has its wire variant, and the
//! tests below hold every wire spelling equal to the domain's identifier.
//! The admin's runtime check of these values (`admin/src/auth/authz.ts`)
//! is typed against the generated unions in the same way.

use serde::Serialize;
use ts_rs::TS;

use crate::domain::authz;

/// A permission identifier (ADR 0007 §1), as `user_permissions` stores it.
// The doc comments here are exported into the TypeScript binding. Each
// variant names its spelling: `rename_all` cannot write the spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub enum Permission {
    /// Posts.
    #[serde(rename = "edit posts")]
    EditPosts,
    /// Publishing posts and writing published ones, with `edit posts`.
    #[serde(rename = "publish posts")]
    PublishPosts,
    /// Labels.
    #[serde(rename = "edit labels")]
    EditLabels,
    /// Authors.
    #[serde(rename = "edit authors")]
    EditAuthors,
    /// Events.
    #[serde(rename = "edit events")]
    EditEvents,
    /// Dishes and canteen days.
    #[serde(rename = "edit canteens")]
    EditCanteens,
    /// Colleagues.
    #[serde(rename = "edit colleagues")]
    EditColleagues,
    /// Users, and the roles and permissions lists.
    #[serde(rename = "edit users")]
    EditUsers,
    /// Pages.
    #[serde(rename = "edit pages")]
    EditPages,
    /// Menu items.
    #[serde(rename = "edit menu")]
    EditMenu,
    /// The media library.
    #[serde(rename = "manage media")]
    ManageMedia,
    /// The global activity log and the global trash.
    #[serde(rename = "view logs")]
    ViewLogs,
    /// Home-page notices.
    #[serde(rename = "edit notices")]
    EditNotices,
    /// Partner banners.
    #[serde(rename = "edit banners")]
    EditBanners,
    /// The site settings.
    #[serde(rename = "edit site settings")]
    EditSiteSettings,
    /// Diáktoll issues.
    #[serde(rename = "edit diaktoll")]
    EditDiaktoll,
}

impl From<authz::Permission> for Permission {
    /// The wire spelling of a domain permission.
    ///
    /// - `permission`: the domain value.
    fn from(permission: authz::Permission) -> Self {
        match permission {
            authz::Permission::EditPosts => Self::EditPosts,
            authz::Permission::PublishPosts => Self::PublishPosts,
            authz::Permission::EditLabels => Self::EditLabels,
            authz::Permission::EditAuthors => Self::EditAuthors,
            authz::Permission::EditEvents => Self::EditEvents,
            authz::Permission::EditCanteens => Self::EditCanteens,
            authz::Permission::EditColleagues => Self::EditColleagues,
            authz::Permission::EditUsers => Self::EditUsers,
            authz::Permission::EditPages => Self::EditPages,
            authz::Permission::EditMenu => Self::EditMenu,
            authz::Permission::ManageMedia => Self::ManageMedia,
            authz::Permission::ViewLogs => Self::ViewLogs,
            authz::Permission::EditNotices => Self::EditNotices,
            authz::Permission::EditBanners => Self::EditBanners,
            authz::Permission::EditSiteSettings => Self::EditSiteSettings,
            authz::Permission::EditDiaktoll => Self::EditDiaktoll,
        }
    }
}

/// A role (ADR 0007 §2), by its name in `roles`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../admin/src/bindings/")]
pub enum Role {
    /// Writes posts, with labels, authors and media.
    Writer,
    /// Events, the canteen and colleagues.
    Secretary,
    /// A writer who also publishes.
    Supervisor,
    /// Every permission, and what only the admin role unlocks.
    Admin,
}

impl From<authz::Role> for Role {
    /// The wire spelling of a domain role.
    ///
    /// - `role`: the domain value.
    fn from(role: authz::Role) -> Self {
        match role {
            authz::Role::Writer => Self::Writer,
            authz::Role::Secretary => Self::Secretary,
            authz::Role::Supervisor => Self::Supervisor,
            authz::Role::Admin => Self::Admin,
        }
    }
}

/// An action the admin UI checks on an ra-core resource (ADR 0007 §7):
/// ra-core's `list`, `show`, `create`, `edit` and `delete`, and backend3's
/// `publish`, `restore`, `purge` and `history`.
// Exported doc comments here and on the variants. The name keeps the
// binding apart from the activity log's actions (ADR 0011 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../admin/src/bindings/")]
pub enum AccessAction {
    /// The list screen and the sidebar entry.
    List,
    /// One record, read only.
    Show,
    /// A new record.
    Create,
    /// An update of a record.
    Edit,
    /// The soft delete into the trash.
    Delete,
    /// Publishing and unpublishing.
    Publish,
    /// Restoring from the trash.
    Restore,
    /// Permanent delete from the trash (the admin role only).
    Purge,
    /// One record's history.
    History,
}

impl From<authz::AccessAction> for AccessAction {
    /// The wire spelling of a domain action.
    ///
    /// - `action`: the domain value.
    fn from(action: authz::AccessAction) -> Self {
        match action {
            authz::AccessAction::List => Self::List,
            authz::AccessAction::Show => Self::Show,
            authz::AccessAction::Create => Self::Create,
            authz::AccessAction::Edit => Self::Edit,
            authz::AccessAction::Delete => Self::Delete,
            authz::AccessAction::Publish => Self::Publish,
            authz::AccessAction::Restore => Self::Restore,
            authz::AccessAction::Purge => Self::Purge,
            authz::AccessAction::History => Self::History,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn every_permission_is_sent_as_its_stored_identifier() {
        for permission in authz::Permission::ALL {
            assert_eq!(
                serde_json::to_value(Permission::from(permission)).expect("serializes"),
                json!(permission.name()),
                "{permission}"
            );
        }
    }

    #[test]
    fn every_role_is_sent_as_its_stored_name() {
        for role in authz::Role::ALL {
            assert_eq!(
                serde_json::to_value(Role::from(role)).expect("serializes"),
                json!(role.name()),
                "{role}"
            );
        }
    }

    #[test]
    fn every_access_action_is_sent_as_its_domain_name() {
        for action in authz::AccessAction::ALL {
            assert_eq!(
                serde_json::to_value(AccessAction::from(action)).expect("serializes"),
                json!(action.name()),
                "{action}"
            );
        }
        // The wire enum keeps the domain's order, which the access map's
        // lists follow.
        let wire: Vec<AccessAction> = authz::AccessAction::ALL
            .into_iter()
            .map(AccessAction::from)
            .collect();
        assert!(wire.is_sorted());
    }

    #[test]
    fn typescript_declarations() {
        let config = ts_rs::Config::default();
        let actions: Vec<String> = authz::AccessAction::ALL
            .iter()
            .map(|action| format!("\"{}\"", action.name()))
            .collect();
        assert_eq!(
            AccessAction::decl(&config),
            format!("type AccessAction = {};", actions.join(" | "))
        );
        let names: Vec<String> = authz::Permission::ALL
            .iter()
            .map(|permission| format!("\"{}\"", permission.name()))
            .collect();
        assert_eq!(
            Permission::decl(&config),
            format!("type Permission = {};", names.join(" | "))
        );
        assert_eq!(
            Role::decl(&config),
            r#"type Role = "writer" | "secretary" | "supervisor" | "admin";"#
        );
    }
}
