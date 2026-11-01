//! The architecture test of ADR 0008 ("Enforcement").
//!
//! `src/domain/` must stay pure: no I/O, no framework, no clock, no
//! randomness, no `async`, and no dependency on any other module of the
//! crate. The compiler cannot see these rules, so this test scans the source
//! text. It strips comments, string and character literals first, so prose
//! about `tokio` in a doc comment or an error message is not a violation, and
//! then checks the remaining tokens:
//!
//! - banned crates and paths: `loco_rs`, `sea_orm`, `sqlx`, `tokio`, `rand`,
//!   `std::fs`, `std::net`, `std::env`, `std::process` (also inside
//!   `use std::{...}` groups), and `AppContext`;
//! - clock reads: `Utc::now`, `Local::now`, `SystemTime::now`,
//!   `Instant::now`;
//! - the `async` keyword;
//! - `crate::` paths outside `crate::domain`, and `super::` paths that climb
//!   out of `src/domain/`;
//! - every domain file that defines a function has a `#[cfg(test)]` block.
//!
//! It also fails if `src/` holds a directory that rule 5 bans (`services/`,
//! `repositories/`, `utils/`, `helpers/`, `common/`, `shared/`, `lib/`).
//!
//! A third check covers all of `src/`, unit tests included: "now" comes from
//! the app clock (ADR 0010 point 7, public-api.md §16.2). It fails on the
//! reads of the system time and the spellings of chrono's `Local` (which
//! reads the process `TZ`, ADR 0010 point 4) that the constants below list,
//! except inside `impl Clock for SystemClock` in `src/app.rs`, and on
//! `set_clock` outside that file. `Instant::now` stays allowed outside
//! `src/domain/`: it measures a duration and never becomes an instant.
//!
//! A fourth check covers all of `src/` too: every task runs under the task
//! lock, so a manual run and a `CronJob` run never overlap (ADR 0016 §4;
//! research: DECISIONS R15, D95). The body of every `impl Task for <type>`
//! must call `with_task_lock` with `&self.task().name`, the name the task
//! registers, as its second argument. So that a `Task` impl spelled in a way
//! the scan does not recognise cannot pass unseen, every type that
//! `register_tasks` in `src/app.rs` registers must have such an impl at the
//! same crate path (the impl's file gives its module), every statement of
//! `register_tasks` must be a `tasks.register(...)` call, so that no helper
//! or macro registers tasks out of sight, and the check fails if
//! `register_tasks` is gone. It checks that the call is there, not that all
//! of the task's database work runs inside it: review checks that.
//!
//! A fifth check, in [`sql_dates`], reads the SQL in `src/` and `migration/`
//! and fails where it takes a day from an instant without naming the
//! school's zone, Europe/Budapest (ADR 0010 points 4 and 10). It reuses this
//! file's scanners for comments and literals.
//!
//! A sixth check is ADR 0011's ("What must be true in code and tests"): a
//! public-API module never reads rows itself, only through live-row
//! finders, so a trashed row cannot reach `/api/v1`. The public modules are
//! everything under `src/controllers/v1/` and `src/dtos/v1/`. They may not
//! name Sea-ORM's entities, columns, query traits or finders
//! ([`DIRECT_READS`], and `::find(`), nor raw SQL. A model function can read
//! rows only through the connection or transaction it takes as an argument
//! (ADR 0008 rules 2 and 4), so the check sorts the functions of
//! `src/models/` by their signatures ([`CONNECTION_TYPES`]). Every function
//! a public module calls through `crate::models` (by path, by a name a
//! `use` of it brought in, with or without a turbofish) must be a live-row
//! finder, a model function whose name has the word `live`, such as
//! `users::Model::find_live_by_email`, or a model function that takes no
//! connection and so reads no rows, such as `serve_content` (ADR 0013) and
//! the read counter (public-api.md §15). A method call (`post.labels(db)`)
//! must not use the name of a model function that takes a connection and
//! is not a live-row finder; the scan cannot see the receiver's type, so it
//! goes by the name. A glob import from `crate::models` is refused, because
//! it would hide the names. On the models' side, every function whose name
//! has the word `live` must filter the trash in its body:
//! `DeletedAt.is_null()` in code, `deleted_at IS NULL` in its SQL, or a
//! call of another live-row finder. What a finder that reads several tables
//! filters on each is beyond a text check; the soft-deleted twins of the
//! public fixtures (`tests/support/fixtures.rs`) catch that.
//!
//! The scan is a text check, so a determined bypass is possible (ADR 0008,
//! Consequences); it exists to catch honest mistakes early.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

// This file is the root of its test crate, so a plain `mod` would look in
// `tests/`, where Cargo would also build a `.rs` file as a test crate of its
// own; `#[path]` keeps the module beside this file instead.
#[path = "architecture/sql_dates.rs"]
mod sql_dates;

/// Identifiers that must not appear as tokens in `src/domain/`.
const BANNED_IDENTIFIERS: &[&str] = &["loco_rs", "sea_orm", "sqlx", "tokio", "rand", "AppContext"];

/// Second segments of `std::<x>` that `src/domain/` must not use.
const BANNED_STD_MODULES: &[&str] = &["fs", "net", "env", "process"];

/// Types whose `::now` reads the clock.
const CLOCK_TYPES: &[&str] = &["Utc", "Local", "SystemTime", "Instant"];

/// Associated functions that read a clock when a type calls them without
/// arguments. Matching any type name catches aliases such as Sea-ORM's
/// `ChronoUtc` and `TimeDateTimeWithTimeZone`; requiring no arguments lets a
/// pure function such as `Week::today(now)` pass.
const CLOCK_FUNCTIONS: &[&str] = &["now", "today", "now_utc", "now_local"];

/// Names that read the system time or stand for chrono's `Local` wherever
/// they appear: jsonwebtoken's clock, `UNIX_EPOCH` (whose `elapsed` reads the
/// clock), and Sea-ORM's two aliases of `DateTime<Local>`.
const CLOCK_NAMES: &[&str] = &[
    "get_current_timestamp",
    "UNIX_EPOCH",
    "DateTimeLocal",
    "ChronoDateTimeLocal",
];

/// The file, relative to `src/`, that holds the system clock, and the impl
/// block whose body is the one place allowed to read the system time.
const SYSTEM_CLOCK_FILE: &str = "app.rs";
const SYSTEM_CLOCK_IMPL: &[&str] = &["impl", "Clock", "for", "SystemClock"];

/// The second argument every task passes to `with_task_lock`: the name in its
/// own `TaskInfo`, so the lock cannot drift from the name the `CronJob` runs
/// (recipe 6 of the backend3 skill).
const REGISTERED_NAME: &str = "&self.task().name";

/// The file, relative to `src/`, whose `register_tasks` registers every task.
const TASK_REGISTRY_FILE: &str = "app.rs";

/// Directory names rule 5 bans directly under `src/`.
const BANNED_DIRECTORIES: &[&str] = &[
    "services",
    "repositories",
    "utils",
    "helpers",
    "common",
    "shared",
    "lib",
];

/// One token of the stripped source: an identifier, `::`, or another single
/// punctuation character.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    PathSep,
    Punct(char),
}

/// Replaces comments, string literals and character literals with spaces,
/// keeping line structure. Handles nested block comments, raw strings with
/// any number of `#`, byte strings, escapes, and lifetimes (which look like
/// character literals but are not).
fn strip(source: &str) -> String {
    blank(source, false)
}

/// Like [`strip`], but keeps string and character literals, so that the SQL
/// text of a query can be read. Both keep every character's position.
fn strip_comments(source: &str) -> String {
    blank(source, true)
}

/// Replaces comments, and literals unless `keep_literals`, with spaces.
fn blank(source: &str, keep_literals: bool) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let (end, literal) = match (chars[i], chars.get(i + 1)) {
            ('/', Some('/')) => (Some(line_comment_end(&chars, i)), false),
            ('/', Some('*')) => (Some(block_comment_end(&chars, i)), false),
            ('"', _) => (Some(string_end(&chars, i)), true),
            ('\'', _) => (char_literal_end(&chars, i), true),
            ('r' | 'b', _) => (raw_string_end(&chars, i), true),
            _ => (None, false),
        };
        if let Some(end) = end {
            for &skipped in &chars[i..end] {
                out.push(if skipped == '\n' || (literal && keep_literals) {
                    skipped
                } else {
                    ' '
                });
            }
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// The end (exclusive) of a `//` comment starting at `start`.
fn line_comment_end(chars: &[char], start: usize) -> usize {
    (start..chars.len())
        .find(|&j| chars[j] == '\n')
        .unwrap_or(chars.len())
}

/// The end (exclusive) of a possibly nested `/* */` comment at `start`.
fn block_comment_end(chars: &[char], start: usize) -> usize {
    let mut depth = 0usize;
    let mut j = start;
    while j < chars.len() {
        if chars[j] == '/' && chars.get(j + 1) == Some(&'*') {
            depth += 1;
            j += 2;
        } else if chars[j] == '*' && chars.get(j + 1) == Some(&'/') {
            depth -= 1;
            j += 2;
            if depth == 0 {
                return j;
            }
        } else {
            j += 1;
        }
    }
    chars.len()
}

/// The end (exclusive) of a `"..."` string at `start`, honouring escapes.
fn string_end(chars: &[char], start: usize) -> usize {
    let mut j = start + 1;
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 2,
            '"' => return j + 1,
            _ => j += 1,
        }
    }
    chars.len()
}
/// The end (exclusive) of a character literal at `start`, or `None` for a
/// lifetime such as `'a`.
fn char_literal_end(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start + 1) == Some(&'\\') {
        (start + 2..chars.len().min(start + 12))
            .find(|&j| chars[j] == '\'')
            .map(|j| j + 1)
    } else if chars.get(start + 2) == Some(&'\'') {
        Some(start + 3)
    } else {
        None
    }
}

