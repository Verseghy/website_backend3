/**
 * The retry queue of the in-place sign-in: what happens to a request that
 * the admin API answers with `401` while a signed-in page is open (ADR 0006,
 * "Follow-up work": the admin must not drop an open form on a `401`; it
 * shows a sign-in dialog in place, then retries).
 *
 * Why it exists: a session ends two hours after the last activity, at the
 * absolute limit, or when the account's sessions are revoked (ADR 0006 §1,
 * §2). Left to ra-core, the next `401` signs the user out: `checkError`
 * rejects, ra-core swaps the whole layout for the sign-in page, and an
 * editor's unsaved post is gone. Instead, every request that the providers
 * send through {@link sendWithSignIn} waits here on a `401 unauthorized`;
 * the layout's dialog (`SessionExpiryDialog.tsx`) asks for the password,
 * and after the sign-in each waiting request is sent again, unchanged,
 * while the form that started it stays mounted with every value the user
 * typed.
 *
 * The rules this module carries:
 *
 * - **One dialog for every waiting request.** The first `401` opens it, and
 *   later ones join the same wait; one sign-in releases them all.
 * - **Rounds.** Every settled wait (a sign-in, or the user giving up) ends
 *   a round. A request remembers the round it left in. A `401` for a request
 *   that left before the last round ended carries the old cookie, so it does
 *   not open a second dialog: it is sent again at once after a sign-in, and
 *   handed back after a give-up. Only a `401` to a request sent with the
 *   current cookie waits, so a request is never sent twice per sign-in, and
 *   it waits again only after the user signed in again.
 * - **Giving up hands the `401` back.** When the user leaves the dialog (a
 *   sign-out that the server confirmed, see `SessionExpiryDialog.tsx`), or
 *   the dialog unmounts (the sign-in page replaced the layout), every waiting
 *   request resolves with its own `401`: a data-provider handler rejects it
 *   with `refusalError`, `checkError` signs out on it, and ra-core shows the
 *   sign-in page, as it did before this queue existed.
 * - **Given up stays given up.** After a give-up, every later `401` is
 *   handed back at once too, until a dialog is attached again (the layout
 *   mounts after the next sign-in on the sign-in page) or a sign-in ends a
 *   round. ra-core needs that: before it signs out on a refused request, it
 *   asks `checkAuth` once more whether a session is left, and that check's
 *   `401` must not open the dialog the user has just left. The auth
 *   provider gives up the same way on every sign-out, so nothing asks for
 *   an in-place sign-in while ra-core leaves the signed-in pages.
 * - **No dialog, no wait.** A `401` while no dialog is mounted (the first
 *   session check of a page load, the sign-in page) is handed back at once,
 *   so a visitor without a session still lands on the sign-in page.
 * - **Only an ended session waits.** A `401` waits only when its error
 *   envelope's code is `unauthorized`, the admin API's answer for a request
 *   without a valid session (ADR 0021 §4). Any other `401`, such as an
 *   `invalid_credentials` for a wrong current password, or a body that is
 *   not an envelope, is the request's own answer: it is handed back at once
 *   and never opens the dialog over a session that still holds.
 * - **A cancelled request stops waiting.** A request whose `AbortSignal`
 *   fires leaves the queue with the signal's reason, and the dialog closes
 *   when nothing waits any more. ra-core passes react-query's signal to
 *   `checkAuth` and, because the data provider declares
 *   `supportAbortSignal`, to its queries; react-query fires it when the
 *   screen that asked unmounts.
 *
 * Nothing here reads `window` or the clock, and it sends nothing itself:
 * the request to send is a function argument, so the tests drive every
 * interleaving of answers and sign-ins with stubs.
 */
import { parseErrorEnvelope } from "@/api/error-envelope"
import type { ErrorCode } from "@/bindings/ErrorCode"

/**
 * The error code of a `401` whose session has ended: the admin API's answer
 * when a request carries no valid session (ADR 0021 §4, ADR 0006).
 */
export const SESSION_ENDED_CODE = "unauthorized" satisfies ErrorCode

