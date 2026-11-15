/**
 * The retry queue of the in-place sign-in (`session-expiry.ts`): when a
 * request waits, when it is sent again, when it gets its `401` back, and
 * what every interleaving of answers and sign-ins does to a batch of
 * requests (ADR 0006, "Follow-up work").
 */
import fc from "fast-check"
import { describe, expect, it, vi } from "vitest"

import {
  createSessionExpiry,
  isSessionEnded,
  sendWithSignIn,
  SESSION_ENDED_CODE,
  type SessionExpiry,
} from "./session-expiry"

/**
 * An answer with only a status.
 *
 * @param status - the HTTP status.
 * @returns a `Response` with a short text body, or none for `204`.
 */
function answer(status: number): Response {
  return new Response(status === 204 ? null : `status ${String(status)}`, {
    status,
  })
}

/**
 * The admin API's error envelope as JSON text.
 *
 * @param error - the code.
 * @returns the body.
 */
function envelopeText(error: string): string {
  return JSON.stringify({
    error,
    description: `English text for ${error}.`,
    field: null,
    params: {},
  })
}

/**
 * A `401` with the envelope `code`.
 *
 * @param code - the error code; `unauthorized` by default, the answer for
 *   an ended session.
 * @returns the answer.
 */
function refused401(code: string = SESSION_ENDED_CODE): Response {
  return new Response(envelopeText(code), {
    status: 401,
    headers: { "Content-Type": "application/json" },
  })
}

/**
 * Counts how often the open state of `expiry` changes, and in which
 * direction.
 *
 * @param expiry - the queue to watch.
 * @returns the list of states, appended to at each change.
 */
function watch(expiry: SessionExpiry): boolean[] {
  const states: boolean[] = []
  expiry.subscribe(() => {
    states.push(expiry.isWaiting())
  })
  return states
}

