// @vitest-environment node
/**
 * The admin never calls the scripts' sign-in, `POST /api/admin/auth/token`
 * (ADR 0006 §3, admin/CLAUDE.md). That endpoint answers the session token
 * in its body, where page scripts could read it, while the admin's session
 * lives only in the `HttpOnly` cookie of `/auth/login`. Its bindings,
 * `TokenRequest` and `TokenResponse`, are generated only because every
 * admin API type has one (ADR 0005), so no source outside `src/bindings/`
 * may name the path or import them.
 */
import { readdirSync, readFileSync } from "node:fs"
import { join, relative, resolve } from "node:path"

import { describe, expect, it } from "vitest"

const SRC = resolve(import.meta.dirname, "..")
const BINDINGS = join(SRC, "bindings")

/**
 * Lists the TypeScript sources under `dir`, without tests and without the
 * generated bindings.
 *
 * @param dir - the directory to walk.
 * @returns absolute file paths.
 */
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) {
      return path === BINDINGS ? [] : sources(path)
    }
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)
      ? [path]
      : []
  })
}

/**
 * The `file:line` of every line of the sources that matches `pattern`.
 *
 * @param pattern - what a line must not contain.
 * @returns the offending lines, relative to `src/`.
 */
function offenders(pattern: RegExp): string[] {
  return sources(SRC).flatMap((file) =>
    readFileSync(file, "utf8")
      .split("\n")
      .flatMap((line, index) =>
        pattern.test(line) ? [`${relative(SRC, file)}:${index + 1}`] : []
      )
  )
}

describe("the scripts' sign-in", () => {
  it("is never called by the admin", () => {
    expect(offenders(/auth\/token\b/)).toEqual([])
  })

  it("has bindings that no admin source imports", () => {
    expect(offenders(/bindings\/Token(Request|Response)\b/)).toEqual([])
  })

  it("is looked for in every source, the auth provider included", () => {
    // The walk must see the one module that talks to `/api/admin/auth`,
    // or the two checks above would pass on nothing.
    const walked = sources(SRC).map((file) => relative(SRC, file))
    expect(walked).toContain(join("auth", "auth-provider.ts"))
    expect(walked.some((file) => file.startsWith("bindings"))).toBe(false)
  })
})
