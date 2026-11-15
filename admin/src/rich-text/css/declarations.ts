/**
 * Reads a `style` attribute into declarations, as the server's
 * `src/domain/rich_text/css/declarations.rs` does with the cssparser 0.38
 * tokenizer (editor.md §3.1, §7.3).
 *
 * This is the token-level half of `canonicalStyle`: it splits the attribute
 * into declarations, reads each value component by component, and refuses
 * every token that no content-v1 property can use. The property-level half
 * is `canonical-style.ts`. editor.md §3.1 calls the two canonicalisers "the
 * same rules written twice", so this module mirrors the server's reader rule
 * for rule, and `css-vectors.jsonl` checks that they agree:
 *
 * - **Recovery is per declaration.** A declaration that cannot be read is
 *   skipped up to and including the next top-level `;`. A `;` inside a
 *   block, a function, a string, a comment or a `url()` does not end it.
 * - **Names** are identifiers, ASCII-lower-cased, followed by `:`.
 * - **Values** are identifiers, numbers, dimensions, percentages, hashes,
 *   `/`, and the functions `rgb()` and `rgba()` with numeric arguments.
 *   Every other token makes the declaration unreadable.
 * - **Comments** make the declaration they are in unreadable.
 * - **`!important`** at the end of a value is stripped; a `!` anywhere else
 *   makes the declaration unreadable.
 * - **Limits.** At most {@link MAX_DECLARATIONS} non-empty declarations are
 *   read; a value of more than {@link MAX_COMPONENTS} components, or an
 *   `rgb()` of more than seven arguments, is unreadable.
 * - **Numbers** are read from their source text with `Number()`, which
 *   rounds correctly, as the server's `f64` parser does, so both sides start
 *   from the same value.
 *
 * The tokenizer follows cssparser 0.38.0 (`tokenizer.rs`), not only CSS
 * Syntax 3, where the two differ in a way that moves a declaration's end:
 * a `-` followed by `\` always starts an identifier, an unclosed block or
 * `rgb(` ends at the end of the input, and a bad string stops before its
 * newline. Delimiters are found the way cssparser's parser finds them
 * (`parser.rs`): a `;` ends a declaration only between tokens at the top
 * level, and inside `rgb(...)` only `)` ends the arguments.
 */

/**
 * The most declarations one `style` attribute may hold; later ones are not
 * read (editor.md §7.3, "Limits"). Like the server's other reader limits it
 * is not in `policy.json`; `css-vectors.jsonl` pins it.
 */
export const MAX_DECLARATIONS = 32

/**
 * The most components one value may hold. No content-v1 grammar needs more
 * than four (`border-width: 1px 2px 3px 4px`).
 */
export const MAX_COMPONENTS = 8

/** The most arguments of one `rgb()` call: three channels, an alpha, three separators. */
const MAX_ARGUMENTS = 7

/** One argument of an `rgb()` or `rgba()` call. */
export type Argument =
  | { readonly kind: "number"; readonly value: number }
  | { readonly kind: "percentage"; readonly value: number }
  | { readonly kind: "comma" }
  | { readonly kind: "slash" }

/** One component of a declaration value. */
export type Component =
  /** An identifier, ASCII-lower-cased. */
  | { readonly kind: "ident"; readonly value: string }
  /** A number without a unit. */
  | { readonly kind: "number"; readonly value: number }
  /** A number with a unit; the unit is ASCII-lower-cased. */
  | {
      readonly kind: "dimension"
      readonly value: number
      readonly unit: string
    }
  /** A percentage, as the number before the `%`. */
  | { readonly kind: "percentage"; readonly value: number }
  /** A hash token such as `#c00000`, without the `#`, as written. */
  | { readonly kind: "hash"; readonly value: string }
  /** An `rgb()` or `rgba()` call with its arguments. */
  | { readonly kind: "rgb"; readonly arguments: readonly Argument[] }
  /** `/`, as in `aspect-ratio: 16 / 9`. */
  | { readonly kind: "slash" }

/** One declaration that could be read. */
export interface Declaration {
  /** The property name, ASCII-lower-cased. */
  readonly name: string
  /** The value's components, without whitespace and without `!important`. */
  readonly value: readonly Component[]
}

/** What reading one `style` attribute gave. */
export interface Declarations {
  /** The declarations that could be read, in source order. */
  readonly read: readonly Declaration[]
  /**
   * How many non-empty declarations could not be read, or came after the
   * first {@link MAX_DECLARATIONS}.
   */
  readonly unreadable: number
}

