//! Owns: one paragraph's content in the view as inline items (docx-agent's `_Inline`):
//! runs with the marks their effective formatting adds over the paragraph's baseline,
//! links, breaks, note references, pictures, other drawings as comments, content controls,
//! bookmarks a reader may target, revisions (CriticMarkup in the markup view) and fields
//! (marked in the markup view).
//!
//! Must not: group paragraphs or render text.

use super::detail::{drawing_record, every_hyperlink};
use super::{child_at, comment, index, ParagraphRecord, Placed, Reader};
use crate::docx::controls::control_kind;
use crate::docx::drawings::drawing_kind;
use crate::docx::formatting::{Cell, EffectiveRun};
use crate::docx::text::{leading_text, View, NON_BREAKING_HYPHEN, OBJECT, SOFT_HYPHEN};
use crate::docx::{child_val, live_paths};
use crate::ids::{MC, W, WP};
use crate::markdown::model::{plain_text, Inline, Mark, Marks};
use crate::markdown::stylemap::Construct;
use ooxml_xml::{Element, Node};
use std::sync::Arc;

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

/// What a run holds in the view.
enum Piece {
    Char(char),
    Break,
    Html(String),
    Note(String),
    Drawing(Placed),
    Comment(String),
}

/// One paragraph's walk.
pub(super) struct Walk<'r, 'a> {
    reader: &'r mut Reader<'a>,
    record: &'r mut ParagraphRecord,
    paragraph: &'r Placed,
    cell: Option<Cell>,
    href: Vec<Option<String>>,
    /// Open complex fields: in their result, marked in the markup view, the instruction.
    fields: Vec<(bool, bool, String)>,
    baseline: Option<EffectiveRun>,
    /// The places of the paragraph's hyperlinks, in document order (their `hl:` numbers).
    links: Vec<Box<[u32]>>,
    /// The revisions the walk is inside.
    revision: Vec<Option<String>>,
}

fn is_w(e: &Element, names: &[&str]) -> bool {
    e.name().namespace() == Some(W) && names.contains(&e.name().local())
}

impl<'r, 'a> Walk<'r, 'a> {
    pub(super) fn new(
        reader: &'r mut Reader<'a>,
        record: &'r mut ParagraphRecord,
        paragraph: &'r Placed,
        cell: Option<Cell>,
    ) -> Walk<'r, 'a> {
        Walk {
            reader,
            record,
            paragraph,
            cell,
            href: Vec::new(),
            fields: Vec::new(),
            baseline: None,
            links: every_hyperlink(&paragraph.element, &paragraph.path),
            revision: Vec::new(),
        }
    }

    pub(super) fn walk(mut self) {
        let paragraph = Placed {
            element: Arc::clone(&self.paragraph.element),
            path: self.paragraph.path.clone(),
        };
        self.container(&paragraph);
    }

    fn visible(&self) -> bool {
        self.fields.iter().all(|f| f.0)
    }

    fn key(&self, path: &[u32]) -> (String, Box<[u32]>) {
        (self.record.part.clone(), Box::from(path))
    }

    fn children(node: &Placed) -> Vec<Placed> {
        node.element
            .children()
            .iter()
            .enumerate()
            .filter_map(|(k, n)| match n {
                Node::Element(e) => Some(Placed {
                    element: Arc::clone(e),
                    path: [&node.path[..], &[index(k)]].concat().into_boxed_slice(),
                }),
                _ => None,
            })
            .collect()
    }

    fn container(&mut self, node: &Placed) {
        for child in Self::children(node) {
            let e = &child.element;
            if is_w(e, &["r"]) {
                self.run(&child);
            } else if is_w(e, &["hyperlink"]) {
                self.hyperlink(&child);
            } else if is_w(e, &["smartTag", "customXml", "dir", "bdo"]) {
                self.container(&child);
            } else if is_w(e, &["fldSimple"]) {
                self.simple_field(&child);
            } else if is_w(e, &["sdt"]) {
                self.inline_control(&child);
            } else if is_w(e, &["ins", "moveTo", "del", "moveFrom"]) {
                self.revision_container(&child);
            } else if is_w(e, &["bookmarkStart"]) {
                self.bookmark(e);
            } else if (e.name().is(Some(M), "oMath") || e.name().is(Some(M), "oMathPara")) && self.visible() {
                let math: String = descendants(e, M, "t").iter().map(|t| leading_text(t)).collect();
                self.record
                    .inline
                    .push(Inline::Html(comment(&format!("equation: {math}"))));
            }
        }
    }

