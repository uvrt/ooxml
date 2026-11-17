//! Owns: a document read into records in one view of its revisions (docx-agent's
//! `markdown/read.py`, `Reader`): every story's blocks -- paragraphs joined where the view
//! removes their marks (named by the last, as Word keeps it), tables, content controls'
//! starts and ends -- each paragraph with its id, style, Markdown construct, list label and
//! number, and its inline items in the view.
//!
//! Elements are found by their place in their part (paths), as [`crate::ids::PartIds`]
//! keys them; nothing is found by pointer.
//!
//! Must not: write the document or render text ([`super::render`] does).

mod build;
mod inline;
mod lookups;
mod table;

pub use build::{build, project};
pub use lookups::note_elements;
use lookups::{anchors, notes_of};

use crate::docx::controls::{control_kind, controls};
use crate::docx::drawings::{all_drawings, pictures};
use crate::docx::formatting::Formatting;
use crate::docx::numbering::{membership, Lists, Membership};
use crate::docx::revisions::records as revision_records;
use crate::docx::sections::section_id;
use crate::docx::text::{paragraph_text, View};
use crate::docx::{child_val, find_path, style_name, Docx, StoryPart};
use crate::ids::W;
use crate::labels::Counters;
use crate::markdown::model::Inline;
use crate::markdown::stylemap::{Construct, StyleMap};
use crate::markdown::ViewName;
use ooxml_xml::{Element, Node};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// An element of a part with its place.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The element.
    pub element: Arc<Element>,
    /// Its place in its part.
    pub path: Box<[u32]>,
}

/// What a paragraph is in Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A paragraph.
    Paragraph,
    /// A heading.
    Heading,
    /// A bullet list item.
    Bullet,
    /// A numbered list item.
    Ordered,
    /// A quote.
    Quote,
    /// A line of code.
    Code,
    /// A thematic break.
    Rule,
}

/// A paragraph as read.
#[derive(Debug, Clone)]
pub struct ParagraphRecord {
    /// Its id (the last of those joined).
    pub id: String,
    /// Its part.
    pub part: String,
    /// Its story.
    pub story: String,
    /// The paragraphs joined into it, in order.
    pub elements: Vec<Placed>,
    /// Whether its id is volatile.
    pub volatile: bool,
    /// Its paragraph style's id.
    pub style_id: Option<String>,
    /// Its style's name.
    pub style_name: Option<String>,
    /// Its construct.
    pub kind: Kind,
    /// Its heading or list level.
    pub level: i64,
    /// Whether no rule of the style map names its style.
    pub unknown_style: bool,
    /// Its list.
    pub membership: Option<Membership>,
    /// Its list label.
    pub label: Option<String>,
    /// Its running number.
    pub number: Option<i64>,
    /// Its inline items.
    pub inline: Vec<Inline>,
    /// Its text in the view.
    pub text: String,
    /// The ids of the paragraphs joined before it.
    pub joins: Vec<String>,
    /// The notes it references.
    pub notes: Vec<String>,
    /// Its text boxes: the drawing's id, the box's content.
    pub text_boxes: Vec<(Option<String>, Placed)>,
    /// The section it ends: its id and type.
    pub section: Option<(String, String)>,
    /// Comments hoisted from the start of its line.
    pub hoisted: Vec<String>,
    /// What its own comment adds in the markup view.
    pub changes: Vec<String>,
}

/// A cell as read.
#[derive(Debug, Clone)]
pub struct CellRecord {
    /// Its id (`t:X/c<row>,<column>`).
    pub id: String,
    /// Its grid row and column.
    pub row: usize,
    /// Its grid column.
    pub column: usize,
    /// Rows it spans.
    pub row_span: usize,
    /// Columns it spans.
    pub column_span: usize,
    /// Its blocks.
    pub records: Vec<Record>,
}

/// A table as read.
#[derive(Debug, Clone)]
pub struct TableRecord {
    /// Its id.
    pub id: String,
    /// The `w:tbl`.
    pub element: Arc<Element>,
    /// Its style's name.
    pub style_name: Option<String>,
    /// Its columns in the view.
    pub columns: usize,
    /// Its visible rows.
    pub rows: Vec<Vec<CellRecord>>,
    /// Header rows.
    pub header_rows: usize,
    /// Whether its id is volatile.
    pub volatile: bool,
    /// Its innermost table style (`""`: none named), for its paragraphs' formatting.
    pub table_style: String,
}