/** The three kinds of block a token can open. */
type Block = "paren" | "square" | "curly"

/** One token, reduced to what this reader tells apart. */
type Token =
  | { readonly type: "whitespace" }
  | { readonly type: "comment" }
  | { readonly type: "ident"; readonly value: string }
  | { readonly type: "function"; readonly name: string }
  | { readonly type: "hash"; readonly value: string }
  | { readonly type: "number"; readonly text: string }
  | { readonly type: "percentage"; readonly text: string }
  | { readonly type: "dimension"; readonly text: string; readonly unit: string }
  | { readonly type: "delim"; readonly value: string }
  | { readonly type: "colon" }
  | { readonly type: "semicolon" }
  | { readonly type: "comma" }
  | { readonly type: "open"; readonly block: Block }
  | { readonly type: "close"; readonly block: Block }
  /** Strings, URLs, at-keywords, CDO, CDC and the match operators: never usable. */
  | { readonly type: "other" }

/** Thrown inside the reader when a declaration cannot be read. */
class Unreadable {}

const REPLACEMENT = "�"

const isDigit = (c: number) => c >= 0x30 && c <= 0x39
const isHexDigit = (c: number) =>
  isDigit(c) || (c >= 0x41 && c <= 0x46) || (c >= 0x61 && c <= 0x66)
const isLetter = (c: number) =>
  (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a)
const isNewline = (c: number) => c === 0x0a || c === 0x0d || c === 0x0c
/** A code unit that may start a name: a letter, `_`, NUL or any non-ASCII unit. */
const isNameStart = (c: number) =>
  isLetter(c) || c === 0x5f || c === 0 || c >= 0x80
/** A code unit that may continue a name without an escape. */
const isNameUnit = (c: number) => isNameStart(c) || isDigit(c) || c === 0x2d

/**
 * Lower-cases ASCII letters only, as Rust's `to_ascii_lowercase` does: CSS
 * keywords and units match regardless of ASCII case, and nothing else.
 */
export function asciiLowerCase(text: string): string {
  return text.replace(/[A-Z]+/g, (upper) => upper.toLowerCase())
}

/** cssparser 0.38's tokenizer over one string, as far as this reader needs it. */
class Tokenizer {
  position = 0

  private readonly input: string

  constructor(input: string) {
    this.input = input
  }

  /** Whether the whole input has been read. */
  get atEnd(): boolean {
    return this.position >= this.input.length
  }

  /** The code unit `offset` units ahead, or -1 past the end. */
  peek(offset = 0): number {
    const index = this.position + offset
    return index < this.input.length ? this.input.charCodeAt(index) : -1
  }

  /** Whether a newline is `offset` units ahead. */
  private newlineAt(offset: number): boolean {
    const c = this.peek(offset)
    return c !== -1 && isNewline(c)
  }

  private startsWith(text: string): boolean {
    return this.input.startsWith(text, this.position)
  }

  /** Steps over one newline; CR LF counts as one. */
  private consumeNewline(): void {
    const c = this.peek()
    this.position += 1
    if (c === 0x0d && this.peek() === 0x0a) {
      this.position += 1
    }
  }

  /** cssparser's `is_ident_start`, including its reading of `-\`. */
  private identStartsHere(): boolean {
    const c = this.peek()
    if (c === -1) {
      return false
    }
    if (c === 0x2d) {
      const next = this.peek(1)
      if (next === -1) {
        return false
      }
      // cssparser 0.38 checks for a newline at offset 1, which is the `\`
      // itself, so `-\` always starts an identifier.
      return isNameStart(next) || next === 0x2d || next === 0x5c
    }
    if (c === 0x5c) {
      return !this.newlineAt(1)
    }
    return isNameStart(c)
  }

  /** One escaped code point, after the `\` (cssparser's `consume_escape`). */
  private consumeEscape(): string {
    const c = this.peek()
    if (c === -1) {
      return REPLACEMENT
    }
    if (isHexDigit(c)) {
      let value = 0
      let digits = 0
      while (digits < 6 && this.peek() !== -1 && isHexDigit(this.peek())) {
        value = value * 16 + Number.parseInt(this.input[this.position], 16)
        digits += 1
        this.position += 1
      }
      const after = this.peek()
      if (after === 0x20 || after === 0x09) {
        this.position += 1
      } else if (after !== -1 && isNewline(after)) {
        this.consumeNewline()
      }
      const isScalar =
        value !== 0 &&
        value <= 0x10ffff &&
        !(value >= 0xd800 && value <= 0xdfff)
      return isScalar ? String.fromCodePoint(value) : REPLACEMENT
    }
    if (c === 0) {
      this.position += 1
      return REPLACEMENT
    }
    const codePoint = this.input.codePointAt(this.position) ?? c
    const text = String.fromCodePoint(codePoint)
    this.position += text.length
    return text
  }