/// The end (exclusive) of a raw string (`r"..."`, `r#"..."#`, `br"..."`) at
/// `start`, or `None` when `start` begins an ordinary identifier.
fn raw_string_end(chars: &[char], start: usize) -> Option<usize> {
    let preceded_by_ident =
        start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_');
    if preceded_by_ident {
        return None;
    }
    let mut j = match (chars[start], chars.get(start + 1)) {
        ('r', _) => start + 1,
        ('b', Some('r')) => start + 2,
        _ => return None,
    };
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    j += 1;
    while j < chars.len() {
        if chars[j] == '"' && (0..hashes).all(|k| chars.get(j + 1 + k) == Some(&'#')) {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(chars.len())
}

/// Splits stripped source into tokens.
fn tokenize(stripped: &str) -> Vec<Token> {
    let chars: Vec<char> = stripped.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
        } else if c == ':' && chars.get(i + 1) == Some(&':') {
            tokens.push(Token::PathSep);
            i += 2;
        } else if c.is_whitespace() || c.is_ascii_digit() {
            i += 1;
        } else {
            tokens.push(Token::Punct(c));
            i += 1;
        }
    }
    tokens
}

fn ident(token: Option<&Token>) -> Option<&str> {
    match token {
        Some(Token::Ident(name)) => Some(name),
        _ => None,
    }
}

/// The module depth of a file below `src/domain/`: `mod.rs` at the root is
/// 0, `foo.rs` and `foo/mod.rs` are 1, `foo/bar.rs` is 2. `super` may climb
/// at most this many levels.
fn domain_depth(relative: &Path) -> usize {
    let components = relative.components().count();
    let is_mod = relative.file_name().is_some_and(|name| name == "mod.rs");
    if is_mod { components - 1 } else { components }
}

/// Every rule violation in one domain file, as human-readable lines.
fn violations(relative: &Path, source: &str) -> Vec<String> {
    let tokens = tokenize(&strip(source));
    let mut found = Vec::new();
    let depth = domain_depth(relative);

    for (index, token) in tokens.iter().enumerate() {
        let Token::Ident(name) = token else { continue };
        let next = tokens.get(index + 1);
        let after = ident(tokens.get(index + 2));

        if BANNED_IDENTIFIERS.contains(&name.as_str()) {
            found.push(format!("uses `{name}`"));
        }
        if name == "async" {
            found.push("uses `async`".to_owned());
        }
        if name == "std" && next == Some(&Token::PathSep) {
            if let Some(module) = after.filter(|module| BANNED_STD_MODULES.contains(module)) {
                found.push(format!("uses `std::{module}`"));
            }
            if tokens.get(index + 2) == Some(&Token::Punct('{')) {
                let mut level = 0;
                for inner in &tokens[index + 2..] {
                    match inner {
                        Token::Punct('{') => level += 1,
                        Token::Punct('}') => {
                            level -= 1;
                            if level == 0 {
                                break;
                            }
                        }
                        Token::Ident(module)
                            if level == 1 && BANNED_STD_MODULES.contains(&module.as_str()) =>
                        {
                            found.push(format!("uses `std::{module}` in a `use` group"));
                        }
                        _ => {}
                    }
                }
            }
        }
        if CLOCK_TYPES.contains(&name.as_str())
            && next == Some(&Token::PathSep)
            && after == Some("now")
        {
            found.push(format!("reads the clock with `{name}::now`"));
        }
        if name == "crate" && next == Some(&Token::PathSep) && after != Some("domain") {
            found.push(format!(
                "names `crate::{}` outside `crate::domain`",
                after.unwrap_or("{...}")
            ));
        }
        if name == "super" && tokens.get(index.wrapping_sub(1)) != Some(&Token::PathSep) {
            let mut climbs = 1;
            let mut cursor = index + 1;
            while tokens.get(cursor) == Some(&Token::PathSep)
                && ident(tokens.get(cursor + 1)) == Some("super")
            {
                climbs += 1;
                cursor += 2;
            }
            if climbs > depth {
                found.push("climbs out of `src/domain/` with `super`".to_owned());
            }
        }
    }

    let defines_fn = tokens
        .iter()
        .any(|token| token == &Token::Ident("fn".to_owned()));
    let has_tests = source.contains("#[cfg(test)]");
    if defines_fn && !has_tests {
        found.push("defines functions but has no `#[cfg(test)]` block".to_owned());
    }
    found
}

/// Marks the tokens of every `use` statement that names chrono, so that
/// `Local` in `use chrono::{DateTime, offset::{Local}};` is seen as chrono's.
fn chrono_uses(tokens: &[Token]) -> Vec<bool> {
    let mut marked = vec![false; tokens.len()];
    for (start, token) in tokens.iter().enumerate() {
        if ident(Some(token)) != Some("use") {
            continue;
        }
        let end = tokens[start..]
            .iter()
            .position(|token| token == &Token::Punct(';'))
            .map_or(tokens.len(), |length| start + length);
        if tokens[start..end].contains(&Token::Ident("chrono".to_owned())) {
            marked[start..end].fill(true);
        }
    }
    marked
}

/// Whether the path that ends at `tokens[index]` passes through `chrono`,
/// as in `chrono::prelude::Local`.
fn through_chrono(tokens: &[Token], mut index: usize) -> bool {
    while index >= 2 && tokens[index - 1] == Token::PathSep {
        index -= 2;
        match ident(tokens.get(index)) {
            Some("chrono") => return true,
            Some(_) => {}
            None => return false,
        }
    }
    false
}

/// Every read of the system time and every use of chrono's `Local` in
/// `tokens`, as (token index, what) pairs.
///
/// A bare `Local` is chrono's only if a `use` of chrono brings it in, by name
/// or by a glob, and both are reported. So other `Local`s, such as
/// `Locality::Local` in `src/domain/database_target.rs`, pass.
fn clock_reads(tokens: &[Token]) -> Vec<(usize, String)> {
    let in_chrono_use = chrono_uses(tokens);
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token == &Token::Punct('*') && in_chrono_use[index] {
            found.push((
                index,
                "a glob import of chrono, which brings in `Local`".to_owned(),
            ));
        }
        let Token::Ident(name) = token else { continue };
        if name.starts_with(char::is_uppercase)
            && name != "Instant"
            && tokens.get(index + 1) == Some(&Token::PathSep)
            && let Some(function) = ident(tokens.get(index + 2))
            && CLOCK_FUNCTIONS.contains(&function)
            // A call with arguments is not a clock read; `Utc::now` passed
            // as a function, without the parentheses, is one.
            && !(tokens.get(index + 3) == Some(&Token::Punct('('))
                && tokens.get(index + 4) != Some(&Token::Punct(')')))
        {
            found.push((index, format!("`{name}::{function}`")));
        }
        if name == "Local" && (in_chrono_use[index] || through_chrono(tokens, index)) {
            found.push((index, "chrono's `Local`".to_owned()));
        }
        if CLOCK_NAMES.contains(&name.as_str()) {
            found.push((index, format!("`{name}`")));
        }
    }
    found
}

/// The first brace block at or after `start`, from its `{` to its matching
/// `}` (exclusive), or `None` when no block opens or it never closes.
fn block_from(tokens: &[Token], start: usize) -> Option<std::ops::Range<usize>> {
    let open = start
        + tokens[start..]
            .iter()
            .position(|token| token == &Token::Punct('{'))?;
    let mut level = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::Punct('{') => level += 1,
            Token::Punct('}') => {
                level = level.saturating_sub(1);
                if level == 0 {
                    return Some(open..index);
                }
            }
            _ => {}
        }
    }
    None
}

/// The token range of `impl Clock for SystemClock { ... }` in `tokens`, if
/// there is one, from `impl` to the closing brace.
fn system_clock_impl(tokens: &[Token]) -> Option<std::ops::Range<usize>> {
    let start = tokens.windows(SYSTEM_CLOCK_IMPL.len()).position(|window| {
        window
            .iter()
            .zip(SYSTEM_CLOCK_IMPL)
            .all(|(token, word)| ident(Some(token)) == Some(*word))
    })?;
    Some(start..block_from(tokens, start)?.end)
}

/// What one file of `src/` breaks of the rule that "now" comes from the app
/// clock, one report line per finding. `relative` is the path below `src/`:
/// only `app.rs` may read the system time, inside `impl Clock for
/// SystemClock`, and only it may call `set_clock`.
fn clock_report(relative: &Path, source: &str) -> Vec<String> {
    let tokens = tokenize(&strip(source));
    let is_system_clock_file = relative == Path::new(SYSTEM_CLOCK_FILE);
    let allowed = is_system_clock_file
        .then(|| system_clock_impl(&tokens))
        .flatten()
        .unwrap_or_default();
    let mut found: Vec<String> = clock_reads(&tokens)
        .into_iter()
        .filter(|(index, _)| !allowed.contains(index))
        .map(|(_, what)| format!("uses {what}"))
        .collect();
    if !is_system_clock_file && tokens.contains(&Token::Ident("set_clock".to_owned())) {
        found.push(
            "replaces the app clock with `set_clock`, which only `App::after_context` and \
             tests/support call"
                .to_owned(),
        );
    }
    found
        .into_iter()
        .map(|finding| format!("src/{}: {finding}", relative.display()))
        .collect()
}

