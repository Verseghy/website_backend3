/**
 * The image in the editor (editor.md §4.3 row 8): the SiteImage node's
 * React node view.
 *
 * The node view's own element is a `p` (`ReactNodeViewRenderer(…, { as:
 * "p" })`), so it renders phrasing content only: a `span` wrapper, the
 * image, and the chrome of spans and buttons; the alt dialog is portalled
 * out. The chrome carries `data-vfg-ui`, which the editor stylesheet keeps
 * apart from the content's styles (§11.2), and is not editable text. It
 * shows:
 *
 * - the red „Alt szöveg hiányzik” badge while the alt text is missing or
 *   blank (POS-C03); the badge and the alt button open the alt dialog;
 * - the width presets 175, 350, 525 and 700 px; the pressed one is the
 *   image's width;
 * - the state of an upload that is still running or failed (§4.3 row 13).
 *
 * The image itself is shown with the stored shape's box: block, centred,
 * at most the column's width, at its display width.
 */
import { useState } from "react"
import type { ReactNodeViewProps } from "@tiptap/react"
import { NodeViewWrapper } from "@tiptap/react"

import { Button } from "@/components/ui/button"
import { useT } from "@/i18n/keys"

import { ImageAltDialog } from "../dialogs/ImageAltDialog"
import type { SiteImageAttributes } from "../image-attributes"
import { IMAGE_WIDTH_PRESETS, isAltMissing } from "../image-attributes"

/**
 * The image's node view.
 *
 * @param props - TipTap's node view props: the node, whose attributes are
 *   {@link SiteImageAttributes}, and `updateAttributes`, which the presets
 *   and the alt dialog call.
 */
export function SiteImageView({ node, updateAttributes }: ReactNodeViewProps) {
  const t = useT()
  const [altOpen, setAltOpen] = useState(false)
  const attributes = node.attrs as SiteImageAttributes
  const missingAlt = isAltMissing(attributes.alt)
  return (
    <NodeViewWrapper as="span" className="relative block">
      <img
        src={attributes.src ?? undefined}
        alt={attributes.alt ?? ""}
        data-drag-handle=""
        style={{
          display: "block",
          height: "auto",
          margin: "0 auto",
          maxWidth: "100%",
          width:
            attributes.displayWidth === null
              ? undefined
              : `${attributes.displayWidth}px`,
        }}
      />
      <span
        data-vfg-ui=""
        contentEditable={false}
        className="flex flex-wrap items-center justify-center gap-1 pt-1"
      >
        {missingAlt ? (
          <Button
            type="button"
            size="xs"
            variant="destructive"
            onClick={() => setAltOpen(true)}
          >
            {t("editor.image.alt_missing")}
          </Button>
        ) : null}
        <Button
          type="button"
          size="xs"
          variant="outline"
          onClick={() => setAltOpen(true)}
        >
          {t("editor.image.alt_edit")}
        </Button>
        <span
          role="group"
          aria-label={t("editor.image.sizes")}
          className="inline-flex gap-1"
        >
          {IMAGE_WIDTH_PRESETS.map((width) => (
            <Button
              key={width}
              type="button"
              size="xs"
              variant={
                attributes.displayWidth === width ? "secondary" : "ghost"
              }
              aria-pressed={attributes.displayWidth === width}
              onClick={() => updateAttributes({ displayWidth: width })}
            >
              {t("editor.image.size_preset", { width })}
            </Button>
          ))}
        </span>
        {attributes.uploadState === "pending" ? (
          <span role="status">{t("editor.image.uploading")}</span>
        ) : null}
        {attributes.uploadState === "failed" ? (
          <span role="alert" className="text-destructive">
            {t("editor.image.upload_failed")}
          </span>
        ) : null}
      </span>
      <ImageAltDialog
        open={altOpen}
        onOpenChange={setAltOpen}
        alt={attributes.alt}
        onSave={(alt) => updateAttributes({ alt })}
      />
    </NodeViewWrapper>
  )
}
