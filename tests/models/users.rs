//! The `users` model (ADR 0006, ADR 0011): password hashing and the cost of
//! every failed check, then, against PostgreSQL, the two finders, the `pid`
//! hook, a hash that round-trips through the table, `App::truncate`, which
//! leaves the accounts and their roles and grants alone, sign-in and its
//! cost, the session token, the session state the admin session check reads,
//! the `Authenticable` impl, the seed account, which the seed creates once
//! with its roles, in one transaction, and never changes, the
//! stored TOTP secret, which decrypts only for its own row (ADR 0006
//! §6), and the revocation of every session, one atomic increment of
//! `token_version` that leaves `updated_at` alone (ADR 0006 §2, ADR 0011
//! §4). Every account, password and secret is invented.

use std::time::{Duration, Instant};

use backend3::{
    app::App,
    domain::{
        authz::{AccountGrants, Role},
        password_hash::DUMMY_PASSWORD_HASH,
        seed::SeedAccount,
        session::AccountState,
        totp::TotpSecret,
        totp_secret::{self, DecryptError},
    },
    models::users::{self, ActiveModel, Model, SeedOutcome},
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
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, PaginatorTrait, Statement,
};
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

/// An invented seed account for the `seed_account` tests; its password is a
/// test value, not the published seed password. It has two roles, so the
/// tests see every role written.
const SEED: SeedAccount = SeedAccount {
    name: "Mag Próba",
    email: "Mag.Proba@example.test",
    password: "egy kitalált magjelszó",
    roles: &[Role::Writer, Role::Secretary],
};

#[tokio::test]
async fn seed_account_creates_an_active_account_that_signs_in() {
    with_app(|_router, ctx| async move {
        let outcome = Model::seed_account(&ctx.db, &SEED).await.expect("seed");

        let SeedOutcome::Created(created) = outcome else {
            panic!("expected Created, got {outcome:?}");
        };
        assert_eq!(created.name, SEED.name);
        assert_eq!(created.email.as_deref(), Some(SEED.email));
        assert!(created.active);
        assert!(!created.must_change_password);
        assert_eq!(created.token_version, 0);
        assert!(created.totp_secret_encrypted.is_none());
        assert_eq!(created.created_by, None);
        // A fresh Argon2id hash, with the hasher's default cost.
        let hash = created.password_hash.as_deref().expect("hash");
        assert_eq!(phc_head(hash), ["", "argon2id", "v=19", "m=19456,t=2,p=1"]);
        let signed_in = Model::authenticate(&ctx.db, "mag.proba@EXAMPLE.test", SEED.password)
            .await
            .expect("authenticate");
        assert_eq!(signed_in.accepted().map(|user| user.id), Some(created.id));
        // Its roles, and no extra grant.
        assert_eq!(
            created.grants(&ctx.db).await.expect("grants"),
            AccountGrants {
                roles: [Role::Writer, Role::Secretary].into(),
                extra: [].into(),
            }
        );
    })
    .await;
}

/// The account and its roles are one transaction: when a role cannot be
/// written, no account is left behind without it.
#[tokio::test]
async fn seed_account_writes_nothing_when_a_role_cannot_be_written() {
    with_app(|_router, ctx| async move {
        // This test's own database only: without the secretary row, the
        // second role link fails its foreign key.
        ctx.db
            .execute_unprepared("DELETE FROM roles WHERE id = 2")
            .await
            .expect("remove the secretary row");

        let error = Model::seed_account(&ctx.db, &SEED)
            .await
            .expect_err("the secretary link is refused");

        assert!(
            error.to_string().contains("user_roles_role_id_fkey"),
            "{error}"
        );
        assert_eq!(
            users::Entity::find().count(&ctx.db).await.expect("count"),
            0,
            "the account was rolled back with its roles"
        );
        let links = ctx
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) FROM user_roles",
            ))
            .await
            .expect("count the links")
            .expect("a row")
            .try_get_by_index::<i64>(0)
            .expect("a count");
        assert_eq!(links, 0);
    })
    .await;
}

