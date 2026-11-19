//! Owns: the [`Writer`], which makes model blocks into elements of one part, as docx-agent's
//! `markdown/write.py` `Writer` makes them: each construct in the style the style map
//! names for it (found by name, alias or id, a heading also by outline level, the table
//! style the document's default or the one its tables use most, a built-in style the
//! document lacks added as Word writes it, the default paragraph style written as none),
//! paragraphs, captions, code, quotes, thematic breaks, raw HTML as text, and the runs and
//! hyperlinks of inline content.
//!
//! Must not: put blocks in place, stamp or track them (the command does), or write but
//! through the sink.

use super::Options;
use crate::names::{R, W};
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::text::write::make_run;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::{style_id, style_kind, style_name};
use ooxml_word_model::markdown::model::{has_content, Block, Inline, Mark, Marks};
use ooxml_word_model::markdown::stylemap::{construct_kind, WriteMap};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The relationship type of a hyperlink.
const HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
/// Word's thematic break: a bottom border, single, 3/4 pt, one point from the text.
const RULE_BORDER: [(&str, &str); 4] = [("val", "single"), ("sz", "6"), ("space", "1"), ("color", "auto")];

/// Model blocks as elements of one part.
pub(crate) struct Writer<'m> {
    pub(crate) part: String,
    pub(crate) map: &'m WriteMap,
    pub(crate) options: Options,
    pub(crate) warnings: Vec<String>,
    styles: HashMap<(String, i64), Option<String>>,
    pub(crate) abstracts: HashMap<bool, i64>,
    pub(crate) numbered: bool,
    /// Per definition a style numbers with: the levels this insertion's lists count at.
    pub(crate) counting: HashMap<Option<i64>, HashSet<i64>>,
    pub(crate) text_width: Option<i64>,
    quote: usize,
    pub(crate) in_list: usize,
    pub(crate) tables: Vec<(Id, Vec<i64>)>,
    /// Relationships parts changed (hyperlinks), to put once the edit's nodes are built.
    pub(crate) linked: bool,
}

