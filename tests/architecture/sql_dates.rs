//! The SQL date lint of ADR 0010 point 10.
//!
//! backend3's database session runs in UTC (point 9). SQL that takes the day
//! of a `timestamptz` without naming the zone therefore takes the UTC day: a
//! Budapest calendar day, stored as its Budapest midnight (22:00 or 23:00 UTC
//! of the day before, point 2), comes out one day early, the bug that dated
//! the legacy posts (ADR 0010, Context). Point 10 makes such SQL a
//! review-blocking defect, and "What must be true" asks for a lint that flags
//! `::date`, `now()::date` and `date_trunc(` without `AT TIME ZONE`. This is
//! that lint. It reads every string literal of every Rust file under `src/`
//! and `migration/`, and every `.sql` file there, as SQL, and reports:
//!
//! - a cast to `date` (`x::date`, `x :: DATE`, `x::"date"`,
//!   `x::pg_catalog.date`, and so `now()::date`) unless the value cast, past
//!   any other casts before it, is in parentheses that name the school's zone
//!   (below): `(starts_at AT TIME ZONE 'Europe/Budapest')::date` passes, and
//!   so does the index expression of ADR 0018 §4.1. `AT TIME ZONE` binds less
//!   tightly than `::`, so `x AT TIME ZONE 'Europe/Budapest'::date` casts the
//!   zone's name and is reported;
//! - the two other spellings of that cast, `CAST(x AS date)` and the
//!   function form `date(x)` (also `pg_catalog.date(x)` and `"date"(x)`; it
//!   is the MySQL idiom `DATE(col)` of the legacy code too), unless `x` names
//!   the school's zone;
//! - `date_trunc(...)` unless it truncates in the school's zone: its third
//!   argument, the zone of PostgreSQL's `date_trunc(field, source, zone)`, is
//!   that zone, or, with two arguments, its source names it;
//! - `current_date`, which is `now()::date`: today comes from the app clock,
//!   `budapest::today(now)` (point 7).
//!
//! SQL names a zone with `value AT TIME ZONE zone` or with its function form
//! `timezone(zone, value)`. A zone counts as the school's only when it is the
//! literal `'Europe/Budapest'` (`budapest::ZONE_NAME`, in any case, as
//! PostgreSQL reads zone names) or a bound parameter such as `$1`, and every
//! zone that the checked SQL names must count. The school's days are
//! Budapest days (point 4): `(x AT TIME ZONE 'UTC')::date` or
//! `date_trunc('day', x, 'UTC')` takes the UTC day, the very bug of the
//! legacy posts, and a second conversion such as
//! `x AT TIME ZONE 'Europe/Budapest' AT TIME ZONE 'UTC'` turns a wall time
//! back into an instant, which SQL never does (point 5). The lint does not
//! check what a parameter binds: the code binds `budapest::ZONE_NAME`.
//!
//! Comments and quoted text of the SQL itself are skipped (`--`, `/* */`,
//! `'...'`, `E'...'`), and so are the Rust comments around it, so prose about
//! a bare cast is no finding. The bodies of dollar-quoted functions
//! (`$$ ... $$`) are SQL and are read.
//!
//! Limits: the lint reads one literal at a time and knows no column types.
//! SQL built from several literals, or by Sea-ORM's query builder (a
//! `cast_as` to `date`), is not seen. The function form counts only when its
//! parenthesis follows `date` directly: `date (x)`, with a space, is read as
//! the prose that a string literal may hold ("a date (YYYY-MM-DD)"). Of a
//! zone, the lint reads only its first token, which is enough for a literal
//! or a parameter but passes `'Europe/Budapest' || x`. A cast that needs no
//! zone (a text literal, which can be written `DATE '2026-09-01'` instead,
//! or an interval for `date_trunc`) is reported too. Taking the hour, month
//! or year in other ways stays a review rule of point 10: with `extract`,
//! `date_part` or `to_char`, whether the zone is needed depends on the field
//! (`epoch` does not), and a cast of a `timestamptz` to `time` or
//! `timestamp` (`x::time`, `CAST(x AS time)`, `x::timestamp`) takes the UTC
//! time of day unless the value is `(x AT TIME ZONE 'Europe/Budapest')`, as
//! in ADR 0018's midnight check. ADR 0010's "What must be true" asks the
//! lint only for the casts to `date`, `date_trunc(` and the day of `now()`.

use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use backend3::domain::budapest::ZONE_NAME;
use proptest::prelude::*;

use super::{block_comment_end, char_literal_end, line_comment_end, raw_string_end, string_end};

/// The directories, relative to the repository root, whose SQL is checked.
const CHECKED_DIRECTORIES: &[&str] = &["src", "migration"];

/// The most characters of SQL that a finding quotes.
const SNIPPET_LENGTH: usize = 60;

/// Text that is read as SQL: the characters as the program sees them, each
/// with the line of the file it comes from.
#[derive(Debug, Default)]
struct SqlText {
    chars: Vec<char>,
    lines: Vec<usize>,
}

impl SqlText {
    fn push(&mut self, character: char, line: usize) {
        self.chars.push(character);
        self.lines.push(line);
    }
}

