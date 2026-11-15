/**
 * The in-place sign-in („Lejárt a munkamenet”): the dialog that opens over
 * a signed-in page when its session has ended, so that an open form survives
 * the end of the session (ADR 0006, "Follow-up work"; ADR 0021).
 *
 * The layout mounts it next to the page, never inside one, so a form, its
 * values and ra-core's pending save stay exactly as they were behind it. It
 * opens while a request waits in the retry queue (`session-expiry.ts`): a
 * data-provider request or ra-core's session check that met a
 * `401 unauthorized`. It asks only for the password of the account whose
 * session ended (the address is shown, not typed), and signs it in again
 * through the auth provider's `resumeSession`:
 *
 * - `resumed`: the round ends with a sign-in, every waiting request is sent
 *   again, and the dialog closes over the unchanged page;
 * - `restart`: the new session belongs to another account, or is not a full
 *   session, so the waiting requests must not be sent under it; the dialog
 *   leaves as „Kilépés” does (below), which ends that session on the server;
 * - a failed sign-in keeps the dialog open with the sign-in screen's own
 *   message (`signInErrorMessage`), and the user may try again.
 *
 * „Kilépés” is a sign-out that the server confirms (`signOut`, the auth
 * provider's `logout` with `SIGN_OUT_PARAMS`): `POST /auth/logout`, whose
 * `401` (the usual answer: the session had ended) or `204` (a session was
 * still there, for example one that another tab or a `restart` left in the
 * cookie) says that no session is left. Only then do the waiting requests
 * get their `401`, ra-core shows the sign-in page, and what was not saved is
 * lost, as the dialog says. So the sign-in page never shows while the
 * cookie may still hold a session, which on a shared computer would be left
 * to the next person (ADR 0006 §2). A sign-out that the server did not
 * confirm keeps the dialog open with the user menu's message
 * (`signOutErrorMessage`), and the requests keep waiting.
 *
 * These rules (only the account whose session ended continues; a `restart`
 * sends nothing that waits and ends that session; „Kilépés” is the
 * confirmed sign-out) are the maintainer's answer of 2026-11-15, "Confirm
 * all three as built" (research: DECISIONS R37 (D158)). ADR 0006 does not
 * say who signs in here; `admin/CLAUDE.md` (Authentication) quotes the
 * question.
 *
 * Nothing else closes the dialog: not Escape and not a click beside it,
 * because either would leave the page waiting with no way to go on. Both
 * buttons are disabled while a sign-in or a sign-out is on its way: leaving
 * during a sign-in could race a session cookie that the sign-in sets after
 * the sign-out. The sign-in gives up after the provider's
 * `RESUME_TIMEOUT_MS`, so it always hands the buttons back. A sign-out that
 * stalls leaves a reload as the way on, which costs nothing more: the user
 * has already chosen to leave the unsaved changes.
 *
 * The queue and the three auth calls come in as props, so tests mount the
 * dialog with stubs, and nothing here reads `window`.
 */
import {
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
  type FormEvent,
} from "react"

import type { UiMessage } from "@/api/error-envelope"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useT } from "@/i18n/keys"

import type { ResumeOutcome, SessionAccount } from "./auth-provider"
import type { SessionExpiry } from "./session-expiry"
import { signInErrorMessage } from "./SignInPage"
import { signOutErrorMessage } from "./use-sign-out"

/**
 * The parts of {@link SessionExpiryDialogProps} that are fixed for the
 * admin's lifetime, so the layout binds them once at start-up
 * (`createAppLayout`); the sign-out comes from ra-core's context at render.
 */
export interface SessionExpiryOptions {
  /** The retry queue that the providers send through. */
  readonly expiry: SessionExpiry
  /**
   * The account whose session ended; the auth provider's `sessionAccount`.
   * Read when the dialog opens.
   */
  readonly account: () => SessionAccount | undefined
  /**
   * Signs that account in again; the auth provider's `resumeSession`. It
   * rejects with a `SignInError` when the sign-in failed.
   */
  readonly resume: (password: string) => Promise<ResumeOutcome>
}

/** Props of {@link SessionExpiryDialog}. */
export interface SessionExpiryDialogProps extends SessionExpiryOptions {
  /**
   * Ends the session on the server and leaves for the sign-in page: the
   * layout passes `useConfirmedSignOut()` (`use-sign-out.ts`). It resolves
   * once the server confirmed that no session is left, and rejects (with a
   * `SignOutError` from the auth provider) when it did not.
   */
  readonly signOut: () => Promise<void>
}

/**
 * Registers with the retry queue while it is mounted, and shows the
 * in-place sign-in while a request waits.
 *
 * @param props - see {@link SessionExpiryDialogProps}.
 */
export function SessionExpiryDialog({
  expiry,
  account,
  resume,
  signOut,
}: SessionExpiryDialogProps) {
  const open = useSyncExternalStore(expiry.subscribe, expiry.isWaiting)
  // Mounted means a 401 may wait: the layout renders this only with a
  // session. Unmounting (the sign-in page replaced the layout) hands every
  // waiting request its 401.
  useEffect(() => expiry.attach(), [expiry])
  return (
    <Dialog
      open={open}
      // Controlled: the queue alone opens and closes the dialog, so Escape,
      // which asks to close, changes nothing; and a click beside it does
      // not even ask.
      onOpenChange={ignoreCloseRequest}
      disablePointerDismissal
    >
      {/* Mounted only while open, so every opening starts with an empty
          password and no message. */}
      {open && (
        <SignInAgain
          expiry={expiry}
          email={account()?.email ?? ""}
          resume={resume}
          signOut={signOut}
        />
      )}
    </Dialog>
  )
}

