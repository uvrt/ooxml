//! Owns: records grouped into the Markdown model (docx-agent's `read.build` and
//! `markdown.project`): lists nested by level, quotes, code blocks, tables (GFM where GFM
//! can say them, HTML where it cannot), id comments, section breaks, text boxes, footnote
//! definitions; and which stories a read covers.
//!
//! Must not: read elements beyond the records, or render text.

use super::inline::one_line;
use super::{comment, comment_text, note_elements, Kind, ParagraphRecord, Reader, Record, TableRecord};
use crate::address::Address;
use crate::docx::Docx;
use crate::ids::W;
use crate::markdown::model::{has_content, Block, Inline, ListBlock, Table};
use crate::markdown::select::{select, RangeError, Selected};
use crate::markdown::{Stories, ViewName};

/// The model blocks `to_markdown` writes for `range` (`None`: the body) of `docx`, with
/// the id comments or (`ids` off) as plain Markdown.
///
/// # Errors
/// A [`RangeError`] when `range` names nothing.
pub fn project(
    docx: &Docx,
    range: Option<&Address>,
    view: ViewName,
    stories: Stories,
    ids: bool,
) -> Result<Vec<Block>, RangeError> {
    let mut reader = Reader::new(docx, view);
    reader.ids = ids;
    let mut blocks = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let notes = note_elements(docx);
    for selected in select(&mut reader, range)? {
        match selected {
            Selected::Notes(part) => labels.extend(notes.iter().filter(|n| n.0 == part).map(|n| n.1.clone())),
            Selected::Records { story, records, .. } => {
                if let Some(story) = story.filter(|_| ids) {
                    blocks.push(Block::Comment(format!("story: {story}")));
                }
                blocks.extend(build(&mut reader, &records, false));
                labels.extend(referenced(&records));
            }
        }
    }
    if stories == Stories::All {
        let mut every = Vec::new();
        for part in &docx.parts {
            if !has_story_content(part.root()) {
                continue;
            }
            let root = part.root();
            if root.name().is(Some(W), "hdr") || root.name().is(Some(W), "ftr") {
                let kind = if root.name().is(Some(W), "hdr") {
                    "header"
                } else {
                    "footer"
                };
                let records = reader.story(part);
                if ids {
                    blocks.push(Block::Comment(format!("story: {} ({kind})", part.story)));
                }
                blocks.extend(build(&mut reader, &records, false));
                labels.extend(referenced(&records));
            } else if root.name().is(Some(W), "footnotes") || root.name().is(Some(W), "endnotes") {
                every.push(part.part.clone());
            }
        }
        labels.extend(notes.iter().filter(|n| every.contains(&n.0)).map(|n| n.1.clone()));
    }
    let mut seen = std::collections::HashSet::new();
    labels.retain(|l| seen.insert(l.clone()));
    blocks.extend(note_definitions(&mut reader, labels));
    Ok(blocks)
}

/// Whether a story holds anything a reader sees.
fn has_story_content(root: &ooxml_xml::Element) -> bool {
    if root.name().is(Some(W), "footnotes") || root.name().is(Some(W), "endnotes") {
        return root
            .child_elements()
            .any(|n| matches!(n.attribute(Some(W), "type"), None | Some("normal")));
    }
    let mut stack: Vec<&ooxml_xml::Element> = vec![root];
    while let Some(e) = stack.pop() {
        for c in e.child_elements() {
            let n = c.name();
            if n.is(Some(W), "t") && !crate::docx::text::leading_text(c).trim().is_empty() {
                return true;
            }
            if ["drawing", "pict", "fldChar", "fldSimple", "commentReference"]
                .iter()
                .any(|l| n.is(Some(W), l))
            {
                return true;
            }
            stack.push(c);
        }
    }
    false
}

fn referenced(records: &[Record]) -> Vec<String> {
    let mut out = Vec::new();
    for record in records {
        match record {
            Record::Paragraph(p) => out.extend(p.notes.iter().cloned()),
            Record::Table(t) => {
                for cell in t.rows.iter().flatten() {
                    out.extend(referenced(&cell.records));
                }
            }
            Record::Marker(_) => {}
        }
    }
    out
}