/// The top-level arguments of the call whose opening parenthesis is
/// `tokens[0]`. Angle brackets and closure parameters are not tracked, so a
/// comma inside them splits an argument; only the first two arguments are
/// read, and a task passes plain expressions there.
fn call_arguments(tokens: &[Token]) -> Vec<&[Token]> {
    let mut arguments = Vec::new();
    let mut level = 0usize;
    let mut start = 1;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Punct('(' | '[' | '{') => level += 1,
            Token::Punct(')' | ']' | '}') => {
                level = level.saturating_sub(1);
                if level == 0 {
                    arguments.push(&tokens[start..index]);
                    break;
                }
            }
            Token::Punct(',') if level == 1 => {
                arguments.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    arguments
}

/// Whether `body` calls `with_task_lock`, by name or by path, with
/// [`REGISTERED_NAME`] as its second argument. A mention in a comment or a
/// string does not count, because both are stripped before tokenizing.
fn locks_under_registered_name(body: &[Token]) -> bool {
    let name = tokenize(REGISTERED_NAME);
    body.iter().enumerate().any(|(index, token)| {
        ident(Some(token)) == Some("with_task_lock")
            && body.get(index + 1) == Some(&Token::Punct('('))
            && call_arguments(&body[index + 1..]).get(1) == Some(&name.as_slice())
    })
}

/// The index of the bracket that closes the one at `tokens[open]`, with
/// `()`, `[]` and `{}` counted alike, or `None` when it never closes.
fn closing(tokens: &[Token], open: usize) -> Option<usize> {
    let mut level = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::Punct('(' | '[' | '{') => level += 1,
            Token::Punct(')' | ']' | '}') => {
                level = level.saturating_sub(1);
                if level == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// The segments of the path that starts at `tokens[start]`: `a::b::C` is
/// `["a", "b", "C"]`. Empty when no identifier starts there.
fn path_at(tokens: &[Token], start: usize) -> Vec<String> {
    let mut segments = Vec::new();
    let mut cursor = start;
    while let Some(segment) = ident(tokens.get(cursor)) {
        segments.push(segment.to_owned());
        if tokens.get(cursor + 1) != Some(&Token::PathSep) {
            break;
        }
        cursor += 2;
    }
    segments
}

/// The module path that `crate::` paths give a file below `src/`:
/// `tasks/probe.rs` and `tasks/probe/mod.rs` are both `tasks::probe`, and
/// `lib.rs` is the crate root.
fn module_path(relative: &Path) -> Vec<String> {
    let mut segments: Vec<String> = relative
        .with_extension("")
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if segments.last().is_some_and(|last| last == "mod") || segments == ["lib"] {
        segments.pop();
    }
    segments
}

/// The crate path of the type that an impl in `module` names by `path`, as
/// Rust resolves it: a `crate::` path from the root, `self::` and `super::`
/// from `module`, and any other path below `module`, where a type that the
/// module or one of its children defines lives. A type that a `use` brings
/// in from elsewhere resolves to the wrong path, so its registration is
/// reported, never passed: `cargo loco generate task` writes the impl beside
/// the type.
fn crate_path(module: &[String], path: &[String]) -> Vec<String> {
    if path.first().is_some_and(|first| first == "crate") {
        return path[1..].to_vec();
    }
    let mut resolved = module.to_vec();
    let mut rest = path;
    while let Some((first, tail)) = rest.split_first() {
        match first.as_str() {
            "self" => {}
            "super" => {
                resolved.pop();
            }
            _ => break,
        }
        rest = tail;
    }
    resolved.extend_from_slice(rest);
    resolved
}

/// `tokens` as source text again, for a report line: a space only between
/// two identifiers. Comments and literals were stripped before tokenizing,
/// so a string argument shows as nothing.
fn render(tokens: &[Token]) -> String {
    let mut text = String::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Ident(name) => {
                if index > 0 && ident(tokens.get(index - 1)).is_some() {
                    text.push(' ');
                }
                text.push_str(name);
            }
            Token::PathSep => text.push_str("::"),
            Token::Punct(c) => text.push(*c),
        }
    }
    text
}

/// A `Task` implementation found in a file of `src/`.
#[derive(Debug, PartialEq, Eq)]
struct TaskImpl {
    /// The implementing type's path as the impl writes it, without generic
    /// arguments: `["Probe"]` or `["crate", "tasks", "probe", "Probe"]`.
    /// Empty when the scan cannot read it.
    path: Vec<String>,
    /// Whether the impl's body passes [`locks_under_registered_name`].
    locked: bool,
}

/// Every `impl Task for <type> { ... }` in `tokens`, path-qualified
/// (`impl loco_rs::task::Task for`) or generic (`impl<T> Task for`).
///
/// The trait's `for` is the one that directly follows the name `Task`; the
/// `for` of a higher-ranked bound (`F: for<'a> Fn(&'a str)`) follows `:`,
/// `+` or `,` instead.
fn task_impls(tokens: &[Token]) -> Vec<TaskImpl> {
    let mut found = Vec::new();
    for (start, token) in tokens.iter().enumerate() {
        if ident(Some(token)) != Some("impl") {
            continue;
        }
        let Some(body) = block_from(tokens, start) else {
            continue;
        };
        let header = &tokens[start..body.start];
        let Some(trait_for) = header.windows(2).position(|pair| {
            ident(Some(&pair[0])) == Some("Task") && ident(Some(&pair[1])) == Some("for")
        }) else {
            continue;
        };
        found.push(TaskImpl {
            path: path_at(header, trait_for + 2),
            locked: locks_under_registered_name(&tokens[body]),
        });
    }
    found
}

/// One statement of `fn register_tasks`, as the registry check reads it.
#[derive(Debug, PartialEq, Eq)]
enum Registration {
    /// `<receiver>.register(<argument>)`, the form `cargo loco generate task`
    /// writes: the path of the argument up to its first capitalised segment,
    /// which is `tasks::foo::Foo` in the generated `tasks::foo::Foo` and also
    /// in `tasks::foo::Foo::new()`, without a leading `crate::`. `None` when
    /// the argument does not start with such a path.
    Task(Option<Vec<String>>),
    /// Any other statement, rendered: it may register tasks in a way this
    /// test cannot follow, such as a helper function or a macro.
    Other(String),
}

/// `statement` without the outer attributes in front of it, such as the
/// `#[cfg(feature = "...")]` of a task that only a Cargo feature builds.
fn without_attributes(mut statement: &[Token]) -> &[Token] {
    while statement.first() == Some(&Token::Punct('#'))
        && statement.get(1) == Some(&Token::Punct('['))
        && let Some(end) = closing(statement, 1)
    {
        statement = &statement[end + 1..];
    }
    statement
}

/// What one statement of `register_tasks` registers (see [`Registration`]).
fn registration(statement: &[Token]) -> Registration {
    let is_register_call = ident(statement.first()).is_some()
        && statement.get(1) == Some(&Token::Punct('.'))
        && ident(statement.get(2)) == Some("register")
        && statement.get(3) == Some(&Token::Punct('('))
        && closing(statement, 3) == Some(statement.len() - 1);
    if !is_register_call {
        return Registration::Other(render(statement));
    }
    let mut path = path_at(statement, 4);
    if path.first().is_some_and(|first| first == "crate") {
        path.remove(0);
    }
    let type_path = path
        .iter()
        .position(|segment| segment.starts_with(char::is_uppercase))
        .map(|type_index| path[..=type_index].to_vec());
    Registration::Task(type_path)
}

/// The statements of `fn register_tasks` in `tokens`, each read by
/// [`registration`]: the block's contents split at every `;` outside
/// brackets, each without its outer attributes. `None` when the function is
/// missing.
fn registered_tasks(tokens: &[Token]) -> Option<Vec<Registration>> {
    let start = tokens.windows(2).position(|pair| {
        ident(Some(&pair[0])) == Some("fn") && ident(Some(&pair[1])) == Some("register_tasks")
    })?;
    // The range starts at the block's `{`, which is left out.
    let body = &tokens[block_from(tokens, start)?][1..];
    let mut statements = Vec::new();
    let mut level = 0usize;
    let mut statement_start = 0;
    for (index, token) in body.iter().enumerate() {
        match token {
            Token::Punct('(' | '[' | '{') => level += 1,
            Token::Punct(')' | ']' | '}') => level = level.saturating_sub(1),
            Token::Punct(';') if level == 0 => {
                statements.push(&body[statement_start..index]);
                statement_start = index + 1;
            }
            _ => {}
        }
    }
    statements.push(&body[statement_start..]);
    let registered = statements
        .into_iter()
        .map(without_attributes)
        .filter(|statement| !statement.is_empty())
        .map(registration)
        .collect();
    Some(registered)
}

/// What `files` (paths below `src/` with their sources) break of the rule
/// that every task runs under the task lock, one report line per finding: a
/// `Task` impl that does not lock under its registered name, a registered
/// task without a `Task` impl that the scan found at the same crate path, a
/// statement of `register_tasks` that is not a plain `register` call, and a
/// missing `register_tasks`.
fn task_lock_report(files: &[(PathBuf, String)]) -> Vec<String> {
    let mut report = Vec::new();
    let mut implemented = Vec::new();
    let mut registered = None;
    for (relative, source) in files {
        let tokens = tokenize(&strip(source));
        let file = format!("src/{}", relative.display());
        let module = module_path(relative);
        for task in task_impls(&tokens) {
            let Some(type_name) = task.path.last() else {
                report.push(format!(
                    "{file}: an `impl Task for` a type this test cannot read; implement `Task` \
                     for a named type, as `cargo loco generate task` does"
                ));
                continue;
            };
            if !task.locked {
                report.push(format!(
                    "{file}: `impl Task for {type_name}` does not call `with_task_lock(&ctx.db, \
                     {REGISTERED_NAME}, || async {{ ... }})`"
                ));
            }
            implemented.push(crate_path(&module, &task.path));
        }
        if relative == Path::new(TASK_REGISTRY_FILE) {
            registered = registered_tasks(&tokens);
        }
    }
    let registry = format!("src/{TASK_REGISTRY_FILE}");
    let Some(registered) = registered else {
        report.push(format!(
            "{registry}: has no `fn register_tasks`, so this test cannot list the tasks"
        ));
        return report;
    };
    for registration in registered {
        match registration {
            // `ends_with`: a registration may name its type through a `use`,
            // as `Foo` or `foo::Foo`, so it matches the end of the crate path.
            Registration::Task(Some(path))
                if implemented
                    .iter()
                    .any(|candidate| candidate.ends_with(&path)) => {}
            Registration::Task(Some(path)) => {
                let path = path.join("::");
                report.push(format!(
                    "{registry}: `register_tasks` registers `{path}`, but src/ has no `impl \
                     Task for {path}` that this test can read, so its lock is unchecked"
                ));
            }
            Registration::Task(None) => report.push(format!(
                "{registry}: `register_tasks` registers a task whose type this test cannot \
                 read; register it as `tasks::<module>::<Type>`, as `cargo loco generate task` \
                 does"
            )),
            Registration::Other(statement) => report.push(format!(
                "{registry}: `register_tasks` holds `{statement}`, which is not a \
                 `tasks.register(...)` call, so this test cannot list the tasks it registers; \
                 register each task in `register_tasks` itself, one \
                 `tasks.register(tasks::<module>::<Type>);` each, as `cargo loco generate task` \
                 does"
            )),
        }
    }
    report
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())) {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn the_domain_layer_is_pure() {
    let domain = src_dir().join("domain");
    let mut files = Vec::new();
    rust_files(&domain, &mut files);
    assert!(!files.is_empty(), "src/domain/ has no Rust files");

    let mut report = Vec::new();
    for file in files {
        let relative = file.strip_prefix(&domain).expect("below src/domain");
        let source = fs::read_to_string(&file).expect("read source");
        for violation in violations(relative, &source) {
            report.push(format!("src/domain/{}: {violation}", relative.display()));
        }
    }
    assert!(
        report.is_empty(),
        "src/domain/ must stay pure (ADR 0008, rule 1):\n{}",
        report.join("\n")
    );
}

#[test]
fn src_has_no_banned_directories() {
    let banned: Vec<String> = fs::read_dir(src_dir())
        .expect("read src/")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| BANNED_DIRECTORIES.contains(&name.as_str()))
        .collect();
    assert!(
        banned.is_empty(),
        "src/ must not hold {banned:?} (ADR 0008, rule 5): logic goes on a model, in \
         src/domain/, or in the module of its only caller"
    );
}