  /** A name, with escapes decoded and NUL replaced (`consume_name`). */
  private consumeName(): string {
    let name = ""
    for (;;) {
      const c = this.peek()
      if (c === -1) {
        return name
      }
      if (c === 0x5c) {
        if (this.newlineAt(1)) {
          return name
        }
        this.position += 1
        name += this.consumeEscape()
      } else if (c === 0) {
        this.position += 1
        name += REPLACEMENT
      } else if (isNameUnit(c)) {
        name += this.input[this.position]
        this.position += 1
      } else {
        return name
      }
    }
  }

  /** A number, percentage or dimension (`consume_numeric`). */
  private consumeNumeric(): Token {
    const start = this.position
    if (this.peek() === 0x2b || this.peek() === 0x2d) {
      this.position += 1
    }
    while (isDigit(this.peek())) {
      this.position += 1
    }
    if (this.peek() === 0x2e && isDigit(this.peek(1))) {
      this.position += 1
      while (isDigit(this.peek())) {
        this.position += 1
      }
    }
    const e = this.peek()
    if (e === 0x45 || e === 0x65) {
      const sign = this.peek(1) === 0x2b || this.peek(1) === 0x2d ? 1 : 0
      if (isDigit(this.peek(1 + sign))) {
        this.position += 1 + sign
        while (isDigit(this.peek())) {
          this.position += 1
        }
      }
    }
    const text = this.input.slice(start, this.position)
    if (this.peek() === 0x25) {
      this.position += 1
      return { type: "percentage", text }
    }
    if (this.identStartsHere()) {
      return { type: "dimension", text, unit: this.consumeName() }
    }
    return { type: "number", text }
  }

  /** An identifier, a function or a URL (`consume_ident_like`). */
  private consumeIdentLike(): Token {
    const name = this.consumeName()
    if (this.peek() !== 0x28) {
      return { type: "ident", value: name }
    }
    this.position += 1
    if (asciiLowerCase(name) === "url" && this.consumeUnquotedUrl()) {
      return { type: "other" }
    }
    return { type: "function", name }
  }

  /**
   * After `url(`: an unquoted URL, good or bad, consumed whole, or `false`
   * (nothing consumed) when a quote follows, which makes `url(` a function
   * (`consume_unquoted_url`).
   */
  private consumeUnquotedUrl(): boolean {
    let offset = 0
    for (;;) {
      const c = this.peek(offset)
      if (c === -1) {
        this.position = this.input.length
        return true
      }
      if (c === 0x22 || c === 0x27) {
        return false
      }
      if (c === 0x29) {
        this.position += offset + 1
        return true
      }
      if (c !== 0x20 && c !== 0x09 && !isNewline(c)) {
        break
      }
      offset += 1
    }
    this.position += offset
    for (;;) {
      const c = this.peek()
      if (c === -1 || c === 0x29) {
        this.position += c === -1 ? 0 : 1
        return true
      }
      if (c === 0x20 || c === 0x09 || isNewline(c)) {
        this.consumeUrlEnd()
        return true
      }
      if (
        c === 0x22 ||
        c === 0x27 ||
        c === 0x28 ||
        (c >= 0x01 && c <= 0x08) ||
        c === 0x0b ||
        (c >= 0x0e && c <= 0x1f) ||
        c === 0x7f
      ) {
        this.position += 1
        this.consumeBadUrl()
        return true
      }
      if (c === 0x5c) {
        this.position += 1
        if (this.newlineAt(0)) {
          this.consumeBadUrl()
          return true
        }
        this.consumeEscape()
      } else {
        this.position += 1
      }
    }
  }

  /** Whitespace after an unquoted URL, then `)` or a bad URL. */
  private consumeUrlEnd(): void {
    for (;;) {
      const c = this.peek()
      if (c === -1) {
        return
      }
      if (c === 0x29) {
        this.position += 1
        return
      }
      if (isNewline(c)) {
        this.consumeNewline()
      } else if (c === 0x20 || c === 0x09) {
        this.position += 1
      } else {
        this.position += 1
        this.consumeBadUrl()
        return
      }
    }
  }

