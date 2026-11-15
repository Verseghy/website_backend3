//! The account events of sign-in and logout on the `users` model (ADR 0006
//! §4, §2; ADR 0011 §4, §5): `authenticate` tells a known account from an
//! address that names none; `record_sign_in` sets `last_login_at` from
//! PostgreSQL's clock, never backwards, and writes `login` on the same
//! transaction, but only while the account is still the live, active one
//! at the token version that the sign-in checked; `record_failed_sign_in`
//! writes `login_failed` on a connection of its own, so a rollback
//! elsewhere leaves it; `log_out` revokes and writes `logout` on one
//! transaction. None of them writes a revision or touches `updated_at`, and
//! a refused entry rolls its change back.
//!
//! Addresses come from the documentation ranges of RFC 5737 and RFC 3849;
//! every account and password is invented.

use std::{net::IpAddr, time::Duration};

use backend3::{
    domain::activity::{Action, Actor, EntryError},
    models::{
        activity_log::{self, RequestContext},
        users::{self, AcceptedSignIn, ActiveModel, Model, RecordSignInError, SignInOutcome},
    },
};
use loco_rs::model::ModelError;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
    IntoActiveModel, QueryOrder, Statement, TransactionTrait,
    entity::prelude::DateTimeWithTimeZone,
};

use crate::support::with_app;

/// The password of every account below.
const PASSWORD: &str = "helyes jelszó 2026";

/// The request id of the requests below, in the form Loco makes.
const REQUEST_ID: &str = "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d";

/// The `updated_at` every account starts with.
fn instant() -> DateTimeWithTimeZone {
    DateTimeWithTimeZone::parse_from_rfc3339("2026-10-25T08:00:00+00:00").expect("RFC 3339")
}

/// A request with [`REQUEST_ID`] from `address`.
fn request_from(address: &str) -> RequestContext {
    RequestContext {
        request_id: Some(REQUEST_ID.to_owned()),
        ip: Some(address.parse().expect("an address")),
    }
}

/// Inserts a live, active account with `email`, the Argon2id hash of
/// [`PASSWORD`] and `updated_at` at [`instant`], after `change` has adjusted
/// it.
async fn account(
    db: &DatabaseConnection,
    email: &str,
    change: impl FnOnce(&mut ActiveModel),
) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Szerkesztő".to_owned()),
        password_hash: ActiveValue::Set(Some(users::hash_password(PASSWORD).expect("hash"))),
        updated_at: ActiveValue::Set(instant()),
        ..Default::default()
    };
    change(&mut new);
    new.insert(db).await.expect("insert the account")
}

/// The stored row of `user`.
async fn stored(db: &DatabaseConnection, user: &Model) -> Model {
    Model::find_by_pid(db, &user.pid.to_string())
        .await
        .expect("the stored account")
}

/// What [`entries`] returns while the log is empty.
const NO_ENTRIES: [activity_log::Model; 0] = [];

/// Every activity entry, oldest first.
async fn entries(db: &DatabaseConnection) -> Vec<activity_log::Model> {
    activity_log::Entity::find()
        .order_by_asc(activity_log::Column::Id)
        .all(db)
        .await
        .expect("read the activity log")
}

/// The number of revisions of every record.
async fn revision_count(db: &DatabaseConnection) -> i64 {
    value(db, "SELECT count(*) FROM revisions").await
}

/// The one value that `sql` returns on `db`.
async fn value<C, T>(db: &C, sql: &str) -> T
where
    C: ConnectionTrait,
    T: sea_orm::TryGetable,
{
    db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .expect(sql)
        .expect("one row")
        .try_get_by_index(0)
        .expect(sql)
}

/// Asserts that `entry` is the account entry `action` about `user`, by
/// `actor`, from [`request_from`]`(address)`.
fn assert_account_entry(
    entry: &activity_log::Model,
    action: Action,
    actor: Actor,
    user: &Model,
    address: &str,
) {
    assert_eq!(entry.action(), Ok(action));
    assert_eq!(entry.actor(), Ok(actor), "{action}");
    assert_eq!(entry.entity_type.as_deref(), Some("users"), "{action}");
    assert_eq!(entry.entity_id, Some(user.id), "{action}");
    assert_eq!(entry.revision_id, None, "{action}");
    // An account is a person: no label, ever (ADR 0011 §5).
    assert_eq!(entry.label, None, "{action}");
    assert_eq!(entry.details, None, "{action}");
    assert_eq!(entry.request_id.as_deref(), Some(REQUEST_ID), "{action}");
    assert_eq!(
        entry.client_address(),
        Some(address.parse::<IpAddr>().expect("an address")),
        "{action}"
    );
}