/// Seeding never touches an existing account with the address, in any case
/// spelling and in any state, the trash included: the unique index covers
/// trashed rows, so an insert would fail, and the seed must not restore,
/// reactivate or re-password anything.
#[tokio::test]
async fn seed_account_leaves_an_existing_account_with_the_address_alone() {
    with_app(|_router, ctx| async move {
        let existing = insert(&ctx.db, "mag.proba@example.test", |new| {
            new.name = ActiveValue::Set("Már Létező".to_owned());
            new.active = ActiveValue::Set(false);
        })
        .await;
        let existing = update(&ctx.db, existing, |active| {
            active.deleted_at = ActiveValue::Set(Some(instant()));
        })
        .await;

        let outcome = Model::seed_account(&ctx.db, &SEED).await.expect("seed");

        let SeedOutcome::AlreadyPresent(found) = outcome else {
            panic!("expected AlreadyPresent, got {outcome:?}");
        };
        assert_eq!(found, existing);
        let stored = Model::find_by_pid(&ctx.db, &existing.pid.to_string())
            .await
            .expect("stored");
        assert_eq!(stored, existing, "the row is unchanged");
        // Not even its roles: the seed gives that account none.
        assert_eq!(
            stored.grants(&ctx.db).await.expect("grants"),
            AccountGrants::default()
        );
        assert_eq!(
            users::Entity::find().count(&ctx.db).await.expect("count"),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn truncate_never_empties_the_accounts() {
    // The append-only history tables reference users and reject TRUNCATE, so
    // App::truncate leaves users alone (AGENTS.md, "Commands"), and with them
    // the roles and grants they hold and the four seeded roles.
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "marad@example.test", |_| {}).await;
        ctx.db
            .execute_unprepared(&format!(
                "INSERT INTO user_roles (user_id, role_id) VALUES ({0}, 1); \
                 INSERT INTO user_permissions (user_id, permission) VALUES ({0}, 'edit events')",
                user.id
            ))
            .await
            .expect("a role and a grant");
        App::truncate(&ctx).await.expect("truncate");
        let found = Model::find_by_pid(&ctx.db, &user.pid.to_string()).await;
        assert_eq!(found.map(|found| found.id).ok(), Some(user.id));
        assert_eq!(
            user.grants(&ctx.db).await.expect("grants"),
            AccountGrants::from_stored([1], ["edit events"]).expect("known values")
        );
        let roles = ctx
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT count(*) FROM roles",
            ))
            .await
            .expect("count the roles")
            .expect("a row")
            .try_get_by_index::<i64>(0)
            .expect("a count");
        assert_eq!(roles, 4);
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
                assert_eq!(outcome.expect("query").accepted(), None);
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

#[tokio::test]
async fn revoke_sessions_increments_the_version_and_changes_nothing_else() {
    // ADR 0006 §2: logout increments `token_version`; ADR 0011 §4: as an
    // account event it leaves `updated_at` and `updated_by` alone.
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "kilep@example.test", |new| {
            new.token_version = ActiveValue::Set(4);
            new.updated_at = ActiveValue::Set(instant());
        })
        .await;
        let revoked = user
            .clone()
            .into_active_model()
            .revoke_sessions(&ctx.db)
            .await
            .expect("revoke");
        assert_eq!(revoked.token_version, 5);
        // The returned row is the stored one, and it differs from the old
        // row in the version alone.
        let stored = Model::find_by_pid(&ctx.db, &user.pid.to_string())
            .await
            .expect("stored");
        assert_eq!(revoked, stored);
        assert_eq!(
            Model {
                token_version: user.token_version,
                ..revoked.clone()
            },
            user
        );

        // Each revocation moves the version by one more.
        let again = revoked
            .into_active_model()
            .revoke_sessions(&ctx.db)
            .await
            .expect("revoke again");
        assert_eq!((again.token_version, again.updated_at), (6, instant()));
    })
    .await;
}

#[tokio::test]
async fn revoke_sessions_writes_the_pending_changes_in_the_same_update() {
    // A deactivation ends the account's sessions in the statement that
    // switches it off (ADR 0006 §2, §7).
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "kikapcsol@example.test", |_| {}).await;
        let other = insert(&ctx.db, "marad@example.test", |_| {}).await;
        let mut active = user.into_active_model();
        active.active = ActiveValue::Set(false);
        let revoked = active.revoke_sessions(&ctx.db).await.expect("revoke");
        assert_eq!((revoked.active, revoked.token_version), (false, 1));
        // Only the addressed row changed.
        let untouched = Model::find_by_pid(&ctx.db, &other.pid.to_string())
            .await
            .expect("stored");
        assert_eq!(untouched, other);
    })
    .await;
}

#[tokio::test]
async fn revoke_sessions_refuses_a_model_without_id_or_with_a_version_of_its_own() {
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "hibas@example.test", |_| {}).await;
        let no_id = ActiveModel {
            name: ActiveValue::Set("Próba Felhasználó".to_owned()),
            ..Default::default()
        };
        let mut own_version = user.clone().into_active_model();
        own_version.token_version = ActiveValue::Set(9);
        for (case, active, expected) in [
            ("no id", no_id, users::RevokeSessionsError::NoId),
            (
                "a version of its own",
                own_version,
                users::RevokeSessionsError::TokenVersionSet,
            ),
        ] {
            let error = active
                .revoke_sessions(&ctx.db)
                .await
                .expect_err("refused before any query");
            // The message says what to fix.
            assert_eq!(error.to_string(), expected.to_string(), "{case}");
        }
        assert!(
            users::RevokeSessionsError::NoId
                .to_string()
                .contains("into_active_model")
        );
        assert!(
            users::RevokeSessionsError::TokenVersionSet
                .to_string()
                .contains("leave it unset")
        );
        // Nothing was written.
        let stored = Model::find_by_pid(&ctx.db, &user.pid.to_string())
            .await
            .expect("stored");
        assert_eq!(stored.token_version, 0);
    })
    .await;
}

#[tokio::test]
async fn revoke_sessions_of_a_row_that_does_not_exist_is_not_found() {
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "nincs@example.test", |_| {}).await;
        let mut gone = user.into_active_model();
        gone.id = ActiveValue::Unchanged(i64::MAX);
        assert_not_found(&gone.revoke_sessions(&ctx.db).await, "an unknown id");
    })
    .await;
}

#[tokio::test]
async fn concurrent_revocations_never_lose_an_increment() {
    // One `UPDATE ... SET token_version = token_version + 1` per call, not a
    // read and a write: with eight calls in flight from the same stale row
    // (the test pool's two connections run two of them at a time), the
    // version still moves eight times, and the eight returned versions are
    // 1 to 8. A read-then-write would return a repeated version here.
    with_app(|_router, ctx| async move {
        let user = insert(&ctx.db, "parhuzamos@example.test", |_| {}).await;
        let calls = (0..8).map(|_| {
            let active = user.clone().into_active_model();
            let db = ctx.db.clone();
            async move { active.revoke_sessions(&db).await }
        });
        let mut versions: Vec<i32> = futures_util::future::join_all(calls)
            .await
            .into_iter()
            .map(|revoked| revoked.expect("revoke").token_version)
            .collect();
        versions.sort_unstable();
        assert_eq!(versions, (1..=8).collect::<Vec<_>>());
        let stored = Model::find_by_pid(&ctx.db, &user.pid.to_string())
            .await
            .expect("stored");
        assert_eq!(stored.token_version, 8);
    })
    .await;
}