/**
 * Ignores the dialog's own requests to close: Escape, and anything else
 * Base UI may add (`disablePointerDismissal` keeps a click beside it from
 * asking at all).
 */
function ignoreCloseRequest(): void {}

/** Props of {@link SignInAgain}. */
interface SignInAgainProps {
  /** The retry queue, whose round the form ends. */
  readonly expiry: SessionExpiry
  /** The address of the account whose session ended, shown read-only. */
  readonly email: string
  /** The auth provider's `resumeSession`. */
  readonly resume: (password: string) => Promise<ResumeOutcome>
  /** The confirmed sign-out; see {@link SessionExpiryDialogProps}. */
  readonly signOut: () => Promise<void>
}

/**
 * The dialog's content: the address, the password, the message of a failed
 * attempt, and the two ways on.
 *
 * @param props - see {@link SignInAgainProps}.
 */
function SignInAgain({ expiry, email, resume, signOut }: SignInAgainProps) {
  const t = useT()
  const passwordRef = useRef<HTMLInputElement>(null)
  const emailId = useId()
  const passwordId = useId()
  const requiredId = useId()
  const [password, setPassword] = useState("")
  const [missing, setMissing] = useState(false)
  const [failure, setFailure] = useState<UiMessage | null>(null)
  const [pending, setPending] = useState(false)

  /**
   * Signs out with the server's confirmation, then gives the waiting
   * requests their `401`; on a failure, shows why and keeps them waiting.
   * Called with `pending` already set.
   */
  const leave = async () => {
    try {
      await signOut()
    } catch (error) {
      setFailure(signOutErrorMessage(error))
      setPending(false)
      return
    }
    // The auth provider's sign-out has given the in-place sign-in up
    // already; this repeats it for a `signOut` that does not, so the
    // dialog never relies on what the provider does inside. The queue then
    // closes the dialog, which unmounts this form, so no state is set after
    // it.
    expiry.abandon()
  }

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) {
      return
    }
    // The browser's own validation is off (`noValidate`): it would speak
    // the browser's language. ra-core's Hungarian message instead.
    if (password === "") {
      setMissing(true)
      passwordRef.current?.focus()
      return
    }
    setMissing(false)
    setFailure(null)
    setPending(true)
    let outcome: ResumeOutcome
    try {
      outcome = await resume(password)
    } catch (error) {
      setFailure(signInErrorMessage(error))
      setPending(false)
      return
    }
    if (outcome === "resumed") {
      // Ends the round; the queue then closes the dialog, which unmounts
      // this form, so no state is set after it.
      expiry.resume()
      return
    }
    // `restart`: the cookie holds a session that the waiting requests are
    // not sent under; end it before the sign-in page shows. `pending` stays
    // set.
    await leave()
  }

  return (
    <DialogContent
      showCloseButton={false}
      initialFocus={passwordRef}
      data-testid="session-expiry-dialog"
    >
      <DialogHeader>
        <DialogTitle>{t("auth.session_expired.title")}</DialogTitle>
        <DialogDescription>{t("auth.session_expired.intro")}</DialogDescription>
      </DialogHeader>
      {failure !== null && (
        <Alert variant="destructive">
          <AlertDescription>
            {failure.kind === "key"
              ? t(failure.key, failure.values)
              : failure.text}
          </AlertDescription>
        </Alert>
      )}
      <form
        className="flex flex-col gap-4"
        noValidate
        onSubmit={(event) => {
          void submit(event)
        }}
      >
        <div className="flex flex-col gap-2">
          <Label htmlFor={emailId}>{t("auth.sign_in.email")}</Label>
          {/* Read-only: only the account whose session ended may take the
              waiting requests over. `autoComplete` lets a password manager
              match the password to it. */}
          <Input
            id={emailId}
            name="email"
            type="email"
            autoComplete="username"
            value={email}
            readOnly
          />
        </div>
        <div className="flex flex-col gap-2">
          <Label htmlFor={passwordId}>{t("auth.sign_in.password")}</Label>
          <Input
            ref={passwordRef}
            id={passwordId}
            name="password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(event) => {
              setPassword(event.target.value)
            }}
            aria-invalid={missing}
            aria-describedby={missing ? requiredId : undefined}
          />
          {missing && (
            <p id={requiredId} className="text-sm text-destructive">
              {t("ra.validation.required")}
            </p>
          )}
        </div>
        <p className="text-sm text-muted-foreground">
          {t("auth.session_expired.leave_notice")}
        </p>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            className="cursor-pointer"
            disabled={pending}
            onClick={() => {
              if (pending) {
                return
              }
              setFailure(null)
              setPending(true)
              void leave()
            }}
          >
            {t("auth.session_expired.leave")}
          </Button>
          <Button type="submit" className="cursor-pointer" disabled={pending}>
            {t("auth.sign_in.submit")}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}
