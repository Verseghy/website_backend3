import { act, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { Error as KitErrorPage } from "@/components/admin/error"
import { renderHungarian } from "@/test/render"

import { ChunkLoadReloadPrompt, UpdateAvailableNotice } from "./UpdatePrompt"

const PROMPT = "Frissítés elérhető. Kérlek töltsd újra az oldalt!"

describe("UpdateAvailableNotice", () => {
  it("shows the public site's wording and reloads on request (ADR 0004 §5)", async () => {
    const onReload = vi.fn()
    renderHungarian(<UpdateAvailableNotice onReload={onReload} />)

    expect(screen.getByRole("alert")).toHaveTextContent(PROMPT)
    await userEvent.click(screen.getByRole("button", { name: "Újratöltés" }))
    expect(onReload).toHaveBeenCalledTimes(1)
  })
})

describe("ChunkLoadReloadPrompt", () => {
  it("renders nothing until a chunk fails to load", () => {
    const target = new EventTarget() as unknown as Window
    renderHungarian(
      <ChunkLoadReloadPrompt target={target} onReload={vi.fn()} />
    )
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("appears after Vite reports a failed preload and stays", () => {
    const target = new EventTarget() as unknown as Window
    renderHungarian(
      <ChunkLoadReloadPrompt target={target} onReload={vi.fn()} />
    )

    act(() => {
      target.dispatchEvent(new Event("vite:preloadError"))
    })

    expect(screen.getByRole("alert")).toHaveTextContent(PROMPT)
  })

  it("appears after an unhandled chunk-load rejection, not after other rejections", () => {
    const target = new EventTarget() as unknown as Window
    renderHungarian(
      <ChunkLoadReloadPrompt target={target} onReload={vi.fn()} />
    )
    const rejection = (reason: unknown) =>
      Object.assign(new Event("unhandledrejection"), { reason })

    act(() => {
      target.dispatchEvent(rejection(new Error("unrelated")))
    })
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()

    act(() => {
      target.dispatchEvent(
        rejection(
          new TypeError(
            "Failed to fetch dynamically imported module: /admin/assets/x.js"
          )
        )
      )
    })
    expect(screen.getByRole("alert")).toHaveTextContent(PROMPT)
  })
})

describe("the kit's error page", () => {
  it("shows the reload prompt for a chunk-load error caught by an error boundary", () => {
    renderHungarian(
      <KitErrorPage
        error={
          new TypeError(
            "Failed to fetch dynamically imported module: /admin/assets/x.js"
          )
        }
        resetErrorBoundary={vi.fn()}
      />,
      { route: "/" }
    )
    expect(screen.getByRole("alert")).toHaveTextContent(PROMPT)
  })

  it("shows the Hungarian generic error for any other error", () => {
    renderHungarian(
      <KitErrorPage error={new Error("boom")} resetErrorBoundary={vi.fn()} />,
      { route: "/" }
    )
    expect(screen.getByRole("alert")).toHaveTextContent("Hiba történt")
  })
})
