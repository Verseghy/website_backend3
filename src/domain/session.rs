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
//! and the scopes the route accepts. `open_session` in
//! `controllers::admin::session`, which the admin route table's guard calls
//! before every handler that needs a session, reads the account from the
//! database on every request, never from a cache (ADR 0016), and calls it.
//!
//! The rules need no clock: Loco has already refused an expired token.
//!
//! The expiry arithmetic of the sliding refresh (ADR 0006 §1, §8) is here
//! too, and takes "now" as an argument (ADR 0008 rule 1):
//!
//! - a refresh at "now" gives the session a token that expires two hours
//!   later ([`SESSION_LIFETIME_SECS`]), but never later than its
//!   `auth_time` plus the absolute limit, `admin_auth.session_absolute_max_secs`
//!   ([`AbsoluteLimit`], 12 hours by default). Past that the session cannot
//!   be refreshed, and the user signs in again ([`renewal`]);
//! - the passive refresh renews a full `admin` session that the session
//!   cookie carries when less than [`PASSIVE_REFRESH_THRESHOLD_SECS`]
//!   (3,600 s) of its token remain, and only when the renewal ends later
//!   than the token it replaces ([`passive_renewal`]);
//! - the explicit refresh, `POST /api/admin/auth/refresh`, renews such a
//!   session whatever is left ([`explicit_renewal`]). The admin UI calls it
//!   on typing or clicking at most every 10 minutes, so the session ends
//!   between 1 h 50 min and 2 h after the last activity, or at the absolute
//!   limit (ADR 0006 §1, "Idle result"; the property tests below model it).
//!
//! A Bearer token is never refreshed (ADR 0006 §3, "It is not refreshed, so
//! scripts sign in again"): both renewals answer `None` for one.
//!
//! The arithmetic is in whole seconds since the Unix epoch, the JWT
//! `NumericDate` of `exp` and `auth_time`, and counts the two hours from
//! "now" rounded down to its second, as sign-in's `auth_time` is. Loco's
//! `JWT::generate_token` adds the lifetime it is given to the system time,
//! which it reads again a moment after the app clock, and keeps no instant
//! of its own (loco-rs 1.2.0 `src/auth/jwt.rs`). So a [`Renewal`] carries
//! both the `exp` it aims at and the lifetime that the caller hands Loco and
//! writes as the cookie's `Max-Age`, and when "now" has a fraction of a
//! second, Loco's second may already be the next one. The absolute end
//! therefore counts the time left from "now" rounded up, so that even the
//! next second's `exp` is never past it ([`renewal`]); recipe 7 step 4 of
//! the project skill says how the two clocks meet.
//!
//! The claim names live here, so the token that sign-in signs and the check
//! that reads it share one spelling.

use std::fmt;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use super::session_jwt::SESSION_LIFETIME_SECS;

/// The custom claim that carries `users.token_version` (ADR 0006 §2).
pub const TOKEN_VERSION_CLAIM: &str = "tv";
/// The custom claim that names what the token may do (ADR 0006 §1).
pub const SCOPE_CLAIM: &str = "scope";
/// The custom claim that holds when the password step succeeded, in seconds
/// since the Unix epoch like `exp` (ADR 0006 §1).
pub const AUTH_TIME_CLAIM: &str = "auth_time";
/// Loco's claim that holds when the token expires, in seconds since the Unix
/// epoch (loco-rs 1.2.0 `UserClaims`). Loco checks it; the sliding refresh
/// reads it ([`SessionTimes::from_claims`]).
pub const EXPIRY_CLAIM: &str = "exp";

/// The passive refresh's threshold in seconds (ADR 0006 §1): a request whose
/// full `admin` session has less than this left gets a new cookie. It
/// applies only to that path; the explicit refresh renews whatever is left.
pub const PASSIVE_REFRESH_THRESHOLD_SECS: u64 = 3600;

/// [`SESSION_LIFETIME_SECS`] as the signed seconds the arithmetic uses; the
/// conversion of a constant, checked by the compiler.
#[expect(
    clippy::cast_possible_wrap,
    reason = "the assertion above the cast proves that 7,200 fits; `i64::try_from` is not const"
)]
const LIFETIME_SECS: i64 = {
    assert!(SESSION_LIFETIME_SECS <= i64::MAX as u64);
    SESSION_LIFETIME_SECS as i64
};

/// The nanoseconds of one second, for the exact threshold comparison of
/// [`passive_refresh_due`].
const NANOS_PER_SEC: i128 = 1_000_000_000;

/// The nanoseconds of one second as chrono counts a second's fraction, for
/// rounding "now" up in [`renewal`].
const NANOS_PER_SEC_U32: u32 = 1_000_000_000;

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
    /// cookie without the admin route table's session guard (ADR 0006 §4),
    /// so no scope set of an admin route holds `mfa`. Its rules (`scope = mfa`, the current
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
    /// `const`: `accepted_scopes` in `controllers::admin::session`, through
    /// which the route table and the session extractor read a route's
    /// scopes, evaluates it at compile time to refuse a scope set that holds
    /// [`SessionScope::Mfa`].
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

/// Where the request carried its session token (ADR 0006 §1, §3).
///
/// `auth.jwt.location` reads the session cookie first, then the Bearer
/// header, and nothing else (checked at boot, `session_jwt`), so a request
/// that sends the session cookie is authenticated by it, whatever else it
/// sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenCarrier {
    /// The `__Host-admin_session` cookie: a browser session, which the
    /// sliding refresh renews.
    Cookie,
    /// The `Authorization: Bearer` header: a script, whose token is never
    /// refreshed (ADR 0006 §3).
    Bearer,
}

