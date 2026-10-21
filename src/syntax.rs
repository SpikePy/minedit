//! Syntax highlighting for JSON, YAML, CSV, Markdown, INI and XML, one line at a time.

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
}

impl Lang {
    pub fn from_ext(ext: &str) -> Lang {
        match ext.to_ascii_lowercase().as_str() {
            "json" | "jsonc" | "geojson" => Lang::Json,
            "yaml" | "yml" => Lang::Yaml,
            "csv" | "tsv" => Lang::Csv,
            "md" | "markdown" => Lang::Markdown,
            "ini" | "cfg" | "conf" | "properties" => Lang::Ini,
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
    Emphasis,
    Column(u8),
    Tag,
    Attr,
}

/// Text color (0x00BBGGRR) for a style.
pub fn color(s: Style) -> u32 {
    const COLUMNS: [u32; 8] = [0xA55104, 0x1515A3, 0x588609, 0x265E79, 0xDB00AF, 0x801000, 0x3F1F81, 0x997F26];
    match s {
        Style::Key => 0xA55104,
        Style::Str => 0x1515A3,
        Style::Num => 0x588609,
        Style::Keyword => 0xFF0000,
        Style::Comment => 0x008000,
        Style::Punct => 0x808080,
        Style::Heading => 0x000080,
        Style::Code => 0x265E79,
        Style::Link => 0xA55104,
        Style::Emphasis => 0x800080,
        Style::Column(i) => COLUMNS[i as usize % COLUMNS.len()],
        Style::Tag => 0x000080,
        Style::Attr => 0x0000E5,
    }
}

/// State carried from one line to the next.
#[derive(Clone, Copy, Default)]
pub struct Ctx {
    fence: bool,   // inside a Markdown code block
    sep: u16,      // CSV separator
    comment: bool, // inside an XML comment
    tag: bool,     // inside an XML tag, between its name and '>'
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
        Lang::Markdown => {
            for l in lines.take(upto) {
                ctx.fence ^= is_fence(l);
            }
        }
        Lang::Xml => {
            for l in lines.take(upto) {
                xml(l, &mut ctx);
            }
        }
        _ => {}
    }
    ctx
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
    }
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
            // *em*, **strong**, _em_: up to the next run of the same marker.
            let run = if l.get(i + 1) == Some(&c) { 2 } else { 1 };
            let close = (i + run + 1..l.len()).find(|&j| l[j..].starts_with(&vec![c; run]) && l[j - 1] != SP);
            match close {
                Some(j) => {
                    out.push((i..j + run, Style::Emphasis));
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
        assert!(has(&v, "**bold**", Style::Emphasis));
        assert!(has(&v, "_it_", Style::Emphasis));
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
    fn markdown_fences_span_lines() {
        let lines: Vec<Vec<u16>> = ["text", "```rust", "# not a heading", "```", "# heading"].iter().map(|s| s.encode_utf16().collect()).collect();
        let mut ctx = context(Lang::Markdown, lines.iter().map(|l| &l[..]), 2);
        assert_eq!(line_spans(Lang::Markdown, &lines[2], &mut ctx)[0].1, Style::Code);
        assert_eq!(line_spans(Lang::Markdown, &lines[3], &mut ctx)[0].1, Style::Code);
        assert_eq!(line_spans(Lang::Markdown, &lines[4], &mut ctx)[0].1, Style::Heading);
    }
}
