/**
 * The Padlet insert dialog (editor.md §4.3 row 10, §4.5: "Padlet (a URL)").
 *
 * Staff paste a board's embed address, `https://padlet.com/embed/<key>`,
 * which the PadletEmbed node's reader (`padlet-input.ts`) checks against the
 * server's rule; anything else is refused with a message that shows the
 * expected start. The board is inserted at the stored height of 608 px with
 * the title „Padlet” (editor.md §3.7). The toolbar's Padlet button
 * (`site-rich-text-input`) opens the dialog and inserts with
 * `insertPadletEmbed`.
 */
import { useId, useState } from "react"

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

import type { PadletEmbedAttributes } from "../extensions/padlet-embed"
import type { PadletRules } from "../padlet-input"
import {
  PADLET_DEFAULT_HEIGHT,
  PADLET_DEFAULT_TITLE,
  readPadletAddress,
} from "../padlet-input"

/** Props of {@link PadletEmbedDialog}. */
export interface PadletEmbedDialogProps {
  /** Whether the dialog is shown. */
  readonly open: boolean
  /** Called when the dialog asks to open or close (Escape, close, insert). */
  readonly onOpenChange: (open: boolean) => void
  /** The Padlet rules of the policy (`padletRules(CONTENT_V1_POLICY)`). */
  readonly rules: PadletRules
  /** Inserts the board; the dialog closes after it. */
  readonly onInsert: (attributes: PadletEmbedAttributes) => void
}

/** The form inside the dialog, mounted anew at each opening. */
function PadletEmbedForm({
  rules,
  onInsert,
}: {
  readonly rules: PadletRules
  readonly onInsert: (attributes: PadletEmbedAttributes) => void
}) {
  const t = useT()
  const inputId = useId()
  const messageId = useId()
  const [input, setInput] = useState("")
  const key = readPadletAddress(input, rules)
  const refused = input.trim() !== "" && key === null
  return (
    <form
      className="grid gap-4"
      onSubmit={(event) => {
        event.preventDefault()
        if (key !== null) {
          onInsert({
            key,
            height: PADLET_DEFAULT_HEIGHT,
            title: PADLET_DEFAULT_TITLE,
          })
        }
      }}
    >
      <div className="grid gap-2">
        <Label htmlFor={inputId}>{t("editor.padlet.input_label")}</Label>
        <Input
          id={inputId}
          type="url"
          value={input}
          autoFocus
          aria-invalid={refused}
          aria-describedby={messageId}
          onChange={(event) => setInput(event.target.value)}
        />
        <p
          id={messageId}
          aria-live="polite"
          className="text-sm text-destructive"
        >
          {refused ? t("editor.padlet.not_padlet") : null}
        </p>
      </div>
      <DialogFooter>
        <Button type="submit" disabled={key === null}>
          {t("editor.padlet.insert")}
        </Button>
      </DialogFooter>
    </form>
  )
}

/**
 * The Padlet insert dialog.
 *
 * @param props - see {@link PadletEmbedDialogProps}.
 */
export function PadletEmbedDialog({
  open,
  onOpenChange,
  rules,
  onInsert,
}: PadletEmbedDialogProps) {
  const t = useT()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("editor.padlet.title")}</DialogTitle>
          <DialogDescription>
            {t("editor.padlet.description")}
          </DialogDescription>
        </DialogHeader>
        {open ? (
          <PadletEmbedForm
            rules={rules}
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