/// The line number, from 1, of every character of `chars`.
fn line_numbers(chars: &[char]) -> Vec<usize> {
    let mut line = 1;
    chars
        .iter()
        .map(|&character| {
            let this = line;
            if character == '\n' {
                line += 1;
            }
            this
        })
        .collect()
}

/// Every string literal of a Rust source, unescaped. Comments, character
/// literals and lifetimes are skipped with the scanners of the purity check.
fn string_literals(source: &str) -> Vec<SqlText> {
    let chars: Vec<char> = source.chars().collect();
    let lines = line_numbers(&chars);
    let mut literals = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        index = match (chars[index], chars.get(index + 1)) {
            ('/', Some('/')) => line_comment_end(&chars, index),
            ('/', Some('*')) => block_comment_end(&chars, index),
            ('"', _) => {
                let end = string_end(&chars, index);
                let content_end = if chars[end - 1] == '"' && end - 1 > index {
                    end - 1
                } else {
                    end
                };
                literals.push(unescape(&chars, &lines, index + 1..content_end));
                end
            }
            ('\'', _) => char_literal_end(&chars, index).unwrap_or(index + 1),
            ('r' | 'b', _) => match raw_string_end(&chars, index) {
                Some(end) => {
                    literals.push(raw_content(&chars, &lines, index, end));
                    end
                }
                None => index + 1,
            },
            _ => index + 1,
        };
    }
    literals
}

/// The content of the raw string literal `chars[start..end]` (`r"..."`,
/// `r#"..."#`, `br"..."`), which has no escapes.
fn raw_content(chars: &[char], lines: &[usize], start: usize, end: usize) -> SqlText {
    let quote = (start..end)
        .find(|&index| chars[index] == '"')
        .unwrap_or(end);
    let hashes = quote - start - if chars[start] == 'b' { 2 } else { 1 };
    let closed = end - hashes >= quote + 2 && chars[end - hashes - 1] == '"';
    let content_end = if closed { end - hashes - 1 } else { end };
    let mut text = SqlText::default();
    let content = quote + 1..content_end;
    for (&character, &line) in chars[content.clone()].iter().zip(&lines[content]) {
        text.push(character, line);
    }
    text
}

/// The content of an ordinary string literal as the program sees it: the
/// escapes that SQL could contain are resolved, a line continuation (a
/// backslash before the end of a line) drops the line break and the
/// indentation after it, and other escapes stay as written.
fn unescape(chars: &[char], lines: &[usize], range: Range<usize>) -> SqlText {
    let mut text = SqlText::default();
    let mut index = range.start;
    while index < range.end {
        let line = lines[index];
        if chars[index] != '\\' || index + 1 >= range.end {
            text.push(chars[index], line);
            index += 1;
            continue;
        }
        let escaped = match chars[index + 1] {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            '\\' | '"' | '\'' => chars[index + 1],
            next if next.is_whitespace() => {
                index += 1;
                while index < range.end && chars[index].is_whitespace() {
                    index += 1;
                }
                continue;
            }
            _ => {
                text.push('\\', line);
                index += 1;
                continue;
            }
        };
        text.push(escaped, line);
        index += 2;
    }
    text
}

/// A text file of SQL as a whole.
fn sql_file(source: &str) -> SqlText {
    let chars: Vec<char> = source.chars().collect();
    let lines = line_numbers(&chars);
    SqlText { chars, lines }
}

/// What a token of SQL is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// An unquoted keyword or identifier, in lower case.
    Word(String),
    /// A quoted identifier, as written between its double quotes.
    Quoted(String),
    /// A string literal (`'...'` or `E'...'`), as written between its quotes.
    Text(String),
    /// A bound parameter, `$1`.
    Parameter,
    /// `::`
    Cast,
    /// `(`
    Open,
    /// `)`
    Close,
    /// `,`
    Comma,
    /// `.`, between a schema and a name.
    Dot,
    /// A number or any other character.
    Other,
}

/// A token of SQL and the characters `start..end` of the text it spans.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

/// The end (exclusive) of a quoted run at `start` that ends with `quote`; a
/// doubled `quote` stands for itself, and `backslash` makes `\` escape the
/// next character, as in PostgreSQL's `E'...'`.
fn quoted_end(chars: &[char], start: usize, quote: char, backslash: bool) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '\\' if backslash => index += 2,
            found if found == quote && chars.get(index + 1) == Some(&quote) => index += 2,
            found if found == quote => return index + 1,
            _ => index += 1,
        }
    }
    chars.len()
}

/// Whether `character` may continue an unquoted SQL identifier.
fn word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// The end (exclusive) of a dollar-quote delimiter (`$$` or `$tag$`) at
/// `start`, or `None` when the `$` starts none.
fn dollar_quote_end(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
        index += 1;
    }
    (chars.get(index) == Some(&'$')).then_some(index + 1)
}

