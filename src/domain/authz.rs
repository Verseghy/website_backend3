//! Roles, permissions and an account's effective permissions (ADR 0007
//! §1-§3).
//!
//! Why this module exists: ADR 0007 §3 keeps the role-to-permission mapping
//! out of the database. The mapping is the constant [`ROLE_PERMISSIONS`]
//! here, so every change to what a role may do is a reviewed code change. The
//! database holds only what an account was given: its roles (`user_roles`,
//! referencing the four `roles` rows that a migration seeds) and its extra
//! grants („Egyéni jogosultságok”, `user_permissions`, whose CHECK lists the
//! sixteen names of [`Permission`]). The admin route table's guard reads
//! both on every request (`open_session` in `controllers::admin::session`),
//! before the handler runs, and turns them into [`AccountGrants`] with
//! [`AccountGrants::from_stored`]; the permission checks and `/auth/me` work
//! from that value.
//!
//! The conventions this module embodies:
//!
//! - **The names are backend1's.** Permission identifiers stay in English
//!   exactly as backend1 wrote them, spaces included (`edit posts`), so the
//!   importer maps the legacy grants one to one and old notes stay readable
//!   (ADR 0007 §1, ADR 0021 §6). The admin UI shows Hungarian labels.
//! - **The role ids are backend1's.** 1 `writer`, 2 `secretary`,
//!   3 `supervisor`, 4 `admin` (ADR 0007 §3, ADR 0009).
//! - **Sixteen permissions, no more.** `edit newsletter` and `manage backups`
//!   are not created: the newsletter and the Backups screen are retired, and
//!   the maintainer confirmed both points, and `view logs` gating the global
//!   activity log and the global trash, on 2026-10-25 (ADR 0007, follow-up
//!   points 1-3; research: DECISIONS R9 (D06)).
//! - **The code and the schema agree.** Adding a role or a permission is a
//!   migration (a `roles` row, or a new CHECK on `user_permissions`) and a
//!   change here, in the same PR. `tests/migration/authz.rs` fails when the
//!   seeded rows or the CHECK differ from [`Role::ALL`] and
//!   [`Permission::ALL`].
//! - **Declaration order is the documented order.** The variants are declared
//!   in the order of ADR 0007's tables and derive `Ord`, so a
//!   [`BTreeSet`] of them, and every list built from one (the `/auth/me`
//!   answer), comes out in that order.
//!
//! [`Requirement`] is what an admin API route asks of the account that calls
//! it (ADR 0007 §5, "Deny by default"): the admin route table
//! (`controllers::admin::routes`) declares one for every route, and its
//! guard answers `403 forbidden` when [`Requirement::is_met_by`] says no.
//!
//! The record-level rules of ADR 0007 §5 (published posts, the user guards
//! of §6) and the per-resource access map of §7 join this module with the
//! slices that need them.

use std::{collections::BTreeSet, fmt};

/// A permission of ADR 0007 §1.
///
/// The variants are declared in the order of the ADR's table: backend1's
/// eight kept names first, then the four new ones for areas backend1 already
/// had, then the four release-1 permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    /// `edit posts`: posts, including preview-token regeneration.
    EditPosts,
    /// `publish posts`: together with `edit posts`, publishing,
    /// unpublishing, „featured”, and any write to a published post.
    PublishPosts,
    /// `edit labels`: labels.
    EditLabels,
    /// `edit authors`: authors.
    EditAuthors,
    /// `edit events`: events.
    EditEvents,
    /// `edit canteens`: dishes and canteen days, the week editor included.
    EditCanteens,
    /// `edit colleagues`: colleagues.
    EditColleagues,
    /// `edit users`: users, the read-only roles and permissions lists, and
    /// the account actions of ADR 0006.
    EditUsers,
    /// `edit pages`: pages, including Clone.
    EditPages,
    /// `edit menu`: menu items, including reordering the tree.
    EditMenu,
    /// `manage media`: the media library.
    ManageMedia,
    /// `view logs`: the global activity log and the global trash.
    ViewLogs,
    /// `edit notices`: home-page notices (release 1).
    EditNotices,
    /// `edit banners`: partner banners (release 1).
    EditBanners,
    /// `edit site settings`: the site settings record (release 1).
    EditSiteSettings,
    /// `edit diaktoll`: Diáktoll issues (release 1).
    EditDiaktoll,
}