impl<'m> Writer<'m> {
    pub(crate) fn new(part: &str, map: &'m WriteMap, options: Options) -> Writer<'m> {
        Writer {
            part: part.to_owned(),
            map,
            options,
            warnings: Vec::new(),
            styles: HashMap::new(),
            abstracts: HashMap::new(),
            numbered: false,
            counting: HashMap::new(),
            text_width: None,
            quote: 0,
            in_list: 0,
            tables: Vec::new(),
            linked: false,
        }
    }

    /// The style id `construct` is written with (none: none, or the default paragraph
    /// style, which Word does not name).
    pub(crate) fn style(&mut self, sink: &mut EditSink<'_, '_>, construct: &str, level: i64) -> Result<Option<String>> {
        let key = (construct.to_owned(), level);
        if let Some(found) = self.styles.get(&key) {
            return Ok(found.clone());
        }
        let found = self.resolve_style(sink, construct, level)?;
        self.styles.insert(key, found.clone());
        Ok(found)
    }

    fn resolve_style(&mut self, sink: &mut EditSink<'_, '_>, construct: &str, level: i64) -> Result<Option<String>> {
        let Some(name) = self.map.style_for(construct, level).and_then(|r| r.style.clone()) else {
            return Ok(None);
        };
        let kind = construct_kind(construct);
        let docx = sink.docx()?;
        let mut found: Option<String> = None;
        if construct == "table" && self.map.is_default("table", level) {
            found = super::table::table_default(sink, &docx)?.or_else(|| super::table::table_style_in_use(&docx));
        }
        if found.is_none() {
            found = docx.find_style(&name, kind).map(|s| style_id(s).to_owned());
        }
        if found.is_none() && construct == "heading" {
            found = docx
                .styles()
                .iter()
                .filter(|s| style_kind(s) == "paragraph")
                .find(|s| {
                    let outline = stated_outline_level(&docx, style_id(s));
                    ooxml_word_model::markdown::stylemap::StyleMap::default().heading_level(style_name(s), outline)
                        == Some(level)
                })
                .map(|s| style_id(s).to_owned());
        }
        if found.is_none()
            && matches!(construct, "paragraph" | "table_cell")
            && ooxml_word_model::docx::casefold(&name) == "normal"
        {
            return Ok(None);
        }
        let id = match found {
            Some(id) => id,
            None => {
                let id = crate::builtin::ensure(sink, &name, kind)
                    .map_err(|e| EditError::Refused(format!("no {kind} style for {construct}: {e}")))?;
                self.warnings
                    .push(format!("style added: {name} ({id}), as Word writes it"));
                id
            }
        };
        if kind == "paragraph" {
            let docx = sink.docx()?;
            if docx.default_style("paragraph").is_some_and(|d| style_id(d) == id) {
                return Ok(None);
            }
        }
        Ok(Some(id))
    }

    /// A plain paragraph's style.
    pub(crate) fn body_style(&mut self, sink: &mut EditSink<'_, '_>) -> Result<Option<String>> {
        self.style(sink, "paragraph", 0)
    }

    pub(crate) fn warn_once(&mut self, text: String) {
        if !self.warnings.contains(&text) {
            self.warnings.push(text);
        }
    }

    /// `blocks` as elements, a separator paragraph between two tables.
    pub(crate) fn blocks(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work, blocks: &[Block]) -> Result<Vec<Id>> {
        let mut out: Vec<Id> = Vec::new();
        for k in 0..blocks.len() {
            let made = match self.caption(blocks, k) {
                Some(inline) => {
                    let style = self.style(sink, "caption", 0)?;
                    vec![self.paragraph(sink, work, &inline, style.as_deref(), None)?]
                }
                None => match blocks.get(k) {
                    Some(block) => self.block(sink, work, block)?,
                    None => Vec::new(),
                },
            };
            let table = |id: Option<&Id>| id.is_some_and(|i| work.is(*i, W, "tbl"));
            if table(made.first()) && table(out.last()) {
                let separator = self.separator(sink, work)?;
                out.push(separator);
            }
            out.extend(made);
        }
        Ok(out)
    }

    /// The inline of `blocks[k]` without its caption marker when it is a caption paragraph:
    /// one of the body (not in a quote, a list or a note) whose text starts `: `, or
    /// `Table:` right before or after a table.
    fn caption(&self, blocks: &[Block], k: usize) -> Option<Vec<Inline>> {
        let Some(Block::Paragraph(inline)) = blocks.get(k) else {
            return None;
        };
        if self.quote > 0 {
            return None;
        }
        let Some(Inline::Text { text, marks, href }) = inline.first() else {
            return None;
        };
        if !marks.is_empty() || href.is_some() {
            return None;
        }
        let beside = [k.checked_sub(1), Some(k + 1)]
            .into_iter()
            .flatten()
            .any(|j| matches!(blocks.get(j), Some(Block::Table(_))));
        let rest = caption_rest(text).or_else(|| beside.then(|| table_caption_rest(text)).flatten())?;
        let mut out = Vec::new();
        if !rest.is_empty() {
            out.push(Inline::text(rest));
        }
        out.extend(inline.iter().skip(1).cloned());
        has_content(&out).then_some(out)
    }

    /// An empty paragraph in the body's style, between two tables.
    pub(crate) fn separator(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work) -> Result<Id> {
        let style = self.body_style(sink)?;
        self.paragraph(sink, work, &[], style.as_deref(), None)
    }

    fn block(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work, block: &Block) -> Result<Vec<Id>> {
        match block {
            Block::Paragraph(inline) => {
                if inline.is_empty() {
                    return Ok(Vec::new());
                }
                let style = if self.quote > 0 {
                    self.style(sink, "blockquote", 0)?
                } else {
                    self.body_style(sink)?
                };
                let made = self.paragraph(sink, work, inline, style.as_deref(), None)?;
                Ok(vec![made])
            }
            Block::Heading(level, inline) => {
                let style = self.style(sink, "heading", *level)?;
                let made = self.paragraph(sink, work, inline, style.as_deref(), None)?;
                Ok(vec![made])
            }
            Block::Code(text) => self.code(sink, work, text),
            Block::Quote(inner) => {
                self.quote += 1;
                let made = self.blocks(sink, work, inner);
                self.quote -= 1;
                made
            }
            Block::List(list) => self.list(sink, work, list, 0),
            Block::Table(table) => Ok(vec![self.table(sink, work, table)?]),
            Block::Rule => {
                let style = self.body_style(sink)?;
                let paragraph = self.paragraph(sink, work, &[], style.as_deref(), None)?;
                let properties = properties(work, paragraph);
                let borders = make(work, "pBdr", &[]);
                work.append(properties, borders);
                let bottom = make(work, "bottom", &RULE_BORDER);
                work.append(borders, bottom);
                Ok(vec![paragraph])
            }
            Block::Html(text) => {
                let style = self.body_style(sink)?;
                let trimmed = text.trim_end_matches('\n');
                trimmed
                    .split('\n')
                    .map(|line| self.paragraph(sink, work, &[Inline::text(line)], style.as_deref(), None))
                    .collect()
            }
            Block::Comment(_) | Block::Footnote(..) => Ok(Vec::new()),
        }
    }

    fn code(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work, text: &str) -> Result<Vec<Id>> {
        let style = self.style(sink, "code_block", 0)?;
        let body = text.strip_suffix('\n').unwrap_or(text);
        body.split('\n')
            .map(|line| {
                let inline = if line.is_empty() {
                    Vec::new()
                } else {
                    vec![Inline::text(line)]
                };
                self.paragraph(sink, work, &inline, style.as_deref(), None)
            })
            .collect()
    }

    /// A paragraph of `inline` in `style`, aligned `align`.
    pub(crate) fn paragraph(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        inline: &[Inline],
        style: Option<&str>,
        align: Option<&str>,
    ) -> Result<Id> {
        let paragraph = make(work, "p", &[]);
        if let Some(style) = style {
            let properties = properties(work, paragraph);
            let node = make(work, "pStyle", &[("val", style)]);
            work.append(properties, node);
        }
        if let Some(align) = align {
            let properties = properties(work, paragraph);
            let node = make(work, "jc", &[("val", align)]);
            work.append(properties, node);
        }
        for node in self.inline(sink, work, inline)? {
            work.append(paragraph, node);
        }
        Ok(paragraph)
    }

    /// Runs, and hyperlinks holding runs, for `items`.
    fn inline(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work, items: &[Inline]) -> Result<Vec<Id>> {
        let mut out = Vec::new();
        let mut link: Option<(Id, String)> = None;
        for item in items {
            if matches!(item, Inline::Html(t) if t.starts_with("<!--")) {
                continue;
            }
            let href = match item {
                Inline::Text { href, .. } | Inline::Image { href, .. } => href.clone().filter(|h| !h.is_empty()),
                _ => None,
            };
            let nodes = self.inline_item(sink, work, item, href.is_some())?;
            if nodes.is_empty() {
                continue;
            }
            match href {
                Some(href) => {
                    let holder = match &link {
                        Some((node, current)) if *current == href => *node,
                        _ => {
                            let node = self.link(sink, work, &href)?;
                            out.push(node);
                            link = Some((node, href));
                            node
                        }
                    };
                    for node in nodes {
                        work.append(holder, node);
                    }
                }
                None => {
                    link = None;
                    out.extend(nodes);
                }
            }
        }
        Ok(out)
    }

    fn link(&mut self, sink: &mut EditSink<'_, '_>, work: &mut Work, href: &str) -> Result<Id> {
        let link = make(work, "hyperlink", &[]);
        if let Some(anchor) = href.strip_prefix('#') {
            let name = work.qname(W, "anchor", "w");
            work.set(link, name, &super::percent_decode(anchor));
        } else {
            let (package, rid) = crate::new_part::add_external(sink.doc().package(), &self.part, HYPERLINK, href)?;
            if let Some(package) = package {
                let rels = ooxml_opc::name::rels_for(&self.part);
                if let Some(tree) = package.tree(&rels)? {
                    sink.put(&rels, tree)?;
                }
                self.linked = true;
            }
            let root = work.root();
            let name = if work.has_prefix_for(root, R) {
                work.qname(R, "id", "r")
            } else {
                work.declare(link, "ns1", R);
                Arc::new(ooxml_xml::QName::new(Some("ns1"), "id", Some(R)))
            };
            work.set(link, name, &rid);
        }
        let history = work.qname(W, "history", "w");
        work.set(link, history, "1");
        Ok(link)
    }

    fn inline_item(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        item: &Inline,
        link: bool,
    ) -> Result<Vec<Id>> {
        Ok(match item {
            Inline::Text { text, marks, .. } => {
                let text = valid_text(&text.replace('\n', " "));
                if text.is_empty() {
                    return Ok(Vec::new());
                }
                let properties = self.run_properties(sink, work, marks, link)?;
                vec![make_run(work, &text, properties)]
            }
            Inline::Html(text) => vec![make_run(work, &valid_text(&text.replace('\n', " ")), None)],
            Inline::Break => {
                let run = make(work, "r", &[]);
                let br = make(work, "br", &[]);
                work.append(run, br);
                vec![run]
            }
            Inline::Image { .. } => return Err(EditError::Unsupported("a picture in Markdown".to_owned())),
            Inline::NoteRef(_) => return Err(EditError::Unsupported("a footnote in Markdown".to_owned())),
            Inline::Critic(_) => Vec::new(),
        })
    }

    fn run_properties(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        marks: &Marks,
        link: bool,
    ) -> Result<Option<Id>> {
        let mut marks = marks.clone();
        let direct = self.options.emphasis_direct;
        let style = if marks.remove(&Mark::Code) {
            self.style(sink, "code", 0)?
        } else if link {
            self.style(sink, "link", 0)?
        } else if !direct && marks.remove(&Mark::Strong) {
            self.style(sink, "strong", 0)?
        } else if !direct && marks.remove(&Mark::Em) {
            self.style(sink, "emphasis", 0)?
        } else {
            None
        };
        let properties = make(work, "rPr", &[]);
        let mut any = false;
        let mut add = |work: &mut Work, local: &str, attributes: &[(&str, &str)]| {
            let node = make(work, local, attributes);
            work.append(properties, node);
            any = true;
        };
        if let Some(style) = &style {
            add(work, "rStyle", &[("val", style)]);
        }
        if marks.contains(&Mark::Strong) {
            add(work, "b", &[]);
            add(work, "bCs", &[]);
        }
        if marks.contains(&Mark::Em) {
            add(work, "i", &[]);
            add(work, "iCs", &[]);
        }
        if marks.contains(&Mark::Strike) {
            add(work, "strike", &[]);
        }
        Ok(any.then_some(properties))
    }
}

/// A paragraph's `w:pPr`, made as its first child when missing.
pub(crate) fn properties(work: &mut Work, paragraph: Id) -> Id {
    if let Some(found) = work.find(paragraph, W, "pPr") {
        return found;
    }
    let node = make(work, "pPr", &[]);
    work.insert(paragraph, 0, node);
    node
}

/// Characters XML 1.0 cannot hold written as U+FFFD.
fn valid_text(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => '\u{fffd}',
            c => c,
        })
        .collect()
}