/// A content control's start or end.
#[derive(Debug, Clone)]
pub struct Marker {
    /// Its id.
    pub id: String,
    /// Its kind.
    pub kind: &'static str,
    /// Whether it starts.
    pub begin: bool,
    /// Its tag.
    pub tag: Option<String>,
    /// Its title.
    pub alias: Option<String>,
}

/// A record.
#[derive(Debug, Clone)]
pub enum Record {
    /// A paragraph.
    Paragraph(Box<ParagraphRecord>),
    /// A table.
    Table(Box<TableRecord>),
    /// A content control's start or end.
    Marker(Marker),
}

/// One item of a container.
enum Item {
    P(Placed),
    Tbl(Placed),
    Begin(Marker),
    End(Marker),
}

/// One reading of a document in one view.
pub struct Reader<'a> {
    /// The document.
    pub docx: &'a Docx,
    /// The view.
    pub view: View,
    /// The view's name.
    pub view_name: ViewName,
    map: StyleMap,
    formatting: Formatting,
    lists: Lists,
    counters: HashMap<(String, bool), Counters>,
    controls: HashMap<(String, Box<[u32]>), String>,
    revisions: HashMap<(String, Box<[u32]>), String>,
    pictures: HashMap<(String, Box<[u32]>), String>,
    drawings: HashMap<(String, Box<[u32]>), String>,
    notes: HashMap<(String, String), String>,
    anchors: HashSet<String>,
    comments: HashMap<String, crate::docx::comments::Comment>,
    outline: HashMap<Option<String>, Option<i64>>,
    /// Paragraphs whose marks are deleted just before a table, joined into its first cell.
    into: HashMap<(String, Box<[u32]>), Vec<Placed>>,
}

