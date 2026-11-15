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
//! [`RESOURCE_ACCESS`] is the access table of ADR 0007 §4 in the admin UI's
//! words: for each ra-core resource, the [`AccessAction`]s the admin offers
//! on it and what each asks of the account. [`access_map`] turns it into an
//! account's access map, and [`session_access_map`] into the one that
//! `/auth/me` sends a session (ADR 0007 §7), so that the admin's
//! `authProvider.canAccess` answers from it without asking the server. The map only spares the UI a refused request: the route table
//! still decides every request itself. Each resource slice adds its row
//! here, together with its routes, and its cases in
//! `tests/fixtures/authz_cases.json`, the case table that the Rust test
//! (`tests/domain/authz_cases.rs`) and the admin's Vitest test run alike.
//!
//! The record-level rules of ADR 0007 §5 (published posts, the user guards
//! of §6) join this module with the slices that need them.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use super::session::SessionScope;

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
/// The ADR's "or" rows (the labels and authors that `edit posts` may read
/// for the post form's pickers, the pages that `edit menu` may read for the
/// menu form's picker) are [`Requirement::AnyPermission`]. Only routes take
/// it; an [`ActionRule`] asks for the resource's own permission alone
/// (research: DECISIONS R35 (D153)).
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
    /// At least one of these permissions among the account's effective
    /// permissions: an "or" row of ADR 0007 §4, such as the labels'
    /// list and read, which `edit labels` gives and `edit posts` gives for
    /// the post form's picker. An empty list is met by no account (deny by
    /// default, ADR 0007 §5), so a route that forgot its permissions is
    /// closed rather than open.
    AnyPermission(&'static [Permission]),
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
    /// [`Requirement::Permission`] and [`Requirement::AnyPermission`] read
    /// the effective permissions, the union of the roles' sets and the extra
    /// grants.
    #[must_use]
    pub fn is_met_by(self, grants: &AccountGrants) -> bool {
        match self {
            Self::Public | Self::AnyActiveUser => true,
            Self::AdminRole => grants.is_admin(),
            Self::Permission(permission) => grants.effective().contains(&permission),
            Self::AnyPermission(permissions) => {
                let effective = grants.effective();
                permissions
                    .iter()
                    .any(|permission| effective.contains(permission))
            }
        }
    }
}

impl fmt::Display for Requirement {
    /// Writes the requirement in words, for log lines and test messages:
    /// `public`, `any active user`, `the admin role`,
    /// `the permission "edit posts"` or
    /// `any of the permissions "edit labels", "edit posts"`.
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
            Self::AnyPermission(permissions) => {
                formatter.write_str("any of the permissions")?;
                for (index, permission) in permissions.iter().enumerate() {
                    let separator = if index == 0 { " " } else { ", " };
                    write!(formatter, "{separator}{:?}", permission.name())?;
                }
                Ok(())
            }
        }
    }
}

/// An action that the admin UI asks `authProvider.canAccess` about on an
/// ra-core resource (ADR 0007 §7).
///
/// The first five are ra-core's own (its page components and the kit's
/// sidebar ask for `list`, `show`, `create` and `edit`; backend3's button
/// wrappers ask for `delete`); the last four are backend3's. The variants
/// are declared in the ADR's order and derive `Ord`, so an access map lists
/// a resource's actions in that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccessAction {
    /// `list`: the resource's list screen and its entry in the sidebar.
    List,
    /// `show`: one record, read only.
    Show,
    /// `create`: a new record (Clone is a create, ADR 0007 §4).
    Create,
    /// `edit`: an update of a record (reordering and reverting to a
    /// revision are updates, ADR 0007 §4).
    Edit,
    /// `delete`: the soft delete into the trash, „lomtár” (ADR 0011).
    Delete,
    /// `publish`: publishing and unpublishing (posts, ADR 0007 §4).
    Publish,
    /// `restore`: „Visszaállítás” from the trash (ADR 0011).
    Restore,
    /// `purge`: permanent delete from the trash, „Végleges törlés”, which
    /// only the `admin` role may do (ADR 0007 §2).
    Purge,
    /// `history`: one record's revisions, „Előzmények”, which ask for the
    /// resource's list permission (ADR 0007 §4).
    History,
}

impl AccessAction {
    /// Every action, in declaration order.
    pub const ALL: [Self; 9] = [
        Self::List,
        Self::Show,
        Self::Create,
        Self::Edit,
        Self::Delete,
        Self::Publish,
        Self::Restore,
        Self::Purge,
        Self::History,
    ];

