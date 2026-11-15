// @ts-check
/**
 * The licence notices of the npm packages and the copied code bundled into
 * `admin/dist` (ADR 0038).
 *
 * Why this module exists. MIT, ISC, BSD and Apache-2.0 ask every copy of the
 * software, or of a substantial portion of it, to carry the copyright and
 * licence notice. The minified admin keeps no source comments, so without a
 * separate file the image and every `/admin` visitor would get the packages'
 * code without their notices (ADR 0027, Consequences). The maintainer chose
 * Vite's own option for it (research: DECISIONS R15, D88): with
 * `build.license`, Vite 8 writes one file that lists the name, version, SPDX
 * identifier and licence text of every package with a module in a JavaScript
 * chunk. `vite.config.ts` names that file {@link LICENCE_NOTICES_FILE}, at the
 * root of `dist/`, where backend3's root-file row serves it (ADR 0004 §3).
 *
 * Vite's scan reads only the modules of the JavaScript chunks
 * (vite@8.3.5:src/node/plugins/license.ts, `chunk.moduleIds`), so it misses
 * two ways a package reaches `dist/`:
 *
 * - **A stylesheet `@import`.** `src/index.css` imports `tailwindcss`,
 *   `tw-animate-css`, `shadcn/tailwind.css` and `@fontsource-variable/inter`.
 *   Tailwind's Vite plugin inlines the imported CSS while it compiles the
 *   file, so those files never become modules of a chunk.
 * - **An asset copied from a package**, such as Inter's font files, which the
 *   CSS references with `url()`.
 *
 * It also skips every module outside `node_modules`, so the code copied into
 * the admin's own `src/` (shadcn-admin-kit, shadcn/ui, the Hungarian strings
 * of ra-language-hungarian; `THIRD_PARTY.md`) reaches the bundle without its
 * MIT notice, although it is compiled into the same chunks.
 *
 * {@link licenceNotices} is a small Vite plugin that adds those packages to
 * the same file, in Vite's own format, after Vite has written it, and then
 * the licence files of the copied code, in a section of their own. It adds no
 * dependency and keeps one file, as D88 asks. Its limits: it reads the
 * `@import` rules of the stylesheets that are modules of the bundle, not of
 * the package stylesheets those rules pull in; like Vite it reads only
 * licence files, not an Apache-2.0 `NOTICE` file; and it does not list the
 * helpers that the bundler generates into the chunks (Vite's module-preload
 * polyfill, Rolldown's runtime), which ADR 0038 §1 reads as not a
 * substantial portion of either project. None of today's imported packages
 * imports another package from its CSS, no bundled package has a `NOTICE`
 * file, and `licence-notices.build.test.mjs` fails if either changes.
 *
 * The helpers are pure except {@link findPackageDir},
 * {@link readNoticeEntry} and {@link readCopiedLicences}, which read files.
 */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs"
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path"

/**
 * The notices file, relative to `dist/`. A `.txt` name, although Vite writes
 * Markdown into every file name that does not end in `.json`: browsers show
 * `text/plain` inline, while Firefox offers `text/markdown` as a download.
 * Markdown reads as plain text by design.
 */
export const LICENCE_NOTICES_FILE = "licenses.txt"

/** Where backend3 serves the notices file (ADR 0004 §3, root files). */
export const LICENCE_NOTICES_PATH = `/admin/${LICENCE_NOTICES_FILE}`

/**
 * The comment that starts every JavaScript chunk, so a chunk copied on its
 * own still says where the notices of its code are (the tip of Vite's
 * `build.license` documentation). `/*!` marks it as a legal comment.
 */
export const LICENCE_NOTICES_BANNER = `/*! Licence notices of the bundled code: ${LICENCE_NOTICES_PATH} */`

/** The sentence Vite writes before the entries of a non-empty file. */
export const VITE_INTRO =
  "The app bundles dependencies which contain the following licenses:"

/** The sentence Vite writes instead when it found no package. */
export const VITE_EMPTY =
  "The app does not bundle any dependencies with licenses."

/**
 * The directory of the admin's own source, relative to the Vite root. Code
 * copied into it keeps its licence file beside it (ADR 0012 §4), where
 * {@link readCopiedLicences} looks.
 */
export const SOURCE_DIR = "src"

