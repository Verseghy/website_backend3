/**
 * The admin's side of the shared admin list contract (parity SHR-02, SHR-03;
 * ADR 0005): how an ra-core list request becomes the query string of
 * `GET /api/admin/<resource>`, and which ids can name a record.
 *
 * Why it exists: every admin resource is listed through one endpoint shape,
 * which the server reads with `src/domain/list_query.rs` and answers with
 * `ListResponse<T>` (`src/dtos/admin/list.rs`). ra-core's list screens, its
 * pickers and its `getMany` all ask that endpoint, so the admin writes the
 * query in one place, and the generic handlers of `./list-handlers.ts` use
 * it for every resource. The constants below mirror the server's; the Rust
 * test `tests/domain/list_contract.rs` reads these lines and fails when
 * they differ from `domain::list_query`, so keep each one on its own line,
 * as written.
 *
 * The server decides what a request may ask: an unknown sort field, a page
 * size that is not offered, or a filter the resource does not take is
 * answered with a `422` whose `field` names the parameter (ADR 0021 §4).
 * So this module passes ra-core's values on as they are, and refuses only
 * what it cannot write as a query parameter at all.
 */
import type { GetListParams, Identifier } from "ra-core"

import type { MessageKey } from "@/i18n/keys"

/** The query parameter of the page number, counted from 1. */
export const PAGE_PARAM = "page"

/** The query parameter of the page size, ra-core's name for it. */
export const PER_PAGE_PARAM = "perPage"

/** The query parameter of the sort field. */
export const SORT_PARAM = "sort"

/** The query parameter of the sort direction, `ASC` or `DESC`. */
export const ORDER_PARAM = "order"

/**
 * The filter that reads records by id, repeated once per id (`id=3&id=7`):
 * every resource's list takes it, for ra-core's `getMany`.
 */
export const ID_FILTER_PARAM = "id"

/** The page sizes an admin list offers (SHR-02): no other, and no "all". */
export const PAGE_SIZES = [10, 25, 50, 100] as const

/** The page size of a list that sets none (SHR-02). */
export const DEFAULT_PER_PAGE = 25

/**
 * The most ids one request may filter on: the largest page, so every
 * record it names fits on one page of the answer.
 */
export const MAX_FILTER_IDS = 100

/**
 * The `hu.ts` key of a filter value that no query parameter can carry (a
 * nested object, a function); the server's own code for a refused filter
 * value says the same to the editor.
 */
export const INVALID_FILTER_KEY = "errors.invalid_filter" satisfies MessageKey

/**
 * A filter value that cannot be written as a query parameter, found before
 * any request is sent.
 *
 * `message` is a key of `hu.ts`, which ra-core's notification translates;
 * `field` names the filter, as the server's envelope would.
 */
export class ListFilterError extends Error {
  /** The filter whose value cannot be sent. */
  readonly field: string

  /**
   * @param field - the filter's name in ra-core's `filter` object.
   */
  constructor(field: string) {
    super(INVALID_FILTER_KEY)
    this.name = "ListFilterError"
    this.field = field
  }
}

/** A filter value that a query parameter can carry. */
type ScalarFilterValue = string | number | boolean

/**
 * Whether `value` can be written as one query parameter value.
 *
 * @param value - one value of ra-core's `filter` object, or of an array in
 *   it.
 * @returns `true` for a string, a finite number or a boolean.
 */
function isScalarFilterValue(value: unknown): value is ScalarFilterValue {
  return (
    typeof value === "string" ||
    typeof value === "boolean" ||
    (typeof value === "number" && Number.isFinite(value))
  )
}

/**
 * Appends ra-core's `filter` object to a query string, one parameter per
 * value.
 *
 * - An array is the parameter repeated once per element, in order, as the
 *   server reads a list of ids.
 * - `undefined` and `null` set no filter: ra-core clears a filter that way.
 * - A string, a finite number or a boolean is one value, written as
 *   `String` writes it.
 *
 * @param search - the query string to append to.
 * @param filter - ra-core's `filter`, of any shape (it may come from a URL
 *   that the editor changed).
 * @throws {ListFilterError} for a value that is none of the above, such as
 *   a nested object or an array inside an array.
 */
function appendFilters(search: URLSearchParams, filter: unknown): void {
  if (filter === undefined || filter === null) {
    return
  }
  if (typeof filter !== "object" || Array.isArray(filter)) {
    // ra-core always passes an object; anything else names no filter.
    throw new ListFilterError("filter")
  }
  for (const [name, value] of Object.entries(filter)) {
    if (value === undefined || value === null) {
      continue
    }
    const values: unknown[] = Array.isArray(value) ? value : [value]
    for (const item of values) {
      if (!isScalarFilterValue(item)) {
        throw new ListFilterError(name)
      }
      search.append(name, String(item))
    }
  }
}

/**
 * The query string of a list request, as the server's list contract reads
 * it (`domain::list_query::parse`).
 *
 * - `pagination` becomes `page` and `perPage`; without it the server
 *   answers its first page of {@link DEFAULT_PER_PAGE} rows.
 * - `sort` becomes `sort` and `order`; a sort without a field is left out,
 *   so the server applies the resource's own default order. The server
 *   adds the `id` tie-break itself (SHR-03).
 * - `filter` becomes one parameter per value (see `appendFilters` above).
 *
 * Values are written as ra-core holds them: a page or page size that a
 * hand-edited URL made invalid reaches the server, which answers it with
 * its own Hungarian-coded `422`.
 *
 * @param params - ra-core's list parameters.
 * @returns the query parameters, in the order above; encoded by
 *   `URLSearchParams.toString()`, which the server decodes with the
 *   `application/x-www-form-urlencoded` rules.
 * @throws {ListFilterError} when a filter value cannot be written.
 */
export function listSearchParams(params: GetListParams): URLSearchParams {
  const search = new URLSearchParams()
  const { pagination, sort, filter } = params
  if (pagination !== undefined) {
    search.append(PAGE_PARAM, String(pagination.page))
    search.append(PER_PAGE_PARAM, String(pagination.perPage))
  }
  if (sort !== undefined && sort.field !== "") {
    search.append(SORT_PARAM, sort.field)
    search.append(ORDER_PARAM, sort.order)
  }
  appendFilters(search, filter)
  return search
}

/**
 * Whether `value` can be the id of a stored record: an integer from 1 to
 * 2^53 − 1, the server's id rule (`domain::v1_params::id`, ADR 0005 §3), so
 * a JavaScript number holds it exactly.
 *
 * @param value - any value, such as a member of a JSON answer.
 * @returns `true` for such a number.
 */
export function isRecordId(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 1
}

/** ASCII digits only, the form a record id takes in an admin URL. */
const DIGITS = /^[0-9]+$/

/**
 * The record id that ra-core's `id` names, or `undefined` when it cannot
 * name a record.
 *
 * ra-core passes a record's own id (a number) or the text of a URL segment
 * (`/admin/labels/12` gives `"12"`). Only a value that {@link isRecordId}
 * accepts, or ASCII digits that spell one (leading zeros allowed, as the
 * server allows them), names a record. Anything else (`"../x"`, `"1e3"`,
 * `" 7"`, `0`) is refused here, before it could become a path segment that
 * a URL parser reads as another path.
 *
 * @param id - ra-core's identifier.
 * @returns the id as a number, or `undefined`.
 */
export function recordId(id: Identifier): number | undefined {
  if (typeof id === "number") {
    return isRecordId(id) ? id : undefined
  }
  if (!DIGITS.test(id)) {
    return undefined
  }
  const value = Number(id)
  return isRecordId(value) ? value : undefined
}
