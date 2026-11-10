//! Ported from the console's `lib/markdown.ts`.
//!
//! A small Markdown reader for agent reports, safe by construction: it
//! returns plain data (blocks of inline runs), never markup. Every character
//! of a report is drawn as text. What it reads: headings, paragraphs, bullet
//! and numbered lists, quotes, fenced code, rules and pipe tables; inline
//! code, bold, italics and links. A link keeps its URL only for http(s) and
//! mailto; an image is its alt text (a report never makes the app fetch).

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Em(Vec<Inline>),
    Link {
        href: Option<String>,
        c: Vec<Inline>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        c: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        items: Vec<Vec<Inline>>,
    },
    Quote(Vec<Inline>),
    Code {
        lang: Option<String>,
        v: String,
    },
    Rule,
    Table {
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

/// Longest report parsed as Markdown; past it the pane shows it as text.
pub const MARKDOWN_MAX: usize = 1_000_000;

/// A link target the app may follow: http(s) or mailto, else None.
pub fn safe_href(raw: &str) -> Option<String> {
    let url = raw.trim();
    let lower: String = url.chars().take(8).collect::<String>().to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:"))
        .then(|| url.to_string())
}

/// Finds the next token at or after a position, remembering the last answer
/// per token: a paragraph full of openers with no closer would otherwise
/// rescan to its end for each one; with the memo it is linear.
struct NextFinder<'a> {
    text: &'a [char],
    memo: HashMap<&'static str, (usize, Option<usize>)>,
}

impl<'a> NextFinder<'a> {
    fn new(text: &'a [char]) -> Self {
        NextFinder {
            text,
            memo: HashMap::new(),
        }
    }

    fn next(&mut self, token: &'static str, from: usize) -> Option<usize> {
        if let Some(&(m_from, at)) = self.memo.get(token)
            && from >= m_from
            && at.is_none_or(|a| a >= from)
        {
            return at;
        }
        let tok: Vec<char> = token.chars().collect();
        let at = if from > self.text.len() || tok.len() > self.text.len() {
            None
        } else {
            (from..=self.text.len() - tok.len()).find(|&p| self.text[p..p + tok.len()] == tok[..])
        };
        self.memo.insert(token, (from, at));
        at
    }
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Inline runs of one line or paragraph.
pub fn inline(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    inline_chars(&chars)
}

fn inline_chars(text: &[char]) -> Vec<Inline> {
    let mut next = NextFinder::new(text);
    let mut out = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<Inline>| {
        if !buf.is_empty() {
            out.push(Inline::Text(std::mem::take(buf)));
        }
    };
    let at = |i: usize| text.get(i).copied();
    let mut i = 0;
    while i < text.len() {
        let ch = text[i];
        if ch == '\\' && at(i + 1).is_some_and(|c| "\\`*_[]()#!>-".contains(c)) {
            buf.push(text[i + 1]);
            i += 2;
            continue;
        }
        if ch == '`'
            && let Some(end) = next.next("`", i + 1)
        {
            flush(&mut buf, &mut out);
            out.push(Inline::Code(text[i + 1..end].iter().collect()));
            i = end + 1;
            continue;
        }
        if (ch == '*' || ch == '_') && at(i + 1) == Some(ch) {
            let tok = if ch == '*' { "**" } else { "__" };
            if let Some(end) = next.next(tok, i + 2)
                && end > i + 2
            {
                flush(&mut buf, &mut out);
                out.push(Inline::Strong(inline_chars(&text[i + 2..end])));
                i = end + 2;
                continue;
            }
        }
        if ch == '*' || (ch == '_' && (i == 0 || !is_word(text[i - 1]))) {
            let tok = if ch == '*' { "*" } else { "_" };
            if let Some(end) = next.next(tok, i + 1)
                && end > i + 1
                && at(i + 1) != Some(' ')
            {
                flush(&mut buf, &mut out);
                out.push(Inline::Em(inline_chars(&text[i + 1..end])));
                i = end + 1;
                continue;
            }
        }
        let image = ch == '!' && at(i + 1) == Some('[');
        if ch == '[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = next.next("]", open + 1)
                && at(close + 1) == Some('(')
                && let Some(end) = next.next(")", close + 2)
            {
                flush(&mut buf, &mut out);
                let label = &text[open + 1..close];
                if image {
                    let alt: String = label.iter().collect();
                    out.push(Inline::Text(if alt.is_empty() {
                        "image".into()
                    } else {
                        alt
                    }));
                } else {
                    let url: String = text[close + 2..end].iter().collect();
                    out.push(Inline::Link {
                        href: safe_href(&url),
                        c: inline_chars(label),
                    });
                }
                i = end + 1;
                continue;
            }
        }
        buf.push(ch);
        i += 1;
    }
    flush(&mut buf, &mut out);
    out
}

