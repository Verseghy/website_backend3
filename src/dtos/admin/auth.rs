//! Wire types of the browser sign-in, `POST /api/admin/auth/login`
//! (ADR 0006 §4).
//!
//! The response names the next step of the sign-in and never carries the
//! session token: the token travels only in the `HttpOnly` session cookie
//! (ADR 0006 §3, "The browser login endpoint never returns the token in the
//! response body").

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor, value::MapAccessDeserializer},
};
use ts_rs::TS;

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

    #[test]
    fn typescript_declarations() {
        let config = ts_rs::Config::default();
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
