//! Syntax highlighting, one line at a time: data formats (JSON, YAML, CSV, Markdown, INI, TOML, XML)
//! and code (Bash, PowerShell, Python, JavaScript/TypeScript).

use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    Plain,
    Json,
    Yaml,
    Csv,
    Markdown,
    Ini,
    Xml,
    Bash,
    Toml,
    PowerShell,
    Python,
    JavaScript,
}

impl Lang {
    /// Saved sessions store positions in this list: only ever append.
    pub const ALL: [Lang; 12] = [Lang::Plain, Lang::Csv, Lang::Ini, Lang::Json, Lang::Markdown, Lang::Xml, Lang::Yaml, Lang::Bash,
        Lang::Toml, Lang::PowerShell, Lang::Python, Lang::JavaScript];

    pub fn from_ext(ext: &str) -> Lang {
        match ext.to_ascii_lowercase().as_str() {
            "json" | "jsonc" | "geojson" => Lang::Json,
            "yaml" | "yml" => Lang::Yaml,
            "csv" | "tsv" => Lang::Csv,
            "md" | "markdown" => Lang::Markdown,
            "ini" | "cfg" | "conf" | "properties" => Lang::Ini,
            "sh" | "bash" | "zsh" | "ksh" => Lang::Bash,
            "toml" => Lang::Toml,
            "ps1" | "psm1" | "psd1" => Lang::PowerShell,
            "py" | "pyw" | "pyi" => Lang::Python,
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" => Lang::JavaScript,
            "xml" | "xsd" | "xsl" | "xslt" | "svg" | "xaml" | "csproj" | "vbproj" | "props" | "targets" | "resx" | "config"
            | "plist" | "rss" | "atom" | "xhtml" => Lang::Xml,
            _ => Lang::Plain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Plain => "Plain Text",
            Lang::Json => "JSON",
            Lang::Yaml => "YAML",
            Lang::Csv => "CSV",
            Lang::Markdown => "Markdown",
            Lang::Ini => "INI",
            Lang::Xml => "XML",
            Lang::Bash => "Bash",
            Lang::Toml => "TOML",
            Lang::PowerShell => "PowerShell",
            Lang::Python => "Python",
            Lang::JavaScript => "JavaScript",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Style {
    Key,
    Str,
    Num,
    Keyword,
    Comment,
    Punct,
    Heading,
    Code,
    Link,
    Column(u8),
    Tag,
    Attr,
    Bold,
    Italic,
    BoldItalic,
}

/// Text color for a style in the current theme.
pub fn color(s: Style) -> u32 {
    let p = crate::theme::palette();
    match s {
        Style::Key => p.key,
        Style::Str => p.string,
        Style::Num => p.number,
        Style::Keyword => p.keyword,
        Style::Comment => p.comment,
        Style::Punct => p.punct,
        Style::Heading => p.heading,
        Style::Code => p.code,
        Style::Link => p.link,
        Style::Column(i) => p.columns[i as usize % p.columns.len()],
        Style::Tag => p.tag,
        Style::Attr => p.attr,
        Style::Bold | Style::Italic | Style::BoldItalic => p.fg,
    }
}

/// Font variant for a style: 0 normal, 1 bold, 2 italic, 3 bold italic.
pub fn font_style(s: Style) -> u8 {
    match s {
        Style::Heading | Style::Bold => 1,
        Style::Italic => 2,
        Style::BoldItalic => 3,
        _ => 0,
    }
}

/// State carried from one line to the next.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Ctx {
    fence: bool,   // inside a Markdown code block
    sep: u16,      // CSV separator
    comment: bool, // inside an XML comment
    tag: bool,     // inside an XML tag, between its name and '>'
    block: bool,   // inside a block comment (code languages)
    open: u16,     // inside a string spanning lines: its quote character
    triple: bool,  // ... written with three quotes (Python)
}

type Spans = Vec<(Range<usize>, Style)>;

const fn ch(b: u8) -> u16 {
    b as u16
}

const SP: u16 = ch(b' ');
const TAB: u16 = ch(b'\t');

fn is_digit(c: u16) -> bool {
    (ch(b'0')..=ch(b'9')).contains(&c)
}

fn is_word(c: u16) -> bool {
    is_digit(c) || (ch(b'a')..=ch(b'z')).contains(&c) || (ch(b'A')..=ch(b'Z')).contains(&c) || c == ch(b'_') || c > 127
}

fn indent(l: &[u16]) -> usize {
    l.iter().take_while(|&&c| c == SP || c == TAB).count()
}

fn skip_spaces(l: &[u16], i: usize) -> usize {
    i + indent(&l[i.min(l.len())..])
}

fn eq(l: &[u16], s: &str) -> bool {
    l.iter().copied().eq(s.encode_utf16())
}

fn starts(l: &[u16], s: &str) -> bool {
    l.len() >= s.len() && eq(&l[..s.len()], s)
}

/// Index just past the closing quote of the string starting at `start`.
fn string_end(l: &[u16], start: usize) -> usize {
    let q = l[start];
    let mut i = start + 1;
    while i < l.len() {
        if l[i] == ch(b'\\') && q == ch(b'"') {
            i += 2;
        } else if l[i] == q {
            return i + 1;
        } else {
            i += 1;
        }
    }
    l.len()
}

fn is_fence(l: &[u16]) -> bool {
    let t = &l[indent(l)..];
    starts(t, "```") || starts(t, "~~~")
}

/// Context at the start of line `upto`; `lines` yields every line's text from the first.
pub fn context<'a>(lang: Lang, mut lines: impl Iterator<Item = &'a [u16]>, upto: usize) -> Ctx {
    let mut ctx = Ctx::default();
    match lang {
        Lang::Csv => {
            // The separator used most on the first line; ties go to the comma.
            let first = lines.next().unwrap_or_default();
            ctx.sep = [TAB, ch(b';'), ch(b',')].into_iter().max_by_key(|&s| first.iter().filter(|&&c| c == s).count()).unwrap();
        }
        _ => {
            for l in lines.take(upto) {
                advance(lang, l, &mut ctx);
            }
        }
    }
    ctx
}

/// Updates `ctx` from the start of line `l` to the start of the next line.
pub fn advance(lang: Lang, l: &[u16], ctx: &mut Ctx) {
    match lang {
        Lang::Markdown => ctx.fence ^= is_fence(l),
        Lang::Xml => _ = xml(l, ctx),
        Lang::PowerShell | Lang::Python | Lang::JavaScript => _ = code(l, ctx, code_spec(lang)),
        _ => {}
    }
}

/// Styled ranges of one line; `ctx` is advanced to the next line.
pub fn line_spans(lang: Lang, l: &[u16], ctx: &mut Ctx) -> Spans {
    match lang {
        Lang::Plain => Vec::new(),
        Lang::Json => json(l),
        Lang::Yaml => yaml(l),
        Lang::Csv => csv(l, ctx.sep),
        Lang::Markdown => markdown(l, ctx),
        Lang::Ini => ini(l),
        Lang::Xml => xml(l, ctx),
        Lang::Bash => bash(l),
        Lang::Toml => ini(l),
        Lang::PowerShell | Lang::Python | Lang::JavaScript => code(l, ctx, code_spec(lang)),
    }
}

/// How a programming language writes comments, strings and keywords.
struct CodeSpec {
    line: &'static [&'static str],             // line comment starters
    block: Option<(&'static str, &'static str)>, // block comment delimiters
    quotes: &'static str,                      // string quotes ending at the line end
    spanning: &'static str,                    // string quotes that may span lines
    triple: bool,                              // """ and ''' strings (Python)
    keywords: &'static [&'static str],
    ignore_case: bool,
    dollar_vars: bool, // $name
    decorators: bool,  // @name
    dash_params: bool, // -Name parameters and Verb-Noun commands (PowerShell)
}

