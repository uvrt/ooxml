//! Owns: the Markdown model written as text (docx-agent's `markdown/render.py`): blocks
//! separated by blank lines with an id comment directly above its block; text escaped so
//! it reads back as text; emphasis delimiters never touching whitespace; a block whose marks
//! would not read back as meant written without them (strikethrough first, then emphasis).
//!
//! The read-back check is CommonMark's flanking rules for the delimiters written here
//! (`*`, `**`, `~~`); docx-agent parses the line back with markdown-it, which also applies
//! the rule of three -- the two agree on every delimiter run this writer makes except runs
//! of three or more stars beside punctuation, which it does not make.
//!
//! Must not: read a document.

use crate::markdown::model::{Block, Inline, ListBlock, Mark, Marks, Table};

/// The blocks as Markdown, ending in one newline.
#[must_use]
pub fn render(blocks: &[Block]) -> String {
    let mut lines = render_blocks(blocks);
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n") + "\n"
}

fn render_blocks(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for (k, block) in blocks.iter().enumerate() {
        let lines = render_block(block);
        if lines.is_empty() {
            continue;
        }
        out.extend(lines);
        let attached = matches!(block, Block::Comment(_))
            && blocks
                .get(k + 1)
                .is_some_and(|b| !matches!(b, Block::Comment(_) | Block::Footnote(..)));
        if !attached {
            out.push(String::new());
        }
    }
    out
}

fn trim_blank_end(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn render_block(block: &Block) -> Vec<String> {
    match block {
        Block::Comment(text) => vec![format!("<!-- {text} -->")],
        Block::Paragraph(inline) => {
            let inline = settle(inline, false, false);
            let text = inline_text(&inline, true, false);
            if text.is_empty() {
                Vec::new()
            } else {
                text.split('\n').map(str::to_owned).collect()
            }
        }
        Block::Heading(level, inline) => {
            let inline = settle(inline, true, false);
            let mut text = inline_text(&inline, false, false);
            if text.ends_with('#') {
                text.pop();
                text.push_str("\\#");
            }
            let marker = "#".repeat(usize::try_from(*level).unwrap_or(1));
            vec![format!("{marker} {text}").trim_end().to_owned()]
        }
        Block::Code(text) => {
            let fence = "`".repeat(3.max(longest_run(text, '`') + 1));
            let body = if text.ends_with('\n') || text.is_empty() {
                text.clone()
            } else {
                format!("{text}\n")
            };
            let mut out = vec![fence.clone()];
            let parts: Vec<&str> = body.split('\n').collect();
            out.extend(
                parts
                    .iter()
                    .take(parts.len().saturating_sub(1))
                    .map(|s| (*s).to_owned()),
            );
            out.push(fence);
            out
        }
        Block::Quote(blocks) => trim_blank_end(render_blocks(blocks))
            .into_iter()
            .map(|line| format!("> {line}").trim_end().to_owned())
            .collect(),
        Block::List(list) => render_list(list),
        Block::Table(table) => render_table(table),
        Block::Html(text) => text.trim_end_matches('\n').split('\n').map(str::to_owned).collect(),
        Block::Rule => vec!["---".to_owned()],
        Block::Footnote(label, blocks) => {
            let inner = trim_blank_end(render_blocks(blocks));
            let Some((first, rest)) = inner.split_first() else {
                return vec![format!("[^{label}]:")];
            };
            let mut out = vec![format!("[^{label}]: {first}")];
            out.extend(rest.iter().map(|l| format!("    {l}").trim_end().to_owned()));
            out
        }
    }
}

fn render_list(list: &ListBlock) -> Vec<String> {
    let mut out = Vec::new();
    let mut number = list.start;
    for (k, item) in list.items.iter().enumerate() {
        let marker = if list.ordered {
            let m = format!("{number}{} ", if list.alternate { ')' } else { '.' });
            number += 1;
            m
        } else if list.alternate {
            "* ".to_owned()
        } else {
            "- ".to_owned()
        };
        let inner = render_item(item);
        let Some((first, rest)) = inner.split_first() else {
            out.push(marker.trim_end().to_owned());
            continue;
        };
        let pad = " ".repeat(marker.chars().count());
        out.push(format!("{marker}{first}"));
        out.extend(rest.iter().map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}").trim_end().to_owned()
            }
        }));
        if k + 1 < list.items.len() && inner.iter().any(String::is_empty) {
            out.push(String::new());
        }
    }
    out
}

fn render_item(blocks: &[Block]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (k, block) in blocks.iter().enumerate() {
        let after_comment = k > 0 && blocks.get(k - 1).is_some_and(|b| matches!(b, Block::Comment(_)));
        if !lines.is_empty() && !matches!(block, Block::List(_)) && !after_comment {
            lines.push(String::new());
        }
        lines.extend(render_block(block));
    }
    lines
}

fn render_table(table: &Table) -> Vec<String> {
    let header: Vec<Vec<Inline>> = if table.headed {
        table.header.iter().map(|c| settle(c, false, true)).collect()
    } else {
        table.aligns.iter().map(|_| Vec::new()).collect()
    };
    let row = |cells: &[Vec<Inline>]| {
        let texts: Vec<String> = cells.iter().map(|c| inline_text(c, false, true)).collect();
        format!("| {} |", texts.join(" | "))
    };
    let delimiter: Vec<&str> = table
        .aligns
        .iter()
        .map(|a| match a.as_deref() {
            Some("left") => ":---",
            Some("center") => ":---:",
            Some("right") => "---:",
            _ => "---",
        })
        .collect();
    let mut lines = vec![row(&header), format!("| {} |", delimiter.join(" | "))];
    for cells in &table.rows {
        let settled: Vec<Vec<Inline>> = cells.iter().map(|c| settle(c, false, true)).collect();
        lines.push(row(&settled));
    }
    lines
}

fn longest_run(text: &str, c: char) -> usize {
    let mut best = 0;
    let mut run = 0;
    for x in text.chars() {
        run = if x == c { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

fn delimiter(mark: Mark) -> &'static str {
    match mark {
        Mark::Strong => "**",
        Mark::Em => "*",
        Mark::Strike => "~~",
        Mark::Code => "",
    }
}

fn marks_of(item: &Inline) -> Marks {
    match item {
        Inline::Text { marks, .. } => marks.iter().copied().filter(|m| *m != Mark::Code).collect(),
        _ => Marks::new(),
    }
}

fn href_of(item: &Inline) -> Option<&str> {
    match item {
        Inline::Text { href, .. } | Inline::Image { href, .. } => href.as_deref(),
        _ => None,
    }
}

mod inline;
mod settle;

use inline::inline_text;
use settle::settle;