#[test]
fn src_reads_the_time_only_through_the_app_clock() {
    let src = src_dir();
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(!files.is_empty(), "src/ has no Rust files");

    let report: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let relative = file.strip_prefix(&src).expect("below src/");
            clock_report(relative, &fs::read_to_string(file).expect("read source"))
        })
        .collect();
    assert!(
        report.is_empty(),
        "Take \"now\" from `backend3::app::clock(&ctx)?.now()` and pass it down; unit tests use \
         `FixedClock` (ADR 0010 point 7, recipe 7 of .claude/skills/backend3/SKILL.md):\n{}",
        report.join("\n")
    );
}

#[test]
fn every_task_runs_under_the_task_lock() {
    let src = src_dir();
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let sources: Vec<(PathBuf, String)> = files
        .iter()
        .map(|file| {
            let relative = file.strip_prefix(&src).expect("below src/").to_path_buf();
            (relative, fs::read_to_string(file).expect("read source"))
        })
        .collect();

    let report = task_lock_report(&sources);
    assert!(
        report.is_empty(),
        "A task does all of its database work inside \
         `backend3::models::task_lock::with_task_lock`, under its registered name, so a manual \
         run and a CronJob run never overlap (ADR 0016 §4, recipe 6 of \
         .claude/skills/backend3/SKILL.md):\n{}",
        report.join("\n")
    );
}

// The scanners themselves are tested on small samples, so a bug in one cannot
// make a real check pass silently.

/// What [`task_lock_report`] finds in `tasks/probe.rs` beside an `app.rs`
/// whose `register_tasks` holds `registered`.
fn task_findings(task_source: &str, registered: &str) -> Vec<String> {
    task_findings_in("tasks/probe.rs", task_source, registered)
}

/// What [`task_lock_report`] finds in the task file `task_file` (below
/// `src/`) beside an `app.rs` whose `register_tasks` holds `registered`.
fn task_findings_in(task_file: &str, task_source: &str, registered: &str) -> Vec<String> {
    let app =
        format!("impl Hooks for App {{ fn register_tasks(tasks: &mut Tasks) {{ {registered} }} }}");
    task_lock_report(&[
        (PathBuf::from("app.rs"), app),
        (PathBuf::from(task_file), task_source.to_owned()),
    ])
}

/// A task impl as `cargo loco generate task` writes it, with `run_body` as
/// the body of its `run`.
fn task_source(header: &str, run_body: &str) -> String {
    format!(
        "#[async_trait]\n{header} {{\n    fn task(&self) -> TaskInfo {{ TaskInfo {{ name: \
         \"probe\".to_string(), detail: String::new() }} }}\n    async fn run(&self, ctx: \
         &AppContext, _vars: &task::Vars) -> Result<()> {{\n        {run_body}\n    }}\n}}\n"
    )
}

#[test]
fn task_scanner_accepts_tasks_that_lock_under_their_registered_name() {
    let register = "tasks.register(tasks::probe::Probe);";
    for run_body in [
        "with_task_lock(&ctx.db, &self.task().name, || async { Ok(()) }).await?; Ok(())",
        "crate::models::task_lock::with_task_lock(\n &ctx.db,\n &self.task().name,\n \
         move || async move { work(&ctx.db, a, b).await },\n).await?;\nOk(())",
        "let args = parse(vars)?; // with_task_lock(&ctx.db, \"probe\", ..) is wrong\n \
         with_task_lock(&ctx.db, &self.task().name, || async { run(&ctx.db, &args).await })\
         .await.map(|_| ())",
    ] {
        let source = task_source("impl Task for Probe", run_body);
        assert_eq!(
            task_findings(&source, register),
            Vec::<String>::new(),
            "{run_body}"
        );
    }
    for header in [
        "impl loco_rs::task::Task for crate::tasks::probe::Probe",
        "impl<T: Send> Task for Probe<T> where T: Sync",
        "impl<F: for<'a> Fn(&'a str)> Task for Probe<F>",
    ] {
        let run_body =
            "with_task_lock(&ctx.db, &self.task().name, || async { Ok(()) }).await?; Ok(())";
        let source = task_source(header, run_body);
        assert_eq!(
            task_findings(&source, register),
            Vec::<String>::new(),
            "{header}"
        );
    }
    let source = task_source(
        "impl Task for Probe",
        "with_task_lock(&ctx.db, &self.task().name, f).await",
    );
    for registered in [
        "tasks.register(tasks::probe::Probe::new(Vec::new()));",
        "tasks.register(crate::tasks::probe::Probe);",
        "tasks.register(Probe);",
        "tasks.register(probe::Probe)",
        "// A task that only its Cargo feature builds.\n#[cfg(feature = \"probe\")]\n\
         tasks.register(tasks::probe::Probe);\n// tasks-inject (do not remove)\n",
    ] {
        assert_eq!(
            task_findings(&source, registered),
            Vec::<String>::new(),
            "{registered}"
        );
    }
    for (file, header) in [
        ("tasks/probe/mod.rs", "impl Task for Probe"),
        ("tasks/probe/run.rs", "impl Task for super::Probe"),
        ("tasks/mod.rs", "impl Task for self::probe::Probe"),
        ("app.rs", "impl Task for crate::tasks::probe::Probe"),
    ] {
        let source = task_source(
            header,
            "with_task_lock(&ctx.db, &self.task().name, f).await",
        );
        let findings = if file == "app.rs" {
            let app = format!(
                "{source}\nimpl Hooks for App {{ fn register_tasks(tasks: &mut Tasks) {{ \
                 tasks.register(tasks::probe::Probe); }} }}"
            );
            task_lock_report(&[(PathBuf::from(file), app)])
        } else {
            task_findings_in(file, &source, "tasks.register(tasks::probe::Probe);")
        };
        assert_eq!(findings, Vec::<String>::new(), "{file}: {header}");
    }
    assert_eq!(
        task_findings(
            "impl Hooks for App {}\nfn f() -> impl Future<Output = ()> { async {} }",
            ""
        ),
        Vec::<String>::new(),
        "no tasks, nothing to check"
    );
}

#[test]
fn task_scanner_flags_a_task_that_does_not_lock_under_its_registered_name() {
    let register = "tasks.register(tasks::probe::Probe);";
    let expected = [
        "src/tasks/probe.rs: `impl Task for Probe` does not call `with_task_lock(&ctx.db, \
         &self.task().name, || async { ... })`",
    ];
    for run_body in [
        "work(&ctx.db).await",
        "with_task_lock(&ctx.db, \"probe\", || async { Ok(()) }).await?; Ok(())",
        "with_task_lock(&ctx.db, TASK_NAME, || async { Ok(()) }).await?; Ok(())",
        "// with_task_lock(&ctx.db, &self.task().name, || async { .. })\n work(&ctx.db).await",
        "let _ = \"with_task_lock(&ctx.db, &self.task().name, f)\"; work(&ctx.db).await",
        "let lock = with_task_lock; work(&ctx.db, &self.task().name).await",
    ] {
        let source = task_source("impl Task for Probe", run_body);
        assert_eq!(task_findings(&source, register), expected, "{run_body}");
    }
    let generic = task_source(
        "impl<T> loco_rs::task::Task for crate::tasks::probe::Probe<T> where T: Send",
        "work(&ctx.db).await",
    );
    assert_eq!(
        task_findings(&generic, register),
        expected,
        "path and generics"
    );
}

