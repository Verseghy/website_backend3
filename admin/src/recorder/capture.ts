/**
 * The clipboard capture: what the recorder keeps of one paste (editor.md
 * §13.1).
 *
 * Exactly what the paste pipeline reads: the clipboard's `types`, the
 * `text/html` and `text/plain` data, and each file's type and size. No RTF and
 * no file bytes or names, because the editor reads neither (editor.md §5.3)
 * and a picture can show personal data; `getData` is never called for another
 * format. Both texts are scrubbed (`scrub.ts`) before the capture exists, so
 * nothing unscrubbed is ever written. `format` and `version` let the paste
 * slices that load captures as fixtures refuse a file they do not know.
 * Pure: the clipboard, the clock and the user agent are arguments.
 */
import { scrubUserPaths } from "./scrub"

/** The `format` of every capture file. */
export const CAPTURE_FORMAT = "backend3-clipboard-capture"

/** The version of the capture shape; any shape change bumps it. */
export const CAPTURE_VERSION = 1

/** A clipboard file as a capture keeps it. */
export interface CapturedFile {
  /** The MIME type the browser reports, `""` when it knows none. */
  readonly type: string
  /** The size in bytes. */
  readonly size: number
}

/** The part of a paste event's `DataTransfer` that a capture reads. */
export interface ClipboardSource {
  /** The formats on the clipboard, in the browser's order. */
  readonly types: ArrayLike<string>
  /**
   * Reads one format; only `text/html` and `text/plain` are ever requested.
   *
   * @param format - the MIME type to read.
   */
  getData(format: string): string
  /** The files on the clipboard; only their type and size are read. */
  readonly files: ArrayLike<CapturedFile>
}

/** What surrounds a paste. */
export interface CaptureContext {
  /** The moment of the paste; must be a valid `Date`. */
  readonly capturedAt: Date
  /** The user agent: which engine of the editor.md §13.2 matrix pasted. */
  readonly userAgent: string
}

/** One capture, as its JSON file holds it. */
export interface ClipboardCapture {
  /** Always {@link CAPTURE_FORMAT}. */
  readonly format: typeof CAPTURE_FORMAT
  /** Always {@link CAPTURE_VERSION}. */
  readonly version: typeof CAPTURE_VERSION
  /** The moment of the paste, an ISO 8601 instant in UTC. */
  readonly capturedAt: string
  /** The browser's user agent. */
  readonly userAgent: string
  /** Every format on the clipboard, `text/rtf` and `Files` included. */
  readonly types: readonly string[]
  /** The scrubbed `text/html`, or `null` when the clipboard had none. */
  readonly html: string | null
  /** The scrubbed `text/plain`, or `null` when the clipboard had none. */
  readonly text: string | null
  /** Each file's type and size, in clipboard order. */
  readonly files: readonly CapturedFile[]
  /** How many user folder names were replaced in `html` and `text`. */
  readonly scrubbedPaths: number
}

/**
 * Builds the capture of one paste.
 *
 * @param source - the paste event's clipboard data.
 * @param context - the moment and the user agent; see {@link CaptureContext}.
 * @returns the scrubbed capture.
 * @throws {RangeError} when `context.capturedAt` is not a valid date.
 */
export function buildCapture(
  source: ClipboardSource,
  context: CaptureContext
): ClipboardCapture {
  if (Number.isNaN(context.capturedAt.getTime())) {
    throw new RangeError(
      "buildCapture: capturedAt must be a valid Date (the moment of the paste)."
    )
  }
  const types = Array.from(source.types)
  let scrubbedPaths = 0
  // `getData` answers "" for an absent format too; `types` tells an absent
  // format (null) from an empty one ("").
  const read = (format: string): string | null => {
    if (!types.includes(format)) {
      return null
    }
    const scrubbed = scrubUserPaths(source.getData(format))
    scrubbedPaths += scrubbed.replacements
    return scrubbed.value
  }
  const html = read("text/html")
  const text = read("text/plain")
  return {
    format: CAPTURE_FORMAT,
    version: CAPTURE_VERSION,
    capturedAt: context.capturedAt.toISOString(),
    userAgent: context.userAgent,
    types,
    html,
    text,
    files: Array.from(source.files, ({ type, size }) => ({ type, size })),
    scrubbedPaths,
  }
}

/**
 * Names a capture's file after the moment of the paste, so one session's
 * files sort in order. A file name is an identifier, so it stays English
 * (ADR 0021 §6).
 *
 * @param capture - the capture to name.
 * @returns for example `clipboard-capture-2026-10-25T08-30-00-000Z.json`,
 *   without the `:` that Windows forbids.
 */
export function captureFileName(capture: ClipboardCapture): string {
  return `clipboard-capture-${capture.capturedAt.replace(/[:.]/g, "-")}.json`
}

/**
 * Serialises a capture as indented JSON with a final newline, so a committed
 * fixture diffs line by line.
 *
 * @param capture - the capture to write.
 * @returns the file contents.
 */
export function serializeCapture(capture: ClipboardCapture): string {
  return `${JSON.stringify(capture, null, 2)}\n`
}
