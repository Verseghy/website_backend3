/**
 * The image's alt-text dialog (editor.md §4.3 row 8, "the alt dialog";
 * parity POS-C03, "Alt text required for images").
 *
 * The image's view in the editor opens it from its alt button and from the
 * red „Alt szöveg hiányzik” badge. Saving stores the text as typed; a blank
 * text still counts as missing (`isAltMissing`), so the badge stays and the
 * editor input keeps refusing to save the body (§4.5, `requireImageAlt`).
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
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { useT } from "@/i18n/keys"

/** Props of {@link ImageAltDialog}. */
export interface ImageAltDialogProps {
  /** Whether the dialog is shown. */
  readonly open: boolean
  /** Called when the dialog asks to open or close (Escape, close, save). */
  readonly onOpenChange: (open: boolean) => void
  /** The image's alt text now, or `null` when it has none. */
  readonly alt: string | null
  /** Stores the new alt text; the dialog closes after it. */
  readonly onSave: (alt: string) => void
}

/** The form inside the dialog, mounted anew at each opening. */
function ImageAltForm({
  alt,
  onSave,
}: {
  readonly alt: string | null
  readonly onSave: (alt: string) => void
}) {
  const t = useT()
  const fieldId = useId()
  const [text, setText] = useState(alt ?? "")
  return (
    <form
      className="grid gap-4"
      onSubmit={(event) => {
        event.preventDefault()
        onSave(text)
      }}
    >
      <div className="grid gap-2">
        <Label htmlFor={fieldId}>{t("editor.image.alt_label")}</Label>
        <Textarea
          id={fieldId}
          value={text}
          rows={3}
          autoFocus
          onChange={(event) => setText(event.target.value)}
        />
      </div>
      <DialogFooter>
        <Button type="submit">{t("ra.action.save")}</Button>
      </DialogFooter>
    </form>
  )
}

/**
 * The alt-text dialog.
 *
 * @param props - see {@link ImageAltDialogProps}.
 */
export function ImageAltDialog({
  open,
  onOpenChange,
  alt,
  onSave,
}: ImageAltDialogProps) {
  const t = useT()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("editor.image.alt_title")}</DialogTitle>
          <DialogDescription>
            {t("editor.image.alt_description")}
          </DialogDescription>
        </DialogHeader>
        {open ? (
          <ImageAltForm
            alt={alt}
            onSave={(text) => {
              onSave(text)
              onOpenChange(false)
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}
