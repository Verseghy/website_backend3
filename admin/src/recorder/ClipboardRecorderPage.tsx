/**
 * The clipboard recorder („Vágólap-rögzítő”, `/admin/clipboard-recorder`): a
 * development tool that turns each paste into a JSON capture file for the
 * paste tests (editor.md §13.1, `capture.ts`).
 *
 * The paste area is `contentEditable` because the rich-text editor is, so the
 * browser offers it the same clipboard formats. The page inserts nothing and
 * sends nothing anywhere; the capture is only downloaded. Only the dynamic
 * import in `main.tsx` reaches this module, and the default build drops it
 * (`recorder-flag.ts`). The clock, the user agent and the download come in as
 * props, so tests mount the page with stubs.
 */
import { useState, type ClipboardEvent } from "react"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useT } from "@/i18n/keys"

import { buildCapture, captureFileName, serializeCapture } from "./capture"

/** What {@link saveTextFile} needs from the browser window. */
export interface DownloadHost {
  /** The document that the temporary download link is added to. */
  readonly document: Document
  /** The window's object-URL functions. */
  readonly URL: Pick<typeof URL, "createObjectURL" | "revokeObjectURL">
  /** The window's timer, which revokes the object URL later. */
  readonly setTimeout: (handler: () => void, timeout: number) => unknown
}

/**
 * How long a download's object URL stays valid: revoking it right after the
 * click can cancel the download in some browsers (FileSaver.js waits 40 s).
 */
export const OBJECT_URL_LIFETIME_MS = 40_000

/**
 * Downloads a text as a JSON file through a temporary `<a download>` link.
 *
 * @param host - the window, or a stub in tests; see {@link DownloadHost}.
 * @param fileName - the name the browser suggests for the file.
 * @param contents - the file's text.
 */
export function saveTextFile(
  host: DownloadHost,
  fileName: string,
  contents: string
): void {
  const url = host.URL.createObjectURL(
    new Blob([contents], { type: "application/json" })
  )
  const link = host.document.createElement("a")
  link.href = url
  link.download = fileName
  // Firefox follows a link's download only while it is in the document.
  host.document.body.append(link)
  link.click()
  link.remove()
  host.setTimeout(() => host.URL.revokeObjectURL(url), OBJECT_URL_LIFETIME_MS)
}

/** Props of {@link ClipboardRecorderPage}. */
export interface ClipboardRecorderPageProps {
  /** The browser's user agent, recorded in each capture. */
  readonly userAgent: string
  /** The clock, called once per paste. */
  readonly now: () => Date
  /**
   * Hands a capture file to the browser; the app passes {@link saveTextFile}
   * bound to `window`.
   *
   * @param fileName - the suggested file name.
   * @param contents - the JSON text.
   */
  readonly saveFile: (fileName: string, contents: string) => void
}

/** What the page last reported: nothing yet, a saved file, or no clipboard. */
type Status =
  | { readonly kind: "idle" }
  | {
      readonly kind: "saved"
      readonly file: string
      readonly types: string
      readonly scrubbed: number
    }
  | { readonly kind: "no-clipboard" }

/**
 * Renders the instructions and the paste area, and after each paste says
 * which file was downloaded and what it holds.
 *
 * @param props - see {@link ClipboardRecorderPageProps}.
 */
export function ClipboardRecorderPage({
  userAgent,
  now,
  saveFile,
}: ClipboardRecorderPageProps) {
  const t = useT()
  const [status, setStatus] = useState<Status>({ kind: "idle" })

  const onPaste = (event: ClipboardEvent<HTMLDivElement>) => {
    // The recorder only reads the clipboard; nothing is inserted.
    event.preventDefault()
    // React types `clipboardData` as always set, but a paste event created by
    // a script without one carries null.
    const data: DataTransfer | null = event.clipboardData ?? null
    if (data === null) {
      setStatus({ kind: "no-clipboard" })
      return
    }
    const capture = buildCapture(data, { capturedAt: now(), userAgent })
    const file = captureFileName(capture)
    saveFile(file, serializeCapture(capture))
    setStatus({
      kind: "saved",
      file,
      types: capture.types.join(", "),
      scrubbed: capture.scrubbedPaths,
    })
  }

  return (
    <div className="py-4" data-testid="clipboard-recorder">
      <Card>
        <CardHeader>
          <CardTitle>
            <h1>{t("recorder.title")}</h1>
          </CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <p className="text-muted-foreground">{t("recorder.dev_only")}</p>
          <p>{t("recorder.intro")}</p>
          <p>{t("recorder.synthetic")}</p>
          <p>{t("recorder.scrub_notice")}</p>
          <p id="recorder-paste-hint">{t("recorder.paste_hint")}</p>
          <div
            role="textbox"
            aria-multiline="true"
            aria-label={t("recorder.paste_area")}
            aria-describedby="recorder-paste-hint"
            contentEditable
            onPaste={onPaste}
            className="min-h-32 rounded-md border-2 border-dashed p-4 outline-none focus-visible:border-ring"
          />
          {status.kind === "saved" && (
            <p role="status" className="font-mono text-sm break-all">
              {t("recorder.saved", {
                file: status.file,
                types: status.types,
                scrubbed: status.scrubbed,
              })}
            </p>
          )}
          {status.kind === "no-clipboard" && (
            <Alert variant="destructive">
              <AlertDescription>{t("recorder.no_clipboard")}</AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