#[test]
fn task_scanner_flags_registered_tasks_it_cannot_check() {
    let locked = task_source(
        "impl Task for Probe",
        "with_task_lock(&ctx.db, &self.task().name, f).await",
    );
    assert_eq!(
        task_findings(
            &locked,
            "tasks.register(tasks::probe::Probe);\ntasks.register(tasks::other::Other);"
        ),
        [
            "src/app.rs: `register_tasks` registers `tasks::other::Other`, but src/ has no `impl \
             Task for tasks::other::Other` that this test can read, so its lock is unchecked"
        ]
    );
    // A locked impl of the same name in another module does not stand in for
    // the registered type.
    assert_eq!(
        task_findings(&locked, "tasks.register(tasks::other::Probe);"),
        [
            "src/app.rs: `register_tasks` registers `tasks::other::Probe`, but src/ has no `impl \
             Task for tasks::other::Probe` that this test can read, so its lock is unchecked"
        ]
    );
    assert_eq!(
        task_findings(&locked, "tasks.register(make_task());"),
        [
            "src/app.rs: `register_tasks` registers a task whose type this test cannot read; \
             register it as `tasks::<module>::<Type>`, as `cargo loco generate task` does"
        ]
    );
    // Registration outside a plain `register` call could hide a task whose
    // impl the scan cannot read.
    for (statement, rendered) in [
        (
            "crate::tasks::register_all(tasks);",
            "crate::tasks::register_all(tasks)",
        ),
        ("register(tasks);", "register(tasks)"),
        ("register_all!(tasks);", "register_all!(tasks)"),
        (
            "if cfg!(test) { tasks.register(tasks::probe::Probe); }",
            "if cfg!(test){tasks.register(tasks::probe::Probe);}",
        ),
        (
            "tasks.register(tasks::probe::Probe).register(tasks::other::Other);",
            "tasks.register(tasks::probe::Probe).register(tasks::other::Other)",
        ),
    ] {
        let registered = format!("tasks.register(tasks::probe::Probe);\n{statement}");
        assert_eq!(
            task_findings(&locked, &registered),
            [format!(
                "src/app.rs: `register_tasks` holds `{rendered}`, which is not a \
                 `tasks.register(...)` call, so this test cannot list the tasks it registers; \
                 register each task in `register_tasks` itself, one \
                 `tasks.register(tasks::<module>::<Type>);` each, as `cargo loco generate task` \
                 does"
            )],
            "{statement}"
        );
    }
    let generated_by_a_macro = "macro_rules! task { ($name:ident) => { impl Task for $name { \
         async fn run(&self) { with_task_lock(&ctx.db, &self.task().name, f).await } } } }\n\
         task!(Probe);";
    assert_eq!(
        task_findings(generated_by_a_macro, "tasks.register(tasks::probe::Probe);"),
        [
            "src/tasks/probe.rs: an `impl Task for` a type this test cannot read; implement \
             `Task` for a named type, as `cargo loco generate task` does",
            "src/app.rs: `register_tasks` registers `tasks::probe::Probe`, but src/ has no `impl \
             Task for tasks::probe::Probe` that this test can read, so its lock is unchecked",
        ]
    );
    let without_registry = task_lock_report(&[
        (PathBuf::from("app.rs"), "impl Hooks for App {}".to_owned()),
        (PathBuf::from("tasks/probe.rs"), locked),
    ]);
    assert_eq!(
        without_registry,
        ["src/app.rs: has no `fn register_tasks`, so this test cannot list the tasks"]
    );
}

#[test]
fn task_scanner_resolves_impl_paths_as_rust_does() {
    let strings = |items: &[&str]| {
        items
            .iter()
            .map(|&item| item.to_owned())
            .collect::<Vec<_>>()
    };
    for (file, module) in [
        ("tasks/probe.rs", &["tasks", "probe"][..]),
        ("tasks/probe/mod.rs", &["tasks", "probe"]),
        ("tasks/mod.rs", &["tasks"]),
        ("app.rs", &["app"]),
        ("lib.rs", &[]),
    ] {
        assert_eq!(module_path(Path::new(file)), strings(module), "{file}");
    }
    let module = strings(&["tasks", "probe"]);
    for (path, resolved) in [
        (&["Probe"][..], &["tasks", "probe", "Probe"][..]),
        (&["run", "Probe"], &["tasks", "probe", "run", "Probe"]),
        (&["self", "Probe"], &["tasks", "probe", "Probe"]),
        (&["super", "other", "Other"], &["tasks", "other", "Other"]),
        (&["super", "super", "Probe"], &["Probe"]),
        (
            &["crate", "tasks", "other", "Other"],
            &["tasks", "other", "Other"],
        ),
    ] {
        assert_eq!(
            crate_path(&module, &strings(path)),
            strings(resolved),
            "{path:?}"
        );
    }
}

/// What [`clock_reads`] finds in `source`, without the token indexes.
fn reads(source: &str) -> Vec<String> {
    clock_reads(&tokenize(&strip(source)))
        .into_iter()
        .map(|(_, what)| what)
        .collect()
}

#[test]
fn clock_scanner_flags_each_read() {
    let local = "chrono's `Local`";
    for (source, expected) in [
        ("chrono::Utc::now()", "`Utc::now`"),
        ("Utc :: now ( ) . date_naive()", "`Utc::now`"),
        ("iter::repeat_with(Utc::now)", "`Utc::now`"),
        ("Utc::today()", "`Utc::today`"),
        ("Local::now()", "`Local::now`"),
        ("sea_orm::prelude::ChronoUtc::now()", "`ChronoUtc::now`"),
        ("std::time::SystemTime::now()", "`SystemTime::now`"),
        ("OffsetDateTime::now_local()", "`OffsetDateTime::now_local`"),
        (
            "TimeDateTimeWithTimeZone::now_utc()",
            "`TimeDateTimeWithTimeZone::now_utc`",
        ),
        ("time::UtcDateTime::now()", "`UtcDateTime::now`"),
        ("jiff::Zoned::now()", "`Zoned::now`"),
        ("get_current_timestamp()", "`get_current_timestamp`"),
        ("std::time::UNIX_EPOCH.elapsed()", "`UNIX_EPOCH`"),
        ("fn f(_: prelude::DateTimeLocal) {}", "`DateTimeLocal`"),
        ("type T = ChronoDateTimeLocal;", "`ChronoDateTimeLocal`"),
        ("use chrono::Local;", local),
        ("use chrono::{DateTime, Local, Utc};", local),
        ("pub use chrono::{offset::{Local as Here}};", local),
        ("use chrono::prelude::Local;", local),
        ("fn f() -> DateTime<chrono::prelude::Local> {}", local),
        ("let _: (u8, chrono::Local);", local),
        (
            "use chrono::prelude::*;",
            "a glob import of chrono, which brings in `Local`",
        ),
    ] {
        assert_eq!(reads(source), [expected], "{source}");
    }
    assert_eq!(
        reads("chrono::offset::Local::now()"),
        ["`Local::now`", local],
        "both a read and chrono's zone"
    );
}

#[test]
fn clock_scanner_accepts_code_that_reads_no_clock() {
    let source = r#"
        //! Mentions Utc::now(), SystemTime::now() and chrono::Local in prose.
        /* Local::now() in a block comment */
        use chrono::{DateTime, TimeDelta, Utc};
        use crate::domain::database_target::Locality::{self, Local};
        enum Locality { Local, NonLocal { host: String } }
        fn f(clock: &dyn Clock, at: DateTime<Utc>) -> DateTime<Utc> {
            let _ = "Utc::now() and Local::now() in a string";
            let _ = Utc.timestamp_opt(0, 0);
            let _ = (std::time::Instant::now(), tokio::time::Instant::now());
            let _ = (budapest::today(at), Week::today(at), HashMap::<u8, Local>::new());
            match Locality::Local { Locality::Local => {}, Self::Local => {}, _ => {} }
            clock.now() + TimeDelta::seconds(1)
        }
        fn now(&self) -> DateTime<Utc> { self.0 }
    "#;
    assert_eq!(reads(source), Vec::<String>::new());
}

#[test]
fn only_the_system_clock_in_app_rs_reads_the_time_or_sets_the_clock() {
    let system_clock = "impl Clock for SystemClock {\n    fn now(&self) -> DateTime<Utc> {\n        \
                        Utc::now()\n    }\n}\n";
    let set_clock = "fn after_context(ctx: AppContext) { set_clock(&ctx, clock) }\n";
    let app = Path::new("app.rs");
    assert_eq!(
        clock_report(app, &format!("{system_clock}{set_clock}")),
        Vec::<String>::new()
    );
    assert_eq!(
        clock_report(app, &format!("{system_clock}fn sneaky() {{ Utc::now(); }}")),
        ["src/app.rs: uses `Utc::now`"],
        "a read outside the impl, in the same file"
    );
    assert_eq!(
        clock_report(Path::new("models/clock.rs"), system_clock),
        ["src/models/clock.rs: uses `Utc::now`"],
        "the impl is allowed in src/app.rs only"
    );
    assert_eq!(
        clock_report(Path::new("controllers/debug.rs"), set_clock),
        [
            "src/controllers/debug.rs: replaces the app clock with `set_clock`, which only \
             `App::after_context` and tests/support call"
        ]
    );
}

// The scanner itself is tested on small samples, so a bug in it cannot make
// the real check pass silently.

fn check(file: &str, source: &str) -> Vec<String> {
    violations(Path::new(file), source)
}

#[test]
fn scanner_accepts_pure_code() {
    let source = r##"
        //! Talks about tokio, sqlx and std::fs in prose only.
        use core::net::IpAddr;
        use crate::domain::admin_path;
        use super::helper;
        /* block comment with async fn and Utc::now() /* nested */ */
        pub fn f(x: &str) -> String { format!("std::env {x} AppContext") }
        fn g<'a>(s: &'a str) -> char { let _ = r#"tokio"#; '"' }
        #[cfg(test)]
        mod tests { use super::*; }
    "##;
    assert_eq!(check("x.rs", source), Vec::<String>::new());
}

#[test]
fn scanner_flags_each_rule() {
    let flagged =
        |source: &str| !check("x.rs", &format!("{source}\n#[cfg(test)] mod t {{}}")).is_empty();
    assert!(flagged("use tokio::sync::Mutex;"));
    assert!(flagged("use loco_rs::prelude::*;"));
    assert!(flagged("fn f(db: &sea_orm::DatabaseConnection) {}"));
    assert!(flagged("fn f() { let _ = std::fs::read(\"x\"); }"));
    assert!(flagged("use std::{collections::HashMap, env};"));
    assert!(flagged("use std :: net :: TcpStream;"));
    assert!(flagged("fn f() { let _ = chrono::Utc::now(); }"));
    assert!(flagged("fn f() { let _ = std::time::Instant::now(); }"));
    assert!(flagged("pub async fn f() {}"));
    assert!(flagged("use crate::models::posts;"));
    assert!(flagged("use crate::{settings::Settings};"));
    assert!(flagged("fn f(ctx: &AppContext) {}"));
    assert!(flagged("fn f() { let _ = rand::random::<u8>(); }"));
    assert!(flagged("use super::super::app;"));
    assert!(
        !check("x.rs", "pub fn f() {}").is_empty(),
        "missing #[cfg(test)]"
    );
    assert!(
        !check("mod.rs", "use super::app;").is_empty(),
        "mod.rs is the domain root"
    );
    assert_eq!(
        check("a/mod.rs", "use super::admin_path;"),
        Vec::<String>::new()
    );
    assert_ne!(
        check(
            "a/b.rs",
            "use super::super::super::x;\n#[cfg(test)] mod t {}"
        ),
        Vec::<String>::new()
    );
}