impl TokenCarrier {
    /// Both carriers.
    pub const ALL: [Self; 2] = [Self::Cookie, Self::Bearer];
}

/// `settings.admin_auth.session_absolute_max_secs`, checked: how long after
/// its `auth_time` a session may last at most, however often it is
/// refreshed (ADR 0006 §1, §8). Never shorter than the session lifetime,
/// [`SESSION_LIFETIME_SECS`], or a fresh sign-in would already outlive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbsoluteLimit(u64);

/// The configured absolute limit is shorter than the session lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "is {found} seconds; the absolute session limit must be at least {SESSION_LIFETIME_SECS} \
     seconds, the two-hour session lifetime, or a fresh sign-in would outlive it (ADR 0006 §8; \
     the default is 43200, 12 hours)"
)]
pub struct AbsoluteLimitError {
    /// The configured value, in seconds.
    pub found: u64,
}

impl AbsoluteLimit {
    /// The default of ADR 0006 §8, 43,200 s (12 hours), which the maintainer
    /// kept (research: DECISIONS R9, D07). Every committed configuration
    /// writes it out; this constant is for the tests and the documentation.
    pub const DEFAULT_SECS: u64 = 43_200;

    /// Checks a configured absolute limit.
    ///
    /// - `secs`: the configured `session_absolute_max_secs`.
    ///
    /// # Errors
    ///
    /// [`AbsoluteLimitError`] when `secs` is below [`SESSION_LIFETIME_SECS`],
    /// zero included. Any longer limit is accepted: the arithmetic saturates,
    /// so even `u64::MAX` only means that the limit never binds.
    pub const fn new(secs: u64) -> Result<Self, AbsoluteLimitError> {
        if secs < SESSION_LIFETIME_SECS {
            Err(AbsoluteLimitError { found: secs })
        } else {
            Ok(Self(secs))
        }
    }

    /// The limit in seconds.
    #[must_use]
    pub const fn secs(self) -> u64 {
        self.0
    }

    /// When a session that signed in at `auth_time` must end, in seconds
    /// since the Unix epoch: `auth_time` plus the limit, saturating at the
    /// end of `i64`.
    ///
    /// - `auth_time`: the session's [`AUTH_TIME_CLAIM`].
    #[must_use]
    pub fn session_end(self, auth_time: i64) -> i64 {
        auth_time.saturating_add(i64::try_from(self.0).unwrap_or(i64::MAX))
    }
}

/// The two instants of a session token that the sliding refresh reads, in
/// seconds since the Unix epoch (JWT `NumericDate`s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionTimes {
    /// When the password step succeeded: the [`AUTH_TIME_CLAIM`]. A refresh
    /// keeps it, so the absolute limit counts from the sign-in.
    pub auth_time: i64,
    /// When the token expires: the [`EXPIRY_CLAIM`].
    pub expires_at: i64,
}

/// Why a token's claims hold no [`SessionTimes`]. Every token backend3 signs
/// carries both claims, so either is a defect, never a client's mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionTimesError {
    /// The [`AUTH_TIME_CLAIM`] is missing or not a JSON integer in the range
    /// of `i64`.
    #[error("the token has no `auth_time` claim that is an integer")]
    InvalidAuthTime,
    /// The [`EXPIRY_CLAIM`] is missing or not a JSON integer in the range of
    /// `i64`.
    #[error("the token has no `exp` claim that is an integer")]
    InvalidExpiry,
}

impl SessionTimes {
    /// Reads the two instants from a token's claims.
    ///
    /// - `claims`: every claim of the token as one JSON object, Loco's
    ///   `exp` included: loco-rs 1.2.0 keeps `UserClaims::exp` private, so
    ///   the caller serialises the validated `UserClaims`, which writes `pid`,
    ///   `exp` and the custom claims side by side.
    ///
    /// # Errors
    ///
    /// [`SessionTimesError::InvalidAuthTime`] or
    /// [`SessionTimesError::InvalidExpiry`], in that order, for a claim that
    /// is missing or is not a JSON integer that fits `i64` (`"1"`, `1.0` and
    /// `true` are not instants).
    pub fn from_claims(claims: &Map<String, Value>) -> Result<Self, SessionTimesError> {
        let auth_time = claims
            .get(AUTH_TIME_CLAIM)
            .and_then(Value::as_i64)
            .ok_or(SessionTimesError::InvalidAuthTime)?;
        let expires_at = claims
            .get(EXPIRY_CLAIM)
            .and_then(Value::as_i64)
            .ok_or(SessionTimesError::InvalidExpiry)?;
        Ok(Self {
            auth_time,
            expires_at,
        })
    }
}

/// A refreshed session token's expiry: what a refresh asks Loco for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Renewal {
    /// When the refreshed token expires, in seconds since the Unix epoch:
    /// "now" rounded down to its second plus [`Renewal::lifetime_secs`], the
    /// `exp` that Loco writes when it reads the system clock within "now"'s
    /// second. When that second has passed by the time Loco reads it, Loco
    /// writes one second later; [`renewal`] keeps even that `exp` within the
    /// absolute end.
    pub expires_at: i64,
    /// The lifetime that the caller hands Loco's `JWT::generate_token` and
    /// writes as the cookie's `Max-Age`, in seconds. From 1 to
    /// [`SESSION_LIFETIME_SECS`].
    pub lifetime_secs: u64,
}