/** The heading of the section that lists the copied code's licence files. */
export const COPIED_CODE_HEADING = "# Code copied into the admin"

/**
 * The paragraph below {@link COPIED_CODE_HEADING}, wrapped like the licence
 * texts so the file reads well in a browser.
 */
export const COPIED_CODE_INTRO = [
  "The admin's own source (admin/ of website_backend3) holds code copied from",
  "other projects, under their own licences. Their licence files follow, by",
  "path below admin/; THIRD_PARTY.md of the repository lists the files that",
  "each one covers.",
].join("\n")

/**
 * One package's notice, in the shape of Vite's JSON output.
 *
 * @typedef {object} NoticeEntry
 * @property {string} name - the package name, for example `tailwindcss`.
 * @property {string} version - its installed version; `0.0.0` when its
 *   `package.json` has none, as Vite writes it.
 * @property {string} [identifier] - its `license` field, an SPDX expression.
 * @property {string} [text] - the trimmed text of its licence file.
 */

/**
 * The licence file of code copied into the admin's source.
 *
 * @typedef {object} CopiedLicence
 * @property {string} path - the file, relative to the Vite root, with `/`
 *   separators, for example `src/components/admin/LICENSE`.
 * @property {string} text - its trimmed text.
 */

/** File names that hold a package's licence, as Vite looks for them. */
const LICENCE_FILE = [/^license/i, /^licence/i, /^copying/i]

/**
 * Whether a file name is a licence file, by the patterns Vite uses
 * (vite@8.3.5:src/node/plugins/license.ts, `licenseFiles`).
 *
 * @param {string} name - a file name without its directory.
 * @returns {boolean} whether it starts with `license`, `licence` or
 *   `copying`, in any case.
 */
export function isLicenceFileName(name) {
  return LICENCE_FILE.some((pattern) => pattern.test(name))
}

/**
 * Whether a file name in the admin's own source is the licence file of
 * copied code.
 *
 * Stricter than {@link isLicenceFileName}: `src/` also holds backend3's own
 * modules, and one may well be called `licence-link.tsx`. Licence files of
 * copied code are named in capitals by convention (`LICENSE`,
 * `LICENSE.ra-language-hungarian`; ADR 0012 §4), and no module is.
 *
 * @param {string} name - a file name without its directory.
 * @returns {boolean} whether it is `LICENSE`, `LICENCE` or `COPYING`, in
 *   capitals, alone or followed by `.` or `-` and a suffix.
 */
export function isCopiedLicenceFileName(name) {
  return /^(?:LICEN[CS]E|COPYING)(?:[.-].*)?$/s.test(name)
}

/**
 * Lists the specifiers of the `@import` rules of a stylesheet.
 *
 * @param {string} css - the stylesheet's source.
 * @returns {string[]} each imported specifier in source order, from
 *   `@import "x"`, `@import 'x'` and `@import url(x)` with or without quotes.
 *   Comments are skipped, so a commented-out import does not count.
 */
