/**
 * How the schema's extensions tell staff about something they did to the
 * pasted or inserted content, without depending on React or the admin's
 * notification system: the editor input passes a callback, and an
 * extension calls it with the `editor.*` key of the message to show
 * (ADR 0021 §2, `editor.*`). The YouTube paste rule reports with it the
 * player options it dropped (ADR 0043 §4, D134) and the YouTube links it
 * cannot embed.
 */
import type { MessageKey } from "@/i18n/keys"

/** The key of a message of the rich-text editor's namespace. */
export type EditorNotice = Extract<MessageKey, `editor.${string}`>

/**
 * Shows one editor notice to the staff member.
 *
 * @param notice - the message's key in `hu.ts`.
 */
export type NotifyEditor = (notice: EditorNotice) => void

/** The callback of a schema that shows no notices (headless editors, tests). */
export const IGNORE_NOTICES: NotifyEditor = () => {}