impl Permission {
    /// Every permission, in declaration order: the sixteen names that the
    /// `user_permissions` CHECK lists.
    pub const ALL: [Self; 16] = [
        Self::EditPosts,
        Self::PublishPosts,
        Self::EditLabels,
        Self::EditAuthors,
        Self::EditEvents,
        Self::EditCanteens,
        Self::EditColleagues,
        Self::EditUsers,
        Self::EditPages,
        Self::EditMenu,
        Self::ManageMedia,
        Self::ViewLogs,
        Self::EditNotices,
        Self::EditBanners,
        Self::EditSiteSettings,
        Self::EditDiaktoll,
    ];

    /// The permission's identifier: the value stored in
    /// `user_permissions.permission` and sent to the admin UI.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EditPosts => "edit posts",
            Self::PublishPosts => "publish posts",
            Self::EditLabels => "edit labels",
            Self::EditAuthors => "edit authors",
            Self::EditEvents => "edit events",
            Self::EditCanteens => "edit canteens",
            Self::EditColleagues => "edit colleagues",
            Self::EditUsers => "edit users",
            Self::EditPages => "edit pages",
            Self::EditMenu => "edit menu",
            Self::ManageMedia => "manage media",
            Self::ViewLogs => "view logs",
            Self::EditNotices => "edit notices",
            Self::EditBanners => "edit banners",
            Self::EditSiteSettings => "edit site settings",
            Self::EditDiaktoll => "edit diaktoll",
        }
    }

    /// The permission whose identifier is exactly `name`, or `None`.
    ///
    /// - `name`: a stored or submitted permission identifier.
    ///
    /// The comparison is exact, as the database's CHECK is: `Edit posts`,
    /// `edit_posts` and `edit posts ` name no permission, and neither do the
    /// two names backend3 never creates, `edit newsletter` and
    /// `manage backups`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|permission| permission.name() == name)
    }
}

impl fmt::Display for Permission {
    /// Writes the identifier, for log lines and error messages.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A role of ADR 0007 §2, with backend1's id (ADR 0007 §3).
///
/// The variants are declared in id order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// `writer` (id 1): writes posts, with labels, authors and media.
    Writer,
    /// `secretary` (id 2): events, the canteen and colleagues.
    Secretary,
    /// `supervisor` (id 3): a writer who also publishes.
    Supervisor,
    /// `admin` (id 4): every permission, and the three things of ADR 0007 §2
    /// that are no permission (permanent delete, actions on other admin-role
    /// accounts, figures about individual people).
    Admin,
}

impl Role {
    /// Every role, in id order: the rows that the migration seeds into
    /// `roles`.
    pub const ALL: [Self; 4] = [Self::Writer, Self::Secretary, Self::Supervisor, Self::Admin];

    /// The role's id, `roles.id`, kept from backend1 (ADR 0007 §3).
    #[must_use]
    pub const fn id(self) -> i64 {
        match self {
            Self::Writer => 1,
            Self::Secretary => 2,
            Self::Supervisor => 3,
            Self::Admin => 4,
        }
    }

    /// The role's name, `roles.name`, kept from backend1.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Writer => "writer",
            Self::Secretary => "secretary",
            Self::Supervisor => "supervisor",
            Self::Admin => "admin",
        }
    }

    /// The role whose id is `id`, or `None`.
    ///
    /// - `id`: a `roles.id` or `user_roles.role_id` value.
    #[must_use]
    pub fn from_id(id: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.id() == id)
    }

    /// The role whose name is exactly `name`, or `None`.
    ///
    /// - `name`: a role name, such as the legacy importer reads from
    ///   backend1 (ADR 0007 §9). The comparison is exact: `Admin` names no
    ///   role.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    /// The permissions this role grants: its row of [`ROLE_PERMISSIONS`].
    #[must_use]
    pub const fn permissions(self) -> &'static [Permission] {
        ROLE_PERMISSIONS[self.index()].1
    }

    /// The role's position in [`Role::ALL`] and in [`ROLE_PERMISSIONS`],
    /// which list the roles in the same order (a test pins both).
    const fn index(self) -> usize {
        match self {
            Self::Writer => 0,
            Self::Secretary => 1,
            Self::Supervisor => 2,
            Self::Admin => 3,
        }
    }
}

