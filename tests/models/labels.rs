//! The `labels` model (parity LBL-01; ADR 0011 §3, ADR 0020 §4): the live
//! finders leave trashed labels out; a page of the list comes in the
//! Hungarian alphabet with the id tie-break, filtered and windowed as the
//! checked query asks, with the total of the whole list; and `App::truncate`
//! empties the table. Rows are written in SQL, because the label writes
//! (and their safety net) come with a later slice. Every name is invented.

use backend3::{
    app::App,
    domain::{
        labels::LIST,
        list_query::{self, ListQuery},
    },
    models::{
        labels::{Entity, Model},
        list_page,
    },
};
use loco_rs::{app::Hooks, model::ModelError};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, sea_query::Expr,
};

use crate::support::with_app;

/// The instant these tests write to `deleted_at`: only whether it is NULL
/// matters, so a fixed instant keeps the tests off the database's clock.
const TRASHED_AT: &str = "TIMESTAMPTZ '2026-11-15 08:00:00+00'";

/// Inserts a live label with an explicit id, as the importer will.
async fn label(db: &DatabaseConnection, id: i64, name: &str, color: &str) {
    db.execute_unprepared(&format!(
        "INSERT INTO labels (id, name, color) VALUES ({id}, '{name}', '{color}')"
    ))
    .await
    .unwrap_or_else(|error| panic!("{name}: {error}"));
}

/// Moves the label `id` to the trash, as the trash slice will.
async fn trash(db: &DatabaseConnection, id: i64) {
    db.execute_unprepared(&format!(
        "UPDATE labels SET deleted_at = {TRASHED_AT} WHERE id = {id}"
    ))
    .await
    .expect("trash");
}

/// The labels' list query of `raw`.
fn query(raw: &str) -> ListQuery {
    list_query::parse(&LIST, Some(raw)).unwrap_or_else(|error| panic!("{raw}: {error}"))
}

/// The ids of a page of the live labels for `raw`, and the total.
async fn page(db: &DatabaseConnection, raw: &str) -> (Vec<i64>, u64) {
    let page = Model::list_live_page(db, &query(raw))
        .await
        .expect("a page");
    (page.rows.iter().map(|row| row.id).collect(), page.total)
}

#[tokio::test]
async fn the_live_finders_leave_trashed_labels_out() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        label(db, 1, "Hírek", "#123456").await;
        label(db, 2, "Sport", "#654321").await;
        trash(db, 2).await;

        let live = Model::find_live().all(db).await.expect("live labels");
        assert_eq!(live.iter().map(|label| label.id).collect::<Vec<_>>(), [1]);
        // The trashed row is still there.
        assert_eq!(Entity::find().count(db).await.expect("count"), 2);

        let found = Model::find_live_by_id(db, 1).await.expect("live");
        assert_eq!(
            (found.id, found.name.as_str(), found.color.as_str()),
            (1, "Hírek", "#123456")
        );
        for missing in [2, 3, 0, -1] {
            assert!(
                matches!(
                    Model::find_live_by_id(db, missing).await,
                    Err(ModelError::EntityNotFound)
                ),
                "{missing}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_ten_words_come_in_hungarian_order() {
    // ADR 0020, the probe's ten words: only `hu` puts every `c…` before
    // every `cs…` and every `z…` before every `zs…`. Inserted in an order
    // that is neither the Hungarian nor the byte order.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let inserted = [
            "zsák", "alma", "óra", "csiga", "őz", "cukor", "zúgó", "olló", "ábra", "öröm",
        ];
        for (index, name) in inserted.iter().enumerate() {
            label(
                db,
                i64::try_from(index).expect("small") + 1,
                name,
                "#123456",
            )
            .await;
        }
        let page = Model::list_live_page(db, &query(""))
            .await
            .expect("the default order");
        assert_eq!(
            page.rows
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            [
                "ábra", "alma", "cukor", "csiga", "olló", "óra", "öröm", "őz", "zúgó", "zsák"
            ]
        );
        assert_eq!(page.total, 10);
        // Descending is the exact reverse: the names are distinct.
        let descending = Model::list_live_page(db, &query("sort=name&order=DESC"))
            .await
            .expect("descending");
        assert_eq!(
            descending
                .rows
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            [
                "zsák", "zúgó", "őz", "öröm", "óra", "olló", "csiga", "cukor", "alma", "ábra"
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn equal_sort_values_fall_back_to_the_id_descending() {
    // SHR-03: "then id descending", in both directions of the sort.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        label(db, 1, "Egy", "#AAAAAA").await;
        label(db, 2, "Kettő", "#BBBBBB").await;
        label(db, 3, "Három", "#AAAAAA").await;
        label(db, 4, "Négy", "#BBBBBB").await;
        assert_eq!(page(db, "sort=color&order=ASC").await.0, [3, 1, 4, 2]);
        assert_eq!(page(db, "sort=color&order=DESC").await.0, [4, 2, 3, 1]);
        // The id itself, both ways.
        assert_eq!(page(db, "sort=id&order=ASC").await.0, [1, 2, 3, 4]);
        assert_eq!(page(db, "sort=id&order=DESC").await.0, [4, 3, 2, 1]);
    })
    .await;
}

#[tokio::test]
async fn a_page_is_a_window_of_the_order_and_the_total_counts_the_whole_list() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        // 27 labels named so that the name order is the id order.
        for id in 1..=27 {
            label(db, id, &format!("Címke {id:02}"), "#123456").await;
        }
        trash(db, 27).await;
        let all: Vec<i64> = (1..=26).collect();
        assert_eq!(page(db, "").await, (all[..25].to_vec(), 26));
        assert_eq!(page(db, "page=2").await, (all[25..].to_vec(), 26));
        assert_eq!(
            page(db, "page=3&perPage=10").await,
            (all[20..].to_vec(), 26)
        );
        assert_eq!(
            page(db, "page=1&perPage=10").await,
            (all[..10].to_vec(), 26)
        );
        // Past the end: no rows, the total still there.
        assert_eq!(page(db, "page=4&perPage=10").await, (Vec::new(), 26));
        assert_eq!(
            page(db, "page=4294967295&perPage=100").await,
            (Vec::new(), 26)
        );
        // Every page of every size visits every live label once.
        for size in [10, 25, 50, 100] {
            let mut seen = Vec::new();
            for number in 1..=3 {
                seen.extend(page(db, &format!("page={number}&perPage={size}")).await.0);
            }
            assert_eq!(seen, all, "perPage={size}");
        }
    })
    .await;
}

