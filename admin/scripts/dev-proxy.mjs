/**
 * The Vite dev server's proxy to a locally running backend3 (ADR 0004 §4),
 * and the `Origin` it forwards (ADR 0006 §3, as amended by ADR 0044).
 *
 * In development the dev server (`npm run dev`, `http://localhost:5173` by
 * default) serves the admin UI and proxies `/api` and `/storage` to backend3
 * at {@link DEV_BACKEND}, so the browser sees one origin and the
 * `SameSite=Strict` session cookie behaves as in production. backend3
 * refuses a write that carries the session cookie unless its `Origin` header
 * is the admin origin it is configured with (`settings.admin_auth.origin`,
 * `http://localhost:5150` in development). The browser sends the dev
 * server's own origin, so the proxy translates exactly that one into the
 * backend's origin ({@link proxiedOrigin}): the dev server stands in for
 * backend3's origin, as one origin in front of it.
 *
 * Only a request from a page of the dev server itself is translated. A
 * request whose `Origin` names any other origin, a sibling port of
 * `localhost` included (the same site, so the browser sends the cookie), is
 * forwarded as it came and refused by backend3, so the check works in
 * development as in production. No `Origin` is added to a request that has
 * none either. Vite's `allowedHosts` check refuses an unknown `Host` before
 * the proxy runs, so a rebound DNS name cannot pass itself off as the dev
 * server.
 */

/** @import { ProxyOptions } from "vite" */

/** The backend3 that the dev server proxies to: `cargo loco start`. */
export const DEV_BACKEND = "http://localhost:5150"

/**
 * The parts of a proxied request that decide its forwarded `Origin`.
 *
 * @typedef {object} ProxiedRequest
 * @property {string | string[] | undefined} origin - the request's `Origin`
 *   header as Node.js reads it: `undefined` when absent, a string for one.
 * @property {string | undefined} host - the request's `Host` header: the
 *   dev server's host and port as the browser addressed it.
 * @property {boolean} secure - whether the dev server answered it over TLS.
 */

/**
 * The `Origin` to forward to backend3 for a proxied request.
 *
 * @param {ProxiedRequest} request - the request's `Origin`, `Host` and
 *   scheme.
 * @param {string} backend - the proxy's target, an absolute `http` or
 *   `https` URL.
 * @returns {string | undefined} the backend's origin when the request's
 *   one `Origin` is the dev server's own (`<scheme>://<Host>`), so that a
 *   write from the admin passes backend3's check; `undefined` (forward the
 *   request unchanged) for every other origin, `null`, no header, or no
 *   `Host`.
 */
export function proxiedOrigin(request, backend) {
  const { origin, host, secure } = request
  if (typeof origin !== "string" || host === undefined || host === "") {
    return undefined
  }
  const devServer = `${secure ? "https" : "http"}://${host}`
  return origin === devServer ? new URL(backend).origin : undefined
}

/**
 * The dev server's `server.proxy` table: `/api` with the `Origin`
 * translation of {@link proxiedOrigin}, and `/storage`, the root-relative
 * media URLs that the media library previews (ADR 0015, ADR 0023 §6).
 *
 * @param {string} backend - the backend3 to proxy to; {@link DEV_BACKEND}
 *   in `vite.config.ts`, a probe server in the tests.
 * @returns {Record<string, ProxyOptions>} the table for `server.proxy`.
 */
export function devProxy(backend) {
  return {
    "/api": {
      target: backend,
      // Vite's default for a target given as a string: backend3 sees its
      // own `Host`, as behind the router in production.
      changeOrigin: true,
      configure(proxy) {
        proxy.on("proxyReq", (proxyReq, req) => {
          const origin = proxiedOrigin(
            {
              origin: req.headers.origin,
              host: req.headers.host,
              secure:
                "encrypted" in req.socket && req.socket.encrypted === true,
            },
            backend
          )
          if (origin !== undefined) {
            proxyReq.setHeader("origin", origin)
          }
        })
      },
    },
    "/storage": { target: backend, changeOrigin: true },
  }
}
