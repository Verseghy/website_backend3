/**
 * The user's own sign-out, „Kijelentkezés” in the user menu (ADR 0006 §2).
 *
 * Why a hook of its own instead of ra-core's `useLogout()` alone: ra-core
 * calls the same `authProvider.logout` by itself after it found no session,
 * with `{}`, and there the server has nothing to end. This hook passes
 * {@link SIGN_OUT_PARAMS}, the one call that makes the auth provider post
 * `/auth/logout` and revoke every session of the account. ra-core's
 * `useLogout` neither catches a rejected `logout` nor tells the user, so
 * this hook shows the failure: the user stays signed in, on the page, with a
 * Hungarian notification that says the sign-out did not happen.
 *
 * After a confirmed sign-out ra-core clears its store and query cache and
 * shows the sign-in page. The next sign-in starts at the dashboard, not at
 * the page this user left: on a shared computer it may be someone else's.
 *
 * The in-place sign-in dialog signs out the same confirmed way through
 * {@link useConfirmedSignOut}, which leaves the failure to the dialog.
 */
import { useCallback } from "react"
import { useLogout, useNotify } from "ra-core"

import { keyMessage, type UiMessage } from "@/api/error-envelope"

import {
  SIGN_OUT_FAILED_KEY,
  SIGN_OUT_PARAMS,
  SignOutError,
} from "./auth-provider"

/**
 * The message for whatever the sign-out rejected with.
 *
 * @param error - the rejection of ra-core's `logout`.
 * @returns the provider's message for a {@link SignOutError}, and the generic
 *   `auth.sign_out.failed` for anything else.
 */
export function signOutErrorMessage(error: unknown): UiMessage {
  return error instanceof SignOutError
    ? error.display
    : keyMessage(SIGN_OUT_FAILED_KEY)
}

/**
 * Returns the sign-out of the in-place sign-in dialog („Kilépés”, and the
 * end of a session that the dialog's sign-in left with another account):
 * the same confirmed sign-out as the user menu's, which posts
 * `/auth/logout` through {@link SIGN_OUT_PARAMS}, but one that rejects
 * instead of notifying, because the dialog shows the failure itself, over
 * the page that ra-core's notifications sit behind.
 *
 * Unlike the user menu's, it lets ra-core return to the current page after
 * the next sign-in: ra-core's own `logout({})`, which the waiting requests'
 * `401`s set off at the same time, does so too, and the two must not race
 * to different targets.
 *
 * @returns a function that resolves once the server confirmed that no
 *   session is left (`204`, or `401` for one that had ended already) and
 *   ra-core is on its way to the sign-in page, and rejects with the auth
 *   provider's {@link SignOutError} (or whatever else `logout` rejected
 *   with) when the server did not confirm it; the user then stays where
 *   they are.
 */
export function useConfirmedSignOut(): () => Promise<void> {
  const logout = useLogout()
  return useCallback(async () => {
    await logout(SIGN_OUT_PARAMS)
  }, [logout])
}

/**
 * Returns the user menu's sign-out action.
 *
 * @returns a function that signs the user out on the server and then shows
 *   the sign-in page, or, when the server did not confirm it, keeps the user
 *   signed in and notifies why. It never rejects.
 */
export function useSignOut(): () => Promise<void> {
  const logout = useLogout()
  const notify = useNotify()
  return useCallback(async () => {
    try {
      // `undefined`: the provider's default target, the sign-in page.
      // `false`: no return to the current page after the next sign-in.
      await logout(SIGN_OUT_PARAMS, undefined, false)
    } catch (error) {
      const message = signOutErrorMessage(error)
      if (message.kind === "key") {
        notify(message.key, { type: "error", messageArgs: message.values })
      } else {
        // The server's own text for a code this build does not know
        // (ADR 0021 §4). The notification translates its message, so the
        // text is also the fallback that polyglot returns for it.
        notify(message.text, {
          type: "error",
          messageArgs: { _: message.text },
        })
      }
    }
  }, [logout, notify])
}
