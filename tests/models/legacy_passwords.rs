//! Legacy bcrypt passwords on the `users` model (ADR 0006 §4, §6; ADR 0011
//! §4, §5): `check_stored_password` verifies the Laravel `$2y$` hash that
//! the importer copies, and its `$2a$` and `$2b$` spellings, with the bcrypt
//! crate, truncating a password at 72 bytes as Laravel did; `authenticate`
//! accepts it with the Argon2id rehash of the password, which
//! `record_sign_in` stores in the sign-in's own `UPDATE`, with no revision,
//! no change of `updated_at` and no entry besides `login`; the next sign-in
//! verifies the Argon2id hash. A failed or refused sign-in, and one whose
//! account changed between the check and the write, leave the bcrypt hash
//! in place, and so does a legacy password shorter than 12 characters while
//! `force_change_short_legacy_password` is on.
//!
//! Every hash here is made at run time from an invented password (ADR 0006,
//! follow-up work; ADR 0034 §2): none is a credential.

use backend3::models::{
    activity_log::{self, RequestContext},
    users::{
        self, AcceptedSignIn, ActiveModel, Model, PasswordCheck, SignInOutcome,
        check_stored_password,
    },
};
use bcrypt::Version;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
    IntoActiveModel, QueryOrder, Statement, TransactionTrait,
    entity::prelude::DateTimeWithTimeZone,
};

use crate::support::with_app;

/// The invented password of the legacy accounts below: 18 characters.
const PASSWORD: &str = "helyes jelszó 2026";
/// An invented legacy password of 11 characters, one short of ADR 0006 §6's
/// minimum.
const SHORT_PASSWORD: &str = "rovid jelsz";

/// The `updated_at` every account starts with.
fn instant() -> DateTimeWithTimeZone {
    DateTimeWithTimeZone::parse_from_rfc3339("2026-10-25T08:00:00+00:00").expect("RFC 3339")
}

/// The bcrypt hash of `password` at `cost`, written with the prefix of
/// `version`, as Laravel (`$2y$`) or another library wrote it.
///
/// - `password`: the invented password.
/// - `cost`: the bcrypt cost; 10 for the shape of every imported hash, 4
///   (the least) where the cost does not matter, to keep the tests quick.
/// - `version`: the prefix.
fn legacy_hash(password: &str, cost: u32, version: Version) -> String {
    bcrypt::hash_with_result(password, cost)
        .expect("bcrypt hash")
        .format_for_version(version)
}

/// Inserts a live, active account with `email`, the stored hash `hash` and
/// `updated_at` at [`instant`], after `change` has adjusted it.
async fn account(
    db: &DatabaseConnection,
    email: &str,
    hash: &str,
    change: impl FnOnce(&mut ActiveModel),
) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Régi Szerkesztő".to_owned()),
        password_hash: ActiveValue::Set(Some(hash.to_owned())),
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

/// The actions of every activity entry, oldest first.
async fn actions(db: &DatabaseConnection) -> Vec<String> {
    activity_log::Entity::find()
        .order_by_asc(activity_log::Column::Id)
        .all(db)
        .await
        .expect("read the activity log")
        .into_iter()
        .map(|entry| entry.action)
        .collect()
}

/// The number of revisions of every record.
async fn revision_count(db: &DatabaseConnection) -> i64 {
    db.query_one_raw(Statement::from_string(
        DbBackend::Postgres,
        "SELECT count(*) FROM revisions",
    ))
    .await
    .expect("count")
    .expect("one row")
    .try_get_by_index(0)
    .expect("a count")
}

/// The accepted sign-in of `email` with `password`; panics on any other
/// outcome.
async fn accepted(
    db: &DatabaseConnection,
    email: &str,
    password: &str,
    force_change_short: bool,
) -> AcceptedSignIn {
    match Model::authenticate(db, email, password, force_change_short)
        .await
        .expect("query")
    {
        SignInOutcome::Accepted(sign_in) => *sign_in,
        other => panic!("{email}: not accepted: {other:?}"),
    }
}

/// Records `sign_in` on a transaction of its own, as the sign-in handlers
/// do, and returns what `record_sign_in` returned.
async fn record(db: &DatabaseConnection, sign_in: AcceptedSignIn) -> Option<Model> {
    let txn = db.begin().await.expect("begin");
    let recorded = sign_in
        .into_active_model()
        .record_sign_in(&txn, RequestContext::default())
        .await
        .expect("record the sign-in");
    txn.commit().await.expect("commit");
    recorded
}

