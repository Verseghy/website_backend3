//! The controller half of the `Origin` check of cookie-authenticated admin
//! writes (ADR 0006 §3, as amended by ADR 0044): the first step of the admin
//! route table's guard (`super::routes`), which runs for every route of the
//! table, before the guard opens a session and before the handler.
//!
//! [`refusal`] reads from the request what the pure rules of
//! [`crate::domain::origin`] take, and answers a refused write with
//! `403 origin_mismatch`:
//!
//! - whether the request carries a credential cookie
//!   ([`carries_credential_cookie`]), read with Loco's own cookie reader
//!   (`extract_token_from_cookie`), the one `JWTWithUser` uses for
//!   `auth.jwt.location`. Any cookie that Loco would take a token from is
//!   therefore seen here, however the `Cookie` header spells it (beside
//!   other cookies, in a second `Cookie` header, percent-encoded);
//! - the method, and the values of every `Origin` header;
//! - the admin origin, `settings.admin_auth.origin`, which
//!   `App::after_context` puts into the shared store ([`set_admin_origin`])
//!   and [`admin_origin`] reads back for each checked request.
//!
//! The check runs before the session is opened, so a refused write costs no
//! database read and gets the same answer whether its session is valid or
//! not: a page on another origin learns nothing about the session from it.
//! It is a step of the guard rather than a tower layer on `/api/admin`. A
//! layer could read the admin origin from the shared store on each request
//! as well (Loco runs `App::after_context` before `App::routes`), so the
//! reasons are where the step runs: the guard wraps the handler of each
//! route of the table, after routing, so the check covers exactly the
//! table's routes, the `Public` ones included, while an unknown path keeps
//! its `404`; and its place before the session is written down in
//! `Guard::admit`, beside the guard's other steps.

use axum::{
    http::{HeaderValue, header::ORIGIN, request::Parts},
    response::{IntoResponse as _, Response},
};
use loco_rs::{
    Error, Result,
    app::{AppContext, SharedStore},
    controller::extractor::auth::extract_token_from_cookie,
};

use super::error_response;
use crate::{
    domain::origin::{AdminOrigin, CREDENTIAL_COOKIES, check_origin, is_checked},
    dtos::admin::error::ErrorCode,
};

/// The shared-store entry of the admin origin. `SharedStore` is keyed by
/// type, and this one is private, so only [`admin_origin`] and
/// [`set_admin_origin`] reach it.
#[derive(Debug, Clone)]
struct ConfiguredOrigin(AdminOrigin);

/// The most characters of a refused `Origin` value that the log line shows.
/// The value comes from the client, so the line stays bounded whatever it
/// sends; a browser's origin is far shorter.
const SHOWN_ORIGIN_CHARS: usize = 120;

/// Stores `origin` as the admin origin of this app context, replacing any
/// origin stored before.
///
/// - `ctx`: the context; every copy of it sees the new origin, including the
///   copies the router took at boot (they share one `SharedStore` through an
///   `Arc`).
/// - `origin`: `settings.admin_auth.origin`.
///
/// `App::after_context` calls it at boot; a test may call it to check
/// another origin, such as production's.
pub fn set_admin_origin(ctx: &AppContext, origin: AdminOrigin) {
    ctx.shared_store.insert(ConfiguredOrigin(origin));
}

/// The admin origin of this app context.
///
/// - `ctx`: the request's app context.
///
/// # Errors
///
/// [`Error::Message`] when the context carries none: every context Loco
/// builds has it (`App::after_context`), one built by hand does not.
pub fn admin_origin(ctx: &AppContext) -> Result<AdminOrigin> {
    admin_origin_in(&ctx.shared_store)
}

/// The admin origin held in `store`, or the error [`admin_origin`]
/// documents.
fn admin_origin_in(store: &SharedStore) -> Result<AdminOrigin> {
    store
        .get::<ConfiguredOrigin>()
        .map(|ConfiguredOrigin(origin)| origin)
        .ok_or_else(|| {
            Error::string(
                "the admin origin is missing from the shared store; App::after_context \
                 registers it from settings.admin_auth.origin, so this context was not built by \
                 App::boot",
            )
        })
}

