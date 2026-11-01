/**
 * Files under `/storage` in three browser engines (editor.md §13 P9 and §16
 * item 4; ADR 0013 §7; ADR 0015 §4).
 *
 * The E2E server stands in for the backend3 host and serves `/storage`
 * itself with the fallback router (`config/e2e.yaml`,
 * `src/controllers/storage.rs`), from its media root. The tests write their
 * files straight into that root, as a file that reached the volume some
 * other way would sit there: the upload endpoints refuse SVG and HTML.
 *
 * What they show, in Chromium, Firefox and WebKit (the `storage-*` projects
 * of `playwright.config.ts`), each file opened in a fresh page the way a
 * link opens it, as the top-level document:
 *
 * - **No script runs.** An `.svg` and an `.html` file with an inline script
 *   and an event handler run neither, as served, nor with
 *   `Content-Disposition` removed on the way, where the sandbox policy is
 *   the only layer left. A control with both headers removed shows that the
 *   scripts do run without them, so the detector works.
 * - **How each file opens, recorded per engine.** Whether the SVG, the HTML
 *   and a PDF are shown inline or downloaded, the PDF both as served (no
 *   sandbox) and with the sandbox policy added on the way. [`OUTCOMES`]
 *   holds what each engine did when this spec was written, and editor.md
 *   §16 item 4 records the same as the evidence for the PDF exemption. A
 *   browser update that changes an outcome fails here; the record is then
 *   measured again and updated.
 *
 * These tests do not use the CSP and console guard of `fixtures.ts`: a
 * sandboxed document reports the scripts it blocks as console errors and
 * CSP violations, which is exactly what is being tested.
 */
