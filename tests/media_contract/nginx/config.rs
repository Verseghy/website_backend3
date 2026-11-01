//! The static half of the `stub_status` checks: where
//! `deploy/media/nginx.conf` names the directive, whatever path or `Host` a
//! request carries (research: DECISIONS R15, D93).
//!
//! The container test asks the running server, so it covers the requests it
//! sends and no others. A configuration that served the page on port 8080
//! under a path it does not try, or in a server that also listens on 8080
//! under a `server_name` it does not send, would pass it. nginx serves the
//! page only where a `stub_status` directive stands, so a reading of the file
//! covers every request at once. [`stub_status_problems`] requires:
//!
//! - one `stub_status` directive in the whole file, inside the server whose
//!   only `listen` is the loopback listener;
//! - two servers, the media server on `8080 default_server` and that one,
//!   and no `listen` anywhere else (a `stream` proxy would need one);
//! - no `include` but the image's `mime.types`, because an included file
//!   could add a server, a listener or the directive out of this reading's
//!   sight;
//! - no `*_pass` directive, because a proxy on 8080 to the loopback listener
//!   would publish the page. The media server serves files only (ADR 0015
//!   §1).
//!
//! [`parse`] reads nginx's configuration syntax the way nginx 1.26 tokenises
//! it (`ngx_conf_read_token` in `src/core/ngx_conf_file.c`): words separated
//! by white space; `;`, `{` and `}`; `#` comments from the start of a word to
//! the end of the line; single- and double-quoted words, in which `;`, `{`
//! and `#` are text; backslash escapes; and `${name}` inside a word. It
//! checks nothing that nginx's own parser checks, such as which directives
//! exist or where they may stand.

use super::{MEDIA_PORT, stub_status};

/// One directive of the configuration: its name, its arguments and, for a
/// block directive such as `server`, the directives of its block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Directive {
    /// The line its name starts on, counted from 1, for messages.
    line: usize,
    /// The first word, such as `listen`.
    name: String,
    /// The other words, unquoted and unescaped as nginx reads them.
    args: Vec<String>,
    /// The directives between `{` and `}`; `None` for one that ends with `;`.
    block: Option<Vec<Directive>>,
}

