import { act, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { Editor } from "@tiptap/react"
import { EditorContent, useEditor } from "@tiptap/react"
import { describe, expect, it } from "vitest"

import { hu } from "@/i18n/hu"
import { renderHungarian } from "@/test/render"

import type { SiteImageAttributes } from "../image-attributes"
import { CONTENT_V1_POLICY } from "../policy"
import { toStoredHtml } from "../serialize/to-stored-html"
import { siteExtensions } from "../site-extensions"

const T = hu.editor.image
const LOCKED = "display: block; height: auto; margin: 0 auto; max-width: 100%"

/** An editor with the site schema, shown with its node views. */
function renderEditor(content: string) {
  let editor: Editor | null = null
  function EditorUnderTest() {
    editor = useEditor({
      extensions: siteExtensions({ policy: CONTENT_V1_POLICY }),
      content,
    })
    return <EditorContent editor={editor} />
  }
  renderHungarian(<EditorUnderTest />)
  return {
    attributes: () =>
      editor!.state.doc.firstChild!.attrs as SiteImageAttributes,
    stored: () =>
      toStoredHtml(editor!.state.doc, { policy: CONTENT_V1_POLICY, document }),
    editor: () => editor!,
  }
}

const IMAGE = `<p><img src="/storage/uploads/a.jpg" alt="kép" width="350" height="175" style="${LOCKED}; width: 350px"></p>`
const NO_ALT = `<p><img src="/storage/uploads/a.jpg" width="350" height="175" style="${LOCKED}; width: 350px"></p>`

describe("SiteImageView", () => {
  it("shows the image in a paragraph, at its display width", async () => {
    renderEditor(IMAGE)
    const image = await screen.findByRole("img", { name: "kép" })
    expect(image).toHaveAttribute("src", "/storage/uploads/a.jpg")
    expect(image.style.width).toBe("350px")
    expect(image.closest("p")).not.toBeNull()
    expect(screen.queryByText(T.alt_missing)).not.toBeInTheDocument()
  })

  it("shows the red badge while the alt text is missing (POS-C03)", async () => {
    renderEditor(NO_ALT)
    expect(
      await screen.findByRole("button", { name: T.alt_missing })
    ).toBeInTheDocument()
  })

  it("sets the alt text from the dialog, which removes the badge", async () => {
    const view = renderEditor(NO_ALT)
    await userEvent.click(
      await screen.findByRole("button", { name: T.alt_missing })
    )
    await userEvent.type(screen.getByLabelText(T.alt_label), "Az udvar")
    await userEvent.click(
      screen.getByRole("button", { name: hu.ra.action.save })
    )
    expect(view.attributes().alt).toBe("Az udvar")
    expect(screen.queryByText(T.alt_missing)).not.toBeInTheDocument()
    expect(view.stored()).toBe(
      `<p><img src="/storage/uploads/a.jpg" alt="Az udvar" width="350" height="175" style="${LOCKED}; width: 350px"></p>`
    )
  })

  it("offers the four width presets, the image's own pressed", async () => {
    const view = renderEditor(IMAGE)
    const group = await screen.findByRole("group", { name: T.sizes })
    const presets = within(group).getAllByRole("button")
    expect(presets.map((button) => button.textContent)).toEqual([
      "175 px",
      "350 px",
      "525 px",
      "700 px",
    ])
    expect(
      within(group).getByRole("button", { name: "350 px" })
    ).toHaveAttribute("aria-pressed", "true")
    await userEvent.click(within(group).getByRole("button", { name: "700 px" }))
    expect(view.attributes().displayWidth).toBe(700)
    expect(view.stored()).toBe(
      `<p><img src="/storage/uploads/a.jpg" alt="kép" width="700" height="350" style="${LOCKED}; width: 700px"></p>`
    )
  })

  it("shows an upload that is running or failed", async () => {
    const view = renderEditor(IMAGE)
    await screen.findByRole("img", { name: "kép" })
    act(() => {
      view.editor().commands.command(({ tr }) => {
        tr.setNodeAttribute(0, "uploadState", "pending")
        return true
      })
    })
    expect(await screen.findByRole("status")).toHaveTextContent(T.uploading)
    act(() => {
      view.editor().commands.command(({ tr }) => {
        tr.setNodeAttribute(0, "uploadState", "failed")
        return true
      })
    })
    expect(await screen.findByRole("alert")).toHaveTextContent(T.upload_failed)
    // Neither is stored.
    expect(view.stored()).toBe(IMAGE)
  })
})
