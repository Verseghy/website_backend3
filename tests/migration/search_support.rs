//! The `unaccent` extension and the `hu_icu` collation of ADR 0020 §1 exist
//! after migrating, behave as the ADR's probe recorded, and the migration
//! rolls back and re-applies cleanly. Running this in CI also settles, for
//! the compose.yaml image, that it ships `unaccent` and ICU (ADR 0020).

use migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

use crate::support::with_app;

async fn query_string(db: &DatabaseConnection, sql: &str) -> String {
    let row = db
        .query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .expect("one row");
    row.try_get_by_index::<String>(0).expect("text column")
}

async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    let row = db
        .query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .expect("one row");
    row.try_get_by_index::<i64>(0).expect("count")
}

const EXTENSION_COUNT: &str = "SELECT count(*) FROM pg_extension WHERE extname = 'unaccent'";
const COLLATION_COUNT: &str =
    "SELECT count(*) FROM pg_collation WHERE collname = 'hu_icu' AND collprovider = 'i'";

#[tokio::test]
async fn unaccent_and_hu_icu_exist_and_behave_as_specified() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;

        assert_eq!(count(db, EXTENSION_COUNT).await, 1);
        assert_eq!(count(db, COLLATION_COUNT).await, 1);

        // Folding as ADR 0020 §3 defines it.
        assert_eq!(
            query_string(db, "SELECT unaccent(lower('ÜNNEPI Műsor' COLLATE hu_icu))").await,
            "unnepi musor"
        );
        assert_eq!(
            query_string(db, "SELECT unaccent('ÁÉÍÓÖŐÚÜŰ áéíóöőúüű')").await,
            "AEIOOOUUU aeiooouuu"
        );

        // The probe's ten words in Hungarian order (ADR 0020, Context).
        assert_eq!(
            query_string(
                db,
                "SELECT string_agg(word, ',' ORDER BY word COLLATE hu_icu) FROM unnest(ARRAY[\
             'zsák','alma','óra','csiga','őz','cukor','zúgó','olló','ábra','öröm']) AS word"
            )
            .await,
            "ábra,alma,cukor,csiga,olló,óra,öröm,őz,zúgó,zsák"
        );
    })
    .await;
}

#[tokio::test]
async fn the_migration_rolls_back_and_reapplies() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;

        Migrator::down(db, None).await.expect("down");
        assert_eq!(count(db, EXTENSION_COUNT).await, 0);
        assert_eq!(count(db, COLLATION_COUNT).await, 0);

        Migrator::up(db, None).await.expect("up again");
        assert_eq!(count(db, EXTENSION_COUNT).await, 1);
        assert_eq!(count(db, COLLATION_COUNT).await, 1);
    })
    .await;
}
