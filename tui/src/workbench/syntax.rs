//! Syntax colouring for the preview box.
//!
//! The preview box paints a document as lines ([`state::PreviewView::Text`]);
//! this module decides which runs of each line *are* something — a keyword, a
//! string, a comment — so the box can colour them with the palette in force.
//! It names no colours: a light theme needs different hues than a dark one, so
//! the classifier answers [`Token`] and `theme::Palette::token` decides what
//! that looks like.
//!
//! It is not a parser, and it does not try to be. Reading code needs the
//! *visible shape* of a line to be right — comments, strings, numbers,
//! declarations — and wrong about as little else as possible, cheaply: each
//! line is walked once, carrying only "am I inside a block comment or a
//! multi-line string" across lines, with no backtracking. That is enough for
//! Rust's raw strings, Python's docstrings and JavaScript's template literals,
//! and a file whose language is unknown costs nothing at all
//! ([`Language::Plain`] paints it exactly as it came).
//!
//! [`state::PreviewView::Text`]: crate::workbench::state::PreviewView::Text

/// What a run of text is, for the palette to colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// A reserved word: `fn`, `if`, `SELECT`.
    Keyword,
    /// A type name — a primitive in the language's own vocabulary, or an
    /// identifier capitalized the way types are.
    Type,
    /// A name being called: the word in front of `(`.
    Function,
    /// A string or character literal, including its quotes.
    String,
    /// A numeric literal.
    Number,
    /// A comment, line or block, with its markers.
    Comment,
    /// A declaration marker: Rust's `#[…]` and `name!`, C's `#include`,
    /// Python's `@decorator`, CSS's `@media`.
    Attribute,
    /// A key rather than a value: a JSON member name, a TOML/YAML key, a CSS
    /// property.
    Key,
}

/// One coloured run of a line, as a byte range into [`Line::text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}

/// One line of a previewed document: its text, and the runs of it that are
/// painted in a token's colour.
///
/// The text is kept as one string with ranges rather than a list of owned
/// pieces: a 500 KB log is 500 KB, not a few hundred thousand small
/// allocations, and the ranges stay valid because nothing edits the text after
/// [`highlight`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    text: String,
    spans: Vec<Span>,
}

impl Line {
    /// A line with nothing to colour — what [`Language::Plain`] produces.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// The line as alternating plain and coloured runs, in order — what the
    /// preview box paints.
    pub fn runs(&self) -> Vec<(&str, Option<Token>)> {
        let mut runs = Vec::new();
        let mut at = 0;
        for span in &self.spans {
            if span.start > at {
                runs.push((&self.text[at..span.start], None));
            }
            runs.push((&self.text[span.start..span.end], Some(span.token)));
            at = span.end;
        }
        if at < self.text.len() {
            runs.push((&self.text[at..], None));
        }
        runs
    }
}

/// The language a path is written in, as far as colouring is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    C,
    Cpp,
    Json,
    Toml,
    Yaml,
    Shell,
    Sql,
    Css,
    Html,
    /// Not recognised as code: the text is painted as it is.
    Plain,
}

impl Language {
    /// The language `path` is written in.
    ///
    /// The extension decides, plus the handful of files whose *name* is their
    /// language (`Makefile`, `Dockerfile`). A path with neither is [`Plain`]
    /// rather than guessed at: a wrong guess is worse than no colour.
    pub fn from_path(path: &str) -> Self {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        let extension = lower
            .rsplit_once('.')
            .map_or("", |(_, extension)| extension);
        match extension {
            "rs" => Self::Rust,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "py" | "pyi" => Self::Python,
            "go" => Self::Go,
            "c" | "h" => Self::C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "ino" => Self::Cpp,
            "json" | "jsonc" => Self::Json,
            "toml" => Self::Toml,
            "lock" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "sh" | "bash" | "zsh" | "ksh" => Self::Shell,
            "sql" => Self::Sql,
            "css" | "scss" | "less" => Self::Css,
            "html" | "htm" | "xml" | "svg" | "vue" | "svelte" => Self::Html,
            _ => match lower.as_str() {
                "makefile" | "gnumakefile" | "dockerfile" | "justfile" => Self::Shell,
                ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" | ".zshenv" => Self::Shell,
                _ => Self::Plain,
            },
        }
    }
}

/// Colour every line of `text`, as `path`'s language.
pub fn highlight(path: &str, text: &str) -> Vec<Line> {
    let mut lexer = Lexer::new(Language::from_path(path));
    text.lines().map(|line| lexer.line(line)).collect()
}

/// A line-at-a-time lexer: the state that has to survive into the next line is
/// "inside a block comment" or "inside a multi-line string", and nothing else.
pub struct Lexer {
    language: Language,
    state: State,
}

impl Lexer {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            state: State::Normal,
        }
    }

    /// Classify one line, continuing whatever block comment or multi-line
    /// string the previous line left open.
    pub fn line(&mut self, raw: &str) -> Line {
        let text = expand_tabs(raw);
        let mut scan = Scan {
            text: &text,
            spec: spec(self.language),
            spans: Vec::new(),
            state: std::mem::replace(&mut self.state, State::Normal),
            head: true,
            depth: 0,
            in_tag: false,
        };
        scan.run();
        let spans = scan.spans;
        self.state = scan.state;
        Line { text, spans }
    }
}

/// Where the previous line stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Normal,
    /// Inside a block comment that did not close on its line.
    Block,
    /// Inside a string that may run past its line: what closes it, and whether
    /// a backslash escapes the next character.
    Str {
        close: String,
        escapes: bool,
    },
}

/// How a run of characters is tokenized, once comments are out of the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    CLike,
    /// `#`-comment languages: Python, shell, TOML, YAML.
    Hash,
    Json,
    Sql,
    Css,
    Html,
    Plain,
}

/// What a leading word followed by a punctuation mark means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyStyle {
    None,
    /// `key = value` (TOML).
    Equals,
    /// `key: value` (YAML, CSS properties).
    Colon,
    /// `"key": value` (JSON).
    Json,
}

