/**
 * The YouTube insert dialog (editor.md §4.3 row 9, §4.5; ADR 0043 §2, §4).
 *
 * Staff paste a link or YouTube's own embed code into one field; the
 * dialog reads it with the YoutubeEmbed node's reader (`youtube-input.ts`),
 * the same reader as the parse rules and the paste rule:
 *
 * - every link YouTube's Share panel and player menu write becomes a
 *   player, `http` and the bare host included, with a `t` as the start
 *   (D130, D131, D136); embed code is taken too (D128);
 * - a link with no valid video or list id, or a user-uploads or search
 *   list, is refused with its message, and the insert button stays off
 *   (D129, D132);
 * - when the input sets player options that are not stored (`controls=0`,
 *   `end`, `autoplay=1`, `loop`, `playlist`), the one notice of D134 shows
 *   under the field.
 *
 * The title field starts as „Videó” (D135). Embed code that carries a real
 * title fills it, until staff edit the field; on insert the title rule
 * applies, so a blank field or YouTube's generic English title is stored as
 * „Videó”. The toolbar's video button (`site-rich-text-input`) opens the
 * dialog and inserts with `insertYoutubeEmbed`.
 */
import { useId, useMemo, useState } from "react"

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
import { Textarea } from "@/components/ui/textarea"
import type { MessageKey } from "@/i18n/keys"
import { useT } from "@/i18n/keys"

import type { YoutubeEmbedAttributes } from "../extensions/youtube-embed"
import type { YoutubeReading, YoutubeRules } from "../youtube-input"
import {
  readYoutubeInput,
  YOUTUBE_DEFAULT_TITLE,
  youtubeTitle,
} from "../youtube-input"

/** Props of {@link YoutubeEmbedDialog}. */
export interface YoutubeEmbedDialogProps {
  /** Whether the dialog is shown. */
  readonly open: boolean
  /** Called when the dialog asks to open or close (Escape, close, insert). */
  readonly onOpenChange: (open: boolean) => void
  /** The YouTube rules of the policy (`youtubeRules(CONTENT_V1_POLICY)`). */
  readonly rules: YoutubeRules
  /** Inserts the embed; the dialog closes after it. */
  readonly onInsert: (attributes: YoutubeEmbedAttributes) => void
  /**
   * The parser that reads embed code in an inert document; a fresh
   * `DOMParser` when left out.
   */
  readonly parser?: DOMParser
}

/**
 * The message of a reading that cannot be inserted.
 *
 * @param reading - the reader's result for the field's text.
 * @returns the `editor.youtube.*` key, or `null` for a video.
 */
function refusalMessage(reading: YoutubeReading): MessageKey | null {
  switch (reading.kind) {
    case "video":
      return null
    case "refused":
      return reading.reason === "list_type"
        ? "editor.youtube.list_type"
        : "editor.youtube.video_id"
    default:
      return "editor.youtube.not_youtube"
  }
}

/**
 * The form inside the dialog. It mounts each time the dialog opens, so every
 * opening starts with an empty field and the title „Videó”.
 */
function YoutubeEmbedForm({
  rules,
  parser,
  onInsert,
}: {
  readonly rules: YoutubeRules
  readonly parser: DOMParser
  readonly onInsert: (attributes: YoutubeEmbedAttributes) => void
}) {
  const t = useT()
  const inputId = useId()
  const titleId = useId()
  const messageId = useId()
  const [input, setInput] = useState("")
  const [editedTitle, setEditedTitle] = useState<string | null>(null)
  const reading = useMemo(
    () => readYoutubeInput(input, rules, parser),
    [input, rules, parser]
  )
  const codeTitle =
    reading.kind === "video"
      ? youtubeTitle(reading.title)
      : YOUTUBE_DEFAULT_TITLE
  const title = editedTitle ?? codeTitle
  const refusal = input.trim() === "" ? null : refusalMessage(reading)
  const optionsDropped = reading.kind === "video" && reading.dropped.length > 0

  return (
    <form
      className="grid gap-4"
      onSubmit={(event) => {
        event.preventDefault()
        if (reading.kind === "video") {
          onInsert({ ...reading.video, title: youtubeTitle(title) })
        }
      }}
    >
      <div className="grid gap-2">
        <Label htmlFor={inputId}>{t("editor.youtube.input_label")}</Label>
        <Textarea
          id={inputId}
          value={input}
          rows={3}
          autoFocus
          aria-invalid={refusal !== null}
          aria-describedby={messageId}
          onChange={(event) => setInput(event.target.value)}
        />
        <p id={messageId} aria-live="polite" className="text-sm">
          {refusal !== null ? (
            <span className="text-destructive">{t(refusal)}</span>
          ) : optionsDropped ? (
            <span className="text-muted-foreground">
              {t("editor.youtube.options_dropped")}
            </span>
          ) : null}
        </p>
      </div>
      <div className="grid gap-2">
        <Label htmlFor={titleId}>{t("editor.youtube.title_label")}</Label>
        <Input
          id={titleId}
          value={title}
          onChange={(event) => setEditedTitle(event.target.value)}
        />
        <p className="text-sm text-muted-foreground">
          {t("editor.youtube.title_hint")}
        </p>
      </div>
      <DialogFooter>
        <Button type="submit" disabled={reading.kind !== "video"}>
          {t("editor.youtube.insert")}
        </Button>
      </DialogFooter>
    </form>
  )
}

/**
 * The YouTube insert dialog.
 *
 * @param props - see {@link YoutubeEmbedDialogProps}.
 */
export function YoutubeEmbedDialog({
  open,
  onOpenChange,
  rules,
  onInsert,
  parser,
}: YoutubeEmbedDialogProps) {
  const t = useT()
  const documentParser = useMemo(() => parser ?? new DOMParser(), [parser])
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("editor.youtube.title")}</DialogTitle>
          <DialogDescription>
            {t("editor.youtube.description")}
          </DialogDescription>
        </DialogHeader>
        {open ? (
          <YoutubeEmbedForm
            rules={rules}
            parser={documentParser}
            onInsert={(attributes) => {
              onInsert(attributes)
              onOpenChange(false)
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}