#[test]
fn a_legacy_hash_verifies_its_own_password_in_every_spelling() {
    for (version, prefix) in [
        (Version::TwoY, "$2y$04$"),
        (Version::TwoA, "$2a$04$"),
        (Version::TwoB, "$2b$04$"),
    ] {
        let hash = legacy_hash(PASSWORD, 4, version);
        assert!(hash.starts_with(prefix), "{hash}");
        assert_eq!(hash.len(), 60, "{hash}");
        assert_eq!(
            check_stored_password(Some(&hash), PASSWORD),
            PasswordCheck::MatchesLegacy,
            "{prefix}"
        );
        assert!(users::verify_stored_password(Some(&hash), PASSWORD));
        for wrong in [
            "",
            "helyes jelszo 2026",
            "Helyes jelszó 2026",
            "helyes jelszó",
        ] {
            assert_eq!(
                check_stored_password(Some(&hash), wrong),
                PasswordCheck::Fails,
                "{prefix} {wrong:?}"
            );
        }
    }
    // An Argon2id hash matches as itself, never as a legacy hash.
    let argon2id = users::hash_password(PASSWORD).expect("hash");
    assert_eq!(
        check_stored_password(Some(&argon2id), PASSWORD),
        PasswordCheck::Matches
    );
    assert!(PasswordCheck::Matches.matched());
    assert!(PasswordCheck::MatchesLegacy.matched());
    assert!(!PasswordCheck::Fails.matched());
}

#[test]
fn a_legacy_password_longer_than_72_bytes_verifies_as_laravel_hashed_it() {
    // bcrypt reads at most 72 bytes, so Laravel hashed only the first 72
    // bytes of a longer password; the crate's `verify` truncates the typed
    // password the same way, and its owner still signs in.
    let head = "a".repeat(72);
    let typed = format!("{head} és ami utána jön");
    let hash = legacy_hash(&head, 4, Version::TwoY);
    assert_eq!(
        check_stored_password(Some(&hash), &typed),
        PasswordCheck::MatchesLegacy
    );
    assert_eq!(
        check_stored_password(Some(&hash), &"a".repeat(71)),
        PasswordCheck::Fails
    );
}

#[test]
fn a_bcrypt_prefix_on_a_value_the_crate_cannot_read_never_matches() {
    // 60 characters each where the length is not the defect: the crate
    // refuses a cost outside 4 to 31 or one that is not a number before any
    // bcrypt work, and the dummy verification pays for the refusal.
    for stored in [
        "$2y$",
        "$2y$10$",
        "$2y$10$too-short",
        "$2b$99$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234",
        "$2a$xx$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234",
    ] {
        assert_eq!(
            check_stored_password(Some(stored), PASSWORD),
            PasswordCheck::Fails,
            "{stored}"
        );
    }
}

#[tokio::test]
async fn a_legacy_sign_in_stores_the_argon2id_rehash_with_the_sign_in() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        // The shape of every imported hash: Laravel's `$2y$`, cost 10
        // (data-migration.md §6.12), generated here (ADR 0006, follow-up
        // work).
        let legacy = legacy_hash(PASSWORD, 10, Version::TwoY);
        assert!(legacy.starts_with("$2y$10$"), "{legacy}");
        let user = account(db, "regi@example.test", &legacy, |_| {}).await;

        let sign_in = accepted(db, "Regi@Example.test", PASSWORD, true).await;
        assert_eq!(sign_in.user, user);
        let rehashed = sign_in.rehashed_password.clone().expect("a rehash");
        assert!(
            rehashed.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "{rehashed}"
        );
        assert_eq!(
            check_stored_password(Some(&rehashed), PASSWORD),
            PasswordCheck::Matches
        );
        // `authenticate` writes nothing; the sign-in stores the rehash.
        assert_eq!(stored(db, &user).await, user);

        let recorded = record(db, sign_in).await.expect("the account still holds");
        let after = stored(db, &user).await;
        assert_eq!(recorded, after);
        assert_eq!(after.password_hash.as_deref(), Some(rehashed.as_str()));
        assert!(after.last_login_at.is_some());
        // An account event: `updated_at`, `updated_by` and the token
        // version stay, no revision is written, and `login` is the only
        // entry (ADR 0011 §4, §5).
        assert_eq!(after.updated_at, user.updated_at);
        assert_eq!(after.updated_by, user.updated_by);
        assert_eq!(after.token_version, user.token_version);
        assert_eq!(revision_count(db).await, 0);
        assert_eq!(actions(db).await, ["login"]);

        // The next sign-in verifies the Argon2id hash and rehashes nothing.
        let next = accepted(db, "regi@example.test", PASSWORD, true).await;
        assert_eq!(next.rehashed_password, None);
        assert_eq!(
            check_stored_password(next.user.password_hash.as_deref(), PASSWORD),
            PasswordCheck::Matches
        );
        // Recording it leaves the stored hash as it is.
        record(db, next).await.expect("the account still holds");
        assert_eq!(
            stored(db, &user).await.password_hash.as_deref(),
            Some(rehashed.as_str())
        );
        assert_eq!(actions(db).await, ["login", "login"]);
    })
    .await;
}

