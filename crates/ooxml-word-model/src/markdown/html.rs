//! Owns: raw HTML for a table GFM cannot say (docx-agent's `markdown/html.py`): merged
//! cells, nested tables, several paragraphs in a cell -- one HTML block, ids on the table
//! and its cells, the paragraphs' comments inside the cells.
//!
//! Must not: read records beyond the table it is given.

use crate::markdown::model::{Block, Inline, Mark};
use crate::markdown::read::{build, Reader, TableRecord};

/// `html.escape` with quotes.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "&#10;")
        .replace('\u{b}', "<br>")
}

/// The table as an HTML block.
pub fn table_html(reader: &mut Reader<'_>, table: &TableRecord) -> String {
    let mut lines = vec![if reader.ids {
        format!("<table data-id=\"{}\">", escape(&table.id))
    } else {
        "<table>".to_owned()
    }];
    for (k, line) in table.rows.iter().enumerate() {
        let tag = if k < table.header_rows { "th" } else { "td" };
        let mut cells = String::new();
        for cell in line {
            let mut attributes = if reader.ids {
                format!(" data-id=\"{}\"", escape(&cell.id))
            } else {
                String::new()
            };
            if cell.row_span > 1 {
                attributes.push_str(&format!(" rowspan=\"{}\"", cell.row_span));
            }
            if cell.view_span > 1 {
                attributes.push_str(&format!(" colspan=\"{}\"", cell.view_span));
            }
            let blocks = build(reader, &cell.records, false);
            cells.push_str(&format!("<{tag}{attributes}>{}</{tag}>", blocks_html(&blocks)));
        }
        lines.push(format!("<tr>{cells}</tr>"));
    }
    lines.push("</table>".to_owned());
    lines.join("\n")
}

fn blocks_html(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            Block::Comment(t) => out.push_str(&format!("<!-- {t} -->")),
            Block::Paragraph(i) => out.push_str(&format!("<p>{}</p>", inline_html(i))),
            Block::Heading(l, i) => out.push_str(&format!("<h{l}>{}</h{l}>", inline_html(i))),
            Block::Code(t) => out.push_str(&format!("<pre><code>{}</code></pre>", text(t))),
            Block::Quote(b) => out.push_str(&format!("<blockquote>{}</blockquote>", blocks_html(b))),
            Block::List(list) => {
                let tag = if list.ordered { "ol" } else { "ul" };
                let start = if list.ordered && list.start != 1 {
                    format!(" start=\"{}\"", list.start)
                } else {
                    String::new()
                };
                let items: String = list
                    .items
                    .iter()
                    .map(|i| format!("<li>{}</li>", blocks_html(i)))
                    .collect();
                out.push_str(&format!("<{tag}{start}>{items}</{tag}>"));
            }
            Block::Table(t) => {
                let rows = std::iter::once(&t.header).chain(&t.rows);
                let body: String = rows
                    .map(|row| {
                        format!(
                            "<tr>{}</tr>",
                            row.iter()
                                .map(|c| format!("<td>{}</td>", inline_html(c)))
                                .collect::<String>()
                        )
                    })
                    .collect();
                out.push_str(&format!("<table>{body}</table>"));
            }
            Block::Html(t) => out.push_str(&t.replace('\n', "")),
            Block::Rule => out.push_str("<hr>"),
            Block::Footnote(..) => {}
        }
    }
    out
}

fn inline_html(inline: &[Inline]) -> String {
    let mut out = String::new();
    for item in inline {
        match item {
            Inline::Text { text: t, marks, href } => {
                let mut s = text(t);
                for (mark, tag) in [
                    (Mark::Code, "code"),
                    (Mark::Em, "em"),
                    (Mark::Strong, "strong"),
                    (Mark::Strike, "del"),
                ] {
                    if marks.contains(&mark) {
                        s = format!("<{tag}>{s}</{tag}>");
                    }
                }
                if let Some(href) = href {
                    s = format!("<a href=\"{}\">{s}</a>", escape(href));
                }
                out.push_str(&s);
            }
            Inline::Html(h) => out.push_str(h),
            Inline::Critic(c) => out.push_str(&text(c)),
            Inline::Break => out.push_str("<br>"),
            Inline::Image { alt, src, title, href } => {
                let title = title
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .map(|t| format!(" title=\"{}\"", escape(t)))
                    .unwrap_or_default();
                let image = format!("<img src=\"{}\" alt=\"{}\"{title}>", escape(src), escape(alt));
                match href.as_deref().filter(|h| !h.is_empty()) {
                    Some(h) => out.push_str(&format!("<a href=\"{}\">{image}</a>", escape(h))),
                    None => out.push_str(&image),
                }
            }
            Inline::NoteRef(l) => out.push_str(&format!("<sup>[^{}]</sup>", escape(l))),
        }
    }
    out
}
