import { screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { hu } from "@/i18n/hu"
import { renderHungarian } from "@/test/render"

import { ImageAltDialog } from "./ImageAltDialog"

const T = hu.editor.image

function renderDialog(alt: string | null) {
  const onSave = vi.fn()
  const onOpenChange = vi.fn()
  renderHungarian(
    <ImageAltDialog
      open
      onOpenChange={onOpenChange}
      alt={alt}
      onSave={onSave}
    />
  )
  return { onSave, onOpenChange }
}

describe("ImageAltDialog", () => {
  it("explains the alt text and starts from the image's own", () => {
    renderDialog("régi szöveg")
    expect(screen.getByRole("dialog")).toHaveTextContent(T.alt_title)
    expect(screen.getByRole("dialog")).toHaveTextContent(T.alt_description)
    expect(screen.getByLabelText(T.alt_label)).toHaveValue("régi szöveg")
  })

  it("saves the typed text and closes", async () => {
    const { onSave, onOpenChange } = renderDialog(null)
    expect(screen.getByLabelText(T.alt_label)).toHaveValue("")
    await userEvent.type(
      screen.getByLabelText(T.alt_label),
      "A díjazottak a színpadon"
    )
    await userEvent.click(
      screen.getByRole("button", { name: hu.ra.action.save })
    )
    expect(onSave).toHaveBeenCalledWith("A díjazottak a színpadon")
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })
})