fn cells(line: &str) -> Vec<String> {
    let mut row = line.trim();
    row = row.strip_prefix('|').unwrap_or(row);
    row = row.strip_suffix('|').unwrap_or(row);
    row.split('|').map(|c| c.trim().to_string()).collect()
}

/// Up to three leading whitespace characters, then the rest (`^\s{0,3}`).
/// None when there are four or more.
fn indent3(line: &str) -> Option<&str> {
    let mut rest = line;
    for _ in 0..3 {
        match rest.chars().next() {
            Some(c) if c.is_whitespace() => rest = &rest[c.len_utf8()..],
            _ => return Some(rest),
        }
    }
    match rest.chars().next() {
        Some(c) if c.is_whitespace() => None,
        _ => Some(rest),
    }
}

/// `^\s{0,3}([-*_])(\s*\1){2,}\s*$`
fn is_rule(line: &str) -> bool {
    let Some(rest) = indent3(line) else {
        return false;
    };
    let Some(c) = rest.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else {
        return false;
    };
    rest.chars().all(|x| x == c || x.is_whitespace())
        && rest.chars().filter(|&x| x == c).count() >= 3
}

/// `^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$`
fn is_divider(line: &str) -> bool {
    let mut row = line.trim();
    row = row.strip_prefix('|').unwrap_or(row);
    row = row.strip_suffix('|').unwrap_or(row);
    row.split('|').all(|cell| {
        let c = cell.trim();
        let c = c.strip_prefix(':').unwrap_or(c);
        let c = c.strip_suffix(':').unwrap_or(c);
        c.len() >= 2 && c.chars().all(|x| x == '-')
    })
}

/// `^\s*(```|~~~)\s*([\w+-]*)` → (fence, info word).
fn fence(line: &str) -> Option<(&'static str, String)> {
    let rest = line.trim_start();
    let f = ["```", "~~~"].into_iter().find(|f| rest.starts_with(f))?;
    let info = rest[3..].trim_start();
    let lang: String = info
        .chars()
        .take_while(|&c| is_word(c) || c == '+' || c == '-')
        .collect();
    Some((f, lang))
}

/// `^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$` → (level, content).
fn heading(line: &str) -> Option<(u8, &str)> {
    let rest = indent3(line)?;
    let hashes = rest.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after = &rest[hashes..];
    if !after.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let content = after
        .trim_start()
        .trim_end()
        .trim_end_matches('#')
        .trim_end();
    Some((hashes as u8, content))
}

/// `^\s{0,3}>`
fn is_quote(line: &str) -> bool {
    indent3(line).is_some_and(|r| r.starts_with('>'))
}

/// `^\s{0,3}([-*+]|\d{1,3}[.)])\s+(.*)$` → (ordered, content).
fn list_item(line: &str) -> Option<(bool, &str)> {
    let rest = indent3(line)?;
    let (ordered, after) = if let Some(a) = rest.strip_prefix(['-', '*', '+']) {
        (false, a)
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if !(1..=3).contains(&digits) {
            return None;
        }
        (true, rest[digits..].strip_prefix(['.', ')'])?)
    };
    if !after.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    Some((ordered, after.trim_start()))
}

/// `^\s{2,}\S`
fn is_continuation(line: &str) -> bool {
    let ws = line.chars().take_while(|c| c.is_whitespace()).count();
    ws >= 2 && line.chars().nth(ws).is_some()
}

fn normalize(source: &str) -> String {
    source.replace("\r\n", "\n").replace('\r', "\n")
}