#[derive(Debug, Clone, Copy)]
struct Spec {
    language: Language,
    family: Family,
    key: KeyStyle,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Keywords are matched without case (SQL).
    loose: bool,
    /// A capitalized identifier reads as a type name (`SessionMeta`).
    capital_types: bool,
    /// `#` starts a comment only at a word boundary, because `$#` is a shell
    /// parameter rather than the start of a comment.
    hash_at_boundary: bool,
}

impl Spec {
    fn is_keyword(&self, word: &str) -> bool {
        if self.loose {
            self.keywords.iter().any(|k| k.eq_ignore_ascii_case(word))
        } else {
            self.keywords.contains(&word)
        }
    }

    fn is_type(&self, word: &str) -> bool {
        if self.loose {
            self.types.iter().any(|t| t.eq_ignore_ascii_case(word))
        } else {
            self.types.contains(&word)
        }
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "yield",
    "union",
    "macro_rules",
];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "str", "usize", "isize", "u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32",
    "i64", "i128", "f32", "f64",
];

const TS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "keyof",
    "let",
    "namespace",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "while",
    "with",
    "yield",
];

const TS_TYPES: &[&str] = &[
    "any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown", "void",
];

const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "while",
    "with",
    "yield",
];

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "case", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return",
    "self", "cls", "try", "while", "with", "yield",
];

const PYTHON_TYPES: &[&str] = &[
    "bool", "bytes", "dict", "float", "int", "list", "object", "set", "str", "tuple", "type",
];

const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "false",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "iota",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "true",
    "type",
    "var",
];

const GO_TYPES: &[&str] = &[
    "any",
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
];

const C_KEYWORDS: &[&str] = &[
    "NULL", "auto", "break", "case", "char", "const", "continue", "default", "do", "double",
    "else", "enum", "extern", "false", "float", "for", "goto", "if", "inline", "int", "long",
    "register", "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch",
    "true", "typedef", "union", "unsigned", "void", "volatile", "while",
];

const CPP_KEYWORDS: &[&str] = &[
    "NULL",
    "alignas",
    "alignof",
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "constexpr",
    "const_cast",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "operator",
    "override",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

const JSON_KEYWORDS: &[&str] = &["false", "null", "true"];

const TOML_KEYWORDS: &[&str] = &["false", "inf", "nan", "true"];

const YAML_KEYWORDS: &[&str] = &[
    "False", "No", "None", "Off", "On", "True", "Yes", "false", "no", "null", "off", "on", "true",
    "yes",
];

const SHELL_KEYWORDS: &[&str] = &[
    "break", "case", "continue", "declare", "do", "done", "elif", "else", "esac", "exit", "export",
    "fi", "for", "function", "if", "in", "local", "readonly", "return", "select", "shift", "then",
    "time", "unset", "until", "while",
];

const SQL_KEYWORDS: &[&str] = &[
    "ALL",
    "ALTER",
    "AND",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASE",
    "CHECK",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "EXISTS",
    "FOREIGN",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIMIT",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "PRIMARY",
    "REFERENCES",
    "RIGHT",
    "ROLLBACK",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TRANSACTION",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
];

/// The rules for one language.
fn spec(language: Language) -> Spec {
    let c_like = Spec {
        language,
        family: Family::CLike,
        key: KeyStyle::None,
        keywords: &[],
        types: &[],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        loose: false,
        capital_types: true,
        hash_at_boundary: false,
    };
    match language {
        Language::Rust => Spec {
            keywords: RUST_KEYWORDS,
            types: RUST_TYPES,
            ..c_like
        },
        Language::TypeScript => Spec {
            keywords: TS_KEYWORDS,
            types: TS_TYPES,
            ..c_like
        },
        Language::JavaScript => Spec {
            keywords: JS_KEYWORDS,
            types: TS_TYPES,
            ..c_like
        },
        Language::Go => Spec {
            keywords: GO_KEYWORDS,
            types: GO_TYPES,
            ..c_like
        },
        Language::C => Spec {
            keywords: C_KEYWORDS,
            ..c_like
        },
        Language::Cpp => Spec {
            keywords: CPP_KEYWORDS,
            ..c_like
        },
        Language::Python => Spec {
            language,
            family: Family::Hash,
            key: KeyStyle::None,
            keywords: PYTHON_KEYWORDS,
            types: PYTHON_TYPES,
            line_comments: &["#"],
            block_comment: None,
            loose: false,
            capital_types: true,
            hash_at_boundary: false,
        },
        Language::Shell => Spec {
            language,
            family: Family::Hash,
            key: KeyStyle::None,
            keywords: SHELL_KEYWORDS,
            types: &[],
            line_comments: &["#"],
            block_comment: None,
            loose: false,
            capital_types: false,
            hash_at_boundary: true,
        },
        Language::Toml => Spec {
            language,
            family: Family::Hash,
            key: KeyStyle::Equals,
            keywords: TOML_KEYWORDS,
            types: &[],
            line_comments: &["#"],
            block_comment: None,
            loose: false,
            capital_types: false,
            hash_at_boundary: false,
        },
        Language::Yaml => Spec {
            language,
            family: Family::Hash,
            key: KeyStyle::Colon,
            keywords: YAML_KEYWORDS,
            types: &[],
            line_comments: &["#"],
            block_comment: None,
            loose: false,
            capital_types: false,
            hash_at_boundary: false,
        },
        Language::Json => Spec {
            language,
            family: Family::Json,
            key: KeyStyle::Json,
            keywords: JSON_KEYWORDS,
            types: &[],
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            loose: false,
            capital_types: false,
            hash_at_boundary: false,
        },
        Language::Sql => Spec {
            language,
            family: Family::Sql,
            key: KeyStyle::None,
            keywords: SQL_KEYWORDS,
            types: &[],
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            loose: true,
            capital_types: false,
            hash_at_boundary: false,
        },
        Language::Css => Spec {
            language,
            family: Family::Css,
            key: KeyStyle::Colon,
            keywords: &[],
            types: &[],
            line_comments: &[],
            block_comment: Some(("/*", "*/")),
            loose: false,
            capital_types: false,
            hash_at_boundary: false,
        },
        Language::Html => Spec {
            language,
            family: Family::Html,
            key: KeyStyle::None,
            keywords: &[],
            types: &[],
            line_comments: &[],
            block_comment: Some(("<!--", "-->")),
            loose: false,
            capital_types: false,
            hash_at_boundary: false,
        },
        Language::Plain => Spec {
            language,
            family: Family::Plain,
            key: KeyStyle::None,
            keywords: &[],
            types: &[],
            line_comments: &[],
            block_comment: None,
            loose: false,
            capital_types: false,
            hash_at_boundary: false,
        },
    }
}

/// The work of one line: where the scanner is, and what it has pushed so far.
struct Scan<'a> {
    text: &'a str,
    spec: Spec,
    spans: Vec<Span>,
    state: State,
    /// Only whitespace has been painted so far, so a key may start here.
    head: bool,
    /// Brace depth, so a CSS property is not mistaken for a selector.
    depth: usize,
    /// Inside an HTML tag, where words are attribute names.
    in_tag: bool,
}