fn code_spec(lang: Lang) -> &'static CodeSpec {
    const PYTHON: CodeSpec = CodeSpec {
        line: &["#"], block: None, quotes: "\"'", spanning: "", triple: true,
        keywords: &["False", "None", "True", "and", "as", "assert", "async", "await", "break", "case", "class", "continue",
            "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
            "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "self", "try", "while", "with", "yield"],
        ignore_case: false, dollar_vars: false, decorators: true, dash_params: false,
    };
    const JAVASCRIPT: CodeSpec = CodeSpec {
        line: &["//"], block: Some(("/*", "*/")), quotes: "\"'", spanning: "`", triple: false,
        keywords: &["abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger",
            "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for", "from", "function",
            "get", "if", "implements", "import", "in", "instanceof", "interface", "let", "new", "null", "of", "private",
            "protected", "public", "readonly", "return", "set", "static", "super", "switch", "this", "throw", "true", "try",
            "type", "typeof", "undefined", "var", "void", "while", "with", "yield"],
        ignore_case: false, dollar_vars: false, decorators: true, dash_params: false,
    };
    const POWERSHELL: CodeSpec = CodeSpec {
        line: &["#"], block: Some(("<#", "#>")), quotes: "\"'", spanning: "", triple: false,
        keywords: &["begin", "break", "catch", "class", "continue", "do", "else", "elseif", "end", "enum", "exit", "filter",
            "finally", "for", "foreach", "function", "if", "in", "param", "process", "return", "switch", "throw", "trap",
            "try", "until", "using", "while", "$true", "$false", "$null"],
        ignore_case: true, dollar_vars: true, decorators: false, dash_params: true,
    };
    match lang {
        Lang::Python => &PYTHON,
        Lang::PowerShell => &POWERSHELL,
        _ => &JAVASCRIPT,
    }
}