import { randomUUID } from "node:crypto"
import { mkdir, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { expect, test, type BrowserContext, type Page } from "@playwright/test"

/**
 * The E2E server's media root, `settings.media.root` of `config/e2e.yaml`:
 * `target/media/e2e/public` below the repository root, where CI and
 * AGENTS.md start the server.
 */
const MEDIA_ROOT = fileURLToPath(
  new URL("../../target/media/e2e/public/", import.meta.url)
)

/** What a fixture's scripts log when they run. */
const RAN = "storage-fixture-script-ran"

/** The sandbox of ADR 0013 §7, as the router sends it. */
const SANDBOX_CSP =
  "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox"

/** An SVG with a script element and an `onload` handler. */
const SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200" onload="console.log('${RAN}: onload')">
  <script>console.log("${RAN}: script element")</script>
  <rect width="200" height="200" fill="black"/>
</svg>
`

/** An HTML page with a script element and an `onerror` handler. */
const HTML = `<!doctype html>
<html lang="hu">
  <head>
    <meta charset="utf-8">
    <title>Teszt</title>
    <script>console.log("${RAN}: script element")</script>
  </head>
  <body>
    <img src="nincs.png" alt="" onerror="console.log('${RAN}: onerror')">
    <p>Teszt</p>
  </body>
</html>
`

/** The two fixtures that carry script: file name, body and served type. */
const SCRIPTED = [
  ["rajz.svg", SVG, "image/svg+xml"],
  ["oldal.html", HTML, "text/html"],
] as const

/**
 * A one-page PDF, 200 × 200 points with a black square, built with a
 * correct cross-reference table, so every viewer opens it without repair.
 */
function onePagePdf(): Buffer {
  const content = "0 0 0 rg 20 20 160 160 re f\n"
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>",
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
  ]
  let pdf = "%PDF-1.4\n"
  const offsets = objects.map((body, index) => {
    const offset = pdf.length
    pdf += `${index + 1} 0 obj\n${body}\nendobj\n`
    return offset
  })
  const xref = pdf.length
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`
  for (const offset of offsets) {
    pdf += `${String(offset).padStart(10, "0")} 00000 n \n`
  }
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`
  return Buffer.from(pdf, "latin1")
}

/** How a top-level navigation to a file ended. */
type Opened = "inline" | "download"

/** The engines of the `storage-*` projects. */
type Engine = "chromium" | "firefox" | "webkit"

/** How one engine opened each file, as recorded. */
interface Outcomes {
  /** The SVG as served (sandboxed, `attachment`). */
  readonly svg: Opened
  /** The HTML as served (sandboxed, `attachment`). */
  readonly html: Opened
  /** The PDF as served (no sandbox, shown inline by design). */
  readonly pdf: Opened
  /** The PDF with the sandbox policy added on the way. */
  readonly sandboxedPdf: Opened
}

/**
 * What each engine did when this spec was written (editor.md §16 item 4
 * records the same). Chromium and Firefox download the two `attachment`
 * files and show the PDF in their viewer, also under `sandbox`. Playwright's
 * WebKit for Linux shows SVG and HTML inline despite `attachment`, so there
 * the sandbox is the only layer, and it has no PDF viewer, so it downloads
 * every PDF: its PDF row says nothing about Safari.
 */
const OUTCOMES: Record<Engine, Outcomes> = {
  chromium: {
    svg: "download",
    html: "download",
    pdf: "inline",
    sandboxedPdf: "inline",
  },
  firefox: {
    svg: "download",
    html: "download",
    pdf: "inline",
    sandboxedPdf: "inline",
  },
  webkit: {
    svg: "inline",
    html: "inline",
    pdf: "download",
    sandboxedPdf: "download",
  },
}

/**
 * Writes `files` into a new folder of the media root and returns the
 * absolute URL of each, under `/storage`; `remove` deletes the folder again.
 *
 * - `files`: file names and their bytes.
 * - `baseURL`: the project's base URL, whose origin serves `/storage`.
 */
async function storageFiles(
  files: Record<string, string | Buffer>,
  baseURL: string | undefined
) {
  const folder = `uploads/e2e-storage/${randomUUID()}`
  const directory = path.join(MEDIA_ROOT, folder)
  await mkdir(directory, { recursive: true })
  const urls: Record<string, string> = {}
  for (const [name, bytes] of Object.entries(files)) {
    await writeFile(path.join(directory, name), bytes)
    urls[name] = new URL(`/storage/${folder}/${name}`, baseURL).href
  }
  return {
    urls,
    remove: () => rm(directory, { recursive: true, force: true }),
  }
}

/**
 * Changes the headers of every answer to `url` in `context` on its way to
 * the page: drops `drop` and adds `add`. The router's own answer is fetched
 * as it is, so everything else stays as served.
 *
 * - `context`: the browser context whose requests are rewritten.
 * - `url`: the absolute URL of the file.
 * - `drop`: lower-case names of headers to remove.
 * - `add`: headers to set.
 */
async function rewriteHeaders(
  context: BrowserContext,
  url: string,
  drop: readonly string[],
  add: Record<string, string> = {}
): Promise<void> {
  await context.route(url, async (route) => {
    const response = await route.fetch()
    const headers = { ...response.headers(), ...add }
    for (const name of drop) {
      delete headers[name]
    }
    await route.fulfill({ response, headers })
  })
}

/** One file opened in a fresh page. */
interface Visit {
  /** Whether the browser showed the file or downloaded it. */
  readonly opened: Opened
  /** The page, still open, for checks of a shown file. */
  readonly page: Page
  /** Every console line a fixture's script wrote in that page. */
  readonly ran: readonly string[]
}

/**
 * Opens `url` in a new page of `context`, as the top-level document, the
 * way a link opens it. A shown file must answer `200` and be a document of
 * `contentType`.
 *
 * - `context`: the browser context to open the page in.
 * - `url`: the file's absolute URL.
 * - `contentType`: the type the shown document must have.
 */
async function visit(
  context: BrowserContext,
  url: string,
  contentType: string
): Promise<Visit> {
  const page = await context.newPage()
  const ran: string[] = []
  page.on("console", (message) => {
    if (message.text().includes(RAN)) {
      ran.push(message.text())
    }
  })
  try {
    const response = await page.goto(url, { waitUntil: "load" })
    expect(response?.status(), url).toBe(200)
  } catch (error) {
    // Playwright rejects a navigation that turned into a download. WebKit
    // fires no download event for a routed answer, so the rejection itself
    // is the evidence.
    if (
      error instanceof Error &&
      error.message.includes("Download is starting")
    ) {
      return { opened: "download", page, ran }
    }
    throw error
  }
  // A round trip to the page after `load`: console messages that the
  // document's scripts wrote before it have arrived by then.
  expect(await page.evaluate(() => document.contentType), url).toBe(contentType)
  return { opened: "inline", page, ran }
}

/**
 * Waits until the engine's PDF viewer has drawn the page, beyond the
 * document type: Chromium's viewer is an extension frame, Firefox's pdf.js
 * marks each page it has rendered.
 *
 * - `page`: the page showing the PDF.
 * - `engine`: the browser engine.
 */
async function expectPdfViewer(page: Page, engine: Engine): Promise<void> {
  if (engine === "chromium") {
    await expect
      .poll(() =>
        page
          .frames()
          .some((frame) => frame.url().startsWith("chrome-extension://"))
      )
      .toBe(true)
  } else if (engine === "firefox") {
    await expect(
      page.locator('.page[data-loaded="true"]').first()
    ).toBeVisible()
  }
}

/** The engine of the running project. */
function engineOf(browserName: string): Engine {
  if (
    browserName === "chromium" ||
    browserName === "firefox" ||
    browserName === "webkit"
  ) {
    return browserName
  }
  throw new Error(`no recorded outcome for ${browserName}`)
}

/** The message of a recorded outcome that no longer holds. */
const RECORD =
  "differs from editor.md §16 item 4 and OUTCOMES; measure again and update both"

test.describe("files under /storage on the backend3 host", () => {
  for (const [name, bytes, type] of SCRIPTED) {
    test(`${name} runs no script as served, nor under the sandbox alone`, async ({
      context,
      baseURL,
      browserName,
    }) => {
      const engine = engineOf(browserName)
      const files = await storageFiles({ [name]: bytes }, baseURL)
      try {
        const url = files.urls[name]
        const served = await visit(context, url, type)
        expect(served.ran, "a script ran as served").toEqual([])
        const kind = name.endsWith(".svg") ? "svg" : "html"
        expect(served.opened, `${engine} ${kind} ${RECORD}`).toBe(
          OUTCOMES[engine][kind]
        )

        // Without the download header every engine shows the file, and the
        // sandbox policy alone keeps every script from running.
        await rewriteHeaders(context, url, ["content-disposition"])
        const sandboxed = await visit(context, url, type)
        expect(sandboxed.opened).toBe("inline")
        expect(sandboxed.ran, "a script ran under the sandbox").toEqual([])
      } finally {
        await files.remove()
      }
    })

    test(`${name} runs its scripts once both headers are removed (the control)`, async ({
      context,
      baseURL,
    }) => {
      const files = await storageFiles({ [name]: bytes }, baseURL)
      try {
        const url = files.urls[name]
        await rewriteHeaders(context, url, [
          "content-disposition",
          "content-security-policy",
        ])
        const bare = await visit(context, url, type)
        expect(bare.opened).toBe("inline")
        // The script element and the event handler both ran: the detector
        // above would have seen a script that escaped.
        await expect.poll(() => bare.ran.length).toBe(2)
      } finally {
        await files.remove()
      }
    })
  }

  test("a PDF opens as recorded, as served and under the sandbox", async ({
    context,
    baseURL,
    browserName,
  }) => {
    const engine = engineOf(browserName)
    const files = await storageFiles(
      { "tajekoztato.pdf": onePagePdf() },
      baseURL
    )
    try {
      const url = files.urls["tajekoztato.pdf"]
      const served = await visit(context, url, "application/pdf")
      if (served.opened === "inline") {
        await expectPdfViewer(served.page, engine)
      }

      await rewriteHeaders(context, url, [], {
        "content-security-policy": SANDBOX_CSP,
      })
      const sandboxed = await visit(context, url, "application/pdf")
      if (sandboxed.opened === "inline") {
        await expectPdfViewer(sandboxed.page, engine)
      }

      test.info().annotations.push({
        type: "pdf",
        description: `${engine}: as served ${served.opened}, under sandbox ${sandboxed.opened}`,
      })
      expect(
        { pdf: served.opened, sandboxedPdf: sandboxed.opened },
        `${engine} PDF ${RECORD}`
      ).toEqual({
        pdf: OUTCOMES[engine].pdf,
        sandboxedPdf: OUTCOMES[engine].sandboxedPdf,
      })
    } finally {
      await files.remove()
    }
  })

  test("the PDF fixture's cross-reference table points at its objects", () => {
    const pdf = onePagePdf().toString("latin1")
    const xref = Number(/startxref\n(\d+)\n/.exec(pdf)?.[1])
    expect(pdf.slice(xref, xref + 4)).toBe("xref")
    const entries = [...pdf.matchAll(/^(\d{10}) 00000 n $/gm)]
    expect(entries).toHaveLength(4)
    entries.forEach(([, offset], index) => {
      expect(pdf.slice(Number(offset)).startsWith(`${index + 1} 0 obj\n`)).toBe(
        true
      )
    })
  })
})