    fn simple_field(&mut self, node: &Placed) {
        let instruction = node.element.attribute(Some(W), "instr").unwrap_or("").trim().to_owned();
        self.record.detail.fields.push((instruction.clone(), String::new()));
        let start = self.record.inline.len();
        let marked = self.reader.view == View::Markup && self.visible();
        if marked {
            self.record
                .inline
                .push(Inline::Html(comment(&format!("field: {instruction}"))));
        }
        self.container(node);
        let result = plain_text(self.record.inline.get(start..).unwrap_or(&[]));
        if let Some(last) = self.record.detail.fields.last_mut() {
            last.1 = result;
        }
        if marked {
            self.record.inline.push(Inline::Html(comment("/field")));
        }
    }

    fn hyperlink(&mut self, node: &Placed) {
        let mut target = None;
        if let Some(rid) = node.element.attribute(Some(R), "id").filter(|r| !r.is_empty()) {
            let rels = crate::parts::relationships(&self.reader.docx.package, &self.record.part).unwrap_or_default();
            target = rels.into_iter().find(|r| r.id == rid).map(|r| r.target);
        }
        if let Some(anchor) = node.element.attribute(Some(W), "anchor").filter(|a| !a.is_empty()) {
            target = Some(format!("{}#{anchor}", target.unwrap_or_default()));
        }
        let number = self.links.iter().position(|p| *p == node.path);
        let start = self.record.inline.len();
        self.href.push(target.clone());
        self.container(node);
        self.href.pop();
        if let Some(number) = number {
            let text = plain_text(self.record.inline.get(start..).unwrap_or(&[]));
            let id = format!("hl:{}/{number}", self.record.id);
            self.record.detail.links.push((id, target, text));
        }
    }

    fn inline_control(&mut self, node: &Placed) {
        let found = self.reader.shared.controls.get(&self.key(&node.path)).cloned();
        let kind = control_kind(&node.element);
        self.record.detail.controls.push((found.clone(), kind));
        let id = found.unwrap_or_else(|| "None".to_owned());
        let mut words = vec![format!("{id} {kind}")];
        words.extend(control_words(&node.element));
        self.record.inline.push(Inline::Html(comment(&words.join(" "))));
        if let Some((k, content)) = child_at(&node.element, "sdtContent") {
            let placed = Placed {
                element: content,
                path: [&node.path[..], &[k]].concat().into_boxed_slice(),
            };
            self.container(&placed);
        }
        self.record.inline.push(Inline::Html(comment(&format!("/{id}"))));
    }

    fn revision_container(&mut self, node: &Placed) {
        let inserted = is_w(&node.element, &["ins", "moveTo"]);
        let shown = if inserted {
            self.reader.view != View::Original
        } else {
            self.reader.view != View::Current
        };
        let found = self.reader.shared.revisions.get(&self.key(&node.path)).cloned();
        if !self.record.detail.revisions.contains(&found) {
            self.record.detail.revisions.push(found.clone());
        }
        if !shown {
            return;
        }
        self.revision.push(found.clone());
        if self.reader.view != View::Markup {
            self.container(node);
            self.revision.pop();
            return;
        }
        let id = found.unwrap_or_else(|| "None".to_owned());
        let sign = if inserted { "++" } else { "--" };
        self.record.inline.push(Inline::Critic(format!("{{{sign}")));
        self.container(node);
        self.revision.pop();
        self.record.inline.push(Inline::Critic(format!("{sign}}}")));
        let author = node.element.attribute(Some(W), "author").unwrap_or("");
        let moved = if is_w(&node.element, &["moveTo", "moveFrom"]) {
            " moved"
        } else {
            ""
        };
        self.record
            .inline
            .push(Inline::Html(comment(format!("{id}{moved} {author}").trim())));
    }

    fn bookmark(&mut self, node: &Element) {
        let name = node.attribute(Some(W), "name").unwrap_or("");
        if name.is_empty() || (name.starts_with('_') && !self.reader.shared.anchors.contains(name)) {
            return;
        }
        self.record.detail.bookmarks.push(format!("bm:{name}"));
        self.record.inline.push(Inline::Html(comment(&format!("bm:{name}"))));
    }

