/**
 * The generic read handlers of an admin resource: ra-core's `getList`,
 * `getOne` and `getMany` over the shared admin list contract (parity
 * SHR-02, SHR-03; ADR 0005).
 *
 * Why it exists: every admin resource is read the same way. Its list is
 * `GET /api/admin/<resource>`, which answers one page as
 * `ListResponse<T>` (`{ data, total }`, the `ListResponse` binding), and its
 * record is `GET /api/admin/<resource>/{id}`. ra-core's list screens and
 * pickers call `getList`, its show and edit pages `getOne`, and a form that
 * refers to records `getMany`, which the list endpoint answers through its
 * `id` filter. So a slice registers a resource's reads with one call,
 * {@link createListHandlers}, typed against the resource's binding, and
 * writes no request code of its own.
 *
 * What every handler does:
 *
 * - It sends a `GET` that asks for JSON through its context's
 *   `request(path, init)`, never through `fetch` itself (`admin/CLAUDE.md`,
 *   "Data provider"): the same-origin request with the session cookie
 *   (ADR 0004 §3, ADR 0006 §1) whose `401 unauthorized` waits for the
 *   in-place sign-in and is sent again after it, so the screen that asked
 *   stays as it is (ADR 0006, "Follow-up work"; research: DECISIONS R37
 *   (D158)).
 * - It passes ra-core's `params.signal` on: the provider declares
 *   `supportAbortSignal`, so a query whose screen unmounts ends its request
 *   and its wait for the sign-in. That abort rejects unchanged, as the
 *   `AbortError` that ra-core and react-query leave unreported.
 * - A refused answer (`4xx`, `5xx`) rejects with `refusalError` of
 *   `src/api/error-envelope.ts`: an ra-core `HttpError` whose message is
 *   the Hungarian `errors.<code>` of the envelope (ADR 0021 §4) and whose
 *   status `authProvider.checkError` reads, so a `401` that reaches it (the
 *   user gave up the in-place sign-in) signs out and a `403` keeps the
 *   session (ADR 0007 §5). A list the account may not see is therefore an
 *   error, never an empty list.
 * - A network failure, and an answer it cannot read (not JSON, or not the
 *   shape of the contract), rejects with the `hu.ts` key
 *   `ra.notification.http_error`, so ra-core's notification still shows
 *   Hungarian text. The body is untrusted JSON from the network: every row
 *   goes through the resource's own `parseRow` before ra-core sees it.
 *
 * Configuration arrives as explicit arguments (the resource's path and row
 * reader), never from globals; the requests go through the provider's
 * context, so the tests drive the handlers through `createDataProvider`
 * over a `fetch` stub.
 */
import { HttpError } from "ra-core"

import { basePathProblem } from "@/api/base-path"
import {
  REFUSED_WITHOUT_ENVELOPE_KEY,
  errorCodeKey,
  refusalError,
} from "@/api/error-envelope"

import {
  DataProviderConfigError,
  type DataProviderContext,
  type ResourceHandlers,
} from "./data-provider"
import {
  ID_FILTER_PARAM,
  MAX_FILTER_IDS,
  PER_PAGE_PARAM,
  isRecordId,
  listSearchParams,
  recordId,
} from "./list-contract"

/** What every record of an admin resource has: its numeric id. */
export interface ListRow {
  /** The record's id, an integer from 1 to 2^53 − 1 (ADR 0005 §3). */
  readonly id: number
}

/** One page of a list as the handlers return it to ra-core. */
export interface ListPage<Row extends ListRow> {
  /** The page's rows, in the server's order. */
  readonly data: Row[]
  /** How many rows the whole list has with its filters. */
  readonly total: number
}

/** Options of {@link createListHandlers}. */
export interface ListHandlersOptions<Row extends ListRow> {
  /**
   * The resource's path below the admin API, for example `/labels`: one or
   * more `/segment` parts, with the same characters as the API base path
   * (`src/api/base-path.ts`). The list is `<apiBaseUrl><path>` and a
   * record `<apiBaseUrl><path>/<id>`, both asked through the provider's
   * `request`.
   */
  readonly path: string
  /**
   * Reads one row of an answer: the record when `value` has the shape of
   * the resource's binding, otherwise `undefined`. It copies only the
   * fields the binding declares.
   */
  readonly parseRow: (value: unknown) => Row | undefined
}