fn code(l: &[u16], ctx: &mut Ctx, spec: &CodeSpec) -> Spans {
    let mut out = Vec::new();
    let mut i = 0;
    let name = |c: u16| is_word(c) && c < 128;
    let is = |set: &str, c: u16| set.encode_utf16().any(|q| q == c);
    while i < l.len() {
        // Continue a block comment or a string from earlier lines.
        if ctx.block {
            let (_, close) = spec.block.unwrap();
            let e = find(l, i, close).map_or(l.len(), |e| e + close.len());
            ctx.block = e == l.len() && !l.ends_with(&close.encode_utf16().collect::<Vec<_>>());
            out.push((i..e, Style::Comment));
            i = e;
            continue;
        }
        if ctx.open != 0 {
            let q = ctx.open;
            let quote: Vec<u16> = if ctx.triple { vec![q; 3] } else { vec![q] };
            let mut j = i;
            let e = loop {
                if j >= l.len() {
                    break None;
                }
                if l[j] == ch(b'\\') {
                    j += 2;
                } else if l[j..].starts_with(&quote) {
                    break Some(j + quote.len());
                } else {
                    j += 1;
                }
            };
            out.push((i..e.unwrap_or(l.len()), Style::Str));
            if let Some(e) = e {
                (ctx.open, ctx.triple, i) = (0, false, e);
                continue;
            }
            break;
        }
        let c = l[i];
        let rest = &l[i..];
        let word_start = i == 0 || !name(l[i - 1]);
        if spec.line.iter().any(|m| starts(rest, m)) && !(spec.block.is_some_and(|(o, _)| starts(rest, o))) {
            out.push((i..l.len(), Style::Comment));
            break;
        } else if let Some((open, _)) = spec.block.filter(|(o, _)| starts(rest, o)) {
            (ctx.block, i) = (true, i + open.len());
            out.push((i - open.len()..i, Style::Comment));
        } else if spec.triple && (starts(rest, "\"\"\"") || starts(rest, "'''")) {
            (ctx.open, ctx.triple) = (c, true);
            out.push((i..i + 3, Style::Str));
            i += 3;
        } else if is(spec.spanning, c) {
            (ctx.open, ctx.triple) = (c, false);
            out.push((i..i + 1, Style::Str));
            i += 1;
        } else if is(spec.quotes, c) {
            let e = string_end(l, i);
            out.push((i..e, Style::Str));
            i = e;
        } else if spec.dollar_vars && c == ch(b'$') {
            let e = i + 1 + l[i + 1..].iter().take_while(|&&x| name(x) || x == ch(b':')).count();
            let kw = spec.keywords.iter().any(|k| eq(&l[i..e].to_ascii_lowercase(), k));
            out.push((i..e, if kw { Style::Keyword } else { Style::Key }));
            i = e;
        } else if spec.decorators && c == ch(b'@') && l.get(i + 1).is_some_and(|&x| name(x)) {
            let e = i + 1 + l[i + 1..].iter().take_while(|&&x| name(x) || x == ch(b'.')).count();
            out.push((i..e, Style::Attr));
            i = e;
        } else if spec.dash_params && c == ch(b'-') && word_start && l.get(i + 1).is_some_and(|&x| name(x) && !is_digit(x))
            && (i == 0 || l[i - 1] == SP || l[i - 1] == TAB)
        {
            let e = i + 1 + l[i + 1..].iter().take_while(|&&x| name(x)).count();
            out.push((i..e, Style::Attr));
            i = e;
        } else if word_start && is_digit(c) {
            let e = i + l[i..].iter().take_while(|&&x| name(x) || x == ch(b'.')).count();
            out.push((i..e, Style::Num));
            i = e;
        } else if word_start && name(c) {
            let mut e = i + l[i..].iter().take_while(|&&x| name(x)).count();
            // PowerShell commands are Verb-Noun.
            let command = spec.dash_params && l.get(e) == Some(&ch(b'-')) && l.get(e + 1).is_some_and(|&x| name(x));
            if command {
                e += 1 + l[e + 1..].iter().take_while(|&&x| name(x)).count();
                out.push((i..e, Style::Tag));
            } else {
                let w = &l[i..e];
                let kw = if spec.ignore_case {
                    spec.keywords.iter().any(|k| eq(&w.to_ascii_lowercase(), k))
                } else {
                    spec.keywords.iter().any(|k| eq(w, k))
                };
                if kw {
                    out.push((i..e, Style::Keyword));
                }
            }
            i = e;
        } else {
            i += 1;
        }
    }
    out
}

trait AsciiLower {
    fn to_ascii_lowercase(&self) -> Vec<u16>;
}

impl AsciiLower for [u16] {
    fn to_ascii_lowercase(&self) -> Vec<u16> {
        self.iter().map(|&c| if (ch(b'A')..=ch(b'Z')).contains(&c) { c + 32 } else { c }).collect()
    }
}

