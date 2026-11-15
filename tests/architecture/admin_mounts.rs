//! The admin mount lint of ADR 0007 §5 ("Deny by default").
//!
//! Every `/api/admin` route is declared, with its requirement, in the admin
//! route table, `table()` of `src/controllers/admin/routes.rs`, whose
//! `routes()` is the one builder of the admin router; `App::routes` mounts
//! that router. `tests/requests/admin_routes.rs` compares what `App::routes`
//! mounts under `/api/admin` with the table. The other hooks that can add a
//! route to the served application never reach `App::routes`:
//! `before_routes`, whose router Loco starts from (the admin UI and the
//! fallback are there), `after_routes`, `serve`, and an initializer's
//! `after_routes`. A route added in one of them would be served with no
//! guard, no `Cache-Control: no-store` and no body limit, and that test
//! would not see it. This lint closes the gap by reading the source text of
//! `src/`. Outside the files of [`ALLOWED_FILES`], the table's own and
//! `src/domain/api_path.rs`, which defines the admin API's prefix and sorts
//! paths by it, the code of `src/` must not:
//!
//! - name `ADMIN_API_PREFIX` (also inline in a format string, as in
//!   `format!("{ADMIN_API_PREFIX}/x")`), or hold a string literal that is
//!   `/api` or `/api/`, or starts with `/api/admin`: no path under the admin
//!   API, and no router nested at `/api`, is built anywhere else;
//! - reach the table's router builders, `routes::routes` and
//!   `routes::build`, by path or through a `use` (a group or a glob counts
//!   too), except in one place: the body of `fn routes` in `src/app.rs`,
//!   which is `App::routes`. `routes::table`, which only lists the routes,
//!   stays free, for a check that walks the table.
//!
//! It also fails when `App::routes` no longer reaches `routes::routes`, so
//! the lint cannot pass on an `app.rs` it did not read.
//!
//! The `#[cfg(test)]` items of `src/` are skipped: they are not built into
//! the served application, and the unit tests of the middleware there serve
//! admin paths on routers of their own.
//!
//! Limits: like the rest of `tests/architecture.rs`, this is a text check
//! that catches honest mistakes, not a determined bypass. It does not see a
//! path that only the program computes (`concat!` or `format!` of pieces
//! such as `"/api"` and `"/admin"` from elsewhere, an escape inside the
//! literal), a module alias (`use ...::routes as r`), a wildcard route that
//! also matches admin paths (`/{*path}`), or a router built from the
//! table's routes by some other function; review covers those.

use std::{fs, ops::Range, path::Path};

use super::{
    block_comment_end, char_literal_end, line_comment_end, raw_string_end, rust_files, src_dir,
    string_end,
};

/// The files, relative to `src/`, that may name the admin API's paths and
/// its router builders: the route table, and the module that defines the
/// prefix and sorts request paths by it.
const ALLOWED_FILES: &[&str] = &["controllers/admin/routes.rs", "domain/api_path.rs"];

/// The file, relative to `src/`, whose `fn routes` (`App::routes`) mounts
/// the admin router.
const APP_FILE: &str = "app.rs";

/// The functions of the route table's module that build a router from a
/// table: `routes()` (the application's) and `build(table)`.
const ROUTER_BUILDERS: &[&str] = &["routes", "build"];

/// One token of a Rust source for this lint. Unlike the purity check's
/// tokens, a string literal stays, as one token with its text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// An identifier or keyword.
    Word(String),
    /// The text of a string literal (raw or not), escapes left as written.
    Literal(String),
    /// `::`.
    PathSep,
    /// Any other punctuation character.
    Punct(char),
}

/// The pieces of `source`, each with its line (from 1). Comments, character
/// literals, lifetimes' quotes and numbers are dropped.
fn pieces(source: &str) -> Vec<(Piece, usize)> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut line = 1;
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        let skipped = match (character, chars.get(index + 1)) {
            ('/', Some('/')) => Some((line_comment_end(&chars, index), None)),
            ('/', Some('*')) => Some((block_comment_end(&chars, index), None)),
            ('"', _) => {
                let end = string_end(&chars, index);
                let closed = end - 1 > index && chars[end - 1] == '"';
                let content_end = if closed { end - 1 } else { end };
                Some((end, Some(chars[index + 1..content_end].iter().collect())))
            }
            ('\'', _) => char_literal_end(&chars, index).map(|end| (end, None)),
            ('r' | 'b', _) => raw_string_end(&chars, index)
                .map(|end| (end, Some(raw_literal_text(&chars, index, end)))),
            _ => None,
        };
        if let Some((end, literal)) = skipped {
            if let Some(text) = literal {
                found.push((Piece::Literal(text), line));
            }
            line += chars[index..end].iter().filter(|&&c| c == '\n').count();
            index = end;
        } else if character.is_alphabetic() || character == '_' {
            let start = index;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            found.push((Piece::Word(chars[start..index].iter().collect()), line));
        } else if character == ':' && chars.get(index + 1) == Some(&':') {
            found.push((Piece::PathSep, line));
            index += 2;
        } else {
            if character == '\n' {
                line += 1;
            } else if !character.is_whitespace() && !character.is_ascii_digit() {
                found.push((Piece::Punct(character), line));
            }
            index += 1;
        }
    }
    found
}