impl fmt::Display for Role {
    /// Writes the role's name, for log lines and error messages.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The role matrix of ADR 0007 §2: what each role grants, one row per role
/// in the order of [`Role::ALL`], each row in the order of
/// [`Permission::ALL`].
///
/// This constant is the whole mapping; no table holds it (ADR 0007 §3).
/// Changing a row changes ADR 0007 §2 and needs the maintainer's answer
/// (ADR 0007 §8): `edit pages`, `edit menu`, `view logs` and the four
/// release-1 permissions go to `admin` only, and other accounts get them as
/// extra grants.
pub const ROLE_PERMISSIONS: [(Role, &[Permission]); 4] = [
    (
        Role::Writer,
        &[
            Permission::EditPosts,
            Permission::EditLabels,
            Permission::EditAuthors,
            Permission::ManageMedia,
        ],
    ),
    (
        Role::Secretary,
        &[
            Permission::EditEvents,
            Permission::EditCanteens,
            Permission::EditColleagues,
        ],
    ),
    (
        Role::Supervisor,
        &[
            Permission::EditPosts,
            Permission::PublishPosts,
            Permission::EditLabels,
            Permission::EditAuthors,
            Permission::ManageMedia,
        ],
    ),
    // ADR 0007 §2 gives `admin` every permission, the release-1 ones
    // included (§8).
    (Role::Admin, &Permission::ALL),
];

/// An account's effective permissions: the union of the permission sets of
/// all its roles and its extra grants (ADR 0007 §3).
///
/// - `roles`: the roles the account holds; repeats change nothing.
/// - `extra`: the account's extra grants; repeats change nothing.
///
/// An account with no role and no extra grant holds no permission: it can
/// sign in and use only its own account (ADR 0006).
#[must_use]
pub fn effective_permissions(
    roles: impl IntoIterator<Item = Role>,
    extra: impl IntoIterator<Item = Permission>,
) -> BTreeSet<Permission> {
    roles
        .into_iter()
        .flat_map(|role| role.permissions().iter().copied())
        .chain(extra)
        .collect()
}

/// Whether an account with `roles` holds the `admin` role.
///
/// - `roles`: the roles the account holds.
///
/// Only the role counts, never the permissions: an account that holds all
/// sixteen permissions as extra grants is not an admin, because the three
/// things that only the `admin` role unlocks are not permissions, so no
/// extra grant can give them (ADR 0007 §2).
#[must_use]
pub fn is_admin(roles: impl IntoIterator<Item = Role>) -> bool {
    roles.into_iter().any(|role| role == Role::Admin)
}

/// What an account was given: its roles and its extra grants (ADR 0007 §3),
/// as the admin route table's guard reads them for a request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountGrants {
    /// The roles the account holds (`user_roles`).
    pub roles: BTreeSet<Role>,
    /// The account's extra grants, „Egyéni jogosultságok”
    /// (`user_permissions`).
    pub extra: BTreeSet<Permission>,
}

/// Why the stored roles or grants of an account do not match this module.
///
/// Either one is a defect, never a client error: the `user_roles` foreign
/// key admits only ids of the seeded `roles` rows, and the
/// `user_permissions` CHECK only the sixteen names, and
/// `tests/migration/authz.rs` holds both equal to the enums. A change to the
/// database made outside the migrations, or a migration without its code
/// change, is the only way to get here.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoredGrantError {
    /// `user_roles` names a role id that [`Role`] does not define.
    #[error(
        "user_roles names the role id {0}, which no Role in src/domain/authz.rs has; the roles \
         table and the Role enum must list the same roles, changed together by a migration and \
         a code change (ADR 0007 §3)"
    )]
    UnknownRoleId(i64),
    /// `user_permissions` holds a name that [`Permission`] does not define.
    #[error(
        "user_permissions holds the permission {0:?}, which no Permission in \
         src/domain/authz.rs has; the CHECK user_permissions_permission_known and the \
         Permission enum must list the same names, changed together by a migration and a code \
         change (ADR 0007 §3)"
    )]
    UnknownPermission(String),
}

