/**
 * backend3's hand-written ra-core data provider (ADR 0004 §1).
 *
 * Why hand-written: backend3's admin API (`/api/admin`) has its own paging,
 * filter, error and concurrency contracts (ADR 0005, ADR 0018, ADR 0021 §4),
 * and the generic REST providers (`ra-data-json-server`, `ra-data-simple-rest`)
 * would hide them. Those packages are never installed.
 *
 * Shape: the provider is a dispatch table. Each vertical slice registers
 * `ResourceHandlers` for its resource, typed against the ts-rs bindings in
 * `src/bindings/`. Until a slice registers a method, calling it rejects with
 * {@link DataProviderNotImplementedError}, whose message is a key of `hu.ts`,
 * so ra-core's error notification shows Hungarian text.
 *
 * Every handler talks to the API through {@link DataProviderContext.request},
 * never through `fetch` itself. That is the data provider's `401` handler
 * (ADR 0006, "Follow-up work"): a request whose session has ended waits for
 * the layout's in-place sign-in dialog and is sent again, unchanged, after
 * the sign-in, so the form that started it stays open with what the user
 * typed (`src/auth/session-expiry.ts`). A handler sees a `401 unauthorized`
 * only when the user gave up, and rejects it with `refusalError` like any
 * refusal; ra-core's `checkError` then signs out. A `401` with another code
 * never waits: it is the request's own answer.
 *
 * The provider declares `supportAbortSignal`, so ra-core's query hooks pass
 * react-query's `signal` in a query's params. A handler passes it on in
 * `request`'s options: it cancels the request, and its wait for the
 * sign-in, when the screen that asked unmounts. ra-core gives mutations
 * (`create`, `update`, `delete`) no signal.
 *
 * Configuration arrives as explicit arguments ({@link DataProviderOptions}),
 * never from globals, so tests build a provider against stubs.
 */
import type { DataProvider } from "ra-core"

import { basePathProblem } from "@/api/base-path"
import { sendWithSignIn, type SessionExpiry } from "@/auth/session-expiry"
import type { MessageKey } from "@/i18n/keys"

/** The ra-core data provider methods backend3 implements, in ra-core's order. */
export const DATA_PROVIDER_METHODS = [
  "getList",
  "getOne",
  "getMany",
  "getManyReference",
  "create",
  "update",
  "updateMany",
  "delete",
  "deleteMany",
] as const

/** One of {@link DATA_PROVIDER_METHODS}. */
export type DataProviderMethod = (typeof DATA_PROVIDER_METHODS)[number]

/**
 * A request body that can be sent a second time, after the in-place
 * sign-in: everything `fetch` takes but a stream, which can be read once.
 */
export type ResendableBody =
  | string
  | Blob
  | ArrayBuffer
  | ArrayBufferView<ArrayBuffer>
  | FormData
  | URLSearchParams

/**
 * The options of one admin API request: `fetch`'s, with a body that can be
 * sent again. `credentials` is always `same-origin` (the session cookie).
 */
export type ApiRequestInit = Omit<RequestInit, "body" | "credentials"> & {
  /** The request body, if the method has one. */
  readonly body?: ResendableBody | null
}

/** What every handler receives besides ra-core's parameters. */
export interface DataProviderContext {
  /** The same-origin base path of the admin API, for example `/api/admin`. */
  readonly apiBaseUrl: string
  /**
   * Sends one request to the admin API, with the session cookie.
   *
   * @param path - the path below `apiBaseUrl`, starting with `/`, with any
   *   query, for example `/labels?page=1`.
   * @param init - `fetch`'s options, without `credentials`; its `signal`
   *   (ra-core's `params.signal` for a query) also ends a wait for the
   *   sign-in.
   * @returns the first answer that does not say the session ended: a
   *   `401 unauthorized` waits for the in-place sign-in, and the request is
   *   sent again after it. That `401` itself comes back only when the user
   *   gave up or no dialog is mounted; the handler rejects it with
   *   `refusalError`, and `checkError` signs out. A `401` with another code
   *   comes back at once.
   * @throws {TypeError} when `path` does not start with `/` or the body is a
   *   stream, which could not be sent again.
   * Rejects with `fetch`'s rejection (a network failure, an abort).
   */
  readonly request: (path: string, init?: ApiRequestInit) => Promise<Response>
}

