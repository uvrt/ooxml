//! Owns: one block's inline items as Markdown text (docx-agent's `render._inline` and its
//! helpers): marks opened longest-lasting first and closed innermost first, links with the
//! marks that last through them kept open around them, code spans, escapes, breaks, and
//! the edge spaces CommonMark would strip written as references.
//!
//! Must not: decide which marks a block keeps ([`super::settle`] does).

use super::{delimiter, href_of, marks_of};
use crate::markdown::model::{Inline, Mark, Marks};

/// Characters escaped everywhere.
const ALWAYS: &[char] = &['\\', '`', '*', '_', '[', ']', '<', '~'];

/// The CriticMarkup openers a `{` before them would start.
const CRITIC_OPEN: [&str; 5] = ["++", "--", ">>", "==", "~~"];

/// Whitespace CommonMark trims at a line's edges that a writer must keep as a reference.
const TRIMMED: &[char] = &[
    '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
    '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    '\u{feff}',
];

struct Writer {
    out: Vec<String>,
    open: Vec<Mark>,
    href: Option<String>,
    outer: usize,
    /// Each delimiter written: its byte offset in the text, its length, whether it opens.
    delimiters: Vec<(usize, usize, bool)>,
}

impl Writer {
    fn delimit(&mut self, mark: Mark, opens: bool) {
        let at: usize = self.out.iter().map(String::len).sum();
        let text = delimiter(mark);
        self.delimiters.push((at, text.len(), opens));
        self.out.push(text.to_owned());
    }

    fn close_to(&mut self, keep: &Marks) {
        while self.open.len() > self.outer && !self.open.iter().all(|m| keep.contains(m)) {
            if let Some(m) = self.open.pop() {
                self.delimit(m, false);
            }
        }
    }

    fn close_all(&mut self) {
        self.outer = 0;
        self.close_to(&Marks::new());
    }

    fn close_link(&mut self) {
        if let Some(href) = self.href.take() {
            self.close_to(&Marks::new());
            self.out.push(format!("]({})", destination(&href)));
            self.outer = 0;
        }
    }
}

/// `inline` as Markdown (lines joined by `\n`).
pub(super) fn inline_text(inline: &[Inline], line_start: bool, table: bool) -> String {
    let (text, _) = written(inline, line_start, table);
    edge_spaces(&text)
}

/// `inline` as Markdown before its edge spaces are kept, with the delimiters written:
/// `(byte offset, length, opens)`.
pub(super) fn written(inline: &[Inline], line_start: bool, table: bool) -> (String, Vec<(usize, usize, bool)>) {
    let items = spaced(inline);
    let mut w = Writer {
        out: Vec::new(),
        open: Vec::new(),
        href: None,
        outer: 0,
        delimiters: Vec::new(),
    };
    let mut at_start = line_start;
    let mut after_note = false;
    for (k, item) in items.iter().enumerate() {
        let item_href = href_of(item).map(str::to_owned);
        if item_href != w.href {
            w.close_link();
            if let Some(target) = item_href {
                open_link(&mut w, &items, k, target);
            }
        }
        if let Inline::Text { text, marks, .. } = item {
            let wanted = marks_of(item);
            w.close_to(&wanted);
            let mut opening: Vec<Mark> = wanted.iter().copied().filter(|m| !w.open.contains(m)).collect();
            opening.sort_by_key(|m| std::cmp::Reverse(lasts(&items, k, *m)));
            for m in opening {
                w.delimit(m, true);
                w.open.push(m);
            }
            if marks.contains(&Mark::Code) {
                w.out.push(code(text));
            } else {
                let mut escaped = escape(text, table, after_note);
                if at_start {
                    escaped = escape_line_start(&escaped);
                }
                w.out.push(escaped);
            }
            at_start = at_start && text.trim().is_empty();
            after_note = false;
            continue;
        }
        if w.href.is_none() {
            w.close_all();
        } else {
            w.close_to(&Marks::new());
        }
        match item {
            Inline::Html(text) | Inline::Critic(text) => {
                w.out.push(text.clone());
                at_start = false;
            }
            Inline::Break => {
                w.close_link();
                w.close_all();
                if ends_block(&items, k) {
                    w.out.push("<br>".to_owned());
                    at_start = false;
                } else {
                    w.out.push("\\\n".to_owned());
                    at_start = true;
                }
            }
            Inline::Image { alt, src, title, .. } => {
                let title = title
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .map(|t| format!(" \"{}\"", title_text(t)));
                w.out.push(format!(
                    "![{}]({}{})",
                    escape(alt, table, false),
                    destination(src),
                    title.unwrap_or_default()
                ));
                at_start = false;
            }
            Inline::NoteRef(label) => {
                w.out.push(format!("[^{label}]"));
                at_start = false;
                after_note = true;
                continue;
            }
            Inline::Text { .. } => {}
        }
        after_note = false;
    }
    w.close_link();
    w.close_all();
    (w.out.concat(), w.delimiters)
}