describe("createSessionExpiry", () => {
  it("starts in round 0, with nothing waiting", () => {
    const expiry = createSessionExpiry()
    expect(expiry.round()).toBe(0)
    expect(expiry.isWaiting()).toBe(false)
  })

  it("hands a 401 back at once while no dialog is attached", async () => {
    const expiry = createSessionExpiry()
    const states = watch(expiry)
    await expect(expiry.waitForSignIn(0)).resolves.toBe("abandoned")
    expect(expiry.isWaiting()).toBe(false)
    expect(states).toEqual([])
    // Nothing was settled: the round stays.
    expect(expiry.round()).toBe(0)
  })

  it("makes a 401 wait while a dialog is attached, and sends it again after a sign-in", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const states = watch(expiry)
    const wait = expiry.waitForSignIn(0)
    expect(expiry.isWaiting()).toBe(true)
    expect(states).toEqual([true])

    expiry.resume()
    await expect(wait).resolves.toBe("resumed")
    expect(expiry.isWaiting()).toBe(false)
    expect(expiry.round()).toBe(1)
    expect(states).toEqual([true, false])
  })

  it("opens once for every 401 of the round, and one sign-in releases them all", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const states = watch(expiry)
    const waits = [0, 0, 0].map((round) => expiry.waitForSignIn(round))
    expect(states).toEqual([true])
    expiry.resume()
    await expect(Promise.all(waits)).resolves.toEqual([
      "resumed",
      "resumed",
      "resumed",
    ])
    expect(states).toEqual([true, false])
  })

  it("hands every waiting 401 back when the user gives up", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const waits = [expiry.waitForSignIn(0), expiry.waitForSignIn(0)]
    expiry.abandon()
    await expect(Promise.all(waits)).resolves.toEqual([
      "abandoned",
      "abandoned",
    ])
    expect(expiry.round()).toBe(1)
    expect(expiry.isWaiting()).toBe(false)
  })

  it.each(["resumed", "abandoned"] as const)(
    "gives a 401 of a request sent before the round ended that round's outcome (%s) at once",
    async (outcome) => {
      const expiry = createSessionExpiry()
      expiry.attach()
      const states = watch(expiry)
      if (outcome === "resumed") {
        expiry.resume()
      } else {
        expiry.abandon()
      }
      // The request left in round 0 with the old cookie; its 401 arrives now.
      await expect(expiry.waitForSignIn(0)).resolves.toBe(outcome)
      expect(states).toEqual([])
      expect(expiry.isWaiting()).toBe(false)
    }
  )

  it("ends a round on a sign-in even when nothing waits any more", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    expiry.resume()
    expect(expiry.round()).toBe(1)
    await expect(expiry.waitForSignIn(0)).resolves.toBe("resumed")
  })

  it("makes a 401 of the new round wait for a new sign-in", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    expiry.resume()
    const wait = expiry.waitForSignIn(1)
    expect(expiry.isWaiting()).toBe(true)
    expiry.resume()
    await expect(wait).resolves.toBe("resumed")
    expect(expiry.round()).toBe(2)
  })

  it.each([-1, 0.5, 1, Number.NaN, Number.POSITIVE_INFINITY])(
    "refuses the round %d, which has not begun",
    (sentIn) => {
      const expiry = createSessionExpiry()
      expect(() => expiry.waitForSignIn(sentIn)).toThrow(RangeError)
      expect(() => expiry.waitForSignIn(sentIn)).toThrow(
        /^waitForSignIn expects the round the request was sent in, a whole number from 0 to 0/
      )
    }
  )

  it("hands every waiting 401 back when the last dialog unmounts", async () => {
    const expiry = createSessionExpiry()
    const detachFirst = expiry.attach()
    const detachSecond = expiry.attach()
    const wait = expiry.waitForSignIn(0)

    detachFirst()
    // A second call of the same detach changes nothing.
    detachFirst()
    expect(expiry.isWaiting()).toBe(true)

    detachSecond()
    await expect(wait).resolves.toBe("abandoned")
    expect(expiry.isWaiting()).toBe(false)
    expect(expiry.round()).toBe(1)
    // With no dialog left, the next 401 is handed back at once.
    await expect(expiry.waitForSignIn(1)).resolves.toBe("abandoned")
  })

  it("hands every later 401 back at once after a give-up, until a dialog attaches again", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const states = watch(expiry)
    expiry.abandon()
    // A request sent after the give-up, as ra-core's last session check is.
    await expect(expiry.waitForSignIn(1)).resolves.toBe("abandoned")
    expect(states).toEqual([])

    // The next layout's dialog asks again.
    expiry.attach()
    const wait = expiry.waitForSignIn(1)
    expect(expiry.isWaiting()).toBe(true)
    expiry.resume()
    await expect(wait).resolves.toBe("resumed")
  })

  it("asks again after a sign-in that followed a give-up", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    expiry.abandon()
    expiry.resume()
    const wait = expiry.waitForSignIn(2)
    expect(expiry.isWaiting()).toBe(true)
    expiry.resume()
    await expect(wait).resolves.toBe("resumed")
  })

  it("ends no round when a dialog unmounts with nothing waiting", () => {
    const expiry = createSessionExpiry()
    expiry.attach()()
    expect(expiry.round()).toBe(0)
  })

  it("rejects at once for a request whose signal already fired", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const reason = new DOMException("gone", "AbortError")
    await expect(
      expiry.waitForSignIn(0, AbortSignal.abort(reason))
    ).rejects.toBe(reason)
    expect(expiry.isWaiting()).toBe(false)
  })

  it("lets a cancelled request leave the queue, and closes when nothing waits", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const states = watch(expiry)
    const first = new AbortController()
    const second = new AbortController()
    const firstWait = expiry.waitForSignIn(0, first.signal)
    const secondWait = expiry.waitForSignIn(0, second.signal)

    first.abort("first gone")
    await expect(firstWait).rejects.toBe("first gone")
    expect(expiry.isWaiting()).toBe(true)

    second.abort("second gone")
    await expect(secondWait).rejects.toBe("second gone")
    expect(expiry.isWaiting()).toBe(false)
    expect(states).toEqual([true, false])
    // Leaving is not a round: nobody signed in or gave up.
    expect(expiry.round()).toBe(0)
  })

  it("forgets the abort listener of a request that a sign-in released", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const controller = new AbortController()
    const wait = expiry.waitForSignIn(0, controller.signal)
    expiry.resume()
    await expect(wait).resolves.toBe("resumed")
    // A later abort must not touch the queue.
    const states = watch(expiry)
    controller.abort()
    expect(states).toEqual([])
  })

  it.each(["resume", "abandon"] as const)(
    "removes the very abort listener it added when a %s settles the wait",
    async (settle) => {
      const expiry = createSessionExpiry()
      expiry.attach()
      const { signal } = new AbortController()
      // A long-lived signal (a screen that stays mounted) would otherwise
      // keep one listener per settled wait.
      const add = vi.spyOn(signal, "addEventListener")
      const remove = vi.spyOn(signal, "removeEventListener")
      const wait = expiry.waitForSignIn(0, signal)
      expect(add).toHaveBeenCalledTimes(1)
      expect(remove).not.toHaveBeenCalled()

      expiry[settle]()
      await wait
      expect(remove).toHaveBeenCalledExactlyOnceWith(
        "abort",
        add.mock.calls[0][1]
      )
    }
  )

  it("stops calling a listener after it unsubscribes", () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const listener = vi.fn()
    const unsubscribe = expiry.subscribe(listener)
    void expiry.waitForSignIn(0)
    expect(listener).toHaveBeenCalledTimes(1)
    unsubscribe()
    expiry.resume()
    expect(listener).toHaveBeenCalledTimes(1)
  })
})