/// The blocks of a Markdown document.
pub fn parse(source: &str) -> Vec<Block> {
    let normalized = normalize(source);
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut blocks = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let end_para = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph(inline(&para.join(" "))));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((f, lang)) = fence(line) {
            end_para(&mut para, &mut blocks);
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with(f) {
                body.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::Code {
                lang: (!lang.is_empty()).then_some(lang),
                v: body.join("\n"),
            });
            i += 1;
            continue;
        }
        if line.trim().is_empty() {
            end_para(&mut para, &mut blocks);
            i += 1;
            continue;
        }
        if let Some((level, content)) = heading(line) {
            end_para(&mut para, &mut blocks);
            blocks.push(Block::Heading {
                level,
                c: inline(content),
            });
            i += 1;
            continue;
        }
        if is_rule(line) {
            end_para(&mut para, &mut blocks);
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }
        if line.contains('|') && i + 1 < lines.len() && is_divider(lines[i + 1]) {
            end_para(&mut para, &mut blocks);
            let head = cells(line).iter().map(|c| inline(c)).collect();
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(cells(lines[i]).iter().map(|c| inline(c)).collect());
                i += 1;
            }
            blocks.push(Block::Table { head, rows });
            continue;
        }
        if is_quote(line) {
            end_para(&mut para, &mut blocks);
            let mut body = Vec::new();
            while i < lines.len() && is_quote(lines[i]) {
                let rest = indent3(lines[i]).unwrap_or("");
                let rest = &rest[1..];
                let rest = match rest.chars().next() {
                    Some(c) if c.is_whitespace() => &rest[c.len_utf8()..],
                    _ => rest,
                };
                body.push(rest);
                i += 1;
            }
            blocks.push(Block::Quote(inline(&body.join(" "))));
            continue;
        }
        if let Some((ordered, _)) = list_item(line) {
            end_para(&mut para, &mut blocks);
            let mut items: Vec<Vec<Inline>> = Vec::new();
            while i < lines.len() {
                let l = lines[i];
                match list_item(l) {
                    Some((o, content)) if o == ordered => items.push(inline(content)),
                    _ if !l.trim().is_empty() && is_continuation(l) && !items.is_empty() => {
                        // A continuation line of the last item.
                        if let Some(last) = items.last_mut() {
                            last.push(Inline::Text(format!(" {}", l.trim())));
                        }
                    }
                    _ => break,
                }
                i += 1;
            }
            blocks.push(Block::List { ordered, items });
            continue;
        }
        para.push(line.trim());
        i += 1;
    }
    end_para(&mut para, &mut blocks);
    blocks
}

/// What a line of a unified diff does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Add,
    Del,
    Hunk,
    File,
    Context,
}

/// The lines of a unified diff, by what they do.
pub fn diff_lines(source: &str) -> Vec<(DiffKind, String)> {
    normalize(source)
        .split('\n')
        .map(|text| {
            let kind = if ["+++", "---", "diff ", "index "]
                .iter()
                .any(|p| text.starts_with(p))
            {
                DiffKind::File
            } else if text.starts_with("@@") {
                DiffKind::Hunk
            } else if text.starts_with('+') {
                DiffKind::Add
            } else if text.starts_with('-') {
                DiffKind::Del
            } else {
                DiffKind::Context
            };
            (kind, text.to_string())
        })
        .collect()
}

/// Lines shown before "Show everything".
pub const PREVIEW_LINES: usize = 24;

/// The first lines of a text, and how many more there are.
pub fn preview(source: &str, lines: usize) -> (String, usize) {
    let normalized = normalize(source);
    let body = normalized.strip_suffix('\n').unwrap_or(&normalized);
    let all: Vec<&str> = body.split('\n').collect();
    let shown = all[..lines.min(all.len())].join("\n");
    (shown, all.len().saturating_sub(lines))
}