/// The [`EntryError`] inside a refusal of `activity_log::record`.
fn entry_error(error: &ModelError) -> &EntryError {
    match error {
        ModelError::Any(inner) => inner
            .downcast_ref::<EntryError>()
            .unwrap_or_else(|| panic!("not an EntryError: {inner}")),
        other => panic!("not ModelError::Any: {other:?}"),
    }
}

#[tokio::test]
async fn authenticate_tells_a_known_account_from_an_unknown_address() {
    // A known account is one the sign-in finds by its address: live, in any
    // other state (ADR 0006 §4). Its failures are `Refused` with its id; an
    // address that names none, a trashed account's included, is
    // `UnknownAccount`.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let live = account(db, "elo@example.test", |_| {}).await;
        let inactive = account(db, "inaktiv@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        let pending = account(db, "fuggo@example.test", |new| {
            new.password_hash = ActiveValue::Set(None);
        })
        .await;
        let two_factor = account(db, "ketlepcsos@example.test", |new| {
            new.totp_enabled_at = ActiveValue::Set(Some(instant()));
        })
        .await;
        let must_change = account(db, "csere@example.test", |new| {
            new.must_change_password = ActiveValue::Set(true);
        })
        .await;
        account(db, "lomtar@example.test", |new| {
            new.deleted_at = ActiveValue::Set(Some(instant()));
        })
        .await;

        let outcome = |email: &'static str, password: &'static str| {
            let db = db.clone();
            async move {
                Model::authenticate(&db, email, password, true)
                    .await
                    .expect("query")
            }
        };
        assert_eq!(
            outcome("ELO@example.test", PASSWORD).await,
            SignInOutcome::Accepted(Box::new(AcceptedSignIn {
                user: live.clone(),
                rehashed_password: None,
            }))
        );
        for (case, email, password, user) in [
            ("wrong password", "elo@example.test", "rossz", &live),
            ("inactive", "inaktiv@example.test", PASSWORD, &inactive),
            ("no password yet", "fuggo@example.test", PASSWORD, &pending),
            (
                "two-factor on",
                "ketlepcsos@example.test",
                PASSWORD,
                &two_factor,
            ),
            ("must change", "csere@example.test", PASSWORD, &must_change),
        ] {
            assert_eq!(
                outcome(email, password).await,
                SignInOutcome::Refused {
                    account_id: user.id
                },
                "{case}"
            );
        }
        for (case, email) in [
            ("no account", "nincs@example.test"),
            ("trashed", "lomtar@example.test"),
            ("no account can hold it", "elo\0@example.test"),
        ] {
            assert_eq!(
                outcome(email, PASSWORD).await,
                SignInOutcome::UnknownAccount,
                "{case}"
            );
        }
        // Only `Accepted` hands out the account.
        assert_eq!(
            SignInOutcome::Accepted(Box::new(AcceptedSignIn {
                user: live.clone(),
                rehashed_password: None,
            }))
            .accepted(),
            Some(live.clone())
        );
        assert_eq!(
            SignInOutcome::Refused {
                account_id: live.id
            }
            .accepted(),
            None
        );
        assert_eq!(SignInOutcome::UnknownAccount.accepted(), None);
        // `authenticate` itself writes nothing.
        assert_eq!(entries(db).await, NO_ENTRIES);
        assert_eq!(stored(db, &live).await, live);
    })
    .await;
}