/// Splits SQL into tokens, skipping white space and comments. A
/// dollar-quote delimiter is skipped too, so a function body is read as SQL.
fn sql_tokens(chars: &[char]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let (character, next) = (chars[index], chars.get(index + 1).copied());
        let mut start = index;
        let (kind, end) = match character {
            _ if character.is_whitespace() => {
                index += 1;
                continue;
            }
            '-' if next == Some('-') => {
                index = line_comment_end(chars, index);
                continue;
            }
            '/' if next == Some('*') => {
                index = block_comment_end(chars, index);
                continue;
            }
            '\'' => {
                // `E'...'`: the `E` just before the quote makes it an escape
                // string literal, in which `\'` does not end it.
                let escape_string = tokens.last().is_some_and(|last| {
                    last.end == index && matches!(&last.kind, Kind::Word(word) if word == "e")
                });
                if escape_string {
                    start = tokens.pop().map_or(index, |prefix| prefix.start);
                }
                let end = quoted_end(chars, index, '\'', escape_string);
                // An unclosed literal runs to the end of the text.
                let closed = end > index + 1 && chars[end - 1] == '\'';
                let text = chars[index + 1..if closed { end - 1 } else { end }]
                    .iter()
                    .collect();
                (Kind::Text(text), end)
            }
            '"' => {
                let end = quoted_end(chars, index, '"', false);
                let name = chars[index + 1..end.saturating_sub(1).max(index + 1)]
                    .iter()
                    .collect();
                (Kind::Quoted(name), end)
            }
            '$' if next.is_some_and(|next| next.is_ascii_digit()) => {
                let digits = (index + 1..chars.len()).find(|&j| !chars[j].is_ascii_digit());
                (Kind::Parameter, digits.unwrap_or(chars.len()))
            }
            '$' => match dollar_quote_end(chars, index) {
                Some(end) => {
                    index = end;
                    continue;
                }
                None => (Kind::Other, index + 1),
            },
            ':' if next == Some(':') => (Kind::Cast, index + 2),
            '(' => (Kind::Open, index + 1),
            ')' => (Kind::Close, index + 1),
            ',' => (Kind::Comma, index + 1),
            '.' => (Kind::Dot, index + 1),
            _ if character.is_alphabetic() || character == '_' => {
                let end = (index..chars.len()).find(|&j| !word_char(chars[j]));
                let end = end.unwrap_or(chars.len());
                let word = chars[index..end].iter().collect::<String>().to_lowercase();
                (Kind::Word(word), end)
            }
            _ if character.is_ascii_digit() => {
                let end = (index..chars.len()).find(|&j| !(word_char(chars[j]) || chars[j] == '.'));
                (Kind::Other, end.unwrap_or(chars.len()))
            }
            _ => (Kind::Other, index + 1),
        };
        tokens.push(Token { kind, start, end });
        index = end;
    }
    tokens
}

/// Whether `token` is the unquoted keyword `word`.
fn is_word(token: Option<&Token>, word: &str) -> bool {
    matches!(token, Some(Token { kind: Kind::Word(found), .. }) if found == word)
}

/// Whether `token` is the name `name`, unquoted (and so in lower case) or
/// quoted.
fn is_name(token: Option<&Token>, name: &str) -> bool {
    matches!(
        token,
        Some(Token { kind: Kind::Word(found) | Kind::Quoted(found), .. }) if found == name
    )
}

/// Whether `token` is a name, unquoted or quoted.
fn is_any_name(token: &Token) -> bool {
    matches!(token.kind, Kind::Word(_) | Kind::Quoted(_))
}

/// The index of the last token of the type name `date` that starts at `at`:
/// `date` or `"date"`, or the schema-qualified `pg_catalog.date`. `None`
/// when no such name starts there.
fn date_type_end(tokens: &[Token], at: usize) -> Option<usize> {
    if is_name(tokens.get(at), "pg_catalog")
        && tokens.get(at + 1).is_some_and(|dot| dot.kind == Kind::Dot)
        && is_name(tokens.get(at + 2), "date")
    {
        Some(at + 2)
    } else {
        is_name(tokens.get(at), "date").then_some(at)
    }
}

/// How a piece of SQL names the time zone in which it takes a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    /// It names no zone, so the day is the session's, UTC (point 9).
    Missing,
    /// It names a zone other than the school's: another name, an interval,
    /// a column or an expression.
    Other,
    /// Every zone it names is the school's: the literal `'Europe/Budapest'`
    /// or a bound parameter.
    School,
}

/// Whether `token`, the first token of a zone, is the school's zone: the
/// literal `'Europe/Budapest'` in any case (PostgreSQL reads zone names
/// without regard to case), or a bound parameter, whose value the code
/// binds.
fn is_school_zone(token: Option<&Token>) -> bool {
    match token.map(|token| &token.kind) {
        Some(Kind::Text(name)) => name.eq_ignore_ascii_case(ZONE_NAME),
        Some(Kind::Parameter) => true,
        _ => false,
    }
}

/// The zones that `tokens` name: each `value AT TIME ZONE zone` names one,
/// and so does each `timezone(zone, value)`, its function form.
fn zone_of(tokens: &[Token]) -> Zone {
    let mut zone = Zone::Missing;
    for (index, token) in tokens.iter().enumerate() {
        let named = if is_word(Some(token), "at")
            && is_word(tokens.get(index + 1), "time")
            && is_word(tokens.get(index + 2), "zone")
        {
            Some(tokens.get(index + 3))
        } else if is_word(Some(token), "timezone")
            && tokens
                .get(index + 1)
                .is_some_and(|open| open.kind == Kind::Open)
        {
            // `timezone(value)` with one argument converts nothing.
            match split_arguments(arguments(tokens, index + 1)).as_slice() {
                [zone, _value] => Some(zone.first()),
                _ => None,
            }
        } else {
            None
        };
        match named {
            Some(first) if is_school_zone(first) => zone = Zone::School,
            Some(_) => return Zone::Other,
            None => {}
        }
    }
    zone
}

