//! The session settings of ADR 0002 ("Session settings") and ADR 0010 rule 9:
//! every connection backend3 opens runs with `TimeZone = UTC`, whatever the
//! server's, the database's or the role's default is.
//!
//! backend3 needs no code for this: sqlx-postgres sends `TimeZone=UTC` as a
//! startup parameter of every connection (sqlx-postgres 0.9,
//! `connection/establish.rs`), and PostgreSQL applies startup parameters
//! after the `options` switches and above database and role defaults (see
//! the `src/app.rs` module docs). These tests pin that behaviour. They give
//! the per-test database another default zone and show that the app's pool,
//! a connection opened the way Loco opens one, and a connection whose URI
//! asks for another zone through `options` all still run in UTC. If a driver
//! upgrade ever drops the parameter, they fail, and backend3 must then set
//! the zone itself.

use loco_rs::db;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};

use crate::support::with_app;

/// Runs a query that returns one text value.
async fn query_text(db: &DatabaseConnection, sql: &str) -> String {
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .expect("one row")
        .try_get_by_index::<String>(0)
        .expect("text column")
}

/// The session's `TimeZone` and where the setting came from
/// (`pg_settings.source`, for example `client` or `database`).
async fn time_zone_and_source(db: &DatabaseConnection) -> (String, String) {
    (
        query_text(db, "SHOW TimeZone").await,
        query_text(db, "SELECT source FROM pg_settings WHERE name = 'TimeZone'").await,
    )
}

#[tokio::test]
async fn the_app_pool_runs_in_utc() {
    with_app(|_router, ctx| async move {
        assert_eq!(
            time_zone_and_source(&ctx.db).await,
            ("UTC".to_owned(), "client".to_owned())
        );
    })
    .await;
}

#[tokio::test]
async fn utc_wins_over_database_defaults_and_options() {
    with_app(|_router, ctx| async move {
        let database = query_text(&ctx.db, "SELECT current_database()").await;
        ctx.db
            .execute_raw(Statement::from_string(
                DbBackend::Postgres,
                format!("ALTER DATABASE \"{database}\" SET timezone TO 'Europe/Budapest'"),
            ))
            .await
            .expect("give the per-test database another default zone");
        // The control: the database default really is another zone now.
        assert_eq!(
            query_text(
                &ctx.db,
                "SELECT array_to_string(setconfig, ',') FROM pg_db_role_setting \
                 WHERE setdatabase = (SELECT oid FROM pg_database \
                 WHERE datname = current_database()) AND setrole = 0",
            )
            .await,
            "TimeZone=Europe/Budapest"
        );

        // A database default applies to sessions that start after it is set,
        // so open new ones: as Loco does from the configuration, and with an
        // `options` parameter that asks for the other zone explicitly.
        let as_loco_connects = db::connect(&ctx.config.database)
            .await
            .expect("connect as Loco does");
        let separator = if ctx.config.database.uri.contains('?') {
            '&'
        } else {
            '?'
        };
        let with_options = Database::connect(format!(
            "{}{separator}options=-c%20TimeZone%3DEurope%2FBudapest",
            ctx.config.database.uri
        ))
        .await
        .expect("connect with an options parameter");

        let loco_session = time_zone_and_source(&as_loco_connects).await;
        let options_session = time_zone_and_source(&with_options).await;
        // Close both before the per-test database is dropped.
        as_loco_connects.close().await.expect("close");
        with_options.close().await.expect("close");

        let utc_from_client = ("UTC".to_owned(), "client".to_owned());
        assert_eq!(loco_session, utc_from_client);
        assert_eq!(options_session, utc_from_client);
    })
    .await;
}
