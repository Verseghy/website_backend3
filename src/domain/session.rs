//! The admin session check (ADR 0006 §1, §2, §4).
//!
//! Loco's `JWTWithUser` extractor checks a session token's signature and
//! expiry and loads the account that the token's `pid` names. That is not
//! enough for an admin session. Since the token was signed, the account may
//! have been trashed, anonymised or deactivated; an account event may have
//! revoked every older token by incrementing `users.token_version`
//! (ADR 0006 §2); and a restricted token (`password_change`, `mfa`) may reach
//! only the few routes ADR 0006 §4 names. [`check_session`] decides those
//! rules from plain values: the account's state, the token's custom claims
//! and the scopes the route accepts. The admin extractor in
//! `controllers::admin::session` reads the account from the database on
//! every request, never from a cache (ADR 0016), and calls it.
//!
//! The rules need no clock: Loco has already refused an expired token. The
//! expiry arithmetic of the sliding refresh (ADR 0006 §1) joins this module
//! with the refresh, and takes "now" as an argument (ADR 0008 rule 1).
//!
//! The claim names live here, so the token that sign-in signs and the check
//! that reads it share one spelling.

use std::fmt;

use serde_json::{Map, Value};

/// The custom claim that carries `users.token_version` (ADR 0006 §2).
pub const TOKEN_VERSION_CLAIM: &str = "tv";
/// The custom claim that names what the token may do (ADR 0006 §1).
pub const SCOPE_CLAIM: &str = "scope";
/// The custom claim that holds when the password step succeeded, in seconds
/// since the Unix epoch like `exp` (ADR 0006 §1).
pub const AUTH_TIME_CLAIM: &str = "auth_time";

/// What a session token may do: the value of its [`SCOPE_CLAIM`]
/// (ADR 0006 §1, §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionScope {
    /// `admin`: a full session, which every admin route accepts.
    Admin,
    /// `password_change`: the account must set a new password first. Only
    /// `GET /api/admin/auth/me`, `POST /api/admin/auth/change-password` and
    /// `POST /api/admin/auth/logout` accept it.
    PasswordChange,
    /// `mfa`: the password step passed and the two-factor step is due. Only
    /// `POST /api/admin/auth/totp` accepts it. That route reads its own
    /// cookie without the admin extractor (ADR 0006 §4), so no scope set of
    /// the admin extractor holds `mfa`. Its rules (`scope = mfa`, the current
    /// `tv`, a live and active account) are [`check_session`] with
    /// `&[SessionScope::Mfa]` as the accepted scopes.
    Mfa,
}

impl SessionScope {
    /// Every scope, in the order of ADR 0006 §1.
    pub const ALL: [Self; 3] = [Self::Admin, Self::PasswordChange, Self::Mfa];

    /// The scope's spelling in the [`SCOPE_CLAIM`].
    #[must_use]
    pub const fn claim(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::PasswordChange => "password_change",
            Self::Mfa => "mfa",
        }
    }

    /// The scope whose claim spelling is exactly `value`, or `None`.
    ///
    /// - `value`: the text of a token's [`SCOPE_CLAIM`].
    ///
    /// The comparison is exact: `Admin` or `admin ` names no scope. backend3
    /// signs only the three spellings, so any other value comes from a token
    /// that backend3 did not issue as it stands.
    #[must_use]
    pub fn from_claim(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.claim() == value)
    }

    /// Whether `scopes` holds this scope.
    ///
    /// - `scopes`: a set of accepted scopes, such as a route's.
    ///
    /// A `const fn`, unlike `<[_]>::contains`, whose `PartialEq` is not
    /// `const`: the admin extractor evaluates it at compile time to refuse a
    /// scope set that holds [`SessionScope::Mfa`].
    #[must_use]
    pub const fn is_in(self, scopes: &[Self]) -> bool {
        let mut index = 0;
        while index < scopes.len() {
            // `as u8` compares the variants: `==` on the enum is not
            // available in a `const fn`.
            if scopes[index] as u8 == self as u8 {
                return true;
            }
            index += 1;
        }
        false
    }
}

