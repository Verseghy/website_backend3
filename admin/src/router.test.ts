// @vitest-environment node
import { readdirSync, readFileSync } from "node:fs"
import { join, relative, resolve } from "node:path"

import { describe, expect, it } from "vitest"

import { ADMIN_BASENAME, adminRoutes } from "./router"

const SRC = resolve(import.meta.dirname)

/**
 * Lists the TypeScript sources under `dir`, without tests.
 *
 * @param dir - the directory to walk.
 * @returns absolute file paths.
 */
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) {
      return sources(path)
    }
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)
      ? [path]
      : []
  })
}

describe("adminRoutes", () => {
  it("hands every path under /admin to the admin root, with no router basename", () => {
    const routes = adminRoutes(null)
    expect(routes).toEqual([{ path: "/admin/*", element: null }])
    expect(ADMIN_BASENAME).toBe("/admin")
  })
})

describe("links to fixed admin paths", () => {
  it("never use a root-absolute `to`, which would leave /admin (ra-core's LinkBase adds no basename)", () => {
    const offenders = sources(SRC).flatMap((file) =>
      readFileSync(file, "utf8")
        .split("\n")
        .flatMap((line, index) =>
          /\bto=(["'])\//.test(line) && !/^\s*\*/.test(line)
            ? [`${relative(SRC, file)}:${index + 1}`]
            : []
        )
    )
    expect(offenders).toEqual([])
  })
})