/// Whether the request carries one of the credential cookies of
/// [`CREDENTIAL_COOKIES`], with any value.
///
/// - `parts`: the request's head.
///
/// Read with Loco's `extract_token_from_cookie`, so a cookie counts exactly
/// when Loco, or the two-factor step reading its own cookie the same way,
/// would find it. A Bearer header does not matter: with the session cookie
/// beside it, Loco reads the token from the cookie (`auth.jwt.location`).
#[must_use]
pub fn carries_credential_cookie(parts: &Parts) -> bool {
    CREDENTIAL_COOKIES
        .iter()
        .any(|name| extract_token_from_cookie(name, parts).is_ok())
}

/// The answer to a request that the `Origin` check refuses, or `None` to
/// let it on to the rest of the guard.
///
/// - `parts`: the request's head.
/// - `ctx`: the request's app context, for the admin origin.
///
/// A read, or a request without a credential cookie, is let through without
/// a look at the admin origin. A checked write gets `403 origin_mismatch`
/// unless it carries exactly one `Origin` header with the admin origin; a
/// context without an admin origin answers Loco's `500`, which logs why,
/// and lets no checked write through.
pub(super) fn refusal(parts: &Parts, ctx: &AppContext) -> Option<Response> {
    refusal_in(parts, &ctx.shared_store)
}