/**
 * The rejection of an answer that arrived but cannot be read: a body that is
 * not JSON, or not the shape the contract promises (a proxy's page, a
 * defect on either side).
 *
 * `message` is the `hu.ts` key {@link REFUSED_WITHOUT_ENVELOPE_KEY}, which
 * ra-core's notification shows in Hungarian; `url` and `status` tell a
 * developer which answer it was.
 */
export class UnreadableAnswerError extends Error {
  /** The request's URL. */
  readonly url: string
  /** The answer's HTTP status. */
  readonly status: number

  /**
   * @param url - the URL that was asked.
   * @param status - the status of the answer.
   */
  constructor(url: string, status: number) {
    super(REFUSED_WITHOUT_ENVELOPE_KEY)
    this.name = "UnreadableAnswerError"
    this.url = url
    this.status = status
  }
}

/**
 * Whether `value` is a plain object (not `null`, not an array).
 *
 * @param value - any JSON value.
 * @returns `true` for an object that can hold named members.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

/**
 * Reads a body as one page of the list contract (`ListResponse<T>`).
 *
 * @param body - the parsed JSON body.
 * @param parseRow - the resource's row reader.
 * @returns the page when `data` is an array of rows that `parseRow` accepts
 *   and `total` is a whole number of rows, at least as many as `data` holds;
 *   otherwise `undefined`.
 */
export function parseListPage<Row extends ListRow>(
  body: unknown,
  parseRow: (value: unknown) => Row | undefined
): ListPage<Row> | undefined {
  if (!isRecord(body) || !Array.isArray(body.data)) {
    return undefined
  }
  const { total } = body
  if (
    typeof total !== "number" ||
    !Number.isSafeInteger(total) ||
    total < body.data.length
  ) {
    return undefined
  }
  const data: Row[] = []
  for (const value of body.data) {
    const row = parseRow(value)
    if (row === undefined) {
      return undefined
    }
    data.push(row)
  }
  return { data, total }
}

/**
 * Parses an answer's body as JSON, tolerating a body that is not.
 *
 * @param response - the answer.
 * @returns the parsed body, or `undefined` when it is empty or not JSON.
 */
async function readJson(response: Response): Promise<unknown> {
  try {
    return await response.json()
  } catch {
    return undefined
  }
}

/**
 * Splits `ids` into groups that one request may filter on.
 *
 * @param ids - the ids, each once.
 * @returns groups of at most {@link MAX_FILTER_IDS} ids, in order.
 */
function chunks(ids: readonly number[]): number[][] {
  const groups: number[][] = []
  for (let start = 0; start < ids.length; start += MAX_FILTER_IDS) {
    groups.push(ids.slice(start, start + MAX_FILTER_IDS))
  }
  return groups
}

/**
 * Checks the options and throws an actionable error for the first problem.
 *
 * @param options - the options passed to {@link createListHandlers}.
 * @throws {DataProviderConfigError} when `path` is not a root-relative
 *   path, or `parseRow` is not a function.
 */
function validateOptions<Row extends ListRow>(
  options: ListHandlersOptions<Row>
): void {
  // The same rule as the API base path's: the resource path is appended to
  // it, so it must not be able to leave the admin's origin either.
  const problem = basePathProblem("path", options.path)
  if (problem !== undefined) {
    throw new DataProviderConfigError(problem)
  }
  if (typeof options.parseRow !== "function") {
    throw new DataProviderConfigError(
      `parseRow of the handlers for ${options.path} must be a function that reads one row of the resource's binding.`
    )
  }
}

/**
 * Builds the `getList`, `getOne` and `getMany` handlers of one resource.
 *
 * - `getList` asks `<path>?page=…&perPage=…&sort=…&order=…&<filters>`
 *   (`listSearchParams` of `./list-contract.ts`) and returns the page's
 *   rows with the list's total.
 * - `getOne` asks `<path>/<id>` and returns the record. An id that cannot
 *   name a record (`recordId`) rejects with the Hungarian `not_found`
 *   message and status `404`, as the server would answer, without a
 *   request.
 * - `getMany` asks the list with the `id` filter, the ids once each and at
 *   most {@link MAX_FILTER_IDS} to a request, on one page of that size, so
 *   every record they name is in the answer. An id that cannot name a
 *   record names none, so it is left out; with no id left there is no
 *   request. A record the server does not return (missing or in the
 *   trash) is simply absent from the result, as ra-core expects.
 *
 * Each sends its requests through the context's `request` with ra-core's
 * `params.signal`, so a `401 unauthorized` waits for the in-place sign-in
 * and an unmounted screen's query ends (see the module documentation).
 *
 * @param options - the resource's path and its row reader; see
 *   {@link ListHandlersOptions}.
 * @returns the three handlers, for `createDataProvider`'s `resources`.
 * @throws {DataProviderConfigError} when the options are invalid.
 */