/// The index of the `)` that closes the `(` at `open`, if the text has one.
fn closing(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// The index of the `(` that the `)` at `close` closes, if the text has one.
fn opening(tokens: &[Token], close: usize) -> Option<usize> {
    let mut depth = 0usize;
    for index in (0..=close).rev() {
        match tokens[index].kind {
            Kind::Close => depth += 1,
            Kind::Open => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// The tokens between the `(` at `open` and its `)`, or to the end of the
/// text when the literal closes no parenthesis.
fn arguments(tokens: &[Token], open: usize) -> &[Token] {
    &tokens[open + 1..closing(tokens, open).unwrap_or(tokens.len())]
}

/// The index of the last token of the call whose `(` is at `open`: its `)`,
/// or the last token of the text when the literal closes no parenthesis.
fn call_end(tokens: &[Token], open: usize) -> usize {
    closing(tokens, open).unwrap_or(tokens.len() - 1)
}

/// The indexes of the tokens of `tokens` that are outside every parenthesis
/// they open.
fn top_level(tokens: &[Token]) -> Vec<usize> {
    let mut depth = 0usize;
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth = depth.saturating_sub(1),
            _ if depth == 0 => found.push(index),
            _ => {}
        }
    }
    found
}

/// The arguments of a call, given the tokens between its parentheses: they
/// are split at the commas outside every inner parenthesis. No tokens are
/// no arguments.
fn split_arguments(inner: &[Token]) -> Vec<&[Token]> {
    if inner.is_empty() {
        return Vec::new();
    }
    let mut arguments = Vec::new();
    let mut start = 0;
    for at in top_level(inner) {
        if inner[at].kind == Kind::Comma {
            arguments.push(&inner[start..at]);
            start = at + 1;
        }
    }
    arguments.push(&inner[start..]);
    arguments
}

/// The first token of the value that the cast at `cast` converts, and the
/// zones that the value names when it is in parentheses (a bare value names
/// none that applies: `::` binds tighter than `AT TIME ZONE`). Other casts
/// before it (`x::timestamp::date`, `x::pg_catalog.timestamp::date`) are
/// passed over, so their value counts.
fn cast_operand(tokens: &[Token], cast: usize) -> (usize, Zone) {
    let Some(mut last) = cast.checked_sub(1) else {
        return (cast, Zone::Missing);
    };
    loop {
        if last >= 2 && is_any_name(&tokens[last]) && tokens[last - 1].kind == Kind::Cast {
            last -= 2;
        } else if last >= 4
            && is_any_name(&tokens[last])
            && tokens[last - 1].kind == Kind::Dot
            && is_any_name(&tokens[last - 2])
            && tokens[last - 3].kind == Kind::Cast
        {
            last -= 4;
        } else {
            break;
        }
    }
    if tokens[last].kind != Kind::Close {
        return (last, Zone::Missing);
    }
    match opening(tokens, last) {
        Some(open) => {
            // A function call such as `now()` starts at its name, which is
            // written right before the parenthesis (`SELECT (` is no call).
            // The name counts for the zone: `timezone(zone, value)`.
            let call = open > 0
                && matches!(tokens[open - 1].kind, Kind::Word(_))
                && tokens[open - 1].end == tokens[open].start;
            let first = if call { open - 1 } else { open };
            // Only the value's own tokens count, never a zone named earlier
            // in the same text.
            (first, zone_of(&tokens[first..=last]))
        }
        None => (last, Zone::Missing),
    }
}

/// The report of one place that takes a day without the school's zone, or
/// `None` when it takes the day in that zone.
///
/// - `zone`: the zones that the SQL names.
/// - `sql`: the SQL, as the report quotes it.
/// - `action`: what the SQL does, such as "casts to `date`".
/// - `example`: the same SQL in the school's zone, as the fix to copy.
fn report(zone: Zone, sql: &str, action: &str, example: &str) -> Option<String> {
    match zone {
        Zone::School => None,
        Zone::Missing => Some(format!(
            "`{sql}` {action} in the session time zone, UTC: name the zone, as in {example}"
        )),
        Zone::Other => Some(format!(
            "`{sql}` {action} in a zone other than '{ZONE_NAME}', the school's zone (ADR 0010 \
             point 4): name '{ZONE_NAME}' or bind the zone as a parameter, as in {example}"
        )),
    }
}

/// The fix of a cast to `date`.
const CAST_EXAMPLE: &str = "`(<value> AT TIME ZONE 'Europe/Budapest')::date`";

/// Every place in `text` where SQL takes a day from an instant without
/// naming the school's zone, as (character index, finding) pairs.
fn sql_findings(text: &[char]) -> Vec<(usize, String)> {
    let tokens = sql_tokens(text);
    let snippet = |first: usize, last: usize| -> String {
        let words: Vec<String> = text[tokens[first].start..tokens[last].end]
            .iter()
            .collect::<String>()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let quoted = words.join(" ");
        if quoted.chars().count() > SNIPPET_LENGTH {
            format!(
                "{}...",
                quoted.chars().take(SNIPPET_LENGTH).collect::<String>()
            )
        } else {
            quoted
        }
    };
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let next = tokens.get(index + 1);
        let opens = next.is_some_and(|next| next.kind == Kind::Open);
        let finding = match &token.kind {
            Kind::Cast => date_type_end(&tokens, index + 1).and_then(|end| {
                let (first, zone) = cast_operand(&tokens, index);
                report(zone, &snippet(first, end), "casts to `date`", CAST_EXAMPLE)
                    .map(|finding| (first, finding))
            }),
            // `date(x)`, the function form of the cast; `date (` is prose.
            Kind::Word(_) | Kind::Quoted(_)
                if is_name(Some(token), "date")
                    && next
                        .is_some_and(|next| next.kind == Kind::Open && next.start == token.end) =>
            {
                let zone = zone_of(arguments(&tokens, index + 1));
                let sql = snippet(index, call_end(&tokens, index + 1));
                report(zone, &sql, "casts to `date`", CAST_EXAMPLE).map(|finding| (index, finding))
            }
            Kind::Word(word) if word == "cast" && opens => {
                let inner = arguments(&tokens, index + 1);
                let as_date = top_level(inner)
                    .into_iter()
                    .rev()
                    .find(|&at| is_word(inner.get(at), "as"))
                    .filter(|&at| date_type_end(inner, at + 1).is_some());
                as_date.and_then(|at| {
                    let sql = snippet(index, call_end(&tokens, index + 1));
                    report(
                        zone_of(&inner[..at]),
                        &sql,
                        "casts to `date`",
                        "`CAST(<value> AT TIME ZONE 'Europe/Budapest' AS date)`",
                    )
                    .map(|finding| (index, finding))
                })
            }
            Kind::Word(word) if word == "date_trunc" && opens => {
                let inner = arguments(&tokens, index + 1);
                let named = zone_of(inner);
                let zone = match split_arguments(inner).as_slice() {
                    // `date_trunc(field, source, zone)`: the third argument
                    // is the zone, and the source must name no other.
                    [_, _, zone, ..] if is_school_zone(zone.first()) && named != Zone::Other => {
                        Zone::School
                    }
                    [_, _, _, ..] => Zone::Other,
                    _ => named,
                };
                let sql = snippet(index, call_end(&tokens, index + 1));
                report(
                    zone,
                    &sql,
                    "truncates",
                    "`date_trunc('month', <value>, 'Europe/Budapest')`",
                )
                .map(|finding| (index, finding))
            }
            Kind::Word(word) if word == "current_date" => Some((
                index,
                "`current_date` is today in the session time zone, UTC: take today from the app \
                 clock, `budapest::today(now)` (ADR 0010 point 7), and bind its midnight"
                    .to_owned(),
            )),
            _ => None,
        };
        found.extend(finding.map(|(first, finding)| (tokens[first].start, finding)));
    }
    found
}

/// The findings in one file, as `<path>:<line>: <finding>` report lines.
///
/// - `relative`: the path below the repository root, for the report.
/// - `source`: the file's text; a `.sql` file is SQL as a whole, any other
///   file is Rust whose string literals are SQL.
fn file_report(relative: &Path, source: &str) -> Vec<String> {
    let texts = if relative
        .extension()
        .is_some_and(|extension| extension == "sql")
    {
        vec![sql_file(source)]
    } else {
        string_literals(source)
    };
    texts
        .iter()
        .flat_map(|text| {
            sql_findings(&text.chars).into_iter().map(|(at, finding)| {
                format!("{}:{}: {finding}", relative.display(), text.lines[at])
            })
        })
        .collect()
}

/// Every Rust and SQL file below `dir`, skipping build output.
fn source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())) {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                source_files(&path, files);
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "rs" || extension == "sql")
        {
            files.push(path);
        }
    }
}