/// `:[ \t]+` at the start: the rest.
fn caption_rest(text: &str) -> Option<&str> {
    let rest = text.strip_prefix(':')?;
    let trimmed = rest.trim_start_matches([' ', '\t']);
    (trimmed.len() < rest.len()).then_some(trimmed)
}

/// `[Tt]able:[ \t]*` at the start: the rest.
fn table_caption_rest(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("Table:").or_else(|| text.strip_prefix("table:"))?;
    Some(rest.trim_start_matches([' ', '\t']))
}

/// Every block of `blocks` and below, in order.
pub(crate) fn walk_blocks<'b>(blocks: &'b [Block], out: &mut Vec<&'b Block>) {
    for block in blocks {
        out.push(block);
        match block {
            Block::Quote(inner) | Block::Footnote(_, inner) => walk_blocks(inner, out),
            Block::List(list) => list.items.iter().for_each(|item| walk_blocks(item, out)),
            _ => {}
        }
    }
}

/// Every inline item of `blocks` and below.
pub(crate) fn walk_inline<'b>(blocks: &'b [Block], out: &mut Vec<&'b Inline>) {
    for block in blocks {
        match block {
            Block::Paragraph(inline) | Block::Heading(_, inline) => out.extend(inline),
            Block::Quote(inner) | Block::Footnote(_, inner) => walk_inline(inner, out),
            Block::List(list) => list.items.iter().for_each(|item| walk_inline(item, out)),
            Block::Table(table) => {
                for row in std::iter::once(&table.header).chain(&table.rows) {
                    row.iter().for_each(|cell| out.extend(cell));
                }
            }
            _ => {}
        }
    }
}

/// The `w:outlineLvl` a paragraph style declares through its `basedOn` chain, as written
/// (the reader's `outline_level`).
fn stated_outline_level(docx: &ooxml_word_model::docx::Docx, style: &str) -> Option<i64> {
    let mut current = Some(style.to_owned());
    let mut seen = HashSet::new();
    while let Some(id) = current.take().filter(|i| !i.is_empty()) {
        if !seen.insert(id.clone()) {
            break;
        }
        let node = docx.style(&id)?;
        if let Some(stated) = ooxml_word_model::docx::find_path(node, &["pPr", "outlineLvl"]) {
            return stated
                .attribute(Some(W), "val")
                .and_then(ooxml_word_model::xmlread::py_int);
        }
        current = ooxml_word_model::docx::child_val(node, "basedOn").map(str::to_owned);
    }
    None
}