impl<'a> Reader<'a> {
    /// A reading of `docx` in `view`.
    #[must_use]
    pub fn new(docx: &'a Docx, view_name: ViewName) -> Reader<'a> {
        let view = match view_name {
            ViewName::Final => View::Current,
            ViewName::Original => View::Original,
            ViewName::Markup => View::Markup,
        };
        let key = |part: &str, path: &[u32]| (part.to_owned(), Box::<[u32]>::from(path));
        let controls = controls(docx)
            .into_iter()
            .map(|c| (key(&c.part, &c.path), c.id))
            .collect();
        let revisions = revision_records(docx)
            .into_iter()
            .map(|r| (key(&r.part, &r.path), r.id))
            .collect();
        let pictures = pictures(docx)
            .into_iter()
            .map(|d| (key(&d.part, &d.path), d.id))
            .collect();
        let drawings = all_drawings(docx)
            .into_iter()
            .map(|d| (key(&d.part, &d.path), d.id))
            .collect();
        Reader {
            docx,
            view,
            view_name,
            map: StyleMap::default(),
            formatting: Formatting::read(docx),
            lists: Lists::read(docx),
            counters: HashMap::new(),
            controls,
            revisions,
            pictures,
            drawings,
            notes: notes_of(docx),
            anchors: anchors(docx),
            comments: crate::docx::comments::comments(docx)
                .into_iter()
                .filter_map(|c| c.w_id.clone().map(|w| (w, c)))
                .collect(),
            outline: HashMap::new(),
            into: HashMap::new(),
        }
    }

    /// The name of the paragraph style `style_id`, or of the default where it is none or
    /// unknown.
    fn style_name(&self, style_id: Option<&str>) -> Option<String> {
        let found = style_id.filter(|s| !s.is_empty()).and_then(|s| self.docx.style(s));
        let found = found.or_else(|| self.docx.default_style("paragraph"));
        found.and_then(|s| style_name(s)).map(str::to_owned)
    }

    /// The `w:outlineLvl` a paragraph style declares through its chain (0-8 as written).
    fn outline_level(&mut self, style_id: Option<&str>) -> Option<i64> {
        let key = style_id.map(str::to_owned);
        if let Some(found) = self.outline.get(&key) {
            return *found;
        }
        let default = self
            .docx
            .default_style("paragraph")
            .map(|s| crate::docx::style_id(s).to_owned());
        let mut current = style_id.filter(|s| !s.is_empty()).map(str::to_owned).or(default);
        let mut seen = HashSet::new();
        let mut level = None;
        while let Some(id) = current.take() {
            if !seen.insert(id.clone()) {
                break;
            }
            let Some(node) = self.docx.style(&id) else { break };
            if let Some(stated) = find_path(node, &["pPr", "outlineLvl"]) {
                level = stated.attribute(Some(W), "val").and_then(crate::xmlread::py_int);
                break;
            }
            current = child_val(node, "basedOn").map(str::to_owned);
        }
        self.outline.insert(key, level);
        level
    }

    /// The label and running number of the next item of `listed` in `part` (a text box's
    /// paragraphs count apart).
    fn number(&mut self, part: &str, in_text_box: bool, listed: &Membership) -> (Option<String>, Option<i64>) {
        let numbering = self.formatting.numbering();
        let counters = self.counters.entry((part.to_owned(), in_text_box)).or_default();
        counters.item_number(numbering, listed.num_id, listed.level)
    }

    /// A story part's records (a notes or comments part: every note's).
    pub fn story(&mut self, part: &StoryPart) -> Vec<Record> {
        let root = part.root();
        let notes = ["footnotes", "endnotes", "comments"]
            .iter()
            .any(|n| root.name().is(Some(W), n));
        if notes {
            let mut out = Vec::new();
            for (k, child) in root.children().iter().enumerate() {
                if let Node::Element(child) = child {
                    let placed = Placed {
                        element: Arc::clone(child),
                        path: vec![index(k)].into_boxed_slice(),
                    };
                    out.extend(self.container(&part.part, &placed));
                }
            }
            return out;
        }
        let container = if root.name().is(Some(W), "document") {
            root.children().iter().enumerate().find_map(|(k, n)| match n {
                Node::Element(e) if e.name().is(Some(W), "body") => Some(Placed {
                    element: Arc::clone(e),
                    path: vec![index(k)].into_boxed_slice(),
                }),
                _ => None,
            })
        } else {
            Some(Placed {
                element: Arc::clone(root),
                path: Box::default(),
            })
        };
        container.map(|c| self.container(&part.part, &c)).unwrap_or_default()
    }

    /// A container's blocks in the view.
    pub fn container(&mut self, part: &str, container: &Placed) -> Vec<Record> {
        let items = self.items(part, container);
        let groups = self.groups(part, items);
        self.records(part, groups)
    }

    fn items(&self, part: &str, container: &Placed) -> Vec<Item> {
        let mut out = Vec::new();
        for (k, node) in container.element.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            let mut path = container.path.to_vec();
            path.push(index(k));
            let placed = Placed {
                element: Arc::clone(child),
                path: path.into_boxed_slice(),
            };
            let name = child.name();
            if name.is(Some(W), "p") {
                out.push(Item::P(placed));
            } else if name.is(Some(W), "tbl") {
                out.push(Item::Tbl(placed));
            } else if name.is(Some(W), "sdt") {
                let marker = Marker {
                    id: self
                        .controls
                        .get(&(part.to_owned(), placed.path.clone()))
                        .cloned()
                        .unwrap_or_default(),
                    kind: control_kind(child),
                    begin: true,
                    tag: child
                        .find(Some(W), "sdtPr")
                        .and_then(|p| child_val(p, "tag"))
                        .map(str::to_owned),
                    alias: child
                        .find(Some(W), "sdtPr")
                        .and_then(|p| child_val(p, "alias"))
                        .map(str::to_owned),
                };
                out.push(Item::Begin(marker.clone()));
                if let Some((j, content)) = child_at(child, "sdtContent") {
                    let mut inner = placed.path.to_vec();
                    inner.push(j);
                    out.extend(self.items(
                        part,
                        &Placed {
                            element: content,
                            path: inner.into_boxed_slice(),
                        },
                    ));
                }
                out.push(Item::End(Marker { begin: false, ..marker }));
            } else if name.is(Some(W), "customXml") {
                out.extend(self.items(part, &placed));
            }
        }
        out
    }

    /// The first child of the mark (`w:pPr/w:rPr`) the view removes, where it removes it.
    fn mark_removed(&self, paragraph: &Element) -> bool {
        let gone: &[&str] = match self.view {
            View::Current => &["del", "moveFrom"],
            View::Original => &["ins", "moveTo"],
            View::Markup => &[],
        };
        find_path(paragraph, &["pPr", "rPr"]).is_some_and(|mark| {
            mark.child_elements()
                .any(|c| gone.iter().any(|g| c.name().is(Some(W), g)))
        })
    }