impl fmt::Display for SessionScope {
    /// Writes the claim spelling, for log lines and error messages.
    ///
    /// - `formatter`: where to write.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.claim())
    }
}

/// The columns of an account's `users` row that decide whether its sessions
/// still hold (ADR 0006 §1, §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountState {
    /// `deleted_at` is set: the account is in the trash (ADR 0011).
    pub trashed: bool,
    /// `anonymised_at` is set: permanent delete anonymised the account
    /// (ADR 0006 §7, ADR 0011 §6).
    pub anonymised: bool,
    /// `active`: false once an `edit users` holder deactivated the account.
    pub active: bool,
    /// `token_version`: a token that carries another value was revoked
    /// (ADR 0006 §2).
    pub token_version: i32,
}

/// Why a session token opens no admin session.
///
/// Every refusal answers the same `401 unauthorized`, so a client learns
/// nothing about the account from it; the reason is for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionRefusal {
    /// The account is anonymised.
    #[error("the account is anonymised")]
    Anonymised,
    /// The account is in the trash.
    #[error("the account is in the trash")]
    Trashed,
    /// The account is deactivated.
    #[error("the account is deactivated")]
    Inactive,
    /// The token has no [`TOKEN_VERSION_CLAIM`], or one that is not a JSON
    /// integer in the range of `i64`.
    #[error("the token has no `tv` claim that is an integer")]
    InvalidTokenVersion,
    /// The token's version is not the account's: an account event revoked
    /// the token (ADR 0006 §2).
    #[error(
        "the token's version {token} is not the account's version {stored}: an account event \
         revoked it"
    )]
    StaleTokenVersion {
        /// The token's [`TOKEN_VERSION_CLAIM`].
        token: i64,
        /// The account's `token_version`.
        stored: i32,
    },
    /// The token has no [`SCOPE_CLAIM`], or one that is not the spelling of
    /// a [`SessionScope`].
    #[error("the token has no `scope` claim that names a scope")]
    InvalidScope,
    /// The token's scope is not one the route accepts (ADR 0006 §4).
    #[error("the route does not accept a `{scope}` token")]
    ScopeNotAccepted {
        /// The token's scope.
        scope: SessionScope,
    },
}

