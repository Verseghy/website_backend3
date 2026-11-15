/**
 * The in-place sign-in dialog over a real retry queue, with the auth
 * provider's two calls as stubs: when it opens, what it shows in Hungarian,
 * and how each way out ends the waiting requests (ADR 0006, "Follow-up
 * work"; ADR 0021).
 */
import { act, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { keyMessage } from "@/api/error-envelope"
import { renderHungarian } from "@/test/render"

import {
  SignInError,
  SignOutError,
  type ResumeOutcome,
  type SessionAccount,
} from "./auth-provider"
import { SessionExpiryDialog } from "./SessionExpiryDialog"
import { createSessionExpiry, type SessionExpiry } from "./session-expiry"

const ACCOUNT: SessionAccount = {
  pid: "0b7c4a52-7e3d-4c55-9a7f-4b3c2d1e0f9a",
  name: "Próba Szerkesztő",
  email: "proba@example.test",
}

/**
 * Mounts the dialog over a fresh queue.
 *
 * @param resume - the auth provider's `resumeSession` stand-in.
 * @param signOut - the confirmed sign-out's stand-in; it resolves, as for a
 *   sign-out that the server confirmed, unless the test says otherwise.
 * @returns the queue, the sign-out stand-in and Testing Library's render
 *   result.
 */
function mountDialog(
  resume: (password: string) => Promise<ResumeOutcome> = vi.fn(
    async () => "resumed" as const
  ),
  signOut: () => Promise<void> = vi.fn(async () => undefined)
) {
  const expiry = createSessionExpiry()
  const view = renderHungarian(
    <SessionExpiryDialog
      expiry={expiry}
      account={() => ACCOUNT}
      resume={resume}
      signOut={signOut}
    />
  )
  return { expiry, signOut, view }
}

/**
 * Lets a request meet a `401` in round 0, as the providers do.
 *
 * @param expiry - the queue.
 * @returns the request's wait.
 */
function expire(expiry: SessionExpiry): Promise<string> {
  let wait: Promise<string> = Promise.resolve("not started")
  act(() => {
    wait = expiry.waitForSignIn(expiry.round())
  })
  return wait
}

describe("SessionExpiryDialog", () => {
  it("shows nothing while no request waits", () => {
    mountDialog()
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })

  it("makes a 401 wait while it is mounted, and hands it back when it unmounts", async () => {
    const { expiry, view } = mountDialog()
    const wait = expire(expiry)
    expect(await screen.findByRole("dialog")).toBeInTheDocument()

    view.unmount()
    await expect(wait).resolves.toBe("abandoned")
    // Unmounted, nothing waits any more.
    await expect(expiry.waitForSignIn(expiry.round())).resolves.toBe(
      "abandoned"
    )
  })

  it("asks in Hungarian for the password of the account whose session ended", async () => {
    const { expiry } = mountDialog()
    void expire(expiry)
    const dialog = await screen.findByRole("dialog", {
      name: "Lejárt a munkamenet",
    })
    expect(dialog).toHaveAccessibleDescription(
      "Jelentkezz be újra a jelszavaddal. Az oldal nyitva marad: amit beírtál, megmarad, és a félbemaradt művelet a bejelentkezés után folytatódik."
    )
    const email = screen.getByLabelText("E-mail-cím")
    expect(email).toHaveValue(ACCOUNT.email)
    expect(email).toHaveAttribute("readonly")
    expect(email).toHaveAttribute("autocomplete", "username")
    const password = screen.getByLabelText("Jelszó")
    expect(password).toHaveAttribute("type", "password")
    expect(password).toHaveAttribute("autocomplete", "current-password")
    await waitFor(() => expect(password).toHaveFocus())
    expect(
      screen.getByText(
        "Ha kilépsz, a bejelentkezési oldalra jutsz, és a nem mentett módosítások elvesznek."
      )
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Kilépés" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeEnabled()
    // No close button: leaving is a decision, not a dismissal.
    expect(
      screen.queryByRole("button", { name: "Bezárás" })
    ).not.toBeInTheDocument()
  })

  it("stays open on Escape, which would leave the page waiting", async () => {
    const { expiry } = mountDialog()
    void expire(expiry)
    await screen.findByRole("dialog")
    await userEvent.setup().keyboard("{Escape}")
    expect(screen.getByRole("dialog")).toBeInTheDocument()
    expect(expiry.isWaiting()).toBe(true)
  })

  it("stays open on a click beside it, which would leave the page waiting", async () => {
    const { expiry, signOut } = mountDialog()
    void expire(expiry)
    await screen.findByRole("dialog")
    const backdrop = document.querySelector('[data-slot="dialog-overlay"]')
    expect(backdrop).not.toBeNull()
    // The modal dialog makes the page behind it inert to the pointer;
    // Testing Library would refuse the click without this.
    const user = userEvent.setup({ pointerEventsCheck: 0 })
    await user.click(backdrop as Element)
    await user.click(document.body)
    expect(screen.getByRole("dialog")).toBeInTheDocument()
    expect(expiry.isWaiting()).toBe(true)
    expect(signOut).not.toHaveBeenCalled()
  })

  it("asks for the password before it signs in", async () => {
    const resume = vi.fn(async () => "resumed" as const)
    const { expiry } = mountDialog(resume)
    void expire(expiry)
    await screen.findByRole("dialog")
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Bejelentkezés" }))
    expect(screen.getByText("Kötelező")).toBeInTheDocument()
    expect(screen.getByLabelText("Jelszó")).toHaveAttribute(
      "aria-invalid",
      "true"
    )
    expect(screen.getByLabelText("Jelszó")).toHaveAccessibleDescription(
      "Kötelező"
    )
    expect(resume).not.toHaveBeenCalled()
    expect(expiry.isWaiting()).toBe(true)
  })

  it("signs in again, sends the waiting requests again and closes", async () => {
    const resume = vi.fn(async () => "resumed" as const)
    const { expiry } = mountDialog(resume)
    const wait = expire(expiry)
    await screen.findByRole("dialog")

    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))

    await expect(wait).resolves.toBe("resumed")
    expect(resume).toHaveBeenCalledExactlyOnceWith("helyes jelszó")
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    )
    expect(expiry.round()).toBe(1)
  })

  it("signs in with the Enter key too", async () => {
    const resume = vi.fn(async () => "resumed" as const)
    const { expiry } = mountDialog(resume)
    const wait = expire(expiry)
    await screen.findByRole("dialog")
    await userEvent
      .setup()
      .type(screen.getByLabelText("Jelszó"), "helyes jelszó{Enter}")
    await expect(wait).resolves.toBe("resumed")
  })

  it("ends a session that is not the same account's before the waiting requests get their 401", async () => {
    let confirm: () => void = () => undefined
    const signOut = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          confirm = resolve
        })
    )
    const { expiry } = mountDialog(
      vi.fn(async () => "restart" as const),
      signOut
    )
    const wait = expire(expiry)
    await screen.findByRole("dialog")
    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))

    await waitFor(() => expect(signOut).toHaveBeenCalledTimes(1))
    // Until the server confirms the sign-out, nothing is handed back, and
    // neither way out can be taken twice.
    expect(expiry.isWaiting()).toBe(true)
    expect(screen.getByRole("button", { name: "Kilépés" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeDisabled()
    act(() => {
      confirm()
    })
    await expect(wait).resolves.toBe("abandoned")
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    )
  })

  it("keeps the requests waiting when the sign-out after a restart is not confirmed", async () => {
    const signOut = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(
        new SignOutError(500, keyMessage("auth.sign_out.failed"))
      )
    const { expiry } = mountDialog(
      vi.fn(async () => "restart" as const),
      signOut
    )
    void expire(expiry)
    await screen.findByRole("dialog")
    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "A kijelentkezés nem sikerült, továbbra is be vagy jelentkezve. Próbáld újra!"
    )
    expect(expiry.isWaiting()).toBe(true)
    expect(screen.getByRole("button", { name: "Kilépés" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeEnabled()
  })

  it("keeps the requests waiting after a failed sign-in, with the sign-in screen's message", async () => {
    const resume = vi
      .fn<(password: string) => Promise<ResumeOutcome>>()
      .mockRejectedValueOnce(
        new SignInError(401, keyMessage("errors.invalid_credentials"))
      )
      .mockResolvedValueOnce("resumed")
    const { expiry } = mountDialog(resume)
    const wait = expire(expiry)
    await screen.findByRole("dialog")

    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "rossz jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Hibás e-mail-cím vagy jelszó."
    )
    expect(expiry.isWaiting()).toBe(true)
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeEnabled()

    await user.clear(screen.getByLabelText("Jelszó"))
    await user.type(screen.getByLabelText("Jelszó"), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))
    await expect(wait).resolves.toBe("resumed")
    expect(resume).toHaveBeenLastCalledWith("helyes jelszó")
  })

  it("shows the server's own text for a code this build does not know", async () => {
    const { expiry } = mountDialog(
      vi.fn(async () => {
        throw new SignInError(429, {
          kind: "text",
          text: "Too many attempts; try again later.",
        })
      })
    )
    void expire(expiry)
    await screen.findByRole("dialog")
    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Too many attempts; try again later."
    )
  })

  it("shows the generic failure for anything but a SignInError", async () => {
    const { expiry } = mountDialog(
      vi.fn(async () => {
        throw new Error("a programming error")
      })
    )
    void expire(expiry)
    await screen.findByRole("dialog")
    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "A bejelentkezés most nem sikerült. Próbáld újra később!"
    )
  })

  it("disables both ways out while the sign-in is on its way", async () => {
    let finish: (outcome: ResumeOutcome) => void = () => undefined
    const { expiry } = mountDialog(
      () =>
        new Promise<ResumeOutcome>((resolve) => {
          finish = resolve
        })
    )
    const wait = expire(expiry)
    await screen.findByRole("dialog")
    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "helyes jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Kilépés" })).toBeDisabled()
    act(() => {
      finish("resumed")
    })
    await expect(wait).resolves.toBe("resumed")
  })

  it("signs out on „Kilépés”, then gives the waiting requests their 401", async () => {
    const resume = vi.fn(async () => "resumed" as const)
    const { expiry, signOut } = mountDialog(resume)
    const wait = expire(expiry)
    await screen.findByRole("dialog")
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Kilépés" }))
    await expect(wait).resolves.toBe("abandoned")
    expect(signOut).toHaveBeenCalledTimes(1)
    expect(resume).not.toHaveBeenCalled()
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    )
  })

  it("disables both ways out while the sign-out is on its way, and keeps the requests waiting", async () => {
    let confirm: () => void = () => undefined
    const signOut = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          confirm = resolve
        })
    )
    const { expiry } = mountDialog(undefined, signOut)
    const wait = expire(expiry)
    await screen.findByRole("dialog")
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Kilépés" }))
    expect(screen.getByRole("button", { name: "Kilépés" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Bejelentkezés" })).toBeDisabled()
    expect(expiry.isWaiting()).toBe(true)
    act(() => {
      confirm()
    })
    await expect(wait).resolves.toBe("abandoned")
    expect(signOut).toHaveBeenCalledTimes(1)
  })

  it.each([
    [
      "the provider's message",
      new SignOutError(403, keyMessage("errors.forbidden")),
      "Nincs jogosultságod ehhez a művelethez.",
    ],
    [
      "the generic failure for anything but a SignOutError",
      new TypeError("Failed to fetch"),
      "A kijelentkezés nem sikerült, továbbra is be vagy jelentkezve. Próbáld újra!",
    ],
  ])(
    "stays open with %s when the server did not confirm the sign-out",
    async (_case, failure, text) => {
      const signOut = vi
        .fn<() => Promise<void>>()
        .mockRejectedValueOnce(failure)
        .mockResolvedValueOnce(undefined)
      const { expiry } = mountDialog(undefined, signOut)
      const wait = expire(expiry)
      await screen.findByRole("dialog")
      const user = userEvent.setup()
      await user.click(screen.getByRole("button", { name: "Kilépés" }))

      expect(await screen.findByRole("alert")).toHaveTextContent(text)
      // The cookie may still hold a session: no sign-in page yet.
      expect(expiry.isWaiting()).toBe(true)
      expect(screen.getByRole("button", { name: "Kilépés" })).toBeEnabled()

      // A second try that the server confirms leaves.
      await user.click(screen.getByRole("button", { name: "Kilépés" }))
      await expect(wait).resolves.toBe("abandoned")
      expect(signOut).toHaveBeenCalledTimes(2)
    }
  )

  it("starts over with an empty password and no message when it opens again", async () => {
    const resume = vi
      .fn<(password: string) => Promise<ResumeOutcome>>()
      .mockRejectedValueOnce(
        new SignInError(401, keyMessage("errors.invalid_credentials"))
      )
      .mockResolvedValueOnce("resumed")
    const { expiry } = mountDialog(resume)
    void expire(expiry)
    await screen.findByRole("dialog")
    const user = userEvent.setup()
    await user.type(screen.getByLabelText("Jelszó"), "rossz jelszó")
    await user.click(screen.getByRole("button", { name: "Bejelentkezés" }))
    await screen.findByRole("alert")
    await user.clear(screen.getByLabelText("Jelszó"))
    await user.type(screen.getByLabelText("Jelszó"), "helyes jelszó{Enter}")
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    )

    // The session ends again later.
    void expire(expiry)
    await screen.findByRole("dialog")
    expect(screen.getByLabelText("Jelszó")).toHaveValue("")
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("does not open again after „Kilépés” while ra-core leaves the page", async () => {
    const { expiry } = mountDialog()
    const wait = expire(expiry)
    await screen.findByRole("dialog")
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Kilépés" }))
    await expect(wait).resolves.toBe("abandoned")
    // ra-core's last session check before it signs out.
    await expect(expire(expiry)).resolves.toBe("abandoned")
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })
})
