// @vitest-environment node
import fc from "fast-check"
import { describe, expect, it } from "vitest"

import { SCRUBBED_USER, scrubUserPaths } from "./scrub"

/** The names in these tests are invented placeholders, never real people. */
describe("scrubUserPaths", () => {
  it.each([
    [
      "Word's picture link",
      '<img src="file:///C:/Users/teszt/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png">',
      '<img src="file:///C:/Users/USER/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png">',
    ],
    [
      "a plain-text Windows path with spaces in the name",
      "C:\\Users\\Teszt Elek\\Documents\\eredmeny.xlsx",
      "C:\\Users\\USER\\Documents\\eredmeny.xlsx",
    ],
    [
      "a lower-case drive and folder, and an 8.3 short name",
      "d:/users/TESZTE~1/AppData",
      "d:/users/USER/AppData",
    ],
    [
      "percent-encoded separators and an encoded name",
      "file:///C:%5CUsers%5CTeszt%20Elek%5Cclip.png",
      "file:///C:%5CUsers%5CUSER%5Cclip.png",
    ],
    [
      "the older profile folder",
      "C:\\Documents and Settings\\teszt\\Asztal",
      "C:\\Documents and Settings\\USER\\Asztal",
    ],
    [
      "a name with a quote and parentheses inside the path",
      "C:\\Users\\O'Neill (Teszt)\\x",
      "C:\\Users\\USER\\x",
    ],
    [
      "a path glued to a word before its drive letter",
      "xC:\\Users\\name",
      "xC:\\Users\\USER",
    ],
    [
      "a macOS file URL",
      'src="file:///Users/teszt/Pictures/a.png"',
      'src="file:///Users/USER/Pictures/a.png"',
    ],
    [
      "an Office for Mac link with four slashes",
      '<img src="file:////Users/teszt/Library/Group%20Containers/clip.png">',
      '<img src="file:////Users/USER/Library/Group%20Containers/clip.png">',
    ],
    [
      "a percent-encoded macOS file URL",
      "file:%2F%2F%2FUsers%2Fteszt%2Fx",
      "file:%2F%2F%2FUsers%2FUSER%2Fx",
    ],
    ["a macOS path after a label", "Path:/Users/teszt/a", "Path:/Users/USER/a"],
    ["a Linux home folder", "(/home/teszt/b.png)", "(/home/USER/b.png)"],
    [
      "a profile folder at the end of an Excel cell",
      "C:\\Users\\Teszt Elek\tnext cell",
      "C:\\Users\\USER\tnext cell",
    ],
    [
      "a profile folder at the end of an HTML element",
      "<p>C:\\Users\\Teszt Elek</p>",
      "<p>C:\\Users\\USER</p>",
    ],
    [
      "a profile folder before a space and the end of an element",
      "<p>C:\\Users\\Teszt Elek </p>",
      "<p>C:\\Users\\USER </p>",
    ],
    [
      "a profile folder at the end of a single-quoted attribute",
      "<a href='file:///C:/Users/teszt'>x</a>",
      "<a href='file:///C:/Users/USER'>x</a>",
    ],
    [
      "a profile folder at the end of a CSS url()",
      "style=\"background:url('file:///C:/Users/teszt')\"",
      "style=\"background:url('file:///C:/Users/USER')\"",
    ],
    [
      "a profile folder before a colon in text",
      "C:\\Users\\anna: kész",
      "C:\\Users\\USER: kész",
    ],
  ])("scrubs %s", (_name, input, expected) => {
    expect(scrubUserPaths(input)).toEqual({ value: expected, replacements: 1 })
  })

  it.each([
    [
      "three kinds of path in one text",
      "C:\\Users\\a\\x and file:///C:/Users/b/y and /Users/c/z",
      "C:\\Users\\USER\\x and file:///C:/Users/USER/y and /Users/USER/z",
      3,
    ],
    [
      "a bare profile folder, text, then a second Windows path",
      "C:\\Users\\Anna és C:\\Users\\Bela\\x",
      "C:\\Users\\USER C:\\Users\\USER\\x",
      2,
    ],
    [
      "two percent-encoded Windows paths",
      "C%3A%5CUsers%5CAnna%20C%3A%5CUsers%5CBela%5Cx",
      "C%3A%5CUsers%5CUSER%20C%3A%5CUsers%5CUSER%5Cx",
      2,
    ],
    [
      "two Windows paths glued together",
      "C:\\Users\\abC:\\Users\\def",
      "C:\\Users\\USERC:\\Users\\USER",
      2,
    ],
    [
      "two Linux home folders",
      "/home/anna /home/bela",
      "/home/USER /home/USER",
      2,
    ],
    [
      "a macOS path, then a Windows path",
      "/Users/anna C:\\Users\\bela",
      "/Users/USER C:\\Users\\USER",
      2,
    ],
  ])("scrubs every path of %s", (_name, input, expected, replacements) => {
    expect(scrubUserPaths(input)).toEqual({ value: expected, replacements })
  })

  it.each([
    ["a web URL", "https://example.com/Users/profile/12"],
    ["a web URL whose host is home", "https://home/x"],
    ["a bare folder without a name", "C:\\Users\\ and the rest"],
    ["a folder that is not a profile folder", "C:\\Usr\\name"],
    ["text without paths", "Eredmények 2026. 10. 24."],
  ])("leaves %s alone", (_name, input) => {
    expect(scrubUserPaths(input)).toEqual({ value: input, replacements: 0 })
  })

  it("does not count a name that is already scrubbed", () => {
    expect(scrubUserPaths("C:/Users/USER/a").replacements).toBe(0)
  })
})

