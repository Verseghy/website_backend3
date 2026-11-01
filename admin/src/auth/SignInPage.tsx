/**
 * The Hungarian sign-in screen („Bejelentkezés”, `/admin/login`), which
 * replaces the kit's login page (ADR 0006 §4, ADR 0021).
 *
 * ra-core shows it whenever `checkAuth` finds no session, and after a
 * successful sign-in returns to the page that sent the user here (`useLogin`).
 * The form posts through the auth provider (`auth-provider.ts`), which sets
 * the session cookie; this screen never sees a token.
 *
 * What it shows when the sign-in fails, inline above the form so that a
 * screen reader announces it (`role="alert"`): `errors.invalid_credentials`
 * for every wrong address or password, the envelope's own code for an answer
 * that carries an error envelope (a refused body, for example), and
 * `auth.sign_in.failed` when the server could not be reached or its answer
 * carries no readable envelope. The provider decides which; this screen
 * renders {@link SignInError.display}.
 *
 * The form validates only that both fields are filled, with ra-core's
 * Hungarian message. The browser's own validation is off (`noValidate`): it
 * would show the browser's language, not Hungarian, and the server, not the
 * browser, decides which addresses can sign in.
 *
 * The footer carries the source link, so the AGPL-3.0 §13 offer reaches
 * visitors who are not signed in too (ADR 0012 §4): every other admin page
 * needs a session.
 */
import { useState, type ComponentType } from "react"
import { Form, required, useLogin } from "ra-core"
import type { FieldValues, SubmitHandler } from "react-hook-form"

import { keyMessage, type UiMessage } from "@/api/error-envelope"
import { Notification } from "@/components/admin/notification"
import { TextInput } from "@/components/admin/text-input"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { useT, type MessageKey } from "@/i18n/keys"
import { SourceLink } from "@/source-link/SourceLink"

import { SIGN_IN_FAILED_KEY, SignInError } from "./auth-provider"

/** The label keys of the two inputs; ra-core's `FieldTitle` translates them. */
const EMAIL_LABEL = "auth.sign_in.email" satisfies MessageKey
const PASSWORD_LABEL = "auth.sign_in.password" satisfies MessageKey

/** Both fields are required; the message is ra-core's `ra.validation.required`. */
const REQUIRED = required()

/**
 * The message for whatever `login` rejected with.
 *
 * @param error - the rejection.
 * @returns the provider's message for a {@link SignInError}, and the generic
 *   `auth.sign_in.failed` for anything else (a programming error, which the
 *   user can do nothing about).
 */
export function signInErrorMessage(error: unknown): UiMessage {
  return error instanceof SignInError
    ? error.display
    : keyMessage(SIGN_IN_FAILED_KEY)
}

/**
 * Builds the sign-in page for one build's source commit.
 *
 * ra-core expects a component type for `loginPage`, so the commit is bound
 * here once at start-up, as `createAppLayout` binds it for the layout.
 *
 * @param sourceCommit - the build's raw `VITE_SOURCE_COMMIT`; see
 *   `SourceLink`.
 * @returns the page component for `<Admin loginPage>`.
 */
export function createSignInPage(
  sourceCommit: string | undefined
): ComponentType {
  function SignInPage() {
    const t = useT()
    const login = useLogin()
    const [failure, setFailure] = useState<UiMessage | null>(null)
    const [pending, setPending] = useState(false)

    const submit: SubmitHandler<FieldValues> = async (values) => {
      setPending(true)
      setFailure(null)
      try {
        await login({ email: values.email, password: values.password })
      } catch (error) {
        setFailure(signInErrorMessage(error))
      } finally {
        setPending(false)
      }
    }

    return (
      <div className="flex min-h-svh flex-col">
        <main className="flex flex-1 items-center justify-center p-8">
          <div
            className="flex w-full flex-col gap-6 sm:w-[350px]"
            data-testid="sign-in-page"
          >
            <div className="flex flex-col gap-2 text-center">
              <p className="text-sm text-muted-foreground">{t("app.title")}</p>
              <h1 className="text-2xl font-semibold tracking-tight">
                {t("auth.sign_in.title")}
              </h1>
              <p className="text-sm text-muted-foreground">
                {t("auth.sign_in.intro")}
              </p>
            </div>
            {failure !== null && (
              <Alert variant="destructive">
                <AlertDescription>
                  {failure.kind === "key"
                    ? t(failure.key, failure.values)
                    : failure.text}
                </AlertDescription>
              </Alert>
            )}
            <Form className="flex flex-col gap-6" onSubmit={submit} noValidate>
              <TextInput
                label={EMAIL_LABEL}
                source="email"
                type="email"
                autoComplete="username"
                validate={REQUIRED}
              />
              <TextInput
                label={PASSWORD_LABEL}
                source="password"
                type="password"
                autoComplete="current-password"
                validate={REQUIRED}
              />
              <Button
                type="submit"
                className="cursor-pointer"
                disabled={pending}
              >
                {t("auth.sign_in.submit")}
              </Button>
            </Form>
          </div>
        </main>
        <footer className="flex justify-center py-4 text-xs text-muted-foreground">
          <SourceLink
            className="underline-offset-4 hover:underline"
            sourceCommit={sourceCommit}
          >
            {t("app.footer.source_code")}
          </SourceLink>
        </footer>
        <Notification />
      </div>
    )
  }
  return SignInPage
}