impl AccountGrants {
    /// Reads an account's stored roles and extra grants.
    ///
    /// - `role_ids`: the account's `user_roles.role_id` values.
    /// - `permissions`: the account's `user_permissions.permission` values.
    ///
    /// Repeats collapse; the order of the inputs does not matter.
    ///
    /// # Errors
    ///
    /// [`StoredGrantError`] for the first value, roles first, that names no
    /// [`Role`] or [`Permission`]. The caller answers `500`: guessing would
    /// either drop a permission the account was given or keep one the code
    /// does not know.
    pub fn from_stored<'a>(
        role_ids: impl IntoIterator<Item = i64>,
        permissions: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, StoredGrantError> {
        let roles = role_ids
            .into_iter()
            .map(|id| Role::from_id(id).ok_or(StoredGrantError::UnknownRoleId(id)))
            .collect::<Result<_, _>>()?;
        let extra = permissions
            .into_iter()
            .map(|name| {
                Permission::from_name(name)
                    .ok_or_else(|| StoredGrantError::UnknownPermission(name.to_owned()))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { roles, extra })
    }

    /// The account's effective permissions ([`effective_permissions`]), in
    /// the order of [`Permission::ALL`].
    #[must_use]
    pub fn effective(&self) -> BTreeSet<Permission> {
        effective_permissions(self.roles.iter().copied(), self.extra.iter().copied())
    }

    /// Whether the account holds the `admin` role ([`is_admin`]).
    #[must_use]
    pub fn is_admin(&self) -> bool {
        is_admin(self.roles.iter().copied())
    }
}

/// What an admin API route asks of the account that calls it (ADR 0007 §4,
/// §5).
///
/// Every `/api/admin` route is declared with one in the admin route table;
/// there is no default, so a route cannot be added without saying who may
/// call it. Only [`Requirement::Public`] lets a request in without a
/// session, and the route table's test lists the routes allowed to use it.
/// The other three need a valid session first: a request without one is
/// answered `401` before any requirement is looked at (ADR 0007 §5, the
/// flowchart), so `403` always means "signed in, but not allowed".
///
/// The ADR's "any of" rows (the labels and authors pickers that `edit posts`
/// may read, the page picker of `edit menu`) are not variants yet: the first
/// slice that mounts such a route adds one, with its tests here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Requirement {
    /// No session at all: the sign-in steps that issue one (ADR 0006 §3,
    /// §4), such as `POST /auth/login`.
    Public,
    /// Any account with a valid session, whatever it holds: the account's
    /// own screens („Fiókom”, ADR 0007 §4). The session check already
    /// refuses an inactive, trashed or anonymised account (ADR 0006 §1).
    AnyActiveUser,
    /// The `admin` role itself, which no extra grant gives: permanent
    /// delete and figures about individual people (ADR 0007 §2).
    AdminRole,
    /// One permission among the account's effective permissions
    /// (ADR 0007 §3, §4).
    Permission(Permission),
}

impl Requirement {
    /// Whether a request must carry a valid session before this requirement
    /// is checked: every requirement but [`Requirement::Public`].
    #[must_use]
    pub const fn needs_session(self) -> bool {
        !matches!(self, Self::Public)
    }

    /// Whether an account with `grants` may call a route that asks for this
    /// requirement.
    ///
    /// - `grants`: the roles and extra grants that the session's account
    ///   holds, as the admin route table's guard read them for the request
    ///   (`open_session` in `controllers::admin::session`).
    ///
    /// [`Requirement::Public`] and [`Requirement::AnyActiveUser`] hold for
    /// every account; [`Requirement::AdminRole`] reads the role only, never
    /// the permissions ([`AccountGrants::is_admin`]); and
    /// [`Requirement::Permission`] reads the effective permissions, the
    /// union of the roles' sets and the extra grants.
    #[must_use]
    pub fn is_met_by(self, grants: &AccountGrants) -> bool {
        match self {
            Self::Public | Self::AnyActiveUser => true,
            Self::AdminRole => grants.is_admin(),
            Self::Permission(permission) => grants.effective().contains(&permission),
        }
    }
}

impl fmt::Display for Requirement {
    /// Writes the requirement in words, for log lines and test messages:
    /// `public`, `any active user`, `the admin role` or
    /// `the permission "edit posts"`.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => formatter.write_str("public"),
            Self::AnyActiveUser => formatter.write_str("any active user"),
            Self::AdminRole => formatter.write_str("the admin role"),
            Self::Permission(permission) => {
                write!(formatter, "the permission {:?}", permission.name())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prelude::*, sample::subsequence};

    use super::*;

    #[test]
    fn the_permission_names_are_the_sixteen_of_adr_0007() {
        assert_eq!(
            Permission::ALL.map(Permission::name),
            [
                "edit posts",
                "publish posts",
                "edit labels",
                "edit authors",
                "edit events",
                "edit canteens",
                "edit colleagues",
                "edit users",
                "edit pages",
                "edit menu",
                "manage media",
                "view logs",
                "edit notices",
                "edit banners",
                "edit site settings",
                "edit diaktoll",
            ]
        );
        // ALL is in declaration order, so sorted sets follow the ADR's table.
        assert!(Permission::ALL.is_sorted());
        for permission in Permission::ALL {
            assert_eq!(permission.to_string(), permission.name());
        }
    }

    #[test]
    fn the_retired_names_are_not_permissions() {
        // ADR 0007 follow-up points 1 and 2, confirmed by the maintainer
        // (research: DECISIONS R9 (D06)).
        for retired in ["edit newsletter", "manage backups"] {
            assert_eq!(Permission::from_name(retired), None, "{retired}");
        }
    }

    #[test]
    fn from_name_is_exact() {
        for permission in Permission::ALL {
            assert_eq!(Permission::from_name(permission.name()), Some(permission));
        }
        for near_miss in [
            "Edit posts",
            "edit_posts",
            "edit posts ",
            " edit posts",
            "edit  posts",
            "editposts",
            "edit post",
            "",
        ] {
            assert_eq!(Permission::from_name(near_miss), None, "{near_miss:?}");
        }
    }