    /// The action's name, as ra-core passes it to `canAccess` and as
    /// `/auth/me` sends it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Show => "show",
            Self::Create => "create",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Publish => "publish",
            Self::Restore => "restore",
            Self::Purge => "purge",
            Self::History => "history",
        }
    }

    /// The action whose name is exactly `name`, or `None`.
    ///
    /// - `name`: an action name as ra-core passes it to `canAccess`.
    ///
    /// The comparison is exact: `List` and `list ` name no action, and
    /// neither does an action that ra-core may ask about but backend3 never
    /// offers, such as `export` or `clone`. An access check of such a name
    /// is refused.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.name() == name)
    }
}

impl fmt::Display for AccessAction {
    /// Writes the action's name, for log lines and test messages.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// What one action of a resource asks of the account: every requirement in
/// [`ActionRule::requires`] (ADR 0007 §4).
///
/// More than one requirement is how the ADR's "also need" reads: publishing
/// a post asks for `edit posts` and `publish posts`. A row never carries an
/// "or" of ADR 0007 §4 (the labels and authors that `edit posts` may read for
/// the post form's pickers, the pages that `edit menu` may read for the menu
/// form): it asks only for the resource's own permission, and the other
/// permission goes on the routes the picker calls, whose
/// [`Requirement::AnyPermission`] (recipe 10 of the project skill) lets it
/// in. Asked whether such a row should carry the "or" too, so that an
/// account that holds only the picker's permission also sees the list screen
/// and is offered the resource's `history`, the maintainer answered "The row
/// asks for the resource's own permission; the "or" stays on the routes" on
/// 2026-11-15 (research: DECISIONS R35 (D153)). The picker is no screen of
/// the resource, so rules stay plain all-of lists. The labels' row does so
/// first: `edit posts` alone does not put „Címkék” in the sidebar. A row may
/// offer an action to fewer accounts than its routes let in, never to more
/// (the route table's tests check that).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionRule {
    /// The action the rule is about.
    pub action: AccessAction,
    /// What the account must meet, all of it. Never empty in the table (a
    /// test of [`RESOURCE_ACCESS`] says so); an empty list allows nothing.
    pub requires: &'static [Requirement],
}

impl ActionRule {
    /// A rule for `action` that asks for every requirement of `requires`.
    ///
    /// - `action`: the action.
    /// - `requires`: what the account must meet, all of it.
    #[must_use]
    pub const fn new(action: AccessAction, requires: &'static [Requirement]) -> Self {
        Self { action, requires }
    }

    /// Whether an account with `grants` may take this action.
    ///
    /// - `grants`: the account's roles and extra grants.
    ///
    /// Every requirement must be met ([`Requirement::is_met_by`]). A rule
    /// without requirements allows nothing: deny by default (ADR 0007 §5),
    /// so a row that forgot its requirement hides the action instead of
    /// offering it to everyone.
    #[must_use]
    pub fn is_met_by(&self, grants: &AccountGrants) -> bool {
        !self.requires.is_empty()
            && self
                .requires
                .iter()
                .all(|requirement| requirement.is_met_by(grants))
    }
}

/// One row of the access table: an ra-core resource and the rule of each
/// action the admin offers on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceAccess {
    /// The ra-core resource name, as `<Resource name>` registers it and as
    /// the access map's key: lower-case letters, digits, `-` and `_`,
    /// starting with a letter (a test of [`RESOURCE_ACCESS`] says so).
    pub resource: &'static str,
    /// The resource's actions, each at most once. An action that the row
    /// does not list is never offered.
    pub rules: &'static [ActionRule],
}

