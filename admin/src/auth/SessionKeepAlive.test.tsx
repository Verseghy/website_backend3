/**
 * The session's activity tracker as a component (ADR 0006 §1): it listens
 * while mounted, renders nothing, and stops with the signed-in pages.
 */
import { act, render } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { REFRESH_INTERVAL_MS } from "./activity-refresh"
import { SessionKeepAlive } from "./SessionKeepAlive"

describe("SessionKeepAlive", () => {
  it("refreshes on activity while mounted and renders nothing", () => {
    const target = new EventTarget()
    let time = 0
    const now = () => time
    const refresh = vi.fn<() => Promise<unknown>>(async () => "refreshed")
    const { container, rerender, unmount } = render(
      <SessionKeepAlive target={target} now={now} refresh={refresh} />
    )
    expect(container).toBeEmptyDOMElement()

    act(() => {
      target.dispatchEvent(new Event("pointerdown"))
    })
    expect(refresh).toHaveBeenCalledTimes(1)

    // The same props on a re-render keep the tracker and its interval.
    rerender(<SessionKeepAlive target={target} now={now} refresh={refresh} />)
    act(() => {
      target.dispatchEvent(new Event("keydown"))
    })
    expect(refresh).toHaveBeenCalledTimes(1)

    // Unmounted (signed out): no more calls, however long it waits.
    unmount()
    time += REFRESH_INTERVAL_MS * 3
    target.dispatchEvent(new Event("keydown"))
    target.dispatchEvent(new Event("pointerdown"))
    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it("moves to a new target and stops listening on the old one", () => {
    const first = new EventTarget()
    const second = new EventTarget()
    const now = () => 0
    const refresh = vi.fn<() => Promise<unknown>>(async () => undefined)
    const { rerender, unmount } = render(
      <SessionKeepAlive target={first} now={now} refresh={refresh} />
    )
    rerender(<SessionKeepAlive target={second} now={now} refresh={refresh} />)
    act(() => {
      first.dispatchEvent(new Event("keydown"))
    })
    expect(refresh).not.toHaveBeenCalled()
    act(() => {
      second.dispatchEvent(new Event("keydown"))
    })
    expect(refresh).toHaveBeenCalledTimes(1)
    unmount()
  })
})
