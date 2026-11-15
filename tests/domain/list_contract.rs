//! The admin's mirror of the shared admin list contract,
//! `admin/src/data-provider/list-contract.ts`, against the server's side,
//! `domain::list_query` (parity SHR-02, SHR-03; ADR 0005).
//!
//! Why this test exists: the admin writes every list request itself (the
//! generic handlers of `admin/src/data-provider/list-handlers.ts`), so its
//! parameter names, its page sizes and its id cap must be the server's. A
//! page size the server does not offer, or a parameter it spells another
//! way, would make a list screen answer `422` for every editor. The admin
//! module keeps each constant on one line, in the form below, and this test
//! reads those lines; the admin's own tests (`list-contract.test.ts`) pin
//! what the screens build on them.

use backend3::domain::{
    labels,
    list_query::{
        DEFAULT_PER_PAGE, FilterKind, ListSpec, MAX_FILTER_IDS, ORDER_PARAM, PAGE_PARAM,
        PAGE_SIZES, PER_PAGE_PARAM, SORT_PARAM,
    },
};

use crate::support::repository_root;

/// The admin module that mirrors the contract.
const MIRROR: &str = "admin/src/data-provider/list-contract.ts";

/// The id filter that the admin's `getMany` sends, as the mirror names it.
const ID_FILTER_PARAM: &str = "id";

/// The list spec of every resource the admin lists, by resource. A slice that
/// adds a resource's list adds its spec here.
const LIST_SPECS: &[(&str, &ListSpec)] = &[("labels", &labels::LIST)];

#[test]
fn the_admin_mirrors_the_list_contract() {
    let path = repository_root().join(MIRROR);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let sizes = PAGE_SIZES.map(|size| size.to_string()).join(", ");
    for line in [
        format!("export const PAGE_PARAM = {PAGE_PARAM:?}"),
        format!("export const PER_PAGE_PARAM = {PER_PAGE_PARAM:?}"),
        format!("export const SORT_PARAM = {SORT_PARAM:?}"),
        format!("export const ORDER_PARAM = {ORDER_PARAM:?}"),
        format!("export const ID_FILTER_PARAM = {ID_FILTER_PARAM:?}"),
        format!("export const PAGE_SIZES = [{sizes}] as const"),
        format!("export const DEFAULT_PER_PAGE = {DEFAULT_PER_PAGE}"),
        format!("export const MAX_FILTER_IDS = {MAX_FILTER_IDS}"),
    ] {
        assert!(
            source.lines().any(|candidate| candidate == line),
            "{} must contain the line `{line}`, the value of domain::list_query",
            path.display()
        );
    }
}

#[test]
fn every_resource_list_takes_the_id_filter_that_get_many_sends() {
    // ra-core's getMany reads records through the list's `id` filter (recipe
    // 11 step 1 of the project skill, the list contract).
    for (resource, spec) in LIST_SPECS {
        assert_eq!(
            spec.filter(ID_FILTER_PARAM).map(|filter| filter.kind),
            Some(FilterKind::Ids),
            "{resource}: its ListSpec needs the `{ID_FILTER_PARAM}` filter of kind Ids"
        );
    }
    // getMany asks for its ids on one page of the largest size, so every
    // record they name is in the answer.
    assert_eq!(
        PAGE_SIZES.into_iter().map(usize::from).max(),
        Some(MAX_FILTER_IDS)
    );
}