fn open_link(w: &mut Writer, items: &[Inline], k: usize, target: String) {
    let mut lasting: Option<Marks> = None;
    for item in items.get(k..).unwrap_or(&[]) {
        if !matches!(item, Inline::Text { .. } | Inline::Image { .. }) || href_of(item) != Some(target.as_str()) {
            break;
        }
        let marks = marks_of(item);
        lasting = Some(match lasting {
            None => marks,
            Some(l) => l.intersection(&marks).copied().collect(),
        });
    }
    let lasting = lasting.unwrap_or_default();
    while !w.open.is_empty() && !w.open.iter().all(|m| lasting.contains(m)) {
        if let Some(m) = w.open.pop() {
            w.delimit(m, false);
        }
    }
    w.outer = w.open.len();
    if let Some(last) = w.out.last_mut() {
        if last.ends_with('!') && !last.ends_with("\\!") {
            last.pop();
            last.push_str("\\!");
        }
    }
    w.out.push("[".to_owned());
    w.href = Some(target);
}

/// How far from `k` the mark lasts (the longest opens outermost).
fn lasts(items: &[Inline], k: usize, mark: Mark) -> usize {
    let mut n = k;
    while let Some(Inline::Text { marks, .. }) = items.get(n) {
        if !marks.contains(&mark) {
            break;
        }
        n += 1;
    }
    n
}

/// Whether nothing but breaks, whitespace and comments follows `items[k]`.
fn ends_block(items: &[Inline], k: usize) -> bool {
    items.iter().skip(k + 1).all(|i| match i {
        Inline::Break => true,
        Inline::Text { text, .. } => text.trim().is_empty(),
        Inline::Html(text) => text.starts_with("<!--"),
        _ => false,
    })
}

/// Whitespace moved out of a mark where it opens or closes.
pub(super) fn spaced(inline: &[Inline]) -> Vec<Inline> {
    let neighbour = |k: Option<usize>| -> Marks {
        match k.and_then(|k| inline.get(k)) {
            Some(item @ Inline::Text { text, .. }) if !text.is_empty() => marks_of(item),
            _ => Marks::new(),
        }
    };
    let mut out = Vec::new();
    for (current, item) in inline.iter().enumerate() {
        let Inline::Text { text, marks, href } = item else {
            out.push(item.clone());
            continue;
        };
        if marks.is_empty() || text.is_empty() || marks.contains(&Mark::Code) {
            out.push(item.clone());
            continue;
        }
        let before = neighbour(current.checked_sub(1));
        let after = neighbour(Some(current + 1));
        let core = text.trim_matches([' ', '\t']);
        let piece = |t: &str, m: Marks| Inline::Text {
            text: t.to_owned(),
            marks: m,
            href: href.clone(),
        };
        if core.is_empty() {
            let shared: Marks = marks
                .iter()
                .copied()
                .filter(|m| before.contains(m) && after.contains(m))
                .collect();
            out.push(piece(text, shared));
            continue;
        }
        let lead = &text[..text.len() - text.trim_start_matches([' ', '\t']).len()];
        let trail = &text[text.trim_end_matches([' ', '\t']).len()..];
        if !lead.is_empty() {
            out.push(piece(lead, marks.intersection(&before).copied().collect()));
        }
        out.push(piece(core, marks.clone()));
        if !trail.is_empty() {
            out.push(piece(trail, marks.intersection(&after).copied().collect()));
        }
    }
    out
}