/** How a wait for the in-place sign-in ended. */
export type SignInWait =
  /** The user signed in again: send the request again. */
  | "resumed"
  /** The user gave up, or no dialog can ask: hand the `401` back. */
  | "abandoned"

/**
 * The shared queue between the requests that met a `401` and the dialog
 * that asks for the password. {@link createSessionExpiry} builds the one
 * instance of the admin; `main.tsx` passes it to both providers and to the
 * app, which mounts the dialog.
 */
export interface SessionExpiry {
  /**
   * The current round: how many waits have been settled so far. A request
   * reads it right before it is sent and passes it to
   * {@link SessionExpiry.waitForSignIn} on a `401`.
   *
   * @returns a whole number, 0 before the first sign-in or give-up.
   */
  round(): number
  /**
   * Waits for the in-place sign-in after a `401`.
   *
   * @param sentIn - the {@link SessionExpiry.round} that the request was
   *   sent in.
   * @param signal - the request's abort signal, if it has one; the wait
   *   ends when it fires.
   * @returns `resumed` when the request should be sent again, `abandoned`
   *   when its `401` should be handed back. A request that left before the
   *   last round ended gets that round's outcome at once, and every request
   *   gets `abandoned` at once while no dialog is mounted or after a
   *   give-up (see {@link SessionExpiry.abandon}).
   * @throws {RangeError} when `sentIn` is not a round that has begun: a
   *   programming error in the caller.
   * Rejects with the signal's reason when the signal fires first.
   */
  waitForSignIn(sentIn: number, signal?: AbortSignal): Promise<SignInWait>
  /**
   * Whether a request waits, that is whether the dialog is open; the
   * snapshot of `useSyncExternalStore`.
   *
   * @returns `true` while at least one request waits.
   */
  isWaiting(): boolean
  /**
   * Calls `listener` whenever {@link SessionExpiry.isWaiting} changes.
   *
   * @param listener - called with no arguments.
   * @returns the function that stops the calls.
   */
  subscribe(listener: () => void): () => void
  /**
   * Registers a mounted dialog, which makes `401`s wait. The layout's dialog
   * calls it while it is mounted; a newly mounted dialog also ends an
   * earlier give-up.
   *
   * @returns the function that unregisters it; once the last dialog is gone,
   *   every waiting request is abandoned. Calling it twice does nothing more.
   */
  attach(): () => void
  /**
   * The user signed in again in the dialog: ends the round and sends every
   * waiting request again.
   */
  resume(): void
  /**
   * The user gave up, or signed out: ends the round, hands every waiting
   * request its `401`, and hands back every later `401` at once until a
   * dialog is attached again or a sign-in ends a round.
   */
  abandon(): void
}

/** One request that waits for the sign-in. */
interface Waiter {
  /**
   * Ends the wait with an outcome.
   *
   * @param outcome - how the round ended.
   */
  readonly settle: (outcome: SignInWait) => void
}

/**
 * Builds the retry queue of the in-place sign-in.
 *
 * @returns a queue in round 0, with no dialog attached and nothing waiting.
 */