/**
 * The handlers a slice registers for one resource. Each takes ra-core's
 * parameters for that method plus the {@link DataProviderContext}, and returns
 * what ra-core expects from the method.
 */
export type ResourceHandlers = {
  readonly [Method in DataProviderMethod]?: (
    params: Parameters<DataProvider[Method]>[1],
    context: DataProviderContext
  ) => ReturnType<DataProvider[Method]>
}

/** Options of {@link createDataProvider}. */
export interface DataProviderOptions {
  /**
   * Root-relative base path of the admin API: one or more `/segment` parts made
   * of letters, digits and `-._~` only, with no trailing `/`. The admin is
   * served from the same origin (ADR 0004 §3), so an absolute URL would break
   * the `SameSite=Strict` session cookie. The narrow character set also rules
   * out every spelling that a URL parser reads as another host, such as `//host`
   * or `/\host` (WHATWG URL parsing treats `\` like `/`).
   */
  readonly apiBaseUrl: string
  /** Handlers per ra-core resource name. Empty until the first slice. */
  readonly resources: Readonly<Record<string, ResourceHandlers>>
  /**
   * The `fetch` that {@link DataProviderContext.request} calls. `main.tsx`
   * passes the browser's, bound to `window`; tests pass a stub.
   */
  readonly fetch: typeof globalThis.fetch
  /**
   * The retry queue of the in-place sign-in (`createSessionExpiry`), where
   * a `401` waits. `main.tsx` passes the one queue that the auth provider
   * and the app get too.
   */
  readonly sessionExpiry: SessionExpiry
}

/** The `hu.ts` key that a not-yet-implemented call reports. */
export const NOT_AVAILABLE_MESSAGE_KEY =
  "app.data_provider.not_available" satisfies MessageKey

/**
 * Rejection of a data provider call that no slice implements yet.
 *
 * `message` is the `hu.ts` key {@link NOT_AVAILABLE_MESSAGE_KEY}: ra-core's
 * notification translates an error's message, so the editor reads Hungarian
 * text. `resource` and `method` tell a developer which handler is missing.
 */
export class DataProviderNotImplementedError extends Error {
  /** The ra-core resource name of the failed call. */
  readonly resource: string
  /** The ra-core method of the failed call. */
  readonly method: DataProviderMethod

  /**
   * @param method - the ra-core method that was called.
   * @param resource - the ra-core resource it was called for.
   */
  constructor(method: DataProviderMethod, resource: string) {
    super(NOT_AVAILABLE_MESSAGE_KEY)
    this.name = "DataProviderNotImplementedError"
    this.method = method
    this.resource = resource
  }
}

/**
 * Thrown by {@link createDataProvider} when its options are invalid. The
 * message says which option is wrong and what it must look like.
 */
export class DataProviderConfigError extends Error {
  /**
   * @param message - the actionable description of the problem.
   */
  constructor(message: string) {
    super(message)
    this.name = "DataProviderConfigError"
  }
}

const KNOWN_METHODS: ReadonlySet<string> = new Set(DATA_PROVIDER_METHODS)

/**
 * Checks the provider options and throws an actionable error for the first
 * problem it finds.
 *
 * @param options - the options passed to {@link createDataProvider}.
 * @throws {DataProviderConfigError} when `apiBaseUrl` is not a root-relative
 *   path, `fetch` is not a function, `sessionExpiry` is not a retry queue,
 *   or a resource name or handler name is invalid.
 */