export function createListHandlers<Row extends ListRow>(
  options: ListHandlersOptions<Row>
): ResourceHandlers {
  validateOptions(options)
  const { path, parseRow } = options

  /**
   * Sends one `GET` through the provider and reads its JSON body.
   *
   * @param context - the provider's context, whose `request` sends it.
   * @param target - the path below the API base path, with any query, for
   *   example `/labels?page=2`.
   * @param signal - ra-core's abort signal of the query, if it passed one.
   * @returns the answer's status and parsed body.
   * @throws {Error} with `ra.notification.http_error` on a network failure.
   * @throws {HttpError} from `refusalError` for a refused answer, a `401`
   *   included when the user gave up the in-place sign-in.
   * Rejects unchanged when `signal` ended the request or its wait for the
   * sign-in (an `AbortError`, which ra-core does not report).
   */
  const get = async (
    context: DataProviderContext,
    target: string,
    signal: AbortSignal | undefined
  ): Promise<{ status: number; body: unknown }> => {
    let response: Response
    try {
      response = await context.request(target, {
        method: "GET",
        headers: { Accept: "application/json" },
        ...(signal === undefined ? {} : { signal }),
      })
    } catch (error) {
      if (signal?.aborted === true) {
        // The screen that asked is gone: react-query dropped the query.
        // Reporting a server problem for it would be wrong, and ra-core
        // stays quiet only for the abort itself.
        throw error
      }
      // No answer at all: ra-core shows the message, and no status reaches
      // checkError, so the session is left alone.
      throw new Error(REFUSED_WITHOUT_ENVELOPE_KEY, { cause: error })
    }
    const body = await readJson(response)
    // A signal that fired while the body was read ends the body too, and
    // `readJson` would take that for a body that is not JSON: the abort is
    // still the answer, not an unreadable page.
    signal?.throwIfAborted()
    if (!response.ok) {
      throw refusalError(response.status, body)
    }
    return { status: response.status, body }
  }

  /**
   * Asks the list endpoint for one page.
   *
   * @param context - the provider's context.
   * @param search - the query string.
   * @param signal - ra-core's abort signal of the query, if any.
   * @returns the page.
   * @throws as `get` does, and {@link UnreadableAnswerError} for a body
   *   that is not a page.
   */
  const page = async (
    context: DataProviderContext,
    search: URLSearchParams,
    signal: AbortSignal | undefined
  ): Promise<ListPage<Row>> => {
    const query = search.toString()
    const target = `${path}${query === "" ? "" : `?${query}`}`
    const { status, body } = await get(context, target, signal)
    const parsed = parseListPage(body, parseRow)
    if (parsed === undefined) {
      throw new UnreadableAnswerError(`${context.apiBaseUrl}${target}`, status)
    }
    return parsed
  }

  return {
    async getList(params, context) {
      const { data, total } = await page(
        context,
        listSearchParams(params),
        params.signal
      )
      return { data, total }
    },

    async getOne(params, context) {
      const id = recordId(params.id)
      if (id === undefined) {
        throw new HttpError(errorCodeKey("not_found"), 404, null)
      }
      const target = `${path}/${id}`
      const { status, body } = await get(context, target, params.signal)
      const record = parseRow(body)
      // The record of another id would be a defect on one side; showing it
      // under this id would be worse than an error.
      if (record === undefined || record.id !== id) {
        throw new UnreadableAnswerError(
          `${context.apiBaseUrl}${target}`,
          status
        )
      }
      return { data: record }
    },

    async getMany(params, context) {
      const ids = [
        ...new Set(
          params.ids
            .map((id) => recordId(id))
            .filter((id): id is number => isRecordId(id))
        ),
      ]
      const pages = await Promise.all(
        chunks(ids).map((group) => {
          const search = new URLSearchParams()
          search.append(PER_PAGE_PARAM, String(MAX_FILTER_IDS))
          for (const id of group) {
            search.append(ID_FILTER_PARAM, String(id))
          }
          return page(context, search, params.signal)
        })
      )
      return { data: pages.flatMap((answer) => answer.data) }
    },
  }
}