#[test]
fn sql_names_the_zone_when_it_takes_a_day_from_an_instant() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut report = Vec::new();
    for directory in CHECKED_DIRECTORIES {
        let mut files = Vec::new();
        source_files(&root.join(directory), &mut files);
        assert!(!files.is_empty(), "{directory}/ has no Rust or SQL files");
        for file in files {
            let relative = file.strip_prefix(root).expect("below the repository root");
            let source = fs::read_to_string(&file).expect("read source");
            report.extend(file_report(relative, &source));
        }
    }
    assert!(
        report.is_empty(),
        "SQL that takes the day, month or year of a timestamptz names the school's zone, \
         {ZONE_NAME} (ADR 0010 points 4 and 10; tests/architecture/sql_dates.rs says what \
         counts):\n{}",
        report.join("\n")
    );
}

// The lint itself is tested on small samples, so a bug in it cannot make the
// real check pass silently.

/// The findings in `sql`, without their positions.
fn findings(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    sql_findings(&chars)
        .into_iter()
        .map(|(_, finding)| finding)
        .collect()
}

/// The quoted SQL of each finding in `sql`.
fn quoted(sql: &str) -> Vec<String> {
    findings(sql)
        .into_iter()
        .map(|finding| finding.split('`').nth(1).unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn sql_lint_flags_each_underived_day() {
    for (sql, expected) in [
        ("SELECT created_at::date FROM posts", "created_at::date"),
        ("SELECT now()::date", "now()::date"),
        ("SELECT NOW() :: DATE", "NOW() :: DATE"),
        ("WHERE p.date::date = $1", "date::date"),
        ("WHERE day = $1::date", "$1::date"),
        ("SELECT x::\"date\"", "x::\"date\""),
        ("SELECT x::date[]", "x::date"),
        ("SELECT x::timestamptz::date", "x::timestamptz::date"),
        ("SELECT (x::timestamp)::date", "(x::timestamp)::date"),
        ("SELECT (x AT LOCAL)::date", "(x AT LOCAL)::date"),
        // `::` binds tighter: this casts the zone's name, not the instant.
        (
            "SELECT x AT TIME ZONE 'Europe/Budapest'::date",
            "'Europe/Budapest'::date",
        ),
        ("SELECT '{col}'::date", "'{col}'::date"),
        ("WHERE {column}::date = $1", "}::date"),
        ("SELECT CAST(starts_at AS date)", "CAST(starts_at AS date)"),
        (
            "SELECT cast ( starts_at as DATE )",
            "cast ( starts_at as DATE )",
        ),
        (
            "SELECT date_trunc('month', date)",
            "date_trunc('month', date)",
        ),
        (
            "SELECT DATE_TRUNC ( 'year' , x )",
            "DATE_TRUNC ( 'year' , x )",
        ),
        (
            "SELECT pg_catalog.date_trunc('day', x)",
            "date_trunc('day', x)",
        ),
        (
            "GROUP BY date_trunc('month', date",
            "date_trunc('month', date",
        ),
        (
            "CREATE FUNCTION f() RETURNS date AS $$ BEGIN RETURN NEW.d::date; END $$",
            "d::date",
        ),
        (
            "CREATE FUNCTION f() AS $body$ SELECT x::date $body$",
            "x::date",
        ),
    ] {
        assert_eq!(quoted(sql), [expected], "{sql}");
    }
    assert_eq!(quoted("WHERE day = CURRENT_DATE"), ["current_date"]);
    assert_eq!(
        quoted("SELECT current_date, x::date"),
        ["current_date", "x::date"]
    );
    let long = format!("SELECT ({} )::date", "a ".repeat(100));
    let quote = &quoted(&long)[0];
    assert!(quote.ends_with("...") && quote.len() < 70, "{quote}");
}

#[test]
fn sql_lint_flags_other_spellings_and_each_value_on_its_own() {
    for (sql, expected) in [
        // The function form of the cast, which is also the legacy MySQL
        // idiom, and the schema-qualified type.
        ("SELECT date(created_at) FROM posts", "date(created_at)"),
        ("SELECT DATE(now())", "DATE(now())"),
        ("SELECT pg_catalog.date(created_at)", "date(created_at)"),
        ("SELECT \"date\"(x)", "\"date\"(x)"),
        ("GROUP BY date(created_at", "date(created_at"),
        ("SELECT x::pg_catalog.date", "x::pg_catalog.date"),
        (
            "SELECT x :: \"pg_catalog\" . \"date\"",
            "x :: \"pg_catalog\" . \"date\"",
        ),
        (
            "SELECT CAST(x AS pg_catalog.date)",
            "CAST(x AS pg_catalog.date)",
        ),
        // `date_trunc` in the zone gives that zone's midnight as an instant,
        // and its `::date` is then the UTC day, the day before.
        (
            "SELECT date_trunc('day', x, 'Europe/Budapest')::date",
            "date_trunc('day', x, 'Europe/Budapest')::date",
        ),
        // A zone named earlier in the same text does not cover a later value.
        (
            "SELECT (starts_at AT TIME ZONE 'Europe/Budapest')::date, (created_at)::date",
            "(created_at)::date",
        ),
        (
            "SELECT CAST(a AT TIME ZONE 'Europe/Budapest' AS date), CAST(b AS date)",
            "CAST(b AS date)",
        ),
        (
            "SELECT date_trunc('day', a, 'Europe/Budapest'), date_trunc('day', b)",
            "date_trunc('day', b)",
        ),
        (
            "SELECT date(a AT TIME ZONE 'Europe/Budapest'), date(b)",
            "date(b)",
        ),
        (
            "SELECT timezone('Europe/Budapest', a)::date, b::date",
            "b::date",
        ),
    ] {
        assert_eq!(quoted(sql), [expected], "{sql}");
    }
}

#[test]
fn sql_lint_flags_zones_other_than_the_schools() {
    // Each of them takes another day than the Budapest one.
    for (sql, expected) in [
        (
            "SELECT (x AT TIME ZONE 'UTC')::date",
            "(x AT TIME ZONE 'UTC')::date",
        ),
        (
            "SELECT (x AT TIME ZONE 'Europe/Vienna')::date",
            "(x AT TIME ZONE 'Europe/Vienna')::date",
        ),
        (
            "SELECT (x AT TIME ZONE INTERVAL '+02:00')::date",
            "(x AT TIME ZONE INTERVAL '+02:00')::date",
        ),
        (
            "SELECT (x AT TIME ZONE zone)::date",
            "(x AT TIME ZONE zone)::date",
        ),
        (
            "SELECT (x AT TIME ZONE $1 AT TIME ZONE 'UTC')::date",
            "(x AT TIME ZONE $1 AT TIME ZONE 'UTC')::date",
        ),
        (
            "SELECT timezone('UTC', x)::date",
            "timezone('UTC', x)::date",
        ),
        (
            "SELECT CAST(x AT TIME ZONE 'UTC' AS date)",
            "CAST(x AT TIME ZONE 'UTC' AS date)",
        ),
        (
            "SELECT date(x AT TIME ZONE 'UTC')",
            "date(x AT TIME ZONE 'UTC')",
        ),
        (
            "SELECT date_trunc('day', x, 'UTC')",
            "date_trunc('day', x, 'UTC')",
        ),
        (
            "SELECT date_trunc('day', x, zone)",
            "date_trunc('day', x, zone)",
        ),
        (
            "SELECT date_trunc('month', x AT TIME ZONE 'UTC')",
            "date_trunc('month', x AT TIME ZONE 'UTC')",
        ),
        (
            "SELECT date_trunc('day', timezone('UTC', x))",
            "date_trunc('day', timezone('UTC', x))",
        ),
        (
            "SELECT date_trunc('day', x AT TIME ZONE 'UTC', 'Europe/Budapest')",
            "date_trunc('day', x AT TIME ZONE 'UTC', 'Europe/Budapest')",
        ),
    ] {
        assert_eq!(quoted(sql), [expected], "{sql}");
    }
}

#[test]
fn sql_lint_says_whether_the_zone_is_missing_or_another() {
    assert_eq!(
        findings("SELECT created_at::date"),
        [
            "`created_at::date` casts to `date` in the session time zone, UTC: name the zone, as \
             in `(<value> AT TIME ZONE 'Europe/Budapest')::date`"
        ]
    );
    assert_eq!(
        findings("SELECT date_trunc('day', x, 'UTC')"),
        [
            "`date_trunc('day', x, 'UTC')` truncates in a zone other than 'Europe/Budapest', the \
             school's zone (ADR 0010 point 4): name 'Europe/Budapest' or bind the zone as a \
             parameter, as in `date_trunc('month', <value>, 'Europe/Budapest')`"
        ]
    );
    assert_eq!(
        findings("SELECT CAST(x AS date)"),
        [
            "`CAST(x AS date)` casts to `date` in the session time zone, UTC: name the zone, as in \
             `CAST(<value> AT TIME ZONE 'Europe/Budapest' AS date)`"
        ]
    );
}

#[test]
fn sql_lint_accepts_days_taken_in_the_schools_zone() {
    for sql in [
        "SELECT (starts_at AT TIME ZONE 'Europe/Budapest')::date",
        "SELECT (starts_at at time zone 'Europe/Budapest') :: date",
        "SELECT (x AT TIME ZONE $1)::date",
        // PostgreSQL reads zone names in any case.
        "SELECT (x AT TIME ZONE 'europe/budapest')::date, (y AT TIME ZONE E'EUROPE/BUDAPEST')::date",
        // `timezone(zone, value)` is the function form of `AT TIME ZONE`.
        "SELECT timezone('Europe/Budapest', x)::date",
        "SELECT timezone($1, x)::date",
        "SELECT date_trunc('day', timezone('Europe/Budapest', x))",
        "SELECT CAST(timezone('Europe/Budapest', x) AS pg_catalog.date)",
        // The function form of the cast and the schema-qualified type.
        "SELECT date(x AT TIME ZONE 'Europe/Budapest'), date(timezone('Europe/Budapest', x))",
        "SELECT pg_catalog.date(x AT TIME ZONE $1)",
        "SELECT (x AT TIME ZONE 'Europe/Budapest')::pg_catalog.date",
        "SELECT (x AT TIME ZONE 'Europe/Budapest')::pg_catalog.timestamp::date",
        // Of a zone, only its first token is read.
        "SELECT date_trunc('day', x, $1::text)",
        // Names that only contain `date`, and prose with a space before the
        // parenthesis.
        "SELECT to_date(x, 'YYYY-MM-DD'), update(x), mandate(x), x::pg_catalog.daterange",
        "a calendar date (YYYY-MM-DD) is required",
        // ADR 0018 §1.3 and §4.1.
        "CHECK ((date AT TIME ZONE 'Europe/Budapest')::time = time '00:00')",
        "CREATE UNIQUE INDEX canteen_days_live_date ON canteen_days \
         (((date AT TIME ZONE 'Europe/Budapest')::date)) WHERE deleted_at IS NULL",
        "SELECT (x AT TIME ZONE 'Europe/Budapest')::timestamp::date",
        "SELECT date_trunc('day', x AT TIME ZONE 'Europe/Budapest')::date",
        "SELECT CAST(starts_at AT TIME ZONE 'Europe/Budapest' AS date)",
        "SELECT CAST(x AS text), CAST(x AS daterange)",
        "SELECT date_trunc('month', starts_at AT TIME ZONE 'Europe/Budapest')",
        "SELECT date_trunc('month', starts_at, 'Europe/Budapest')",
        "SELECT date_trunc('day', $1, $2)",
        "SELECT DATE '2026-09-01', x::daterange, x::timestamptz, x::text, date FROM posts",
        "SELECT 'not a cast: x::date, current_date', E'it\\'s x::date'",
        "SELECT 1 -- x::date in a comment\nFROM t /* now()::date /* nested */ date_trunc( */",
        "SELECT \"current_date\", \"date_trunc\"(x) FROM t",
        "SELECT x::date_range, today::datetime",
        "",
    ] {
        assert_eq!(findings(sql), Vec::<String>::new(), "{sql}");
    }
}

#[test]
fn sql_lint_reads_rust_string_literals_and_sql_files() {
    let source = r###"
        //! A doc comment about `created_at::date` is prose.
        /* so is now()::date in a block comment */
        const A: &str = "SELECT (date AT TIME ZONE 'Europe/Budapest')::date";
        const B: &str = "SELECT created_at::date";
        const C: &str = r#"SELECT "x"::date"#;
        const D: &[u8] = b"SELECT current_date";
        const E: &str = "SELECT (date AT TIME ZONE 'Europe/Budapest')\
                         ::date, \"y\"::date";
        const F: &str = "SELECT date_trunc('month',\n x)";
        fn g<'a>(s: &'a str) -> char { let _ = 'x'; '"' }
        const H: &str = br##"x::date"##;
    "###;
    assert_eq!(
        file_report(Path::new("src/models/sample.rs"), source)
            .iter()
            .map(|line| line.split_once(": `").map_or(line.as_str(), |(at, _)| at))
            .collect::<Vec<_>>(),
        [
            "src/models/sample.rs:5",
            "src/models/sample.rs:6",
            "src/models/sample.rs:7",
            "src/models/sample.rs:9",
            "src/models/sample.rs:10",
            "src/models/sample.rs:12",
        ]
    );
    let sql = "-- current_date in a comment\nSELECT 1;\nSELECT x::date;\n";
    let report = file_report(Path::new("migration/sql/sample.sql"), sql);
    assert_eq!(report.len(), 1);
    assert!(
        report[0].starts_with("migration/sql/sample.sql:3: `x::date` casts"),
        "{report:?}"
    );
    assert_eq!(
        file_report(Path::new("src/x.rs"), "const S: &str = \"x::date\";").len(),
        1
    );
}

proptest! {
    // The lint reads any text without panicking, also unbalanced quotes,
    // parentheses and escapes.
    #[test]
    fn sql_lint_reads_any_text(text in prop_oneof![
        any::<String>(),
        "[()':,.\"$\\\\ a-z_*/-]{0,40}",
        "(date_trunc|cast|current_date|timezone|pg_catalog|at|time|zone|::|date|\
          [()',.\" $1]|--|/\\*|\\*/|\n){0,30}",
    ]) {
        let _ = findings(&text);
        let _ = file_report(Path::new("src/x.rs"), &text);
        let _ = file_report(Path::new("migration/x.sql"), &text);
    }

    // Whatever the value, its spelling and the spaces around `::`, a bare
    // cast is reported and the same cast of the value in the school's zone
    // is not; so is each other zone, in each place that a zone is named.
    #[test]
    fn sql_lint_reports_exactly_the_casts_without_a_zone(
        value in "[a-z_][a-z0-9_]{0,10}(\\.[a-z_][a-z0-9_]{0,10})?",
        before in " {0,2}",
        after in " {0,2}",
        upper in any::<bool>(),
        zone in "[A-Za-z][A-Za-z_/+-]{0,20}",
    ) {
        let case = |word: &str| if upper { word.to_uppercase() } else { word.to_owned() };
        let keywords = ["at", "time", "zone", "current_date", "cast", "date_trunc"];
        prop_assume!(!keywords.contains(&value.as_str()));
        let bare = format!("SELECT {value}{before}::{after}{}", case("date"));
        prop_assert_eq!(findings(&bare).len(), 1, "{}", bare);
        let zoned = format!(
            "SELECT ({value} {} 'Europe/Budapest'){before}::{after}{}",
            case("at time zone"),
            case("date")
        );
        prop_assert_eq!(findings(&zoned), Vec::<String>::new(), "{}", zoned);
        let converted = format!(
            "SELECT {}('Europe/Budapest', {value}){before}::{after}{}",
            case("timezone"),
            case("date")
        );
        prop_assert_eq!(findings(&converted), Vec::<String>::new(), "{}", converted);
        let truncated = format!("SELECT {}('month', {value})", case("date_trunc"));
        prop_assert_eq!(findings(&truncated).len(), 1, "{}", truncated);
        let called = format!("SELECT {}({value})", case("date"));
        prop_assert_eq!(findings(&called).len(), 1, "{}", called);
        let called_zoned = format!(
            "SELECT {}({value} {} 'Europe/Budapest')",
            case("date"),
            case("at time zone")
        );
        prop_assert_eq!(findings(&called_zoned), Vec::<String>::new(), "{}", called_zoned);
        if !zone.eq_ignore_ascii_case(ZONE_NAME) {
            for other in [
                format!("SELECT ({value} AT TIME ZONE '{zone}'){before}::{after}date"),
                format!("SELECT timezone('{zone}', {value})::date"),
                format!("SELECT date({value} AT TIME ZONE '{zone}')"),
                format!("SELECT CAST({value} AT TIME ZONE '{zone}' AS date)"),
                format!("SELECT date_trunc('month', {value}, '{zone}')"),
            ] {
                let found = findings(&other);
                prop_assert_eq!(found.len(), 1, "{}", other);
                prop_assert!(found[0].contains("a zone other than"), "{}", found[0]);
            }
        }
    }
}