    fn shows_text(&self, p: &Placed) -> bool {
        !paragraph_text(&p.element, self.view).is_empty() || find_path(&p.element, &["pPr", "sectPr"]).is_some()
    }

    /// Paragraphs that are one in the view grouped (docx2svg's join, both ways).
    fn groups(&mut self, part: &str, items: Vec<Item>) -> Vec<Item2> {
        let mut groups = Vec::new();
        let mut pending: Vec<Placed> = Vec::new();
        for item in items {
            match item {
                Item::P(p) => {
                    let removed = self.mark_removed(&p.element);
                    pending.push(p);
                    if !removed {
                        groups.push(Item2::P(std::mem::take(&mut pending)));
                    }
                }
                Item::Tbl(t) => {
                    if !pending.is_empty() && self.view == View::Current {
                        let cut = pending
                            .iter()
                            .rposition(|p| find_path(&p.element, &["pPr", "sectPr"]).is_some());
                        let rest = match cut {
                            Some(cut) => {
                                let rest = pending.split_off(cut + 1);
                                groups.push(Item2::P(std::mem::take(&mut pending)));
                                rest
                            }
                            None => std::mem::take(&mut pending),
                        };
                        if !rest.is_empty() {
                            self.into.insert((part.to_owned(), t.path.clone()), rest);
                        }
                    } else if !pending.is_empty() {
                        if pending.iter().any(|p| self.shows_text(p)) {
                            groups.push(Item2::P(std::mem::take(&mut pending)));
                        }
                        pending.clear();
                    }
                    groups.push(Item2::Tbl(t));
                }
                Item::Begin(m) | Item::End(m) => groups.push(Item2::Marker(m)),
            }
        }
        if !pending.is_empty() && (self.view != View::Original || pending.iter().any(|p| self.shows_text(p))) {
            groups.push(Item2::P(pending));
        }
        groups
    }

    fn records(&mut self, part: &str, groups: Vec<Item2>) -> Vec<Record> {
        let mut out = Vec::new();
        for group in groups {
            match group {
                Item2::P(elements) => out.push(Record::Paragraph(Box::new(self.paragraph(part, elements, None)))),
                Item2::Tbl(t) => out.extend(self.table_with(part, &t, None)),
                Item2::Marker(m) => out.push(Record::Marker(m)),
            }
        }
        out
    }

    /// A paragraph's record: `elements` are one in the view, named by the last.
    fn paragraph(&mut self, part: &str, elements: Vec<Placed>, table_style: Option<&str>) -> ParagraphRecord {
        let story_part = self.docx.part(part);
        let story = story_part.map(|p| p.story.clone()).unwrap_or_default();
        let entry = |p: &Placed| story_part.and_then(|s| s.ids.paragraph_at(&p.path));
        let last = elements.last().map(|p| self.as_viewed(&p.element));
        let named = elements.last().and_then(entry);
        let style_id = last
            .as_ref()
            .and_then(|l| find_path(l, &["pPr", "pStyle"]))
            .and_then(|s| s.attribute(Some(W), "val"))
            .map(str::to_owned);
        let mut record = ParagraphRecord {
            id: named.map(|e| e.id.clone()).unwrap_or_default(),
            part: part.to_owned(),
            story,
            elements: Vec::new(),
            volatile: named.is_some_and(|e| e.state.volatile()),
            style_name: self.style_name(style_id.as_deref()),
            style_id,
            kind: Kind::Paragraph,
            level: 0,
            unknown_style: false,
            membership: None,
            label: None,
            number: None,
            inline: Vec::new(),
            text: String::new(),
            joins: elements
                .iter()
                .take(elements.len().saturating_sub(1))
                .filter_map(|p| entry(p).map(|e| e.id.clone()))
                .collect(),
            notes: Vec::new(),
            text_boxes: Vec::new(),
            section: None,
            hoisted: Vec::new(),
            changes: Vec::new(),
        };
        for element in &elements {
            inline::Walk::new(self, &mut record, element, table_style).walk();
            self.mark_revision(&mut record, element);
        }
        record.inline = tidy(std::mem::take(&mut record.inline));
        record.text = view_text(&record.inline);
        if let (Some(last), Some(viewed)) = (elements.last(), last.as_ref()) {
            let shown = Placed {
                element: Arc::clone(viewed),
                path: last.path.clone(),
            };
            self.classify(&mut record, &shown);
            if let Some(section) = find_path(viewed, &["pPr", "sectPr"]) {
                if record.story == "body" {
                    let kind = section
                        .find(Some(W), "type")
                        .map_or(Some("nextPage"), |t| t.attribute(Some(W), "val"))
                        .unwrap_or("None");
                    record.section = Some((section_id(&record.id), kind.to_owned()));
                }
            }
        }
        record.elements = elements;
        record
    }