/// The expiry that a refresh at `now` gives a session that signed in at
/// `auth_time` (ADR 0006 §1): two hours after `now`, or the session's
/// absolute end if that comes first.
///
/// - `now`: the app clock's instant of the request. The two hours count
///   from its whole second, rounded down, as sign-in's `auth_time` does, so
///   no renewal ends more than two hours after the request. The time left
///   before the absolute end counts from its second rounded up, because
///   Loco reads the system clock again after it (the module documentation
///   says why).
/// - `auth_time`: the session's [`AUTH_TIME_CLAIM`].
/// - `limit`: the absolute limit.
///
/// The lifetime is therefore two hours, or the whole seconds from `now`
/// rounded up to the absolute end, whichever is shorter. Loco adds it to its
/// own reading of the system clock: as long as that reading comes less than
/// a second after `now` (microseconds in practice), the `exp` it writes is
/// never later than `auth_time` plus the limit. A capped renewal may end up
/// to one second before the end instead, which ADR 0006 §1 allows ("never
/// later than").
///
/// Returns `None` when the absolute end leaves not even one second after
/// `now` rounded up: the session cannot be refreshed any more, and the user
/// signs in again once its token expires.
#[must_use]
pub fn renewal(now: DateTime<Utc>, auth_time: i64, limit: AbsoluteLimit) -> Option<Renewal> {
    let start = now.timestamp();
    // `now` rounded up to its second: the latest second that Loco's own
    // reading of the system clock can fall in, if it comes within a second.
    // `div_ceil`, because chrono marks a leap second by a nanosecond field
    // of one second or more, which rounds up to the second after it.
    let latest_start = start.saturating_add(i64::from(
        now.timestamp_subsec_nanos().div_ceil(NANOS_PER_SEC_U32),
    ));
    // `min` keeps the cap: nothing expires after `auth_time` plus the limit.
    // `saturating_sub`: an `auth_time` near either end of `i64` would
    // overflow, and saturating keeps the result's side (far past the end,
    // or the end out of reach).
    let lifetime = limit
        .session_end(auth_time)
        .saturating_sub(latest_start)
        .min(LIFETIME_SECS);
    let lifetime_secs = u64::try_from(lifetime).ok().filter(|secs| *secs >= 1)?;
    Some(Renewal {
        expires_at: start.saturating_add(lifetime),
        lifetime_secs,
    })
}

/// Whether less than [`PASSIVE_REFRESH_THRESHOLD_SECS`] of a token are left
/// at `now`: the passive refresh's trigger (ADR 0006 §1).
///
/// - `now`: the app clock's instant of the request, with its fraction of a
///   second.
/// - `expires_at`: the token's [`EXPIRY_CLAIM`].
///
/// The comparison is exact, in nanoseconds: 3,600 s left is not due, a
/// nanosecond less is. A token that has expired already (which Loco refuses
/// before this is asked) counts as due.
#[must_use]
pub fn passive_refresh_due(now: DateTime<Utc>, expires_at: i64) -> bool {
    // `i128`: no `exp` and no instant of chrono's range overflows it. The
    // fraction can exceed one second only on a leap second, which then
    // simply counts as later.
    let remaining = (i128::from(expires_at) - i128::from(now.timestamp())) * NANOS_PER_SEC
        - i128::from(now.timestamp_subsec_nanos());
    remaining < i128::from(PASSIVE_REFRESH_THRESHOLD_SECS) * NANOS_PER_SEC
}

/// The passive refresh of a request's session, or `None` when it gets none
/// (ADR 0006 §1, "Sliding refresh, passive").
///
/// - `scope`: the session's scope; only a full `admin` session is renewed.
/// - `carrier`: where the token came from; only the session cookie is
///   renewed (ADR 0006 §3).
/// - `now`: the app clock's instant of the request.
/// - `times`: the token's `auth_time` and `exp`.
/// - `limit`: the absolute limit.
///
/// A renewal is due when [`passive_refresh_due`] holds, and is given only if
/// it ends later than the token it replaces: in the last hour before the
/// absolute end, a renewal would end where the token already does, and a new
/// cookie per request would change nothing.
#[must_use]
pub fn passive_renewal(
    scope: SessionScope,
    carrier: TokenCarrier,
    now: DateTime<Utc>,
    times: SessionTimes,
    limit: AbsoluteLimit,
) -> Option<Renewal> {
    if scope != SessionScope::Admin
        || carrier != TokenCarrier::Cookie
        || !passive_refresh_due(now, times.expires_at)
    {
        return None;
    }
    renewal(now, times.auth_time, limit).filter(|renewed| renewed.expires_at > times.expires_at)
}