#[tokio::test]
async fn record_sign_in_sets_last_login_at_and_writes_login_on_its_transaction() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "belep@example.test", |_| {}).await;
        let other = account(db, "marad@example.test", |_| {}).await;
        assert_eq!(user.last_login_at, None);

        let txn = db.begin().await.expect("begin");
        let transaction_start: DateTimeWithTimeZone = value(&txn, "SELECT now()").await;
        let signed_in = user
            .clone()
            .into_active_model()
            .record_sign_in(&txn, request_from("192.0.2.20"))
            .await
            .expect("record the sign-in")
            .expect("the account still matches");
        // Nothing is visible outside the transaction before it commits.
        assert_eq!(stored(db, &user).await.last_login_at, None);
        assert_eq!(entries(db).await, NO_ENTRIES);
        txn.commit().await.expect("commit");

        // `last_login_at` is PostgreSQL's clock at the start of the
        // transaction, the instant of the entry too (recipe 7 step 6).
        assert_eq!(signed_in.last_login_at, Some(transaction_start));
        assert_eq!(signed_in, stored(db, &user).await);
        let [login] = &entries(db).await[..] else {
            panic!("one entry expected");
        };
        assert_account_entry(
            login,
            Action::Login,
            Actor::User(user.id),
            &user,
            "192.0.2.20",
        );
        assert_eq!(Some(login.occurred_at), signed_in.last_login_at);

        // An account event (ADR 0011 §4): only `last_login_at` changed, not
        // `updated_at` or `updated_by`, and no revision was written.
        assert_eq!(
            Model {
                last_login_at: None,
                ..signed_in.clone()
            },
            user
        );
        assert_eq!(
            (signed_in.updated_at, signed_in.updated_by),
            (instant(), None)
        );
        assert_eq!(revision_count(db).await, 0);
        assert_eq!(stored(db, &other).await, other);

        // The next sign-in moves it on, again with the entry's instant.
        let txn = db.begin().await.expect("begin");
        let again = signed_in
            .into_active_model()
            .record_sign_in(&txn, request_from("2001:db8::20"))
            .await
            .expect("record the second sign-in")
            .expect("the account still matches");
        txn.commit().await.expect("commit");
        let all = entries(db).await;
        assert_eq!(all.len(), 2);
        assert_eq!(again.last_login_at, Some(all[1].occurred_at));
        assert!(all[1].occurred_at >= all[0].occurred_at);
        assert_eq!(all[1].client_address(), "2001:db8::20".parse().ok());
    })
    .await;
}

#[tokio::test]
async fn a_refused_login_entry_rolls_last_login_at_back() {
    // ADR 0011 §5: a failed insert of the entry rolls the change back. A
    // request id of 0 characters breaks a rule of `check_entry`, so
    // `record` refuses the entry after the `UPDATE`; the caller drops the
    // transaction.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "visszagorgetes@example.test", |_| {}).await;
        let txn = db.begin().await.expect("begin");
        let error = user
            .clone()
            .into_active_model()
            .record_sign_in(
                &txn,
                RequestContext {
                    request_id: Some(String::new()),
                    ip: None,
                },
            )
            .await
            .expect_err("the entry is refused");
        assert!(matches!(
            entry_error(&error),
            EntryError::RequestIdLength { chars: 0, .. }
        ));
        txn.rollback().await.expect("roll back");
        assert_eq!(stored(db, &user).await, user);
        assert_eq!(entries(db).await, NO_ENTRIES);
    })
    .await;
}

#[tokio::test]
async fn record_sign_in_writes_the_pending_changes_in_the_same_update() {
    // An account event that belongs to the sign-in that completes in this
    // request can ride in its `UPDATE`: the bcrypt-to-Argon2id rehash of
    // ADR 0006 §4 when the password step itself issues the full `admin`
    // session. An account that goes on to the two-factor step or the
    // forced password change gets no `admin` session at the password step,
    // so its rehash is an `UPDATE` of its own there, not this one.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "athash@example.test", |_| {}).await;
        let new_hash = users::hash_password(PASSWORD).expect("hash");
        let mut active = user.clone().into_active_model();
        active.password_hash = ActiveValue::Set(Some(new_hash.clone()));
        let txn = db.begin().await.expect("begin");
        let signed_in = active
            .record_sign_in(&txn, RequestContext::default())
            .await
            .expect("record the sign-in")
            .expect("the account still matches");
        txn.commit().await.expect("commit");
        assert_eq!(signed_in.password_hash, Some(new_hash));
        assert!(signed_in.last_login_at.is_some());
        assert_eq!(signed_in.updated_at, instant());
        // Without a request, the entry has neither part.
        let [login] = &entries(db).await[..] else {
            panic!("one entry expected");
        };
        assert_eq!((login.request_id.as_deref(), login.ip), (None, None));
    })
    .await;
}