function validateOptions(options: DataProviderOptions): void {
  const { apiBaseUrl, resources } = options
  // The same rule as the auth provider's (src/api/base-path.ts).
  const problem = basePathProblem("apiBaseUrl", apiBaseUrl)
  if (problem !== undefined) {
    throw new DataProviderConfigError(problem)
  }
  if (typeof options.fetch !== "function") {
    throw new DataProviderConfigError(
      "fetch must be a function; main.tsx passes window.fetch bound to window."
    )
  }
  if (typeof options.sessionExpiry?.waitForSignIn !== "function") {
    throw new DataProviderConfigError(
      "sessionExpiry must be the retry queue of createSessionExpiry(); main.tsx creates one and passes the same queue to both providers and to the app."
    )
  }
  for (const [resource, handlers] of Object.entries(resources)) {
    if (resource.trim() === "") {
      throw new DataProviderConfigError(
        "resources has an entry with an empty name; use the ra-core <Resource name> it serves."
      )
    }
    for (const method of Object.keys(handlers)) {
      if (!KNOWN_METHODS.has(method)) {
        throw new DataProviderConfigError(
          `resources.${resource} has a handler "${method}", which is not an ra-core data provider ` +
            `method. Use one of: ${DATA_PROVIDER_METHODS.join(", ")}.`
        )
      }
    }
  }
}

/**
 * Builds backend3's data provider.
 *
 * @param options - the API base path, the handlers per resource, the `fetch`
 *   to call and the in-place sign-in's retry queue; see
 *   {@link DataProviderOptions}.
 * @returns an ra-core `DataProvider` with `supportAbortSignal: true`. A
 *   call whose resource or method has no handler returns a promise rejected
 *   with {@link DataProviderNotImplementedError}.
 * @throws {DataProviderConfigError} when the options are invalid.
 */
export function createDataProvider(options: DataProviderOptions): DataProvider {
  validateOptions(options)
  const { apiBaseUrl, fetch, sessionExpiry } = options
  const context: DataProviderContext = {
    apiBaseUrl,
    request(path, init = {}) {
      // Checked here, not by the type alone: a handler builds the path at run
      // time, and a path without its leading "/" would run into the base
      // path's last segment ("/api/adminlabels").
      if (typeof path !== "string" || !path.startsWith("/")) {
        throw new TypeError(
          `request expects a path below apiBaseUrl that starts with "/", such as "/labels"; got ${JSON.stringify(path)}.`
        )
      }
      if (init.body instanceof ReadableStream) {
        throw new TypeError(
          "request cannot send a stream body: a request is sent again after the in-place sign-in, and a stream can be read only once. Send a string, Blob or FormData."
        )
      }
      const signal = init.signal ?? undefined
      return sendWithSignIn(
        () =>
          fetch(`${apiBaseUrl}${path}`, {
            ...init,
            credentials: "same-origin",
          }),
        sessionExpiry,
        signal
      )
    },
  }

  const dispatch = (
    method: DataProviderMethod,
    resource: string,
    params: unknown
  ) => {
    const handler = Object.hasOwn(options.resources, resource)
      ? options.resources[resource][method]
      : undefined
    if (handler === undefined) {
      return Promise.reject(
        new DataProviderNotImplementedError(method, resource)
      )
    }
    // The handler table is typed per method above; `dispatch` is generic over
    // the method, so TypeScript cannot relate the parameter types here.
    return (
      handler as (
        params: unknown,
        context: DataProviderContext
      ) => Promise<unknown>
    )(params, context)
  }

  const provider = {
    ...Object.fromEntries(
      DATA_PROVIDER_METHODS.map((method) => [
        method,
        (resource: string, params: unknown) =>
          dispatch(method, resource, params),
      ])
    ),
    // Without it, ra-core's query hooks (`useGetList`, `useGetOne`,
    // `useGetMany` and the rest) leave `params.signal` undefined. With it,
    // they pass react-query's signal, which fires when the screen that asked
    // unmounts, so a query that waits for the in-place sign-in stops waiting
    // and the dialog closes once nothing else waits.
    supportAbortSignal: true,
  }
  // ra-core's DataProvider methods are generic over the record type; the
  // entries above satisfy every instantiation because they forward unchanged.
  return provider as unknown as DataProvider
}