    /// The paragraph whose properties the view shows: in the original view, one with the
    /// properties a `w:pPrChange` recorded (its mark and section break kept); otherwise the
    /// paragraph itself.
    fn as_viewed(&self, paragraph: &Arc<Element>) -> Arc<Element> {
        if self.view != View::Original {
            return Arc::clone(paragraph);
        }
        let Some(current) = paragraph.find(Some(W), "pPr") else {
            return Arc::clone(paragraph);
        };
        let Some(old) = current.find(Some(W), "pPrChange").and_then(|c| c.find(Some(W), "pPr")) else {
            return Arc::clone(paragraph);
        };
        let mut children = old.children().to_vec();
        for keep in ["rPr", "sectPr"] {
            if let Some(node) = current.find(Some(W), keep) {
                children.push(Node::Element(Arc::clone(node)));
            }
        }
        let properties = old.with_children(ooxml_xml::Children::from_vec(children));
        let shadow = Element::new(
            paragraph.name().clone(),
            paragraph.namespaces().to_vec(),
            paragraph.attributes().to_vec(),
            vec![Node::Element(Arc::new(properties))],
        );
        Arc::new(shadow)
    }

    /// A paragraph mark's own insertion or deletion; a paragraph-properties change.
    fn mark_revision(&self, record: &mut ParagraphRecord, element: &Placed) {
        let Some((k, ppr)) = child_at(&element.element, "pPr") else {
            return;
        };
        let key = |path: Vec<u32>| (record.part.clone(), path.into_boxed_slice());
        for (j, node) in ppr.children().iter().enumerate() {
            let Node::Element(child) = node else { continue };
            let path = [&element.path[..], &[k, index(j)]].concat();
            if child.name().is(Some(W), "pPrChange") && self.view == View::Markup {
                let id = self.revisions.get(&key(path.clone())).cloned().unwrap_or_default();
                let author = child.attribute(Some(W), "author").unwrap_or("");
                record
                    .changes
                    .push(format!("{id} paragraph-properties {author}").trim().to_owned());
            }
            if child.name().is(Some(W), "rPr") && self.view == View::Markup {
                for (m, mark) in child.children().iter().enumerate() {
                    let Node::Element(mark) = mark else { continue };
                    let inserted = mark.name().is(Some(W), "ins") || mark.name().is(Some(W), "moveTo");
                    let deleted = mark.name().is(Some(W), "del") || mark.name().is(Some(W), "moveFrom");
                    if !(inserted || deleted) {
                        continue;
                    }
                    let id = self
                        .revisions
                        .get(&key([&path[..], &[index(m)]].concat()))
                        .cloned()
                        .unwrap_or_default();
                    let sign = if inserted { "++" } else { "--" };
                    record.inline.push(Inline::Critic(format!("{{{sign}\u{b6}{sign}}}")));
                    let author = mark.attribute(Some(W), "author").unwrap_or("");
                    record
                        .inline
                        .push(Inline::Html(comment(format!("{id} {author}").trim())));
                }
            }
        }
    }