    fn marks(&mut self, run: &Element, first: char) -> (Marks, EffectiveRun) {
        let paragraph = Arc::clone(&self.paragraph.element);
        let base = match &self.baseline {
            Some(b) => b.clone(),
            None => {
                let b = self
                    .reader
                    .shared
                    .formatting
                    .run(&paragraph, None, 'a', self.cell.as_ref());
                self.baseline = Some(b.clone());
                b
            }
        };
        let effective = self
            .reader
            .shared
            .formatting
            .run(&paragraph, Some(run), first, self.cell.as_ref());
        let mut marks = Marks::new();
        if effective.bold && !base.bold {
            marks.insert(Mark::Strong);
        }
        if effective.italic && !base.italic {
            marks.insert(Mark::Em);
        }
        if (effective.strike || effective.double_strike) && !(base.strike || base.double_strike) {
            marks.insert(Mark::Strike);
        }
        if let Some(style) = &effective.style {
            let found = self.reader.docx.style(style);
            let name = found.and_then(|s| crate::docx::style_name(s)).unwrap_or(style);
            if self.reader.map.character_rule(Some(name)) == Some(Construct::Code) {
                marks.insert(Mark::Code);
            }
        }
        (marks, effective)
    }

    fn run(&mut self, run: &Placed) {
        let pieces = self.pieces(run);
        if pieces.is_empty() {
            return;
        }
        let first = pieces
            .iter()
            .find_map(|p| if let Piece::Char(c) = p { Some(*c) } else { None })
            .unwrap_or('a');
        let (marks, effective) = self.marks(&run.element, first);
        if effective.hidden {
            return;
        }
        self.run_record(run, &pieces, effective.clone());
        let href = self.href.last().cloned().flatten();
        let mut buffer = String::new();
        for piece in pieces {
            if let Piece::Char(c) = piece {
                buffer.push(c);
                continue;
            }
            if !buffer.is_empty() {
                let text = std::mem::take(&mut buffer);
                self.record.inline.push(Inline::Text {
                    text,
                    marks: marks.clone(),
                    href: href.clone(),
                });
            }
            match piece {
                Piece::Break => self.record.inline.push(Inline::Break),
                Piece::Html(h) => self.record.inline.push(Inline::Html(h)),
                Piece::Note(n) => {
                    self.record.notes.push(n.clone());
                    self.record.inline.push(Inline::NoteRef(n));
                }
                Piece::Drawing(node) => self.drawing(&node, href.clone()),
                Piece::Comment(w_id) => self.comment_reference(&w_id),
                Piece::Char(_) => {}
            }
        }
        if !buffer.is_empty() {
            self.record.inline.push(Inline::Text {
                text: buffer,
                marks,
                href,
            });
        }
        if let Some((k, _)) =
            child_at(&run.element, "rPr").and_then(|(k, p)| child_at(&p, "rPrChange").map(|(j, c)| ([k, j], c)))
        {
            let found = self
                .reader
                .shared
                .revisions
                .get(&self.key(&[&run.path[..], &k].concat()))
                .cloned();
            if !self.record.detail.revisions.contains(&found) {
                self.record.detail.revisions.push(found);
            }
        }
        if self.reader.view == View::Markup {
            if let Some((k, change)) =
                child_at(&run.element, "rPr").and_then(|(k, p)| child_at(&p, "rPrChange").map(|(j, c)| ([k, j], c)))
            {
                let id = self
                    .reader
                    .shared
                    .revisions
                    .get(&self.key(&[&run.path[..], &k].concat()))
                    .cloned()
                    .unwrap_or_default();
                let author = change.attribute(Some(W), "author").unwrap_or("");
                self.record
                    .inline
                    .push(Inline::Html(comment(format!("{id} formatting {author}").trim())));
            }
        }
    }