export function createSessionExpiry(): SessionExpiry {
  let round = 0
  // The outcome of the last round, for a 401 that comes back after it;
  // read only once `round` is above 0.
  let lastOutcome: SignInWait = "abandoned"
  let dialogs = 0
  // Set by a give-up, cleared by a newly attached dialog or a sign-in.
  let givenUp = false
  let waiting = false
  const waiters = new Set<Waiter>()
  const listeners = new Set<() => void>()

  /** Tells the listeners when the open state changed. */
  const publish = () => {
    const next = waiters.size > 0
    if (next === waiting) {
      return
    }
    waiting = next
    // A copy: a listener may unsubscribe while the calls run.
    for (const listener of [...listeners]) {
      listener()
    }
  }

  /**
   * Ends the round with `outcome` and releases every waiting request.
   *
   * @param outcome - how the round ended.
   */
  const endRound = (outcome: SignInWait) => {
    round += 1
    lastOutcome = outcome
    givenUp = outcome === "abandoned"
    const released = [...waiters]
    waiters.clear()
    for (const waiter of released) {
      waiter.settle(outcome)
    }
    publish()
  }

  return {
    round: () => round,

    waitForSignIn(sentIn, signal) {
      if (!Number.isInteger(sentIn) || sentIn < 0 || sentIn > round) {
        throw new RangeError(
          `waitForSignIn expects the round the request was sent in, a whole number from 0 to ${String(round)}; got ${String(sentIn)}. Read round() right before the request is sent.`
        )
      }
      if (sentIn < round) {
        return Promise.resolve(lastOutcome)
      }
      if (dialogs === 0 || givenUp) {
        return Promise.resolve<SignInWait>("abandoned")
      }
      if (signal?.aborted === true) {
        return Promise.reject(signal.reason)
      }
      return new Promise<SignInWait>((resolve, reject) => {
        const onAbort = () => {
          waiters.delete(waiter)
          publish()
          reject(signal?.reason)
        }
        const waiter: Waiter = {
          settle(outcome) {
            signal?.removeEventListener("abort", onAbort)
            resolve(outcome)
          },
        }
        signal?.addEventListener("abort", onAbort, { once: true })
        waiters.add(waiter)
        publish()
      })
    },

    isWaiting: () => waiting,

    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },

    attach() {
      dialogs += 1
      // A dialog mounts with a new layout, after the next sign-in.
      givenUp = false
      let attached = true
      return () => {
        if (!attached) {
          return
        }
        attached = false
        dialogs -= 1
        // Nothing can ask for the password any more: whatever waits gets
        // its 401, so ra-core signs out as it would without the dialog.
        if (dialogs === 0 && waiters.size > 0) {
          endRound("abandoned")
        }
      }
    },

    resume: () => endRound("resumed"),

    abandon: () => endRound("abandoned"),
  }
}

/**
 * Whether an answer says that the request's session has ended: a `401`
 * whose error envelope's code is {@link SESSION_ENDED_CODE}.
 *
 * It reads a copy of the body (`clone`), so the caller can still read the
 * answer itself when it is handed back.
 *
 * @param response - the answer, its body not read yet.
 * @returns `true` for a `401 unauthorized`; `false` for any other status,
 *   any other `401` code (such as `invalid_credentials`) and a `401` whose
 *   body is not an error envelope.
 */
export async function isSessionEnded(response: Response): Promise<boolean> {
  if (response.status !== 401) {
    return false
  }
  let body: unknown
  try {
    body = await response.clone().json()
  } catch {
    // Not JSON (a proxy's page, an empty body): not the admin API's
    // answer for an ended session, so the caller gets it as it is.
    return false
  }
  return parseErrorEnvelope(body)?.error === SESSION_ENDED_CODE
}

/**
 * Sends a request and, when its session has ended (`401 unauthorized`),
 * waits for the in-place sign-in and sends it again, as many times as the
 * user signs in again and the server still answers so to the new cookie.
 *
 * @param send - sends the request once; called again for every retry, so it
 *   must build a request whose body can be sent again (a string, a `Blob`,
 *   a `FormData`; never a stream).
 * @param expiry - the retry queue.
 * @param signal - the request's abort signal, if it has one; it also ends
 *   the wait for the sign-in.
 * @returns the first answer that does not say the session ended (any status
 *   but `401`, and a `401` with another code, see {@link isSessionEnded}),
 *   or the `401 unauthorized` itself when the user gave up or no dialog is
 *   mounted. The body of a `401` that is followed by a retry is discarded.
 * Rejects with whatever `send` rejects with (a network failure, an abort),
 * and with the signal's reason when it fires during the wait.
 */
export async function sendWithSignIn(
  send: () => Promise<Response>,
  expiry: SessionExpiry,
  signal?: AbortSignal
): Promise<Response> {
  for (;;) {
    // Read before the request leaves: a sign-in that ends the round while
    // the request is on its way makes its 401 an old cookie's answer.
    const sentIn = expiry.round()
    const response = await send()
    if (!(await isSessionEnded(response))) {
      return response
    }
    if ((await expiry.waitForSignIn(sentIn, signal)) === "abandoned") {
      return response
    }
    // Nobody reads this 401; release its connection before the retry.
    await response.body?.cancel().catch(() => undefined)
  }
}
