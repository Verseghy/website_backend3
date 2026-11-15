//! Wire types of roles and permissions (ADR 0007 §1, §2, §7).
//!
//! The admin UI receives an account's roles and effective permissions in
//! `/auth/me` ([`super::auth::MeResponse`]), and later the roles and
//! permissions lists and the user editor. The values are the identifiers of
//! `crate::domain::authz`, in English exactly as stored (`edit posts`,
//! `writer`), because identifiers stay in English and the admin shows
//! Hungarian labels for them (ADR 0021 §6).
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
    fn typescript_declarations() {
        let config = ts_rs::Config::default();
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
