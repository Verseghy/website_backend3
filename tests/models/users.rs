//! The `users` model (ADR 0006, ADR 0011): password hashing and the cost of
//! every failed check, then, against PostgreSQL, the two finders, the `pid`
//! hook, a hash that round-trips through the table, `App::truncate`, which
//! leaves the accounts alone, sign-in and its cost, the session token, the
//! session state the admin extractor checks, the `Authenticable` impl, and
//! the stored TOTP secret, which decrypts only for its own row (ADR 0006
//! §6). Every account, password and secret is invented.

use std::time::{Duration, Instant};

use backend3::{
    app::App,
    domain::{
        password_hash::DUMMY_PASSWORD_HASH,
        session::AccountState,
        totp::TotpSecret,
        totp_secret::{self, DecryptError},
    },
    models::users::{self, ActiveModel, Model},
    settings::Settings,
};
use chrono::DateTime;
use loco_rs::{
    app::Hooks,
    auth::jwt::JWT,
    model::ModelError,
    prelude::{
        ActiveModelTrait, ActiveValue, Authenticable, DateTimeWithTimeZone, IntoActiveModel, Uuid,
    },
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde_json::json;

use crate::support::with_app;

/// The password of the hashing tests.
const PASSWORD: &str = "any password";

/// Genuine Argon2i and Argon2d hashes of [`PASSWORD`], made once with the
/// argon2 crate 0.5.3 and its default parameters; test values, not
/// credentials (allowed by ADR 0034 §2). The verifier takes the algorithm
/// from the PHC string and accepts both, so only the classifier keeps them
/// from matching.
const ARGON2I_OF_PASSWORD: &str = "$argon2i$v=19$m=19456,t=2,p=1$x+/NldSXjMwQE9EthLeHXw$mnorUHztCGA0DDlOKS5ClMQdsYGHgsrkdquPYNmOpgY";
const ARGON2D_OF_PASSWORD: &str = "$argon2d$v=19$m=19456,t=2,p=1$ZZwW4B4Ccdcy+C+keoL6xw$LkwEIQ0Fj07ob4VcTnFYtIrKfJSGZH6x6FROTACjMro";

/// The `$argon2id$v=..$m=..,t=..,p=..` head of a PHC string: algorithm,
/// version and cost, without salt and hash.
fn phc_head(phc: &str) -> Vec<&str> {
    phc.split('$').take(4).collect()
}

/// Runs `check` and returns its result with the time it took.
fn timed(check: impl FnOnce() -> bool) -> (bool, Duration) {
    let start = Instant::now();
    let result = check();
    (result, start.elapsed())
}

/// A fixed instant for the timestamp columns.
fn instant() -> DateTimeWithTimeZone {
    DateTimeWithTimeZone::parse_from_rfc3339("2026-10-25T08:00:00+00:00").expect("RFC 3339")
}

/// Inserts a live, active account with `email` and no password, after
/// `change` has adjusted it, through the model hooks.
async fn insert(
    db: &DatabaseConnection,
    email: &str,
    change: impl FnOnce(&mut ActiveModel),
) -> Model {
    let mut new = ActiveModel {
        email: ActiveValue::Set(Some(email.to_owned())),
        name: ActiveValue::Set("Próba Felhasználó".to_owned()),
        ..Default::default()
    };
    change(&mut new);
    new.insert(db).await.expect("insert the account")
}

/// Saves `change` to the stored `user`.
async fn update(
    db: &DatabaseConnection,
    user: Model,
    change: impl FnOnce(&mut ActiveModel),
) -> Model {
    let mut active = user.into_active_model();
    change(&mut active);
    active.update(db).await.expect("update the account")
}

/// Trashes and anonymises the account being saved, in the shape the CHECKs
/// require (ADR 0011 §6).
fn anonymise(active: &mut ActiveModel) {
    active.email = ActiveValue::Set(None);
    active.active = ActiveValue::Set(false);
    active.deleted_at = ActiveValue::Set(Some(instant()));
    active.anonymised_at = ActiveValue::Set(Some(instant()));
}

fn assert_not_found<T: std::fmt::Debug>(result: &Result<T, ModelError>, context: &str) {
    assert!(
        matches!(result, Err(ModelError::EntityNotFound)),
        "{context}: expected EntityNotFound, got {result:?}"
    );
}

#[test]
fn a_hash_verifies_its_own_password_only() {
    for password in ["correct horse battery", "árvíztűrő tükörfúrógép"] {
        let hashed = users::hash_password(password).expect("hash");
        assert_eq!(
            phc_head(&hashed),
            ["", "argon2id", "v=19", "m=19456,t=2,p=1"]
        );
        let again = users::hash_password(password).expect("hash");
        assert_ne!(hashed, again, "fresh salt");
        assert!(users::verify_stored_password(Some(&hashed), password));
        for wrong in ["", "correct horse batterY", "arvizturo tukorfurogep"] {
            assert!(
                !users::verify_stored_password(Some(&hashed), wrong),
                "{wrong}"
            );
        }
    }
    // Verified as Argon2id, so it fails without the dummy's work; nothing
    // stores one (the doc of verify_stored_password).
    for malformed in ["$argon2id$", "$argon2id$not-a-phc-string"] {
        assert!(!users::verify_stored_password(Some(malformed), PASSWORD));
    }
}

#[test]
fn every_failure_without_an_argon2id_hash_costs_one_real_verification() {
    // Argon2 verifies with the parameters stored in the hash, so equal heads
    // mean equal work. If this fails after a dependency bump, regenerate
    // DUMMY_PASSWORD_HASH with `hash_password`.
    let real = users::hash_password(PASSWORD).expect("hash");
    assert_eq!(phc_head(DUMMY_PASSWORD_HASH), phc_head(&real));

    // What a wrong password costs for a real account: the faster of two
    // runs, because load on the machine only ever slows a run down.
    let real_cost = (0..2)
        .map(|_| {
            let (matched, cost) =
                timed(|| users::verify_stored_password(Some(&real), "wrong password"));
            assert!(!matched);
            cost
        })
        .min()
        .expect("two runs");
    // A dummy path that skips the Argon2 work answers in microseconds against
    // milliseconds, so a tenth of the real cost catches it with a wide margin
    // for a noisy machine.
    let floor = real_cost / 10;

    let (_, dummy_cost) = timed(|| {
        users::verify_dummy_password(PASSWORD);
        false
    });
    assert!(
        dummy_cost >= floor,
        "dummy {dummy_cost:?}, real {real_cost:?}"
    );
    for stored in [
        None,
        // Legacy bcrypt: verified only by the bcrypt step (ADR 0006 §4).
        Some("$2y$10$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234"),
        Some(ARGON2I_OF_PASSWORD),
        Some(ARGON2D_OF_PASSWORD),
        // The password itself as plain text.
        Some(PASSWORD),
        Some(""),
    ] {
        let (matched, cost) = timed(|| users::verify_stored_password(stored, PASSWORD));
        assert!(!matched, "{stored:?}");
        assert!(cost >= floor, "{stored:?}: {cost:?}, real {real_cost:?}");
    }
}

#[tokio::test]
async fn a_new_account_gets_a_pid_and_the_column_defaults() {
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "uj@example.test", |_| {}).await;
        let other = insert(&ctx.db, "masik@example.test", |_| {}).await;
        assert_ne!(user.pid, Uuid::nil());
        assert_ne!(user.pid, other.pid);
        assert_eq!(
            user.password_hash, None,
            "pending until a credential is issued"
        );
        assert!(user.active && !user.must_change_password);
        assert_eq!(user.token_version, 0);
        assert_eq!((user.last_login_at, user.totp_enabled_at), (None, None));
        assert_eq!((user.anonymised_at, user.deleted_at), (None, None));

        // A pid the caller sets is kept.
        let pid = Uuid::parse_str("0b9a4f3c-2d1e-4c5b-8a79-6e5d4c3b2a19").expect("UUID");
        let chosen = insert(&ctx.db, "pid@example.test", |new| {
            new.pid = ActiveValue::Set(pid);
        })
        .await;
        assert_eq!(chosen.pid, pid);
    })
    .await;
}