describe("isSessionEnded", () => {
  it("is true for a 401 whose envelope says unauthorized", async () => {
    await expect(isSessionEnded(refused401())).resolves.toBe(true)
  })

  it.each([
    ["a 401 invalid_credentials", refused401("invalid_credentials")],
    ["a 401 with a code this build does not know", refused401("mfa_required")],
    ["a 401 whose body is not JSON", answer(401)],
    ["a 401 with no body", new Response(null, { status: 401 })],
    [
      "a 401 whose JSON is not an envelope",
      new Response(JSON.stringify({ error: "unauthorized" }), { status: 401 }),
    ],
    [
      "a 403 unauthorized",
      new Response(envelopeText(SESSION_ENDED_CODE), { status: 403 }),
    ],
    ["a 200", answer(200)],
  ])("is false for %s", async (_case, response) => {
    await expect(isSessionEnded(response)).resolves.toBe(false)
  })

  it("leaves the body for the caller to read", async () => {
    const response = refused401()
    await isSessionEnded(response)
    expect(response.bodyUsed).toBe(false)
    await expect(response.text()).resolves.toBe(
      envelopeText(SESSION_ENDED_CODE)
    )
  })

  it("is true exactly for status 401 with the code unauthorized", async () => {
    await fc.assert(
      fc.asyncProperty(
        // Every status that may carry a body (204, 205 and 304 may not).
        fc
          .integer({ min: 200, max: 599 })
          .filter((status) => ![204, 205, 304].includes(status)),
        fc.oneof(
          fc.constant(SESSION_ENDED_CODE),
          fc.constantFrom("invalid_credentials", "forbidden", "not_found"),
          fc.string({ minLength: 1 })
        ),
        async (status, code) => {
          const response = new Response(envelopeText(code), { status })
          await expect(isSessionEnded(response)).resolves.toBe(
            status === 401 && code === SESSION_ENDED_CODE
          )
        }
      )
    )
  })
})

describe("sendWithSignIn", () => {
  it("returns any answer but a 401 as it is, sent once", async () => {
    for (const status of [200, 204, 403, 404, 422, 500]) {
      const send = vi.fn(async () => answer(status))
      const response = await sendWithSignIn(send, createSessionExpiry())
      expect(response.status).toBe(status)
      expect(send).toHaveBeenCalledTimes(1)
    }
  })

  it("returns a 401 as it is while no dialog is attached", async () => {
    const send = vi.fn(async () => refused401())
    const response = await sendWithSignIn(send, createSessionExpiry())
    expect(response.status).toBe(401)
    await expect(response.text()).resolves.toBe(
      envelopeText(SESSION_ENDED_CODE)
    )
    expect(send).toHaveBeenCalledTimes(1)
  })

  it.each([
    ["invalid_credentials", () => refused401("invalid_credentials")],
    ["a body that is not an envelope", () => answer(401)],
  ])(
    "returns a 401 with %s at once, even with a dialog attached: it is the request's own answer",
    async (_case, make) => {
      const expiry = createSessionExpiry()
      expiry.attach()
      const states = watch(expiry)
      const send = vi.fn(async () => make())
      const response = await sendWithSignIn(send, expiry)
      expect(response.status).toBe(401)
      // The caller still reads the body, as a handler does for its message.
      expect(response.bodyUsed).toBe(false)
      expect(send).toHaveBeenCalledTimes(1)
      expect(states).toEqual([])
      expect(expiry.round()).toBe(0)
    }
  )

  it("sends again after the sign-in and discards the 401's body", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const unauthorized = refused401()
    const answers = [unauthorized, answer(200)]
    const send = vi.fn(async () => answers.shift() as Response)
    const pending = sendWithSignIn(send, expiry)
    await vi.waitFor(() => expect(expiry.isWaiting()).toBe(true))
    expiry.resume()
    expect((await pending).status).toBe(200)
    expect(send).toHaveBeenCalledTimes(2)
    expect(unauthorized.bodyUsed).toBe(true)
  })

  it("waits again when the server still answers 401 to the new cookie", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const answers = [refused401(), refused401(), answer(200)]
    const send = vi.fn(async () => answers.shift() as Response)
    const pending = sendWithSignIn(send, expiry)
    await vi.waitFor(() => expect(expiry.isWaiting()).toBe(true))
    expiry.resume()
    await vi.waitFor(() => expect(send).toHaveBeenCalledTimes(2))
    await vi.waitFor(() => expect(expiry.isWaiting()).toBe(true))
    expiry.resume()
    expect((await pending).status).toBe(200)
    expect(send).toHaveBeenCalledTimes(3)
  })

  it("returns the 401 when the user gives up", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const send = vi.fn(async () => refused401())
    const pending = sendWithSignIn(send, expiry)
    await vi.waitFor(() => expect(expiry.isWaiting()).toBe(true))
    expiry.abandon()
    const response = await pending
    expect(response.status).toBe(401)
    await expect(response.text()).resolves.toBe(
      envelopeText(SESSION_ENDED_CODE)
    )
    expect(send).toHaveBeenCalledTimes(1)
  })

  it("sends at once again when a sign-in ended the round while the request was on its way", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    let release: (response: Response) => void = () => undefined
    const answers: Promise<Response>[] = [
      new Promise((resolve) => {
        release = resolve
      }),
      Promise.resolve(answer(200)),
    ]
    const send = vi.fn(() => answers.shift() as Promise<Response>)
    const states = watch(expiry)
    const pending = sendWithSignIn(send, expiry)
    // Another request's dialog signs in while this one is still on its way.
    expiry.resume()
    release(refused401())
    expect((await pending).status).toBe(200)
    expect(send).toHaveBeenCalledTimes(2)
    expect(states).toEqual([])
  })

  it("rejects with what send rejects with", async () => {
    const failure = new TypeError("Failed to fetch")
    await expect(
      sendWithSignIn(() => Promise.reject(failure), createSessionExpiry())
    ).rejects.toBe(failure)
  })

  it("rejects with the signal's reason when it fires during the wait", async () => {
    const expiry = createSessionExpiry()
    expiry.attach()
    const controller = new AbortController()
    const send = vi.fn(async () => refused401())
    const pending = sendWithSignIn(send, expiry, controller.signal)
    await vi.waitFor(() => expect(expiry.isWaiting()).toBe(true))
    controller.abort("unmounted")
    await expect(pending).rejects.toBe("unmounted")
    expect(send).toHaveBeenCalledTimes(1)
  })
})

