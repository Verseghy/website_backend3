/**
 * The reload prompt shown after a deploy (ADR 0004 §5).
 *
 * Two entry points share one presentation:
 * - {@link ChunkLoadReloadPrompt} sits at the root of the app and appears when
 *   the window reports a failed chunk load (`listenForChunkLoadErrors`).
 * - {@link UpdateAvailableNotice} is rendered by the kit's error page when the
 *   error a boundary caught is a chunk-load error, so a lazy screen that fails
 *   to load shows the same prompt instead of a generic error.
 *
 * The wording is the public site's, from `hu.ts` (`app.update.available`).
 */
import { useEffect, useState } from "react"
import { RefreshCw } from "lucide-react"

import { Alert, AlertAction, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { useT } from "@/i18n/keys"

import {
  listenForChunkLoadErrors,
  type ChunkLoadEventTarget,
} from "./chunk-load-error"

/** Reloads the page; `index.html` is `no-cache`, so this fetches the new build. */
export function reloadPage(): void {
  window.location.reload()
}

/** Props of {@link UpdateAvailableNotice}. */
export interface UpdateAvailableNoticeProps {
  /** Called when the editor presses the reload button. */
  readonly onReload: () => void
}

/**
 * The prompt itself: the Hungarian message and a reload button.
 *
 * @param props - see {@link UpdateAvailableNoticeProps}.
 */
export function UpdateAvailableNotice({
  onReload,
}: UpdateAvailableNoticeProps) {
  const t = useT()
  return (
    <Alert role="alert" data-testid="update-available">
      <AlertDescription>{t("app.update.available")}</AlertDescription>
      <AlertAction>
        <Button size="sm" onClick={onReload}>
          <RefreshCw />
          {t("app.update.reload")}
        </Button>
      </AlertAction>
    </Alert>
  )
}

/** Props of {@link ChunkLoadReloadPrompt}. */
export interface ChunkLoadReloadPromptProps {
  /** The window to watch; the app passes `window`, tests a stub. */
  readonly target: ChunkLoadEventTarget
  /** Called when the editor presses the reload button. */
  readonly onReload: () => void
}

/**
 * Renders nothing until the window reports a chunk-load error, then shows
 * {@link UpdateAvailableNotice} pinned to the top of the viewport. It stays
 * until the page reloads: the old build cannot recover on its own.
 *
 * @param props - see {@link ChunkLoadReloadPromptProps}.
 */
export function ChunkLoadReloadPrompt({
  target,
  onReload,
}: ChunkLoadReloadPromptProps) {
  const [visible, setVisible] = useState(false)

  useEffect(
    () => listenForChunkLoadErrors(target, () => setVisible(true)),
    [target]
  )

  if (!visible) {
    return null
  }
  return (
    <div className="fixed inset-x-0 top-0 z-50 mx-auto max-w-xl p-4">
      <UpdateAvailableNotice onReload={onReload} />
    </div>
  )
}
