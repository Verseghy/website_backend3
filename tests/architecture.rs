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
//! The scan is a text check, so a determined bypass is possible (ADR 0008,
//! Consequences); it exists to catch honest mistakes early.

use std::{
    fs,
    path::{Path, PathBuf},
};

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
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let end = match (chars[i], chars.get(i + 1)) {
            ('/', Some('/')) => Some(line_comment_end(&chars, i)),
            ('/', Some('*')) => Some(block_comment_end(&chars, i)),
            ('"', _) => Some(string_end(&chars, i)),
            ('\'', _) => char_literal_end(&chars, i),
            ('r' | 'b', _) => raw_string_end(&chars, i),
            _ => None,
        };
        if let Some(end) = end {
            for &skipped in &chars[i..end] {
                out.push(if skipped == '\n' { '\n' } else { ' ' });
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

/// The token range of `impl Clock for SystemClock { ... }` in `tokens`, if
/// there is one, from `impl` to the closing brace.
fn system_clock_impl(tokens: &[Token]) -> Option<std::ops::Range<usize>> {
    let start = tokens.windows(SYSTEM_CLOCK_IMPL.len()).position(|window| {
        window
            .iter()
            .zip(SYSTEM_CLOCK_IMPL)
            .all(|(token, word)| ident(Some(token)) == Some(*word))
    })?;
    let mut level = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        match token {
            Token::Punct('{') => level += 1,
            Token::Punct('}') => {
                level = level.saturating_sub(1);
                if level == 0 {
                    return Some(start..index);
                }
            }
            _ => {}
        }
    }
    None
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

// The scanners themselves are tested on small samples, so a bug in one cannot
// make a real check pass silently.

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