// ADR 0011: the public API reads only through live-row finders.

/// The directories below `src/` that hold the public read API's modules.
const PUBLIC_API_DIRS: &[&str] = &["controllers/v1", "dtos/v1"];

/// Names that let a module read rows itself: Sea-ORM's crate, the generated
/// entities and their types, the query traits, the entity and relation
/// finders, the raw-SQL entry points, and sqlx. `Entity::find()` is caught
/// through `Entity`, and any `<Type>::find(` by [`public_api_report`]
/// itself, which leaves an iterator's `.find(` alone.
const DIRECT_READS: &[&str] = &[
    "sea_orm",
    "sqlx",
    "_entities",
    "Entity",
    "Column",
    "ActiveModel",
    "EntityTrait",
    "ModelTrait",
    "QueryFilter",
    "QuerySelect",
    "QueryOrder",
    "QueryTrait",
    "PaginatorTrait",
    "find_by_id",
    "find_related",
    "find_also_related",
    "find_with_related",
    "find_linked",
    "find_also_linked",
    "find_with_linked",
    "find_by_statement",
    "from_raw_sql",
    "into_partial_model",
    "Statement",
    "query_one",
    "query_all",
    "query_one_raw",
    "query_all_raw",
    "execute",
    "execute_raw",
    "execute_unprepared",
];

/// Whether `name` is a live-row finder's: one of its snake-case words is
/// `live`.
fn is_live_finder(name: &str) -> bool {
    name.split('_').any(|word| word == "live")
}

/// The types through which a model function reaches the database. ADR 0008
/// rules 2 and 4 make a model function take its connection or transaction
/// as an argument, so a function whose signature names none of these reads
/// no rows: `serve_content` (ADR 0013), which takes the cache and the
/// metrics family, or the read counter (public-api.md §15), which takes its
/// in-memory counts. `AppContext` is listed too, although rule 2 keeps it
/// out of models, because it carries the pool.
const CONNECTION_TYPES: &[&str] = &[
    "ConnectionTrait",
    "DatabaseConnection",
    "DatabaseTransaction",
    "DbConn",
    "TransactionTrait",
    "StreamTrait",
    "AppContext",
];

/// The functions that `src/models/` defines, outside the generated
/// `_entities/`, by name.
#[derive(Debug, Default)]
struct ModelFunctions {
    /// The functions whose name lacks the word `live` and whose signature
    /// names a [`CONNECTION_TYPES`] type, so that they can read rows: the
    /// name and the file (below `src/`) of the first such definition.
    reading: BTreeMap<String, String>,
    /// Every function name defined there.
    defined: BTreeSet<String>,
}

/// The function definitions of the model files `files` (each a path below
/// `src/` and its source), as [`ModelFunctions`]. A name that one file
/// defines with a connection and another without counts as reading: the
/// scan of the public API knows the name only.
fn model_functions(files: &[(PathBuf, String)]) -> ModelFunctions {
    let mut functions = ModelFunctions::default();
    for (relative, source) in files {
        let tokens = tokenize(&strip(source));
        for (index, token) in tokens.iter().enumerate() {
            if ident(Some(token)) != Some("fn") {
                continue;
            }
            let Some(name) = ident(tokens.get(index + 1)) else {
                continue;
            };
            functions.defined.insert(name.to_owned());
            // The signature: from the name to the body's `{`, or to the `;`
            // of a declaration without one. Generic bounds and `where`
            // clauses hold no braces.
            let signature = tokens[index..]
                .iter()
                .take_while(|token| !matches!(token, Token::Punct('{' | ';')));
            let takes_connection = signature
                .filter_map(|token| ident(Some(token)))
                .any(|word| CONNECTION_TYPES.contains(&word));
            if takes_connection && !is_live_finder(name) {
                functions
                    .reading
                    .entry(name.to_owned())
                    .or_insert_with(|| format!("src/{}", relative.display()));
            }
        }
    }
    functions
}

/// One leaf of a `use` tree: the name it brings into scope, the full path
/// it stands for, and whether it is a glob (`*`, with an empty name).
#[derive(Debug, Clone, PartialEq, Eq)]
struct UseLeaf {
    /// The name in scope: the last segment, or the `as` name.
    local: String,
    /// The segments of the path, `self` resolved to its module.
    path: Vec<String>,
    /// Whether the leaf is `*`.
    glob: bool,
}

impl UseLeaf {
    /// Whether the leaf reaches into `crate::models` (or a `super::`
    /// path to it): one of its segments is `models`.
    fn reaches_models(&self) -> bool {
        self.path.iter().any(|segment| segment == "models")
    }
}

/// Every leaf of every `use` statement in `tokens`, in order.
fn use_leaves(tokens: &[Token]) -> Vec<UseLeaf> {
    let mut leaves = Vec::new();
    for (start, token) in tokens.iter().enumerate() {
        if ident(Some(token)) != Some("use") {
            continue;
        }
        let end = tokens[start..]
            .iter()
            .position(|token| token == &Token::Punct(';'))
            .map_or(tokens.len(), |length| start + length);
        use_tree(&tokens[start + 1..end], 0, &[], &mut leaves);
    }
    leaves
}

/// Reads one use tree of `tokens` from `start`, below the path `prefix`,
/// into `leaves`, and returns the position after it. The grammar is
/// Rust's: a path, then `::{...}` with trees separated by commas, `::*`,
/// or `as name`. Anything else ends the tree, so an unexpected token
/// cannot stall the scan.
fn use_tree(tokens: &[Token], start: usize, prefix: &[String], leaves: &mut Vec<UseLeaf>) -> usize {
    let mut path = prefix.to_vec();
    let mut index = start;
    loop {
        match tokens.get(index) {
            Some(Token::PathSep) => index += 1,
            Some(Token::Punct('*')) => {
                leaves.push(UseLeaf {
                    local: String::new(),
                    path,
                    glob: true,
                });
                return index + 1;
            }
            Some(Token::Punct('{')) => {
                index += 1;
                loop {
                    match tokens.get(index) {
                        None => return index,
                        Some(Token::Punct('}')) => return index + 1,
                        Some(Token::Punct(',')) => index += 1,
                        Some(_) => {
                            let next = use_tree(tokens, index, &path, leaves);
                            // A tree that consumed nothing is malformed;
                            // skip its token rather than loop on it.
                            index = if next == index { index + 1 } else { next };
                        }
                    }
                }
            }
            Some(Token::Ident(name)) => {
                path.push(name.clone());
                index += 1;
                if tokens.get(index) == Some(&Token::PathSep) {
                    continue;
                }
                // `module::{self}` stands for the module itself.
                if path.last().is_some_and(|last| last == "self") {
                    path.pop();
                }
                let renamed = ident(tokens.get(index)) == Some("as");
                let local = if renamed {
                    ident(tokens.get(index + 1)).unwrap_or_default().to_owned()
                } else {
                    path.last().cloned().unwrap_or_default()
                };
                leaves.push(UseLeaf {
                    local,
                    path,
                    glob: false,
                });
                return if renamed { index + 2 } else { index };
            }
            _ => return index,
        }
    }
}

/// Whether the identifier at `tokens[index]` is called: `(` follows it,
/// or a turbofish (`::<...>`) and then `(`.
fn is_called(tokens: &[Token], index: usize) -> bool {
    match tokens.get(index + 1) {
        Some(Token::Punct('(')) => true,
        Some(Token::PathSep) if tokens.get(index + 2) == Some(&Token::Punct('<')) => {
            let mut depth = 0usize;
            for position in index + 2..tokens.len() {
                match tokens[position] {
                    Token::Punct('<') => depth += 1,
                    // The `>` of `->` (in `Fn() -> T`) closes nothing.
                    Token::Punct('>') if tokens[position - 1] != Token::Punct('-') => {
                        depth -= 1;
                        if depth == 0 {
                            return tokens.get(position + 1) == Some(&Token::Punct('('));
                        }
                    }
                    _ => {}
                }
            }
            false
        }
        _ => false,
    }
}

/// What one public-API file breaks of the live-row rule, one line each.
///
/// `models` are the functions of `src/models/`. A call by path that goes
/// through `crate::models` (a `models` or `Model` segment, or a name that a
/// `use` of it brought in) must name a live-row finder or a model function
/// that takes no connection; a method call (`.name(`), whose receiver a
/// text check cannot type, must not use the name of a model function that
/// takes a connection and is not a live-row finder.
fn public_api_report(relative: &Path, source: &str, models: &ModelFunctions) -> Vec<String> {
    let tokens = tokenize(&strip(source));
    let imports: Vec<UseLeaf> = use_leaves(&tokens)
        .into_iter()
        .filter(UseLeaf::reaches_models)
        .collect();
    let mut found = Vec::new();
    if imports.iter().any(|leaf| leaf.glob) {
        found
            .push("imports from `crate::models` with a glob; name each live-row finder".to_owned());
    }
    for (index, token) in tokens.iter().enumerate() {
        let Token::Ident(name) = token else { continue };
        let before = index.checked_sub(1).and_then(|before| tokens.get(before));
        let called = is_called(&tokens, index);
        if DIRECT_READS.contains(&name.as_str()) {
            found.push(format!("names `{name}`"));
        }
        if name == "find" && before == Some(&Token::PathSep) && called {
            found.push("calls an entity's `::find()`".to_owned());
        }
        if !called || !name.starts_with(char::is_lowercase) || is_live_finder(name) {
            continue;
        }
        if before == Some(&Token::Punct('.')) {
            if let Some(file) = models.reading.get(name) {
                found.push(format!(
                    "calls the method `.{name}(`, the name of a model function in {file} that \
                     takes a database connection and is not a live-row finder (the scan cannot \
                     see the receiver's type: if it is not a model, rename the model function)"
                ));
            }
            continue;
        }
        // The path that ends in this call, first segment first.
        let mut segments = vec![name.as_str()];
        let mut cursor = index;
        while cursor >= 2 && tokens[cursor - 1] == Token::PathSep {
            match ident(tokens.get(cursor - 2)) {
                Some(segment) => segments.insert(0, segment),
                None => break,
            }
            cursor -= 2;
        }
        let through_models = segments[..segments.len() - 1]
            .iter()
            .any(|segment| *segment == "models" || *segment == "Model")
            || imports.iter().any(|leaf| leaf.local == segments[0]);
        if !through_models {
            continue;
        }
        let call = segments.join("::");
        if models.reading.contains_key(name) {
            found.push(format!(
                "calls `{call}`, which takes a database connection but is not a live-row finder"
            ));
        } else if !models.defined.contains(name) {
            found.push(format!(
                "calls `{call}` through `crate::models`, but src/models defines no `{name}`; \
                 call a live-row finder, or a model function that takes no database connection"
            ));
        }
    }
    found
        .into_iter()
        .map(|finding| format!("src/{}: {finding}", relative.display()))
        .collect()
}