/// JSON pretty-printed with two spaces when it parses, as it is otherwise.
pub fn pretty_json(source: &str) -> String {
    serde_json::from_str::<serde_json::Value>(source)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| source.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(v: &str) -> Inline {
        Inline::Text(v.into())
    }

    fn kind(b: &Block) -> &'static str {
        match b {
            Block::Heading { .. } => "heading",
            Block::Paragraph(_) => "paragraph",
            Block::List { .. } => "list",
            Block::Quote(_) => "quote",
            Block::Code { .. } => "code",
            Block::Rule => "rule",
            Block::Table { .. } => "table",
        }
    }

    #[test]
    fn blocks() {
        let blocks =
            parse("# T\n\nuno\ndos\n\n- a\n- b\n\n1. x\n\n> cita\n\n```sh\nls <x>\n```\n\n---");
        let kinds: Vec<_> = blocks.iter().map(kind).collect();
        assert_eq!(
            kinds,
            [
                "heading",
                "paragraph",
                "list",
                "list",
                "quote",
                "code",
                "rule"
            ]
        );
        assert_eq!(blocks[1], Block::Paragraph(vec![text("uno dos")]));
        assert_eq!(
            blocks[5],
            Block::Code {
                lang: Some("sh".into()),
                v: "ls <x>".into()
            }
        );
        assert_eq!(
            blocks[2],
            Block::List {
                ordered: false,
                items: vec![vec![text("a")], vec![text("b")]]
            }
        );
        assert_eq!(
            blocks[3],
            Block::List {
                ordered: true,
                items: vec![vec![text("x")]]
            }
        );
    }

    #[test]
    fn raw_html_stays_text_links_only_safe_images_are_alt() {
        assert_eq!(
            inline("<script>x</script>"),
            vec![text("<script>x</script>")]
        );
        assert_eq!(
            inline("[a](javascript:alert(1))")[0],
            Inline::Link {
                href: None,
                c: vec![text("a")]
            }
        );
        assert_eq!(safe_href("https://x.es").as_deref(), Some("https://x.es"));
        assert_eq!(
            safe_href("  MAILTO:a@b.es ").as_deref(),
            Some("MAILTO:a@b.es")
        );
        assert_eq!(safe_href("data:text/html,x"), None);
        assert_eq!(inline("![capture](https://x/p.png)"), vec![text("capture")]);
        assert_eq!(inline("![](https://x/p.png)"), vec![text("image")]);
    }

    #[test]
    fn diff_lines_and_previews() {
        let kinds: Vec<_> = diff_lines("+a\n-b\n@@ x\n c")
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            kinds,
            [
                DiffKind::Add,
                DiffKind::Del,
                DiffKind::Hunk,
                DiffKind::Context
            ]
        );
        let files: Vec<_> = diff_lines("--- a\n+++ b\ndiff x\nindex 1")
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(files, [DiffKind::File; 4]);
        assert_eq!(preview("1\n2\n3\n", 2), ("1\n2".into(), 1));
        assert_eq!(preview("1\r\n2", 24), ("1\n2".into(), 0));
    }

    #[test]
    fn unclosed_brackets_stay_linear() {
        let source = "[a ".repeat(130_000);
        let t0 = std::time::Instant::now();
        let blocks = parse(&source);
        // Unoptimised test builds are slower than the console's engine; the
        // quadratic version takes minutes here, so a second still proves it.
        assert!(t0.elapsed().as_millis() < 1000, "took {:?}", t0.elapsed());
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn links_and_code_close_where_they_should() {
        assert_eq!(
            inline("[a](https://x.es) y `b` y [c"),
            vec![
                Inline::Link {
                    href: Some("https://x.es".into()),
                    c: vec![text("a")]
                },
                text(" y "),
                Inline::Code("b".into()),
                text(" y [c"),
            ]
        );
    }

    #[test]
    fn emphasis_rules() {
        assert_eq!(
            inline("**b** *e* __s__ _i_"),
            vec![
                Inline::Strong(vec![text("b")]),
                text(" "),
                Inline::Em(vec![text("e")]),
                text(" "),
                Inline::Strong(vec![text("s")]),
                text(" "),
                Inline::Em(vec![text("i")]),
            ]
        );
        // `_` inside a word is not emphasis; `* x*` opens nothing.
        assert_eq!(inline("snake_case_name"), vec![text("snake_case_name")]);
        assert_eq!(inline("a * b*"), vec![text("a * b*")]);
        assert_eq!(inline("\\*x\\*"), vec![text("*x*")]);
        assert_eq!(inline("`open"), vec![text("`open")]);
    }

    #[test]
    fn headings_rules_tables_quotes_lists() {
        assert_eq!(
            parse("## Title ##")[0],
            Block::Heading {
                level: 2,
                c: vec![text("Title")]
            }
        );
        assert_eq!(kind(&parse("####### no")[0]), "paragraph");
        assert_eq!(kind(&parse("    ---")[0]), "paragraph");
        assert_eq!(parse("* * *")[0], Block::Rule);
        assert_eq!(
            parse("| a | b |\n|---|:--:|\n| 1 | 2 |\n| 3 |\nafter")[0],
            Block::Table {
                head: vec![vec![text("a")], vec![text("b")]],
                rows: vec![
                    vec![vec![text("1")], vec![text("2")]],
                    vec![vec![text("3")]]
                ],
            }
        );
        assert_eq!(
            parse("> one\n>two\nend")[0],
            Block::Quote(vec![text("one two")])
        );
        assert_eq!(
            parse("- a\n  more\n1. b")[0],
            Block::List {
                ordered: false,
                items: vec![vec![text("a"), text(" more")]]
            }
        );
        assert_eq!(
            parse("~~~\nx\n").last(),
            Some(&Block::Code {
                lang: None,
                v: "x\n".into()
            })
        );
    }

    #[test]
    fn pretty_json_indents_or_leaves_alone() {
        assert_eq!(
            pretty_json("{\"b\":1,\"a\":[1]}"),
            "{\n  \"b\": 1,\n  \"a\": [\n    1\n  ]\n}"
        );
        assert_eq!(pretty_json("not json"), "not json");
    }
}