const BASH_KEYWORDS: [&str; 31] = [
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return",
    "local", "export", "select", "break", "continue", "declare", "readonly", "unset", "shift", "source", "alias", "exit",
    "set", "eval", "exec", "trap",
];

fn bash(l: &[u16]) -> Spans {
    let mut out = Vec::new();
    let mut i = 0;
    let var = |c: u16| is_word(c) && c < 128;
    while i < l.len() {
        let c = l[i];
        let at_word_start = i == 0 || !var(l[i - 1]);
        if c == ch(b'#') && (i == 0 || l[i - 1] == SP || l[i - 1] == TAB) {
            out.push((i..l.len(), Style::Comment));
            break;
        } else if c == ch(b'"') || c == ch(b'\'') {
            let e = string_end(l, i);
            out.push((i..e, Style::Str));
            i = e;
        } else if c == ch(b'$') {
            // $name, ${...}, $1, $@, $?, $#, $$ …
            let e = match l.get(i + 1) {
                Some(&b) if b == ch(b'{') => l[i..].iter().position(|&x| x == ch(b'}')).map_or(l.len(), |p| i + p + 1),
                Some(&b) if var(b) => i + 1 + l[i + 1..].iter().take_while(|&&x| var(x)).count(),
                Some(&b) if "@*#?$!-".encode_utf16().any(|s| s == b) => i + 2,
                _ => i + 1,
            };
            out.push((i..e, Style::Key));
            i = e;
        } else if at_word_start && var(c) {
            let e = i + l[i..].iter().take_while(|&&x| var(x) || x == ch(b'-')).count();
            let w = &l[i..e];
            if w.iter().all(|&x| is_digit(x)) {
                out.push((i..e, Style::Num));
            } else if BASH_KEYWORDS.iter().any(|k| eq(w, k)) {
                out.push((i..e, Style::Keyword));
            }
            i = e;
        } else {
            i += 1;
        }
    }
    out
}

fn find(l: &[u16], from: usize, pat: &str) -> Option<usize> {
    (from..=l.len().saturating_sub(pat.len())).find(|&j| starts(&l[j..], pat))
}

fn xml(l: &[u16], ctx: &mut Ctx) -> Spans {
    let name = |c: u16| is_word(c) || c == ch(b':') || c == ch(b'-') || c == ch(b'.');
    let name_end = |from: usize| from + l[from..].iter().take_while(|&&c| name(c)).count();
    let mut out = Vec::new();
    let mut i = 0;
    while i < l.len() {
        let c = l[i];
        if ctx.comment {
            match find(l, i, "-->") {
                Some(e) => {
                    out.push((i..e + 3, Style::Comment));
                    (ctx.comment, i) = (false, e + 3);
                }
                None => {
                    out.push((i..l.len(), Style::Comment));
                    break;
                }
            }
        } else if ctx.tag {
            if c == ch(b'>') || ((c == ch(b'/') || c == ch(b'?')) && l.get(i + 1) == Some(&ch(b'>'))) {
                let n = if c == ch(b'>') { 1 } else { 2 };
                out.push((i..i + n, Style::Punct));
                (ctx.tag, i) = (false, i + n);
            } else if c == ch(b'"') || c == ch(b'\'') {
                let e = string_end(l, i);
                out.push((i..e, Style::Str));
                i = e;
            } else if name(c) {
                let e = name_end(i);
                out.push((i..e, Style::Attr));
                i = e;
            } else {
                i += 1;
            }
        } else if starts(&l[i..], "<!--") {
            out.push((i..i + 4, Style::Comment));
            (ctx.comment, i) = (true, i + 4);
        } else if c == ch(b'<') {
            let j = i + 1 + l.get(i + 1).is_some_and(|&d| d == ch(b'/') || d == ch(b'?') || d == ch(b'!')) as usize;
            let e = name_end(j);
            out.push((i..j, Style::Punct));
            out.push((j..e, Style::Tag));
            (ctx.tag, i) = (true, e);
        } else if c == ch(b'&') {
            let e = l[i..].iter().position(|&d| d == ch(b';')).map_or(i + 1, |p| i + p + 1);
            out.push((i..e, Style::Keyword));
            i = e;
        } else {
            i += 1;
        }
    }
    // Merge comment pieces split at "<!--" so a comment is one span.
    out.dedup_by(|b, a| a.1 == Style::Comment && b.1 == Style::Comment && a.0.end == b.0.start && { a.0.end = b.0.end; true });
    out
}

