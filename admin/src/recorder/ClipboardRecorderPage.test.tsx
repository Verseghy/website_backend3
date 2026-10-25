import { fireEvent, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { renderHungarian } from "@/test/render"

import type { ClipboardSource } from "./capture"
import {
  ClipboardRecorderPage,
  OBJECT_URL_LIFETIME_MS,
  saveTextFile,
} from "./ClipboardRecorderPage"

const AT = new Date("2026-10-25T08:30:00.000Z")
const FILE_NAME = "clipboard-capture-2026-10-25T08-30-00-000Z.json"

/** An Excel-like clipboard with a profile path in its HTML; invented data. */
const EXCEL: ClipboardSource = {
  types: ["text/html", "text/plain", "text/rtf", "Files"],
  getData: (format) =>
    ({
      "text/html":
        '<link rel=File-List href="file:///C:/Users/teszt/AppData/Local/Temp/msohtmlclip1/01/clip_filelist.xml"><table><tr><td>42</td></tr></table>',
      "text/plain": "42",
    })[format] ?? "",
  files: [{ type: "image/png", size: 12_400 }],
}

/**
 * Mounts the recorder with a fixed clock and a recording `saveFile`.
 *
 * @returns the `saveFile` mock.
 */
function mountRecorder() {
  const saveFile = vi.fn<(name: string, contents: string) => void>()
  renderHungarian(
    <ClipboardRecorderPage
      userAgent="test-agent/1.0"
      now={() => AT}
      saveFile={saveFile}
    />
  )
  return saveFile
}

/** The paste area, found by its Hungarian accessible name. */
const pasteArea = () =>
  screen.getByRole("textbox", { name: "Beillesztési mező" })

describe("ClipboardRecorderPage", () => {
  it("explains itself in Hungarian before anything is pasted", () => {
    mountRecorder()
    expect(
      screen.getByRole("heading", { name: "Vágólap-rögzítő" })
    ).toBeInTheDocument()
    expect(screen.getByText(/RTF-et és fájltartalmat nem ment/)).toBeVisible()
    expect(
      screen.getByText(/nézd át, mielőtt a tesztek közé kerül/)
    ).toBeVisible()
    expect(pasteArea()).toHaveAttribute("contenteditable", "true")
    expect(pasteArea()).toHaveAccessibleDescription(/Ctrl\+V/)
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
  })

  it("downloads the scrubbed capture of a paste, inserts nothing and says so", () => {
    const saveFile = mountRecorder()

    const notCancelled = fireEvent.paste(pasteArea(), { clipboardData: EXCEL })

    expect(notCancelled).toBe(false)
    expect(pasteArea()).toBeEmptyDOMElement()
    expect(saveFile).toHaveBeenCalledTimes(1)
    const [name, contents] = saveFile.mock.calls[0]
    expect(name).toBe(FILE_NAME)
    expect(JSON.parse(contents)).toMatchObject({
      format: "backend3-clipboard-capture",
      capturedAt: AT.toISOString(),
      userAgent: "test-agent/1.0",
      types: ["text/html", "text/plain", "text/rtf", "Files"],
      text: "42",
      files: [{ type: "image/png", size: 12_400 }],
      scrubbedPaths: 1,
    })
    expect(contents).toContain("file:///C:/Users/USER/AppData")
    expect(contents).not.toContain("teszt")
    expect(screen.getByRole("status")).toHaveTextContent(
      `Letöltve: ${FILE_NAME}. Típusok: text/html, text/plain, text/rtf, Files. Kitakart mappanevek: 1.`
    )
  })

  it("explains a paste event that carries no clipboard data", () => {
    const saveFile = mountRecorder()

    fireEvent.paste(pasteArea())

    expect(screen.getByRole("alert")).toHaveTextContent(
      "A böngésző nem adta át a vágólap tartalmát."
    )
    expect(saveFile).not.toHaveBeenCalled()
  })
})

describe("saveTextFile", () => {
  it("clicks a temporary download link and revokes its URL later", async () => {
    const URL = {
      createObjectURL: vi.fn<(blob: Blob) => string>(() => "blob:capture"),
      revokeObjectURL: vi.fn(),
    }
    const setTimeout = vi.fn<(handler: () => void, timeout: number) => void>()
    const clicked: HTMLAnchorElement[] = []
    // Cancel the click, so jsdom does not try to navigate to the blob URL.
    const onClick = (event: Event) => {
      clicked.push(event.target as HTMLAnchorElement)
      event.preventDefault()
    }
    document.addEventListener("click", onClick)

    saveTextFile({ document, URL, setTimeout }, FILE_NAME, '{"a":1}\n')
    document.removeEventListener("click", onClick)

    expect(clicked).toHaveLength(1)
    expect(clicked[0].download).toBe(FILE_NAME)
    expect(clicked[0].getAttribute("href")).toBe("blob:capture")
    expect(clicked[0].isConnected).toBe(false)
    const blob = URL.createObjectURL.mock.calls[0][0]
    expect(blob.type).toBe("application/json")
    expect(await blob.text()).toBe('{"a":1}\n')

    expect(URL.revokeObjectURL).not.toHaveBeenCalled()
    const [[revoke, delay]] = setTimeout.mock.calls
    expect(delay).toBe(OBJECT_URL_LIFETIME_MS)
    revoke()
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:capture")
  })
})