    #[test]
    fn the_roles_keep_backend1_s_ids_and_names() {
        assert_eq!(
            Role::ALL.map(|role| (role.id(), role.name())),
            [
                (1, "writer"),
                (2, "secretary"),
                (3, "supervisor"),
                (4, "admin")
            ]
        );
        assert!(Role::ALL.is_sorted());
        for role in Role::ALL {
            assert_eq!(Role::from_id(role.id()), Some(role));
            assert_eq!(Role::from_name(role.name()), Some(role));
            assert_eq!(role.to_string(), role.name());
        }
        for id in [0, 5, -1, i64::MIN, i64::MAX] {
            assert_eq!(Role::from_id(id), None, "{id}");
        }
        for name in ["Admin", "admin ", "administrator", ""] {
            assert_eq!(Role::from_name(name), None, "{name:?}");
        }
    }

    /// ADR 0007 §2's table, transcribed row by row: for each permission, the
    /// roles with a ✓ (writer, secretary, supervisor, admin).
    const MATRIX: [(Permission, [bool; 4]); 16] = [
        (Permission::EditPosts, [true, false, true, true]),
        (Permission::PublishPosts, [false, false, true, true]),
        (Permission::EditLabels, [true, false, true, true]),
        (Permission::EditAuthors, [true, false, true, true]),
        (Permission::EditEvents, [false, true, false, true]),
        (Permission::EditCanteens, [false, true, false, true]),
        (Permission::EditColleagues, [false, true, false, true]),
        (Permission::EditUsers, [false, false, false, true]),
        (Permission::EditPages, [false, false, false, true]),
        (Permission::EditMenu, [false, false, false, true]),
        (Permission::ManageMedia, [true, false, true, true]),
        (Permission::ViewLogs, [false, false, false, true]),
        (Permission::EditNotices, [false, false, false, true]),
        (Permission::EditBanners, [false, false, false, true]),
        (Permission::EditSiteSettings, [false, false, false, true]),
        (Permission::EditDiaktoll, [false, false, false, true]),
    ];

    #[test]
    fn the_mapping_is_the_role_matrix_of_adr_0007() {
        assert_eq!(MATRIX.map(|(permission, _)| permission), Permission::ALL);
        for (permission, granted) in MATRIX {
            for (role, expected) in Role::ALL.into_iter().zip(granted) {
                assert_eq!(
                    role.permissions().contains(&permission),
                    expected,
                    "{role} / {permission}"
                );
            }
        }
    }

    #[test]
    fn the_mapping_lists_each_role_once_in_order_without_repeats() {
        assert_eq!(ROLE_PERMISSIONS.map(|(role, _)| role), Role::ALL);
        for (index, role) in Role::ALL.into_iter().enumerate() {
            assert_eq!(role.index(), index, "{role}");
            assert_eq!(role.permissions(), ROLE_PERMISSIONS[index].1, "{role}");
            // Strictly increasing: in ADR order, and no permission twice.
            assert!(
                role.permissions().windows(2).all(|pair| pair[0] < pair[1]),
                "{role}"
            );
        }
    }

    #[test]
    fn no_role_and_no_grant_hold_no_permission() {
        assert_eq!(effective_permissions([], []), BTreeSet::new());
        let nothing = AccountGrants::default();
        assert_eq!(nothing.effective(), BTreeSet::new());
        assert!(!nothing.is_admin());
    }

    #[test]
    fn the_effective_set_is_the_union_of_roles_and_grants() {
        assert_eq!(
            effective_permissions([Role::Secretary], [Permission::ManageMedia]),
            BTreeSet::from([
                Permission::EditEvents,
                Permission::EditCanteens,
                Permission::EditColleagues,
                Permission::ManageMedia,
            ])
        );
        // backend1's two-role accounts keep both roles (ADR 0007 §9).
        assert_eq!(
            effective_permissions([Role::Writer, Role::Secretary], []),
            BTreeSet::from([
                Permission::EditPosts,
                Permission::EditLabels,
                Permission::EditAuthors,
                Permission::EditEvents,
                Permission::EditCanteens,
                Permission::EditColleagues,
                Permission::ManageMedia,
            ])
        );
        // A grant that a role already gives adds nothing.
        assert_eq!(
            effective_permissions([Role::Writer], [Permission::EditPosts]),
            effective_permissions([Role::Writer], [])
        );
        assert_eq!(
            effective_permissions([Role::Admin], []),
            BTreeSet::from(Permission::ALL)
        );
    }

