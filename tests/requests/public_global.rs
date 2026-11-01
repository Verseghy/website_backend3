//! The global rules of `/api/v1` (public-api.md §16.3), checked for every
//! route of Loco's route table with the samples registered for it in
//! `tests/support/public_samples.rs`.
//!
//! - `every_public_route_has_registered_samples` fails for a route without
//!   samples, for samples without a route, and for samples that leave out a
//!   path parameter or a documented query parameter, so a route that a
//!   later slice adds cannot skip the rest.
//! - `every_public_route_follows_the_global_rules` loads every fixture the
//!   samples name, at the fixed instant `SUITE_NOW`, and sends each route
//!   the requests of `tests/support/public_rules.rs`: the trailing-slash
//!   form, `POST`, unknown and repeated parameters, `+`, `%20` and `%2B`,
//!   and the revalidation, checking every answer for CORS, the JSON error
//!   envelope, the caching headers, the declared schema and the soft-deleted
//!   twins. It also fails when a route's samples cannot show a rule: a base
//!   request that does not get a cacheable `200` listing a twinned row, or
//!   values that the route answers alike.
//! - `no_value_of_any_parameter_makes_a_public_route_fail` is the property
//!   test of §16.3: random strings in every parameter, and in an unknown
//!   one, never get a `5xx`.
//! - `every_schema_requires_every_field`: every field of every schema is
//!   present in every response (§2.1), so the document requires each.
//!
//! `OPTIONS` and the other methods are checked for every route by
//! `tests/requests/public_router.rs`, and the caching rules in depth by
//! `tests/requests/public_caching.rs`. The rule checks themselves are
//! tested against faulty probe routes in `tests/requests/public_harness.rs`.

use chrono::{DateTime, Utc};

use crate::support::{
    fixtures::{self, fixture_names},
    openapi_schema::{served_document, unrequired_properties},
    public_routes,
    public_rules::{random_input_failures, route_violations},
    public_samples::{ROUTE_SAMPLES, SUITE_NOW, registry_gaps},
    with_server, with_server_at,
};

/// How many random cases the property test sends per route. Every request
/// goes through the whole middleware stack and may query the database, so
/// the count stays well below proptest's default of 256 per route.
const RANDOM_CASES: u32 = 96;

/// [`SUITE_NOW`] as an instant.
fn suite_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(SUITE_NOW)
        .expect("SUITE_NOW is RFC 3339")
        .with_timezone(&Utc)
}

/// Every fixture any route's samples name, each once, in registry order.
fn sample_fixtures() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for samples in ROUTE_SAMPLES {
        for fixture in samples.fixtures {
            if !names.contains(fixture) {
                names.push(fixture);
            }
        }
    }
    names
}

#[tokio::test]
async fn every_public_route_has_registered_samples() {
    with_server(|server, ctx| async move {
        let document = served_document(&server).await;
        let gaps = registry_gaps(
            &public_routes(&ctx),
            &document,
            ROUTE_SAMPLES,
            &fixture_names(),
        );
        assert!(
            gaps.is_empty(),
            "the samples of the global /api/v1 suite do not match the routes:\n{}",
            gaps.join("\n")
        );
    })
    .await;
}

#[tokio::test]
async fn every_public_route_follows_the_global_rules() {
    with_server_at(suite_now(), |server, ctx, _clock| async move {
        let loaded = fixtures::load(&ctx.db, &sample_fixtures()).await;
        let document = served_document(&server).await;
        let mut violations = Vec::new();
        for samples in ROUTE_SAMPLES {
            violations.extend(route_violations(&server, &document, samples, &loaded).await);
        }
        assert!(
            violations.is_empty(),
            "the global rules of public-api.md §16.3 are broken:\n{}",
            violations.join("\n")
        );
    })
    .await;
}

// `random_input_failures` runs each request on this runtime from inside
// proptest's runner, through `block_in_place`, which needs the multi-thread
// runtime.
#[tokio::test(flavor = "multi_thread")]
async fn no_value_of_any_parameter_makes_a_public_route_fail() {
    with_server_at(suite_now(), |server, ctx, _clock| async move {
        fixtures::load(&ctx.db, &sample_fixtures()).await;
        let document = served_document(&server).await;
        let failures: Vec<String> = ROUTE_SAMPLES
            .iter()
            .filter_map(|samples| {
                random_input_failures(&server, &document, samples, RANDOM_CASES).err()
            })
            .collect();
        assert!(
            failures.is_empty(),
            "user input never produces a 5xx (public-api.md §8, §16.3):\n{}",
            failures.join("\n")
        );
    })
    .await;
}

#[tokio::test]
async fn every_schema_requires_every_field() {
    with_server(|server, _ctx| async move {
        let document = served_document(&server).await;
        let missing = unrequired_properties(&document);
        assert!(
            missing.is_empty(),
            "every field of a public schema is always present (public-api.md §2.1), so it is \
             required; add #[schema(required = true)] to these Option fields \
             (docs/design/openapi-spike.md §3, rule 1):\n{}",
            missing.join("\n")
        );
    })
    .await;
}
