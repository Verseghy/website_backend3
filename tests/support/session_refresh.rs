//! Helpers for the tests of the sliding session refresh (ADR 0006 §1, §8).
//!
//! The refresh decides from the app clock: how much of a token is left at
//! "now", and how far "now" is from its `auth_time`. Loco's JWT reads the
//! system time instead: `JWT::generate_token` sets `exp` to the wall clock
//! plus the lifetime it is given, and `validate` refuses a token whose `exp`
//! is behind the wall clock (loco-rs 1.2.0 `src/auth/jwt.rs`; recipe 7
//! step 4 of the project skill). So a test cannot age a token by moving the
//! [`TestClock`]: Loco would still accept it.
//!
//! [`place_session`] puts the app clock where a test wants it relative to a
//! real token instead: it signs the token, reads the `exp` that Loco gave
//! it, and sets the clock to that `exp` minus the time the test wants left.
//! Loco keeps accepting the token, because the wall clock is still before
//! its `exp`, while the refresh sees exactly the remaining time and the
//! `auth_time` the test chose. No helper here reads the wall clock: the
//! only wall-clock instant is the `exp` that Loco wrote into the token.
//!
//! Every account and token here is invented.

use axum_test::TestResponse;
use backend3::{
    controllers::admin::session::session_times,
    domain::{session::SessionTimes, session_cookie::SESSION_COOKIE_NAME},
    models::users::Model,
};
use chrono::{DateTime, TimeDelta, Utc};
use loco_rs::{app::AppContext, auth::jwt::JWT};

use super::TestClock;

/// A session token placed on the app clock's timeline by [`place_session`].
#[derive(Debug, Clone)]
pub struct PlacedSession {
    /// The full `admin` session token.
    pub token: String,
    /// Where the app clock now stands: the token's `exp` minus the time the
    /// test asked to be left.
    pub now: DateTime<Utc>,
    /// The token's `auth_time` and `exp`, as signed.
    pub times: SessionTimes,
}

/// `auth.jwt.secret` of the test configuration.
///
/// - `ctx`: the test's app context.
///
/// # Panics
///
/// When the configuration has no `auth.jwt` section.
pub fn jwt_secret(ctx: &AppContext) -> String {
    ctx.config
        .get_jwt_config()
        .expect("auth.jwt")
        .secret
        .clone()
}

/// The `auth_time` and `exp` of `token`, which Loco validates with the test
/// configuration's secret first.
///
/// - `ctx`: the test's app context, for the secret.
/// - `token`: a session token.
///
/// # Panics
///
/// When the token does not validate (expired included) or lacks either
/// claim.
pub fn token_times(ctx: &AppContext, token: &str) -> SessionTimes {
    let claims = JWT::new(&jwt_secret(ctx))
        .validate(token)
        .expect("a valid session token")
        .claims;
    session_times(&claims).expect("auth_time and exp")
}

/// The whole seconds of `delta`.
fn whole_secs(delta: TimeDelta) -> i64 {
    assert_eq!(
        delta.subsec_nanos(),
        0,
        "{delta}: the helpers place tokens to the second, as JWTs count"
    );
    delta.num_seconds()
}

/// The instant `secs` seconds after the Unix epoch.
///
/// # Panics
///
/// Outside chrono's range.
pub fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("in chrono's range")
}

/// Signs a full `admin` session token for `user` and sets `clock` so that
/// the token signed in `signed_in` before "now" and has `left` of its
/// lifetime left at "now".
///
/// - `ctx`: the test's app context, for the JWT secret.
/// - `clock`: the test's app clock, which is moved.
/// - `user`: the account; the token carries its current `token_version`.
/// - `signed_in`: how long before "now" the password step succeeded
///   (`auth_time`); whole seconds.
/// - `left`: how much of the token is left at "now"; whole seconds, more
///   than zero. It is also the token's lifetime for Loco, which refuses it
///   that long after it was signed by the wall clock, so keep it at a
///   minute or more.
///
/// A first, throwaway token tells the helper where Loco's `exp` falls, so
/// that the real token's `auth_time` can be chosen from it; Loco may write
/// the second token's `exp` one second later when the wall clock's second
/// changes in between, which is why the returned [`PlacedSession::times`]
/// are read back from the token: compute every expectation from them, and
/// keep a scenario's margins above one second.
///
/// # Panics
///
/// When `left` is not positive, a duration has a fraction of a second, or
/// a token cannot be signed or read back.
pub fn place_session(
    ctx: &AppContext,
    clock: &TestClock,
    user: &Model,
    signed_in: TimeDelta,
    left: TimeDelta,
) -> PlacedSession {
    let left_secs = whole_secs(left);
    let lifetime = u64::try_from(left_secs)
        .ok()
        .filter(|secs| *secs > 0)
        .expect("left must be more than zero");
    let secret = jwt_secret(ctx);
    let probe = user
        .generate_session_token(&secret, lifetime, at(0))
        .expect("signed");
    let now = token_times(ctx, &probe).expires_at - left_secs;
    let token = user
        .generate_session_token(&secret, lifetime, at(now - whole_secs(signed_in)))
        .expect("signed");
    let times = token_times(ctx, &token);
    let now = at(times.expires_at - left_secs);
    clock.set(now);
    PlacedSession { token, now, times }
}

/// The `Cookie` header a browser sends with `token` in the session cookie,
/// beside another cookie.
///
/// - `token`: the session token.
pub fn cookie_header(token: &str) -> String {
    format!("theme=dark; {SESSION_COOKIE_NAME}={token}")
}

/// Every `Set-Cookie` value of `response`.
///
/// - `response`: the answer.
///
/// # Panics
///
/// When a value is not ASCII.
pub fn set_cookies(response: &TestResponse) -> Vec<String> {
    response
        .iter_headers_by_name("set-cookie")
        .map(|value| value.to_str().expect("ASCII").to_owned())
        .collect()
}

/// A session cookie that an answer set: its token and its `Max-Age`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSessionCookie {
    /// The token, the cookie's value.
    pub token: String,
    /// The `Max-Age` attribute, in seconds.
    pub max_age: u64,
    /// The whole `Set-Cookie` value, for exact comparisons.
    pub header: String,
}

/// The one session cookie that `response` sets, or `None` when it sets no
/// cookie at all.
///
/// - `response`: the answer.
///
/// # Panics
///
/// When the answer sets more than one cookie, a cookie other than the
/// session cookie, or one without the attributes of ADR 0006 §1.
pub fn renewed_cookie(response: &TestResponse) -> Option<SetSessionCookie> {
    let cookies = set_cookies(response);
    let header = match &cookies[..] {
        [] => return None,
        [one] => one.clone(),
        many => panic!("one Set-Cookie at most expected: {many:?}"),
    };
    let (pair, attributes) = header.split_once("; ").expect("attributes");
    let token = pair
        .strip_prefix(&format!("{SESSION_COOKIE_NAME}="))
        .unwrap_or_else(|| panic!("the session cookie expected: {header}"))
        .to_owned();
    let max_age = attributes
        .strip_prefix("HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=")
        .unwrap_or_else(|| panic!("the attributes of ADR 0006 §1 expected: {header}"))
        .parse()
        .unwrap_or_else(|error| panic!("{header}: {error}"));
    Some(SetSessionCookie {
        token,
        max_age,
        header,
    })
}
