//! `roles::Model::list_in_id_order`, the finder with which the importer
//! compares the target's roles with the code (data-migration.md §6.13,
//! ADR 0007 §3).

use backend3::{domain::authz::Role, models::roles};
use sea_orm::ConnectionTrait;

use crate::support::with_app;

#[tokio::test]
async fn the_finder_lists_the_seeded_roles_in_id_order() {
    with_app(|_router, ctx| async move {
        let found: Vec<(i64, String)> = roles::Model::list_in_id_order(&ctx.db)
            .await
            .expect("the roles")
            .into_iter()
            .map(|role| (role.id, role.name))
            .collect();
        let expected: Vec<(i64, String)> = Role::ALL
            .iter()
            .map(|role| (role.id(), role.name().to_owned()))
            .collect();
        assert_eq!(found, expected);

        // A row inserted out of order still comes back in id order.
        ctx.db
            .execute_unprepared("INSERT INTO roles (id, name) VALUES (0, 'proba-szerepkor')")
            .await
            .expect("a role the code does not define");
        let ids: Vec<i64> = roles::Model::list_in_id_order(&ctx.db)
            .await
            .expect("the roles")
            .iter()
            .map(|role| role.id)
            .collect();
        assert_eq!(ids, [0, 1, 2, 3, 4]);
    })
    .await;
}