/// The text of the raw string literal `chars[start..end]` (`r"..."`,
/// `r#"..."#`, `br"..."`).
fn raw_literal_text(chars: &[char], start: usize, end: usize) -> String {
    let quote = (start..end)
        .find(|&index| chars[index] == '"')
        .unwrap_or(end);
    let hashes = quote - start - if chars[start] == 'b' { 2 } else { 1 };
    let closed = end >= quote + 2 + hashes && chars[end - hashes - 1] == '"';
    let content_end = if closed { end - hashes - 1 } else { end };
    chars[(quote + 1).min(content_end)..content_end]
        .iter()
        .collect()
}

/// Whether the piece at `index` is the word `word`.
fn is_word(pieces: &[Piece], index: usize, word: &str) -> bool {
    matches!(pieces.get(index), Some(Piece::Word(found)) if found == word)
}

/// The index of the bracket that closes the one at `pieces[open]`, with
/// `()`, `[]` and `{}` counted alike, or the last index when it never
/// closes.
fn matching(pieces: &[Piece], open: usize) -> usize {
    let mut level = 0usize;
    for (index, piece) in pieces.iter().enumerate().skip(open) {
        match piece {
            Piece::Punct('(' | '[' | '{') => level += 1,
            Piece::Punct(')' | ']' | '}') => {
                level = level.saturating_sub(1);
                if level == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    pieces.len().saturating_sub(1)
}

/// The end (inclusive) of the item that starts at `start`, past its outer
/// attributes: its `{ ... }` body, or the `;` that ends it first.
fn item_end(pieces: &[Piece], start: usize) -> usize {
    let mut index = start;
    while index < pieces.len() {
        match &pieces[index] {
            Piece::Punct('{') => return matching(pieces, index),
            Piece::Punct(';') => return index,
            // An attribute, a parameter list or an array type: skip it
            // whole, so that a `;` or `{` inside does not end the item.
            Piece::Punct('(' | '[') => index = matching(pieces, index) + 1,
            _ => index += 1,
        }
    }
    pieces.len().saturating_sub(1)
}

/// The piece ranges of the `#[cfg(test)]` items of `pieces`, each from its
/// attribute to the end of the item; the whole file under an inner
/// `#![cfg(test)]`.
fn test_items(pieces: &[Piece]) -> Vec<Range<usize>> {
    let attribute = |index: usize, inner: bool| -> Option<usize> {
        let mut cursor = index + 1;
        if inner {
            if pieces.get(cursor) != Some(&Piece::Punct('!')) {
                return None;
            }
            cursor += 1;
        }
        let expected = [
            Piece::Punct('['),
            Piece::Word("cfg".to_owned()),
            Piece::Punct('('),
            Piece::Word("test".to_owned()),
            Piece::Punct(')'),
            Piece::Punct(']'),
        ];
        (pieces.get(cursor..cursor + expected.len()) == Some(&expected[..]))
            .then_some(cursor + expected.len())
    };
    let mut ranges = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        if piece != &Piece::Punct('#') {
            continue;
        }
        if attribute(index, true).is_some() {
            // One range, the whole file (not a list of its indices).
            return std::iter::once(0..pieces.len()).collect();
        }
        if let Some(after) = attribute(index, false) {
            ranges.push(index..item_end(pieces, after) + 1);
        }
    }
    ranges
}

/// The piece range of the body of `fn <name>`, its first one in `pieces`,
/// from its `{` to its `}`; `None` when there is no such function.
fn function_body(pieces: &[Piece], name: &str) -> Option<Range<usize>> {
    let start = (0..pieces.len()).find(|&index| {
        is_word(pieces, index, "fn")
            && is_word(pieces, index + 1, name)
            && matches!(pieces.get(index + 2), Some(Piece::Punct('(' | '<')))
    })?;
    let open = start
        + pieces[start..]
            .iter()
            .position(|piece| piece == &Piece::Punct('('))?;
    let parameters_end = matching(pieces, open);
    let body = parameters_end
        + 1
        + pieces[parameters_end + 1..]
            .iter()
            .position(|piece| piece == &Piece::Punct('{'))?;
    Some(body..matching(pieces, body) + 1)
}

/// Whether a string literal names a path under the admin API, or the API
/// root that a nested router would mount it under.
fn is_admin_path(text: &str) -> bool {
    text == "/api" || text == "/api/" || text.starts_with("/api/admin")
}

/// Whether a string literal names `ADMIN_API_PREFIX` as an inline argument
/// of a format string, as `format!("{ADMIN_API_PREFIX}/x")` does.
fn formats_the_prefix(text: &str) -> bool {
    text.contains("{ADMIN_API_PREFIX")
}

/// The router builders that the path or `use` at `pieces[index]`
/// (`routes :: ...`) reaches: `routes::build`, `routes::routes`, and in a
/// `use` group or glob each builder it names, `*` for a glob.
fn builders_reached(pieces: &[Piece], index: usize) -> Vec<String> {
    if !is_word(pieces, index, "routes") || pieces.get(index + 1) != Some(&Piece::PathSep) {
        return Vec::new();
    }
    match pieces.get(index + 2) {
        Some(Piece::Word(name)) if ROUTER_BUILDERS.contains(&name.as_str()) => vec![name.clone()],
        Some(Piece::Punct('*')) => vec!["*".to_owned()],
        Some(Piece::Punct('{')) => pieces[index + 3..matching(pieces, index + 2)]
            .iter()
            .filter_map(|piece| match piece {
                Piece::Word(name) if ROUTER_BUILDERS.contains(&name.as_str()) => Some(name.clone()),
                Piece::Punct('*') => Some("*".to_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// What one file of `src/` breaks of the rule that only the admin route
/// table builds admin routes, one report line per finding. `relative` is
/// the path below `src/`.
fn mount_report(relative: &Path, source: &str) -> Vec<String> {
    if ALLOWED_FILES
        .iter()
        .any(|allowed| relative == Path::new(allowed))
    {
        return Vec::new();
    }
    let located = pieces(source);
    let lines: Vec<usize> = located.iter().map(|(_, line)| *line).collect();
    let pieces: Vec<Piece> = located.into_iter().map(|(piece, _)| piece).collect();
    let tests = test_items(&pieces);
    let in_test = |index: usize| tests.iter().any(|range| range.contains(&index));
    let is_app = relative == Path::new(APP_FILE);
    let app_routes = is_app
        .then(|| function_body(&pieces, "routes"))
        .flatten()
        .unwrap_or_default();
    let mut found = Vec::new();
    let mut mounted_by_app_routes = false;
    for (index, piece) in pieces.iter().enumerate() {
        if in_test(index) {
            continue;
        }
        let at = format!("src/{}:{}", relative.display(), lines[index]);
        match piece {
            Piece::Literal(text) if is_admin_path(text) => found.push(format!(
                "{at}: holds the path \"{text}\"; declare an admin route in routes::table() \
                 instead of mounting it here"
            )),
            Piece::Word(name) if name == "ADMIN_API_PREFIX" => found.push(format!(
                "{at}: names `ADMIN_API_PREFIX`; declare an admin route in routes::table() \
                 instead, and sort a request path with `domain::api_path::api_family`"
            )),
            Piece::Literal(text) if formats_the_prefix(text) => found.push(format!(
                "{at}: names `ADMIN_API_PREFIX`; declare an admin route in routes::table() \
                 instead, and sort a request path with `domain::api_path::api_family`"
            )),
            _ => {}
        }
        for builder in builders_reached(&pieces, index) {
            if builder == "routes" && app_routes.contains(&index) {
                mounted_by_app_routes = true;
                continue;
            }
            found.push(format!(
                "{at}: reaches the admin route table's router builder `routes::{builder}`; only \
                 App::routes (`fn routes` in src/app.rs) mounts the admin router, once"
            ));
        }
    }
    if is_app && !mounted_by_app_routes {
        found.push(format!(
            "src/{APP_FILE}: `fn routes` (App::routes) does not reach \
             `controllers::admin::routes::routes()`, which mounts the admin router"
        ));
    }
    found
}

#[test]
fn admin_routes_are_mounted_only_by_the_admin_route_table() {
    let src = src_dir();
    for file in ALLOWED_FILES.iter().chain([&APP_FILE]) {
        assert!(
            src.join(file).is_file(),
            "src/{file} is named in tests/architecture/admin_mounts.rs but does not exist; \
             update the lint"
        );
    }
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let report: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let relative = file.strip_prefix(&src).expect("below src/");
            mount_report(relative, &fs::read_to_string(file).expect("read source"))
        })
        .collect();
    assert!(
        report.is_empty(),
        "Every /api/admin route is one entry of the admin route table, \
         src/controllers/admin/routes.rs, which App::routes mounts (ADR 0007 §5, recipe 10 of \
         .claude/skills/backend3/SKILL.md); a route mounted in another hook is served without \
         its guard:\n{}",
        report.join("\n")
    );
}

// The lint itself is tested on small samples, so a bug in it cannot make the
// real check pass silently.

/// The findings of `source` as the file `file` below `src/`, without the
/// file and line and without the advice after the first `;`.
fn findings(file: &str, source: &str) -> Vec<String> {
    mount_report(Path::new(file), source)
        .into_iter()
        .map(|finding| {
            let without_place = finding
                .split_once(": ")
                .map_or(finding.as_str(), |(_, rest)| rest);
            without_place
                .split(';')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// An `app.rs` whose `fn routes` mounts the admin router as the real one
/// does, with `before` added before it.
fn app_with(before: &str) -> String {
    format!(
        "{before}\nimpl Hooks for App {{\n    fn routes(_ctx: &AppContext) -> AppRoutes {{\n        \
         AppRoutes::with_default_routes().add_route(controllers::admin::routes::routes())\n    \
         }}\n}}\n"
    )
}

#[test]
fn admin_mount_lint_flags_each_way_around_the_table() {
    let path = |text: &str| format!("holds the path \"{text}\"");
    let prefix = "names `ADMIN_API_PREFIX`".to_owned();
    let builder =
        |name: &str| format!("reaches the admin route table's router builder `routes::{name}`");
    for (file, source, expected) in [
        // A route in another hook of `App`, as a reviewer's mutation added it.
        (
            "app.rs",
            app_with(
                "async fn before_routes(ctx: &AppContext) -> Result<AxumRouter<AppContext>> {\n    \
                 Ok(AxumRouter::new().route(\"/api/admin/sneaky\", get(|| async { \"x\" })))\n}",
            ),
            vec![path("/api/admin/sneaky")],
        ),
        (
            "app.rs",
            app_with("fn f(router: Router) -> Router { router.nest(\"/api\", api()) }"),
            vec![path("/api")],
        ),
        (
            "app.rs",
            app_with("const SNEAKY: &str = r#\"/api/admin/x\"#;"),
            vec![path("/api/admin/x")],
        ),
        (
            "controllers/other.rs",
            "fn f(r: Router) -> Router { r.route(&format!(\"{ADMIN_API_PREFIX}/x\"), get(h)) }"
                .to_owned(),
            vec![prefix.clone()],
        ),
        (
            "controllers/other.rs",
            "use crate::domain::api_path::ADMIN_API_PREFIX;".to_owned(),
            vec![prefix],
        ),
        // The table's builders, mounted a second time or for another table.
        (
            "app.rs",
            app_with(
                "async fn before_routes(ctx: &AppContext) -> Result<AxumRouter<AppContext>> {\n    \
                 let admin = controllers::admin::routes::routes();\n    todo!()\n}",
            ),
            vec![builder("routes")],
        ),
        (
            "initializers/sneaky.rs",
            "fn f() -> Routes { crate::controllers::admin::routes::build(vec![]) }".to_owned(),
            vec![builder("build")],
        ),
        (
            "initializers/sneaky.rs",
            "use crate::controllers::admin::routes::{AdminRoute, build, table};".to_owned(),
            vec![builder("build")],
        ),
        (
            "initializers/sneaky.rs",
            "use crate::controllers::admin::routes::*;".to_owned(),
            vec![builder("*")],
        ),
        (
            "controllers/admin/mod.rs",
            "use super::routes::{self, routes as admin_routes};".to_owned(),
            vec![builder("routes")],
        ),
        // `App::routes` that does not mount the admin router at all.
        (
            "app.rs",
            "impl Hooks for App { fn routes(_ctx: &AppContext) -> AppRoutes { \
             AppRoutes::with_default_routes() } }"
                .to_owned(),
            vec![
                "`fn routes` (App::routes) does not reach \
                 `controllers::admin::routes::routes()`, which mounts the admin router"
                    .to_owned(),
            ],
        ),
    ] {
        assert_eq!(findings(file, &source), expected, "{file}:\n{source}");
    }
}

#[test]
fn admin_mount_lint_reports_the_line() {
    let source = app_with("\nfn f() {\n    let _ = \"/api/admin\";\n}");
    assert_eq!(
        mount_report(Path::new("app.rs"), &source),
        [
            "src/app.rs:3: holds the path \"/api/admin\"; declare an admin route in \
             routes::table() instead of mounting it here"
        ]
    );
}

#[test]
fn admin_mount_lint_accepts_the_table_and_other_code() {
    for (file, source) in [
        // `App::routes` mounting the admin router, and other routes.
        ("app.rs", app_with("")),
        (
            "app.rs",
            app_with(
                "async fn before_routes(ctx: &AppContext) -> Result<AxumRouter<AppContext>> {\n    \
                 Ok(AxumRouter::new().merge(admin_ui::router()).fallback(not_found::fallback))\n}",
            ),
        ),
        // Prose, comments and other paths.
        (
            "controllers/admin/auth.rs",
            "//! `POST /api/admin/auth/login`, mounted by `routes::build`.\n\
             /// GET \"/api/admin/auth/me\"\n/* ADMIN_API_PREFIX */\n\
             fn f() -> &'static str { let _ = '\"'; \"/api/v1/posts\" }"
                .to_owned(),
        ),
        (
            "controllers/v1/mod.rs",
            "fn f() { let _ = (\"/admin\", \"/api/v1\", \"/apix\", \"/api/v2/admin\"); }"
                .to_owned(),
        ),
        // Sorting a path, listing the table, and the other `routes`.
        (
            "controllers/not_found.rs",
            "use crate::domain::api_path::{ApiFamily, api_family};\n\
             fn f(p: &str) -> bool { api_family(p) == Some(ApiFamily::Admin) }"
                .to_owned(),
        ),
        (
            "controllers/admin/origin.rs",
            "use super::routes::{self, AdminRoute, table};\n\
             fn f() -> usize { routes::table().len() + controllers::v1::routes().len() }\n\
             pub mod routes;"
                .to_owned(),
        ),
        // The allowed files.
        (
            "controllers/admin/routes.rs",
            "pub fn routes() -> Routes { build(table()) }\n\
             fn build() { Routes::new().prefix(ADMIN_API_PREFIX); let _ = \"/api/admin/x\"; }"
                .to_owned(),
        ),
        (
            "domain/api_path.rs",
            "pub const ADMIN_API_PREFIX: &str = \"/api/admin\";".to_owned(),
        ),
        // Unit tests, which are not served: a `#[cfg(test)]` module, and
        // one function after another attribute.
        (
            "controllers/v1/caching.rs",
            "fn served() {}\n#[cfg(test)]\nmod tests {\n    fn f() { Router::new()\
             .route(\"/api/admin/posts\", get(h)); let _ = ADMIN_API_PREFIX; }\n}\n"
                .to_owned(),
        ),
        (
            "controllers/admin/session.rs",
            "#[cfg(test)]\n#[allow(dead_code)]\nfn probe(x: [u8; 2]) { routes::build(vec![]); \
             let _ = \"/api/admin/x\"; }\nfn served() {}\n"
                .to_owned(),
        ),
        (
            "controllers/admin/probe.rs",
            "#![cfg(test)]\nfn f() { let _ = \"/api/admin\"; }".to_owned(),
        ),
    ] {
        assert_eq!(
            findings(file, &source),
            Vec::<String>::new(),
            "{file}:\n{source}"
        );
    }
}

#[test]
fn admin_mount_lint_ends_a_test_item_where_rust_does() {
    // What follows a `#[cfg(test)]` item is served again and checked.
    for source in [
        "#[cfg(test)]\nmod tests { fn f() {} }\nconst A: &str = \"/api/admin\";",
        "#[cfg(test)]\nuse crate::x;\nconst A: &str = \"/api/admin\";",
        "#[cfg(test)]\nfn f(x: [u8; 2]) -> u8 { 0 }\nconst A: &str = \"/api/admin\";",
    ] {
        assert_eq!(
            findings("controllers/other.rs", source),
            ["holds the path \"/api/admin\""],
            "{source}"
        );
    }
}