impl Directive {
    /// The directive as it would be written, without its block, for
    /// messages: `listen 127.0.0.1:8081`.
    fn text(&self) -> String {
        std::iter::once(self.name.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// One statement of nginx's token reader: a directive that ends with `;`,
/// the head of a block, the end of a block, or the end of the file.
///
/// `line` is where the first word starts; `words` is never empty.
enum Statement {
    /// `name args;`
    Simple { line: usize, words: Vec<String> },
    /// `name args {`
    Block { line: usize, words: Vec<String> },
    /// `}`
    Close { line: usize },
    /// The end of the file, between two statements.
    End,
}

/// The reader's position in the file.
struct Reader<'a> {
    /// The characters not read yet.
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    /// The line of the next character, counted from 1.
    line: usize,
}

impl Reader<'_> {
    /// The next character, counting the lines.
    fn read_char(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        if ch == '\n' {
            self.line += 1;
        }
        Some(ch)
    }

    /// The message for a file that ends inside the statement of `words`.
    fn eof(&self, words: &[String]) -> String {
        format!(
            "line {}: unexpected end of file after {:?}, expecting \";\" or \"}}\"",
            self.line,
            words.join(" ")
        )
    }

    /// The directives up to the `}` of a block opened on line `opened`, or,
    /// with `None`, up to the end of the file.
    fn block(&mut self, opened: Option<usize>) -> Result<Vec<Directive>, String> {
        let mut directives = Vec::new();
        loop {
            match (self.statement()?, opened) {
                (Statement::Simple { line, words }, _) => {
                    directives.push(directive(line, words, None));
                }
                (Statement::Block { line, words }, _) => {
                    let block = self.block(Some(line))?;
                    directives.push(directive(line, words, Some(block)));
                }
                (Statement::Close { .. }, Some(_)) | (Statement::End, None) => {
                    return Ok(directives);
                }
                (Statement::Close { line }, None) => {
                    return Err(format!("line {line}: unexpected \"}}\""));
                }
                (Statement::End, Some(line)) => {
                    return Err(format!(
                        "unexpected end of file: the block opened on line {line} is not closed"
                    ));
                }
            }
        }
    }

    /// The next statement, as `ngx_conf_read_token` returns it.
    fn statement(&mut self) -> Result<Statement, String> {
        let mut words = Vec::new();
        let mut first_line = self.line;
        loop {
            // The start of a word.
            let line = self.line;
            let Some(ch) = self.read_char() else {
                return if words.is_empty() {
                    Ok(Statement::End)
                } else {
                    Err(self.eof(&words))
                };
            };
            if words.is_empty() {
                first_line = line;
            }
            match ch {
                ' ' | '\t' | '\r' | '\n' => {}
                ';' | '{' if words.is_empty() => {
                    return Err(format!("line {line}: unexpected \"{ch}\""));
                }
                ';' => {
                    return Ok(Statement::Simple {
                        line: first_line,
                        words,
                    });
                }
                '{' => {
                    return Ok(Statement::Block {
                        line: first_line,
                        words,
                    });
                }
                '}' if words.is_empty() => return Ok(Statement::Close { line }),
                '}' => {
                    return Err(format!(
                        "line {line}: unexpected \"}}\" after {:?}",
                        words.join(" ")
                    ));
                }
                '#' => {
                    // A comment runs to the end of the line, also in the
                    // middle of a directive.
                    while self.read_char().is_some_and(|ch| ch != '\n') {}
                }
                '"' | '\'' => {
                    words.push(self.quoted(ch, &words)?);
                    // nginx wants white space, `;`, `{` or `)` after the
                    // closing quote; `)` then starts a word of its own, as
                    // in `if ($a = "b")`.
                    match self.chars.peek().copied() {
                        None | Some(' ' | '\t' | '\r' | '\n' | ')') => {}
                        Some(';') => {
                            self.read_char();
                            return Ok(Statement::Simple {
                                line: first_line,
                                words,
                            });
                        }
                        Some('{') => {
                            self.read_char();
                            return Ok(Statement::Block {
                                line: first_line,
                                words,
                            });
                        }
                        Some(other) => {
                            return Err(format!(
                                "line {}: unexpected \"{other}\" after a quoted word",
                                self.line
                            ));
                        }
                    }
                }
                _ => {
                    let (word, end) = self.unquoted(ch, &words)?;
                    words.push(word);
                    match end {
                        ';' => {
                            return Ok(Statement::Simple {
                                line: first_line,
                                words,
                            });
                        }
                        '{' => {
                            return Ok(Statement::Block {
                                line: first_line,
                                words,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// The rest of a word quoted with `quote`, unescaped; the closing quote
    /// is read too.
    fn quoted(&mut self, quote: char, words: &[String]) -> Result<String, String> {
        let mut raw = String::new();
        let mut escaped = false;
        loop {
            let Some(ch) = self.read_char() else {
                return Err(self.eof(words));
            };
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                return Ok(unescape(&raw));
            }
            raw.push(ch);
        }
    }

    /// The rest of an unquoted word that starts with `first`, unescaped, and
    /// the character that ended it: white space, `;` or `{`.
    fn unquoted(&mut self, first: char, words: &[String]) -> Result<(String, char), String> {
        let mut raw = String::from(first);
        let mut escaped = first == '\\';
        let mut variable = first == '$';
        loop {
            let Some(ch) = self.read_char() else {
                let mut words = words.to_vec();
                words.push(raw);
                return Err(self.eof(&words));
            };
            if escaped {
                escaped = false;
                raw.push(ch);
                continue;
            }
            // `${name}`: the `{` after `$` belongs to the word.
            if ch == '{' && variable {
                raw.push(ch);
                continue;
            }
            variable = false;
            match ch {
                ' ' | '\t' | '\r' | '\n' | ';' | '{' => return Ok((unescape(&raw), ch)),
                '\\' => escaped = true,
                '$' => variable = true,
                _ => {}
            }
            raw.push(ch);
        }
    }
}

/// A directive from the words of one statement; the reader never returns a
/// statement without words.
fn directive(line: usize, mut words: Vec<String>, block: Option<Vec<Directive>>) -> Directive {
    let name = words.remove(0);
    Directive {
        line,
        name,
        args: words,
        block,
    }
}

/// A word as nginx copies it: `\"`, `\'` and `\\` stand for the character,
/// `\t`, `\r` and `\n` for the control characters, and any other backslash
/// stays.
fn unescape(raw: &str) -> String {
    let mut word = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let escaped = match chars.peek() {
                Some(&quoted @ ('"' | '\'' | '\\')) => Some(quoted),
                Some('t') => Some('\t'),
                Some('r') => Some('\r'),
                Some('n') => Some('\n'),
                _ => None,
            };
            if let Some(escaped) = escaped {
                chars.next();
                word.push(escaped);
                continue;
            }
        }
        word.push(ch);
    }
    word
}

/// Reads a whole nginx configuration file into its directives.
///
/// - `text`: the file.
///
/// # Errors
///
/// A message with the line where the text leaves nginx's syntax: an
/// unclosed block or quote, a `}` with no block open, a directive without
/// `;`, or a character right after a closing quote.
pub(super) fn parse(text: &str) -> Result<Vec<Directive>, String> {
    Reader {
        chars: text.chars().peekable(),
        line: 1,
    }
    .block(None)
}

/// The only file that the configuration may include: the image's table of
/// file types, which holds a `types` block and nothing else.
const MIME_TYPES: &str = "/etc/nginx/mime.types";

/// Calls `visit` with every directive of `directives`, at any depth, and its
/// enclosing directives from the outermost in.
fn walk<'a>(
    directives: &'a [Directive],
    ancestors: &mut Vec<&'a Directive>,
    visit: &mut impl FnMut(&[&'a Directive], &'a Directive),
) {
    for directive in directives {
        visit(ancestors, directive);
        if let Some(block) = &directive.block {
            ancestors.push(directive);
            walk(block, ancestors, visit);
            ancestors.pop();
        }
    }
}

/// The `listen` lines of a server block, each as it is written.
fn listens(server: &Directive) -> Vec<String> {
    server
        .block
        .iter()
        .flatten()
        .filter(|directive| directive.name == "listen")
        .map(Directive::text)
        .collect()
}

/// What in `config` could serve the `stub_status` page anywhere but on the
/// loopback listener: one message per breach, each naming its line. See the
/// module documentation for the rules.
///
/// - `config`: the directives of `deploy/media/nginx.conf`, from [`parse`].
pub(super) fn stub_status_problems(config: &[Directive]) -> Vec<String> {
    let media = format!("listen {MEDIA_PORT} default_server");
    let loopback = format!("listen 127.0.0.1:{}", stub_status::PORT);
    let mut problems = Vec::new();
    let mut servers = Vec::new();
    let mut pages = Vec::new();
    walk(config, &mut Vec::new(), &mut |ancestors, directive| {
        let names: Vec<&str> = ancestors.iter().map(|outer| outer.name.as_str()).collect();
        let line = directive.line;
        match directive.name.as_str() {
            "server" if names == ["http"] && directive.block.is_some() => servers.push(directive),
            "listen" if names == ["http", "server"] => {}
            "listen" => problems.push(format!(
                "line {line}: `{}` in {names:?}: only the two http servers may listen",
                directive.text()
            )),
            "include" if directive.args == [MIME_TYPES] => {}
            "include" => problems.push(format!(
                "line {line}: `{}`: only {MIME_TYPES} may be included; another file could add a \
                 server, a listener or stub_status out of this check's sight",
                directive.text()
            )),
            // The server it stands in, if that is one of the http servers.
            "stub_status" => pages.push((
                names.starts_with(&["http", "server"]).then(|| ancestors[1]),
                directive,
            )),
            name if name.ends_with("_pass") => problems.push(format!(
                "line {line}: `{}`: the media server serves files only, and a proxy on port \
                 {MEDIA_PORT} could publish the loopback listener",
                directive.text()
            )),
            _ => {}
        }
    });
    let mut found: Vec<Vec<String>> = servers.iter().map(|server| listens(server)).collect();
    found.sort_unstable();
    let mut expected = vec![vec![media.clone()], vec![loopback.clone()]];
    expected.sort_unstable();
    if found != expected {
        problems.push(format!(
            "the http block must hold two servers, one with the single line `{media}` and one \
             with the single line `{loopback}`, so that the stub_status server shares no port \
             with the media server; the servers' listen lines are {found:?}"
        ));
    }
    let in_loopback_server = |server: Option<&Directive>| {
        server.is_some_and(|server| listens(server) == [loopback.as_str()])
    };
    match pages.as_slice() {
        [(server, _)] if in_loopback_server(*server) => {}
        _ => problems.push(format!(
            "stub_status must stand exactly once, inside the server whose only listen line is \
             `{loopback}`; it stands on lines {:?}",
            pages
                .iter()
                .map(|(_, directive)| directive.line)
                .collect::<Vec<_>>()
        )),
    }
    problems
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The deployed configuration, byte for byte.
    const CONFIG: &str = include_str!("../../../deploy/media/nginx.conf");

    /// `directives` with every line number set to 0, to compare shapes.
    fn shapes(directives: Vec<Directive>) -> Vec<Directive> {
        directives
            .into_iter()
            .map(|directive| Directive {
                line: 0,
                block: directive.block.map(shapes),
                ..directive
            })
            .collect()
    }

    /// A directive without a line number.
    fn simple(name: &str, args: &[&str]) -> Directive {
        Directive {
            line: 0,
            name: name.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            block: None,
        }
    }

    /// A block directive without a line number.
    fn block(name: &str, args: &[&str], directives: Vec<Directive>) -> Directive {
        Directive {
            block: Some(directives),
            ..simple(name, args)
        }
    }

    /// The problems of the deployed configuration with `from` replaced by
    /// `to` once; `from` must occur in it.
    fn problems_with(from: &str, to: &str) -> Vec<String> {
        assert!(CONFIG.contains(from), "{from:?} is not in nginx.conf");
        let changed = CONFIG.replacen(from, to, 1);
        stub_status_problems(&parse(&changed).expect("the changed file parses"))
    }

    #[test]
    fn the_config_serves_stub_status_on_the_loopback_listener_only() {
        let config = parse(CONFIG).expect("deploy/media/nginx.conf parses");
        assert_eq!(stub_status_problems(&config), Vec::<String>::new());
    }

    #[test]
    fn the_reader_sees_the_servers_of_the_config() {
        let config = parse(CONFIG).expect("deploy/media/nginx.conf parses");
        let http = config
            .iter()
            .find(|directive| directive.name == "http")
            .expect("an http block");
        let servers: Vec<_> = http
            .block
            .iter()
            .flatten()
            .filter(|directive| directive.name == "server")
            .map(|server| (server.line, listens(server)))
            .collect();
        assert_eq!(servers.len(), 2, "{servers:?}");
        assert_eq!(servers[0].1, ["listen 8080 default_server"]);
        assert_eq!(servers[1].1, ["listen 127.0.0.1:8081"]);
        // The second server's line is the one above its listen line, counted
        // from 1.
        let listen_line = 1 + CONFIG
            .lines()
            .position(|text| text.contains("listen 127.0.0.1:8081;"))
            .expect("the loopback listen line");
        assert_eq!(servers[1].0, listen_line - 1, "{servers:?}");
    }

    #[test]
    fn a_stub_status_that_a_request_on_the_media_port_reaches_is_refused() {
        for (from, to, message) in [
            // The stub_status server also on 8080, picked by a Host that the
            // container test does not send.
            (
                "listen 127.0.0.1:8081;",
                "listen 127.0.0.1:8081;\n        listen 8080;\n        server_name localhost;",
                "two servers",
            ),
            // The page under another path of the media server.
            (
                "location = /healthz {",
                "location = /nginx_status {\n            stub_status;\n        }\n\n        \
                 location = /healthz {",
                "exactly once",
            ),
            // Moved into the media server.
            (
                "location = /healthz {",
                "location = /healthz {\n            stub_status;",
                "exactly once",
            ),
            // The listener on every address, or with another one beside it.
            ("listen 127.0.0.1:8081;", "listen 8081;", "two servers"),
            (
                "listen 127.0.0.1:8081;",
                "listen 127.0.0.1:8081;\n        listen [::1]:8081;",
                "two servers",
            ),
            // A third server, a stream proxy and a proxy location.
            (
                "    server {\n        listen 127.0.0.1:8081;",
                "    server {\n        listen 8082;\n    }\n\n    server {\n        \
                 listen 127.0.0.1:8081;",
                "two servers",
            ),
            (
                "events {",
                "stream {\n    server {\n        listen 9000;\n        \
                 proxy_pass 127.0.0.1:8081;\n    }\n}\n\nevents {",
                "only the two http servers may listen",
            ),
            (
                "location / {\n            return 404;\n        }\n    }\n\n    # The source",
                "location /storage/status {\n            \
                 proxy_pass http://127.0.0.1:8081/stub_status;\n        }\n\n        \
                 location / {\n            return 404;\n        }\n    }\n\n    # The source",
                "a proxy on port 8080",
            ),
            (
                "include /etc/nginx/mime.types;",
                "include /etc/nginx/mime.types;\n    include /etc/nginx/conf.d/*.conf;",
                "only /etc/nginx/mime.types may be included",
            ),
        ] {
            let problems = problems_with(from, to);
            assert!(
                problems.iter().any(|problem| problem.contains(message)),
                "{to:?}: {problems:?}"
            );
        }
    }

    #[test]
    fn a_stub_status_in_a_comment_or_a_string_is_no_directive() {
        for (from, to) in [
            (
                "location = /healthz {",
                "# location = /nginx_status { stub_status; }\n        location = /healthz {",
            ),
            ("return 200 \"ok\\n\";", "return 200 \"stub_status; }\";"),
        ] {
            assert_eq!(problems_with(from, to), Vec::<String>::new(), "{to:?}");
        }
    }

    #[test]
    fn words_quotes_comments_and_blocks_read_as_nginx_reads_them() {
        let text = "a b;\n\
                    c \"d;e\" 'f{g}#h' {\n\
                    \x20 i j # a comment; with { and }\n\
                    \x20   k;\n\
                    \x20 set $x ${y}z;\n\
                    \x20 if ($m = \"GET\") { }\n\
                    }\n\
                    l \"q\\\"r\" s\\;t u\\\\v w\\x \"\\t\";\n";
        assert_eq!(
            parse(text).map(shapes),
            Ok(vec![
                simple("a", &["b"]),
                block(
                    "c",
                    &["d;e", "f{g}#h"],
                    vec![
                        simple("i", &["j", "k"]),
                        simple("set", &["$x", "${y}z"]),
                        block("if", &["($m", "=", "GET", ")"], Vec::new()),
                    ]
                ),
                // `\;` ends no word, and nginx keeps its backslash.
                simple("l", &["q\"r", "s\\;t", "u\\v", "w\\x", "\t"]),
            ])
        );
    }

    #[test]
    fn each_directive_keeps_the_line_of_its_name() {
        let config = parse("a;\n\n# x\nb {\n  c\n  d;\n}\n").expect("parses");
        assert_eq!(config[0].line, 1);
        assert_eq!(config[1].line, 4);
        assert_eq!(config[1].block.as_ref().expect("a block")[0].line, 5);
    }

    #[test]
    fn text_outside_the_syntax_is_refused_with_its_line() {
        for (text, message) in [
            ("a {\n b;\n", "the block opened on line 1 is not closed"),
            ("a;\n}\n", "line 2: unexpected \"}\""),
            ("a b", "unexpected end of file after \"a b\""),
            ("a \"b;\n", "unexpected end of file"),
            ("a \"b\"c;", "unexpected \"c\" after a quoted word"),
            ("a \"b\"};", "unexpected \"}\" after a quoted word"),
            ("\n;", "line 2: unexpected \";\""),
            ("{}", "line 1: unexpected \"{\""),
            ("a b }", "unexpected \"}\" after \"a b\""),
        ] {
            let error = parse(text).expect_err(text);
            assert!(error.contains(message), "{text:?}: {error}");
        }
    }

    #[test]
    fn unescape_keeps_backslashes_that_escape_nothing() {
        assert_eq!(unescape(r#"\"\'\\\t\r\n"#), "\"'\\\t\r\n");
        assert_eq!(unescape(r"\.\|\d\"), r"\.\|\d\");
        assert_eq!(unescape(r"\\\."), r"\\.");
    }

    /// One argument written so that nginx reads it back as `arg`: in double
    /// quotes, with `"` and `\` escaped.
    fn quote(arg: &str) -> String {
        format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
    }

    /// `directives` written out, each followed by `comment` when it is
    /// `Some`.
    fn render(directives: &[Directive], comment: Option<&str>) -> String {
        let mut text = String::new();
        for directive in directives {
            text.push_str(&directive.name);
            for arg in &directive.args {
                text.push(' ');
                text.push_str(&quote(arg));
            }
            match &directive.block {
                Some(block) => {
                    text.push_str(" {\n");
                    text.push_str(&render(block, comment));
                    text.push('}');
                }
                None => text.push(';'),
            }
            if let Some(comment) = comment {
                text.push_str(" #");
                text.push_str(comment);
            }
            text.push('\n');
        }
        text
    }

    /// Directives of up to three levels, with names nginx accepts unquoted
    /// and arguments that hold anything, the syntax's own characters too.
    fn directives() -> impl Strategy<Value = Vec<Directive>> {
        let leaf = ("[a-z_]{1,8}", prop::collection::vec("[ -~\t\n]{0,8}", 0..4)).prop_map(
            |(name, args)| Directive {
                line: 0,
                name,
                args,
                block: None,
            },
        );
        let tree = leaf.prop_recursive(3, 24, 4, |inner| {
            (
                "[a-z_]{1,8}",
                prop::collection::vec("[ -~\t\n]{0,8}", 0..3),
                prop::collection::vec(inner, 0..4),
            )
                .prop_map(|(name, args, block)| Directive {
                    line: 0,
                    name,
                    args,
                    block: Some(block),
                })
        });
        prop::collection::vec(tree, 0..5)
    }

    proptest! {
        #[test]
        fn written_directives_read_back_with_or_without_comments(
            config in directives(),
            comment in proptest::option::of("[^\n]{0,12}"),
        ) {
            let text = render(&config, comment.as_deref());
            prop_assert_eq!(parse(&text).map(shapes), Ok(config));
        }

        #[test]
        fn the_reader_never_panics(text in "\\PC*") {
            let _ = parse(&text).map(|config| stub_status_problems(&config));
        }
    }
}
