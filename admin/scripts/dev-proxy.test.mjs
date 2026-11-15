// @vitest-environment node
/**
 * The dev server's proxy (`dev-proxy.mjs`, ADR 0004 §4, ADR 0044): the
 * `Origin` translation as a pure rule, and the same table in a real Vite
 * dev server in front of a probe backend that answers with the `Origin` and
 * `Host` it received, so the test sees what backend3 would see.
 */
import { mkdtemp, rm } from "node:fs/promises"
import { createServer as createHttpServer, request } from "node:http"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { createServer as createViteServer } from "vite"
import { afterAll, beforeAll, describe, expect, it } from "vitest"

import { DEV_BACKEND, devProxy, proxiedOrigin } from "./dev-proxy.mjs"

describe("proxiedOrigin", () => {
  const backend = "http://localhost:5150"

  it("translates the dev server's own origin into the backend's", () => {
    expect(
      proxiedOrigin(
        {
          origin: "http://localhost:5173",
          host: "localhost:5173",
          secure: false,
        },
        backend
      )
    ).toBe("http://localhost:5150")
    expect(
      proxiedOrigin(
        {
          origin: "https://127.0.0.1:4443",
          host: "127.0.0.1:4443",
          secure: true,
        },
        "http://localhost:5150/some/path"
      )
    ).toBe("http://localhost:5150")
  })

  it.each([
    // A sibling port of localhost: the same site, so the browser sends the
    // cookie; backend3 must see and refuse it.
    ["another port", "http://localhost:5151", "localhost:5173", false],
    ["another site", "https://verseghy-gimnazium.net", "localhost:5173", false],
    ["the other scheme", "https://localhost:5173", "localhost:5173", false],
    ["an opaque origin", "null", "localhost:5173", false],
    ["no Origin", undefined, "localhost:5173", false],
    ["two Origin headers", ["http://localhost:5173"], "localhost:5173", false],
    ["no Host", "http://localhost:5173", undefined, false],
    ["an empty Host", "http://", "", false],
    // Already the backend's own origin: left as it is.
    ["the backend's origin", "http://localhost:5150", "localhost:5173", false],
  ])("forwards %s unchanged", (_case, origin, host, secure) => {
    expect(proxiedOrigin({ origin, host, secure }, backend)).toBeUndefined()
  })

  it("proxies to cargo loco start", () => {
    expect(DEV_BACKEND).toBe("http://localhost:5150")
    expect(Object.keys(devProxy(DEV_BACKEND)).sort()).toEqual([
      "/api",
      "/storage",
    ])
  })
})

/**
 * Sends `method path` to `port` on 127.0.0.1 with `headers`, and returns the
 * status and the JSON body.
 *
 * @param {number} port - the dev server's port.
 * @param {string} method - the request method.
 * @param {string} path - the path.
 * @param {Record<string, string>} headers - the request headers.
 * @returns {Promise<{ status: number | undefined, body: { origin: string | null, host: string | undefined } }>}
 *   the answer.
 */
function send(port, method, path, headers) {
  return new Promise((resolve, reject) => {
    const outgoing = request(
      { host: "127.0.0.1", port, method, path, headers },
      (response) => {
        let text = ""
        response.setEncoding("utf8")
        response.on("data", (chunk) => (text += chunk))
        response.on("end", () =>
          resolve({ status: response.statusCode, body: JSON.parse(text) })
        )
      }
    )
    outgoing.on("error", reject)
    outgoing.end()
  })
}

describe("the dev server's proxy", () => {
  /** @type {import("node:http").Server} */
  let backend
  /** @type {import("vite").ViteDevServer} */
  let vite
  /** @type {string} */
  let root
  /** @type {number} */
  let port
  /** @type {string} */
  let backendUrl

  beforeAll(async () => {
    // The probe backend: answers every request with the Origin and Host
    // it received.
    backend = createHttpServer((incoming, response) => {
      response.setHeader("content-type", "application/json")
      response.end(
        JSON.stringify({
          origin: incoming.headers.origin ?? null,
          host: incoming.headers.host,
        })
      )
    })
    await new Promise((resolve) =>
      backend.listen(0, "127.0.0.1", () => resolve(undefined))
    )
    const address = backend.address()
    if (address === null || typeof address === "string") {
      throw new Error("the probe backend has no TCP address")
    }
    backendUrl = `http://127.0.0.1:${address.port}`
    // An empty root, so the dev server scans and serves nothing else.
    root = await mkdtemp(join(tmpdir(), "dev-proxy-"))
    vite = await createViteServer({
      configFile: false,
      root,
      cacheDir: join(root, ".vite"),
      logLevel: "silent",
      appType: "custom",
      server: {
        host: "127.0.0.1",
        port: 0,
        strictPort: false,
        hmr: false,
        ws: false,
        proxy: devProxy(backendUrl),
      },
    })
    await vite.listen()
    const devAddress = vite.httpServer?.address()
    if (
      devAddress === null ||
      devAddress === undefined ||
      typeof devAddress === "string"
    ) {
      throw new Error("the dev server has no TCP address")
    }
    port = devAddress.port
  })

  afterAll(async () => {
    await vite?.close()
    await new Promise((resolve) => backend?.close(() => resolve(undefined)))
    if (root !== undefined) {
      await rm(root, { recursive: true, force: true })
    }
  })

  it("gives a write from its own page the backend's origin and host", async () => {
    const answer = await send(port, "POST", "/api/admin/auth/logout", {
      origin: `http://127.0.0.1:${port}`,
    })
    expect(answer.status).toBe(200)
    expect(answer.body).toEqual({
      origin: backendUrl,
      host: new URL(backendUrl).host,
    })
  })

  it.each([
    ["a sibling port", "http://127.0.0.1:1"],
    ["another site", "https://verseghy-gimnazium.net"],
    ["an opaque origin", "null"],
  ])("forwards a write from %s unchanged", async (_case, origin) => {
    const answer = await send(port, "POST", "/api/admin/auth/logout", {
      origin,
    })
    expect(answer.body.origin).toBe(origin)
  })

  it("adds no Origin to a request without one", async () => {
    const answer = await send(port, "POST", "/api/admin/auth/logout", {})
    expect(answer.body.origin).toBeNull()
  })

  it("proxies the media URLs as they are", async () => {
    const answer = await send(port, "GET", "/storage/authors/a.jpg", {
      origin: `http://127.0.0.1:${port}`,
    })
    expect(answer.status).toBe(200)
    expect(answer.body.origin).toBe(`http://127.0.0.1:${port}`)
  })
})
