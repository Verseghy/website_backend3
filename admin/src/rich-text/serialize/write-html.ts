/**
 * The string writer of the stored-HTML serialiser (editor.md §6 step 3), in
 * the dialect that html5ever 0.40.1 writes (editor.md §3.8;
 * html5ever@0.40.1:src/serialize/mod.rs, `HtmlSerializer`).
 *
 * The server re-serialises every body with html5ever after sanitising it, so
 * when the editor writes the same dialect, an unchanged body is unchanged
 * byte for byte on the way through (editor.md §6, P2: `S(e) == e`). The
 * writer depends on no engine's `innerHTML`, whose escaping differs between
 * browsers and jsdom:
 *
 * - in text, only `&`, `<`, `>` and U+00A0 are escaped (`&amp;`, `&lt;`,
 *   `&gt;`, `&nbsp;`); in attribute values also `"` (`&quot;`);
 * - every other character is literal, attribute values are always
 *   double-quoted, and an empty value is written `=""`;
 * - the void elements of html5ever's list get no end tag and no children;
 * - text inside the raw-text elements of html5ever's list (`iframe`,
 *   `style`, `script`, ...) is written unescaped, as html5ever does with
 *   scripting on; content-v1's one raw-text element, `iframe`, has no
 *   children (editor.md §3.2), so this only keeps the dialect whole.
 *
 * The writer's own rules write each text and attribute value as an HTML
 * parse reads it back, so the server and the next load see exactly what was
 * written (P2, P-fix):
 *
 * - a lone UTF-16 surrogate, which a JavaScript string can hold but UTF-8
 *   cannot, becomes U+FFFD, the character the server's parser and JSON
 *   reader would put in its place;
 * - CR LF and a lone CR become LF, as the parser's input stream turns them
 *   (WHATWG HTML, "Preprocessing the input stream");
 * - U+0000 is left out of text, where the tree builder's "in body" rule
 *   ignores it (html5ever@0.40.1:src/tree_builder/rules.rs,
 *   `Token::NullCharacter`), and becomes U+FFFD in an attribute value and in
 *   raw text, as the tokenizer's states write it
 *   (html5ever@0.40.1:src/tokenizer/mod.rs).
 *
 * `toStoredHtml`'s text holds neither CR nor U+0000 by the time it gets here
 * (`whitespace.ts`, which must drop U+0000 before tightening sees the
 * paragraph), so the last two rules are for attribute values, such as an
 * `href` set from a dialog, which never passed an HTML parse. Every string
 * the writer returns is therefore well-formed Unicode, and an HTML parse of
 * it reads back the values it wrote.
 */
import type { StoredNode } from "./stored-tree"

/** The elements html5ever writes without an end tag or children. */
export const VOID_ELEMENTS: ReadonlySet<string> = new Set([
  "area",
  "base",
  "basefont",
  "bgsound",
  "br",
  "col",
  "embed",
  "frame",
  "hr",
  "img",
  "input",
  "keygen",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
])

/** The elements whose text html5ever writes unescaped (scripting on). */
export const RAW_TEXT_ELEMENTS: ReadonlySet<string> = new Set([
  "style",
  "script",
  "xmp",
  "iframe",
  "noembed",
  "noframes",
  "plaintext",
  "noscript",
])

const LONE_SURROGATE =
  /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/g

/** CR LF and a lone CR, which the parser's input stream reads as LF. */
const CARRIAGE_RETURN = /\r\n?/g

/** U+0000, which no HTML parse reads back as itself. */
const NUL = "\u0000"

/** U+FFFD, which a parse puts in place of what it cannot keep. */
const REPLACEMENT_CHARACTER = "\uFFFD"

/**
 * A value as an HTML parse reads it back (the writer's own rules, above).
 *
 * @param value - the text or attribute value.
 * @param nul - what U+0000 becomes: nothing in text, U+FFFD in an
 *   attribute value or in raw text.
 */
function asParsed(value: string, nul: "" | typeof REPLACEMENT_CHARACTER) {
  return value
    .replace(LONE_SURROGATE, REPLACEMENT_CHARACTER)
    .replace(CARRIAGE_RETURN, "\n")
    .replaceAll(NUL, nul)
}

const TEXT_ESCAPES: Readonly<Record<string, string>> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  "\u00a0": "&nbsp;",
}

const ATTRIBUTE_ESCAPES: Readonly<Record<string, string>> = {
  ...TEXT_ESCAPES,
  '"': "&quot;",
}

/**
 * Escapes text as html5ever's `write_escaped` does, after the writer's own
 * rules (module doc) have turned it into what an HTML parse reads back.
 *
 * @param value - the text.
 * @param attribute - whether it is an attribute value, which also escapes
 *   `"` and turns U+0000 into U+FFFD instead of leaving it out.
 * @returns the escaped, well-formed text.
 */
export function escapeHtml(value: string, attribute: boolean): string {
  const escapes = attribute ? ATTRIBUTE_ESCAPES : TEXT_ESCAPES
  return asParsed(value, attribute ? REPLACEMENT_CHARACTER : "").replace(
    attribute ? /[&<>"\u00a0]/g : /[&<>\u00a0]/g,
    (c) => escapes[c]
  )
}

/**
 * Writes nodes in html5ever's dialect.
 *
 * @param nodes - the nodes, in order.
 * @param rawText - whether their parent is a raw-text element, so text is
 *   written unescaped.
 * @param into - the output pieces.
 */
function writeNodes(
  nodes: readonly StoredNode[],
  rawText: boolean,
  into: string[]
): void {
  for (const node of nodes) {
    if (node.kind === "text") {
      into.push(
        rawText
          ? asParsed(node.text, REPLACEMENT_CHARACTER)
          : escapeHtml(node.text, false)
      )
      continue
    }
    into.push("<", node.tag)
    for (const [name, value] of node.attributes) {
      into.push(" ", name, '="', escapeHtml(value, true), '"')
    }
    into.push(">")
    if (VOID_ELEMENTS.has(node.tag)) {
      continue
    }
    writeNodes(node.children, RAW_TEXT_ELEMENTS.has(node.tag), into)
    into.push("</", node.tag, ">")
  }
}

/**
 * Writes a stored tree as the HTML string that is stored (editor.md §3.8).
 *
 * @param nodes - the top-level nodes of the body.
 * @returns the HTML, with no whitespace added between elements.
 */
export function writeHtml(nodes: readonly StoredNode[]): string {
  const into: string[] = []
  writeNodes(nodes, false, into)
  return into.join("")
}