/// The access table of ADR 0007 §4, one row per ra-core resource, in the
/// order the resources are added.
///
/// Each resource slice adds its row here in the PR that mounts its first
/// routes, with the requirements that its routes carry in the route table,
/// and adds the cases of its row to `tests/fixtures/authz_cases.json`,
/// whose Rust test fails until every row has a case that allows and a case
/// that refuses. The rows so far:
///
/// - `labels`: `list` and `show`, the list and record reads that `edit
///   labels` gives. Their routes also let in `edit posts` for the post
///   form's label picker (ADR 0007 §4), which is no screen of the resource,
///   so `edit posts` alone is not offered them, as the maintainer answered
///   (research: DECISIONS R35 (D153); [`ActionRule`] says why).
///   `create` and `edit`, the label writes, ask for `edit labels` as their
///   routes do. The trash and the history add their actions with their
///   routes.
///
/// The tests of this module hold every row to the shape of ADR 0007 §4:
/// unique resource names, each
/// action once with at least one requirement and never
/// [`Requirement::Public`], `purge` only with the `admin` role, and
/// `history` with exactly the requirements of `list`.
///
/// The slice also marks each route the admin calls for an action with
/// `AdminRoute::serving` in the admin route table
/// (`controllers::admin::routes`), whose tests hold the rows to the routes:
/// every action of a row needs a route that serves it, a route may serve
/// only an action that a row offers, and a row may offer an action only to
/// accounts that every route serving it lets in, so the sidebar never
/// shows an entry that answers `403`.
pub const RESOURCE_ACCESS: &[ResourceAccess] = &[ResourceAccess {
    resource: "labels",
    rules: &[
        ActionRule::new(
            AccessAction::List,
            &[Requirement::Permission(Permission::EditLabels)],
        ),
        ActionRule::new(
            AccessAction::Show,
            &[Requirement::Permission(Permission::EditLabels)],
        ),
        ActionRule::new(
            AccessAction::Create,
            &[Requirement::Permission(Permission::EditLabels)],
        ),
        ActionRule::new(
            AccessAction::Edit,
            &[Requirement::Permission(Permission::EditLabels)],
        ),
    ],
}];

/// An account's access map: for each resource, the actions the account may
/// take, in [`AccessAction`] order (ADR 0007 §7).
///
/// A resource on which the account may take no action is left out, so an
/// account with no role and no grant has an empty map.
pub type AccessMap = BTreeMap<&'static str, BTreeSet<AccessAction>>;

/// The access map of an account with `grants` over `table`.
///
/// - `table`: the access rows; [`RESOURCE_ACCESS`] in the application, a
///   probe table in the tests.
/// - `grants`: the account's roles and extra grants, as the route table's
///   guard read them for the request.
///
/// An action is in the map exactly when its [`ActionRule::is_met_by`]
/// holds, so the map agrees with the route table as long as each row asks
/// for what its routes ask for. Rows are read in order; the tests of
/// [`RESOURCE_ACCESS`] rule out a resource or an action listed twice.
#[must_use]
pub fn access_map(table: &[ResourceAccess], grants: &AccountGrants) -> AccessMap {
    let mut map = AccessMap::new();
    for row in table {
        let allowed: BTreeSet<AccessAction> = row
            .rules
            .iter()
            .filter(|rule| rule.is_met_by(grants))
            .map(|rule| rule.action)
            .collect();
        if !allowed.is_empty() {
            map.entry(row.resource).or_default().extend(allowed);
        }
    }
    map
}