  /** The rest of a bad URL, up to and including `)`. */
  private consumeBadUrl(): void {
    for (;;) {
      const c = this.peek()
      if (c === -1) {
        return
      }
      if (c === 0x29) {
        this.position += 1
        return
      }
      if (c === 0x5c) {
        this.position += 1
        if (this.peek() === 0x29 || this.peek() === 0x5c) {
          this.position += 1
        }
      } else if (isNewline(c)) {
        this.consumeNewline()
      } else {
        this.position += 1
      }
    }
  }

  /** A quoted string; a bad one stops before its newline (`consume_quoted_string`). */
  private consumeString(quote: number): void {
    this.position += 1
    for (;;) {
      const c = this.peek()
      if (c === -1) {
        return
      }
      if (c === quote) {
        this.position += 1
        return
      }
      if (isNewline(c)) {
        return
      }
      this.position += 1
      if (c === 0x5c && this.peek() !== -1) {
        if (isNewline(this.peek())) {
          this.consumeNewline()
        } else {
          this.consumeEscape()
        }
      }
    }
  }

  /** The next token, or `null` at the end (`next_token_unchecked`). */
  next(): Token | null {
    const c = this.peek()
    if (c === -1) {
      return null
    }
    if (c === 0x20 || c === 0x09 || isNewline(c)) {
      while (this.peek() !== -1) {
        const w = this.peek()
        if (isNewline(w)) {
          this.consumeNewline()
        } else if (w === 0x20 || w === 0x09) {
          this.position += 1
        } else {
          break
        }
      }
      return { type: "whitespace" }
    }
    if (c === 0x22 || c === 0x27) {
      this.consumeString(c)
      return { type: "other" }
    }
    if (isDigit(c)) {
      return this.consumeNumeric()
    }
    if (c === 0x2b || c === 0x2d || c === 0x2e) {
      const numberFollows =
        isDigit(this.peek(1)) ||
        (c !== 0x2e && this.peek(1) === 0x2e && isDigit(this.peek(2)))
      if (numberFollows) {
        return this.consumeNumeric()
      }
      if (c === 0x2d && this.startsWith("-->")) {
        this.position += 3
        return { type: "other" }
      }
      if (c === 0x2d && this.identStartsHere()) {
        return this.consumeIdentLike()
      }
      this.position += 1
      return { type: "delim", value: this.input[this.position - 1] }
    }
    if (c === 0x23) {
      this.position += 1
      if (
        this.identStartsHere() ||
        isDigit(this.peek()) ||
        this.peek() === 0x2d
      ) {
        return { type: "hash", value: this.consumeName() }
      }
      return { type: "delim", value: "#" }
    }
    if (c === 0x2f && this.peek(1) === 0x2a) {
      const end = this.input.indexOf("*/", this.position + 2)
      this.position = end === -1 ? this.input.length : end + 2
      return { type: "comment" }
    }
    if (c === 0x3c && this.startsWith("<!--")) {
      this.position += 4
      return { type: "other" }
    }
    if (c === 0x40) {
      this.position += 1
      if (this.identStartsHere()) {
        this.consumeName()
        return { type: "other" }
      }
      return { type: "delim", value: "@" }
    }
    if (c === 0x5c) {
      if (!this.newlineAt(1)) {
        return this.consumeIdentLike()
      }
      this.position += 1
      return { type: "delim", value: "\\" }
    }
    if (isNameStart(c)) {
      return this.consumeIdentLike()
    }
    const simple = SIMPLE_TOKENS.get(c)
    if (simple !== undefined) {
      this.position += 1
      return simple
    }
    // `$=`, `*=`, `^=`, `|=` and `~=` are one token each in cssparser; none
    // is usable, and neither character ends a declaration, so reading them
    // as two delimiters changes nothing.
    this.position += 1
    return { type: "delim", value: this.input[this.position - 1] }
  }
}

/** The single-character tokens other than delimiters. */
const SIMPLE_TOKENS: ReadonlyMap<number, Token> = new Map<number, Token>([
  [0x3a, { type: "colon" }],
  [0x3b, { type: "semicolon" }],
  [0x2c, { type: "comma" }],
  [0x28, { type: "open", block: "paren" }],
  [0x5b, { type: "open", block: "square" }],
  [0x7b, { type: "open", block: "curly" }],
  [0x29, { type: "close", block: "paren" }],
  [0x5d, { type: "close", block: "square" }],
  [0x7d, { type: "close", block: "curly" }],
])