#[tokio::test]
async fn a_legacy_hash_stays_until_a_sign_in_succeeds() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let legacy = legacy_hash(PASSWORD, 4, Version::TwoY);
        let user = account(db, "regi@example.test", &legacy, |_| {}).await;
        let inactive = account(db, "inaktiv@example.test", &legacy, |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;

        // A wrong password, and the right one of an inactive account.
        for (email, password, refused) in [
            ("regi@example.test", "rossz jelszó 2026", &user),
            ("inaktiv@example.test", PASSWORD, &inactive),
        ] {
            assert_eq!(
                Model::authenticate(db, email, password, true)
                    .await
                    .expect("query"),
                SignInOutcome::Refused {
                    account_id: refused.id
                },
                "{email}"
            );
        }

        // Accepted, but the account's sessions end (as a new password or a
        // deactivation ends them) before the sign-in is recorded:
        // `record_sign_in` matches nothing, so the old password's rehash is
        // never stored over whatever replaced it (ADR 0006 §2).
        let sign_in = accepted(db, "regi@example.test", PASSWORD, true).await;
        assert!(sign_in.rehashed_password.is_some());
        user.clone()
            .into_active_model()
            .revoke_sessions(db)
            .await
            .expect("revoke");
        assert_eq!(record(db, sign_in).await, None);

        for account in [&user, &inactive] {
            assert_eq!(
                stored(db, account).await.password_hash.as_deref(),
                Some(legacy.as_str()),
                "{:?}",
                account.email
            );
        }
        assert_eq!(actions(db).await, Vec::<String>::new());
        assert_eq!(revision_count(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_short_legacy_password_opens_no_session_while_the_setting_is_on() {
    with_app(|_router, ctx| async move {
        let db = &ctx.db;
        let legacy = legacy_hash(SHORT_PASSWORD, 4, Version::TwoY);
        let user = account(db, "rovid@example.test", &legacy, |_| {}).await;

        // On, as in every committed configuration: until the password
        // change session exists, the sign-in is refused like a wrong
        // password (ADR 0006 §6; research: DECISIONS R15 (D74)), and the
        // bcrypt hash stays, so the rule still sees a legacy password next
        // time.
        assert_eq!(
            Model::authenticate(db, "rovid@example.test", SHORT_PASSWORD, true)
                .await
                .expect("query"),
            SignInOutcome::Refused {
                account_id: user.id
            }
        );
        assert_eq!(
            stored(db, &user).await.password_hash.as_deref(),
            Some(legacy.as_str())
        );

        // Off: it signs in and is rehashed like any legacy password.
        let sign_in = accepted(db, "rovid@example.test", SHORT_PASSWORD, false).await;
        assert!(sign_in.rehashed_password.is_some());

        // Twelve characters are enough, with the setting on.
        let twelve = format!("{SHORT_PASSWORD}e");
        let long_enough = account(
            db,
            "eleg@example.test",
            &legacy_hash(&twelve, 4, Version::TwoY),
            |_| {},
        )
        .await;
        let sign_in = accepted(db, "eleg@example.test", &twelve, true).await;
        assert_eq!(sign_in.user.id, long_enough.id);
        assert!(sign_in.rehashed_password.is_some());
    })
    .await;
}