#[tokio::test]
async fn saving_an_account_leaves_updated_at_alone() {
    // Account events must not move updated_at (ADR 0011 §4), so the hook does
    // not stamp it the way Loco's generated one does.
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "esemeny@example.test", |new| {
            new.updated_at = ActiveValue::Set(instant());
        })
        .await;
        let saved = update(&ctx.db, user, |active| {
            active.token_version = ActiveValue::Set(1);
            active.last_login_at = ActiveValue::Set(Some(instant()));
        })
        .await;
        assert_eq!((saved.token_version, saved.updated_at), (1, instant()));
    })
    .await;
}

#[tokio::test]
async fn find_live_by_email_ignores_case_and_finds_inactive_accounts() {
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "Kis.Anna@Example.TEST", |_| {}).await;
        // Deactivation is checked by sign-in after the password (ADR 0006 §4).
        let accented = insert(&ctx.db, "ÉVA.ŐRI@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        for (typed, id) in [
            ("Kis.Anna@Example.TEST", user.id),
            ("kis.anna@example.test", user.id),
            ("KIS.ANNA@EXAMPLE.TEST", user.id),
            // Accented letters fold under the builtin C.UTF-8 default
            // collation of the compose database, which production has too
            // (the doc of find_live_by_email).
            ("éva.őri@EXAMPLE.test", accented.id),
        ] {
            let found = Model::find_live_by_email(&ctx.db, typed).await;
            assert_eq!(found.map(|found| found.id).ok(), Some(id), "{typed}");
        }
        for typed in [
            "",
            "kis.anna",
            " kis.anna@example.test",
            "kis.anna@example.test ",
            "k%@example.test",
            "nincs@example.test",
            // PostgreSQL text cannot hold U+0000; the finder never sends it.
            "kis.anna\0@example.test",
        ] {
            assert_not_found(&Model::find_live_by_email(&ctx.db, typed).await, typed);
        }
    })
    .await;
}

