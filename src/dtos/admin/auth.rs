//! Wire types of the browser sign-in, `POST /api/admin/auth/login`
//! (ADR 0006 §4), and of the signed-in account, `GET /api/admin/auth/me`
//! (ADR 0007 §7).
//!
//! The sign-in response names the next step of the sign-in and never carries
//! the session token: the token travels only in the `HttpOnly` session cookie
//! (ADR 0006 §3, "The browser login endpoint never returns the token in the
//! response body"). [`MeResponse`] tells the admin UI who is signed in and
//! what the session may do; it never carries a secret column of the account
//! (ADR 0005, ADR 0011 §4).

use std::fmt;

use loco_rs::prelude::Uuid;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor, value::MapAccessDeserializer},
};
use ts_rs::TS;

// The domain's scope only: DTOs depend on `domain/` and on no model
// (ADR 0008, "Allowed dependencies"). The handler takes the account's
// columns apart and passes them in.
use crate::domain::session;

/// The body of a browser sign-in: the account's e-mail address and password.
///
/// Read only from a JSON object with exactly these two string members.
// No `Debug` on purpose: a derived one would print the password, and without
// it a `tracing` call that tries to log the request does not compile.
#[derive(Clone, PartialEq, Eq, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct LoginRequest {
    /// The e-mail address, compared without regard to case.
    pub email: String,
    /// The password as typed.
    pub password: String,
}

/// The members of a [`LoginRequest`], read by serde's derive.
///
/// The derive refuses a missing, repeated or wrongly typed member, and
/// `deny_unknown_fields` an unknown one, so a typo in a client is refused
/// rather than silently ignored (research: DECISIONS R15 (D74)).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginFields {
    email: String,
    password: String,
}

/// Reads a [`LoginRequest`] from a map, and from nothing else.
struct LoginRequestVisitor;

impl<'de> Visitor<'de> for LoginRequestVisitor {
    type Value = LoginRequest;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object with the members email and password")
    }

    /// Hands the map, unread, to the derived [`LoginFields`].
    ///
    /// - `map`: the members of the JSON object, read one by one.
    ///
    /// `MapAccessDeserializer` lets the derive read the members as they are
    /// parsed. A body is refused at the name of its first unknown or repeated
    /// member, before that member's value is parsed, and only the values of
    /// `email` and `password` are ever kept.
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<LoginRequest, A::Error> {
        let LoginFields { email, password } =
            LoginFields::deserialize(MapAccessDeserializer::new(map))?;
        Ok(LoginRequest { email, password })
    }
}

impl<'de> Deserialize<'de> for LoginRequest {
    /// Reads the request from a JSON object only.
    ///
    /// - `deserializer`: the JSON body.
    ///
    /// serde's derive also reads a struct from an array of its field values
    /// in order, so `["a@example.test", "pw"]` would sign in. The visitor
    /// implements only `visit_map`, so anything but an object is refused.
    ///
    /// It streams on purpose. Reading a `serde_json::Map` or `Value` first
    /// would build the whole body in memory, many times its size for a body
    /// of empty arrays, before any member is checked; the endpoint takes
    /// unauthenticated bodies up to the request body limit. It would also
    /// keep the last of two members with the same name, where a proxy that
    /// reads the first would see another address.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(LoginRequestVisitor)
    }
}

/// The step the sign-in continues with after a correct password.
///
/// `done`: the full session cookie is set. ADR 0006 §4 also defines the
/// `totp` and `change_password` steps, which this version never sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../admin/src/bindings/")]
pub enum LoginStep {
    /// Signed in: the session cookie is set.
    Done,
}

/// The answer to a successful sign-in: the next step, and no token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct LoginResponse {
    /// What the admin UI does next.
    pub step: LoginStep,
}

/// What the signed-in session may do (ADR 0006 §1, §4).
///
/// `admin`: a full session. `password_change`: the account must set a new
/// password first, and only the own-account endpoints `auth/me`,
/// `auth/change-password` and `auth/logout` answer it.
// The doc comment above is exported into the TypeScript binding. The token's
// third scope, `mfa`, is left out on purpose: no route that reports a
// session accepts an `mfa` token (only the two-factor step reads one, from
// its own cookie), so the admin UI never sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../admin/src/bindings/")]
pub enum SessionScope {
    /// A full session.
    Admin,
    /// Only the password change, `auth/me` and logout.
    PasswordChange,
}

/// The signed-in account and its session, the answer of
/// `GET /api/admin/auth/me`.
///
/// The admin UI reads it after sign-in and on every reload, to show who is
/// signed in and to send a `password_change` session to the password form.
// Exported doc comment above. ADR 0007 §7 adds the roles, the effective
// permissions, `is_admin` and the access map; the slices that add roles
// and the access map add those fields here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../admin/src/bindings/")]
pub struct MeResponse {
    /// The account's public id, a UUID; the database id is never sent.
    pub pid: String,
    /// The account's display name.
    pub name: String,
    /// The e-mail address the account signs in with.
    pub email: String,
    /// What this session may do.
    pub scope: SessionScope,
}