/// Decides whether a validated session token opens an admin session, and
/// returns its scope (ADR 0006 §1).
///
/// - `account`: the state of the account that the token's `pid` names, read
///   from the database for this request.
/// - `claims`: the token's custom claims, everything except Loco's `pid` and
///   `exp`, which Loco has already checked.
/// - `accepted`: the scopes the route accepts.
///
/// The rules, in the order they are checked: the account is not anonymised,
/// not in the trash and active; the [`TOKEN_VERSION_CLAIM`] is a JSON integer
/// equal to the account's `token_version`; the [`SCOPE_CLAIM`] is the exact
/// spelling of a [`SessionScope`] that `accepted` holds. The order decides
/// only which reason the error names: every refusal gets the same answer.
/// Claims other than these two never change the outcome.
///
/// # Errors
///
/// The [`SessionRefusal`] of the first rule that fails.
pub fn check_session(
    account: AccountState,
    claims: &Map<String, Value>,
    accepted: &[SessionScope],
) -> Result<SessionScope, SessionRefusal> {
    if account.anonymised {
        return Err(SessionRefusal::Anonymised);
    }
    if account.trashed {
        return Err(SessionRefusal::Trashed);
    }
    if !account.active {
        return Err(SessionRefusal::Inactive);
    }
    // `as_i64` reads only a JSON integer: `0.0`, `"0"` and `true` are not a
    // version. backend3 signs the column's value, an `i32`, as an integer.
    let token = claims
        .get(TOKEN_VERSION_CLAIM)
        .and_then(Value::as_i64)
        .ok_or(SessionRefusal::InvalidTokenVersion)?;
    if token != i64::from(account.token_version) {
        return Err(SessionRefusal::StaleTokenVersion {
            token,
            stored: account.token_version,
        });
    }
    let scope = claims
        .get(SCOPE_CLAIM)
        .and_then(Value::as_str)
        .and_then(SessionScope::from_claim)
        .ok_or(SessionRefusal::InvalidScope)?;
    if !scope.is_in(accepted) {
        return Err(SessionRefusal::ScopeNotAccepted { scope });
    }
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use proptest::{prelude::*, sample::subsequence};
    use serde_json::json;

    use super::*;

    /// A live, active account at `token_version`.
    fn live(token_version: i32) -> AccountState {
        AccountState {
            trashed: false,
            anonymised: false,
            active: true,
            token_version,
        }
    }

    /// The custom claims of a token: `tv` and `scope` when given, and the
    /// `auth_time` that every token sign-in issues carries.
    fn claims(tv: Option<Value>, scope: Option<Value>) -> Map<String, Value> {
        let mut claims = Map::new();
        if let Some(tv) = tv {
            claims.insert(TOKEN_VERSION_CLAIM.to_owned(), tv);
        }
        if let Some(scope) = scope {
            claims.insert(SCOPE_CLAIM.to_owned(), scope);
        }
        claims.insert(AUTH_TIME_CLAIM.to_owned(), json!(1_790_000_000));
        claims
    }

    /// The claims of a token at version `tv` with `scope`.
    fn token(tv: i32, scope: SessionScope) -> Map<String, Value> {
        claims(Some(json!(tv)), Some(json!(scope.claim())))
    }

    const ADMIN_ONLY: &[SessionScope] = &[SessionScope::Admin];
    const ADMIN_OR_PASSWORD_CHANGE: &[SessionScope] =
        &[SessionScope::Admin, SessionScope::PasswordChange];

    #[test]
    fn the_scope_spellings_are_the_adr_s() {
        assert_eq!(
            SessionScope::ALL.map(SessionScope::claim),
            ["admin", "password_change", "mfa"]
        );
        for scope in SessionScope::ALL {
            assert_eq!(SessionScope::from_claim(scope.claim()), Some(scope));
            assert_eq!(scope.to_string(), scope.claim());
        }
        for other in [
            "",
            "Admin",
            "ADMIN",
            " admin",
            "admin ",
            "password-change",
            "root",
        ] {
            assert_eq!(SessionScope::from_claim(other), None, "{other:?}");
        }
    }

    #[test]
    fn is_in_finds_a_scope_in_a_set() {
        assert!(SessionScope::Admin.is_in(ADMIN_ONLY));
        assert!(SessionScope::PasswordChange.is_in(ADMIN_OR_PASSWORD_CHANGE));
        assert!(!SessionScope::PasswordChange.is_in(ADMIN_ONLY));
        assert!(!SessionScope::Mfa.is_in(ADMIN_OR_PASSWORD_CHANGE));
        assert!(SessionScope::Mfa.is_in(&SessionScope::ALL));
        for scope in SessionScope::ALL {
            assert!(!scope.is_in(&[]), "{scope}");
        }
        // It is evaluated at compile time, as the extractor evaluates it.
        const { assert!(!SessionScope::Mfa.is_in(ADMIN_ONLY)) };
        const { assert!(SessionScope::Admin.is_in(ADMIN_OR_PASSWORD_CHANGE)) };
    }

    #[test]
    fn a_live_account_with_the_current_version_and_an_accepted_scope_passes() {
        assert_eq!(
            check_session(live(0), &token(0, SessionScope::Admin), ADMIN_ONLY),
            Ok(SessionScope::Admin)
        );
        assert_eq!(
            check_session(
                live(7),
                &token(7, SessionScope::PasswordChange),
                ADMIN_OR_PASSWORD_CHANGE
            ),
            Ok(SessionScope::PasswordChange)
        );
        // Other claims are not looked at.
        let mut extra = token(i32::MAX, SessionScope::Admin);
        extra.insert("role".to_owned(), json!("superuser"));
        assert_eq!(
            check_session(live(i32::MAX), &extra, ADMIN_ONLY),
            Ok(SessionScope::Admin)
        );
    }

    #[test]
    fn an_anonymised_trashed_or_inactive_account_is_refused() {
        let admin = token(0, SessionScope::Admin);
        let anonymised = AccountState {
            trashed: true,
            anonymised: true,
            active: false,
            ..live(0)
        };
        let trashed = AccountState {
            trashed: true,
            ..live(0)
        };
        let inactive = AccountState {
            active: false,
            ..live(0)
        };
        let trashed_and_inactive = AccountState {
            trashed: true,
            active: false,
            ..live(0)
        };
        for (account, refusal) in [
            (anonymised, SessionRefusal::Anonymised),
            (trashed, SessionRefusal::Trashed),
            (inactive, SessionRefusal::Inactive),
            (trashed_and_inactive, SessionRefusal::Trashed),
        ] {
            assert_eq!(
                check_session(account, &admin, ADMIN_ONLY),
                Err(refusal),
                "{account:?}"
            );
        }
    }

    #[test]
    fn a_token_of_another_version_is_refused_as_stale() {
        // An older version is a revoked token; a newer one was never issued
        // for this row, and is refused just the same.
        for (token_version, stored) in [(0, 1), (2, 1), (-1, 0), (i64::MAX, i32::MAX)] {
            assert_eq!(
                check_session(
                    live(stored),
                    &claims(Some(json!(token_version)), Some(json!("admin"))),
                    ADMIN_ONLY
                ),
                Err(SessionRefusal::StaleTokenVersion {
                    token: token_version,
                    stored
                }),
                "{token_version} against {stored}"
            );
        }
    }

    #[test]
    fn a_token_version_that_is_not_an_integer_is_refused() {
        for tv in [
            None,
            Some(Value::Null),
            Some(json!("0")),
            Some(json!(0.0)),
            Some(json!(true)),
            Some(json!([0])),
            Some(json!({ "tv": 0 })),
            // Above `i64::MAX`, so no `i32` column can hold it.
            Some(json!(u64::MAX)),
        ] {
            assert_eq!(
                check_session(
                    live(0),
                    &claims(tv.clone(), Some(json!("admin"))),
                    ADMIN_ONLY
                ),
                Err(SessionRefusal::InvalidTokenVersion),
                "{tv:?}"
            );
        }
    }

    #[test]
    fn a_scope_that_names_no_scope_is_refused() {
        for scope in [
            None,
            Some(Value::Null),
            Some(json!("")),
            Some(json!("Admin")),
            Some(json!("root")),
            Some(json!(0)),
            Some(json!(["admin"])),
        ] {
            assert_eq!(
                check_session(live(0), &claims(Some(json!(0)), scope.clone()), ADMIN_ONLY),
                Err(SessionRefusal::InvalidScope),
                "{scope:?}"
            );
        }
    }

    #[test]
    fn a_scope_the_route_does_not_accept_is_refused() {
        for (scope, accepted) in [
            (SessionScope::PasswordChange, ADMIN_ONLY),
            (SessionScope::Mfa, ADMIN_ONLY),
            (SessionScope::Mfa, ADMIN_OR_PASSWORD_CHANGE),
            (SessionScope::Admin, &[][..]),
            (SessionScope::Admin, &[SessionScope::PasswordChange][..]),
        ] {
            assert_eq!(
                check_session(live(0), &token(0, scope), accepted),
                Err(SessionRefusal::ScopeNotAccepted { scope }),
                "{scope} on {accepted:?}"
            );
        }
    }

    #[test]
    fn the_account_is_checked_before_the_token() {
        // An account refusal wins over every claim refusal, and the version
        // over the scope, as the documentation orders them.
        let broken = claims(Some(json!("x")), Some(json!("root")));
        let trashed = AccountState {
            trashed: true,
            ..live(0)
        };
        assert_eq!(
            check_session(trashed, &broken, ADMIN_ONLY),
            Err(SessionRefusal::Trashed)
        );
        assert_eq!(
            check_session(live(0), &broken, ADMIN_ONLY),
            Err(SessionRefusal::InvalidTokenVersion)
        );
        assert_eq!(
            check_session(live(1), &token(0, SessionScope::Mfa), ADMIN_ONLY),
            Err(SessionRefusal::StaleTokenVersion {
                token: 0,
                stored: 1
            })
        );
    }

    #[test]
    fn refusals_say_what_failed() {
        assert_eq!(
            SessionRefusal::StaleTokenVersion {
                token: 2,
                stored: 3
            }
            .to_string(),
            "the token's version 2 is not the account's version 3: an account event revoked it"
        );
        assert_eq!(
            SessionRefusal::ScopeNotAccepted {
                scope: SessionScope::PasswordChange
            }
            .to_string(),
            "the route does not accept a `password_change` token"
        );
        for refusal in [
            SessionRefusal::Anonymised,
            SessionRefusal::Trashed,
            SessionRefusal::Inactive,
            SessionRefusal::InvalidTokenVersion,
            SessionRefusal::InvalidScope,
        ] {
            assert!(refusal.to_string().starts_with("the "), "{refusal}");
        }
    }

    /// Any account state.
    fn account() -> impl Strategy<Value = AccountState> {
        (any::<bool>(), any::<bool>(), any::<bool>(), any::<i32>()).prop_map(
            |(trashed, anonymised, active, token_version)| AccountState {
                trashed,
                anonymised,
                active,
                token_version,
            },
        )
    }

    /// A `tv` claim for an account at `stored`: often the right one, else
    /// another integer, the digits of an integer as text (the right one
    /// too, which must still be refused), a value of another JSON type, or
    /// none.
    fn token_version(stored: i32) -> impl Strategy<Value = Option<Value>> {
        prop_oneof![
            4 => Just(Some(json!(stored))),
            1 => Just(Some(json!(stored.to_string()))),
            1 => any::<i32>().prop_map(|other| Some(json!(other.to_string()))),
            1 => any::<i32>().prop_map(|other| Some(json!(other))),
            1 => any::<i64>().prop_map(|other| Some(json!(other))),
            1 => any::<u64>().prop_map(|other| Some(json!(other))),
            1 => any::<f64>().prop_map(|other| Some(json!(other))),
            1 => "\\PC{0,8}".prop_map(|other| Some(json!(other))),
            1 => any::<bool>().prop_map(|other| Some(json!(other))),
            1 => Just(Some(Value::Null)),
            1 => Just(None),
        ]
    }

    /// A `scope` claim: often a real spelling, else any text, a value of
    /// another JSON type, or none.
    fn scope() -> impl Strategy<Value = Option<Value>> {
        prop_oneof![
            4 => proptest::sample::select(SessionScope::ALL.to_vec())
                .prop_map(|scope| Some(json!(scope.claim()))),
            1 => "\\PC{0,16}".prop_map(|other| Some(json!(other))),
            1 => any::<i64>().prop_map(|other| Some(json!(other))),
            1 => Just(Some(Value::Null)),
            1 => Just(None),
        ]
    }

    /// Claims other than `tv` and `scope`, which the check must ignore.
    fn other_claims() -> impl Strategy<Value = Map<String, Value>> {
        proptest::collection::btree_map("[a-z_]{1,10}", any::<i64>(), 0..4).prop_map(|map| {
            map.into_iter()
                .filter(|(key, _)| key != TOKEN_VERSION_CLAIM && key != SCOPE_CLAIM)
                .map(|(key, value)| (key, json!(value)))
                .collect()
        })
    }

    /// One generated case: the account, the `tv` and `scope` claims (`None`
    /// when absent), the other claims and the accepted scopes.
    type Inputs = (
        AccountState,
        Option<Value>,
        Option<Value>,
        Map<String, Value>,
        Vec<SessionScope>,
    );

    /// Everything the check takes, as an [`Inputs`] case; the `tv` claim is
    /// often the account's own version, so every rule gets both outcomes.
    fn inputs() -> impl Strategy<Value = Inputs> {
        account().prop_flat_map(|account| {
            (
                Just(account),
                token_version(account.token_version),
                scope(),
                other_claims(),
                subsequence(SessionScope::ALL.to_vec(), 0..=SessionScope::ALL.len()),
            )
        })
    }

    /// The position of `refusal`'s rule in the documented order.
    fn rank(refusal: SessionRefusal) -> usize {
        match refusal {
            SessionRefusal::Anonymised => 0,
            SessionRefusal::Trashed => 1,
            SessionRefusal::Inactive => 2,
            SessionRefusal::InvalidTokenVersion => 3,
            SessionRefusal::StaleTokenVersion { .. } => 4,
            SessionRefusal::InvalidScope => 5,
            SessionRefusal::ScopeNotAccepted { .. } => 6,
        }
    }

    proptest! {
        #[test]
        fn from_claim_accepts_exactly_the_three_spellings(value in "\\PC{0,20}") {
            let expected = SessionScope::ALL.into_iter().find(|scope| scope.claim() == value);
            prop_assert_eq!(SessionScope::from_claim(&value), expected);
        }

        /// `is_in` agrees with the slice's own `contains`, for any scope
        /// and any list of scopes, repeats included.
        #[test]
        fn is_in_agrees_with_contains(
            scope in proptest::sample::select(SessionScope::ALL.to_vec()),
            scopes in proptest::collection::vec(
                proptest::sample::select(SessionScope::ALL.to_vec()),
                0..6,
            ),
        ) {
            prop_assert_eq!(scope.is_in(&scopes), scopes.contains(&scope));
        }

        /// The check passes exactly when every rule holds, returns the
        /// token's scope when it does, and otherwise names the first rule
        /// that fails, after which every earlier rule held.
        #[test]
        fn the_check_follows_the_rules_in_order(
            (account, tv, scope, extra, accepted) in inputs(),
        ) {
            let mut all_claims = extra;
            if let Some(tv) = tv.clone() {
                all_claims.insert(TOKEN_VERSION_CLAIM.to_owned(), tv);
            }
            if let Some(scope) = scope.clone() {
                all_claims.insert(SCOPE_CLAIM.to_owned(), scope);
            }
            let version = tv.as_ref().and_then(Value::as_i64);
            let named = scope.as_ref().and_then(Value::as_str).and_then(SessionScope::from_claim);
            // Each rule with whether it holds, in the documented order.
            let rules = [
                !account.anonymised,
                !account.trashed,
                account.active,
                version.is_some(),
                version == Some(i64::from(account.token_version)),
                named.is_some(),
                named.is_some_and(|named| accepted.contains(&named)),
            ];
            match check_session(account, &all_claims, &accepted) {
                Ok(granted) => {
                    prop_assert!(rules.iter().all(|holds| *holds), "{rules:?}");
                    prop_assert_eq!(Some(granted), named);
                }
                Err(refusal) => {
                    let failed = rank(refusal);
                    prop_assert!(!rules[failed], "{refusal:?} but its rule holds: {rules:?}");
                    prop_assert!(rules[..failed].iter().all(|holds| *holds), "{refusal:?}: {rules:?}");
                    // The refusal's data is the input's.
                    match refusal {
                        SessionRefusal::StaleTokenVersion { token, stored } => {
                            prop_assert_eq!((Some(token), stored), (version, account.token_version));
                        }
                        SessionRefusal::ScopeNotAccepted { scope } => {
                            prop_assert_eq!(Some(scope), named);
                        }
                        _ => {}
                    }
                }
            }
        }

        /// Claims other than `tv` and `scope` never change the outcome.
        #[test]
        fn other_claims_never_change_the_outcome(
            (account, tv, scope, extra, accepted) in inputs(),
        ) {
            let bare = claims(tv, scope);
            let mut with_extra = bare.clone();
            with_extra.extend(extra);
            prop_assert_eq!(
                check_session(account, &with_extra, &accepted),
                check_session(account, &bare, &accepted)
            );
        }
    }
}
