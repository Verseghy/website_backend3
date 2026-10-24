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
 * Configuration arrives as explicit arguments ({@link DataProviderOptions}),
 * never from globals, so tests build a provider against stubs.
 */
import type { DataProvider } from "ra-core"

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

/** What every handler receives besides ra-core's parameters. */
export interface DataProviderContext {
  /** The same-origin base path of the admin API, for example `/api/admin`. */
  readonly apiBaseUrl: string
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
 * A same-origin API base path: `/` and an unreserved segment, repeated, with no
 * empty, `.` or `..` segment. A browser resolves it against the admin's own
 * origin, never against another host.
 */
const SAME_ORIGIN_BASE_PATH = /^(?:\/(?!\.{1,2}(?:\/|$))[A-Za-z0-9._~-]+)+$/

/**
 * Checks the provider options and throws an actionable error for the first
 * problem it finds.
 *
 * @param options - the options passed to {@link createDataProvider}.
 * @throws {DataProviderConfigError} when `apiBaseUrl` is not a root-relative
 *   path, or a resource name or handler name is invalid.
 */
function validateOptions(options: DataProviderOptions): void {
  const { apiBaseUrl, resources } = options
  if (
    typeof apiBaseUrl !== "string" ||
    !SAME_ORIGIN_BASE_PATH.test(apiBaseUrl)
  ) {
    throw new DataProviderConfigError(
      `apiBaseUrl must be a root-relative path such as "/api/admin" (leading "/", no trailing "/", ` +
        `no scheme or host, only letters, digits and "-._~" in "/"-separated segments, so requests ` +
        `stay same-origin), but it is ${JSON.stringify(apiBaseUrl)}.`
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
 * @param options - the API base path and the handlers per resource; see
 *   {@link DataProviderOptions}.
 * @returns an ra-core `DataProvider`. A call whose resource or method has no
 *   handler returns a promise rejected with
 *   {@link DataProviderNotImplementedError}.
 * @throws {DataProviderConfigError} when the options are invalid.
 */
export function createDataProvider(options: DataProviderOptions): DataProvider {
  validateOptions(options)
  const context: DataProviderContext = { apiBaseUrl: options.apiBaseUrl }

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

  const provider = Object.fromEntries(
    DATA_PROVIDER_METHODS.map((method) => [
      method,
      (resource: string, params: unknown) => dispatch(method, resource, params),
    ])
  )
  // ra-core's DataProvider methods are generic over the record type; the
  // entries above satisfy every instantiation because they forward unchanged.
  return provider as unknown as DataProvider
}