impl Scan<'_> {
    fn run(&mut self) {
        let mut at = 0;
        if self.state != State::Normal {
            match self.resume() {
                Some(after) => at = after,
                None => return,
            }
        }
        while at < self.text.len() {
            let Some(next) = self.step(at) else {
                return;
            };
            debug_assert!(next > at);
            at = next;
        }
    }

    /// Finish the block comment or multi-line string the previous line left
    /// open. `None` when it did not close on this line either.
    fn resume(&mut self) -> Option<usize> {
        let state = std::mem::replace(&mut self.state, State::Normal);
        let text = self.text;
        match state {
            State::Normal => Some(0),
            State::Block => {
                let close = self.spec.block_comment.map_or("*/", |(_, close)| close);
                match text.find(close) {
                    Some(at) => {
                        let end = at + close.len();
                        self.push(0, end, Token::Comment);
                        Some(end)
                    }
                    None => {
                        self.push(0, text.len(), Token::Comment);
                        self.state = State::Block;
                        None
                    }
                }
            }
            State::Str { close, escapes } => match find_close(text, 0, &close, escapes) {
                Some(end) => {
                    self.push(0, end, Token::String);
                    Some(end)
                }
                None => {
                    self.push(0, text.len(), Token::String);
                    self.state = State::Str { close, escapes };
                    None
                }
            },
        }
    }

    /// Classify whatever starts at `at`; `None` means the rest of the line is
    /// done (it was a comment, or a string that continues on the next line).
    fn step(&mut self, at: usize) -> Option<usize> {
        let text = self.text;
        let ch = char_at(text, at)?;
        if ch.is_whitespace() {
            return Some(at + ch.len_utf8());
        }
        if self.starts_line_comment(at) {
            self.push(at, text.len(), Token::Comment);
            return None;
        }
        if let Some((open, close)) = self.spec.block_comment {
            if text[at..].starts_with(open) {
                let body = at + open.len();
                return match text[body..].find(close) {
                    Some(found) => {
                        let end = body + found + close.len();
                        self.push(at, end, Token::Comment);
                        Some(end)
                    }
                    None => {
                        self.push(at, text.len(), Token::Comment);
                        self.state = State::Block;
                        None
                    }
                };
            }
        }
        match self.spec.family {
            Family::Plain => None,
            Family::CLike => self.c_like(at, ch),
            Family::Hash => self.hash(at, ch),
            Family::Json => self.json(at, ch),
            Family::Sql => self.sql(at, ch),
            Family::Css => self.css(at, ch),
            Family::Html => self.html(at, ch),
        }
    }

    fn starts_line_comment(&self, at: usize) -> bool {
        let rest = &self.text[at..];
        let Some(prefix) = self
            .spec
            .line_comments
            .iter()
            .find(|prefix| rest.starts_with(*prefix))
        else {
            return false;
        };
        // In the shell `$#` is a parameter: a `#` after a word is not a comment.
        !(self.spec.hash_at_boundary && *prefix == "#" && !self.at_word_boundary(at))
    }

    fn at_word_boundary(&self, at: usize) -> bool {
        match self.text[..at].chars().next_back() {
            None => true,
            // `$#` is a parameter, so a `$` before the `#` is *not* a boundary.
            Some(previous) => previous.is_whitespace() || ";&|(){}[]".contains(previous),
        }
    }

    // ─── families ──────────────────────────────────────────────────────────

    fn c_like(&mut self, at: usize, ch: char) -> Option<usize> {
        let text = self.text;
        let language = self.spec.language;
        if language == Language::Rust {
            // `r"…"`, `r#"…"#`, `br#"…"#`: a raw string, possibly multi-line,
            // closed by the same number of `#`s it opened with.
            if let Some((prefix, hashes)) = raw_string_open(text, at) {
                let close = format!("\"{}", "#".repeat(hashes));
                return self.string(at, at + prefix, &close, false);
            }
            // `#[derive(…)]`, `#![allow(…)]`: the whole attribute is a marker.
            if ch == '#' && (text[at..].starts_with("#[") || text[at..].starts_with("#![")) {
                return Some(self.attribute(at));
            }
            if ch == '\'' {
                return Some(self.rust_quote(at));
            }
        } else if matches!(language, Language::C | Language::Cpp) && ch == '#' {
            // A preprocessor line is one marker to the end of the line.
            self.push(at, text.len(), Token::Attribute);
            return None;
        }
        match ch {
            '"' => self.string(at, at + 1, "\"", true),
            '\'' if language != Language::Rust => self.string(at, at + 1, "'", true),
            '`' if matches!(language, Language::TypeScript | Language::JavaScript) => {
                self.string(at, at + 1, "`", true)
            }
            '`' if language == Language::Go => self.string(at, at + 1, "`", false),
            c if c.is_ascii_digit() => Some(self.number(at)),
            c if is_ident_start(c) => self.word(at),
            _ => Some(at + ch.len_utf8()),
        }
    }

    fn hash(&mut self, at: usize, ch: char) -> Option<usize> {
        let text = self.text;
        match self.spec.language {
            Language::Python => {
                if ch == '@' && self.head {
                    let end = self.ident_end(at + 1);
                    if end > at + 1 {
                        self.head = false;
                        self.push(at, end, Token::Attribute);
                        return Some(end);
                    }
                }
                if let Some(after) = triple_quote(text, at) {
                    return self.string(at, after, &text[at..after], true);
                }
                if is_ident_start(ch) {
                    // `f"…"`, `rb'…'`: the prefix belongs to the literal.
                    if let Some(quote) = python_string_prefix(text, at) {
                        return match triple_quote(text, quote) {
                            Some(after) => self.string(at, after, &text[quote..after], true),
                            None => {
                                let close = &text[quote..quote + 1];
                                self.string(at, quote + 1, close, true)
                            }
                        };
                    }
                }
            }
            Language::Shell => {
                if ch == '$' {
                    return Some(self.shell_variable(at));
                }
            }
            Language::Toml => {
                if self.head && ch == '[' {
                    let width = if text[at..].starts_with("[[") { 2 } else { 1 };
                    let close = if width == 2 { "]]" } else { "]" };
                    if let Some(found) = text[at..].find(close) {
                        let end = at + found + close.len();
                        self.head = false;
                        self.push(at, end, Token::Type);
                        return Some(end);
                    }
                }
                if let Some(after) = triple_quote(text, at) {
                    return self.string(at, after, &text[at..after], true);
                }
            }
            Language::Yaml => {
                // A list item's dash is punctuation: the key may still follow.
                if self.head && ch == '-' && matches!(char_at(text, at + 1), Some(' ') | None) {
                    return Some(at + 1);
                }
                if self.head && (text[at..].starts_with("---") || text[at..].starts_with("...")) {
                    self.head = false;
                    self.push(at, at + 3, Token::Comment);
                    return Some(at + 3);
                }
            }
            _ => {}
        }
        let as_key = self.head;
        match ch {
            '"' => self.string_keyed(at, at + 1, "\"", true, as_key),
            '\'' => self.string_keyed(at, at + 1, "'", true, as_key),
            c if c.is_ascii_digit() => Some(self.number(at)),
            c if is_ident_start(c) => self.word(at),
            _ => Some(at + ch.len_utf8()),
        }
    }

    fn json(&mut self, at: usize, ch: char) -> Option<usize> {
        match ch {
            '"' => self.string_keyed(at, at + 1, "\"", true, self.spec.key == KeyStyle::Json),
            c if c.is_ascii_digit() || c == '-' => Some(self.number(at)),
            c if is_ident_start(c) => self.word(at),
            _ => Some(at + ch.len_utf8()),
        }
    }

    fn sql(&mut self, at: usize, ch: char) -> Option<usize> {
        match ch {
            '\'' => Some(self.sql_string(at)),
            // Quoted identifiers are names, not values.
            '"' => self.string(at, at + 1, "\"", false),
            '`' => self.string(at, at + 1, "`", false),
            c if c.is_ascii_digit() => Some(self.number(at)),
            c if is_ident_start(c) => self.word(at),
            _ => Some(at + ch.len_utf8()),
        }
    }

    fn css(&mut self, at: usize, ch: char) -> Option<usize> {
        let text = self.text;
        match ch {
            '{' => {
                self.depth += 1;
                Some(at + 1)
            }
            '}' => {
                self.depth = self.depth.saturating_sub(1);
                Some(at + 1)
            }
            '@' => {
                let end = self.ident_end(at + 1);
                if end > at + 1 {
                    self.push(at, end, Token::Attribute);
                    Some(end)
                } else {
                    Some(at + 1)
                }
            }
            // `#fff`, `#1a2b3c` — a colour, as opposed to `#id`.
            '#' => {
                let hex = text[at + 1..]
                    .chars()
                    .take_while(char::is_ascii_hexdigit)
                    .count();
                let next = text[at + 1..].chars().nth(hex);
                if (3..=8).contains(&hex) && !next.is_some_and(is_ident_char) {
                    let end = at + 1 + hex;
                    self.push(at, end, Token::Number);
                    Some(end)
                } else {
                    Some(at + 1)
                }
            }
            '"' => self.string(at, at + 1, "\"", true),
            '\'' => self.string(at, at + 1, "'", true),
            c if c.is_ascii_digit() => Some(self.number(at)),
            c if is_ident_start(c) => self.word(at),
            _ => Some(at + ch.len_utf8()),
        }
    }

    fn html(&mut self, at: usize, ch: char) -> Option<usize> {
        let text = self.text;
        if ch == '<' {
            let mut cursor = at + 1;
            if text[cursor..].starts_with('/') {
                cursor += 1;
            }
            if text[cursor..].starts_with('!') {
                // `<!doctype html>` — a declaration, not a tag with attributes.
                let end = text[at..]
                    .find('>')
                    .map_or(text.len(), |found| at + found + 1);
                self.push(at, end, Token::Attribute);
                self.in_tag = false;
                return Some(end);
            }
            let mut end = cursor;
            while let Some(c) = char_at(text, end) {
                if c.is_alphanumeric() || matches!(c, '-' | ':' | '_') {
                    end += c.len_utf8();
                } else {
                    break;
                }
            }
            self.push(at, end, Token::Keyword);
            self.in_tag = true;
            return Some(end);
        }
        if ch == '>' {
            self.in_tag = false;
            return Some(at + 1);
        }
        if !self.in_tag {
            return Some(at + ch.len_utf8());
        }
        match ch {
            '"' => self.string(at, at + 1, "\"", false),
            '\'' => self.string(at, at + 1, "'", false),
            c if is_ident_start(c) => {
                let end = self.ident_end(at);
                self.push(at, end, Token::Key);
                Some(end)
            }
            _ => Some(at + ch.len_utf8()),
        }
    }

    // ─── tokens ────────────────────────────────────────────────────────────

    /// From `at` (the first character of the identifier) to its end.
    fn ident_end(&self, at: usize) -> usize {
        let text = self.text;
        let mut end = at;
        while let Some(ch) = char_at(text, end) {
            if !is_ident_char(ch) {
                break;
            }
            end += ch.len_utf8();
        }
        end
    }

    /// A string starting at `start` (its opening quote, or a prefix like `r#"`)
    /// whose body starts at `body`.
    fn string(&mut self, start: usize, body: usize, close: &str, escapes: bool) -> Option<usize> {
        self.string_keyed(start, body, close, escapes, false)
    }

    /// [`Self::string`], with `as_key` making a literal that is followed by
    /// the language's key marker (`:` or `=`) a [`Token::Key`] instead — a JSON
    /// member name, a YAML key, a TOML key.
    fn string_keyed(
        &mut self,
        start: usize,
        body: usize,
        close: &str,
        escapes: bool,
        as_key: bool,
    ) -> Option<usize> {
        let text = self.text;
        self.head = false;
        match find_close(text, body, close, escapes) {
            Some(end) => {
                let key = as_key
                    && self
                        .key_marker()
                        .is_some_and(|marker| self.next_nonspace(end) == Some(marker));
                let token = if key { Token::Key } else { Token::String };
                self.push(start, end, token);
                Some(end)
            }
            None => {
                self.push(start, text.len(), Token::String);
                self.state = State::Str {
                    close: close.to_string(),
                    escapes,
                };
                None
            }
        }
    }

    /// A SQL literal: `'…'`, where `''` is an escaped quote rather than the end.
    fn sql_string(&mut self, at: usize) -> usize {
        let text = self.text;
        let mut cursor = at + 1;
        while cursor < text.len() {
            if text[cursor..].starts_with("''") {
                cursor += 2;
                continue;
            }
            if text[cursor..].starts_with('\'') {
                let end = cursor + 1;
                self.push(at, end, Token::String);
                return end;
            }
            cursor += char_at(text, cursor).map_or(1, char::len_utf8);
        }
        self.push(at, text.len(), Token::String);
        self.state = State::Str {
            close: "'".to_string(),
            escapes: false,
        };
        text.len()
    }

    /// Rust's `'`: a character literal, or a lifetime.
    fn rust_quote(&mut self, at: usize) -> usize {
        let text = self.text;
        let mut cursor = at + 1;
        if char_at(text, cursor) == Some('\\') {
            cursor += 1;
        }
        if let Some(ch) = char_at(text, cursor) {
            let after = cursor + ch.len_utf8();
            if char_at(text, after) == Some('\'') {
                let end = after + 1;
                self.push(at, end, Token::String);
                return end;
            }
        }
        // `'a`, `'static`: a lifetime is a name, and reads as one.
        if char_at(text, at + 1).is_some_and(is_ident_start) {
            let end = self.ident_end(at + 1);
            self.push(at, end, Token::Type);
            return end;
        }
        at + 1
    }

    /// A Rust attribute from `#[` to its closing `]`.
    fn attribute(&mut self, at: usize) -> usize {
        let text = self.text;
        match text[at..].find(']') {
            Some(found) => {
                let end = at + found + 1;
                self.push(at, end, Token::Attribute);
                end
            }
            None => {
                self.push(at, text.len(), Token::Attribute);
                text.len()
            }
        }
    }

    fn number(&mut self, at: usize) -> usize {
        let text = self.text;
        let mut end = at;
        // A signed literal (`-1`) starts at its sign, which is not a digit — but
        // only when a digit follows it.
        if matches!(char_at(text, at), Some('-') | Some('+'))
            && char_at(text, at + 1).is_some_and(|c| c.is_ascii_digit())
        {
            end += 1;
        }
        while let Some(ch) = char_at(text, end) {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                end += ch.len_utf8();
                // `1e+9`: the exponent's sign belongs to the literal …
                if matches!(ch, 'e' | 'E') {
                    if let Some(sign) = char_at(text, end) {
                        if matches!(sign, '+' | '-') {
                            end += 1;
                        }
                    }
                }
                continue;
            }
            // … but `1..2` and `x.0` are not decimals: a point is only part of
            // the number when a digit follows it.
            if ch == '.' && char_at(text, end + 1).is_some_and(|c| c.is_ascii_digit()) {
                end += 1;
                continue;
            }
            break;
        }
        self.push(at, end, Token::Number);
        self.head = false;
        // A sign with nothing after it is punctuation, not a number.
        if end == at {
            return at + char_at(text, at).map_or(1, char::len_utf8);
        }
        end
    }

    fn word(&mut self, at: usize) -> Option<usize> {
        let text = self.text;
        let spec = self.spec;
        let head = self.head;
        self.head = false;
        let end = self.ident_end(at);
        // A bare TOML or YAML key may join words with `-` or `.`
        // (`default-members`, `a.b`): the whole thing is one key, not a word
        // followed by punctuation.
        let end = if head && matches!(spec.language, Language::Toml | Language::Yaml) {
            self.joined_end(end)
        } else {
            end
        };
        let word = &text[at..end];
        if spec.is_keyword(word) {
            self.push(at, end, Token::Keyword);
            return Some(end);
        }
        if spec.is_type(word) {
            self.push(at, end, Token::Type);
            return Some(end);
        }
        // A Rust macro's bang is part of its name.
        if spec.language == Language::Rust && char_at(text, end) == Some('!') {
            self.push(at, end + 1, Token::Attribute);
            return Some(end + 1);
        }
        if char_at(text, end) == Some('(') {
            self.push(at, end, Token::Function);
            return Some(end);
        }
        if (head || self.spec.family == Family::Css) && self.key_follows(end) {
            self.push(at, end, Token::Key);
            return Some(end);
        }
        if spec.capital_types && word.starts_with(char::is_uppercase) {
            self.push(at, end, Token::Type);
        }
        Some(end)
    }

    /// `end` extended over `-`- or `.`-joined words (`default-members`).
    fn joined_end(&self, end: usize) -> usize {
        let text = self.text;
        let mut end = end;
        while let Some(ch) = char_at(text, end) {
            if !matches!(ch, '-' | '.') || !char_at(text, end + 1).is_some_and(is_ident_char) {
                break;
            }
            end = self.ident_end(end + 1);
        }
        end
    }

    fn shell_variable(&mut self, at: usize) -> usize {
        let text = self.text;
        let after = at + 1;
        match char_at(text, after) {
            Some('{') => match text[after..].find('}') {
                Some(found) => {
                    let end = after + found + 1;
                    self.push(at, end, Token::Type);
                    end
                }
                None => {
                    self.push(at, text.len(), Token::Type);
                    text.len()
                }
            },
            Some(c) if is_ident_char(c) => {
                let end = self.ident_end(after);
                self.push(at, end, Token::Type);
                end
            }
            _ => after,
        }
    }

    /// Does a key's marker (`=` in TOML, `:` in YAML and a CSS property)
    /// follow `end`?
    fn key_follows(&self, end: usize) -> bool {
        // Outside a block a CSS `a:hover` is a selector, not a property.
        if self.spec.family == Family::Css && self.depth == 0 {
            return false;
        }
        match self.key_marker() {
            Some(marker) => self.next_nonspace(end) == Some(marker),
            None => false,
        }
    }

    /// What turns a word or literal into a key in this language.
    fn key_marker(&self) -> Option<char> {
        match self.spec.key {
            KeyStyle::None => None,
            KeyStyle::Equals => Some('='),
            KeyStyle::Colon | KeyStyle::Json => Some(':'),
        }
    }

    fn next_nonspace(&self, from: usize) -> Option<char> {
        let text = self.text;
        let mut at = from;
        while let Some(ch) = char_at(text, at) {
            if !ch.is_whitespace() {
                return Some(ch);
            }
            at += ch.len_utf8();
        }
        None
    }

    fn push(&mut self, start: usize, end: usize, token: Token) {
        if start >= end {
            return;
        }
        if let Some(last) = self.spans.last_mut() {
            if last.end == start && last.token == token {
                last.end = end;
                return;
            }
        }
        self.spans.push(Span { start, end, token });
    }
}