#[tokio::test]
async fn the_id_filter_reads_the_live_labels_it_names() {
    // The label picker's getMany.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        label(db, 3, "Alma", "#123456").await;
        label(db, 5, "Barack", "#123456").await;
        label(db, 8, "Citrom", "#123456").await;
        trash(db, 8).await;
        // In the list's order, trashed and missing ids left out; the total
        // counts the filtered list.
        assert_eq!(page(db, "id=8&id=5&id=3&id=99").await, (vec![3, 5], 2));
        assert_eq!(page(db, "id=8").await, (Vec::new(), 0));
        // The filter and the window together.
        assert_eq!(
            page(db, "id=3&id=5&sort=name&order=DESC").await,
            (vec![5, 3], 2)
        );
    })
    .await;
}

#[tokio::test]
async fn the_page_and_the_total_are_read_in_one_read_only_snapshot() {
    // models::list_page: both statements run in the read-only
    // REPEATABLE READ transaction, so the total always matches the rows.
    // The select keeps a row only where the statement's own transaction is
    // that one, so a statement sent to the pool instead (READ COMMITTED,
    // read-write) finds nothing and the assertion names which one strayed.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        label(db, 1, "Hírek", "#123456").await;
        let in_the_snapshot = Model::find_live().filter(Expr::cust(
            "current_setting('transaction_isolation') = 'repeatable read' \
             AND current_setting('transaction_read_only') = 'on'",
        ));
        let page = list_page::fetch(db, in_the_snapshot, &query(""))
            .await
            .expect("a page");
        assert_eq!((page.rows.len(), page.total), (1, 1));
    })
    .await;
}

#[tokio::test]
async fn the_id_filter_reads_up_to_100_labels_on_one_page_of_100() {
    // A getMany of more than 25 ids sends perPage=100, so that every row it
    // names fits on the page (domain::list_query::MAX_FILTER_IDS); without
    // it, the default page of 25 holds the first 25, and the total counts
    // them all.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        for id in 1..=101 {
            label(db, id, &format!("Címke {id:03}"), "#123456").await;
        }
        let ids: Vec<String> = (2..=101).map(|id| format!("id={id}")).collect();
        let named: Vec<i64> = (2..=101).collect();
        assert_eq!(
            page(db, &format!("{}&perPage=100", ids.join("&"))).await,
            (named.clone(), 100)
        );
        assert_eq!(page(db, &ids.join("&")).await, (named[..25].to_vec(), 100));
    })
    .await;
}

#[tokio::test]
async fn truncate_empties_the_labels_and_leaves_the_accounts() {
    // Each entity slice adds its tables to App::truncate (recipe 1 step 2
    // of the project skill); the accounts stay (AGENTS.md, "Commands").
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        db.execute_unprepared(
            "INSERT INTO users (pid, name, email) \
             VALUES (gen_random_uuid(), 'Próba', 'cimkek@example.test')",
        )
        .await
        .expect("an account");
        db.execute_unprepared(
            "INSERT INTO labels (name, color, created_by) \
             SELECT 'Hírek', '#123456', id FROM users",
        )
        .await
        .expect("a label the account created");
        label(db, 9, "Sport", "#654321").await;
        trash(db, 9).await;
        App::truncate(&ctx).await.expect("truncate");
        assert_eq!(Entity::find().count(db).await.expect("count"), 0);
        assert_eq!(
            backend3::models::users::Entity::find()
                .count(db)
                .await
                .expect("count"),
            1
        );
    })
    .await;
}