describe("sendWithSignIn under every interleaving", () => {
  /**
   * A server whose session holds once `signedIn` is set: a request carries
   * the cookie of the moment it is sent, so one sent before the sign-in
   * gets `401` even if its answer arrives after it. Every answer, and the
   * user's sign-in in the dialog, resolve in the scheduler's order.
   *
   * @param scheduler - fast-check's scheduler.
   * @param expiry - the queue.
   * @param userSignsIn - whether the user signs in when the dialog opens,
   *   or gives up.
   * @returns how to send request `index`, and the counters to check.
   */
  function scenario(
    scheduler: fc.Scheduler,
    expiry: SessionExpiry,
    userSignsIn: boolean
  ) {
    const state = { signedIn: false, signIns: 0, giveUps: 0 }
    const sends: number[] = []
    expiry.subscribe(() => {
      if (!expiry.isWaiting()) {
        return
      }
      void scheduler.schedule(Promise.resolve("user")).then(() => {
        if (userSignsIn) {
          state.signIns += 1
          state.signedIn = true
          expiry.resume()
        } else {
          state.giveUps += 1
          expiry.abandon()
        }
      })
    })
    const send = (index: number) => () => {
      sends[index] = (sends[index] ?? 0) + 1
      return scheduler.schedule(
        Promise.resolve(state.signedIn ? answer(200) : refused401())
      )
    }
    return { state, sends, send }
  }

  it("releases every request with one sign-in, sending each at most twice", async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.scheduler(),
        fc.integer({ min: 1, max: 6 }),
        async (scheduler, count) => {
          const expiry = createSessionExpiry()
          expiry.attach()
          const { state, sends, send } = scenario(scheduler, expiry, true)
          const all = Promise.all(
            Array.from({ length: count }, (_, index) =>
              sendWithSignIn(send(index), expiry)
            )
          )
          const responses = await scheduler.waitFor(all)
          expect(responses.map((response) => response.status)).toEqual(
            Array.from({ length: count }, () => 200)
          )
          expect(state.signIns).toBe(1)
          expect(sends).toHaveLength(count)
          for (const times of sends) {
            expect(times).toBeGreaterThanOrEqual(1)
            expect(times).toBeLessThanOrEqual(2)
          }
          expect(expiry.isWaiting()).toBe(false)
        }
      )
    )
  })

  it("hands every request its 401 after one give-up, each sent once", async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.scheduler(),
        fc.integer({ min: 1, max: 6 }),
        async (scheduler, count) => {
          const expiry = createSessionExpiry()
          expiry.attach()
          const { state, sends, send } = scenario(scheduler, expiry, false)
          const all = Promise.all(
            Array.from({ length: count }, (_, index) =>
              sendWithSignIn(send(index), expiry)
            )
          )
          const responses = await scheduler.waitFor(all)
          expect(responses.map((response) => response.status)).toEqual(
            Array.from({ length: count }, () => 401)
          )
          expect(state.giveUps).toBe(1)
          expect(sends).toEqual(Array.from({ length: count }, () => 1))
          expect(expiry.isWaiting()).toBe(false)
        }
      )
    )
  })
})
