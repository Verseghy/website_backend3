/**
 * Scrubbing of user names out of the file paths a clipboard carries.
 *
 * Why: Word and Excel put local file paths on the clipboard. Word's HTML
 * links every picture and its file list to the profile's temporary folder
 * (`file:///C:/Users/<name>/AppData/Local/Temp/msohtmlclip1/...`), and a
 * plain-text copy of a path carries the same folder. The captures become
 * public fixtures, so the profile folder's name is replaced before the
 * capture is ever written (editor.md §13.1, risk R16). Names inside the
 * document itself are not detected; the recorder's page asks for a review
 * before a capture is committed.
 *
 * Only the name segment changes, to {@link SCRUBBED_USER}. The separators,
 * the drive letter and the spelling of `Users` stay as they were. When a
 * path continues after the name, as Office's temporary paths always do, the
 * name runs exactly to the next separator. When the name ends the path, the
 * scrubber cannot know where it ends, so it errs towards removing too much:
 * the name runs on, spaces included („Teszt Elek”), until a character no
 * file name can hold, a tab or line break, a space before more text that
 * contains one of those, the drive letter of the next Windows path, or a
 * closing `'` or `)` (a single-quoted attribute, a CSS `url()`). Anything
 * else after the name, such as an HTML entity, goes with it.
 */

/** What a user folder's name is replaced with. */
export const SCRUBBED_USER = "USER"

/**
 * A Windows path separator: `\` or `/`, or either one percent-encoded, as a
 * file URL may spell them. Hex digits match in both cases explicitly, so the
 * pattern does not depend on the flags of the regex that uses it.
 */
const SEP = String.raw`(?:[\\/]|%5[Cc]|%2[Ff])`

/** A macOS or Linux path separator: `/`, or `%2F` in a file URL. */
const POSIX_SEP = String.raw`(?:\/|%2[Ff])`

/**
 * One character of a folder name: anything except a separator, a character
 * that Windows forbids in file names (`<>:"|?*`), a tab or a line break.
 * Tabs end the name because Excel separates cells with them. A letter that
 * starts the next Windows path (`C:\`, `C%3A%5C`) ends it too, so a second
 * path on the same line is scrubbed on its own instead of being swallowed.
 */
const NAME_CHAR = String.raw`(?:(?!%5[Cc]|%2[Ff]|[A-Za-z](?::|%3[Aa])${SEP})[^\\/<>:"|?*\t\r\n])`

/**
 * A folder name. It starts with a non-space character and does not end in a
 * space or `%20`, so a space before the next path is not taken for part of
 * the name; nor in the middle of a percent escape (`%` or `%2`), which would
 * leave half of it behind. It ends in `'` or `)` only where a separator
 * follows, as inside `C:\Users\Teszt (Elek)\`; elsewhere that character
 * closes the context.
 */
const NAME = String.raw`(?:(?!\s)${NAME_CHAR}+(?<!\s|%20|%[0-9A-Fa-f]?)(?:(?<![')])|(?=${SEP})))`

/**
 * Windows profiles, case-insensitive: `C:\Users\<name>`, `c:/users/<name>`,
 * `C%3A%5CUsers%5C<name>` and the older `Documents and Settings`. The drive
 * letter may follow anything, even a letter: {@link NAME_CHAR} never takes
 * it into the name before it, so every pass sees the same drive letter.
 */
const WINDOWS = new RegExp(
  String.raw`([A-Za-z](?::|%3A)${SEP}+(?:Users|Documents(?: |%20)and(?: |%20)Settings)${SEP}+)(${NAME})`,
  "gi"
)

/**
 * macOS and Linux home folders, case-sensitive: `/Users/<name>` and
 * `/home/<name>`, with `/` or `%2F`, only where a path starts: at the start,
 * after whitespace, a quote, `(`, `=`, `>`, `[`, `,`, `;` or `:` (a label
 * such as `Path:`), after `file:` with any number of slashes (Office for Mac
 * writes `file:////Users/...`), or after `file://localhost`. A web URL such
 * as `https://example.com/Users/x` stays untouched.
 */
const POSIX = new RegExp(
  String.raw`((?<=^|[\s"'(=>\[,;:]|file:${POSIX_SEP}*|file:\/\/localhost)${POSIX_SEP}(?:Users|home)${POSIX_SEP}+)(${NAME})`,
  "g"
)

/** The result of {@link scrubUserPaths}. */
export interface ScrubResult {
  /** The input with every user folder name replaced by {@link SCRUBBED_USER}. */
  readonly value: string
  /**
   * How many names were replaced. A name that already is
   * {@link SCRUBBED_USER} is not counted, so scrubbing twice counts zero.
   */
  readonly replacements: number
}

/**
 * Replaces the user folder names in the Windows, macOS and Linux profile
 * paths of a text.
 *
 * @param input - clipboard text or HTML.
 * @returns the scrubbed text and the number of names replaced; see
 *   {@link ScrubResult}.
 */
export function scrubUserPaths(input: string): ScrubResult {
  let replacements = 0
  const replace = (_match: string, prefix: string, name: string) => {
    if (name !== SCRUBBED_USER) {
      replacements += 1
    }
    return `${prefix}${SCRUBBED_USER}`
  }
  // Windows first. The POSIX pattern can match the `/Users/` of a forward-
  // slash Windows path after its `:`, but by then the name is already
  // SCRUBBED_USER, which is neither changed nor counted again.
  const value = input.replace(WINDOWS, replace).replace(POSIX, replace)
  return { value, replacements }
}
