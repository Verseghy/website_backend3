//! `users::Model::grants` (ADR 0007 §3, §5): the roles and extra grants that
//! an account holds, read in one statement on every admin request.
//!
//! An account reads exactly its own `user_roles` and `user_permissions`
//! rows, as sets; an account with neither reads an empty set, never "not
//! found" (which Loco would answer with `404`); a row that the code does not
//! know, which only a change outside the migrations can store, is an error
//! that names it; and a database failure is a database error. Every account
//! is invented.

use std::collections::BTreeSet;

use backend3::{
    domain::authz::{AccountGrants, Permission, Role, StoredGrantError},
    models::{
        user_permissions, user_roles,
        users::{ActiveModel, Model},
    },
};
use loco_rs::{
    model::ModelError,
    prelude::{ActiveModelTrait, ActiveValue},
};
use sea_orm::{ConnectionTrait, DatabaseConnection};

use crate::support::with_app;

/// Inserts a live, active account with `email` and no password.
async fn account(db: &DatabaseConnection, email: &str) -> Model {
    ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Jogosult".to_owned()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("insert the account")
}

/// Gives `user` the role `role`, through the model.
async fn give_role(db: &DatabaseConnection, user: &Model, role: Role) {
    user_roles::ActiveModel {
        user_id: ActiveValue::Set(user.id),
        role_id: ActiveValue::Set(role.id()),
    }
    .insert(db)
    .await
    .expect("insert the role link");
}

/// Gives `user` the extra grant `permission`, through the model.
async fn give_grant(db: &DatabaseConnection, user: &Model, permission: Permission) {
    user_permissions::ActiveModel {
        user_id: ActiveValue::Set(user.id),
        permission: ActiveValue::Set(permission.name().to_owned()),
    }
    .insert(db)
    .await
    .expect("insert the grant");
}

#[tokio::test]
async fn an_account_reads_exactly_its_own_roles_and_grants() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "jogosult@example.test").await;
        let other = account(db, "masik@example.test").await;
        give_role(db, &user, Role::Writer).await;
        give_role(db, &user, Role::Secretary).await;
        give_grant(db, &user, Permission::ViewLogs).await;
        give_grant(db, &user, Permission::EditEvents).await;
        // Another account's rows never leak in.
        give_role(db, &other, Role::Admin).await;
        give_grant(db, &other, Permission::EditUsers).await;

        let grants = user.grants(db).await.expect("grants");

        assert_eq!(
            grants,
            AccountGrants {
                roles: [Role::Writer, Role::Secretary].into(),
                extra: [Permission::ViewLogs, Permission::EditEvents].into(),
            }
        );
        assert!(!grants.is_admin());
        // A grant that a role already gives (secretary: edit events) adds
        // nothing to the effective set; view logs does.
        assert_eq!(
            grants.effective(),
            [
                Permission::EditPosts,
                Permission::EditLabels,
                Permission::EditAuthors,
                Permission::EditEvents,
                Permission::EditCanteens,
                Permission::EditColleagues,
                Permission::ManageMedia,
                Permission::ViewLogs,
            ]
            .into()
        );
        let other = other.grants(db).await.expect("grants");
        assert!(other.is_admin());
        assert_eq!(other.extra, [Permission::EditUsers].into());
    })
    .await;
}

#[tokio::test]
async fn an_account_without_roles_or_grants_reads_an_empty_set() {
    with_app(|_router, ctx| async move {
        let user = account(&ctx.db, "semmi@example.test").await;
        let grants = user.grants(&ctx.db).await.expect("not an error");
        assert_eq!(grants, AccountGrants::default());
        assert_eq!(grants.effective(), BTreeSet::new());
    })
    .await;
}

#[tokio::test]
async fn every_one_of_the_sixteen_grants_is_read_back() {
    with_app(|_router, ctx| async move {
        let user = account(&ctx.db, "mind@example.test").await;
        for permission in Permission::ALL {
            give_grant(&ctx.db, &user, permission).await;
        }
        let grants = user.grants(&ctx.db).await.expect("grants");
        assert_eq!(grants.extra, Permission::ALL.into());
        // Every permission, and still no admin (ADR 0007 §2).
        assert!(!grants.is_admin());
    })
    .await;
}

#[tokio::test]
async fn a_change_shows_on_the_next_read() {
    // Nothing is cached (ADR 0016): the extractor's next read sees it.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "valtozo@example.test").await;
        give_role(db, &user, Role::Writer).await;
        assert!(!user.grants(db).await.expect("grants").is_admin());

        give_role(db, &user, Role::Admin).await;
        assert!(user.grants(db).await.expect("grants").is_admin());

        db.execute_unprepared(&format!(
            "DELETE FROM user_roles WHERE user_id = {}",
            user.id
        ))
        .await
        .expect("remove the roles");
        assert_eq!(
            user.grants(db).await.expect("grants"),
            AccountGrants::default()
        );
    })
    .await;
}

#[tokio::test]
async fn a_stored_value_the_code_does_not_know_is_an_error_that_names_it() {
    // Only a change made outside the migrations can store one: this test's
    // own database gets a fifth role and loses the permission CHECK.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let role_user = account(db, "otodik@example.test").await;
        db.execute_unprepared(&format!(
            "INSERT INTO roles (id, name) VALUES (5, 'editor'); \
             INSERT INTO user_roles (user_id, role_id) VALUES ({}, 5)",
            role_user.id
        ))
        .await
        .expect("a role the code does not define");
        let grant_user = account(db, "hirlevel@example.test").await;
        db.execute_unprepared(&format!(
            "ALTER TABLE user_permissions DROP CONSTRAINT user_permissions_permission_known; \
             INSERT INTO user_permissions (user_id, permission) VALUES ({}, 'edit newsletter')",
            grant_user.id
        ))
        .await
        .expect("a permission the code does not define");

        for (user, expected) in [
            (&role_user, StoredGrantError::UnknownRoleId(5)),
            (
                &grant_user,
                StoredGrantError::UnknownPermission("edit newsletter".to_owned()),
            ),
        ] {
            let error = user.grants(db).await.expect_err("refused");
            // `Any`, never `EntityNotFound`, which Loco answers with 404.
            assert!(matches!(error, ModelError::Any(_)), "{error:?}");
            assert_eq!(error.to_string(), expected.to_string());
        }
    })
    .await;
}

#[tokio::test]
async fn a_database_failure_is_a_database_error() {
    with_app(|_router, ctx| async move {
        let user = account(&ctx.db, "kieses@example.test").await;
        // `with_app` closes the pool again afterwards, which is a no-op.
        ctx.db.close_by_ref().await.expect("close the pool");
        let error = user.grants(&ctx.db).await.expect_err("no database");
        assert!(matches!(error, ModelError::DbErr(_)), "{error:?}");
    })
    .await;
}