    #[test]
    fn only_the_admin_role_makes_an_admin() {
        assert!(is_admin([Role::Admin]));
        assert!(is_admin([Role::Writer, Role::Admin]));
        for role in [Role::Writer, Role::Secretary, Role::Supervisor] {
            assert!(!is_admin([role]), "{role}");
        }
        assert!(!is_admin([]));
        // Every permission as an extra grant is still no admin (ADR 0007 §2).
        let granted_everything = AccountGrants {
            roles: BTreeSet::new(),
            extra: BTreeSet::from(Permission::ALL),
        };
        assert_eq!(
            granted_everything.effective(),
            BTreeSet::from(Permission::ALL)
        );
        assert!(!granted_everything.is_admin());
    }

    #[test]
    fn stored_values_are_read_as_sets() {
        assert_eq!(
            AccountGrants::from_stored([4, 1, 4], ["edit events", "view logs", "edit events"]),
            Ok(AccountGrants {
                roles: BTreeSet::from([Role::Writer, Role::Admin]),
                extra: BTreeSet::from([Permission::EditEvents, Permission::ViewLogs]),
            })
        );
        assert_eq!(
            AccountGrants::from_stored([], []),
            Ok(AccountGrants::default())
        );
    }

    #[test]
    fn an_unknown_stored_value_is_refused_with_what_to_fix() {
        assert_eq!(
            AccountGrants::from_stored([1, 5], ["edit posts"]),
            Err(StoredGrantError::UnknownRoleId(5))
        );
        assert_eq!(
            AccountGrants::from_stored([1], ["edit posts", "edit newsletter"]),
            Err(StoredGrantError::UnknownPermission(
                "edit newsletter".to_owned()
            ))
        );
        // Roles are read first.
        assert_eq!(
            AccountGrants::from_stored([0], ["manage backups"]),
            Err(StoredGrantError::UnknownRoleId(0))
        );
        let role = StoredGrantError::UnknownRoleId(5).to_string();
        assert!(role.contains("role id 5"), "{role}");
        assert!(role.contains("roles table and the Role enum"), "{role}");
        let permission = StoredGrantError::UnknownPermission("x y".to_owned()).to_string();
        assert!(permission.contains("permission \"x y\""), "{permission}");
        assert!(
            permission.contains("user_permissions_permission_known"),
            "{permission}"
        );
    }

    /// The grants of an account with `roles` and the extra grants `extra`.
    fn account(roles: &[Role], extra: &[Permission]) -> AccountGrants {
        AccountGrants {
            roles: roles.iter().copied().collect(),
            extra: extra.iter().copied().collect(),
        }
    }

    #[test]
    fn only_public_lets_a_request_in_without_a_session() {
        assert!(!Requirement::Public.needs_session());
        for requirement in [
            Requirement::AnyActiveUser,
            Requirement::AdminRole,
            Requirement::Permission(Permission::EditPosts),
        ] {
            assert!(requirement.needs_session(), "{requirement}");
        }
    }

    #[test]
    fn an_account_with_nothing_meets_only_public_and_any_active_user() {
        // ADR 0007 §3: no role and no grant, so only the own account.
        let nothing = AccountGrants::default();
        assert!(Requirement::Public.is_met_by(&nothing));
        assert!(Requirement::AnyActiveUser.is_met_by(&nothing));
        assert!(!Requirement::AdminRole.is_met_by(&nothing));
        for permission in Permission::ALL {
            assert!(
                !Requirement::Permission(permission).is_met_by(&nothing),
                "{permission}"
            );
        }
    }

    #[test]
    fn a_role_meets_the_permissions_of_its_row_only() {
        // ADR 0007 §2: a writer edits posts but does not publish them.
        let writer = account(&[Role::Writer], &[]);
        assert!(Requirement::Permission(Permission::EditPosts).is_met_by(&writer));
        assert!(!Requirement::Permission(Permission::PublishPosts).is_met_by(&writer));
        assert!(!Requirement::AdminRole.is_met_by(&writer));
        // An extra grant counts like a role's permission (ADR 0007 §3).
        let secretary_with_menu = account(&[Role::Secretary], &[Permission::EditMenu]);
        assert!(Requirement::Permission(Permission::EditMenu).is_met_by(&secretary_with_menu));
        assert!(Requirement::Permission(Permission::EditEvents).is_met_by(&secretary_with_menu));
        assert!(!Requirement::Permission(Permission::EditPages).is_met_by(&secretary_with_menu));
    }