/** The block a token opens: functions and `(` a parenthesis, `[` and `{` their own. */
function opening(token: Token): Block | null {
  if (token.type === "function") {
    return "paren"
  }
  return token.type === "open" ? token.block : null
}

/** What stops a delimited parser: a top-level `;`, or the `)` of `rgb(`. */
type Stop = ";" | ")" | null

/**
 * cssparser's `Parser`, as far as this reader uses it: delimited parsing up
 * to a stop character, and blocks that are skipped whole.
 */
class Parser {
  readonly tokenizer: Tokenizer
  private stopBefore: Stop = null
  /** The block that the last token returned opened, not yet entered. */
  private atStartOf: Block | null = null

  constructor(input: string) {
    this.tokenizer = new Tokenizer(input)
  }

  /** Skips the rest of a block whose opening token was read (`consume_until_end_of_block`). */
  private consumeUntilEndOfBlock(block: Block): void {
    const stack: Block[] = [block]
    for (
      let token = this.tokenizer.next();
      token;
      token = this.tokenizer.next()
    ) {
      if (token.type === "close" && stack[stack.length - 1] === token.block) {
        stack.pop()
        if (stack.length === 0) {
          return
        }
      }
      const opened = opening(token)
      if (opened !== null) {
        stack.push(opened)
      }
    }
  }

  /** The next token, or `null` at the end or before the stop character. */
  private nextToken(): Token | null {
    if (this.atStartOf !== null) {
      this.consumeUntilEndOfBlock(this.atStartOf)
      this.atStartOf = null
    }
    const c = this.tokenizer.peek()
    if (
      c === -1 ||
      (this.stopBefore !== null && c === this.stopBefore.charCodeAt(0))
    ) {
      return null
    }
    const token = this.tokenizer.next()
    if (token !== null) {
      this.atStartOf = opening(token)
    }
    return token
  }

  /**
   * The next token that is not whitespace, or `null` at the end.
   *
   * @throws {Unreadable} on a comment.
   */
  nextSignificant(): Token | null {
    for (;;) {
      const token = this.nextToken()
      if (token === null || token.type !== "whitespace") {
        if (token?.type === "comment") {
          throw new Unreadable()
        }
        return token
      }
    }
  }

  /**
   * Runs `parse` on the input up to the next top-level `;`, then skips what
   * it left and the `;` itself (`parse_until_after`).
   *
   * @returns what `parse` returned, or {@link Unreadable} when it threw.
   */
  parseUntilAfterSemicolon<T>(parse: () => T): T | Unreadable {
    const outer = this.stopBefore
    this.stopBefore = ";"
    let result: T | Unreadable
    try {
      result = parse()
    } catch (error) {
      if (!(error instanceof Unreadable)) {
        throw error
      }
      result = error
    } finally {
      this.stopBefore = outer
    }
    if (this.atStartOf !== null) {
      this.consumeUntilEndOfBlock(this.atStartOf)
      this.atStartOf = null
    }
    while (!this.tokenizer.atEnd && this.tokenizer.peek() !== 0x3b) {
      const token = this.tokenizer.next()
      const opened = token === null ? null : opening(token)
      if (opened !== null) {
        this.consumeUntilEndOfBlock(opened)
      }
    }
    if (this.tokenizer.peek() === 0x3b) {
      this.tokenizer.position += 1
    }
    return result
  }

  /**
   * Runs `parse` inside the block that the last token opened, up to its
   * `)`, then skips the rest of the block whether `parse` succeeded or not
   * (`parse_nested_block`).
   */
  parseNestedBlock<T>(parse: () => T): T {
    const block = this.atStartOf ?? "paren"
    this.atStartOf = null
    const outer = this.stopBefore
    this.stopBefore = ")"
    try {
      return parse()
    } finally {
      if (this.atStartOf !== null) {
        this.consumeUntilEndOfBlock(this.atStartOf)
        this.atStartOf = null
      }
      this.consumeUntilEndOfBlock(block)
      this.stopBefore = outer
    }
  }
}

/**
 * The value of a numeric token's text, which the CSS number syntax already
 * limited to a sign, digits, a fraction and an exponent.
 *
 * @throws {Unreadable} when it is not finite (`1e400`).
 */