#[tokio::test]
async fn record_sign_in_refuses_a_model_it_cannot_check_or_with_a_last_login_of_its_own() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "hibas@example.test", |_| {}).await;
        let no_id = ActiveModel {
            name: ActiveValue::Set("Próba Szerkesztő".to_owned()),
            ..Default::default()
        };
        let mut own_instant = user.clone().into_active_model();
        own_instant.last_login_at = ActiveValue::Set(Some(instant()));
        // The stored version, but set: the caller means to change it.
        let mut set_version = user.clone().into_active_model();
        set_version.token_version = ActiveValue::Set(user.token_version);
        let mut no_version = user.clone().into_active_model();
        no_version.token_version = ActiveValue::NotSet;
        for (case, active, expected) in [
            ("no id", no_id, RecordSignInError::NoId),
            (
                "a last_login_at of its own",
                own_instant,
                RecordSignInError::LastLoginAtSet,
            ),
            (
                "a token_version that was set",
                set_version,
                RecordSignInError::TokenVersionNotStored,
            ),
            (
                "no token_version",
                no_version,
                RecordSignInError::TokenVersionNotStored,
            ),
        ] {
            let txn = db.begin().await.expect("begin");
            let error = active
                .record_sign_in(&txn, RequestContext::default())
                .await
                .expect_err("refused before any query");
            txn.commit().await.expect("commit");
            // The message says what to fix.
            assert_eq!(error.to_string(), expected.to_string(), "{case}");
        }
        assert!(
            RecordSignInError::NoId
                .to_string()
                .contains("into_active_model")
        );
        assert!(
            RecordSignInError::LastLoginAtSet
                .to_string()
                .contains("PostgreSQL's clock")
        );
        assert!(
            RecordSignInError::TokenVersionNotStored
                .to_string()
                .contains("into_active_model")
        );
        // Nothing was written.
        assert_eq!(stored(db, &user).await, user);
        assert_eq!(entries(db).await, NO_ENTRIES);

        // An id that names no row matches nothing: no entry either.
        let mut gone = user.into_active_model();
        gone.id = ActiveValue::Unchanged(i64::MAX);
        let txn = db.begin().await.expect("begin");
        let missing = gone
            .record_sign_in(&txn, RequestContext::default())
            .await
            .expect("no error");
        assert_eq!(missing, None);
        txn.rollback().await.expect("roll back");
        assert_eq!(entries(db).await, NO_ENTRIES);
    })
    .await;
}

#[tokio::test]
async fn record_sign_in_writes_nothing_for_an_account_changed_since_the_check() {
    // ADR 0006 §1, §2: the session that the sign-in issues holds only while
    // the account is live, active and at the token version the token
    // carries. An `edit users` holder can change any of them while the
    // password is being verified; the sign-in then records nothing, so no
    // `login` entry ever names a session that the next request refuses.
    // An anonymised account is always in the trash too (ADR 0006 §7's
    // CHECKs), so the trash case stands for it.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        // Each case's `SET` list: how the account changes after the sign-in
        // checked it.
        for (case, set) in [
            ("deactivated", "active = false"),
            ("trashed", "deleted_at = now()"),
            ("sessions ended", "token_version = token_version + 1"),
        ] {
            let email = format!("{}@example.test", case.replace(' ', "."));
            let checked = account(db, &email, |_| {}).await;
            db.execute_unprepared(&format!("UPDATE users SET {set} WHERE id = {}", checked.id))
                .await
                .expect(case);
            let after_change = stored(db, &checked).await;

            let txn = db.begin().await.expect("begin");
            let recorded = checked
                .clone()
                .into_active_model()
                .record_sign_in(&txn, request_from("192.0.2.25"))
                .await
                .expect("no error");
            txn.commit().await.expect("commit");

            assert_eq!(recorded, None, "{case}");
            assert_eq!(stored(db, &checked).await, after_change, "{case}");
            assert_eq!(after_change.last_login_at, None, "{case}");
        }
        assert_eq!(entries(db).await, NO_ENTRIES);
    })
    .await;
}

