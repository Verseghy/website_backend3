/**
 * The development-only clipboard recorder in a real browser (editor.md
 * §13.1). jsdom has no `DataTransfer`, so only this test shows that a real
 * one fits the capture code and that the download passes the admin CSP. The
 * paste is a `ClipboardEvent` with a constructed `DataTransfer`, as in the
 * editor's paste tests (editor.md §13.5), since a test cannot drive the
 * system clipboard reliably. It needs a build with `VITE_CLIPBOARD_RECORDER=1`,
 * as CI's E2E build is; the default build has no recorder
 * (`src/recorder/production-bundle.test.ts`).
 */
import { readFile } from "node:fs/promises"

import { expect, test } from "./fixtures.ts"

test.describe("the clipboard recorder", () => {
  test("downloads a scrubbed JSON capture of a paste", async ({ page }) => {
    await page.goto("/admin/clipboard-recorder")
    await expect(
      page.getByRole("heading", { name: "Vágólap-rögzítő" }),
      "the recorder is missing: build admin/dist with VITE_CLIPBOARD_RECORDER=1"
    ).toBeVisible()
    const area = page.getByRole("textbox", { name: "Beillesztési mező" })

    const downloaded = page.waitForEvent("download")
    // An invented profile name; Word links pictures into the profile's Temp.
    await area.evaluate((element) => {
      const data = new DataTransfer()
      data.setData(
        "text/html",
        '<p>Eredmény</p><img src="file:///C:/Users/Teszt%20Elek/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png">'
      )
      data.setData("text/plain", "Eredmény\r\nC:\\Users\\Teszt Elek\\Asztal")
      data.setData("text/rtf", "{\\rtf1 never recorded}")
      data.items.add(
        new File([new Uint8Array([137, 80, 78, 71])], "kep.png", {
          type: "image/png",
        })
      )
      element.dispatchEvent(
        new ClipboardEvent("paste", {
          clipboardData: data,
          bubbles: true,
          cancelable: true,
        })
      )
    })
    const download = await downloaded

    expect(download.suggestedFilename()).toMatch(
      /^clipboard-capture-\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-\d{3}Z\.json$/
    )
    const text = await readFile(await download.path(), "utf8")
    expect(JSON.parse(text)).toEqual({
      format: "backend3-clipboard-capture",
      version: 1,
      capturedAt: expect.stringMatching(/^\d{4}-\d\d-\d\dT.*Z$/),
      userAgent: expect.stringContaining("Chrome"),
      types: ["text/html", "text/plain", "text/rtf", "Files"],
      html: '<p>Eredmény</p><img src="file:///C:/Users/USER/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png">',
      text: "Eredmény\r\nC:\\Users\\USER\\Asztal",
      files: [{ type: "image/png", size: 4 }],
      scrubbedPaths: 2,
    })
    expect(text).not.toMatch(/Teszt|rtf1|kep\.png/)

    // Nothing was pasted into the area, and the status names the file.
    await expect(area).toBeEmpty()
    await expect(page.getByRole("status")).toContainText(
      download.suggestedFilename()
    )
  })
})