    /// The run's record for `word_inspect`, where it has text.
    fn run_record(&mut self, run: &Placed, pieces: &[Piece], effective: EffectiveRun) {
        let text: String = pieces
            .iter()
            .filter_map(|p| if let Piece::Char(c) = p { Some(*c) } else { None })
            .collect();
        if text.is_empty() {
            return;
        }
        let style = effective
            .style
            .as_ref()
            .and_then(|id| match self.reader.docx.style(id) {
                Some(found) => crate::docx::style_name(found).map(str::to_owned),
                None => Some(id.clone()),
            });
        let direct = run
            .element
            .find(Some(W), "rPr")
            .map(|p| {
                p.child_elements()
                    .filter(|c| !c.name().is(Some(W), "rStyle"))
                    .map(|c| c.name().local().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let revision = self.revision.last().cloned().flatten();
        self.record.detail.runs.push(super::RunRecord {
            text,
            style,
            direct,
            effective,
            revision,
        });
    }

    /// The run's content in the view.
    fn pieces(&mut self, run: &Placed) -> Vec<Piece> {
        let mut out = Vec::new();
        for child in Self::children(run) {
            let e = &child.element;
            if is_w(e, &["fldChar"]) {
                self.field_char(e, &mut out);
                continue;
            }
            if is_w(e, &["instrText", "delInstrText"]) {
                let take = is_w(e, &["instrText"]) || self.reader.view != View::Current;
                if let Some(top) = self.fields.last_mut().filter(|f| !f.0 && take) {
                    top.2 = format!("{}{}", top.2, leading_text(e)).trim().to_owned();
                    if let Some(last) = self.record.detail.fields.last_mut() {
                        last.0 = format!("{}{}", last.0, leading_text(e)).trim().to_owned();
                    }
                }
                continue;
            }
            if is_w(e, &["commentReference"]) {
                if self.visible() {
                    out.push(Piece::Comment(e.attribute(Some(W), "id").unwrap_or("").to_owned()));
                }
                continue;
            }
            const SKIPPED: [&str; 10] = [
                "rPr",
                "footnoteRef",
                "endnoteRef",
                "annotationRef",
                "separator",
                "continuationSeparator",
                "lastRenderedPageBreak",
                "commentReference",
                "instrText",
                "delInstrText",
            ];
            if is_w(e, &SKIPPED) || !self.visible() {
                continue;
            }
            self.piece(child, &mut out);
        }
        out
    }

    fn field_char(&mut self, e: &Element, out: &mut Vec<Piece>) {
        match e.attribute(Some(W), "fldCharType") {
            Some("begin") => {
                self.fields.push((false, false, String::new()));
                self.record.detail.fields.push((String::new(), String::new()));
            }
            Some("separate") => {
                let markup = self.reader.view == View::Markup;
                if let Some(top) = self.fields.last_mut() {
                    top.0 = true;
                }
                if markup && self.visible() {
                    if let Some(top) = self.fields.last_mut() {
                        top.1 = true;
                        out.push(Piece::Html(comment(&format!("field: {}", top.2))));
                    }
                }
            }
            Some("end") => {
                if let Some((_, marked, _)) = self.fields.pop() {
                    if marked && self.visible() {
                        out.push(Piece::Html(comment("/field")));
                    }
                }
            }
            _ => {}
        }
    }

    fn piece(&mut self, child: Placed, out: &mut Vec<Piece>) {
        let e = &child.element;
        let local = e.name().local();
        if e.name().is(Some(MC), "AlternateContent") {
            out.push(Piece::Drawing(child));
            return;
        }
        if e.name().namespace() != Some(W) {
            return;
        }
        match local {
            "t" => {
                let text = leading_text(e);
                if !self.fields.is_empty() {
                    if let Some(last) = self.record.detail.fields.last_mut() {
                        last.1.push_str(&text);
                    }
                }
                out.extend(text.chars().map(Piece::Char));
            }
            "delText" if self.reader.view != View::Current => out.extend(leading_text(e).chars().map(Piece::Char)),
            "br" | "cr" => out.push(match e.attribute(Some(W), "type") {
                Some("page") => Piece::Html(comment("page break")),
                Some("column") => Piece::Html(comment("column break")),
                _ => Piece::Break,
            }),
            "tab" | "ptab" => out.push(Piece::Char('\t')),
            "noBreakHyphen" => out.push(Piece::Char(NON_BREAKING_HYPHEN)),
            "softHyphen" => out.push(Piece::Char(SOFT_HYPHEN)),
            "sym" => out.push(Piece::Char(
                e.attribute(Some(W), "char")
                    .and_then(|c| u32::from_str_radix(c.trim(), 16).ok())
                    .and_then(char::from_u32)
                    .unwrap_or(OBJECT),
            )),
            "footnoteReference" | "endnoteReference" => {
                let prefix = if local == "footnoteReference" { "fn" } else { "en" };
                let id = e.attribute(Some(W), "id").unwrap_or("None").to_owned();
                if let Some(found) = self.reader.shared.notes.get(&(prefix.to_owned(), id)) {
                    out.push(Piece::Note(found.clone()));
                }
            }
            "drawing" | "pict" | "object" => out.push(Piece::Drawing(child)),
            _ => {}
        }
    }

    fn comment_reference(&mut self, w_id: &str) {
        let Some(record) = self.reader.shared.comments.get(w_id).cloned() else {
            return;
        };
        self.record.detail.comments.push(record.id.clone());
        if self.reader.view != View::Markup {
            return;
        }
        let mut who: Vec<&str> = Vec::new();
        who.extend(record.author.as_deref().filter(|a| !a.is_empty()));
        if record.parent.is_some() {
            who.push("in reply");
        }
        if record.done {
            who.push("resolved");
        }
        let who = who.join(", ");
        let text = if who.is_empty() {
            record.text.clone()
        } else {
            format!("{who}: {}", record.text)
        };
        self.record.inline.push(Inline::Critic("{>>".to_owned()));
        self.record.inline.push(Inline::text(text));
        self.record.inline.push(Inline::Critic("<<}".to_owned()));
        self.record.inline.push(Inline::Html(comment(&record.id)));
    }

    fn drawing(&mut self, node: &Placed, href: Option<String>) {
        let frames = live_paths(&node.element, &node.path, &|e| {
            e.name().is(Some(WP), "inline") || e.name().is(Some(WP), "anchor")
        });
        if frames.is_empty() {
            for b in live_or_not(&node.element, &node.path, "txbxContent") {
                self.record.text_boxes.push((None, b));
            }
            self.record.inline.push(Inline::Html(comment("drawing")));
            return;
        }
        for frame in frames {
            let key = self.key(&frame.path);
            let doc_pr = frame.element.find(Some(WP), "docPr");
            let name = doc_pr.and_then(|d| d.attribute(None, "name")).map(str::to_owned);
            let alt = doc_pr.and_then(|d| d.attribute(None, "descr")).unwrap_or("").to_owned();
            self.record.detail.drawings.push(drawing_record(
                &frame.element,
                self.reader.shared.pictures.get(&key).cloned(),
                self.reader.shared.drawings.get(&key).cloned(),
                name.clone(),
                alt.clone(),
            ));
            if let Some(id) = self.reader.shared.pictures.get(&key).cloned() {
                self.record.inline.push(Inline::Image {
                    alt,
                    src: id,
                    title: name,
                    href: href.clone(),
                });
                continue;
            }
            let id = self
                .reader
                .shared
                .drawings
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "None".to_owned());
            let kind = drawing_kind(&frame.element);
            let mut label = format!("{id} {kind}");
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                label.push_str(&format!(" \"{name}\""));
            }
            self.record.inline.push(Inline::Html(comment(&label)));
            let placed = Placed {
                element: Arc::clone(&frame.element),
                path: frame.path.clone(),
            };
            if let Some(b) = live_paths(&placed.element, &placed.path, &|e| e.name().is(Some(W), "txbxContent"))
                .into_iter()
                .next()
            {
                self.record.text_boxes.push((
                    Some(id),
                    Placed {
                        element: b.element,
                        path: b.path,
                    },
                ));
            }
        }
    }
}

/// Every `w:txbxContent` below `node` (fallbacks included, as lxml's `iter` finds them).
fn live_or_not(node: &Element, base: &[u32], local: &str) -> Vec<Placed> {
    let mut out = Vec::new();
    let mut path = base.to_vec();
    collect(node, &mut path, local, &mut out);
    out
}

fn collect(node: &Element, path: &mut Vec<u32>, local: &str, out: &mut Vec<Placed>) {
    for (k, n) in node.children().iter().enumerate() {
        let Node::Element(e) = n else { continue };
        path.push(index(k));
        if e.name().is(Some(W), local) {
            out.push(Placed {
                element: Arc::clone(e),
                path: path.clone().into_boxed_slice(),
            });
        }
        collect(e, path, local, out);
        path.pop();
    }
}

fn descendants(node: &Element, namespace: &str, local: &str) -> Vec<Arc<Element>> {
    let mut out = Vec::new();
    for e in node.child_elements() {
        if e.name().is(Some(namespace), local) {
            out.push(Arc::clone(e));
        }
        out.extend(descendants(e, namespace, local));
    }
    out
}

/// A content control's `tag: ...` and `title: ...` words, where set.
pub(crate) fn control_words(node: &Element) -> Vec<String> {
    let properties = node.find(Some(W), "sdtPr");
    let mut out = Vec::new();
    for (label, local) in [("tag", "tag"), ("title", "alias")] {
        if let Some(value) = properties.and_then(|p| child_val(p, local)).filter(|v| !v.is_empty()) {
            out.push(format!("{label}: {}", one_line(value, 120)));
        }
    }
    out
}

/// `text` on one line, at most `limit` characters (an ellipsis where cut).
pub(crate) fn one_line(text: &str, limit: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let cut: String = joined.chars().take(limit - 1).collect();
    format!("{}\u{2026}", cut.trim_end())
}