export function cssImportSpecifiers(css) {
  const withoutComments = css.replace(/\/\*[\s\S]*?(?:\*\/|$)/g, " ")
  const rule =
    /@import\s+(?:url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s'"]*))\s*\)|"([^"]*)"|'([^']*)')/gi
  return [...withoutComments.matchAll(rule)].map(
    (match) => match.slice(1).find((group) => group !== undefined) ?? ""
  )
}

/**
 * The npm package that a bare import specifier names.
 *
 * @param {string} specifier - an `@import` specifier, for example
 *   `shadcn/tailwind.css` or `@fontsource-variable/inter`.
 * @returns {string | null} the package name (`shadcn`,
 *   `@fontsource-variable/inter`), or `null` when the specifier is not bare:
 *   relative or absolute paths, URLs with a scheme (`https:`, `data:`),
 *   subpath imports (`#x`), and a scope without a name.
 */
export function packageOfSpecifier(specifier) {
  const trimmed = specifier.trim()
  if (
    trimmed === "" ||
    /^[./\\#~]/.test(trimmed) ||
    /^[a-z][a-z0-9+.-]*:/i.test(trimmed)
  ) {
    return null
  }
  const segments = trimmed.split("/")
  if (trimmed.startsWith("@")) {
    const [scope, name] = segments
    return scope.length > 1 && name ? `${scope}/${name}` : null
  }
  return segments[0]
}

/**
 * The package that a file under `node_modules` belongs to.
 *
 * @param {string} path - a file path, absolute or relative, with `/` or `\`
 *   separators, for example
 *   `node_modules/@fontsource-variable/inter/files/inter-latin.woff2`.
 * @returns {{ name: string, dir: string } | null} the package name and its
 *   directory (the path up to and including the name, with `/` separators)
 *   of the innermost `node_modules`, or `null` outside `node_modules`.
 */
export function packageOfPath(path) {
  const normalised = path.replaceAll("\\", "/")
  const marker = "node_modules/"
  const index = normalised.lastIndexOf(marker)
  if (index === -1 || (index > 0 && normalised[index - 1] !== "/")) {
    return null
  }
  const start = index + marker.length
  const segments = normalised.slice(start).split("/")
  const count = segments[0]?.startsWith("@") ? 2 : 1
  const parts = segments.slice(0, count)
  if (parts.length < count || parts.some((part) => part === "")) {
    return null
  }
  // A package's own files sit below its directory; the directory alone is
  // not a file of the package.
  if (segments.length <= count) {
    return null
  }
  const name = parts.join("/")
  return { name, dir: normalised.slice(0, start) + name }
}

/**
 * The heading line of one entry, as Vite writes it.
 *
 * @param {NoticeEntry} entry - the package's notice.
 * @returns {string} for example `## react - 19.3.1 (MIT)`.
 */
export function noticeHeading(entry) {
  const identifier = entry.identifier ? ` (${entry.identifier})` : ""
  return `## ${entry.name} - ${entry.version}${identifier}`
}

/**
 * One entry's section, byte for byte as Vite renders it
 * (vite@8.3.5:src/node/plugins/license.ts, `licenseEntryToMarkdown`).
 *
 * @param {NoticeEntry} entry - the package's notice.
 * @returns {string} a blank line, the heading, and the licence text after a
 *   blank line when the package has one.
 */
export function noticeSection(entry) {
  const text = entry.text ? `\n${entry.text}\n` : ""
  return `\n${noticeHeading(entry)}\n${text}`
}

/**
 * Whether the notices file already lists a package at a version.
 *
 * @param {string} notices - the file's text.
 * @param {NoticeEntry} entry - the package to look for.
 * @returns {boolean} whether a heading line names `entry.name` at
 *   `entry.version`, with any identifier.
 */
export function listsPackage(notices, entry) {
  const prefix = `## ${entry.name} - ${entry.version}`
  return notices
    .split("\n")
    .some((line) => line === prefix || line.startsWith(`${prefix} (`))
}

/**
 * Adds the entries the notices file does not list yet.
 *
 * @param {string} notices - the file as Vite wrote it.
 * @param {NoticeEntry[]} entries - the packages to add; duplicates of each
 *   other or of listed packages (same name and version) are skipped.
 * @returns {string} the file with one section per new package after Vite's
 *   own, ordered by `name@version` as Vite orders its entries. When Vite
 *   found no package, its "does not bundle" sentence becomes the sentence it
 *   writes before entries. With nothing new, the text is returned unchanged.
 */
export function addNotices(notices, entries) {
  /** @type {Map<string, NoticeEntry>} */
  const missing = new Map()
  for (const entry of entries) {
    const key = `${entry.name}@${entry.version}`
    if (!missing.has(key) && !listsPackage(notices, entry)) {
      missing.set(key, entry)
    }
  }
  if (missing.size === 0) {
    return notices
  }
  const sections = [...missing]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([, entry]) => noticeSection(entry))
    .join("")
  const intro = notices.replace(VITE_EMPTY, VITE_INTRO)
  const base = intro.endsWith("\n") ? intro : `${intro}\n`
  return base + sections
}

/**
 * The directories from a file's own up to a top directory, the top included.
 * Path arithmetic only: nothing is read from the disk.
 *
 * @param {string} file - a file path; a relative one is resolved against the
 *   working directory, as `node:path` does.
 * @param {string} top - the directory to stop at, resolved the same way.
 * @returns {string[]} the directories, absolute and innermost first, or `[]`
 *   when `file` is not inside `top` (or is `top` itself).
 */
export function ancestorDirs(file, top) {
  const below = relative(top, file)
  if (
    below === "" ||
    below === ".." ||
    below.startsWith(`..${sep}`) ||
    isAbsolute(below)
  ) {
    return []
  }
  const segments = below.split(sep).slice(0, -1)
  return segments
    .map((_, index) =>
      resolve(top, ...segments.slice(0, segments.length - index))
    )
    .concat(resolve(top))
}

/**
 * Appends the licence files of the copied code to the notices file.
 *
 * @param {string} notices - the file with every package's entry.
 * @param {CopiedLicence[]} licences - the licence files; a path listed twice
 *   is written once.
 * @returns {string} the file followed by {@link COPIED_CODE_HEADING}, its
 *   intro and one `## <path>` section per file, sorted by path, each with
 *   the file's text after a blank line, as Vite writes a package's. With no
 *   licence file, the text is returned unchanged.
 */
export function addCopiedCode(notices, licences) {
  /** @type {Map<string, string>} */
  const byPath = new Map()
  for (const { path, text } of licences) {
    if (!byPath.has(path)) byPath.set(path, text)
  }
  if (byPath.size === 0) {
    return notices
  }
  const sections = [...byPath]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([path, text]) => `\n## ${path}\n${text ? `\n${text}\n` : ""}`)
    .join("")
  const base = notices.endsWith("\n") ? notices : `${notices}\n`
  return `${base}\n${COPIED_CODE_HEADING}\n\n${COPIED_CODE_INTRO}\n${sections}`
}

/**
 * A package heading as Vite writes it. The name and version must look like
 * npm's, so a Markdown heading inside a licence text, such as `## Terms` or
 * `## Section 1 - Definitions`, is not taken for one.
 */
const PACKAGE_HEADING =
  /^## ((?:@[A-Za-z0-9~-][A-Za-z0-9._~-]*\/)?[A-Za-z0-9~-][A-Za-z0-9._~-]*) - (\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)(?: \((.+)\))?$/

/** A heading of the copied-code section: a path below {@link SOURCE_DIR}. */
const COPIED_HEADING = new RegExp(`^## (${SOURCE_DIR}/\\S+)$`)

/**
 * One package of a parsed notices file.
 *
 * @typedef {object} ParsedPackage
 * @property {string} name - the package name.
 * @property {string} version - its version.
 * @property {string} [identifier] - the licence in parentheses, if any.
 * @property {string} body - the text below the heading, trimmed.
 */

/**
 * Reads a notices file back into its entries.
 *
 * @param {string} notices - a file as Vite and {@link licenceNotices} write
 *   it.
 * @returns {{ packages: ParsedPackage[], copied: CopiedLicence[] }} the
 *   package entries in file order, and the copied code's licence files after
 *   {@link COPIED_CODE_HEADING}. A heading counts only after a blank line
 *   and in the form its part of the file writes; any other line, a
 *   Markdown heading of a licence text included, belongs to the entry
 *   above it. Text before the first heading is not an entry.
 */
export function parseNotices(notices) {
  const lines = notices.split("\n")
  const marker = lines.findIndex(
    (line, index) =>
      line === COPIED_CODE_HEADING && (index === 0 || lines[index - 1] === "")
  )
  const packageLines = marker === -1 ? lines : lines.slice(0, marker)
  const copiedLines = marker === -1 ? [] : lines.slice(marker + 1)

  /**
   * Splits lines into sections at the headings a pattern accepts.
   *
   * @param {string[]} part - the lines of one part of the file.
   * @param {RegExp} heading - the heading pattern of that part.
   * @returns {{ match: RegExpExecArray, body: string }[]} one per heading.
   */
  function sections(part, heading) {
    /** @type {{ match: RegExpExecArray, body: string[] }[]} */
    const found = []
    part.forEach((line, index) => {
      const match =
        index === 0 || part[index - 1] === "" ? heading.exec(line) : null
      if (match) {
        found.push({ match, body: [] })
      } else {
        found.at(-1)?.body.push(line)
      }
    })
    return found.map(({ match, body }) => ({
      match,
      body: body.join("\n").trim(),
    }))
  }

  const packages = sections(packageLines, PACKAGE_HEADING).map(
    ({ match, body }) => {
      /** @type {ParsedPackage} */
      const entry = { name: match[1], version: match[2], body }
      if (match[3] !== undefined) entry.identifier = match[3]
      return entry
    }
  )
  const copied = sections(copiedLines, COPIED_HEADING).map(
    ({ match, body }) => ({ path: match[1], text: body })
  )
  return { packages, copied }
}

/**
 * Finds an installed package the way Node.js looks for a bare import: in
 * `node_modules` of the importing file's directory and of each parent.
 *
 * @param {string} fromDir - the directory of the importing file.
 * @param {string} name - the package name.
 * @returns {string | null} the package directory, or `null` when no
 *   `node_modules/<name>/package.json` exists on the way up.
 */
export function findPackageDir(fromDir, name) {
  let dir = resolve(fromDir)
  for (;;) {
    const candidate = join(dir, "node_modules", name)
    if (existsSync(join(candidate, "package.json"))) {
      return candidate
    }
    const parent = dirname(dir)
    if (parent === dir) {
      return null
    }
    dir = parent
  }
}

/**
 * The licence files of one directory.
 *
 * @param {string} dir - the directory.
 * @param {(name: string) => boolean} isLicence - which file names count.
 * @returns {string[]} the names of its regular files that `isLicence`
 *   accepts, in code-unit order, so the choice does not depend on the file
 *   system's listing order; `[]` when `dir` is not a directory.
 */
function licenceFilesIn(dir, isLicence) {
  if (!existsSync(dir) || !statSync(dir).isDirectory()) {
    return []
  }
  return readdirSync(dir)
    .filter(isLicence)
    .filter((file) => statSync(join(dir, file)).isFile())
    .sort()
}

/**
 * Reads one installed package's notice, in Vite's entry format.
 *
 * Unlike Vite, which takes the first match in the file system's listing
 * order, it takes the first licence file in name order, so a package with
 * two (`LICENSE` and `LICENSE-MIT`) gets the same text on every machine.
 * Like Vite's package reader, it ignores a byte-order mark at the start of
 * `package.json`.
 *
 * @param {string} packageDir - the package directory.
 * @returns {NoticeEntry} its name, version, `license` field and the trimmed
 *   text of its licence file (the first file, in name order, whose name
 *   starts with `license`, `licence` or `copying`, in any case).
 * @throws {Error} when `package.json` is missing, is not JSON or has no name.
 */
export function readNoticeEntry(packageDir) {
  const manifestPath = join(packageDir, "package.json")
  /** @type {{ name?: unknown, version?: unknown, license?: unknown }} */
  const manifest = JSON.parse(
    readFileSync(manifestPath, "utf8").replace(/^\uFEFF/, "")
  )
  if (typeof manifest.name !== "string" || manifest.name === "") {
    throw new Error(
      `${manifestPath} has no "name", so its notice cannot name the package.`
    )
  }
  /** @type {NoticeEntry} */
  const entry = {
    name: manifest.name,
    version: typeof manifest.version === "string" ? manifest.version : "0.0.0",
  }
  if (typeof manifest.license === "string" && manifest.license.trim() !== "") {
    entry.identifier = manifest.license.trim()
  }
  const [licenceFile] = licenceFilesIn(packageDir, isLicenceFileName)
  if (licenceFile) {
    entry.text = readFileSync(join(packageDir, licenceFile), "utf8").trim()
  }
  return entry
}

/**
 * Reads the licence files of the code copied into the admin's source that
 * bundled modules sit beside.
 *
 * Copied code keeps its licence file next to it (ADR 0012 §4), for example
 * `src/components/admin/LICENSE`, so every licence file in a module's
 * directory or in a parent directory up to `src/` covers code of the bundle.
 * Code whose licence file lies elsewhere (`src/lib/utils.ts`, covered by
 * `src/components/admin/LICENSE`) is covered as long as code beside that
 * file is bundled too; `licence-notices.build.test.mjs` checks every path
 * of `THIRD_PARTY.md` against the result.
 *
 * @param {string} root - the Vite root, the `admin/` directory.
 * @param {Iterable<string>} files - the source files of bundled modules,
 *   absolute; files outside `root/src` are ignored.
 * @returns {CopiedLicence[]} each licence file once, with its path relative
 *   to `root` and its trimmed text, sorted by path.
 */
export function readCopiedLicences(root, files) {
  const top = resolve(root, SOURCE_DIR)
  /** @type {Set<string>} */
  const dirs = new Set()
  for (const file of files) {
    for (const dir of ancestorDirs(file, top)) dirs.add(dir)
  }
  /** @type {CopiedLicence[]} */
  const licences = []
  for (const dir of dirs) {
    for (const name of licenceFilesIn(dir, isCopiedLicenceFileName)) {
      const file = join(dir, name)
      licences.push({
        path: relative(resolve(root), file).split(sep).join("/"),
        text: readFileSync(file, "utf8").trim(),
      })
    }
  }
  return licences.sort((a, b) =>
    a.path < b.path ? -1 : a.path > b.path ? 1 : 0
  )
}

/**
 * The text of an emitted asset.
 *
 * @param {string | Uint8Array} source - the asset's source, which Rolldown
 *   types as either.
 * @returns {string} the string itself, or the bytes decoded as UTF-8 (the
 *   encoding Vite writes the notices file in).
 */
export function assetText(source) {
  return typeof source === "string" ? source : new TextDecoder().decode(source)
}

/**
 * The plugin that completes Vite's notices file: the stylesheet and asset
 * packages, then the licence files of the copied code.
 *
 * @param {{ fileName: string }} options - `fileName`: the notices file,
 *   relative to `dist/`; the same value as `build.license.fileName`.
 * @returns {import("vite").Plugin} a build-only plugin. Its `generateBundle`
 *   runs after Vite's (`order: "post"`), reads the `@import` rules of every
 *   stylesheet module of the bundle and the source paths of every asset, and
 *   rewrites the notices file with {@link addNotices}, then with
 *   {@link addCopiedCode} for the licence files that
 *   {@link readCopiedLicences} finds beside the bundled modules of `src/`.
 *   It fails the build when Vite wrote no notices file, or when an imported
 *   package cannot be found, so a misconfigured build never ships without
 *   notices.
 */
export function licenceNotices({ fileName }) {
  let root = process.cwd()
  return {
    name: "backend3:licence-notices",
    apply: "build",
    configResolved(config) {
      root = config.root
    },
    generateBundle: {
      order: "post",
      handler(_options, bundle) {
        const notices = bundle[fileName]
        if (notices?.type !== "asset") {
          this.error(
            `Vite wrote no ${fileName}. Set build.license to { fileName: "${fileName}" } in vite.config.ts, so admin/dist carries the licence notices of the bundled packages (ADR 0038).`
          )
        }
        /** @type {Map<string, string>} package directory to package name */
        const packages = new Map()
        /** @type {Set<string>} the source files of every bundled module */
        const sources = new Set()
        for (const output of Object.values(bundle)) {
          if (output.type === "asset") {
            for (const original of output.originalFileNames) {
              const owner = packageOfPath(original)
              if (owner) {
                packages.set(resolve(root, owner.dir), owner.name)
              }
            }
            continue
          }
          for (const id of output.moduleIds) {
            // Virtual modules (`\0` ids: Vite's module-preload polyfill,
            // Rolldown's runtime helpers) have no file of their own; Vite's
            // own scan skips them too (ADR 0038 §1).
            if (id.startsWith("\0")) continue
            const file = id.split("?")[0]
            sources.add(file)
            if (!file.endsWith(".css") || !existsSync(file)) continue
            for (const specifier of cssImportSpecifiers(
              readFileSync(file, "utf8")
            )) {
              const name = packageOfSpecifier(specifier)
              if (name === null) continue
              const dir = findPackageDir(dirname(file), name)
              if (dir === null) {
                this.error(
                  `${file} imports "${specifier}", but no node_modules/${name}/package.json is found above it, so the licence notices cannot name the package it bundles. Install the package (npm ci) or fix the import.`
                )
              }
              packages.set(dir, name)
            }
          }
        }
        const entries = [...packages.keys()].map(readNoticeEntry)
        const withPackages = addNotices(assetText(notices.source), entries)
        notices.source = addCopiedCode(
          withPackages,
          readCopiedLicences(root, sources)
        )
      },
    },
  }
}