/** Folder names: letters (with Hungarian accents), digits, inner spaces, punctuation. */
const folderName = fc
  .string({
    unit: fc.constantFrom(..."aábcdeéEiíoóöőuúüűxyzZ019 .-_~'()&"),
    minLength: 1,
    maxLength: 20,
  })
  .map((name) => name.trim())
  .filter((name) => name !== "" && name !== SCRUBBED_USER)

/** Path pieces that put prefixes, separators and names side by side. */
const pathToken = fc.constantFrom(
  ...["C:", "c:", "%3A", "\\", "/", "%5C", "%2f", "Users", "users", "home"],
  ...["Documents and Settings", "file://", "file:////", "USER", "teszt"],
  ...["Elek", " ", "%20", "\t", "\n", '"', "'", ")", "<", ">", "x", "R"]
)

/**
 * One profile path, as a function of the name in it, so a test can build the
 * same path with the real name and with {@link SCRUBBED_USER}. Some paths end
 * at the name, as a bare profile folder in plain text does.
 */
const profilePath = fc.oneof(
  fc.tuple(fc.constantFrom("C", "d"), fc.constantFrom("\\", "/", "%5C")).map(
    ([drive, sep]) =>
      (who: string, tail: boolean) =>
        `${drive}:${sep}Users${sep}${who}${tail ? `${sep}AppData${sep}clip.png` : ""}`
  ),
  fc.tuple(fc.constantFrom("Users", "home"), fc.constantFrom("/", "%2F")).map(
    ([folder, sep]) =>
      (who: string, tail: boolean) =>
        `${sep}${folder}${sep}${who}${tail ? `${sep}Desktop${sep}a` : ""}`
  )
)

describe("scrubUserPaths properties", () => {
  it("replaces exactly the name of a Windows profile path, whatever the name", () => {
    fc.assert(
      fc.property(
        fc.constantFrom("", "file:///", 'src="file:///', " ", "\t"),
        fc.constantFrom("C", "d"),
        fc.constantFrom("\\", "/", "%5C"),
        fc.constantFrom("Users", "users", "Documents and Settings"),
        folderName,
        (before, drive, sep, folder, name) => {
          const path = (who: string) =>
            `${before}${drive}:${sep}${folder}${sep}${who}${sep}AppData${sep}clip.png"`
          expect(scrubUserPaths(path(name))).toEqual({
            value: path(SCRUBBED_USER),
            replacements: 1,
          })
        }
      )
    )
  })

  it("replaces exactly the name of a macOS or Linux home path", () => {
    fc.assert(
      fc.property(
        fc.constantFrom("", "file://", "file:///", " ", "(", '"', ":"),
        fc.constantFrom("Users", "home"),
        folderName,
        (before, folder, name) => {
          const path = (who: string) => `${before}/${folder}/${who}/Desktop/a`
          expect(scrubUserPaths(path(name))).toEqual({
            value: path(SCRUBBED_USER),
            replacements: 1,
          })
        }
      )
    )
  })

  it("scrubs each of several paths on one line, and keeps what lies between them", () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.record({
            path: profilePath,
            name: folderName,
            tail: fc.boolean(),
            after: fc.constantFrom(" ", "  ", "\t", "\n", " és ", ", ", "; "),
          }),
          { minLength: 2, maxLength: 4 }
        ),
        (paths) => {
          const text = (scrubbed: boolean) =>
            paths
              .map(({ path, name, tail, after }) => {
                // A name that ends a path runs on over text (the module doc),
                // and a trailing `'` or `)` there closes the context; the
                // unit cases above cover both. Here such a path is followed
                // by whitespace only, and its name ends in neither.
                const bare = !tail
                const who = bare ? name.replace(/[\s')]+$/, "") || "x" : name
                const gap = bare ? after.replace(/[^\s]/g, "") || " " : after
                return `${path(scrubbed ? SCRUBBED_USER : who, tail)}${gap}`
              })
              .join("")
          expect(scrubUserPaths(text(false))).toEqual({
            value: text(true),
            replacements: paths.length,
          })
        }
      )
    )
  })

  it("is idempotent: a second pass changes nothing and counts nothing", () => {
    fc.assert(
      fc.property(
        fc.array(pathToken, { maxLength: 30 }).map((tokens) => tokens.join("")),
        (input) => {
          const once = scrubUserPaths(input).value
          expect(scrubUserPaths(once)).toEqual({ value: once, replacements: 0 })
        }
      ),
      { numRuns: 2000 }
    )
  })

  it("never changes a text without a profile folder", () => {
    fc.assert(
      fc.property(
        fc.string().filter((text) => !/users|home|documents/i.test(text)),
        (text) => {
          expect(scrubUserPaths(text)).toEqual({ value: text, replacements: 0 })
        }
      )
    )
  })
})