/// Why a [`MeResponse`] could not be built. Either one is a defect, never a
/// client error: the session check and the `users` CHECK constraints rule
/// both out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MeResponseError {
    /// The account has no e-mail address. The anonymisation CHECKs of
    /// `users` allow that only on an anonymised row, which the session check
    /// refuses (ADR 0006 §7).
    #[error(
        "the signed-in account has no e-mail address, which only an anonymised account may \
         lack, and the session check refuses those (ADR 0006 §7)"
    )]
    NoEmail,
    /// The session's scope is `mfa`, which no route that reports a session
    /// accepts (ADR 0006 §4).
    #[error(
        "an mfa token opens no session that auth/me reports; only the two-factor step reads one \
         (ADR 0006 §4)"
    )]
    MfaScope,
}

impl TryFrom<session::SessionScope> for SessionScope {
    type Error = MeResponseError;

    /// The wire spelling of a session's scope.
    ///
    /// - `scope`: the scope that the session check granted.
    ///
    /// # Errors
    ///
    /// [`MeResponseError::MfaScope`] for an `mfa` scope, which the wire type
    /// leaves out (its definition says why).
    fn try_from(scope: session::SessionScope) -> Result<Self, MeResponseError> {
        match scope {
            session::SessionScope::Admin => Ok(Self::Admin),
            session::SessionScope::PasswordChange => Ok(Self::PasswordChange),
            session::SessionScope::Mfa => Err(MeResponseError::MfaScope),
        }
    }
}