fn ini(l: &[u16]) -> Spans {
    let i = indent(l);
    match l.get(i) {
        None => Vec::new(),
        Some(&c) if c == ch(b';') || c == ch(b'#') => vec![(i..l.len(), Style::Comment)],
        Some(&c) if c == ch(b'[') => vec![(i..l.len(), Style::Heading)],
        _ => match l[i..].iter().position(|&c| c == ch(b'=') || c == ch(b':')) {
            Some(p) => {
                let eq = i + p;
                let mut key_end = eq;
                while key_end > i && (l[key_end - 1] == SP || l[key_end - 1] == TAB) {
                    key_end -= 1;
                }
                let mut out = vec![(i..key_end, Style::Key), (eq..eq + 1, Style::Punct)];
                yaml_value(l, skip_spaces(l, eq + 1), l.len(), &mut out);
                out
            }
            None => Vec::new(),
        },
    }
}

fn json(l: &[u16]) -> Spans {
    let mut out = Vec::new();
    let mut i = 0;
    while i < l.len() {
        let c = l[i];
        if c == ch(b'"') {
            let end = string_end(l, i);
            let key = l.get(skip_spaces(l, end)) == Some(&ch(b':'));
            out.push((i..end, if key { Style::Key } else { Style::Str }));
            i = end;
        } else if is_digit(c) || (c == ch(b'-') && l.get(i + 1).is_some_and(|&d| is_digit(d))) {
            let end = i + 1 + l[i + 1..].iter().take_while(|&&d| is_digit(d) || "+-.eE".encode_utf16().any(|x| x == d)).count();
            out.push((i..end, Style::Num));
            i = end;
        } else if is_word(c) {
            let end = i + l[i..].iter().take_while(|&&d| is_word(d)).count();
            if ["true", "false", "null"].iter().any(|k| eq(&l[i..end], k)) {
                out.push((i..end, Style::Keyword));
            }
            i = end;
        } else if c == ch(b'/') && l.get(i + 1) == Some(&ch(b'/')) {
            out.push((i..l.len(), Style::Comment));
            break;
        } else {
            i += 1;
        }
    }
    out
}