fn note_definitions(reader: &mut Reader<'_>, labels: Vec<String>) -> Vec<Block> {
    let notes = note_elements(reader.docx);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<String> = labels.into();
    while let Some(label) = queue.pop_front() {
        let Some((part, _, element)) = notes.iter().find(|n| n.1 == label) else {
            continue;
        };
        if !seen.insert(label.clone()) {
            continue;
        }
        let mut records = reader.container(part, element);
        for record in &mut records {
            if let Record::Paragraph(p) = record {
                drop_note_mark(p);
            }
        }
        let blocks = build(reader, &records, true);
        out.push(Block::Footnote(label, blocks));
        queue.extend(referenced(&records));
    }
    out
}

fn drop_note_mark(record: &mut ParagraphRecord) {
    if let Some(Inline::Text { text, .. }) = record.inline.first_mut() {
        *text = text.trim_start_matches(' ').to_owned();
        if text.is_empty() {
            record.inline.remove(0);
        }
    }
}

/// The model blocks for one container's records.
pub fn build(reader: &mut Reader<'_>, records: &[Record], trailing: bool) -> Vec<Block> {
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(record) = records.get(k) {
        let run = |kinds: &[Kind]| {
            records
                .get(k..)
                .unwrap_or(&[])
                .iter()
                .take_while(|r| matches!(r, Record::Paragraph(p) if kinds.contains(&p.kind)))
                .filter_map(|r| {
                    if let Record::Paragraph(p) = r {
                        Some(p.as_ref().clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        match record {
            Record::Paragraph(p) if matches!(p.kind, Kind::Bullet | Kind::Ordered) => {
                let group = run(&[Kind::Bullet, Kind::Ordered]);
                k += group.len();
                let lists = lists(reader, &group, &out);
                out.extend(lists);
                continue;
            }
            Record::Paragraph(p) if p.kind == Kind::Quote => {
                let mut group = run(&[Kind::Quote]);
                k += group.len();
                let inner: Vec<Block> = group
                    .iter_mut()
                    .flat_map(|r| paragraph_blocks(reader, r, false))
                    .collect();
                out.push(Block::Quote(inner));
                out.extend(after(&group, reader.ids));
                continue;
            }
            Record::Paragraph(p) if p.kind == Kind::Code => {
                let group = run(&[Kind::Code]);
                k += group.len();
                if reader.ids {
                    out.push(Block::Comment(group.iter().map(label).collect::<Vec<_>>().join(" ")));
                }
                out.push(Block::Code(group.iter().map(|r| code_text(r) + "\n").collect()));
                out.extend(after(&group, reader.ids));
                continue;
            }
            Record::Paragraph(p) => {
                let mut p = p.as_ref().clone();
                out.extend(paragraph_blocks(reader, &mut p, trailing));
                out.extend(after(std::slice::from_ref(&p), reader.ids));
            }
            Record::Table(t) => out.extend(table(reader, t)),
            Record::Marker(m) if !reader.ids => {}
            Record::Marker(m) => {
                if m.begin {
                    let mut words = vec![m.id.clone(), m.kind.to_owned()];
                    words.extend(m.tag.as_deref().map(|t| format!("tag: {}", one_line(t, 120))));
                    words.extend(m.alias.as_deref().map(|a| format!("title: {}", one_line(a, 120))));
                    words.retain(|w| !w.is_empty());
                    out.push(Block::Comment(comment_text(&words.join(" "))));
                } else {
                    out.push(Block::Comment(format!("/{}", m.id)));
                }
            }
        }
        k += 1;
    }
    out
}

/// A heading whose text starts with a typed number.
fn typed_number(text: &str) -> bool {
    let rest = text.trim_start();
    let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let after = if digits > 0 {
        let mut r = rest.get(digits..).unwrap_or("");
        while let Some(more) = r.strip_prefix('.') {
            let n = more.find(|c: char| !c.is_ascii_digit()).unwrap_or(more.len());
            if n == 0 {
                break;
            }
            r = more.get(n..).unwrap_or("");
        }
        Some(r.strip_prefix('.').unwrap_or(r))
    } else {
        let roman = rest.find(|c: char| !"IVXLC".contains(c)).unwrap_or(rest.len());
        let letter = rest.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        if roman > 0 && rest.get(roman..).is_some_and(|r| r.starts_with('.')) {
            rest.get(roman + 1..)
        } else if letter && rest.get(1..).is_some_and(|r| r.starts_with('.')) {
            rest.get(2..)
        } else {
            None
        }
    };
    after.and_then(|r| r.chars().next()).is_some_and(char::is_whitespace)
}

/// What a paragraph's id comment says.
fn label(record: &ParagraphRecord) -> String {
    let mut words = vec![record.id.clone()];
    if !record.joins.is_empty() {
        words.push(format!("joins {}", record.joins.join(" ")));
    }
    if record.unknown_style {
        if let Some(name) = record.style_name.as_deref().filter(|n| !n.is_empty()) {
            words.push(format!("style: {}", comment_text(name)));
        }
    }
    if record.kind == Kind::Heading && record.level > 6 {
        words.push(format!("level {}", record.level));
    }
    if record.kind == Kind::Heading {
        if record.label.as_deref().is_some_and(|l| !l.is_empty()) {
            words.push("numbered: list".to_owned());
        } else if typed_number(&record.text) {
            words.push("numbered: text".to_owned());
        }
    }
    words.extend(record.changes.iter().cloned());
    words.extend(record.hoisted.iter().cloned());
    words.join(" ")
}

/// Comments at the start of the line moved into the paragraph's own comment.
fn hoist(record: &mut ParagraphRecord, inline: Vec<Inline>) -> Vec<Inline> {
    let k = inline
        .iter()
        .take_while(|i| match i {
            Inline::Html(_) => true,
            Inline::Text { text, .. } => text.trim().is_empty(),
            _ => false,
        })
        .count();
    let hoisted: Vec<String> = inline
        .iter()
        .take(k)
        .filter_map(|i| match i {
            Inline::Html(h) => Some(h.get(5..h.len().saturating_sub(4)).unwrap_or("").trim().to_owned()),
            _ => None,
        })
        .collect();
    if hoisted.is_empty() {
        return inline;
    }
    record.hoisted = hoisted;
    inline.into_iter().skip(k).collect()
}

fn paragraph_blocks(reader: &mut Reader<'_>, record: &mut ParagraphRecord, trailing: bool) -> Vec<Block> {
    let mut inline = if reader.ids {
        hoist(record, record.inline.clone())
    } else {
        strip_comments(&record.inline)
    };
    if record.kind == Kind::Heading {
        inline = inline
            .into_iter()
            .map(|i| if i == Inline::Break { Inline::text(" ") } else { i })
            .collect();
        if let Some(l) = record.label.as_deref().filter(|l| !l.is_empty()) {
            inline.insert(0, Inline::text(format!("{l} ")));
        }
    }
    let mut out = Vec::new();
    if record.kind == Kind::Rule {
        if reader.ids {
            out.push(Block::Comment(label(record)));
        }
        out.push(Block::Rule);
        return out;
    }
    let empty = !has_content(&inline);
    if reader.ids {
        if trailing && !(empty && inline.is_empty() && record.kind != Kind::Heading) {
            if !inline.is_empty() {
                inline.push(Inline::text(" "));
            }
            inline.push(Inline::Html(comment(&label(record))));
        } else if empty && record.kind != Kind::Heading {
            out.push(Block::Comment(format!("{} empty", label(record))));
        } else {
            out.push(Block::Comment(label(record)));
        }
    }
    if record.kind == Kind::Heading {
        out.push(Block::Heading(record.level.min(6), inline));
    } else if !(reader.ids && empty && (!trailing || inline.is_empty())) {
        out.push(Block::Paragraph(inline));
    }
    for (identifier, b) in record.text_boxes.clone() {
        let records = reader.container(&record.part, &b);
        let name = identifier.unwrap_or_else(|| "drawing".to_owned());
        if reader.ids {
            out.push(Block::Comment(format!("{name} text-box")));
        }
        out.extend(build(reader, &records, false));
        if reader.ids {
            out.push(Block::Comment(format!("/{name}")));
        }
    }
    out
}

/// A section break a group of paragraphs ends, said where the Markdown carries `ids`.
fn after(records: &[ParagraphRecord], ids: bool) -> Vec<Block> {
    records
        .iter()
        .filter(|_| ids)
        .filter_map(|r| {
            r.section
                .as_ref()
                .map(|(id, kind)| Block::Comment(format!("{id} section break: {kind}")))
        })
        .collect()
}

/// `inline` without its HTML comments (docx-agent's `_strip_comments`).
fn strip_comments(inline: &[Inline]) -> Vec<Inline> {
    inline
        .iter()
        .filter(|i| !matches!(i, Inline::Html(_)))
        .cloned()
        .collect()
}

fn code_text(record: &ParagraphRecord) -> String {
    record
        .inline
        .iter()
        .map(|i| match i {
            Inline::Text { text, .. } | Inline::Critic(text) => text.clone(),
            Inline::Break => "\n".to_owned(),
            _ => String::new(),
        })
        .collect()
}

/// Consecutive list paragraphs as nested Markdown lists.
fn lists(reader: &mut Reader<'_>, records: &[ParagraphRecord], before: &[Block]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    // Stack of (level, path to the list in `out`, num id); a list is addressed by its path.
    let mut stack: Vec<(i64, Vec<usize>, Option<i64>)> = Vec::new();
    let previous = before.iter().rev().find(|b| !matches!(b, Block::Comment(_))).cloned();
    for record in records {
        let ordered = record.kind == Kind::Ordered;
        let num_id = record.membership.as_ref().map(|m| m.num_id);
        let level = record.level;
        while stack.last().is_some_and(|s| s.0 > level) {
            stack.pop();
        }
        if let Some(top) = stack.last_mut() {
            let same = list_at(&mut out, &top.1).is_some_and(|l| l.ordered == ordered);
            if top.0 == level && same && top.2 != num_id && counts_on(reader, top.2, record) {
                top.2 = num_id;
            }
        }
        let top = stack.last().cloned();
        match top {
            Some((l, path, top_num))
                if l == level
                    && (list_at(&mut out, &path).is_some_and(|x| x.ordered != ordered) || top_num != num_id) =>
            {
                stack.pop();
                let ended = list_at(&mut out, &path).map(|x| (x.ordered, x.alternate));
                let alternate = ended.is_some_and(|(o, a)| o == ordered && !a);
                let path = place(
                    &mut out,
                    &stack,
                    ListBlock {
                        ordered,
                        start: start(record),
                        items: Vec::new(),
                        alternate,
                    },
                );
                stack.push((level, path, num_id));
            }
            Some((l, ..)) if l >= level => {}
            _ => {
                let mut alternate = false;
                if stack.is_empty() {
                    let last = out.last().cloned().or_else(|| previous.clone());
                    alternate = matches!(last, Some(Block::List(ref x)) if x.ordered == ordered && !x.alternate);
                }
                let path = place(
                    &mut out,
                    &stack,
                    ListBlock {
                        ordered,
                        start: start(record),
                        items: Vec::new(),
                        alternate,
                    },
                );
                stack.push((level, path, num_id));
            }
        }
        let mut record = record.clone();
        let mut item = paragraph_blocks(reader, &mut record, true);
        item.extend(after(std::slice::from_ref(&record), reader.ids));
        if let Some(list) = stack.last().map(|s| s.1.clone()).and_then(|p| list_at(&mut out, &p)) {
            list.items.push(item);
        }
    }
    out
}

/// The list at `path`: an index into `out`, then per nesting the item index and block index.
fn list_at<'b>(out: &'b mut [Block], path: &[usize]) -> Option<&'b mut ListBlock> {
    let (first, rest) = path.split_first()?;
    let Some(Block::List(list)) = out.get_mut(*first) else {
        return None;
    };
    let mut list = list;
    for pair in rest.chunks(2) {
        let [item, block] = pair else { return None };
        let Some(Block::List(inner)) = list.items.get_mut(*item).and_then(|i| i.get_mut(*block)) else {
            return None;
        };
        list = inner;
    }
    Some(list)
}

fn place(out: &mut Vec<Block>, stack: &[(i64, Vec<usize>, Option<i64>)], new: ListBlock) -> Vec<usize> {
    match stack.last() {
        Some((_, path, _)) => {
            let Some(parent) = list_at(out, path) else {
                return Vec::new();
            };
            if parent.items.is_empty() {
                parent.items.push(Vec::new());
            }
            let item = parent.items.len() - 1;
            let Some(blocks) = parent.items.last_mut() else {
                return Vec::new();
            };
            blocks.push(Block::List(new));
            let mut p = path.clone();
            p.extend([item, blocks.len() - 1]);
            p
        }
        None => {
            out.push(Block::List(new));
            vec![out.len() - 1]
        }
    }
}

fn counts_on(reader: &Reader<'_>, previous: Option<i64>, record: &ParagraphRecord) -> bool {
    let (Some(previous), Some(found)) = (previous, record.membership.as_ref()) else {
        return false;
    };
    if !found.from_style {
        return false;
    }
    let lists = &reader.shared.lists;
    lists.canonical(lists.abstract_of(previous)) == lists.canonical(found.abstract_id)
}

fn start(record: &ParagraphRecord) -> i64 {
    match (record.kind, record.number) {
        (Kind::Ordered, Some(n)) => n,
        _ => 1,
    }
}

fn table(reader: &mut Reader<'_>, t: &TableRecord) -> Vec<Block> {
    let mut words = vec![t.id.clone()];
    if let Some(name) = t.style_name.as_deref().filter(|n| !n.is_empty()) {
        words.push(format!("table style: {}", comment_text(name)));
    }
    for (label, local) in [("alt", "tblCaption"), ("description", "tblDescription")] {
        let value = crate::docx::find_path(&t.element, &["tblPr", local]).and_then(|n| n.attribute(Some(W), "val"));
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            words.push(format!("{label}: {}", comment_text(&one_line(v, 120))));
        }
    }
    let mut out = if reader.ids {
        vec![Block::Comment(words.join(" "))]
    } else {
        Vec::new()
    };
    match gfm(reader, t) {
        Some(table) => out.push(Block::Table(table)),
        None => out.push(Block::Html(crate::markdown::html::table_html(reader, t))),
    }
    out
}

fn gfm(reader: &Reader<'_>, t: &TableRecord) -> Option<Table> {
    let mut rows = Vec::new();
    for line in &t.rows {
        if line.len() != t.columns || line.iter().any(|c| c.row_span != 1 || c.column_span != 1) {
            return None;
        }
        let mut cells = Vec::new();
        for cell in line {
            match cell.records.as_slice() {
                [] => cells.push(Vec::new()),
                [Record::Paragraph(p)] => {
                    if !matches!(p.kind, Kind::Paragraph | Kind::Quote | Kind::Code)
                        || !p.text_boxes.is_empty()
                        || p.section.is_some()
                    {
                        return None;
                    }
                    let inline = if reader.ids {
                        p.inline.clone()
                    } else {
                        strip_comments(&p.inline)
                    };
                    if inline.contains(&Inline::Break) {
                        return None;
                    }
                    cells.push(inline);
                }
                _ => return None,
            }
        }
        rows.push(cells);
    }
    let body = if t.rows.len() > 1 {
        t.rows.get(1..).unwrap_or(&[])
    } else {
        &t.rows[..]
    };
    let aligns = (0..t.columns)
        .map(|column| {
            let mut found: Vec<Option<String>> = Vec::new();
            for line in body {
                for record in line.get(column).map_or(&[][..], |c| &c.records[..]) {
                    if let Record::Paragraph(p) = record {
                        if let Some(last) = p.elements.last() {
                            let cell = reader.cell_of(&p.part, last, &t.table_style);
                            let a = reader.shared.formatting.alignment(&last.element, cell.as_ref());
                            if !found.contains(&a) {
                                found.push(a);
                            }
                        }
                    }
                }
            }
            match found.as_slice() {
                [Some(a)] if a == "center" || a == "right" => Some(a.clone()),
                _ => None,
            }
        })
        .collect();
    let look = crate::docx::find_path(&t.element, &["tblPr", "tblLook"]);
    if t.header_rows == 0 && !first_row_look(look) {
        return Some(Table {
            header: Vec::new(),
            rows,
            aligns,
            headed: false,
        });
    }
    let mut rows = rows.into_iter();
    let header = rows.next().unwrap_or_default();
    Some(Table {
        header,
        rows: rows.collect(),
        aligns,
        headed: true,
    })
}

/// Whether a `w:tblLook` turns the first row's look on (its attribute, else its hex mask;
/// Word's default where there is none).
fn first_row_look(look: Option<&ooxml_xml::Element>) -> bool {
    let Some(look) = look else { return true };
    if let Some(v) = look.attribute(Some(W), "firstRow") {
        return matches!(v, "1" | "true" | "on");
    }
    look.attribute(Some(W), "val")
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|v| u32::from_str_radix(v, 16).ok())
        .is_some_and(|mask| mask & 0x0020 != 0)
}