impl MeResponse {
    /// Builds the answer for a signed-in account from its allow-listed
    /// columns.
    ///
    /// - `pid`: the account's public id (`users.pid`).
    /// - `name`: the account's display name (`users.name`).
    /// - `email`: the account's e-mail address (`users.email`), which only
    ///   an anonymised account lacks.
    /// - `scope`: the session's scope, which the route accepted.
    ///
    /// It takes these values, never the account's row, so nothing else can
    /// go out: not the password hash, the TOTP secret or the database id
    /// (ADR 0005, ADR 0011 §4).
    ///
    /// # Errors
    ///
    /// [`MeResponseError::NoEmail`] when `email` is `None`, and
    /// [`MeResponseError::MfaScope`] for an `mfa` scope. The admin extractor
    /// of `auth/me` lets neither through.
    pub fn new(
        pid: Uuid,
        name: String,
        email: Option<String>,
        scope: session::SessionScope,
    ) -> Result<Self, MeResponseError> {
        let scope = SessionScope::try_from(scope)?;
        let email = email.ok_or(MeResponseError::NoEmail)?;
        Ok(Self {
            pid: pid.to_string(),
            name,
            email,
            scope,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Why `body` is not a [`LoginRequest`]; panics when it is one.
    ///
    /// - `body`: the JSON text to read.
    ///
    /// `expect_err` would need `LoginRequest: Debug`, which it lacks on
    /// purpose (its definition says why).
    fn refusal(body: &str) -> serde_json::Error {
        match serde_json::from_str::<LoginRequest>(body) {
            Ok(_) => panic!("accepted: {body}"),
            Err(error) => error,
        }
    }

    #[test]
    fn a_request_has_exactly_email_and_password() {
        let request: LoginRequest =
            serde_json::from_value(json!({ "email": "a@example.test", "password": "pw" }))
                .expect("valid");
        assert_eq!(
            (request.email.as_str(), request.password.as_str()),
            ("a@example.test", "pw")
        );
        for invalid in [
            json!({ "email": "a@example.test" }),
            json!({ "password": "pw" }),
            json!({ "email": "a@example.test", "password": 5 }),
            json!({ "email": "a@example.test", "password": "pw", "remember": true }),
            json!(["a@example.test", "pw"]),
        ] {
            assert!(
                serde_json::from_value::<LoginRequest>(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn a_repeated_member_is_refused_whichever_copy_is_right() {
        // The derive refuses a second `email` or `password`; a map read
        // first would keep the last copy and sign in with it.
        for body in [
            r#"{"email":"b@example.test","password":"x","email":"a@example.test","password":"pw"}"#,
            r#"{"email":"a@example.test","password":"pw","email":"b@example.test","password":"x"}"#,
            r#"{"email":"a@example.test","email":"a@example.test","password":"pw"}"#,
            r#"{"email":"a@example.test","password":"pw","password":"pw"}"#,
        ] {
            let error = refusal(body);
            assert!(
                error.to_string().contains("duplicate field"),
                "{body}: {error}"
            );
        }
    }

    #[test]
    fn a_body_is_refused_at_its_first_unknown_member_without_reading_on() {
        // What follows the unknown member is not JSON at all. A data error
        // that names the member, rather than a syntax error further on,
        // shows that nothing after it was parsed, let alone kept: the
        // property that bounds the memory of a large hostile body.
        for body in [
            r#"{"x": this is not JSON"#,
            r#"{"email":"a@example.test","password":"pw","x":[[],[],"#,
        ] {
            let error = refusal(body);
            assert_eq!(
                error.classify(),
                serde_json::error::Category::Data,
                "{body}: {error}"
            );
            assert!(
                error.to_string().contains("unknown field `x`"),
                "{body}: {error}"
            );
        }
    }

    #[test]
    fn anything_but_an_object_is_refused() {
        for body in [
            r#"["a@example.test","pw"]"#,
            r#""a@example.test""#,
            "5",
            "null",
            "true",
        ] {
            let error = refusal(body);
            assert_eq!(
                error.classify(),
                serde_json::error::Category::Data,
                "{body}: {error}"
            );
        }
    }

    #[test]
    fn the_response_names_the_step_only() {
        assert_eq!(
            serde_json::to_string(&LoginResponse {
                step: LoginStep::Done
            })
            .expect("serializes"),
            r#"{"step":"done"}"#
        );
    }

    /// An invented account's public id.
    const PID: &str = "0b9c6a1e-3f2d-4c5b-8a79-6e5d4c3b2a19";

    /// [`MeResponse::new`] for the invented account with `email` and
    /// `scope`.
    fn me(
        email: Option<&str>,
        scope: session::SessionScope,
    ) -> Result<MeResponse, MeResponseError> {
        MeResponse::new(
            Uuid::parse_str(PID).expect("a UUID"),
            "Próba Szerkesztő".to_owned(),
            email.map(str::to_owned),
            scope,
        )
    }

    #[test]
    fn the_wire_scope_is_the_domain_scope_without_mfa() {
        assert_eq!(
            SessionScope::try_from(session::SessionScope::Admin),
            Ok(SessionScope::Admin)
        );
        assert_eq!(
            SessionScope::try_from(session::SessionScope::PasswordChange),
            Ok(SessionScope::PasswordChange)
        );
        assert_eq!(
            SessionScope::try_from(session::SessionScope::Mfa),
            Err(MeResponseError::MfaScope)
        );
        // The wire spelling is the claim's, so the admin UI and the token
        // name a scope alike.
        for scope in [
            session::SessionScope::Admin,
            session::SessionScope::PasswordChange,
        ] {
            let wire = SessionScope::try_from(scope).expect("not mfa");
            assert_eq!(
                serde_json::to_value(wire).expect("serializes"),
                json!(scope.claim())
            );
        }
    }

    #[test]
    fn me_sends_the_given_columns_and_the_scope_only() {
        let admin = me(
            Some("szerkeszto@example.test"),
            session::SessionScope::Admin,
        )
        .expect("an address and a full session");
        assert_eq!(
            serde_json::to_value(&admin).expect("serializes"),
            json!({
                "pid": PID,
                "name": "Próba Szerkesztő",
                "email": "szerkeszto@example.test",
                "scope": "admin"
            })
        );
        let password_change = me(
            Some("szerkeszto@example.test"),
            session::SessionScope::PasswordChange,
        )
        .expect("an address and a password_change session");
        assert_eq!(password_change.scope, SessionScope::PasswordChange);
        assert_eq!(
            serde_json::to_value(password_change.scope).expect("serializes"),
            json!("password_change")
        );
    }

    #[test]
    fn me_refuses_an_mfa_scope_and_an_account_without_an_address() {
        assert_eq!(
            me(Some("szerkeszto@example.test"), session::SessionScope::Mfa),
            Err(MeResponseError::MfaScope)
        );
        assert_eq!(
            me(None, session::SessionScope::Admin),
            Err(MeResponseError::NoEmail)
        );
        // The scope is checked first: an mfa session names its own defect.
        assert_eq!(
            me(None, session::SessionScope::Mfa),
            Err(MeResponseError::MfaScope)
        );
        // The messages say why it cannot happen, for the 500's log line.
        assert!(MeResponseError::NoEmail.to_string().contains("anonymised"));
        assert!(
            MeResponseError::MfaScope
                .to_string()
                .contains("two-factor step")
        );
    }

    #[test]
    fn typescript_declarations() {
        let config = ts_rs::Config::default();
        assert_eq!(
            SessionScope::decl(&config),
            r#"type SessionScope = "admin" | "password_change";"#
        );
        let me = MeResponse::decl(&config);
        for field in [
            "pid: string",
            "name: string",
            "email: string",
            "scope: SessionScope",
        ] {
            assert!(me.contains(field), "{field}: {me}");
        }
        assert_eq!(LoginStep::decl(&config), r#"type LoginStep = "done";"#);
        let response = LoginResponse::decl(&config);
        assert!(response.contains("step: LoginStep"), "{response}");
        let request = LoginRequest::decl(&config);
        assert!(
            request.contains("email: string") && request.contains("password: string"),
            "{request}"
        );
    }
}