/// Whether `tokens` filter the trash in code: `DeletedAt.is_null()`, also
/// as `Expr::col(Column::DeletedAt).is_null()` with closing parentheses in
/// between. `DeletedAt.is_not_null()` reads the trash and does not count.
fn filters_trash_in_code(tokens: &[Token]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        if ident(Some(token)) != Some("DeletedAt") {
            return false;
        }
        let mut cursor = index + 1;
        while tokens.get(cursor) == Some(&Token::Punct(')')) {
            cursor += 1;
        }
        tokens.get(cursor) == Some(&Token::Punct('.'))
            && ident(tokens.get(cursor + 1)) == Some("is_null")
            && tokens.get(cursor + 2) == Some(&Token::Punct('('))
    })
}

/// Whether the SQL in `text` (code with its literals kept) filters the
/// trash: `deleted_at IS NULL`, in any letter case, with any white space,
/// a table qualifier or the column quoted. `IS NOT NULL` does not count.
fn filters_trash_in_sql(text: &str) -> bool {
    let words: Vec<String> = text
        .replace('"', " ")
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    words.windows(3).any(|window| {
        (window[0] == "deleted_at" || window[0].ends_with(".deleted_at"))
            && window[1] == "is"
            && window[2].starts_with("null")
    })
}

/// The live-row finders of one model file that do not filter the trash, one
/// line each.
fn live_finder_report(relative: &Path, source: &str) -> Vec<String> {
    let code: Vec<char> = strip(source).chars().collect();
    let text: Vec<char> = strip_comments(source).chars().collect();
    let tokens = tokenize(&strip(source));
    let mut found = Vec::new();
    let mut search_from = 0;
    for (index, token) in tokens.iter().enumerate() {
        if ident(Some(token)) != Some("fn") {
            continue;
        }
        let Some(name) = ident(tokens.get(index + 1)).filter(|name| is_live_finder(name)) else {
            continue;
        };
        // The character range of this function's body in the stripped code:
        // from the first `{` after its name, unless a `;` ends a bodyless
        // declaration first, to the matching `}`.
        let Some(at) = find_word(&code, "fn", name, search_from) else {
            continue;
        };
        search_from = at + 1;
        let Some(open) = (at..code.len()).find(|&i| code[i] == '{' || code[i] == ';') else {
            continue;
        };
        if code[open] == ';' {
            continue;
        }
        let mut level = 0usize;
        let mut close = code.len();
        for (offset, &c) in code[open..].iter().enumerate() {
            match c {
                '{' => level += 1,
                '}' => {
                    level -= 1;
                    if level == 0 {
                        close = open + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body_text: String = text[open..close].iter().collect();
        let body_code: String = code[open..close].iter().collect();
        let body = tokenize(&body_code);
        // Only a call counts: a local variable called `live` filters nothing.
        let calls_live = body.iter().enumerate().any(|(position, token)| {
            ident(Some(token)).is_some_and(|other| other != name && is_live_finder(other))
                && is_called(&body, position)
        });
        if !(filters_trash_in_code(&body) || filters_trash_in_sql(&body_text) || calls_live) {
            found.push(format!(
                "src/{}: `{name}` is named as a live-row finder but does not filter the trash: \
                 add `deleted_at IS NULL` (Column::DeletedAt.is_null()) or call another live-row \
                 finder",
                relative.display()
            ));
        }
    }
    found
}

/// The character index of `fn <name>` in `code` from `from` on, the two
/// words separated by white space only.
fn find_word(code: &[char], keyword: &str, name: &str, from: usize) -> Option<usize> {
    let pattern: Vec<char> = keyword.chars().collect();
    let name: Vec<char> = name.chars().collect();
    (from..code.len()).find(|&start| {
        if !code[start..].starts_with(&pattern)
            || start
                .checked_sub(1)
                .is_some_and(|before| code[before].is_alphanumeric() || code[before] == '_')
        {
            return false;
        }
        let mut cursor = start + pattern.len();
        let gap = cursor;
        while code.get(cursor).is_some_and(|c| c.is_whitespace()) {
            cursor += 1;
        }
        cursor > gap
            && code[cursor..].starts_with(&name)
            && !code
                .get(cursor + name.len())
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    })
}

/// Every Rust file of `src/models/` outside the generated `_entities/`, as
/// its path below `src/` and its source.
fn model_sources() -> Vec<(PathBuf, String)> {
    let src = src_dir();
    let mut files = Vec::new();
    rust_files(&src.join("models"), &mut files);
    files
        .into_iter()
        .filter(|file| {
            !file
                .components()
                .any(|part| part.as_os_str() == "_entities")
        })
        .map(|file| {
            let source = fs::read_to_string(&file).expect("read source");
            (
                file.strip_prefix(&src).expect("below src/").to_path_buf(),
                source,
            )
        })
        .collect()
}

#[test]
fn the_public_api_reads_only_through_live_row_finders() {
    let src = src_dir();
    let mut files = Vec::new();
    for dir in PUBLIC_API_DIRS {
        rust_files(&src.join(dir), &mut files);
    }
    assert!(
        !files.is_empty(),
        "no public-API modules in {PUBLIC_API_DIRS:?}"
    );
    let models = model_functions(&model_sources());
    // Without model functions the method-call rule would check nothing.
    assert!(
        !models.reading.is_empty(),
        "no model function in src/models/ takes a connection: {models:?}"
    );
    let report: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let relative = file.strip_prefix(&src).expect("below src/");
            public_api_report(
                relative,
                &fs::read_to_string(file).expect("read source"),
                &models,
            )
        })
        .collect();
    assert!(
        report.is_empty(),
        "A public /api/v1 module reads rows only through a model's live-row finder, a function \
         whose name has the word `live` and which filters `deleted_at IS NULL`; it may also call \
         a model function that takes no database connection (ADR 0011 §3, recipe 2 step 4 of \
         .claude/skills/backend3/SKILL.md):\n{}",
        report.join("\n")
    );
}

#[test]
fn every_live_row_finder_filters_the_trash() {
    let sources = model_sources();
    assert!(!sources.is_empty(), "src/models/ has no Rust files");
    let report: Vec<String> = sources
        .iter()
        .flat_map(|(relative, source)| live_finder_report(relative, source))
        .collect();
    assert!(
        report.is_empty(),
        "ADR 0011 §3: a live-row finder filters the trash:\n{}",
        report.join("\n")
    );
}

/// A model file for the scanner tests: functions that take a connection
/// in each way a signature can name one, a live-row finder, and functions
/// that take none, as `serve_content` and the read counter will.
const SAMPLE_MODELS: &str = r"
    impl Model {
        pub async fn find_by_pid<C: ConnectionTrait>(db: &C, pid: &str) -> ModelResult<Self> {}
        pub async fn labels(&self, db: &DatabaseConnection) -> Result<Vec<labels::Model>> {}
        pub async fn list<C>(db: &C) -> Result<Vec<Self>> where C: ConnectionTrait {}
        pub async fn page(db: &DatabaseTransaction) -> Result<Vec<Self>> {}
        pub async fn everything(db: &impl ConnectionTrait) -> Result<Vec<Self>> {}
        pub async fn find_live_by_id(db: &impl ConnectionTrait, id: i64) -> Result<Self> {}
        pub fn display_name(&self) -> String {}
    }
    pub async fn stats(db: &DatabaseConnection) -> Result<Stats> {}
    pub fn serve_content(cache: &Cache, resanitized: &Counter, row: &Row) -> Served {}
    pub fn record(counts: &ReadCounts, post: i64) {}
";

/// What [`public_api_report`] finds in `source`, without the file name,
/// with the model functions of [`SAMPLE_MODELS`].
fn public_findings(source: &str) -> Vec<String> {
    let models = model_functions(&[(PathBuf::from("models/posts.rs"), SAMPLE_MODELS.to_owned())]);
    public_api_report(Path::new("controllers/v1/x.rs"), source, &models)
        .into_iter()
        .map(|line| {
            line.trim_start_matches("src/controllers/v1/x.rs: ")
                .to_owned()
        })
        .collect()
}

#[test]
fn model_functions_are_sorted_by_whether_they_take_a_connection() {
    let models = model_functions(&[(PathBuf::from("models/posts.rs"), SAMPLE_MODELS.to_owned())]);
    assert_eq!(
        models.reading.keys().collect::<Vec<_>>(),
        [
            "everything",
            "find_by_pid",
            "labels",
            "list",
            "page",
            "stats"
        ]
    );
    assert_eq!(models.reading["labels"], "src/models/posts.rs");
    for name in ["find_live_by_id", "display_name", "serve_content", "record"] {
        assert!(models.defined.contains(name), "{name}");
        assert!(!models.reading.contains_key(name), "{name}");
    }
}

#[test]
fn use_trees_are_read_leaf_by_leaf() {
    let leaves = use_leaves(&tokenize(
        "use crate::{dtos::v1::posts::Post, models::{posts::{self, Model as P}, labels::*}};\n\
         use super::super::models as m; pub use std::fmt;",
    ));
    let view: Vec<(&str, String, bool)> = leaves
        .iter()
        .map(|leaf| (leaf.local.as_str(), leaf.path.join("::"), leaf.glob))
        .collect();
    assert_eq!(
        view,
        [
            ("Post", "crate::dtos::v1::posts::Post".to_owned(), false),
            ("posts", "crate::models::posts".to_owned(), false),
            ("P", "crate::models::posts::Model".to_owned(), false),
            ("", "crate::models::labels".to_owned(), true),
            ("m", "super::super::models".to_owned(), false),
            ("fmt", "std::fmt".to_owned(), false),
        ]
    );
    let from_models: Vec<&str> = leaves
        .iter()
        .filter(|leaf| leaf.reaches_models())
        .map(|leaf| leaf.local.as_str())
        .collect();
    assert_eq!(from_models, ["posts", "P", "", "m"]);
    // A malformed tree ends the scan of its statement, not the test.
    assert_eq!(use_leaves(&tokenize("use crate::{a, , ::b, 1};")).len(), 2);
}

#[test]
fn calls_are_found_with_and_without_a_turbofish() {
    let calls = |source: &str| -> Vec<String> {
        let tokens = tokenize(source);
        tokens
            .iter()
            .enumerate()
            .filter(|(index, _)| is_called(&tokens, *index))
            .filter_map(|(_, token)| ident(Some(token)).map(str::to_owned))
            .collect()
    };
    assert_eq!(
        calls("a(); b::<C>(x); c::<Vec<u8>>(); d::<fn() -> u8>(); e::<C>; f; g::h()"),
        // `fn(` counts as a call too; the scan skips names that no model
        // function can have.
        ["a", "b", "c", "d", "fn", "h"]
    );
}

#[test]
fn live_row_scanner_flags_each_direct_read() {
    for (source, expected) in [
        (
            "let _ = posts::Entity::find();",
            vec!["names `Entity`", "calls an entity's `::find()`"],
        ),
        (
            "let _ = Posts::find().all(db);",
            vec!["calls an entity's `::find()`"],
        ),
        ("let _ = Posts::find_by_id(1);", vec!["names `find_by_id`"]),
        (
            "let _ = post.find_related(Labels);",
            vec!["names `find_related`"],
        ),
        (
            "db.query_all_raw(Statement::from_string(b, s))",
            vec!["names `query_all_raw`", "names `Statement`"],
        ),
        (
            "use crate::models::_entities::posts;",
            vec!["names `_entities`"],
        ),
        (
            "use sea_orm::QueryFilter;",
            vec!["names `sea_orm`", "names `QueryFilter`"],
        ),
        (
            ".filter(Column::DeletedAt.is_null())",
            vec!["names `Column`"],
        ),
    ] {
        assert_eq!(public_findings(source), expected, "{source}");
    }
}

#[test]
fn live_row_scanner_flags_each_model_call_that_may_read_the_trash() {
    for (source, expected) in [
        (
            "crate::models::users::Model::find_by_pid(db, pid)",
            vec![
                "calls `crate::models::users::Model::find_by_pid`, which takes a database \
                 connection but is not a live-row finder",
            ],
        ),
        (
            "use crate::models::users;\nusers::Model::find_by_pid(db, pid)",
            vec![
                "calls `users::Model::find_by_pid`, which takes a database connection but is \
                 not a live-row finder",
            ],
        ),
        (
            "use crate::models::{posts::{self, Model as Post}, labels};\n\
             Post::page(db); posts::list(db); labels::Model::everything(db)",
            vec![
                "calls `Post::page`, which takes a database connection but is not a live-row \
                 finder",
                "calls `posts::list`, which takes a database connection but is not a live-row \
                 finder",
                "calls `labels::Model::everything`, which takes a database connection but is \
                 not a live-row finder",
            ],
        ),
        (
            "use super::super::models::job_queue::stats;\nstats(db)",
            vec!["calls `stats`, which takes a database connection but is not a live-row finder"],
        ),
        (
            "use crate::models as m;\nm::posts::Model::page(db)",
            vec![
                "calls `m::posts::Model::page`, which takes a database connection but is not \
                 a live-row finder",
            ],
        ),
        (
            "posts::Model::list::<DatabaseConnection>(db)",
            vec![
                "calls `posts::Model::list`, which takes a database connection but is not a \
                 live-row finder",
            ],
        ),
        (
            "let post = posts::Model::find_live_by_id(db, 1).await?;\n\
             let labels = post.labels(db).await?; let more = post.labels::<C>(db);",
            vec![
                "calls the method `.labels(`, the name of a model function in \
                 src/models/posts.rs that takes a database connection and is not a live-row \
                 finder (the scan cannot see the receiver's type: if it is not a model, rename \
                 the model function)",
                "calls the method `.labels(`, the name of a model function in \
                 src/models/posts.rs that takes a database connection and is not a live-row \
                 finder (the scan cannot see the receiver's type: if it is not a model, rename \
                 the model function)",
            ],
        ),
        (
            "use crate::models::posts;\nposts::Model::lookup(db)",
            vec![
                "calls `posts::Model::lookup` through `crate::models`, but src/models defines \
                 no `lookup`; call a live-row finder, or a model function that takes no \
                 database connection",
            ],
        ),
        (
            "use crate::models::posts::*;",
            vec!["imports from `crate::models` with a glob; name each live-row finder"],
        ),
        (
            "use crate::{dtos::v1::*, models::{labels, posts::*}};",
            vec!["imports from `crate::models` with a glob; name each live-row finder"],
        ),
    ] {
        assert_eq!(public_findings(source), expected, "{source}");
    }
}

#[test]
fn live_row_scanner_accepts_live_finders_and_other_code() {
    let source = r#"
        //! Mentions Entity::find() and query_all_raw in prose.
        use crate::models::{posts, users::{self, Model as User}, content, read_counts::record};
        use crate::{dtos::v1::posts::Post, models::labels};
        use crate::dtos::v1::*;
        use crate::dtos::v1::error::{Error, ErrorCode};
        /* Statement::from_string in a comment */
        async fn show(db: &C, id: i64) -> Response {
            let _ = "Entity::find() in a string";
            let post = posts::Model::live_page(db, 20).await?;
            let user = users::Model::find_live_by_email(db, "a").await?;
            let other = User::live_by_id(db, 1).await?;
            let _ = crate::models::labels::Model::list_live_for_post(db, &post).await?;
            let _ = labels::Model::list_live::<C>(db).await?;
            let body = content::serve_content(&cache, &resanitized, &post);
            record(&counts, post.id);
            let name = post.display_name();
            let first = items.iter().find(|item| item.id == id);
            let summary = PostSummary::from(&post);
            let detail = Post::from(&post);
            let digits = "5".parse::<u8>();
            let _ = Error::new(ErrorCode::NotFound);
            let _ = v1_params::id(&raw).map_err(Error::from);
            let _ = format!("{}", posts::Model::TABLE);
            first.map(Item::from)
        }
        impl From<&posts::Model> for PostSummary { fn from(m: &posts::Model) -> Self { todo!() } }
    "#;
    assert_eq!(public_findings(source), Vec::<String>::new());
}

/// What [`live_finder_report`] finds in a model file `source`.
fn finder_findings(source: &str) -> Vec<String> {
    live_finder_report(Path::new("models/x.rs"), source)
}

/// The report line of [`live_finder_report`] for the finder `name`.
fn unfiltered(name: &str) -> String {
    format!(
        "src/models/x.rs: `{name}` is named as a live-row finder but does not filter the \
         trash: add `deleted_at IS NULL` (Column::DeletedAt.is_null()) or call another live-row \
         finder"
    )
}

#[test]
fn live_finder_scanner_requires_a_trash_filter() {
    let accepted = r##"
        impl Model {
            pub async fn find_live_by_email<C>(db: &C, email: &str) -> ModelResult<Self>
            where
                C: ConnectionTrait,
            {
                Entity::find().filter(Column::DeletedAt.is_null()).one(db).await
            }
            pub async fn live_count(db: &C) -> Result<i64> {
                let sql = "SELECT count(*) FROM posts WHERE deleted_at IS NULL";
                query(db, sql).await
            }
            pub async fn live_joined(db: &C) -> Result<i64> {
                let sql = r#"SELECT 1 FROM posts p WHERE p."deleted_at"
                             is null"#;
                query(db, sql).await
            }
            pub async fn live_by_expr(db: &C) -> Result<Vec<Self>> {
                Entity::find().filter(Expr::col((Posts, Column::DeletedAt)).is_null()).all(db).await
            }
            pub async fn live_page(db: &C) -> Result<Vec<Self>> {
                Self::live_rows().all(db).await
            }
            pub async fn list_live<C>(db: &C) -> Result<Vec<Self>> {
                Self::live_rows_of::<C>(db).all(db).await
            }
            fn live_rows() -> Select<Entity> { Entity::find().filter(Column::DeletedAt.is_null()) }
            fn find_by_pid(db: &C) -> Result<Self> { Entity::find().one(db) }
        }
        trait Live { fn live() -> Select<Self>; }
    "##;
    assert_eq!(finder_findings(accepted), Vec::<String>::new());
    let flagged = r#"
        impl Model {
            pub async fn live_page(db: &C) -> Result<Vec<Self>> {
                // deleted_at is forgotten here
                Entity::find().all(db).await
            }
            fn alive() {}
            pub fn list_live(db: &C) -> Select<Entity> { Entity::find() }
            pub async fn live_trash(db: &C) -> Result<Vec<Self>> {
                Entity::find().filter(Column::DeletedAt.is_not_null()).all(db).await
            }
            pub async fn live_sql_trash(db: &C) -> Result<i64> {
                query(db, "SELECT count(*) FROM posts WHERE deleted_at IS NOT NULL").await
            }
            pub async fn find_live_rows(db: &C) -> Result<Vec<Self>> {
                let live = Entity::find();
                live.all(db).await
            }
        }
    "#;
    assert_eq!(
        finder_findings(flagged),
        [
            unfiltered("live_page"),
            unfiltered("list_live"),
            unfiltered("live_trash"),
            unfiltered("live_sql_trash"),
            unfiltered("find_live_rows"),
        ]
    );
}