/// Tabs are the file's, not the terminal's: expanding them here keeps the
/// preview's columns aligned, and a raw `\t` in a painted row would move the
/// cursor to the terminal's own tab stop and tear the frame.
fn expand_tabs(line: &str) -> String {
    if line.contains('\t') {
        line.replace('\t', "    ")
    } else {
        line.to_string()
    }
}

fn char_at(text: &str, at: usize) -> Option<char> {
    text.get(at..)?.chars().next()
}

fn is_ident_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_'
}

fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The end (exclusive) of `close`, searched from `body`, skipping escaped
/// characters when `escapes`.
fn find_close(text: &str, body: usize, close: &str, escapes: bool) -> Option<usize> {
    let mut at = body;
    while at < text.len() {
        if escapes && text[at..].starts_with('\\') {
            at += 1;
            if let Some(ch) = char_at(text, at) {
                at += ch.len_utf8();
            }
            continue;
        }
        if text[at..].starts_with(close) {
            return Some(at + close.len());
        }
        at += char_at(text, at)?.len_utf8();
    }
    None
}

/// `r"`, `r#"`, `br#"`: the byte length of the opening and its `#` count.
fn raw_string_open(text: &str, at: usize) -> Option<(usize, usize)> {
    let bytes = &text.as_bytes()[at..];
    let mut cursor = 0;
    if bytes.first() == Some(&b'b') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'r') {
        return None;
    }
    cursor += 1;
    let mut hashes = 0;
    while bytes.get(cursor) == Some(&b'#') {
        hashes += 1;
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    Some((cursor + 1, hashes))
}

