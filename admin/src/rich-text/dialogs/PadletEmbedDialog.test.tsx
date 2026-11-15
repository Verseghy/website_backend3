import { fireEvent, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { hu } from "@/i18n/hu"
import { renderHungarian } from "@/test/render"

import { padletRules } from "../padlet-input"
import { CONTENT_V1_POLICY } from "../policy"
import { PadletEmbedDialog } from "./PadletEmbedDialog"

const RULES = padletRules(CONTENT_V1_POLICY)
const T = hu.editor.padlet

function renderDialog() {
  const onInsert = vi.fn()
  const onOpenChange = vi.fn()
  renderHungarian(
    <PadletEmbedDialog
      open
      onOpenChange={onOpenChange}
      rules={RULES}
      onInsert={onInsert}
    />
  )
  return { onInsert, onOpenChange }
}

const field = () => screen.getByLabelText(T.input_label)
const insert = () => screen.getByRole("button", { name: T.insert })

describe("PadletEmbedDialog", () => {
  it("shows its Hungarian title and an empty field", () => {
    renderDialog()
    expect(screen.getByRole("dialog")).toHaveTextContent(T.title)
    expect(screen.getByRole("dialog")).toHaveTextContent(T.description)
    expect(field()).toHaveValue("")
    expect(insert()).toBeDisabled()
  })

  it("inserts a board at 608 px, titled „Padlet”", async () => {
    const { onInsert, onOpenChange } = renderDialog()
    await userEvent.type(field(), "https://padlet.com/embed/bbcgkx6oq8valf3l")
    await userEvent.click(insert())
    expect(onInsert).toHaveBeenCalledWith({
      key: "bbcgkx6oq8valf3l",
      height: 608,
      title: "Padlet",
    })
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it.each([
    "https://padlet.com/someone/board",
    "https://padlet.com/embed/KEY?x=1",
    "http://padlet.com/embed/KEY",
    "https://youtu.be/dQw4w9WgXcQ",
  ])("refuses %s with the expected start of the address", (text) => {
    const { onInsert } = renderDialog()
    fireEvent.change(field(), { target: { value: text } })
    expect(screen.getByText(T.not_padlet)).toBeInTheDocument()
    expect(field()).toHaveAttribute("aria-invalid", "true")
    expect(insert()).toBeDisabled()
    fireEvent.submit(insert().closest("form")!)
    expect(onInsert).not.toHaveBeenCalled()
  })
})