    #[test]
    fn every_extra_grant_still_does_not_make_an_admin() {
        // ADR 0007 §2: the three things the admin role unlocks are not
        // permissions, so no extra grant gives them.
        let every_grant = account(&[], &Permission::ALL);
        assert!(!Requirement::AdminRole.is_met_by(&every_grant));
        for permission in Permission::ALL {
            assert!(Requirement::Permission(permission).is_met_by(&every_grant));
        }
        assert!(Requirement::AdminRole.is_met_by(&account(&[Role::Admin], &[])));
    }

    #[test]
    fn requirements_read_in_words() {
        assert_eq!(Requirement::Public.to_string(), "public");
        assert_eq!(Requirement::AnyActiveUser.to_string(), "any active user");
        assert_eq!(Requirement::AdminRole.to_string(), "the admin role");
        assert_eq!(
            Requirement::Permission(Permission::EditSiteSettings).to_string(),
            r#"the permission "edit site settings""#
        );
    }

    /// Any requirement.
    fn requirement() -> impl Strategy<Value = Requirement> {
        prop_oneof![
            Just(Requirement::Public),
            Just(Requirement::AnyActiveUser),
            Just(Requirement::AdminRole),
            permission().prop_map(Requirement::Permission),
        ]
    }

    /// Any set of roles.
    fn roles() -> impl Strategy<Value = Vec<Role>> {
        subsequence(Role::ALL.to_vec(), 0..=Role::ALL.len())
    }

    /// Any set of extra grants.
    fn grants() -> impl Strategy<Value = Vec<Permission>> {
        subsequence(Permission::ALL.to_vec(), 0..=Permission::ALL.len())
    }

    /// Any role.
    fn role() -> impl Strategy<Value = Role> {
        proptest::sample::select(Role::ALL.to_vec())
    }

    /// Any permission.
    fn permission() -> impl Strategy<Value = Permission> {
        proptest::sample::select(Permission::ALL.to_vec())
    }