/// The access map that `/auth/me` sends a session of `scope`: the
/// account's [`access_map`] over `table` for a full `admin` session, and an
/// empty map for every other scope.
///
/// - `scope`: the session's scope, as the session check granted it.
/// - `table`: the access rows; [`RESOURCE_ACCESS`] in the application, a
///   probe table in the tests.
/// - `grants`: the account's roles and extra grants, as the route table's
///   guard read them for the request.
///
/// A `password_change` session reaches only the own-account routes of
/// ADR 0006 §4: the route table answers it `401` on every resource route,
/// so a sidebar entry would sign the user out instead of opening a screen.
/// An `mfa` session reaches no admin route at all. Neither is offered
/// anything, whatever the account holds.
#[must_use]
pub fn session_access_map(
    scope: SessionScope,
    table: &[ResourceAccess],
    grants: &AccountGrants,
) -> AccessMap {
    match scope {
        SessionScope::Admin => access_map(table, grants),
        SessionScope::PasswordChange | SessionScope::Mfa => AccessMap::new(),
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
            Requirement::AnyPermission(&[Permission::EditLabels, Permission::EditPosts]),
            Requirement::AnyPermission(&[]),
        ] {
            assert!(requirement.needs_session(), "{requirement}");
        }
    }

    #[test]
    fn any_permission_is_met_by_one_of_its_permissions_from_a_role_or_a_grant() {
        // ADR 0007 §4: labels are read with `edit labels`, or with
        // `edit posts` for the post form's picker.
        const PICKER: Requirement =
            Requirement::AnyPermission(&[Permission::EditLabels, Permission::EditPosts]);
        // Every permission, in a static, so that each one-element slice of
        // it below is `'static`.
        static EVERY: [Permission; 16] = Permission::ALL;
        for (grants, expected) in [
            (account(&[Role::Writer], &[]), true),
            (account(&[Role::Supervisor], &[]), true),
            (account(&[Role::Admin], &[]), true),
            (account(&[], &[Permission::EditPosts]), true),
            (account(&[], &[Permission::EditLabels]), true),
            (account(&[Role::Secretary], &[]), false),
            (
                account(&[Role::Secretary], &[Permission::EditAuthors]),
                false,
            ),
            (AccountGrants::default(), false),
        ] {
            assert_eq!(PICKER.is_met_by(&grants), expected, "{grants:?}");
        }
        // An empty list lets nobody in, not even the admin role with every
        // grant: deny by default.
        let everything = account(&[Role::Admin], &Permission::ALL);
        assert!(!Requirement::AnyPermission(&[]).is_met_by(&everything));
        // One permission in the list reads like `Permission`.
        for single in &EVERY {
            let one = std::slice::from_ref(single);
            let permission = *single;
            for grants in [account(&[Role::Writer], &[]), account(&[], &[permission])] {
                assert_eq!(
                    Requirement::AnyPermission(one).is_met_by(&grants),
                    Requirement::Permission(permission).is_met_by(&grants),
                    "{permission}"
                );
            }
        }
    }

    #[test]
    fn the_labels_row_offers_the_screens_and_the_writes_to_edit_labels_only() {
        // ADR 0007 §4: `edit labels` lists, shows, creates and edits labels.
        // `edit posts` reads them for the post form's picker, which is no
        // screen, so the access map offers it none of the labels' actions
        // (the read routes still let it in, the write routes do not).
        let labels = |grants: &AccountGrants| -> Vec<&'static str> {
            access_map(RESOURCE_ACCESS, grants)
                .get("labels")
                .map(|actions| actions.iter().map(|action| action.name()).collect())
                .unwrap_or_default()
        };
        for role in [Role::Writer, Role::Supervisor, Role::Admin] {
            assert_eq!(
                labels(&account(&[role], &[])),
                ["list", "show", "create", "edit"],
                "{role}"
            );
        }
        assert_eq!(
            labels(&account(&[], &[Permission::EditLabels])),
            ["list", "show", "create", "edit"]
        );
        for grants in [
            account(&[], &[Permission::EditPosts]),
            account(&[Role::Secretary], &[]),
            AccountGrants::default(),
        ] {
            assert!(labels(&grants).is_empty(), "{grants:?}");
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
        assert_eq!(
            Requirement::AnyPermission(&[Permission::EditLabels, Permission::EditPosts])
                .to_string(),
            r#"any of the permissions "edit labels", "edit posts""#
        );
        assert_eq!(
            Requirement::AnyPermission(&[Permission::EditMenu]).to_string(),
            r#"any of the permissions "edit menu""#
        );
    }

    /// Any requirement. An [`Requirement::AnyPermission`] lists one to four
    /// permissions; the empty list, which no account meets, has its own
    /// unit test. Its slices are leaked to get the `'static` slice the
    /// variant holds; a test process ends soon.
    fn requirement() -> impl Strategy<Value = Requirement> {
        prop_oneof![
            Just(Requirement::Public),
            Just(Requirement::AnyActiveUser),
            Just(Requirement::AdminRole),
            permission().prop_map(Requirement::Permission),
            subsequence(Permission::ALL.to_vec(), 1..=4)
                .prop_shuffle()
                .prop_map(|permissions| Requirement::AnyPermission(Vec::leak(permissions))),
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
            let given = |permission: &Permission| {
                grants.contains(permission)
                    || roles.iter().any(|role| role.permissions().contains(permission))
            };
            let expected = match requirement {
                Requirement::Public | Requirement::AnyActiveUser => true,
                Requirement::AdminRole => roles.contains(&Role::Admin),
                Requirement::Permission(permission) => given(&permission),
                Requirement::AnyPermission(permissions) => permissions.iter().any(given),
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
            // And a grant of the asked permission, or of any one of the
            // permissions an "or" asks for, always lets the account in.
            let asked: &[Permission] = match &requirement {
                Requirement::Permission(permission) => std::slice::from_ref(permission),
                Requirement::AnyPermission(permissions) => permissions,
                _ => &[],
            };
            for permission in asked {
                let mut granted = account(&[], &[]);
                granted.extra.insert(*permission);
                prop_assert!(requirement.is_met_by(&granted), "{}", requirement);
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

    // The access table and the access map (ADR 0007 §4, §7).

    #[test]
    fn the_action_names_are_ra_core_s_five_and_backend3_s_four() {
        assert_eq!(
            AccessAction::ALL.map(AccessAction::name),
            [
                "list", "show", "create", "edit", "delete", "publish", "restore", "purge",
                "history"
            ]
        );
        // ALL is in declaration order, so an access map lists the actions
        // in the ADR's order.
        assert!(AccessAction::ALL.is_sorted());
        for action in AccessAction::ALL {
            assert_eq!(AccessAction::from_name(action.name()), Some(action));
            assert_eq!(action.to_string(), action.name());
        }
        // ra-core may ask about actions that backend3 never offers, and
        // about other spellings: none of them is an action.
        for other in [
            "List", "list ", " list", "read", "update", "export", "clone", "*", "",
        ] {
            assert_eq!(AccessAction::from_name(other), None, "{other:?}");
        }
    }

    const EDIT_POSTS: Requirement = Requirement::Permission(Permission::EditPosts);
    const PUBLISH_POSTS: Requirement = Requirement::Permission(Permission::PublishPosts);
    const EDIT_EVENTS: Requirement = Requirement::Permission(Permission::EditEvents);

    /// A probe table in the shape of ADR 0007 §4: a resource with all nine
    /// actions and the two-permission `publish`, a resource with two, and a
    /// resource that any active user may read. Invented names: no resource
    /// slice owns them.
    const PROBE_TABLE: &[ResourceAccess] = &[
        ResourceAccess {
            resource: "probe-posts",
            rules: &[
                ActionRule::new(AccessAction::List, &[EDIT_POSTS]),
                ActionRule::new(AccessAction::Show, &[EDIT_POSTS]),
                ActionRule::new(AccessAction::Create, &[EDIT_POSTS]),
                ActionRule::new(AccessAction::Edit, &[EDIT_POSTS]),
                ActionRule::new(AccessAction::Delete, &[EDIT_POSTS]),
                ActionRule::new(AccessAction::Publish, &[EDIT_POSTS, PUBLISH_POSTS]),
                ActionRule::new(AccessAction::Restore, &[EDIT_POSTS]),
                ActionRule::new(AccessAction::Purge, &[Requirement::AdminRole]),
                ActionRule::new(AccessAction::History, &[EDIT_POSTS]),
            ],
        },
        ResourceAccess {
            resource: "probe_events",
            rules: &[
                ActionRule::new(AccessAction::Edit, &[EDIT_EVENTS]),
                ActionRule::new(AccessAction::List, &[EDIT_EVENTS]),
            ],
        },
        ResourceAccess {
            resource: "probe-account",
            rules: &[ActionRule::new(
                AccessAction::Show,
                &[Requirement::AnyActiveUser],
            )],
        },
    ];

    /// Whether `name` is an ra-core resource name the table accepts.
    fn is_resource_name(name: &str) -> bool {
        let mut chars = name.chars();
        chars.next().is_some_and(|first| first.is_ascii_lowercase())
            && chars.all(|rest| {
                rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '-' || rest == '_'
            })
    }

    /// What is wrong with `table` against ADR 0007 §4, one line per defect;
    /// empty for a table in the ADR's shape.
    fn table_problems(table: &[ResourceAccess]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        for row in table {
            let resource = row.resource;
            if !seen.insert(resource) {
                problems.push(format!("{resource:?}: the resource has two rows"));
            }
            if !is_resource_name(resource) {
                problems.push(format!(
                    "{resource:?}: not an ra-core resource name (lower-case letters, digits, - \
                     and _, starting with a letter)"
                ));
            }
            if row.rules.is_empty() {
                problems.push(format!("{resource:?}: the row offers no action"));
            }
            let mut actions = BTreeSet::new();
            for rule in row.rules {
                let action = rule.action;
                if !actions.insert(action) {
                    problems.push(format!("{resource:?}/{action}: the action is listed twice"));
                }
                if rule.requires.is_empty() {
                    problems.push(format!("{resource:?}/{action}: no requirement"));
                }
                if rule.requires.contains(&Requirement::Public) {
                    problems.push(format!(
                        "{resource:?}/{action}: public, but every resource action needs a session"
                    ));
                }
            }
            let requires = |wanted: AccessAction| {
                row.rules
                    .iter()
                    .find(|rule| rule.action == wanted)
                    .map(|rule| rule.requires)
            };
            if requires(AccessAction::Purge)
                .is_some_and(|purge| !purge.contains(&Requirement::AdminRole))
            {
                problems.push(format!(
                    "{resource:?}/purge: permanent delete needs the admin role (ADR 0007 §2)"
                ));
            }
            if let Some(history) = requires(AccessAction::History)
                && requires(AccessAction::List) != Some(history)
            {
                problems.push(format!(
                    "{resource:?}/history: a record's history asks for the resource's list \
                     permission (ADR 0007 §4)"
                ));
            }
        }
        problems
    }

    #[test]
    fn the_access_table_has_the_shape_of_adr_0007_section_4() {
        assert_eq!(table_problems(RESOURCE_ACCESS), Vec::<String>::new());
        assert_eq!(table_problems(PROBE_TABLE), Vec::<String>::new());
    }

    /// A table row with `resource` and `rules`, for the defect cases.
    const fn row(resource: &'static str, rules: &'static [ActionRule]) -> ResourceAccess {
        ResourceAccess { resource, rules }
    }

    #[test]
    fn the_table_check_names_every_defect() {
        const LIST: ActionRule = ActionRule::new(AccessAction::List, &[EDIT_POSTS]);
        const SHOW_FOR_NOBODY: ActionRule = ActionRule::new(AccessAction::Show, &[]);
        const SHOW_IN_PUBLIC: ActionRule =
            ActionRule::new(AccessAction::Show, &[Requirement::Public]);
        const PURGE_BY_PERMISSION: ActionRule = ActionRule::new(AccessAction::Purge, &[EDIT_POSTS]);
        const HISTORY_ASKING_MORE: ActionRule =
            ActionRule::new(AccessAction::History, &[EDIT_POSTS, PUBLISH_POSTS]);
        const HISTORY: ActionRule = ActionRule::new(AccessAction::History, &[EDIT_POSTS]);
        let defects: [(&[ResourceAccess], &str); 11] = [
            (&[row("twice", &[LIST]), row("twice", &[LIST])], "two rows"),
            (&[row("Labels", &[LIST])], "not an ra-core resource name"),
            (&[row("", &[LIST])], "not an ra-core resource name"),
            (&[row("1labels", &[LIST])], "not an ra-core resource name"),
            (&[row("lab els", &[LIST])], "not an ra-core resource name"),
            (&[row("none", &[])], "offers no action"),
            (&[row("repeat", &[LIST, LIST])], "listed twice"),
            (&[row("empty", &[SHOW_FOR_NOBODY])], "no requirement"),
            (
                &[row("open", &[SHOW_IN_PUBLIC])],
                "every resource action needs a session",
            ),
            (
                &[row("purge", &[PURGE_BY_PERMISSION])],
                "needs the admin role",
            ),
            (
                &[row("history", &[LIST, HISTORY_ASKING_MORE])],
                "list permission",
            ),
        ];
        for (table, defect) in defects {
            let problems = table_problems(table);
            assert!(
                problems.iter().any(|problem| problem.contains(defect)),
                "{defect}: {problems:?}"
            );
        }
        // A history without a list is a defect too.
        let problems = table_problems(&[row("orphan", &[HISTORY])]);
        assert!(
            problems.iter().any(|problem| problem.contains("orphan")),
            "{problems:?}"
        );
        // And the names the table accepts.
        for name in ["labels", "canteen-days", "canteen_days", "a1"] {
            assert!(is_resource_name(name), "{name}");
        }
    }

    #[test]
    fn a_rule_asks_for_every_requirement_and_an_empty_one_allows_nothing() {
        let publish = ActionRule::new(AccessAction::Publish, &[EDIT_POSTS, PUBLISH_POSTS]);
        // ADR 0007 §2: a writer edits posts but does not publish them; a
        // supervisor does both.
        assert!(!publish.is_met_by(&account(&[Role::Writer], &[])));
        assert!(publish.is_met_by(&account(&[Role::Supervisor], &[])));
        // `publish posts` alone is not enough: publishing needs `edit posts`
        // too (ADR 0007 §1).
        assert!(!publish.is_met_by(&account(&[], &[Permission::PublishPosts])));
        assert!(publish.is_met_by(&account(&[Role::Writer], &[Permission::PublishPosts])));
        // Deny by default, whoever asks.
        let empty = ActionRule::new(AccessAction::Show, &[]);
        assert!(!empty.is_met_by(&account(&[Role::Admin], &Permission::ALL)));
        assert!(!empty.is_met_by(&AccountGrants::default()));
    }

    /// The access map over [`PROBE_TABLE`] as resource and action names.
    fn probe_map(grants: &AccountGrants) -> Vec<(&'static str, Vec<&'static str>)> {
        access_map(PROBE_TABLE, grants)
            .into_iter()
            .map(|(resource, actions)| {
                (
                    resource,
                    actions.into_iter().map(AccessAction::name).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn the_access_map_lists_what_each_account_may_do() {
        // A writer: every post action but publish and purge, in the ADR's
        // order (not the row's), and the any-active-user resource.
        assert_eq!(
            probe_map(&account(&[Role::Writer], &[])),
            [
                ("probe-account", vec!["show"]),
                (
                    "probe-posts",
                    vec![
                        "list", "show", "create", "edit", "delete", "restore", "history"
                    ]
                ),
            ]
        );
        // A supervisor also publishes.
        assert_eq!(
            probe_map(&account(&[Role::Supervisor], &[]))[1],
            (
                "probe-posts",
                vec![
                    "list", "show", "create", "edit", "delete", "publish", "restore", "history"
                ]
            )
        );
        // A secretary sees the events, listed in the ADR's order.
        assert_eq!(
            probe_map(&account(&[Role::Secretary], &[])),
            [
                ("probe-account", vec!["show"]),
                ("probe_events", vec!["list", "edit"]),
            ]
        );
        // The admin role: every action of every row, purge included.
        let admin = access_map(PROBE_TABLE, &account(&[Role::Admin], &[]));
        for row in PROBE_TABLE {
            let every: BTreeSet<AccessAction> = row.rules.iter().map(|rule| rule.action).collect();
            assert_eq!(admin.get(row.resource), Some(&every), "{}", row.resource);
        }
        // Every permission as an extra grant: everything but purge, which
        // only the admin role gives (ADR 0007 §2).
        let granted = access_map(PROBE_TABLE, &account(&[], &Permission::ALL));
        assert!(!granted["probe-posts"].contains(&AccessAction::Purge));
        assert_eq!(granted["probe-posts"].len(), 8);
        // No role and no grant: only what any active user may do. A
        // resource without an allowed action is left out.
        assert_eq!(
            probe_map(&AccountGrants::default()),
            [("probe-account", vec!["show"])]
        );
        assert_eq!(
            access_map(&PROBE_TABLE[..2], &AccountGrants::default()),
            AccessMap::new()
        );
    }

    #[test]
    fn only_a_full_session_is_offered_the_account_s_access() {
        // A full session: the account's map, whatever it holds.
        for grants in [
            AccountGrants::default(),
            account(&[Role::Writer], &[]),
            account(&[Role::Admin], &Permission::ALL),
        ] {
            assert_eq!(
                session_access_map(SessionScope::Admin, PROBE_TABLE, &grants),
                access_map(PROBE_TABLE, &grants),
                "{grants:?}"
            );
        }
        // ADR 0006 §4: a password_change session reaches only the own
        // account's routes, and an mfa one no admin route, so neither is
        // offered anything, not even what any active user may do, and not
        // for the admin role with every grant.
        let everything = account(&[Role::Admin], &Permission::ALL);
        assert!(!access_map(PROBE_TABLE, &everything).is_empty());
        for scope in [SessionScope::PasswordChange, SessionScope::Mfa] {
            assert_eq!(
                session_access_map(scope, PROBE_TABLE, &everything),
                AccessMap::new(),
                "{scope:?}"
            );
            assert_eq!(
                session_access_map(scope, PROBE_TABLE, &AccountGrants::default()),
                AccessMap::new(),
                "{scope:?}"
            );
        }
    }

    #[test]
    fn the_application_table_gives_an_account_with_nothing_nothing() {
        // ADR 0007 §3: no role and no grant, so only the own account, which
        // is no ra-core resource.
        assert_eq!(
            access_map(RESOURCE_ACCESS, &AccountGrants::default()),
            AccessMap::new()
        );
        // And the admin role gets every action of every row.
        let admin = access_map(RESOURCE_ACCESS, &account(&[Role::Admin], &[]));
        for row in RESOURCE_ACCESS {
            assert_eq!(
                admin.get(row.resource).map(BTreeSet::len),
                Some(row.rules.len()),
                "{}",
                row.resource
            );
        }
    }

    /// Any requirement but [`Requirement::Public`], which no resource action
    /// carries.
    fn session_requirement() -> impl Strategy<Value = Requirement> {
        requirement().prop_filter("a resource action needs a session", |requirement| {
            requirement.needs_session()
        })
    }

    /// Any action.
    fn access_action() -> impl Strategy<Value = AccessAction> {
        proptest::sample::select(AccessAction::ALL.to_vec())
    }

    /// Any access table over three resource names, a name possibly twice,
    /// each rule with up to three requirements. The rows are leaked to get
    /// the `'static` slices of the real table; a test process ends soon.
    fn table() -> impl Strategy<Value = &'static [ResourceAccess]> {
        let rule = (
            access_action(),
            proptest::collection::vec(session_requirement(), 0..=3),
        )
            .prop_map(|(action, requires)| ActionRule::new(action, Vec::leak(requires)));
        let row = (
            proptest::sample::select(vec!["alpha", "beta", "gamma"]),
            proptest::collection::vec(rule, 0..=6),
        )
            .prop_map(|(resource, rules)| ResourceAccess {
                resource,
                rules: Vec::leak(rules),
            });
        proptest::collection::vec(row, 0..=4).prop_map(|rows| &*Vec::leak(rows))
    }

    proptest! {
        #[test]
        fn the_access_map_holds_exactly_the_rules_the_account_meets(
            table in table(),
            roles in roles(),
            grants in grants(),
        ) {
            let account = account(&roles, &grants);
            let map = access_map(table, &account);
            for (resource, actions) in &map {
                // Only resources of the table, never with an empty list.
                prop_assert!(table.iter().any(|row| row.resource == *resource));
                prop_assert!(!actions.is_empty(), "{}", resource);
            }
            for row in table {
                for rule in row.rules {
                    let expected = !rule.requires.is_empty()
                        && rule.requires.iter().all(|requirement| requirement.is_met_by(&account));
                    prop_assert_eq!(rule.is_met_by(&account), expected);
                    if expected {
                        prop_assert!(map[row.resource].contains(&rule.action));
                    }
                }
            }
            // An action in the map comes from a rule the account meets.
            for (resource, actions) in &map {
                for action in actions {
                    prop_assert!(table.iter().any(|row| {
                        row.resource == *resource
                            && row.rules.iter().any(|rule| {
                                rule.action == *action && rule.is_met_by(&account)
                            })
                    }), "{}/{}", resource, action);
                }
            }
        }

        #[test]
        fn adding_a_role_or_a_grant_never_takes_an_action_away(
            table in table(),
            roles in roles(),
            grants in grants(),
            added_role in role(),
            added_grant in permission(),
        ) {
            let before = access_map(table, &account(&roles, &grants));
            let mut more_roles = roles.clone();
            more_roles.push(added_role);
            let mut more_grants = grants.clone();
            more_grants.push(added_grant);
            for after in [
                access_map(table, &account(&more_roles, &grants)),
                access_map(table, &account(&roles, &more_grants)),
            ] {
                for (resource, actions) in &before {
                    prop_assert!(actions.is_subset(&after[resource]), "{}", resource);
                }
            }
        }

        #[test]
        fn the_admin_role_gets_every_action_that_has_a_requirement(
            table in table(),
            roles in roles(),
            grants in grants(),
        ) {
            let mut roles = roles;
            roles.push(Role::Admin);
            let map = access_map(table, &account(&roles, &grants));
            for row in table {
                for rule in row.rules.iter().filter(|rule| !rule.requires.is_empty()) {
                    prop_assert!(map[row.resource].contains(&rule.action));
                }
            }
        }

        #[test]
        fn the_access_map_reads_only_what_the_requirements_read(
            table in table(),
            roles in roles(),
            grants in grants(),
        ) {
            // Two accounts with the same effective permissions and the same
            // admin flag get the same map, however they got them: the
            // admin UI receives only those two (ADR 0007 §7), and the case
            // table describes an account by them.
            let account = account(&roles, &grants);
            let effective: Vec<Permission> = account.effective().into_iter().collect();
            let admin_roles: &[Role] = if account.is_admin() { &[Role::Admin] } else { &[] };
            let flattened = self::account(admin_roles, &effective);
            prop_assert_eq!(access_map(table, &account), access_map(table, &flattened));
        }

        #[test]
        fn a_session_is_offered_its_account_s_map_only_in_full_scope(
            table in table(),
            roles in roles(),
            grants in grants(),
            scope in proptest::sample::select(vec![
                SessionScope::Admin,
                SessionScope::PasswordChange,
                SessionScope::Mfa,
            ]),
        ) {
            let account = account(&roles, &grants);
            let offered = session_access_map(scope, table, &account);
            if scope == SessionScope::Admin {
                prop_assert_eq!(offered, access_map(table, &account));
            } else {
                prop_assert!(offered.is_empty(), "{:?}: {:?}", scope, offered);
            }
        }

        #[test]
        fn from_name_knows_exactly_the_nine_actions(name in "\\PC{0,10}") {
            match AccessAction::from_name(&name) {
                Some(action) => prop_assert_eq!(action.name(), name.as_str()),
                None => prop_assert!(AccessAction::ALL.iter().all(|action| action.name() != name)),
            }
        }
    }
}