function numberOf(text: string): number {
  const value = Number(text)
  if (!Number.isFinite(value)) {
    throw new Unreadable()
  }
  return value
}

/** One declaration, up to the next top-level `;`: `null` when it is empty. */
function declaration(parser: Parser): Declaration | null {
  const name = parser.nextSignificant()
  if (name === null) {
    return null
  }
  if (name.type !== "ident" || parser.nextSignificant()?.type !== "colon") {
    throw new Unreadable()
  }
  return { name: asciiLowerCase(name.value), value: components(parser) }
}

/** The components of a value, up to its end or a trailing `!important`. */
function components(parser: Parser): Component[] {
  const value: Component[] = []
  for (
    let token = parser.nextSignificant();
    token;
    token = parser.nextSignificant()
  ) {
    let component: Component
    switch (token.type) {
      case "ident":
        component = { kind: "ident", value: asciiLowerCase(token.value) }
        break
      case "number":
        component = { kind: "number", value: numberOf(token.text) }
        break
      case "percentage":
        component = { kind: "percentage", value: numberOf(token.text) }
        break
      case "dimension":
        component = {
          kind: "dimension",
          value: numberOf(token.text),
          unit: asciiLowerCase(token.unit),
        }
        break
      case "hash":
        component = { kind: "hash", value: token.value }
        break
      case "delim":
        if (token.value === "/") {
          component = { kind: "slash" }
          break
        }
        if (token.value === "!") {
          important(parser)
          return value
        }
        throw new Unreadable()
      case "function": {
        const name = asciiLowerCase(token.name)
        if (name !== "rgb" && name !== "rgba") {
          throw new Unreadable()
        }
        component = {
          kind: "rgb",
          arguments: parser.parseNestedBlock(() => rgbArguments(parser)),
        }
        break
      }
      default:
        throw new Unreadable()
    }
    value.push(component)
    if (value.length > MAX_COMPONENTS) {
      throw new Unreadable()
    }
  }
  return value
}

/** After a `!`: the keyword `important` and then nothing but the end. */
function important(parser: Parser): void {
  const keyword = parser.nextSignificant()
  if (
    keyword?.type !== "ident" ||
    asciiLowerCase(keyword.value) !== "important" ||
    parser.nextSignificant() !== null
  ) {
    throw new Unreadable()
  }
}

/** The arguments inside `rgb(...)` or `rgba(...)`. */
function rgbArguments(parser: Parser): Argument[] {
  const list: Argument[] = []
  for (
    let token = parser.nextSignificant();
    token;
    token = parser.nextSignificant()
  ) {
    if (token.type === "number") {
      list.push({ kind: "number", value: numberOf(token.text) })
    } else if (token.type === "percentage") {
      list.push({ kind: "percentage", value: numberOf(token.text) })
    } else if (token.type === "comma") {
      list.push({ kind: "comma" })
    } else if (token.type === "delim" && token.value === "/") {
      list.push({ kind: "slash" })
    } else {
      throw new Unreadable()
    }
    if (list.length > MAX_ARGUMENTS) {
      throw new Unreadable()
    }
  }
  return list
}

/**
 * Reads a `style` attribute into its declarations.
 *
 * @param style - the attribute value as the DOM holds it, character
 *   references already decoded (as ammonia passes it to the server's
 *   attribute filter).
 * @returns the readable declarations in source order, and how many were
 *   not read.
 */
export function readDeclarations(style: string): Declarations {
  const parser = new Parser(style)
  const read: Declaration[] = []
  let unreadable = 0
  while (!parser.tokenizer.atEnd) {
    const chunk = parser.parseUntilAfterSemicolon(() => declaration(parser))
    const counted = read.length + unreadable
    if (chunk === null) {
      // Whitespace between two `;` is no declaration.
      continue
    }
    if (!(chunk instanceof Unreadable) && counted < MAX_DECLARATIONS) {
      read.push(chunk)
    } else {
      unreadable += 1
    }
  }
  return { read, unreadable }
}

/**
 * Reads a bare value, such as the locked value `0 auto`, with the same rules
 * as a declaration's value. Nothing ends it but the end of the input, so a
 * `;` makes it unreadable.
 *
 * @param value - the value text, without a property name.
 * @returns its components, or `null` when it cannot be read.
 */
export function readValue(value: string): Component[] | null {
  const parser = new Parser(value)
  try {
    return components(parser)
  } catch (error) {
    if (error instanceof Unreadable) {
      return null
    }
    throw error
  }
}