/// The renewal that `POST /api/admin/auth/refresh` gives a session, or
/// `None` when it gives none (ADR 0006 §1, "Sliding refresh, explicit").
///
/// - `carrier`: where the token came from; a Bearer token is never refreshed
///   (ADR 0006 §3).
/// - `now`: the app clock's instant of the request.
/// - `times`: the token's `auth_time` and `exp`.
/// - `limit`: the absolute limit.
///
/// The route accepts only a full `admin` session, so the scope is not asked
/// again. The renewal is [`renewal`]'s, whatever is left of the token: two
/// hours after `now`, or the absolute end. It may end earlier than the
/// token it replaces, when the absolute limit was lowered after the token
/// was issued; the cookie then follows the new limit.
#[must_use]
pub fn explicit_renewal(
    carrier: TokenCarrier,
    now: DateTime<Utc>,
    times: SessionTimes,
    limit: AbsoluteLimit,
) -> Option<Renewal> {
    match carrier {
        TokenCarrier::Cookie => renewal(now, times.auth_time, limit),
        TokenCarrier::Bearer => None,
    }
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
        // It is evaluated at compile time, as `accepted_scopes` evaluates it.
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

/// The sliding refresh's arithmetic (ADR 0006 §1, §8): the unit tests pin
/// each documented case, and the properties cover the time-travel cases of
/// ADR 0006's follow-up work, the passive threshold, the end of an idle
/// session between 1 h 50 min and 2 h after its last activity, and never a
/// renewal past the absolute limit.
#[cfg(test)]
mod refresh_tests {
    use chrono::TimeDelta;
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    /// The admin UI's least interval between two explicit refreshes, 10
    /// minutes (ADR 0006 §1): `REFRESH_INTERVAL_MS` of
    /// `admin/src/auth/activity-refresh.ts`, which the server never sees.
    /// The idle-result model below follows it.
    const CLIENT_REFRESH_INTERVAL_SECS: i64 = 600;

    /// A start instant for the cases: 2026-11-02T09:00:00Z, a whole second.
    const T: i64 = 1_793_610_000;

    /// The instant `secs` seconds and `nanos` nanoseconds after the epoch.
    fn at(secs: i64, nanos: u32) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, nanos).expect("in chrono's range")
    }

    /// The default absolute limit, 12 hours.
    fn default_limit() -> AbsoluteLimit {
        AbsoluteLimit::new(AbsoluteLimit::DEFAULT_SECS).expect("the default is valid")
    }

    /// A token signed in at `auth_time` that expires at `expires_at`.
    fn times(auth_time: i64, expires_at: i64) -> SessionTimes {
        SessionTimes {
            auth_time,
            expires_at,
        }
    }

    /// The renewal that ends at `expires_at`, `lifetime_secs` after now, as
    /// the functions under test return it.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "it builds the expected `Option` that the renewals return"
    )]
    fn renewed(expires_at: i64, lifetime_secs: u64) -> Option<Renewal> {
        Some(Renewal {
            expires_at,
            lifetime_secs,
        })
    }

    #[test]
    fn the_absolute_limit_is_at_least_the_session_lifetime() {
        assert_eq!(AbsoluteLimit::DEFAULT_SECS, 12 * 60 * 60);
        for secs in [
            SESSION_LIFETIME_SECS,
            7201,
            AbsoluteLimit::DEFAULT_SECS,
            u64::MAX,
        ] {
            assert_eq!(AbsoluteLimit::new(secs).map(AbsoluteLimit::secs), Ok(secs));
        }
        for found in [0, 1, 3600, SESSION_LIFETIME_SECS - 1] {
            let error = AbsoluteLimit::new(found).expect_err("refused");
            assert_eq!(error, AbsoluteLimitError { found });
            let message = error.to_string();
            assert!(
                message.starts_with(&format!(
                    "is {found} seconds; the absolute session limit must be at least 7200 seconds"
                )),
                "{message}"
            );
            assert!(message.contains("the default is 43200"), "{message}");
        }
    }

    #[test]
    fn the_session_end_is_the_sign_in_plus_the_limit_and_saturates() {
        assert_eq!(default_limit().session_end(T), T + 43_200);
        let endless = AbsoluteLimit::new(u64::MAX).expect("valid");
        assert_eq!(endless.session_end(T), i64::MAX);
        assert_eq!(default_limit().session_end(i64::MAX - 1), i64::MAX);
        assert_eq!(default_limit().session_end(i64::MIN), i64::MIN + 43_200);
    }

    #[test]
    fn session_times_are_read_from_the_claims() {
        let claims = json!({ "pid": "x", "exp": T + 7200, "tv": 0, "auth_time": T });
        assert_eq!(
            SessionTimes::from_claims(claims.as_object().expect("object")),
            Ok(times(T, T + 7200))
        );
        for (claims, error) in [
            (json!({ "exp": T }), SessionTimesError::InvalidAuthTime),
            (
                json!({ "exp": T, "auth_time": "1793610000" }),
                SessionTimesError::InvalidAuthTime,
            ),
            (
                json!({ "exp": T, "auth_time": 1.5 }),
                SessionTimesError::InvalidAuthTime,
            ),
            (json!({ "auth_time": T }), SessionTimesError::InvalidExpiry),
            (
                json!({ "auth_time": T, "exp": null }),
                SessionTimesError::InvalidExpiry,
            ),
            // Loco's `exp` is a `u64`; beyond `i64` it is no instant here.
            (
                json!({ "auth_time": T, "exp": u64::MAX }),
                SessionTimesError::InvalidExpiry,
            ),
            // Both missing: the `auth_time` is named first.
            (json!({}), SessionTimesError::InvalidAuthTime),
        ] {
            assert_eq!(
                SessionTimes::from_claims(claims.as_object().expect("object")),
                Err(error),
                "{claims}"
            );
        }
        assert_eq!(
            SessionTimesError::InvalidAuthTime.to_string(),
            "the token has no `auth_time` claim that is an integer"
        );
        assert_eq!(
            SessionTimesError::InvalidExpiry.to_string(),
            "the token has no `exp` claim that is an integer"
        );
    }

    #[test]
    fn a_renewal_ends_two_hours_after_the_second_of_now() {
        // An hour after the sign-in, the absolute limit is far away.
        let auth_time = T - 3600;
        assert_eq!(
            renewal(at(T, 0), auth_time, default_limit()),
            renewed(T + 7200, 7200)
        );
        // The fraction of `now` is dropped, as sign-in drops it from
        // `auth_time`: the renewal never ends more than two hours after the
        // request.
        assert_eq!(
            renewal(at(T, 999_999_999), auth_time, default_limit()),
            renewed(T + 7200, 7200)
        );
        // A sign-in "in the future" (only a defect could sign one) is not
        // capped any earlier.
        assert_eq!(
            renewal(at(T, 0), i64::MAX, default_limit()),
            renewed(T + 7200, 7200)
        );
    }

    #[test]
    fn a_renewal_never_ends_past_the_absolute_limit() {
        let limit = default_limit();
        // Eleven hours after the sign-in, one hour of the limit is left.
        assert_eq!(
            renewal(at(T, 0), T - 11 * 3600, limit),
            renewed(T + 3600, 3600)
        );
        // One second left.
        assert_eq!(renewal(at(T, 0), T - 43_200 + 1, limit), renewed(T + 1, 1));
        // At and past the limit: no renewal.
        for auth_time in [T - 43_200, T - 43_201, T - 13 * 3600, i64::MIN] {
            assert_eq!(renewal(at(T, 0), auth_time, limit), None, "{auth_time}");
        }
        // The shortest limit allowed renews a fresh sign-in to the same two
        // hours that sign-in gave it, and no further.
        let shortest = AbsoluteLimit::new(SESSION_LIFETIME_SECS).expect("valid");
        assert_eq!(renewal(at(T, 0), T, shortest), renewed(T + 7200, 7200));
        assert_eq!(
            renewal(at(T + 600, 0), T, shortest),
            renewed(T + 7200, 6600)
        );
    }

    #[test]
    fn a_capped_renewal_counts_from_now_rounded_up() {
        // Loco reads the system clock after "now". With a fraction of a
        // second on "now", that reading may fall in the next second, so the
        // time left before the absolute end counts from there: the lifetime
        // is one second shorter, and `exp` is the end at the latest.
        let limit = default_limit();
        let eleven_hours_ago = T - 11 * 3600;
        for nanos in [1, 500_000_000, 999_999_999] {
            assert_eq!(
                renewal(at(T, nanos), eleven_hours_ago, limit),
                renewed(T + 3599, 3599),
                "{nanos} ns"
            );
        }
        // One second left at a whole second renews it; a moment later, not.
        let one_second_left = T - 43_200 + 1;
        assert_eq!(renewal(at(T, 0), one_second_left, limit), renewed(T + 1, 1));
        assert_eq!(renewal(at(T, 1), one_second_left, limit), None);
        // Two seconds left a moment after a whole second: one second.
        assert_eq!(
            renewal(at(T, 1), one_second_left + 1, limit),
            renewed(T + 1, 1)
        );
        // Exactly two hours before the end, the cap binds only once "now"
        // has a fraction; with more left, the two hours stay whole.
        let two_hours_left = T + 7200 - 43_200;
        assert_eq!(
            renewal(at(T, 0), two_hours_left, limit),
            renewed(T + 7200, 7200)
        );
        assert_eq!(
            renewal(at(T, 1), two_hours_left, limit),
            renewed(T + 7199, 7199)
        );
        assert_eq!(
            renewal(at(T, 1), two_hours_left + 1, limit),
            renewed(T + 7200, 7200)
        );
        // On a leap second, chrono keeps the second before it (`T - 1`, a
        // minute's 59th) and counts nanoseconds of one second or more: half
        // way through it, "now" is past `T` and rounds up to `T + 1`; at its
        // start, exactly `T`.
        let mid_leap = DateTime::from_timestamp(T - 1, 1_500_000_000).expect("a leap second");
        assert_eq!(
            renewal(mid_leap, eleven_hours_ago, limit),
            renewed(T + 3598, 3599)
        );
        let leap_start = DateTime::from_timestamp(T - 1, 1_000_000_000).expect("a leap second");
        assert_eq!(
            renewal(leap_start, eleven_hours_ago, limit),
            renewed(T + 3599, 3600)
        );
    }

    #[test]
    fn loco_s_later_reading_never_issues_an_exp_past_the_absolute_end() {
        // Loco writes `exp` as its own reading of the system clock, in whole
        // seconds, plus the lifetime (loco-rs 1.2.0 `JWT::generate_token`).
        // For every reading less than a second after "now", that `exp` stays
        // within the absolute end (ADR 0006 §1), and it is `expires_at` or
        // one second later.
        let limit = default_limit();
        for auth_time in [T - 3600, T - 11 * 3600, T - 43_200 + 7200, T - 43_200 + 2] {
            for nanos in [0, 1, 400_000_000, 999_999_999] {
                let now = at(T, nanos);
                let renewed = renewal(now, auth_time, limit).expect("time is left");
                let lifetime = i64::try_from(renewed.lifetime_secs).expect("small");
                for delay_nanos in [0, 1, 1_000, 600_000_000, 999_999_999] {
                    let loco_reads = now + TimeDelta::nanoseconds(delay_nanos);
                    let exp = loco_reads.timestamp() + lifetime;
                    assert!(
                        exp <= limit.session_end(auth_time),
                        "{auth_time}, {nanos} ns, read {delay_nanos} ns later: {exp}"
                    );
                    assert!(
                        (renewed.expires_at..=renewed.expires_at + 1).contains(&exp),
                        "{auth_time}, {nanos} ns, read {delay_nanos} ns later: {exp}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_passive_threshold_is_exact() {
        // 3,600 s left is not due; anything less is.
        assert!(!passive_refresh_due(at(T, 0), T + 3600));
        assert!(passive_refresh_due(at(T, 1), T + 3600));
        assert!(passive_refresh_due(at(T, 0), T + 3599));
        // 3,600.5 s left is not due, 3,599.5 s is.
        assert!(!passive_refresh_due(at(T, 500_000_000), T + 3601));
        assert!(passive_refresh_due(at(T, 500_000_000), T + 3600));
        // A fresh two-hour token is not due; an expired one counts as due.
        assert!(!passive_refresh_due(at(T, 0), T + 7200));
        assert!(passive_refresh_due(at(T, 0), T));
        assert!(passive_refresh_due(at(T, 0), T - 10));
        // The ends of `i64` do not overflow.
        assert!(!passive_refresh_due(at(T, 0), i64::MAX));
        assert!(passive_refresh_due(at(T, 0), i64::MIN));
    }

    #[test]
    fn the_passive_refresh_renews_a_due_admin_cookie_session_only() {
        let limit = default_limit();
        // Signed in 1 h 10 min ago, 50 min left: due.
        let due = times(T - 4200, T + 3000);
        assert_eq!(
            passive_renewal(
                SessionScope::Admin,
                TokenCarrier::Cookie,
                at(T, 0),
                due,
                limit
            ),
            renewed(T + 7200, 7200)
        );
        // Not for another scope, nor for a Bearer token (ADR 0006 §3).
        for (scope, carrier) in [
            (SessionScope::PasswordChange, TokenCarrier::Cookie),
            (SessionScope::Mfa, TokenCarrier::Cookie),
            (SessionScope::Admin, TokenCarrier::Bearer),
            (SessionScope::PasswordChange, TokenCarrier::Bearer),
        ] {
            assert_eq!(
                passive_renewal(scope, carrier, at(T, 0), due, limit),
                None,
                "{scope} by {carrier:?}"
            );
        }
        // Not with an hour or more left.
        for left in [3600, 3601, 7200] {
            assert_eq!(
                passive_renewal(
                    SessionScope::Admin,
                    TokenCarrier::Cookie,
                    at(T, 0),
                    times(T - 600, T + left),
                    limit
                ),
                None,
                "{left} s left"
            );
        }
        // In the last hour before the absolute end the token already ends
        // there, so a renewal would change nothing.
        let at_the_end = times(T - 43_200 + 1800, T + 1800);
        assert_eq!(
            passive_renewal(
                SessionScope::Admin,
                TokenCarrier::Cookie,
                at(T, 0),
                at_the_end,
                limit
            ),
            None
        );
        // Ninety minutes before the end, with 50 min left, it extends the
        // token to the end and no further.
        assert_eq!(
            passive_renewal(
                SessionScope::Admin,
                TokenCarrier::Cookie,
                at(T, 0),
                times(T - 43_200 + 5400, T + 3000),
                limit
            ),
            renewed(T + 5400, 5400)
        );
    }

    #[test]
    fn the_explicit_refresh_renews_a_cookie_session_whatever_is_left() {
        let limit = default_limit();
        // With the full two hours left, it still sets a new two-hour expiry.
        for left in [1, 3000, 3600, 7200] {
            assert_eq!(
                explicit_renewal(
                    TokenCarrier::Cookie,
                    at(T, 0),
                    times(T - 600, T + left),
                    limit
                ),
                renewed(T + 7200, 7200),
                "{left} s left"
            );
        }
        // A Bearer token is not refreshed (ADR 0006 §3).
        assert_eq!(
            explicit_renewal(
                TokenCarrier::Bearer,
                at(T, 0),
                times(T - 600, T + 3000),
                limit
            ),
            None
        );
        // At the absolute end there is nothing to renew.
        assert_eq!(
            explicit_renewal(TokenCarrier::Cookie, at(T, 0), times(T - 43_200, T), limit),
            None
        );
        // A limit lowered after the token was issued shortens it.
        let four_hours = AbsoluteLimit::new(4 * 3600).expect("valid");
        assert_eq!(
            explicit_renewal(
                TokenCarrier::Cookie,
                at(T, 0),
                times(T - 3 * 3600, T + 7000),
                four_hours
            ),
            renewed(T + 3600, 3600)
        );
    }

    #[test]
    fn both_carriers_are_listed() {
        assert_eq!(
            TokenCarrier::ALL,
            [TokenCarrier::Cookie, TokenCarrier::Bearer]
        );
    }

    /// An instant within a few centuries of the cases, with any fraction.
    fn instant() -> impl Strategy<Value = DateTime<Utc>> {
        (0_i64..=8_000_000_000, 0_u32..1_000_000_000).prop_map(|(secs, nanos)| at(secs, nanos))
    }

    /// Any valid absolute limit: often near the lifetime and the default,
    /// sometimes enormous.
    fn limit() -> impl Strategy<Value = AbsoluteLimit> {
        prop_oneof![
            3 => SESSION_LIFETIME_SECS..=100_000_u64,
            1 => Just(AbsoluteLimit::DEFAULT_SECS),
            1 => any::<u64>().prop_map(|secs| secs.max(SESSION_LIFETIME_SECS)),
        ]
        .prop_map(|secs| AbsoluteLimit::new(secs).expect("at least the lifetime"))
    }

    /// The instant `now` and an `auth_time` up to a day before it, or near
    /// the ends of `i64`.
    fn now_and_auth_time() -> impl Strategy<Value = (DateTime<Utc>, i64)> {
        instant().prop_flat_map(|now| {
            let secs = now.timestamp();
            (
                Just(now),
                prop_oneof![
                    8 => (secs - 2 * 86_400)..=(secs + 60),
                    1 => Just(i64::MIN),
                    1 => Just(i64::MAX),
                ],
            )
        })
    }

    /// The scopes and carriers, for the decision properties.
    fn scope() -> impl Strategy<Value = SessionScope> {
        proptest::sample::select(SessionScope::ALL.to_vec())
    }

    /// Either carrier.
    fn carrier() -> impl Strategy<Value = TokenCarrier> {
        proptest::sample::select(TokenCarrier::ALL.to_vec())
    }

    /// A token's times for a request at `now`: signed in up to 13 hours
    /// before, expiring from a few seconds before `now` to two hours after.
    fn token_times(now: DateTime<Utc>) -> impl Strategy<Value = SessionTimes> {
        let secs = now.timestamp();
        ((secs - 13 * 3600)..=secs, (secs - 5)..=(secs + 7200))
            .prop_map(|(auth_time, expires_at)| times(auth_time, expires_at))
    }

    /// One user activity of the idle-result model: the whole seconds since
    /// the previous one (or since the sign-in), and whether it also sends a
    /// data request, which gets the passive refresh.
    fn activities() -> impl Strategy<Value = Vec<(i64, bool)>> {
        proptest::collection::vec(
            (
                prop_oneof![
                    4 => 1_i64..=CLIENT_REFRESH_INTERVAL_SECS,
                    3 => 1_i64..=6600,
                    1 => 6601_i64..=9000,
                ],
                any::<bool>(),
            ),
            0..60,
        )
    }

    proptest! {
        /// A renewal is two hours after `now`'s second or ends at the
        /// absolute end counted from `now` rounded up, whichever is earlier:
        /// never past the end, never more than two hours after the request,
        /// and at most one second short of the end when capped.
        #[test]
        fn a_renewal_is_the_earlier_of_two_hours_and_the_absolute_end(
            (now, auth_time) in now_and_auth_time(),
            limit in limit(),
        ) {
            let start = now.timestamp();
            // `now` rounded up, through chrono rather than the nanoseconds.
            let ceil = if now == at(start, 0) { start } else { start + 1 };
            let end = limit.session_end(auth_time);
            match renewal(now, auth_time, limit) {
                Some(renewed) => {
                    let lifetime = i64::try_from(renewed.lifetime_secs).expect("small");
                    prop_assert_eq!(lifetime, LIFETIME_SECS.min(end - ceil));
                    prop_assert_eq!(renewed.expires_at, start + lifetime);
                    prop_assert!((1..=SESSION_LIFETIME_SECS).contains(&renewed.lifetime_secs));
                    // Even Loco's reading in the next second stays within
                    // the end.
                    prop_assert!(ceil + lifetime <= end);
                    // Never more than two hours after the request itself;
                    // uncapped, less than one second short of it; capped,
                    // at most one second short of the end.
                    let expiry = at(renewed.expires_at, 0);
                    prop_assert!(expiry <= now + TimeDelta::seconds(LIFETIME_SECS));
                    if lifetime == LIFETIME_SECS {
                        prop_assert!(expiry > now + TimeDelta::seconds(LIFETIME_SECS - 1));
                    } else {
                        prop_assert!(renewed.expires_at >= end - 1);
                    }
                }
                None => prop_assert!(end <= ceil, "{} > {}", end, ceil),
            }
        }

        /// Loco writes `exp` from its own reading of the system clock, in
        /// whole seconds, plus the lifetime (loco-rs 1.2.0
        /// `JWT::generate_token`). Whenever that reading comes less than a
        /// second after `now`, the `exp` is `expires_at` or one second
        /// later, and never past `auth_time` plus the limit (ADR 0006 §1).
        #[test]
        fn loco_s_exp_never_passes_the_absolute_end(
            (now, auth_time) in now_and_auth_time(),
            limit in limit(),
            delay_nanos in prop_oneof![Just(0_i64), 0_i64..1_000_000_000, Just(999_999_999_i64)],
        ) {
            if let Some(renewed) = renewal(now, auth_time, limit) {
                let lifetime = i64::try_from(renewed.lifetime_secs).expect("small");
                let exp = (now + TimeDelta::nanoseconds(delay_nanos)).timestamp() + lifetime;
                prop_assert!(exp <= limit.session_end(auth_time), "{} past the end", exp);
                prop_assert!(
                    (renewed.expires_at..=renewed.expires_at + 1).contains(&exp),
                    "{} for {}",
                    exp,
                    renewed.expires_at
                );
            }
        }

        /// The threshold agrees with chrono's own arithmetic.
        #[test]
        fn the_threshold_agrees_with_chrono(
            now in instant(),
            offset in -10_i64..=10_000,
        ) {
            let expires_at = now.timestamp() + offset;
            let left = at(expires_at, 0) - now;
            prop_assert_eq!(
                passive_refresh_due(now, expires_at),
                left < TimeDelta::seconds(3600)
            );
        }

        /// The passive refresh renews only a due full session in the cookie,
        /// only to a later expiry, and then exactly as a refresh would.
        #[test]
        fn the_passive_refresh_only_extends_a_due_admin_cookie_session(
            (now, token) in instant().prop_flat_map(|now| (Just(now), token_times(now))),
            scope in scope(),
            carrier in carrier(),
            limit in limit(),
        ) {
            let renewed = renewal(now, token.auth_time, limit);
            let eligible = scope == SessionScope::Admin
                && carrier == TokenCarrier::Cookie
                && passive_refresh_due(now, token.expires_at);
            match passive_renewal(scope, carrier, now, token, limit) {
                Some(passive) => {
                    prop_assert!(eligible);
                    prop_assert!(passive.expires_at > token.expires_at);
                    prop_assert_eq!(Some(passive), renewed);
                }
                None => prop_assert!(
                    !eligible || renewed.is_none_or(|r| r.expires_at <= token.expires_at)
                ),
            }
        }

        /// The explicit refresh is the plain renewal for the cookie, and
        /// nothing for a Bearer token.
        #[test]
        fn the_explicit_refresh_is_the_renewal_for_the_cookie_only(
            (now, token) in instant().prop_flat_map(|now| (Just(now), token_times(now))),
            carrier in carrier(),
            limit in limit(),
        ) {
            let expected = match carrier {
                TokenCarrier::Cookie => renewal(now, token.auth_time, limit),
                TokenCarrier::Bearer => None,
            };
            prop_assert_eq!(explicit_renewal(carrier, now, token, limit), expected);
        }

        /// The idle result of ADR 0006 §1, time-travelled through one
        /// session: a sign-in at `sign_in`, then user activity, each of which
        /// calls the explicit refresh when at least 10 minutes have passed
        /// since the previous call (or none was made yet), and may send a
        /// data request that gets the passive refresh. Loco refuses a token
        /// once `exp` is behind the clock, which ends the session. Then:
        ///
        /// - no token ever expires past `auth_time` plus the limit;
        /// - the session ends no later than 2 h after the last activity it
        ///   saw, and no earlier than 1 h 50 min after it, unless the
        ///   absolute limit ends it first.
        #[test]
        fn an_idle_session_ends_between_1h50_and_2h_after_its_last_activity(
            sign_in in 0_i64..=8_000_000_000,
            steps in activities(),
            limit in limit(),
        ) {
            let limit_end = limit.session_end(sign_in);
            // Sign-in: a two-hour token with `auth_time` at the sign-in.
            let mut token = times(sign_in, sign_in + LIFETIME_SECS);
            let mut last_call: Option<i64> = None;
            let mut last_activity = sign_in;
            let mut now = sign_in;
            for (gap, data_request) in steps {
                now += gap;
                if token.expires_at < now {
                    // The session ended; this activity meets a 401.
                    break;
                }
                last_activity = now;
                if last_call.is_none_or(|last| now - last >= CLIENT_REFRESH_INTERVAL_SECS) {
                    last_call = Some(now);
                    if let Some(renewed) =
                        explicit_renewal(TokenCarrier::Cookie, at(now, 0), token, limit)
                    {
                        token.expires_at = renewed.expires_at;
                    }
                }
                if data_request
                    && let Some(renewed) = passive_renewal(
                        SessionScope::Admin,
                        TokenCarrier::Cookie,
                        at(now, 0),
                        token,
                        limit,
                    )
                {
                    token.expires_at = renewed.expires_at;
                }
                prop_assert!(token.expires_at <= limit_end.max(sign_in + LIFETIME_SECS));
                prop_assert_eq!(token.auth_time, sign_in, "a refresh keeps auth_time");
            }
            let end = token.expires_at;
            // The sign-in's own token is the only one the limit does not
            // cap, and the shortest limit equals its lifetime.
            prop_assert!(end <= limit_end);
            prop_assert!(end <= last_activity + LIFETIME_SECS, "{end} > {last_activity} + 2 h");
            prop_assert!(
                end >= (last_activity + LIFETIME_SECS - CLIENT_REFRESH_INTERVAL_SECS).min(limit_end),
                "{end} < {last_activity} + 1 h 50 min"
            );
        }

        /// The limit check accepts exactly the values from the lifetime up.
        #[test]
        fn only_limits_of_at_least_two_hours_are_accepted(secs in any::<u64>()) {
            prop_assert_eq!(AbsoluteLimit::new(secs).is_ok(), secs >= SESSION_LIFETIME_SECS);
        }

        /// The times read back what the claims hold, whatever else they
        /// hold.
        #[test]
        fn session_times_read_back_their_claims(
            auth_time in any::<i64>(),
            expires_at in any::<i64>(),
            extra in proptest::collection::btree_map("[a-z_]{1,8}", any::<i64>(), 0..4),
        ) {
            let mut claims: Map<String, Value> = extra
                .into_iter()
                .map(|(key, value)| (key, json!(value)))
                .collect();
            claims.insert(AUTH_TIME_CLAIM.to_owned(), json!(auth_time));
            claims.insert(EXPIRY_CLAIM.to_owned(), json!(expires_at));
            prop_assert_eq!(
                SessionTimes::from_claims(&claims),
                Ok(times(auth_time, expires_at))
            );
        }
    }
}