fn code(text: &str) -> String {
    let fence = "`".repeat(super::longest_run(text, '`') + 1);
    let pad = text.starts_with('`')
        || text.ends_with('`')
        || (text.starts_with(' ') && text.ends_with(' ') && !text.trim_matches(' ').is_empty());
    if pad {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

/// Whether an entity reference (`&amp;`, `&#38;`, `&#x26;`) starts at `rest`.
fn entity_at(rest: &str) -> bool {
    let Some(body) = rest.strip_prefix('&') else {
        return false;
    };
    let Some(end) = body.find(';') else { return false };
    let name = &body[..end];
    if let Some(num) = name.strip_prefix('#') {
        if let Some(hex) = num.strip_prefix(['x', 'X']) {
            return (1..=6).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit());
        }
        return (1..=7).contains(&num.len()) && num.bytes().all(|b| b.is_ascii_digit());
    }
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && (2..=32).contains(&name.len())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

fn escape(text: &str, table: bool, after_note: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for (k, (at, c)) in text.char_indices().enumerate() {
        let rest = &text[at..];
        if ALWAYS.contains(&c) || (table && c == '|') {
            out.push('\\');
            out.push(c);
        } else if c == '&' && entity_at(rest) {
            out.push_str("\\&");
        } else if c == '{' && CRITIC_OPEN.iter().any(|o| rest[1..].starts_with(o)) {
            out.push_str("\\{");
        } else if c == ':' && k == 0 && after_note {
            out.push_str("\\:");
        } else if c == '\n' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_line_start(text: &str) -> String {
    let stripped = text.trim_start_matches([' ', '\t']);
    if stripped.starts_with(['#', '>', '+', '=', '-']) {
        return format!("\\{stripped}");
    }
    let digits = stripped.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        if let Some(rest) = stripped[digits..].strip_prefix(['.', ')']) {
            let mark = &stripped[digits..=digits];
            return format!("{}\\{mark}{rest}", &stripped[..digits]);
        }
    }
    stripped.to_owned()
}

fn destination(url: &str) -> String {
    if url.chars().any(|c| c.is_whitespace() || "()<>".contains(c)) {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_owned()
    }
}

fn title_text(title: &str) -> String {
    title.replace('\\', "\\\\").replace('"', "\\\"")
}

fn edge_spaces(text: &str) -> String {
    let trimmed = |c: char| TRIMMED.contains(&c) || c == ' ' || c == '\t';
    text.split('\n')
        .map(|line| {
            let chars: Vec<char> = line.chars().collect();
            let start = chars.iter().take_while(|c| trimmed(**c)).count();
            let end = chars.len() - chars[start..].iter().rev().take_while(|c| trimmed(**c)).count();
            let edges = chars[..start].iter().chain(&chars[end..]);
            if !edges.clone().any(|c| TRIMMED.contains(c)) {
                return line.to_owned();
            }
            let reference = |c: &char| {
                if TRIMMED.contains(c) {
                    format!("&#x{:X};", u32::from(*c))
                } else {
                    c.to_string()
                }
            };
            let lead: String = chars[..start].iter().map(reference).collect();
            let middle: String = chars[start..end].iter().collect();
            let trail: String = chars[end..].iter().map(reference).collect();
            format!("{lead}{middle}{trail}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