#[tokio::test]
async fn last_login_at_never_moves_back() {
    // Two sign-ins of one account: the first begins first, but the second
    // commits first. Its `UPDATE` then runs after the second's commit and
    // reads the later instant, which `GREATEST` keeps, so the column stays
    // the instant of the newest `login` entry.
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "ketszer@example.test", |_| {}).await;

        let first = db.begin().await.expect("begin the first sign-in");
        let first_start: DateTimeWithTimeZone = value(&first, "SELECT now()").await;
        // `now()` has a resolution of one microsecond.
        tokio::time::sleep(Duration::from_millis(5)).await;
        let second = db.begin().await.expect("begin the second sign-in");
        let second_start: DateTimeWithTimeZone = value(&second, "SELECT now()").await;
        assert!(first_start < second_start, "{first_start} < {second_start}");

        let later = user
            .clone()
            .into_active_model()
            .record_sign_in(&second, request_from("192.0.2.21"))
            .await
            .expect("record the second sign-in")
            .expect("the account still matches");
        second.commit().await.expect("commit the second sign-in");
        assert_eq!(later.last_login_at, Some(second_start));

        let earlier = user
            .clone()
            .into_active_model()
            .record_sign_in(&first, request_from("192.0.2.22"))
            .await
            .expect("record the first sign-in")
            .expect("the account still matches");
        first.commit().await.expect("commit the first sign-in");

        // The column kept the later instant; both entries hold their own.
        assert_eq!(earlier.last_login_at, Some(second_start));
        assert_eq!(stored(db, &user).await.last_login_at, Some(second_start));
        let all = entries(db).await;
        let instants: Vec<_> = all.iter().map(|entry| entry.occurred_at).collect();
        assert_eq!(instants, [second_start, first_start]);
        assert_eq!(
            instants.iter().max().copied(),
            stored(db, &user).await.last_login_at
        );
    })
    .await;
}

#[tokio::test]
async fn record_failed_sign_in_writes_login_failed_on_its_own_connection() {
    // ADR 0011 §5: a failed sign-in changes no data and is written on its
    // own, so the rollback of a transaction that the request had open (the
    // throttle's writes, later) cannot take it back. The function takes the
    // pool, never a transaction (its doc test shows that a transaction does
    // not compile).
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "sikertelen@example.test", |_| {}).await;

        let request_work = db.begin().await.expect("begin");
        let noted = value::<_, i64>(&request_work, "SELECT count(*) FROM users").await;
        assert_eq!(noted, 1);
        let entry = users::record_failed_sign_in(db, user.id, request_from("192.0.2.30"))
            .await
            .expect("record the failure");
        request_work.rollback().await.expect("roll back");

        let [stored_entry] = &entries(db).await[..] else {
            panic!("one entry expected");
        };
        assert_eq!(stored_entry, &entry);
        // The caller has not signed in: the system writes it, about the
        // account (ADR 0011 §6 finds these by their record).
        assert_account_entry(
            stored_entry,
            Action::LoginFailed,
            Actor::System,
            &user,
            "192.0.2.30",
        );
        assert_eq!(stored_entry.actor_id, None);
        // The account itself is untouched, `last_login_at` included.
        assert_eq!(stored(db, &user).await, user);
        assert_eq!(revision_count(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn log_out_revokes_and_writes_logout_on_its_transaction() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "kilep@example.test", |new| {
            new.token_version = ActiveValue::Set(2);
        })
        .await;
        let txn = db.begin().await.expect("begin");
        let logged_out = user
            .clone()
            .into_active_model()
            .log_out(&txn, request_from("2001:db8::40"))
            .await
            .expect("log out");
        assert_eq!(stored(db, &user).await.token_version, 2, "not committed");
        txn.commit().await.expect("commit");

        assert_eq!(logged_out.token_version, 3);
        assert_eq!(logged_out, stored(db, &user).await);
        let [logout] = &entries(db).await[..] else {
            panic!("one entry expected");
        };
        assert_account_entry(
            logout,
            Action::Logout,
            Actor::User(user.id),
            &user,
            "2001:db8::40",
        );
        // An account event: the version alone moved, no revision.
        assert_eq!(
            Model {
                token_version: user.token_version,
                ..logged_out
            },
            user
        );
        assert_eq!(revision_count(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_refused_logout_entry_rolls_the_revocation_back() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let user = account(db, "marad.bent@example.test", |_| {}).await;
        let txn = db.begin().await.expect("begin");
        let error = user
            .clone()
            .into_active_model()
            .log_out(
                &txn,
                RequestContext {
                    request_id: Some("x".repeat(256)),
                    ip: None,
                },
            )
            .await
            .expect_err("the entry is refused");
        assert!(matches!(
            entry_error(&error),
            EntryError::RequestIdLength { chars: 256, .. }
        ));
        txn.rollback().await.expect("roll back");
        // The sessions hold: the version did not move.
        assert_eq!(stored(db, &user).await, user);
        assert_eq!(entries(db).await, NO_ENTRIES);
    })
    .await;
}
