import { fireEvent, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { describe, expect, it, vi } from "vitest"

import { hu } from "@/i18n/hu"
import { renderHungarian } from "@/test/render"
import { youtubeFixture } from "@/test/youtube-fixtures"

import { CONTENT_V1_POLICY } from "../policy"
import { youtubeRules } from "../youtube-input"
import { YoutubeEmbedDialog } from "./YoutubeEmbedDialog"

const RULES = youtubeRules(CONTENT_V1_POLICY)
const T = hu.editor.youtube

/** Renders the open dialog and returns its insert callback. */
function renderDialog() {
  const onInsert = vi.fn()
  const onOpenChange = vi.fn()
  renderHungarian(
    <YoutubeEmbedDialog
      open
      onOpenChange={onOpenChange}
      rules={RULES}
      onInsert={onInsert}
    />
  )
  return { onInsert, onOpenChange }
}

const field = () => screen.getByLabelText(T.input_label)
const titleField = () => screen.getByLabelText(T.title_label)
const insert = () => screen.getByRole("button", { name: T.insert })
/** Puts text into the field at once, as a paste does. */
const paste = (text: string) =>
  fireEvent.change(field(), { target: { value: text } })

describe("YoutubeEmbedDialog", () => {
  it("shows its Hungarian title, an empty field and the title „Videó” (D135)", () => {
    renderDialog()
    expect(screen.getByRole("dialog")).toHaveTextContent(T.title)
    expect(field()).toHaveValue("")
    expect(titleField()).toHaveValue("Videó")
    expect(insert()).toBeDisabled()
  })

  it("inserts a share link with its start, titled „Videó” (D130, D131)", async () => {
    const { onInsert, onOpenChange } = renderDialog()
    paste(youtubeFixture("youtube-share-link-video-start"))
    expect(insert()).toBeEnabled()
    await userEvent.click(insert())
    expect(onInsert).toHaveBeenCalledWith({
      videoId: "dQw4w9WgXcQ",
      start: "90",
      list: null,
      title: "Videó",
    })
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it("takes YouTube's embed code, and replaces its generic title (D128, D135)", async () => {
    const { onInsert } = renderDialog()
    paste(youtubeFixture("youtube-embed-dialog-playlist"))
    expect(titleField()).toHaveValue("Videó")
    await userEvent.click(insert())
    expect(onInsert).toHaveBeenCalledWith({
      videoId: "videoseries",
      start: null,
      list: "PLMOZDTW7ZvXM",
      title: "Videó",
    })
  })

  it("fills the title from embed code with a real title, until it is edited", async () => {
    const { onInsert } = renderDialog()
    paste(youtubeFixture("youtube-embed-context-menu-video"))
    expect(titleField()).toHaveValue(
      "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)"
    )
    await userEvent.clear(titleField())
    await userEvent.type(titleField(), "A ballagás")
    paste(youtubeFixture("youtube-embed-context-menu-playlist"))
    expect(titleField()).toHaveValue("A ballagás")
    await userEvent.click(insert())
    expect(onInsert).toHaveBeenCalledWith({
      videoId: "71ya_JC_IbM",
      start: null,
      list: "PLMOZDTW7ZvXM",
      title: "A ballagás",
    })
  })

  it("stores „Videó” for a blank title", async () => {
    const { onInsert } = renderDialog()
    paste("https://youtu.be/dQw4w9WgXcQ")
    await userEvent.clear(titleField())
    await userEvent.click(insert())
    expect(onInsert).toHaveBeenCalledWith(
      expect.objectContaining({ title: "Videó" })
    )
  })

  it.each([
    "youtube-embed-dialog-controls-off",
    "youtube-embed-dialog-all-options",
    "youtube-embed-dialog-playlist-all-options",
    "youtube-documented-embed-start-end",
    "youtube-documented-embed-loop",
    "youtube-help-autoplay",
  ])(
    "shows the dropped-options notice under the field for %s (D134)",
    (name) => {
      renderDialog()
      paste(youtubeFixture(name))
      expect(screen.getByText(T.options_dropped)).toBeInTheDocument()
      expect(insert()).toBeEnabled()
    }
  )

  it("shows no notice for si alone", () => {
    renderDialog()
    paste(youtubeFixture("youtube-embed-dialog-default"))
    expect(screen.queryByText(T.options_dropped)).not.toBeInTheDocument()
  })

  it.each([
    ["https://vimeo.com/123", T.not_youtube],
    ["https://youtu.be/abc", T.video_id],
    ["https://www.youtube.com/embed?listType=user_uploads&list=x", T.list_type],
    ["https://www.youtube.com/embed?listType=search&list=x", T.list_type],
    ['<iframe src="https://padlet.com/embed/KEY"></iframe>', T.not_youtube],
  ])("refuses %s with its message (D129, D132)", async (text, message) => {
    const { onInsert } = renderDialog()
    paste(text)
    expect(screen.getByText(message)).toBeInTheDocument()
    expect(field()).toHaveAttribute("aria-invalid", "true")
    expect(insert()).toBeDisabled()
    fireEvent.submit(insert().closest("form")!)
    expect(onInsert).not.toHaveBeenCalled()
  })

  it("starts over at every opening", async () => {
    function Harness() {
      const [open, setOpen] = useState(true)
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            open
          </button>
          <YoutubeEmbedDialog
            open={open}
            onOpenChange={setOpen}
            rules={RULES}
            onInsert={vi.fn()}
          />
        </>
      )
    }
    renderHungarian(<Harness />)
    paste("https://youtu.be/dQw4w9WgXcQ")
    await userEvent.keyboard("{Escape}")
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "open" }))
    expect(field()).toHaveValue("")
  })
})