#[tokio::test]
async fn find_live_by_email_skips_trashed_and_anonymised_accounts() {
    with_app(|_router, ctx| async move {
        let trashed = insert(&ctx.db, "lomtar@example.test", |_| {}).await;
        update(&ctx.db, trashed, |active| {
            active.deleted_at = ActiveValue::Set(Some(instant()));
        })
        .await;
        assert_not_found(
            &Model::find_live_by_email(&ctx.db, "lomtar@example.test").await,
            "trashed",
        );

        // The CHECKs clear an anonymised row's address and keep it trashed,
        // so this pins the outcome; the anonymised_at filter itself is
        // defence in depth that no row can reach.
        let gone = insert(&ctx.db, "anonim@example.test", |_| {}).await;
        update(&ctx.db, gone, anonymise).await;
        assert_not_found(
            &Model::find_live_by_email(&ctx.db, "anonim@example.test").await,
            "anonymised",
        );
    })
    .await;
}

#[tokio::test]
async fn find_by_pid_returns_the_account_in_any_state() {
    // The session check rejects trashed and anonymised accounts itself
    // (ADR 0006 §1), so the finder still returns them.
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "pid@example.test", |_| {}).await;
        let pid = user.pid.to_string();
        for typed in [pid.clone(), pid.to_uppercase()] {
            let found = Model::find_by_pid(&ctx.db, &typed).await;
            assert_eq!(found.map(|found| found.id).ok(), Some(user.id), "{typed}");
        }
        update(&ctx.db, user, anonymise).await;
        let found = Model::find_by_pid(&ctx.db, &pid).await.expect("anonymised");
        assert!(found.anonymised_at.is_some());

        for typed in [
            "6f1d2c3b-4a59-4e8d-9c7b-1a2b3c4d5e6f",
            "",
            "not-a-uuid",
            "6f1d2c3b-4a59-4e8d-9c7b-1a2b3c4d5e6",
            "1 OR 1=1",
        ] {
            assert_not_found(&Model::find_by_pid(&ctx.db, typed).await, typed);
        }
    })
    .await;
}

