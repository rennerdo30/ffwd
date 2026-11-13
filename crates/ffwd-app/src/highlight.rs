//! Syntax colours for diff lines: a small tokenizer (keywords, strings,
//! comments, numbers, type names) per language family, picked by file
//! extension. It runs only for the lines on screen, one line at a time,
//! with no dependency; exact parsing is not the goal, readable code is.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Keyword,
    Type,
    String,
    Number,
    Comment,
}

pub struct Lang {
    keywords: &'static [&'static str],
    line_comment: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    /// Quote characters that start strings.
    quotes: &'static [char],
    /// Capitalised identifiers are types (Rust, Java, C#, Go, Swift, …).
    caps_are_types: bool,
}

const C_LIKE: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "return",
    "goto",
    "struct",
    "union",
    "enum",
    "typedef",
    "const",
    "static",
    "extern",
    "inline",
    "void",
    "int",
    "char",
    "short",
    "long",
    "float",
    "double",
    "unsigned",
    "signed",
    "sizeof",
    "class",
    "public",
    "private",
    "protected",
    "virtual",
    "override",
    "template",
    "typename",
    "namespace",
    "using",
    "new",
    "delete",
    "this",
    "true",
    "false",
    "nullptr",
    "auto",
    "bool",
    "try",
    "catch",
    "throw",
    "volatile",
    "constexpr",
    "noexcept",
    "operator",
    "friend",
    "explicit",
    "include",
    "define",
    "ifdef",
    "ifndef",
    "endif",
    "pragma",
];
const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "Some", "None", "Ok",
    "Err",
];
const JS: &[&str] = &[
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
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "async",
    "await",
    "of",
    "true",
    "false",
    "null",
    "undefined",
    "interface",
    "type",
    "enum",
    "implements",
    "private",
    "public",
    "protected",
    "readonly",
    "static",
    "from",
    "as",
];
const PY: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "self",
];
const GO: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    "true",
    "false",
    "nil",
];
const CS_JAVA: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "foreach",
    "if",
    "implements",
    "import",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "out",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sealed",
    "short",
    "static",
    "string",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "throws",
    "true",
    "try",
    "using",
    "var",
    "virtual",
    "void",
    "while",
    "async",
    "await",
    "get",
    "set",
    "val",
    "fun",
    "when",
    "let",
    "func",
    "guard",
    "self",
    "nil",
];
const SH: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "in", "function", "return",
    "local", "export", "echo", "set", "unset", "param", "foreach", "true", "false",
];
const DATA: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];
const SQL: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop",
    "alter", "index", "join", "left", "right", "inner", "outer", "on", "and", "or", "not", "null", "as", "group", "by",
    "order", "having", "limit", "primary", "key", "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE",
    "SET", "DELETE", "CREATE", "TABLE", "JOIN", "ON", "AND", "OR", "NOT", "NULL", "AS", "GROUP", "BY", "ORDER",
];
const LUA: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or",
    "repeat", "return", "then", "true", "until", "while",
];

/// The language of a path, by its extension (None: no colours).
pub fn lang_for(path: &str) -> Option<&'static Lang> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    static C: Lang = Lang {
        keywords: C_LIKE,
        line_comment: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        caps_are_types: true,
    };
    static R: Lang =
        Lang { keywords: RUST, line_comment: &["//"], block: Some(("/*", "*/")), quotes: &['"'], caps_are_types: true };
    static J: Lang = Lang {
        keywords: JS,
        line_comment: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\'', '`'],
        caps_are_types: true,
    };
    static P: Lang =
        Lang { keywords: PY, line_comment: &["#"], block: None, quotes: &['"', '\''], caps_are_types: true };
    static G: Lang = Lang {
        keywords: GO,
        line_comment: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\'', '`'],
        caps_are_types: true,
    };
    static CS: Lang = Lang {
        keywords: CS_JAVA,
        line_comment: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        caps_are_types: true,
    };
    static S: Lang =
        Lang { keywords: SH, line_comment: &["#"], block: None, quotes: &['"', '\''], caps_are_types: false };
    static D: Lang =
        Lang { keywords: DATA, line_comment: &["#", ";"], block: None, quotes: &['"', '\''], caps_are_types: false };
    static Q: Lang = Lang {
        keywords: SQL,
        line_comment: &["--"],
        block: Some(("/*", "*/")),
        quotes: &['\''],
        caps_are_types: false,
    };
    static L: Lang =
        Lang { keywords: LUA, line_comment: &["--"], block: None, quotes: &['"', '\''], caps_are_types: false };
    static X: Lang = Lang {
        keywords: &[],
        line_comment: &[],
        block: Some(("<!--", "-->")),
        quotes: &['"', '\''],
        caps_are_types: false,
    };
    Some(match ext.as_str() {
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" | "glsl" | "hlsl" | "shader" | "cginc" => &C,
        "rs" => &R,
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "css" | "scss" | "less" => &J,
        "py" | "pyi" | "rb" | "nim" => &P,
        "go" => &G,
        "cs" | "java" | "kt" | "kts" | "swift" | "scala" | "dart" | "groovy" | "gradle" => &CS,
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "bat" | "cmd" => &S,
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "conf" | "properties" | "env" => &D,
        "sql" => &Q,
        "lua" => &L,
        "html" | "htm" | "xml" | "svg" | "xaml" | "csproj" | "props" | "vue" => &X,
        _ => return None,
    })
}