/// Start of a `#` comment (at the start or after whitespace), outside quotes.
fn comment_start(l: &[u16], from: usize) -> Option<usize> {
    let mut i = from;
    while i < l.len() {
        match l[i] {
            q if q == ch(b'"') || q == ch(b'\'') => i = string_end(l, i),
            h if h == ch(b'#') && (i == 0 || l[i - 1] == SP || l[i - 1] == TAB) => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn yaml(l: &[u16]) -> Spans {
    let mut out = Vec::new();
    let mut i = indent(l);
    let t = &l[i..];
    if starts(t, "---") || starts(t, "...") {
        return vec![(i..l.len(), Style::Punct)];
    }
    let end = comment_start(l, i).unwrap_or(l.len());
    if end < l.len() {
        out.push((end..l.len(), Style::Comment));
    }
    // List item dashes.
    while i < end && l[i] == ch(b'-') && (i + 1 == end || l[i + 1] == SP) {
        out.push((i..i + 1, Style::Punct));
        i = skip_spaces(l, i + 1);
    }
    // A key ends at ':' followed by a space or the end.
    let mut j = if i < end && (l[i] == ch(b'"') || l[i] == ch(b'\'')) { string_end(l, i) } else { i };
    while j < end && !(l[j] == ch(b':') && (j + 1 == end || l[j + 1] == SP)) {
        j += 1;
    }
    if j < end && j > i {
        out.push((i..j, Style::Key));
        i = j + 1;
    }
    yaml_value(l, skip_spaces(l, i), end, &mut out);
    out
}

fn yaml_value(l: &[u16], i: usize, end: usize, out: &mut Spans) {
    if i >= end {
        return;
    }
    let c = l[i];
    if c == ch(b'"') || c == ch(b'\'') {
        out.push((i..string_end(l, i).min(end), Style::Str));
    } else if c == ch(b'&') || c == ch(b'*') || c == ch(b'!') {
        // Anchor, alias or tag, then possibly a value.
        let w = i + l[i..end].iter().take_while(|&&d| d != SP).count();
        out.push((i..w, Style::Punct));
        yaml_value(l, skip_spaces(l, w), end, out);
    } else if c == ch(b'|') || c == ch(b'>') {
        out.push((i..i + 1, Style::Punct));
    } else {
        let mut e = end;
        while e > i && (l[e - 1] == SP || l[e - 1] == TAB) {
            e -= 1;
        }
        let v = &l[i..e];
        let num = !v.is_empty() && v.iter().all(|&d| is_digit(d) || "+-._eExXoO".encode_utf16().any(|x| x == d))
            && v.iter().any(|&d| is_digit(d));
        let keyword = ["true", "false", "null", "yes", "no", "on", "off", "~", "True", "False", "Null", "TRUE", "FALSE", "NULL"]
            .iter().any(|k| eq(v, k));
        out.push((i..e, if num { Style::Num } else if keyword { Style::Keyword } else { Style::Str }));
    }
}

fn csv(l: &[u16], sep: u16) -> Spans {
    let mut out = Vec::new();
    let (mut start, mut col, mut quoted) = (0, 0u8, false);
    for i in 0..=l.len() {
        if i == l.len() || (!quoted && l[i] == sep) {
            out.push((start..i, Style::Column(col)));
            (start, col) = (i + 1, col.wrapping_add(1));
        } else if l[i] == ch(b'"') {
            quoted = !quoted;
        }
    }
    out
}

fn markdown(l: &[u16], ctx: &mut Ctx) -> Spans {
    let all = vec![(0..l.len(), Style::Code)];
    if is_fence(l) {
        ctx.fence = !ctx.fence;
        return all;
    }
    if ctx.fence {
        return all;
    }
    let mut out = Vec::new();
    let mut i = indent(l);
    let t = &l[i..];
    let hashes = t.iter().take_while(|&&c| c == ch(b'#')).count();
    if (1..=6).contains(&hashes) && t.get(hashes).is_none_or(|&c| c == SP) {
        return vec![(i..l.len(), Style::Heading)];
    }
    // Horizontal rule: three or more of the same - * _ and nothing else.
    if t.len() >= 3 && "-*_".encode_utf16().any(|m| t.iter().filter(|&&c| c != SP).all(|&c| c == m) && t.iter().filter(|&&c| c == m).count() >= 3) {
        return vec![(i..l.len(), Style::Punct)];
    }
    while l.get(i) == Some(&ch(b'>')) {
        out.push((i..i + 1, Style::Punct));
        i = skip_spaces(l, i + 1);
    }
    let digits = l[i..].iter().take_while(|&&c| is_digit(c)).count();
    let marker = if digits > 0 && matches!(l.get(i + digits), Some(&c) if c == ch(b'.') || c == ch(b')')) {
        digits + 1
    } else if matches!(l.get(i), Some(&c) if c == ch(b'-') || c == ch(b'*') || c == ch(b'+')) {
        1
    } else {
        0
    };
    if marker > 0 && l.get(i + marker).is_none_or(|&c| c == SP) {
        out.push((i..i + marker, Style::Punct));
        i += marker;
    }
    inline(l, i, &mut out);
    out
}

/// Inline code, links and emphasis.
fn inline(l: &[u16], mut i: usize, out: &mut Spans) {
    let find = |from: usize, c: u16| l[from.min(l.len())..].iter().position(|&x| x == c).map(|p| from + p);
    while i < l.len() {
        let c = l[i];
        if c == ch(b'`') {
            let end = find(i + 1, c).map_or(l.len(), |e| e + 1);
            out.push((i..end, Style::Code));
            i = end;
        } else if c == ch(b'[') {
            match find(i + 1, ch(b']')) {
                Some(close) if l.get(close + 1) == Some(&ch(b'(')) => {
                    let end = find(close + 2, ch(b')')).map_or(l.len(), |e| e + 1);
                    out.push((i..close + 1, Style::Link));
                    out.push((close + 1..end, Style::Punct));
                    i = end;
                }
                _ => i += 1,
            }
        } else if (c == ch(b'*') || c == ch(b'_')) && l.get(i + 1).is_some_and(|&n| n != SP) {
            // *italic*, **bold**, ***both*** (or with _): up to the next run of the same marker.
            let run = l[i..].iter().take(3).take_while(|&&x| x == c).count();
            let close = (i + run + 1..l.len()).find(|&j| l[j..].starts_with(&vec![c; run]) && l[j - 1] != SP);
            match close {
                Some(j) => {
                    let style = [Style::Italic, Style::Bold, Style::BoldItalic][run - 1];
                    out.extend([(i..i + run, Style::Punct), (i + run..j, style), (j..j + run, Style::Punct)]);
                    i = j + run;
                }
                None => i += run,
            }
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(lang: Lang, line: &str) -> Vec<(String, Style)> {
        let l: Vec<u16> = line.encode_utf16().collect();
        let mut ctx = context(lang, std::iter::once(&l[..]), 0);
        line_spans(lang, &l, &mut ctx).into_iter().map(|(r, s)| (String::from_utf16_lossy(&l[r]), s)).collect()
    }

    fn has(v: &[(String, Style)], text: &str, s: Style) -> bool {
        v.iter().any(|(t, st)| t == text && *st == s)
    }

    #[test]
    fn json_tokens() {
        let v = spans(Lang::Json, r#"  "name": "x \"y\"", "n": -1.5e3, "ok": true, "z": null"#);
        assert!(has(&v, "\"name\"", Style::Key));
        assert!(has(&v, r#""x \"y\"""#, Style::Str));
        assert!(has(&v, "-1.5e3", Style::Num));
        assert!(has(&v, "true", Style::Keyword));
        assert!(has(&v, "null", Style::Keyword));
    }

    #[test]
    fn yaml_tokens() {
        let v = spans(Lang::Yaml, "  - name: \"web # not a comment\" # real comment");
        assert!(has(&v, "-", Style::Punct));
        assert!(has(&v, "name", Style::Key));
        assert!(has(&v, "\"web # not a comment\"", Style::Str));
        assert!(has(&v, "# real comment", Style::Comment));
        assert!(has(&spans(Lang::Yaml, "port: 8080"), "8080", Style::Num));
        assert!(has(&spans(Lang::Yaml, "debug: true"), "true", Style::Keyword));
        assert!(has(&spans(Lang::Yaml, "url: http://x:80/a"), "http://x:80/a", Style::Str));
        assert!(has(&spans(Lang::Yaml, "base: &base"), "&base", Style::Punct));
        assert!(has(&spans(Lang::Yaml, "---"), "---", Style::Punct));
    }

    #[test]
    fn csv_columns() {
        let v = spans(Lang::Csv, r#"a,"b,c",d"#);
        assert_eq!(v, [("a".into(), Style::Column(0)), ("\"b,c\"".into(), Style::Column(1)), ("d".into(), Style::Column(2))]);
        assert_eq!(spans(Lang::Csv, "a;b,c;d")[1], ("b,c".into(), Style::Column(1))); // semicolons are the most common
    }

    #[test]
    fn markdown_tokens() {
        assert!(has(&spans(Lang::Markdown, "## Title"), "## Title", Style::Heading));
        let v = spans(Lang::Markdown, "- see `code`, [docs](http://x) and **bold** _it_");
        assert!(has(&v, "-", Style::Punct));
        assert!(has(&v, "`code`", Style::Code));
        assert!(has(&v, "[docs]", Style::Link));
        assert!(has(&v, "(http://x)", Style::Punct));
        assert!(has(&v, "bold", Style::Bold));
        assert!(has(&v, "**", Style::Punct));
        assert!(has(&v, "it", Style::Italic));
        assert!(has(&spans(Lang::Markdown, "a ***both*** b"), "both", Style::BoldItalic));
        assert!(has(&spans(Lang::Markdown, "__strong__"), "strong", Style::Bold));
        assert!(has(&spans(Lang::Markdown, "1. item"), "1.", Style::Punct));
        assert!(has(&spans(Lang::Markdown, "---"), "---", Style::Punct));
    }

    #[test]
    fn ini_tokens() {
        assert!(has(&spans(Lang::Ini, "[server]"), "[server]", Style::Heading));
        assert!(has(&spans(Lang::Ini, "; note"), "; note", Style::Comment));
        let v = spans(Lang::Ini, "font = Consolas");
        assert!(has(&v, "font", Style::Key));
        assert!(has(&v, "=", Style::Punct));
        assert!(has(&v, "Consolas", Style::Str));
        assert!(has(&spans(Lang::Ini, "size=11"), "11", Style::Num));
        assert!(has(&spans(Lang::Ini, "use_spaces = false"), "false", Style::Keyword));
    }

    #[test]
    fn xml_tokens() {
        let v = spans(Lang::Xml, r#"<item id="1" ns:x='y'>a &amp; b</item><br/>"#);
        assert!(has(&v, "<", Style::Punct));
        assert!(has(&v, "item", Style::Tag));
        assert!(has(&v, "id", Style::Attr));
        assert!(has(&v, "\"1\"", Style::Str));
        assert!(has(&v, "ns:x", Style::Attr));
        assert!(has(&v, "&amp;", Style::Keyword));
        assert!(has(&v, "</", Style::Punct));
        assert!(has(&v, "/>", Style::Punct));
        assert!(has(&spans(Lang::Xml, "<?xml version=\"1.0\"?>"), "?>", Style::Punct));
        assert!(has(&spans(Lang::Xml, "x <!-- c --> y"), "<!-- c -->", Style::Comment));
    }

    #[test]
    fn xml_comments_and_tags_span_lines() {
        let lines: Vec<Vec<u16>> = ["<!-- start", "<still> comment -->", "<a", "  href=\"x\">"].iter().map(|s| s.encode_utf16().collect()).collect();
        let mut ctx = context(Lang::Xml, lines.iter().map(|l| &l[..]), 1);
        assert_eq!(line_spans(Lang::Xml, &lines[1], &mut ctx)[0], (0..19, Style::Comment));
        line_spans(Lang::Xml, &lines[2], &mut ctx);
        let v = line_spans(Lang::Xml, &lines[3], &mut ctx);
        assert_eq!(v[0], (2..6, Style::Attr)); // attribute on the line after the tag name
    }

    #[test]
    fn bash_tokens() {
        let v = spans(Lang::Bash, r#"if [ "$name" = 'x' ]; then echo ${HOME} $1 $@ 42 # done"#);
        assert!(has(&v, "if", Style::Keyword));
        assert!(has(&v, "\"$name\"", Style::Str));
        assert!(has(&v, "'x'", Style::Str));
        assert!(has(&v, "then", Style::Keyword));
        assert!(has(&v, "${HOME}", Style::Key));
        assert!(has(&v, "$1", Style::Key));
        assert!(has(&v, "$@", Style::Key));
        assert!(has(&v, "42", Style::Num));
        assert!(has(&v, "# done", Style::Comment));
        assert!(!has(&v, "echo", Style::Keyword));
        assert!(!has(&spans(Lang::Bash, "echo a#b"), "#b", Style::Comment)); // not a comment mid-word
    }

    fn multi(lang: Lang, lines: &[&str]) -> Vec<Vec<(String, Style)>> {
        let ls: Vec<Vec<u16>> = lines.iter().map(|s| s.encode_utf16().collect()).collect();
        let mut ctx = context(lang, ls.iter().map(|l| &l[..]), 0);
        ls.iter().map(|l| line_spans(lang, l, &mut ctx).into_iter().map(|(r, s)| (String::from_utf16_lossy(&l[r]), s)).collect()).collect()
    }

    #[test]
    fn python_tokens() {
        let v = spans(Lang::Python, "def f(x): return 42 if x is None else 'a#b' # c");
        assert!(has(&v, "def", Style::Keyword));
        assert!(has(&v, "42", Style::Num));
        assert!(has(&v, "None", Style::Keyword));
        assert!(has(&v, "'a#b'", Style::Str));
        assert!(has(&v, "# c", Style::Comment));
        assert!(has(&spans(Lang::Python, "@dataclass"), "@dataclass", Style::Attr));
        let m = multi(Lang::Python, &["x = \"\"\"doc", "# still string", "end\"\"\" + y"]);
        assert_eq!(m[1], [("# still string".into(), Style::Str)]);
        assert!(has(&m[2], "end\"\"\"", Style::Str));
    }

    #[test]
    fn javascript_tokens() {
        let v = spans(Lang::JavaScript, "const x = await f('s', 3.5); // note");
        assert!(has(&v, "const", Style::Keyword));
        assert!(has(&v, "await", Style::Keyword));
        assert!(has(&v, "'s'", Style::Str));
        assert!(has(&v, "3.5", Style::Num));
        assert!(has(&v, "// note", Style::Comment));
        let m = multi(Lang::JavaScript, &["a /* start", "inside", "end */ let b = `t", "u` + 1"]);
        assert_eq!(m[1], [("inside".into(), Style::Comment)]);
        assert!(has(&m[2], "end */", Style::Comment));
        assert!(has(&m[2], "let", Style::Keyword));
        assert!(has(&m[3], "u`", Style::Str));
    }

    #[test]
    fn powershell_tokens() {
        let v = spans(Lang::PowerShell, "foreach ($f in Get-ChildItem -Path $env:TEMP) { if ($f -eq $null) { Write-Host 'x' } } # c");
        assert!(has(&v, "foreach", Style::Keyword));
        assert!(has(&v, "$f", Style::Key));
        assert!(has(&v, "Get-ChildItem", Style::Tag));
        assert!(has(&v, "-Path", Style::Attr));
        assert!(has(&v, "$env:TEMP", Style::Key));
        assert!(has(&v, "$null", Style::Keyword));
        assert!(has(&v, "Write-Host", Style::Tag));
        assert!(has(&v, "# c", Style::Comment));
        assert!(has(&spans(Lang::PowerShell, "IF ($x) {}"), "IF", Style::Keyword)); // case-insensitive
        let m = multi(Lang::PowerShell, &["<# help", "text #>", "$x = 1"]);
        assert_eq!(m[1], [("text #>".into(), Style::Comment)]);
        assert!(has(&m[2], "$x", Style::Key));
    }

    #[test]
    fn toml_tokens() {
        let v = spans(Lang::Toml, "name = \"minedit\"");
        assert!(has(&v, "name", Style::Key));
        assert!(has(&v, "\"minedit\"", Style::Str));
        assert!(has(&spans(Lang::Toml, "[[bin]]"), "[[bin]]", Style::Heading));
        assert!(has(&spans(Lang::Toml, "# c"), "# c", Style::Comment));
    }

    #[test]
    fn markdown_fences_span_lines() {
        let lines: Vec<Vec<u16>> = ["text", "```rust", "# not a heading", "```", "# heading"].iter().map(|s| s.encode_utf16().collect()).collect();
        let mut ctx = context(Lang::Markdown, lines.iter().map(|l| &l[..]), 2);
        assert_eq!(line_spans(Lang::Markdown, &lines[2], &mut ctx)[0].1, Style::Code);
        assert_eq!(line_spans(Lang::Markdown, &lines[3], &mut ctx)[0].1, Style::Code);
        assert_eq!(line_spans(Lang::Markdown, &lines[4], &mut ctx)[0].1, Style::Heading);
    }
}