    fn classify(&mut self, record: &mut ParagraphRecord, element: &Placed) {
        let name = record.style_name.clone();
        let outline = self.outline_level(record.style_id.as_deref());
        let heading = self.map.heading_level(name.as_deref(), outline);
        let listed = membership(self.docx, &self.lists, &element.element)
            .filter(|l| l.format.as_deref().is_some_and(|f| f != "none"));
        let rule = self.map.paragraph_rule(name.as_deref());
        let in_box = in_text_box(
            &self.docx.part(&record.part).map(|p| Arc::clone(p.root())),
            &element.path,
        );
        if let Some(level) = heading {
            record.kind = Kind::Heading;
            record.level = level;
            record.unknown_style = !matches!(rule, Some(Construct::Heading(_)) if level <= 6);
            if let Some(listed) = listed {
                (record.label, record.number) = self.number(&record.part, in_box, &listed);
                record.membership = Some(listed);
            }
            return;
        }
        if let Some(listed) = listed {
            record.kind = if listed.format.as_deref() == Some("bullet") {
                Kind::Bullet
            } else {
                Kind::Ordered
            };
            record.level = listed.level;
            (record.label, record.number) = self.number(&record.part, in_box, &listed);
            record.membership = Some(listed);
            record.unknown_style = rule.is_none();
            return;
        }
        match rule {
            Some(Construct::Bullet(level) | Construct::Ordered(level)) => {
                record.kind = if matches!(rule, Some(Construct::Bullet(_))) {
                    Kind::Bullet
                } else {
                    Kind::Ordered
                };
                record.level = level - 1;
                return;
            }
            Some(Construct::Quote) => {
                record.kind = Kind::Quote;
                return;
            }
            Some(Construct::CodeBlock) => {
                record.kind = Kind::Code;
                return;
            }
            _ => {}
        }
        record.unknown_style = matches!(rule, None | Some(Construct::Caption));
        let border = find_path(&element.element, &["pPr", "pBdr", "bottom"]);
        let drawn = border.is_some_and(|b| !matches!(b.attribute(Some(W), "val"), None | Some("nil" | "none")));
        if record.text.trim().is_empty() && drawn && record.inline.is_empty() {
            record.kind = Kind::Rule;
        }
    }
}

/// A container's groups: paragraphs one in the view, tables, markers.
enum Item2 {
    P(Vec<Placed>),
    Tbl(Placed),
    Marker(Marker),
}

/// A child index as a path step.
pub(crate) fn index(k: usize) -> u32 {
    u32::try_from(k).unwrap_or(u32::MAX)
}

/// The first child `w:{local}` of `element`, with its index.
pub(crate) fn child_at(element: &Element, local: &str) -> Option<(u32, Arc<Element>)> {
    element.children().iter().enumerate().find_map(|(k, n)| match n {
        Node::Element(e) if e.name().is(Some(W), local) => Some((index(k), Arc::clone(e))),
        _ => None,
    })
}

/// Whether the element at `path` of `root` is inside a `w:txbxContent`.
fn in_text_box(root: &Option<Arc<Element>>, path: &[u32]) -> bool {
    let Some(mut node) = root.clone() else { return false };
    for k in path.iter().take(path.len().saturating_sub(1)) {
        let Some(next) = node
            .children()
            .iter()
            .nth(*k as usize)
            .and_then(|n| n.as_element().cloned())
        else {
            return false;
        };
        if next.name().is(Some(W), "txbxContent") {
            return true;
        }
        node = next;
    }
    false
}

/// An HTML comment holding `text`, which cannot end it early.
#[must_use]
pub fn comment(text: &str) -> String {
    format!("<!-- {} -->", comment_text(text))
}

/// `text` as it may stand in an HTML comment.
#[must_use]
pub fn comment_text(text: &str) -> String {
    text.replace("--", "- -").replace('>', "&gt;")
}

/// The view's text: no comments, no CriticMarkup delimiters, no comment's words.
fn view_text(inline: &[Inline]) -> String {
    let mut out = String::new();
    let mut commenting = false;
    for item in inline {
        match item {
            Inline::Critic(t) => commenting = t == "{>>" || (commenting && t != "<<}"),
            Inline::Text { text, .. } if !commenting => out.push_str(text),
            Inline::Break => out.push('\u{b}'),
            _ => {}
        }
    }
    out
}

/// Adjacent alike texts merged; a comment after a hard break moved before it.
fn tidy(inline: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for item in inline {
        if let Inline::Text { text, marks, href } = &item {
            if text.is_empty() {
                continue;
            }
            if let Some(Inline::Text {
                text: last,
                marks: m,
                href: h,
            }) = out.last_mut()
            {
                if m == marks && h == href {
                    last.push_str(text);
                    continue;
                }
            }
        }
        out.push(item);
    }
    for k in 1..out.len() {
        if matches!(out.get(k), Some(Inline::Html(_))) && matches!(out.get(k - 1), Some(Inline::Break)) {
            let mut j = k;
            while j > 0 && matches!(out.get(j - 1), Some(Inline::Break)) {
                out.swap(j - 1, j);
                j -= 1;
            }
        }
    }
    out
}