/// [`refusal`], with the admin origin read from `store`: the unit tests
/// call it with a store of their own, since an `AppContext` needs a
/// database.
fn refusal_in(parts: &Parts, store: &SharedStore) -> Option<Response> {
    if !is_checked(parts.method.as_str(), carries_credential_cookie(parts)) {
        return None;
    }
    let admin = match admin_origin_in(store) {
        Ok(admin) => admin,
        Err(error) => {
            tracing::error!(%error, "admin write refused: the admin origin is not configured");
            return Some(Error::InternalServerError.into_response());
        }
    };
    let origins = parts.headers.get_all(ORIGIN);
    let refusal = check_origin(origins.iter().map(HeaderValue::as_bytes), &admin).err()?;
    let shown: String = origins.iter().next().map_or_else(String::new, |value| {
        String::from_utf8_lossy(value.as_bytes())
            .chars()
            .take(SHOWN_ORIGIN_CHARS)
            .collect()
    });
    // WARN: the admin's own writes always carry its origin, so a refusal is
    // a page on another origin, a client or browser extension that drops
    // the header, or an admin origin that does not match where the admin is
    // served (HOST in production), which refuses every write and must be
    // seen at once.
    tracing::warn!(
        method = %parts.method,
        path = parts.uri.path(),
        %refusal,
        origin = shown,
        expected = admin.as_str(),
        "admin write refused: a cookie-authenticated write without the admin origin"
    );
    Some(error_response(ErrorCode::OriginMismatch))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fmt,
        sync::{Arc, Mutex},
    };

    use axum::http::{
        Request, StatusCode,
        header::{AUTHORIZATION, COOKIE},
    };
    use tracing::{
        Event, Level, Metadata, Subscriber,
        field::{Field, Visit},
        span::{Attributes, Id, Record},
    };

    use super::*;
    use crate::domain::origin::OriginRefusal;

    /// The admin origin of the log tests' store.
    const ADMIN: &str = "https://backend3.example.org";

    /// The head of a request with these headers.
    fn parts(headers: &[(&str, &str)]) -> Parts {
        let mut request = Request::post("/api/admin/auth/logout");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(()).expect("request").into_parts().0
    }

    /// A shared store that holds [`ADMIN`] as the admin origin.
    fn store() -> SharedStore {
        let store = SharedStore::default();
        store.insert(ConfiguredOrigin(
            AdminOrigin::parse(ADMIN).expect("an origin"),
        ));
        store
    }

    /// One event that a [`CapturedLog`] saw: its level and each of its
    /// fields as text (`message` included).
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Captured {
        /// The event's level.
        level: Level,
        /// The event's fields by name.
        fields: BTreeMap<String, String>,
    }

    /// A subscriber that keeps every event it sees. A test sets it for its
    /// own thread only (`tracing::subscriber::with_default`), so the events
    /// of tests on other threads never reach it.
    #[derive(Debug, Clone, Default)]
    struct CapturedLog(Arc<Mutex<Vec<Captured>>>);

    impl CapturedLog {
        /// The events seen so far.
        fn events(&self) -> Vec<Captured> {
            self.0.lock().expect("the log").clone()
        }
    }

    /// Writes each field of an event into a map, as text: a `%` field
    /// arrives as its `Display` text, a string as itself.
    struct Fields<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for Fields<'_> {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    impl Subscriber for CapturedLog {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut fields = BTreeMap::new();
            event.record(&mut Fields(&mut fields));
            self.0.lock().expect("the log").push(Captured {
                level: *event.metadata().level(),
                fields,
            });
        }

        fn enter(&self, _: &Id) {}

        fn exit(&self, _: &Id) {}
    }

    /// [`refusal_in`] of `parts` against [`store`], with every event it
    /// logs.
    fn refuse_and_log(parts: &Parts, store: &SharedStore) -> (Option<Response>, Vec<Captured>) {
        let log = CapturedLog::default();
        let answer = tracing::subscriber::with_default(log.clone(), || refusal_in(parts, store));
        (answer, log.events())
    }

    /// The `WARN` event that ADR 0044 §5 specifies for a refusal.
    fn warning(refusal: OriginRefusal, origin: &str) -> Captured {
        let refusal = refusal.to_string();
        Captured {
            level: Level::WARN,
            fields: [
                (
                    "message",
                    "admin write refused: a cookie-authenticated write without the admin origin",
                ),
                ("method", "POST"),
                ("path", "/api/admin/auth/logout"),
                ("refusal", refusal.as_str()),
                ("origin", origin),
                ("expected", ADMIN),
            ]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        }
    }

    #[test]
    fn each_refusal_is_logged_at_warn_with_its_reason_and_both_origins() {
        let cookie = (COOKIE.as_str(), "__Host-admin_session=a.b.c");
        let foreign = "https://verseghy-gimnazium.net";
        for (case, headers, expected) in [
            (
                "another origin",
                vec![cookie, (ORIGIN.as_str(), foreign)],
                warning(OriginRefusal::Foreign, foreign),
            ),
            (
                "no Origin header",
                vec![cookie],
                warning(OriginRefusal::Missing, ""),
            ),
            (
                // The line shows the first of the two.
                "two Origin headers",
                vec![cookie, (ORIGIN.as_str(), foreign), (ORIGIN.as_str(), ADMIN)],
                warning(OriginRefusal::Repeated, foreign),
            ),
        ] {
            let (answer, events) = refuse_and_log(&parts(&headers), &store());
            assert_eq!(
                answer.map(|answer| answer.status()),
                Some(StatusCode::FORBIDDEN),
                "{case}"
            );
            assert_eq!(events, [expected], "{case}");
        }
    }

    #[test]
    fn the_logged_origin_is_cut_to_120_characters() {
        let cookie = (COOKIE.as_str(), "__Host-admin_session=a.b.c");
        let long = format!("https://{}", "a".repeat(492));
        let (_, events) = refuse_and_log(
            &parts(&[cookie, (ORIGIN.as_str(), long.as_str())]),
            &store(),
        );
        assert_eq!(
            events,
            [warning(OriginRefusal::Foreign, &long[..SHOWN_ORIGIN_CHARS])]
        );
        assert_eq!(SHOWN_ORIGIN_CHARS, 120, "ADR 0044 §5");

        // Characters, not bytes: bytes that are no UTF-8 are shown as
        // U+FFFD, three bytes each, and still 120 of them at most.
        let mut request = parts(&[cookie]);
        request.headers.insert(
            ORIGIN,
            HeaderValue::from_bytes(&[0xE9; 200]).expect("obs-text is a header value"),
        );
        let (_, events) = refuse_and_log(&request, &store());
        assert_eq!(
            events,
            [warning(
                OriginRefusal::Foreign,
                &"\u{FFFD}".repeat(SHOWN_ORIGIN_CHARS)
            )]
        );
    }

    #[test]
    fn a_write_let_through_and_an_unchecked_request_log_nothing() {
        let cookie = (COOKIE.as_str(), "__Host-admin_session=a.b.c");
        for headers in [
            // The admin's own write.
            vec![cookie, (ORIGIN.as_str(), ADMIN)],
            // No credential cookie: not checked.
            vec![(ORIGIN.as_str(), "https://verseghy-gimnazium.net")],
        ] {
            let (answer, events) = refuse_and_log(&parts(&headers), &store());
            assert!(answer.is_none(), "{headers:?}");
            assert_eq!(events, [], "{headers:?}");
        }
    }

    #[test]
    fn a_store_without_an_origin_refuses_a_checked_write_with_500_and_an_error_line() {
        let cookie = (COOKIE.as_str(), "__Host-admin_session=a.b.c");
        let (answer, events) = refuse_and_log(
            &parts(&[cookie, (ORIGIN.as_str(), ADMIN)]),
            &SharedStore::default(),
        );
        assert_eq!(
            answer.map(|answer| answer.status()),
            Some(StatusCode::INTERNAL_SERVER_ERROR)
        );
        let levels: Vec<Level> = events.iter().map(|event| event.level).collect();
        assert!(levels.contains(&Level::ERROR), "{events:?}");
        assert!(!levels.contains(&Level::WARN), "{events:?}");
    }

    #[test]
    fn a_store_without_an_origin_says_how_to_get_one() {
        let error = admin_origin_in(&SharedStore::default())
            .expect_err("an empty store has no origin")
            .to_string();
        assert!(error.contains("App::after_context"), "{error}");
        assert!(error.contains("settings.admin_auth.origin"), "{error}");
    }

    #[test]
    fn the_last_origin_stored_is_the_one_returned() {
        let store = SharedStore::default();
        for text in ["http://localhost:5150", "https://backend3.example.org"] {
            store.insert(ConfiguredOrigin(
                AdminOrigin::parse(text).expect("an origin"),
            ));
            assert_eq!(admin_origin_in(&store).expect("stored").as_str(), text);
        }
    }

    #[test]
    fn either_credential_cookie_counts_however_the_header_spells_it() {
        for cookie in [
            "__Host-admin_session=a.b.c",
            "__Host-admin_mfa=a.b.c",
            "theme=dark; __Host-admin_session=a.b.c",
            "theme=dark;__Host-admin_session=a.b.c; lang=hu",
            " __Host-admin_session = a.b.c ",
            // Loco's reader decodes percent-encoded names and values.
            "__Host-admin%5Fsession=a.b.c",
            // An empty value is still the cookie: Loco would read it and
            // refuse the token, so the write is checked.
            "__Host-admin_session=",
        ] {
            assert!(
                carries_credential_cookie(&parts(&[(COOKIE.as_str(), cookie)])),
                "{cookie:?}"
            );
        }
        // A second `Cookie` header, as HTTP/2 clients may send.
        assert!(carries_credential_cookie(&parts(&[
            (COOKIE.as_str(), "theme=dark"),
            (COOKIE.as_str(), "__Host-admin_session=a.b.c"),
        ])));
    }

    #[test]
    fn other_cookies_and_a_bearer_token_alone_are_not_credential_cookies() {
        for headers in [
            vec![],
            vec![(COOKIE.as_str(), "theme=dark")],
            // The names without the prefix, as an older draft spelt them.
            vec![(COOKIE.as_str(), "admin_session=a.b.c; admin_mfa=a.b.c")],
            vec![(COOKIE.as_str(), "__host-admin_session=a.b.c")],
            vec![(COOKIE.as_str(), "__Host-admin_session_old=a.b.c")],
            // No `=`: not a cookie pair, and Loco reads nothing from it.
            vec![(COOKIE.as_str(), "__Host-admin_session")],
            vec![(AUTHORIZATION.as_str(), "Bearer a.b.c")],
            vec![
                (COOKIE.as_str(), "theme=dark"),
                (AUTHORIZATION.as_str(), "Bearer a.b.c"),
            ],
        ] {
            assert!(!carries_credential_cookie(&parts(&headers)), "{headers:?}");
        }
    }
}