#[tokio::test]
async fn a_stored_hash_round_trips_through_the_table() {
    with_app(|_router, ctx| async move {
        let hashed = users::hash_password("hosszú tesztjelszó").expect("hash");
        insert(&ctx.db, "jelszo@example.test", |new| {
            new.password_hash = ActiveValue::Set(Some(hashed));
        })
        .await;
        let user = Model::find_live_by_email(&ctx.db, "jelszo@example.test")
            .await
            .expect("live");
        assert!(user.verify_password("hosszú tesztjelszó"));
        assert!(!user.verify_password("hosszu tesztjelszo"));

        // A pending account (no password) never verifies.
        let pending = insert(&ctx.db, "fuggo@example.test", |_| {}).await;
        assert!(!pending.verify_password(""));
    })
    .await;
}

#[tokio::test]
async fn truncate_never_empties_the_accounts() {
    // The append-only history tables reference users and reject TRUNCATE, so
    // App::truncate leaves users alone (AGENTS.md, "Commands").
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "marad@example.test", |_| {}).await;
        App::truncate(&ctx).await.expect("truncate");
        let found = Model::find_by_pid(&ctx.db, &user.pid.to_string()).await;
        assert_eq!(found.map(|found| found.id).ok(), Some(user.id));
    })
    .await;
}

