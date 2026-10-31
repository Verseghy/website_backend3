/**
 * The URL-query cases of `src/ra-core-query-string.test.ts`.
 *
 * Why a module of its own: that test runs every case twice, once in the
 * module graph Node builds from node_modules and once inside a production
 * Vite bundle of the admin, because the two resolve and interoperate with
 * CommonJS packages independently. This module is the bundle's entry, so both
 * runs execute the same code, and the expected values live next to the inputs.
 *
 * Each case calls a ra-core helper that the admin's screens rely on: a list
 * screen keeps its filter, sort, order and page in the URL
 * (`parseQueryFromLocation`, written with the same `stringify` as
 * `fetchUtils.queryParameters`), and a create screen pre-fills its record from
 * `?source=` (`getRecordFromLocation`).
 */
import {
  fetchUtils,
  getRecordFromLocation,
  parseQueryFromLocation,
} from "ra-core"

/** One case: the call to make and the value it must return. */
export interface QueryCase {
  /** Calls the ra-core helper under test with a fixed input. */
  readonly run: () => unknown
  /** What `run` returns when ra-core's query parsing works. */
  readonly expected: unknown
}

/** A case's result, as {@link runQueryCases} reports it. */
export type QueryOutcome =
  { readonly value: unknown } | { readonly error: string }

/** The filter that the round-trip case writes and reads back. */
const ROUND_TRIP_FILTER = { q: "a&b=c ő", published: true }

/** The cases, by name. */
export const QUERY_CASES = {
  "decodes a UTF-8 filter with its sort, order and page": {
    // {"q":"árvíztűrő"}, percent-encoded as UTF-8.
    run: () =>
      parseQueryFromLocation({
        search:
          "?filter=%7B%22q%22%3A%22%C3%A1rv%C3%ADzt%C5%B1r%C5%91%22%7D&order=ASC&page=2&sort=title",
      }),
    expected: {
      filter: { q: "árvíztűrő" },
      order: "ASC",
      page: "2",
      sort: "title",
    },
  },
  "reads back what ra-core's own stringify writes": {
    // useListParams writes the URL with the same query-string `stringify` as
    // `fetchUtils.queryParameters`; `&`, `=` and spaces must survive the trip.
    run: () =>
      parseQueryFromLocation({
        search: `?${fetchUtils.queryParameters({
          filter: JSON.stringify(ROUND_TRIP_FILTER),
          page: 3,
        })}`,
      }),
    expected: { filter: ROUND_TRIP_FILTER, page: "3" },
  },
  "keeps an undecodable escape as it is instead of throwing": {
    // `%E0%A4%A` is a truncated UTF-8 sequence, which `decodeURIComponent`
    // rejects. decode-uri-component falls back to decoding what it can and
    // leaves the rest literal, so a mistyped link still opens the list. A
    // decoder that throws here would crash the screen; one that writes U+FFFD
    // instead (as query-string 10 does) is a deliberate change of this case.
    run: () =>
      parseQueryFromLocation({
        search: "?filter=%7B%22q%22%3A%22a%E0%A4%A%22%7D",
      }),
    expected: { filter: { q: "a%E0%A4%A" } },
  },
  "pre-fills a create form from a percent-encoded source": {
    // {"title":"Étel"}. getRecordFromLocation logs and returns null on any
    // parse failure, so a broken decoder shows up as a null record (and, in
    // the Node run, the shared setup fails the test on the console.error).
    run: () =>
      getRecordFromLocation({
        pathname: "/admin/posts/create",
        search: "?source=%7B%22title%22%3A%22%C3%89tel%22%7D",
        hash: "",
      }),
    expected: { title: "Étel" },
  },
} as const satisfies Record<string, QueryCase>

/** A case name of {@link QUERY_CASES}. */
export type QueryCaseName = keyof typeof QUERY_CASES

/**
 * Runs every case and catches what each one throws, so that one broken case
 * does not hide the others.
 *
 * @returns each case's returned value, or the message of what it threw.
 */
export function runQueryCases(): Record<QueryCaseName, QueryOutcome> {
  const outcomes = {} as Record<QueryCaseName, QueryOutcome>
  for (const name of Object.keys(QUERY_CASES) as QueryCaseName[]) {
    try {
      outcomes[name] = { value: QUERY_CASES[name].run() }
    } catch (error) {
      outcomes[name] = {
        error:
          error instanceof Error
            ? `${error.name}: ${error.message}`
            : String(error),
      }
    }
  }
  return outcomes
}