/// The coloured spans of one line: (byte start, byte end, kind), in order
/// and covering the whole line. `in_block` says the line starts inside a
/// block comment.
pub fn spans(lang: &Lang, line: &str, in_block: bool) -> Vec<(usize, usize, Kind)> {
    let b = line.as_bytes();
    let mut out: Vec<(usize, usize, Kind)> = Vec::new();
    let push = |out: &mut Vec<(usize, usize, Kind)>, s: usize, e: usize, k: Kind| {
        if e <= s {
            return;
        }
        match out.last_mut() {
            Some(last) if last.2 == k && last.1 == s => last.1 = e,
            _ => out.push((s, e, k)),
        }
    };
    let mut i = 0;
    if in_block && let Some((_, close)) = lang.block {
        match line.find(close) {
            Some(p) => {
                push(&mut out, 0, p + close.len(), Kind::Comment);
                i = p + close.len();
            }
            None => return vec![(0, line.len(), Kind::Comment)],
        }
    }
    while i < b.len() {
        let rest = &line[i..];
        if lang.line_comment.iter().any(|c| rest.starts_with(c)) {
            push(&mut out, i, line.len(), Kind::Comment);
            break;
        }
        if let Some((open, close)) = lang.block
            && rest.starts_with(open)
        {
            let end = rest[open.len()..].find(close).map_or(line.len(), |p| i + open.len() + p + close.len());
            push(&mut out, i, end, Kind::Comment);
            i = end;
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        if lang.quotes.contains(&c) {
            // To the closing quote (backslash escapes), or the line's end.
            let mut j = i + c.len_utf8();
            let mut esc = false;
            while j < b.len() {
                let d = line[j..].chars().next().unwrap_or(' ');
                j += d.len_utf8();
                if esc {
                    esc = false;
                } else if d == '\\' {
                    esc = true;
                } else if d == c {
                    break;
                }
            }
            push(&mut out, i, j, Kind::String);
            i = j;
            continue;
        }
        if c.is_ascii_digit() {
            let j = i + rest.find(|x: char| !(x.is_ascii_alphanumeric() || x == '.' || x == '_')).unwrap_or(rest.len());
            push(&mut out, i, j, Kind::Number);
            i = j;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let j = i + rest.find(|x: char| !(x.is_alphanumeric() || x == '_')).unwrap_or(rest.len());
            let w = &line[i..j];
            let k = if lang.keywords.contains(&w) {
                Kind::Keyword
            } else if lang.caps_are_types && w.chars().next().is_some_and(char::is_uppercase) {
                Kind::Type
            } else {
                Kind::Plain
            };
            push(&mut out, i, j, k);
            i = j;
            continue;
        }
        push(&mut out, i, i + c.len_utf8(), Kind::Plain);
        i += c.len_utf8();
    }
    out
}

/// Does a line end inside a block comment (given how it started)?
pub fn ends_in_block(lang: &Lang, line: &str, mut in_block: bool) -> bool {
    let Some((open, close)) = lang.block else { return false };
    let mut rest = line;
    loop {
        if in_block {
            match rest.find(close) {
                Some(p) => {
                    rest = &rest[p + close.len()..];
                    in_block = false;
                }
                None => return true,
            }
        } else {
            // A line comment or a string hides what follows (good enough).
            let cut = lang.line_comment.iter().filter_map(|c| rest.find(c)).min().unwrap_or(rest.len());
            match rest[..cut].find(open) {
                Some(p) => {
                    rest = &rest[p + open.len()..];
                    in_block = true;
                }
                None => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(path: &str, line: &str) -> Vec<(&'static str, Kind)> {
        let l = lang_for(path).unwrap();
        let s = spans(l, line, false);
        // The spans cover the whole line, in order.
        assert_eq!(s.first().map(|x| x.0), Some(0));
        assert_eq!(s.last().map(|x| x.1), Some(line.len()));
        assert!(s.windows(2).all(|w| w[0].1 == w[1].0));
        s.into_iter()
            .filter(|x| x.2 != Kind::Plain)
            .map(|(a, b, k)| (Box::leak(line[a..b].to_string().into_boxed_str()) as &str, k))
            .collect()
    }

    #[test]
    fn rust_line() {
        assert_eq!(
            kinds("x.rs", r#"let s: String = "a \" b"; // note 42"#),
            vec![
                ("let", Kind::Keyword),
                ("String", Kind::Type),
                (r#""a \" b""#, Kind::String),
                ("// note 42", Kind::Comment)
            ]
        );
        assert_eq!(kinds("x.rs", "x = 0x1f + 3.5;"), vec![("0x1f", Kind::Number), ("3.5", Kind::Number)]);
    }

    #[test]
    fn keywords_at_the_line_start() {
        assert_eq!(kinds("x.rs", "fn main() {"), vec![("fn", Kind::Keyword)]);
        assert_eq!(kinds("x.rs", "use std::fmt;"), vec![("use", Kind::Keyword)]);
    }

    #[test]
    fn block_comments_across_lines() {
        let l = lang_for("a.c").unwrap();
        assert!(ends_in_block(l, "int a; /* start", false));
        assert!(ends_in_block(l, "middle", true));
        assert!(!ends_in_block(l, "end */ int b;", true));
        assert!(!ends_in_block(l, "// /* not a block", false));
        let s = spans(l, "end */ int b;", true);
        assert_eq!(s[0], (0, 6, Kind::Comment));
        assert!(lang_for("README").is_none());
        assert!(kinds("a.py", "def f(x): # hi").contains(&("def", Kind::Keyword)));
    }
}