#[tokio::test]
async fn every_failed_sign_in_costs_what_a_wrong_password_costs() {
    with_app(|_router, ctx| async move {
        /// How an account below differs from a live, active one.
        type Change = fn(&mut ActiveModel);
        let hashed = users::hash_password(PASSWORD).expect("hash");
        let accounts: [(&str, Change); 4] = [
            ("ido@example.test", |_| {}),
            // The accounts that may not sign in: their state is checked only
            // after their password, so they cost a verification too.
            ("inaktiv@example.test", |new| {
                new.active = ActiveValue::Set(false);
            }),
            ("ketlepcsos@example.test", |new| {
                new.totp_enabled_at = ActiveValue::Set(Some(instant()));
            }),
            ("csere@example.test", |new| {
                new.must_change_password = ActiveValue::Set(true);
            }),
        ];
        for (email, change) in accounts {
            insert(&ctx.db, email, |new| {
                new.password_hash = ActiveValue::Set(Some(hashed.clone()));
                change(new);
            })
            .await;
        }
        let cost = |email: &'static str| {
            let db = ctx.db.clone();
            async move {
                let start = Instant::now();
                let outcome = Model::authenticate(&db, email, "wrong password").await;
                assert_eq!(outcome.expect("query"), None);
                start.elapsed()
            }
        };
        // The faster of two runs each, as in the hashing test above: load
        // only slows a run down, and a skipped verification is three orders
        // of magnitude faster.
        let wrong = cost("ido@example.test")
            .await
            .min(cost("ido@example.test").await);
        for email in [
            "nincs@example.test",
            "ido\0@example.test",
            "inaktiv@example.test",
            "ketlepcsos@example.test",
            "csere@example.test",
        ] {
            let failed = cost(email).await.min(cost(email).await);
            assert!(
                failed >= wrong / 10,
                "{email:?}: {failed:?}, wrong password {wrong:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_session_token_carries_the_adr_claims() {
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "token@example.test", |new| {
            new.token_version = ActiveValue::Set(3);
        })
        .await;
        let secret = ctx
            .config
            .get_jwt_config()
            .expect("auth.jwt")
            .secret
            .clone();
        // Sub-second parts are dropped: `auth_time` is a NumericDate.
        let auth_time = DateTime::from_timestamp(1_790_000_000, 999_000_000).expect("in range");
        let token = user
            .generate_session_token(&secret, 7200, auth_time)
            .expect("signed");
        let claims = JWT::new(&secret).validate(&token).expect("valid").claims;
        assert_eq!(claims.pid, user.pid.to_string());
        assert_eq!(
            serde_json::Value::Object(claims.claims),
            json!({ "tv": 3, "scope": "admin", "auth_time": 1_790_000_000 })
        );
        assert!(matches!(
            user.generate_session_token("not base64!", 7200, auth_time),
            Err(ModelError::Jwt(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn the_session_state_is_the_stored_row_s() {
    // The columns the admin session check reads (ADR 0006 §1), straight
    // from each stored state.
    with_app(|_router, ctx| async move {
        let live = insert(&ctx.db, "elo@example.test", |new| {
            new.token_version = ActiveValue::Set(5);
        })
        .await;
        let trashed = insert(&ctx.db, "lomtar@example.test", |new| {
            new.deleted_at = ActiveValue::Set(Some(instant()));
        })
        .await;
        let inactive = insert(&ctx.db, "inaktiv@example.test", |new| {
            new.active = ActiveValue::Set(false);
        })
        .await;
        let anonymised = insert(&ctx.db, "anonim@example.test", anonymise).await;
        let state = |trashed, anonymised, active, token_version| AccountState {
            trashed,
            anonymised,
            active,
            token_version,
        };
        for (user, expected) in [
            (&live, state(false, false, true, 5)),
            (&trashed, state(true, false, true, 0)),
            (&inactive, state(false, false, false, 0)),
            (&anonymised, state(true, true, false, 0)),
        ] {
            assert_eq!(user.session_state(), expected, "{:?}", user.email);
        }
    })
    .await;
}

#[tokio::test]
async fn loco_s_extractor_finds_accounts_by_pid_and_never_by_api_key() {
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "kulcs@example.test", |_| {}).await;
        let found =
            <Model as Authenticable>::find_by_claims_key(&ctx.db, &user.pid.to_string()).await;
        assert_eq!(found.map(|found| found.id).ok(), Some(user.id));
        assert_not_found(
            &<Model as Authenticable>::find_by_claims_key(&ctx.db, "not-a-uuid").await,
            "pid",
        );
        for key in ["", "kulcs@example.test", &user.pid.to_string()] {
            assert_not_found(
                &<Model as Authenticable>::find_by_api_key(&ctx.db, key).await,
                key,
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_stored_totp_secret_decrypts_for_its_own_row_only() {
    // ADR 0006, "Follow-up work": an encrypted TOTP secret decrypts for its
    // own row, and the same bytes copied into another user's row fail to.
    with_app(|_router, ctx| async move {
        // The derived key of config/test.yaml, as a handler takes it.
        let key = Settings::from_section(ctx.config.settings.as_ref())
            .expect("settings")
            .admin_auth
            .totp_encryption_key;
        let secret = TotpSecret::from_bytes(*b"invented-totp-secret");
        let owner = insert(&ctx.db, "ketlepcso@example.test", |_| {}).await;
        let other = insert(&ctx.db, "masik@example.test", |_| {}).await;
        assert_eq!(
            other.totp_secret(&key).map(|found| found.is_some()),
            Ok(false)
        );

        // A fixed nonce; an enrolment reads 12 random bytes instead.
        let stored = totp_secret::encrypt(&key, owner.id, [7; 12], &secret);
        let owner = update(&ctx.db, owner, |active| {
            active.totp_secret_encrypted = ActiveValue::Set(Some(stored.to_vec()));
        })
        .await;
        let owner = Model::find_by_pid(&ctx.db, &owner.pid.to_string())
            .await
            .expect("owner");
        let decrypted = owner.totp_secret(&key).expect("own row").expect("stored");
        assert_eq!(decrypted.as_bytes(), secret.as_bytes());

        // Copied by a database write, not through the model.
        ctx.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE users SET totp_secret_encrypted = \
                 (SELECT totp_secret_encrypted FROM users WHERE id = $1) WHERE id = $2",
                [owner.id.into(), other.id.into()],
            ))
            .await
            .expect("copy the ciphertext");
        let other = Model::find_by_pid(&ctx.db, &other.pid.to_string())
            .await
            .expect("other");
        assert_eq!(other.totp_secret_encrypted.as_deref(), Some(&stored[..]));
        assert_eq!(
            other.totp_secret(&key).map(|found| found.is_some()),
            Err(DecryptError::Authentication)
        );
    })
    .await;
}