    proptest! {
        #[test]
        fn the_admin_role_holds_every_permission(roles in roles(), grants in grants()) {
            let mut roles = roles;
            roles.push(Role::Admin);
            prop_assert_eq!(
                effective_permissions(roles.iter().copied(), grants),
                BTreeSet::from(Permission::ALL)
            );
            prop_assert!(is_admin(roles));
        }

        #[test]
        fn adding_a_role_or_a_grant_never_removes_a_permission(
            roles in roles(),
            grants in grants(),
            added_role in role(),
            added_grant in permission(),
        ) {
            let before = effective_permissions(roles.iter().copied(), grants.iter().copied());
            let with_role = effective_permissions(
                roles.iter().copied().chain([added_role]),
                grants.iter().copied(),
            );
            let with_grant = effective_permissions(
                roles.iter().copied(),
                grants.iter().copied().chain([added_grant]),
            );
            prop_assert!(before.is_subset(&with_role));
            prop_assert!(before.is_subset(&with_grant));
            // And it adds what it confers.
            prop_assert!(added_role.permissions().iter().all(|p| with_role.contains(p)));
            prop_assert!(with_grant.contains(&added_grant));
        }

        #[test]
        fn a_permission_is_held_exactly_when_a_role_or_a_grant_gives_it(
            roles in roles(),
            grants in grants(),
        ) {
            let effective = effective_permissions(roles.iter().copied(), grants.iter().copied());
            for permission in Permission::ALL {
                let given = grants.contains(&permission)
                    || roles.iter().any(|role| role.permissions().contains(&permission));
                prop_assert_eq!(effective.contains(&permission), given, "{}", permission);
            }
            // Nothing yields nothing; anything else yields something, since
            // every role grants at least one permission.
            prop_assert_eq!(effective.is_empty(), roles.is_empty() && grants.is_empty());
        }

        #[test]
        fn is_admin_reads_the_role_and_never_the_grants(roles in roles(), grants in grants()) {
            let account = AccountGrants {
                roles: roles.iter().copied().collect(),
                extra: grants.into_iter().collect(),
            };
            prop_assert_eq!(account.is_admin(), roles.contains(&Role::Admin));
            prop_assert_eq!(
                account.effective(),
                effective_permissions(account.roles.iter().copied(), account.extra.iter().copied())
            );
        }

        #[test]
        fn a_requirement_is_met_exactly_as_adr_0007_reads_it(
            roles in roles(),
            grants in grants(),
            requirement in requirement(),
        ) {
            let account = account(&roles, &grants);
            let expected = match requirement {
                Requirement::Public | Requirement::AnyActiveUser => true,
                Requirement::AdminRole => roles.contains(&Role::Admin),
                Requirement::Permission(permission) => {
                    grants.contains(&permission)
                        || roles.iter().any(|role| role.permissions().contains(&permission))
                }
            };
            prop_assert_eq!(requirement.is_met_by(&account), expected, "{}", requirement);
            // The admin role meets every requirement (it holds every
            // permission, ADR 0007 §2).
            let mut with_admin = account.clone();
            with_admin.roles.insert(Role::Admin);
            prop_assert!(requirement.is_met_by(&with_admin), "{}", requirement);
        }

        #[test]
        fn adding_a_role_or_a_grant_never_takes_a_route_away(
            roles in roles(),
            grants in grants(),
            added_role in role(),
            added_grant in permission(),
            requirement in requirement(),
        ) {
            // A role change applies on the next request (ADR 0007 §5); giving
            // more never refuses what was allowed.
            let before = account(&roles, &grants);
            let mut with_role = before.clone();
            with_role.roles.insert(added_role);
            let mut with_grant = before.clone();
            with_grant.extra.insert(added_grant);
            if requirement.is_met_by(&before) {
                prop_assert!(requirement.is_met_by(&with_role), "{}", requirement);
                prop_assert!(requirement.is_met_by(&with_grant), "{}", requirement);
            }
            // And a grant of the asked permission always lets the account in.
            if let Requirement::Permission(permission) = requirement {
                let mut granted = before;
                granted.extra.insert(permission);
                prop_assert!(requirement.is_met_by(&granted));
            }
        }

        #[test]
        fn from_name_knows_exactly_the_sixteen_names(name in "\\PC{0,24}") {
            let known = Permission::ALL.iter().any(|permission| permission.name() == name);
            match Permission::from_name(&name) {
                Some(permission) => {
                    prop_assert!(known);
                    prop_assert_eq!(permission.name(), name.as_str());
                }
                None => prop_assert!(!known),
            }
        }

        #[test]
        fn a_changed_letter_makes_no_permission_name(
            index in 0usize..16,
            position in 0usize..18,
            letter in proptest::char::range(' ', '~'),
        ) {
            let name = Permission::ALL[index].name();
            let position = position % name.len();
            let mut changed: Vec<char> = name.chars().collect();
            prop_assume!(changed[position] != letter);
            changed[position] = letter;
            let changed: String = changed.into_iter().collect();
            prop_assume!(Permission::ALL.iter().all(|p| p.name() != changed));
            prop_assert_eq!(Permission::from_name(&changed), None);
        }

        #[test]
        fn role_lookups_know_exactly_the_four_roles(id in any::<i64>(), name in "\\PC{0,12}") {
            match Role::from_id(id) {
                Some(role) => prop_assert_eq!(role.id(), id),
                None => prop_assert!(!(1..=4).contains(&id)),
            }
            match Role::from_name(&name) {
                Some(role) => prop_assert_eq!(role.name(), name.as_str()),
                None => prop_assert!(Role::ALL.iter().all(|role| role.name() != name)),
            }
        }

        #[test]
        fn stored_values_round_trip(
            roles in roles(),
            grants in grants(),
            repeats in 1usize..3,
        ) {
            // Repeated and shuffled (reversed) inputs give the same sets.
            let role_ids: Vec<i64> = roles.iter().rev().map(|role| role.id()).collect();
            let names: Vec<&str> = grants.iter().rev().map(|grant| grant.name()).collect();
            let read = AccountGrants::from_stored(
                role_ids.iter().copied().cycle().take(role_ids.len() * repeats),
                names.iter().copied().cycle().take(names.len() * repeats),
            );
            prop_assert_eq!(
                read,
                Ok(AccountGrants {
                    roles: roles.into_iter().collect(),
                    extra: grants.into_iter().collect(),
                })
            );
        }

        #[test]
        fn an_unknown_stored_value_is_never_dropped(
            roles in roles(),
            grants in grants(),
            unknown_id in any::<i64>().prop_filter("not a role id", |id| !(1..=4).contains(id)),
            unknown_name in "\\PC{0,24}".prop_filter("not a permission", |name| {
                Permission::from_name(name).is_none()
            }),
            at in any::<prop::sample::Index>(),
        ) {
            let mut role_ids: Vec<i64> = roles.iter().map(|role| role.id()).collect();
            role_ids.insert(at.index(role_ids.len() + 1), unknown_id);
            prop_assert_eq!(
                AccountGrants::from_stored(role_ids, grants.iter().map(|grant| grant.name())),
                Err(StoredGrantError::UnknownRoleId(unknown_id))
            );
            let mut names: Vec<&str> = grants.iter().map(|grant| grant.name()).collect();
            names.insert(at.index(names.len() + 1), &unknown_name);
            prop_assert_eq!(
                AccountGrants::from_stored(roles.iter().map(|role| role.id()), names),
                Err(StoredGrantError::UnknownPermission(unknown_name.clone()))
            );
        }
    }
}