/// Where a Python (or TOML) `"""` / `'''` literal's body starts.
fn triple_quote(text: &str, at: usize) -> Option<usize> {
    if text[at..].starts_with("\"\"\"") || text[at..].starts_with("'''") {
        Some(at + 3)
    } else {
        None
    }
}

/// The quote index of a Python string prefix (`f"`, `rb'`), when `at` is one.
fn python_string_prefix(text: &str, at: usize) -> Option<usize> {
    let mut end = at;
    while let Some(ch) = char_at(text, end) {
        if !ch.is_ascii_alphabetic() {
            break;
        }
        end += 1;
    }
    let prefix = text[at..end].to_ascii_lowercase();
    if !matches!(
        prefix.as_str(),
        "f" | "r" | "b" | "u" | "fr" | "rf" | "br" | "rb"
    ) {
        return None;
    }
    match char_at(text, end) {
        Some('"') | Some('\'') => Some(end),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The runs of one line, with its tokens spelled out — the shape the
    /// assertions read best in.
    fn runs(path: &str, line: &str) -> Vec<(String, Option<Token>)> {
        let lines = highlight(path, line);
        assert_eq!(lines.len(), 1, "one line in, one line out");
        lines[0]
            .runs()
            .into_iter()
            .map(|(text, token)| (text.to_string(), token))
            .collect()
    }

    fn tokens(path: &str, line: &str) -> Vec<(String, Token)> {
        runs(path, line)
            .into_iter()
            .filter_map(|(text, token)| token.map(|token| (text, token)))
            .collect()
    }

    /// The text of a token, wherever it appears, is painted as that token.
    fn has(path: &str, line: &str, text: &str, token: Token) -> bool {
        tokens(path, line)
            .iter()
            .any(|(run, kind)| run == text && *kind == token)
    }

    #[test]
    fn rust_declarations_strings_numbers_and_comments_split_apart() {
        let line = "pub fn main(x: u32) -> String { let s = \"hi\"; 1 } // done";
        assert!(has("a.rs", line, "pub", Token::Keyword));
        assert!(has("a.rs", line, "fn", Token::Keyword));
        assert!(has("a.rs", line, "main", Token::Function));
        assert!(has("a.rs", line, "u32", Token::Type));
        assert!(has("a.rs", line, "\"hi\"", Token::String));
        assert!(has("a.rs", line, "// done", Token::Comment));
        // A capitalized name is a type: `String` is declared nowhere here.
        assert!(has("a.rs", line, "String", Token::Type));
        // Adjacent keywords stay separate runs: the space between them is plain.
        assert!(has("a.rs", "let mut x = 1;", "let", Token::Keyword));
        assert!(has("a.rs", "let mut x = 1;", "mut", Token::Keyword));
    }

    #[test]
    fn a_rust_macro_attribute_and_lifetime_are_marks_rather_than_names() {
        assert!(has(
            "a.rs",
            "#[derive(Debug)]",
            "#[derive(Debug)]",
            Token::Attribute
        ));
        assert!(has(
            "a.rs",
            "println!(\"{x}\");",
            "println!",
            Token::Attribute
        ));
        // `'a` is a lifetime, not an unterminated character literal.
        assert!(has("a.rs", "fn f<'a>(x: &'a str)", "'a", Token::Type));
        assert!(has("a.rs", "let c = 'x';", "'x'", Token::String));
        assert!(has("a.rs", "let n = '\\n';", "'\\n'", Token::String));
    }

    #[test]
    fn a_rust_raw_string_runs_past_its_line() {
        let lines = highlight("a.rs", "let s = r#\"one\ntwo\"#;");
        assert!(lines[0]
            .spans()
            .iter()
            .any(|span| span.token == Token::String));
        assert_eq!(lines[0].text(), "let s = r#\"one");
        assert!(lines[1]
            .spans()
            .iter()
            .any(|span| span.token == Token::String));
        assert_eq!(lines[1].text(), "two\"#;");
        // The `;` after the closing hashes is code again.
        let last = lines[1].spans().last().copied().expect("a span");
        assert_eq!(&lines[1].text()[last.end..], ";");
    }

    #[test]
    fn a_block_comment_carries_across_lines() {
        let lines = highlight("a.rs", "fn f() {\n    /* one\n       two */ let x = 1;\n}");
        assert!(lines[1]
            .spans()
            .iter()
            .any(|span| span.token == Token::Comment));
        // The comment's last line ends the comment, and code resumes on it.
        assert!(lines[2]
            .runs()
            .iter()
            .any(|(text, token)| { *text == "let" && *token == Some(Token::Keyword) }));
    }

    #[test]
    fn a_python_docstring_and_decorator() {
        let lines = highlight(
            "a.py",
            "@app.route(\"/\")\ndef f():\n    \"\"\"Doc\n    more\"\"\"",
        );
        assert!(has("a.py", "@app.route(\"/\")", "@app", Token::Attribute));
        assert_eq!(
            lines[2].spans().last().map(|span| span.token),
            Some(Token::String)
        );
        assert_eq!(
            lines[3].spans().last().map(|span| span.token),
            Some(Token::String)
        );
        // A prefixed literal is one string, prefix included.
        assert!(has("a.py", "name = f\"{x}\"", "f\"{x}\"", Token::String));
    }

    #[test]
    fn a_javascript_template_literal_and_its_comment() {
        let lines = highlight("a.ts", "const u = `/api/${id}`; // see docs");
        assert!(lines[0]
            .spans()
            .iter()
            .any(|span| span.token == Token::String
                && lines[0].text()[span.start..span.end].starts_with('`')));
        assert!(has(
            "a.ts",
            "const u = `/api/${id}`; // see docs",
            "// see docs",
            Token::Comment
        ));
        // A multi-line template keeps its next line a string.
        let lines = highlight("a.ts", "const t = `one\ntwo`;");
        assert_eq!(
            lines[1].spans().first().map(|span| span.token),
            Some(Token::String)
        );
    }

    #[test]
    fn json_colours_a_member_name_apart_from_its_value() {
        let line = "  \"name\": \"future\", \"count\": 3, \"on\": true,";
        assert!(has("a.json", line, "\"name\"", Token::Key));
        assert!(has("a.json", line, "\"future\"", Token::String));
        assert!(has("a.json", line, "3", Token::Number));
        assert!(has("a.json", line, "true", Token::Keyword));
    }

    #[test]
    fn toml_and_yaml_keys_are_keys() {
        assert!(has(
            "Cargo.toml",
            "name = \"future-tui\"",
            "name",
            Token::Key
        ));
        assert!(has(
            "Cargo.toml",
            "[dependencies]",
            "[dependencies]",
            Token::Type
        ));
        assert!(has("Cargo.toml", "[[bin]]", "[[bin]]", Token::Type));
        // A joined word is one key, not a word and a stray dash.
        assert!(has(
            "Cargo.toml",
            "default-members = [\"agent\"]",
            "default-members",
            Token::Key
        ));
        assert!(has(
            "Cargo.toml",
            "\"quoted key\" = 1",
            "\"quoted key\"",
            Token::Key
        ));
        assert!(has("a.yaml", "  name: future", "name", Token::Key));
        assert!(has("a.yaml", "  build-args: x", "build-args", Token::Key));
        // A value that is not followed by a colon is not a key.
        assert!(!has("a.yaml", "  - future os", "future", Token::Key));
        assert!(has("a.yaml", "  - name: future", "name", Token::Key));
    }

    #[test]
    fn shell_comments_parameters_and_commands() {
        let line = "for f in $FILES; do echo \"$f\"; done # loop";
        assert!(has("a.sh", line, "$FILES", Token::Type));
        assert!(has("a.sh", line, "# loop", Token::Comment));
        // `$#` is a parameter, not a comment: the line keeps its code colour.
        let runs = runs("a.sh", "echo $#");
        assert!(!runs.iter().any(|(_, token)| *token == Some(Token::Comment)));
    }

    #[test]
    fn sql_keywords_are_case_insensitive() {
        for line in ["SELECT * FROM t WHERE a = 'x'", "select * from t"] {
            assert!(
                has("a.sql", line, "SELECT", Token::Keyword)
                    || has("a.sql", line, "select", Token::Keyword)
            );
            assert!(has("a.sql", "SELECT 'it''s'", "'it''s'", Token::String));
        }
    }

    #[test]
    fn css_properties_are_keys_and_hex_colours_are_numbers() {
        assert!(has("a.css", ".a { color: #ff8800; }", "color", Token::Key));
        assert!(has(
            "a.css",
            ".a { color: #ff8800; }",
            "#ff8800",
            Token::Number
        ));
        assert!(has(
            "a.css",
            "@media (min-width: 600px) {",
            "@media",
            Token::Attribute
        ));
        // A selector's colon is not a property: the name stays plain.
        assert!(!has("a.css", "a:hover {", "a", Token::Key));
    }

    #[test]
    fn html_names_tags_and_attributes() {
        let line = "<a href=\"/docs\" class=\"x\">Docs</a>";
        assert!(has("a.html", line, "<a", Token::Keyword));
        assert!(has("a.html", line, "href", Token::Key));
        assert!(has("a.html", line, "\"/docs\"", Token::String));
        // Text between tags is not an attribute.
        assert!(!has("a.html", line, "Docs", Token::Key));
        let lines = highlight("a.html", "<!-- one\ntwo -->");
        assert!(lines[1]
            .spans()
            .iter()
            .any(|span| span.token == Token::Comment));
    }

    #[test]
    fn an_unknown_extension_is_painted_exactly_as_it_came() {
        for path in ["notes.txt", "LICENSE", "blob"] {
            let lines = highlight(path, "fn main() { /* not code here */ }");
            assert!(lines[0].spans().is_empty(), "{path}");
            assert_eq!(lines[0].text(), "fn main() { /* not code here */ }");
        }
    }

    #[test]
    fn tabs_become_spaces_and_the_spans_still_line_up() {
        let lines = highlight("a.go", "\tif x {");
        assert_eq!(lines[0].text(), "    if x {");
        for span in lines[0].spans() {
            assert!(lines[0].text().is_char_boundary(span.start));
            assert!(lines[0].text().is_char_boundary(span.end));
            assert!(span.start < span.end);
        }
    }

    /// Every span is inside the line, ordered, disjoint, and on character
    /// boundaries — including on lines with CJK, which is where a byte/char
    /// mix-up shows up first.
    #[test]
    fn spans_are_ordered_disjoint_and_on_character_boundaries() {
        let samples: &[(&str, &[&str])] = &[
            (
                "a.rs",
                &[
                    "let 变量 = \"中文\"; // 注释",
                    "fn f<'a>() -> &'a str { \"\" }",
                ],
            ),
            ("a.py", &["def f(名字):  # 说明", "\"\"\"文档\"\"\""]),
            ("a.json", &["{\"键\": \"值\", \"n\": 1}"]),
            ("a.html", &["<p title=\"中文\">文本</p>"]),
            ("a.sql", &["SELECT '中文' FROM t -- 注释"]),
            ("a.css", &[".a::before { content: \"→\"; }"]),
        ];
        for (path, lines) in samples {
            for line in *lines {
                for line in highlight(path, line) {
                    let text = line.text();
                    let mut at = 0;
                    for span in line.spans() {
                        assert!(text.is_char_boundary(span.start), "{text:?}");
                        assert!(text.is_char_boundary(span.end), "{text:?}");
                        assert!(span.start >= at, "spans overlap: {text:?}");
                        assert!(span.end > span.start, "empty span: {text:?}");
                        assert!(span.end <= text.len(), "{text:?}");
                        at = span.end;
                    }
                }
            }
        }
    }

    #[test]
    fn a_signed_number_is_one_token_and_a_lone_sign_is_punctuation() {
        assert!(has("a.json", "{\"n\": -1.5}", "-1.5", Token::Number));
        // A sign with no digits still advances the scanner: a stall here would
        // be an endless loop, not a wrong colour.
        assert!(tokens("a.json", "a - b").is_empty());
        assert_eq!(highlight("a.json", "a - b")[0].text(), "a - b");
    }

    /// Whatever a line holds, the scan ends, every span is inside the text, and
    /// spans stay ordered and disjoint — the three ways a lexer can hang or
    /// corrupt a frame.
    #[test]
    fn pathological_lines_terminate_and_keep_their_spans_inside() {
        let sources = [
            "a.rs", "a.py", "a.json", "a.css", "a.html", "a.sql", "a.sh", "a.toml", "a.yaml",
            "a.go", "a.ts", "a.txt",
        ];
        let fragments = [
            "-",
            "+",
            "\"",
            "'",
            "`",
            "#",
            "$",
            "@",
            "<",
            ">",
            "\"\"\"",
            "'''",
            "/*",
            "*/",
            "#[",
            "$(",
            "${",
            "\\",
            "r#",
            "b\"",
            "0x",
            "1e",
            "<a",
            "<!",
            "::",
            "&",
            "|",
            "中文",
            "'",
            "\"unterminated",
            "<!--",
            "-->",
            "{{",
            "}}",
        ];
        for path in sources {
            for fragment in fragments {
                for line in highlight(path, fragment) {
                    let text = line.text();
                    let mut at = 0;
                    for span in line.spans() {
                        assert!(text.is_char_boundary(span.start), "{path} {text:?}");
                        assert!(text.is_char_boundary(span.end), "{path} {text:?}");
                        assert!(span.start >= at, "{path} {text:?}");
                        assert!(span.end > span.start, "{path} {text:?}");
                        assert!(span.end <= text.len(), "{path} {text:?}");
                        at = span.end;
                    }
                }
            }
        }
    }

    #[test]
    fn a_language_is_recognised_by_extension_and_by_name() {
        assert_eq!(Language::from_path("/src/main.rs"), Language::Rust);
        assert_eq!(Language::from_path("App.TSX"), Language::TypeScript);
        assert_eq!(Language::from_path("Makefile"), Language::Shell);
        assert_eq!(Language::from_path("docker/Dockerfile"), Language::Shell);
        assert_eq!(Language::from_path("Cargo.lock"), Language::Toml);
        assert_eq!(Language::from_path("photo.png"), Language::Plain);
        assert_eq!(Language::from_path(""), Language::Plain);
    }

    #[test]
    fn a_plain_line_keeps_its_text_and_offers_one_run() {
        let line = Line::plain("just text");
        assert_eq!(line.text(), "just text");
        assert!(line.spans().is_empty());
        assert_eq!(line.runs(), vec![("just text", None)]);
    }
}
