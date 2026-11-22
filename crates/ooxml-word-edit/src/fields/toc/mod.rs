//! Owns: a table of contents rebuilt as docx-agent's `_rebuild_toc` rebuilds it: the
//! headings it lists ([`select`]), each given a hidden `_Toc` bookmark round its text (one
//! there kept), their pages from the layout, the `toc N` and Hyperlink styles added as Word
//! writes them, and the entries ([`build`]) put in place of the old ones -- everything from
//! the field's begin to the paragraph holding its end, that paragraph's content after the
//! end kept -- each new entry keeping the ids of the old one in its place.
//!
//! A numbered heading's entry shows its list number, with a left stop after it measured
//! through the layout port's fonts ([`number`]).
//!
//! Must not: lay out (the layout port says the pages and measures), or write but through
//! the sink.

pub(crate) mod build;
pub(crate) mod number;
pub(crate) mod select;

use super::pages::{FieldPlace, LaidFields};
use super::scan::{paragraph_of, Located};
use super::switches::{has, switch};
use super::write::{locate, run_of as work_run_of};
use crate::doc::{Draw, DrawFrom, Key};
use crate::links::{marker_draws, new_w};
use crate::names::{W, W14};
use crate::sink::EditSink;
use crate::stamping::w14_prefix;
use crate::work::{scope_above, Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::{at_path, find_path, Docx};
use ooxml_word_model::ids::valid_long_hex;
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// Where a TOC entry's right tab stop is: the text width less this (measured).
const TAB_INSET: i64 = 10;
/// The text width taken where the section says none (A4 with Word's margins).
const DEFAULT_WIDTH: i64 = 9026;
/// The first number of a `_Toc` bookmark made here (Word's are 9 digits).
const BOOKMARK_BASE: i64 = 100_000_000;

/// The table of contents `toc` (in the main part) rebuilt: its entries' pages from `laid`,
/// its style names read under `locale` (the call's; `None`: the document's own language);
/// each heading whose page the layout cannot say listed in `unknown`.
///
/// # Errors
/// [`EditError::Unsupported`] for a field whose end is not beside its begin; a part that
/// is not XML.
pub(crate) fn rebuild(
    sink: &mut EditSink<'_, '_>,
    toc: &Located,
    laid: &dyn LaidFields,
    locale: Option<&ooxml_numfmt::Locale>,
    unknown: &mut Vec<String>,
) -> Result<()> {
    let docx = sink.docx()?;
    let part = toc.part.clone();
    let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
    let root = Arc::clone(story.root());
    let region = Region::of(&root, toc).ok_or_else(|| {
        EditError::Unsupported("a table of contents whose end is not in a paragraph beside its begin".to_owned())
    })?;
    let inside: Vec<Vec<u32>> = region.places.iter().take(region.places.len() - 1).cloned().collect();
    let headings: Vec<select::Heading> = select::entries(&docx, &toc.found.instruction, locale)
        .into_iter()
        .filter(|h| !inside.iter().any(|p| **p == *h.paragraph.path))
        .collect();
    let instruction = toc.found.instruction.clone();
    let hyperlinks = has(&instruction, 'h');
    let mut styles: Vec<(i64, String)> = Vec::new();
    for heading in &headings {
        if !styles.iter().any(|(l, _)| *l == heading.level) {
            let id = crate::builtin::ensure(&mut *sink, &format!("toc {}", heading.level), "paragraph")?;
            styles.push((heading.level, id));
        }
    }
    let link_style = if hyperlinks && !headings.is_empty() {
        Some(crate::builtin::ensure(&mut *sink, "Hyperlink", "character")?)
    } else {
        None
    };
    let docx = sink.docx()?;
    let tab = text_width(&docx, region.places.last().map_or(&[][..], |p| p)) - TAB_INSET;
    let bookmarks = bookmarks(sink, &part, &headings)?;
    let pages = pages(&toc.id(), &headings, laid, unknown);
    let omit = omitted_levels(&instruction);
    let docx = sink.docx()?;
    let labels = number::labels(&docx, &headings);
    let mut stops = number::NumberStops::new(&docx, link_style.as_deref(), laid);
    let numbers: Vec<Option<build::Number<'_>>> = labels
        .iter()
        .zip(&headings)
        .map(|(label, heading)| {
            label.as_ref().map(|l| build::Number {
                label: &l.text,
                stop: l.tab.then(|| stops.stop(heading.level, &l.text)),
            })
        })
        .collect();
    let texts: Vec<String> = headings
        .iter()
        .map(|h| select::heading_text(&h.paragraph.element))
        .collect();
    let entries: Vec<build::Entry<'_>> = headings
        .iter()
        .enumerate()
        .map(|(k, heading)| {
            let style = styles
                .iter()
                .find(|(l, _)| *l == heading.level)
                .map_or("", |(_, s)| s.as_str());
            build::Entry {
                style,
                right_stop: (!style_has_tabs(&docx, style)).then_some(tab),
                text: texts.get(k).map_or("", String::as_str),
                bookmark: bookmarks.get(k).map_or("", String::as_str),
                page: omit
                    .is_none_or(|(a, b)| !(a <= heading.level && heading.level <= b))
                    .then(|| pages.get(k).map_or("", String::as_str)),
                number: numbers.get(k).copied().flatten(),
            }
        })
        .collect();
    let looks = build::Looks {
        instruction: &instruction,
        hyperlinks,
        hidden: has(&instruction, 'z'),
        link_style: link_style.as_deref(),
    };
    let story = docx.part(&part).ok_or_else(|| EditError::NoPart(part.clone()))?;
    let root = Arc::clone(story.root());
    let region = Region::of(&root, toc).ok_or_else(|| EditError::Path(part.clone()))?;
    region.replace(sink, (&part, &root), &looks, &entries)?;
    sink.ensure_w14(&part, false, &[])
}

/// Each heading's page as its entry shows it (empty, and listed in `unknown` with the
/// field's id `field`, where the layout cannot say).
fn pages(field: &str, headings: &[select::Heading], laid: &dyn LaidFields, unknown: &mut Vec<String>) -> Vec<String> {
    headings
        .iter()
        .map(|heading| {
            laid.page_reference(" PAGEREF x \\h ", FieldPlace::Body(&heading.paragraph.id))
                .unwrap_or_else(|_| {
                    unknown.push(format!("{field} ({})", heading.paragraph.id));
                    String::new()
                })
        })
        .collect()
}

/// The levels `\n` leaves the page numbers off (`\n` alone: every level).
fn omitted_levels(instruction: &str) -> Option<(i64, i64)> {
    if !has(instruction, 'n') {
        return None;
    }
    let Some(span) = switch(instruction, 'n') else {
        return Some((1, 9));
    };
    let bytes = span.as_bytes();
    match bytes {
        [a @ b'0'..=b'9', b'-', b @ b'0'..=b'9'] => Some((i64::from(a - b'0'), i64::from(b - b'0'))),
        _ => None,
    }
}

/// Whether the paragraph style `id` has tab stops of its own or through its `basedOn`
/// chain (Word's entries then rely on them).
fn style_has_tabs(docx: &Docx, id: &str) -> bool {
    let mut current = Some(id.to_owned());
    let mut seen = Vec::new();
    while let Some(style_id) = current.take() {
        if seen.contains(&style_id) {
            break;
        }
        let Some(style) = docx.style(&style_id) else { break };
        if let Some(tabs) = find_path(style, &["pPr", "tabs"]) {
            let set = tabs
                .child_elements()
                .any(|t| t.name().is(Some(W), "tab") && t.attribute(Some(W), "val") != Some("clear"));
            if set {
                return true;
            }
        }
        current = find_path(style, &["basedOn"])
            .and_then(|b| b.attribute(Some(W), "val"))
            .map(str::to_owned);
        seen.push(style_id);
    }
    false
}

/// The width between the margins (and gutter) of the section the body block at `path` is
/// in, in twips (`Section.text_width`); [`DEFAULT_WIDTH`] where it says none.
fn text_width(docx: &Docx, path: &[u32]) -> i64 {
    let sections = docx.sections();
    let section = sections.iter().find(|s| path <= &*s.path);
    let int = |node: Option<&Element>, local: &str| {
        node.and_then(|n| n.attribute(Some(W), local))
            .and_then(ooxml_word_model::xmlread::py_int)
    };
    let Some(properties) = section.map(|s| &s.properties) else {
        return DEFAULT_WIDTH;
    };
    let size = find_path(properties, &["pgSz"]);
    let margins = find_path(properties, &["pgMar"]);
    let Some(width) = int(size, "w") else {
        return DEFAULT_WIDTH;
    };
    let side = |local| int(margins, local).unwrap_or(0);
    width - side("left") - side("right") - side("gutter")
}

/// Each heading's `_Toc` bookmark ([`bookmark`]), in order: the annotation ids of the new
/// ones taken together for the headings that have none.
fn bookmarks(sink: &mut EditSink<'_, '_>, part: &str, headings: &[select::Heading]) -> Result<Vec<String>> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    let unmarked = headings
        .iter()
        .filter(|h| at_path(tree.root(), &h.paragraph.path).is_some_and(|e| toc_names(e).is_empty()))
        .count();
    drop(tree);
    let mut ids = sink.marker_ids(unmarked)?.into_iter();
    let mut numbers = TocNumbers::default();
    headings
        .iter()
        .map(|heading| bookmark(sink, part, &heading.paragraph.path, (&mut numbers, &mut ids)))
        .collect()
}

/// The numbers of the `_Toc` bookmarks one rebuild makes: the document's largest is read
/// once, at the first new one; each new one is then the largest, so the next is one above
/// it (as docx-agent's `_toc_bookmark`, which reads the document's largest each time).
#[derive(Default)]
struct TocNumbers {
    next: Option<i64>,
}

impl TocNumbers {
    /// The next new bookmark's number, the document read for the largest only the first time.
    ///
    /// # Errors
    /// When a story part is not XML.
    fn take(&mut self, sink: &EditSink<'_, '_>) -> Result<i64> {
        let number = match self.next {
            Some(number) => number,
            None => {
                let docx = sink.docx()?;
                docx.parts
                    .iter()
                    .flat_map(|p| toc_names(p.root()))
                    .filter_map(|n| n.get(4..).and_then(|d| d.parse::<i64>().ok()))
                    .fold(BOOKMARK_BASE, i64::max)
                    + 1
            }
        };
        self.next = Some(number + 1);
        Ok(number)
    }
}

/// The heading's `_Toc` bookmark at `path` of `part` (`_toc_bookmark`): one starting in it
/// kept, else a new one round its text, numbered one above the document's largest
/// ([`TocNumbers`]), its annotation id the next of `ids` (taken for the unmarked headings
/// together, as one write after another would number them).
fn bookmark(
    sink: &mut EditSink<'_, '_>,
    part: &str,
    path: &[u32],
    (numbers, ids): (&mut TocNumbers, &mut impl Iterator<Item = (i64, u32)>),
) -> Result<String> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::NoPart(part.to_owned()))?;
    let element = at_path(tree.root(), path).ok_or_else(|| EditError::Path(part.to_owned()))?;
    if let Some(name) = toc_names(element).into_iter().next() {
        return Ok(name);
    }
    let name = format!("_Toc{}", numbers.take(sink)?);
    let (value, offset) = ids
        .next()
        .ok_or_else(|| EditError::Path(format!("{part}: a heading unmarked when the ids were taken")))?;
    let mut work = Work::of(element, scope_above(tree.root(), path));
    let paragraph = work.root();
    let id = value.to_string();
    let start = new_w(&mut work, paragraph, "bookmarkStart", &[("id", &id), ("name", &name)]);
    let end = new_w(&mut work, paragraph, "bookmarkEnd", &[("id", &id)]);
    let children: Vec<Id> = work
        .children(paragraph)
        .iter()
        .copied()
        .filter(|c| work.is_element(*c))
        .collect();
    let mut at = 0;
    for (k, child) in children.iter().enumerate() {
        let field_only =
            !work.iter(*child, W, &["fldChar", "instrText"]).is_empty() && work.iter(*child, W, &["t"]).is_empty();
        if work.is(*child, W, "pPr") || field_only {
            at = k + 1;
        } else if !work.is(*child, W, "proofErr") {
            break;
        }
    }
    match children.get(at) {
        Some(next) => work.add_previous(*next, start),
        None => work.append(paragraph, start),
    }
    work.append(paragraph, end);
    let new = work.build().ok_or_else(|| EditError::Path(part.to_owned()))?;
    let draws = marker_draws(
        &[Node::Element(Arc::clone(&new))],
        &["bookmarkStart", "bookmarkEnd"],
        value,
        offset,
    );
    let keys = vec![Key::Part(Arc::from(part)), Key::Annotations];
    sink.replace_drawing(part, path, new, keys, draws)?;
    Ok(name)
}

/// The `_Toc<digits>` bookmarks starting below `element`, in order.
fn toc_names(element: &Element) -> Vec<String> {
    let mut out = Vec::new();
    for node in element.children().iter() {
        let Node::Element(child) = node else { continue };
        if child.name().is(Some(W), "bookmarkStart") {
            let name = child.attribute(Some(W), "name").unwrap_or("");
            let digits = name.strip_prefix("_Toc").unwrap_or("");
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                out.push(name.to_owned());
            }
        }
        out.extend(toc_names(child));
    }
    out
}

/// A table of contents' paragraphs in its container: from the one its begin is in to the
/// one its end is in.
struct Region {
    /// The container's path.
    container: Vec<u32>,
    /// The begin's paragraph's and the end's paragraph's child indexes.
    span: (u32, u32),
    /// The paths of the paragraphs from the begin's to the end's (`_toc_paragraphs`).
    places: Vec<Vec<u32>>,
    /// The begin's and the end's paths.
    begin: Vec<u32>,
    end: Vec<u32>,
}

impl Region {
    fn of(root: &Arc<Element>, toc: &Located) -> Option<Region> {
        let end = toc.found.end.clone()?;
        let first = paragraph_of(root, &toc.found.begin)?;
        let last = paragraph_of(root, &end)?;
        let (&b, container) = first.split_last()?;
        let (&e, other) = last.split_last()?;
        if container != other || e < b {
            return None;
        }
        let parent = at_path(root, container)?;
        let places = (b..=e)
            .filter(|k| {
                *k == b
                    || parent
                        .children()
                        .get(*k as usize)
                        .and_then(Node::as_element)
                        .is_some_and(|c| c.name().is(Some(W), "p"))
            })
            .map(|k| {
                let mut p = container.to_vec();
                p.push(k);
                p
            })
            .collect();
        Some(Region {
            container: container.to_vec(),
            span: (b, e),
            places,
            begin: toc.found.begin.clone(),
            end,
        })
    }

    /// The region replaced by `entries`, through `sink`.
    fn replace(
        &self,
        sink: &mut EditSink<'_, '_>,
        (part, root): (&str, &Arc<Element>),
        looks: &build::Looks<'_>,
        entries: &[build::Entry<'_>],
    ) -> Result<()> {
        let path_error = || EditError::Path(part.to_owned());
        let parent = at_path(root, &self.container).ok_or_else(path_error)?;
        let (b, e) = self.span;
        let child = |k: u32| parent.children().get(k as usize).and_then(Node::as_element).cloned();
        let last = child(e).ok_or_else(path_error)?;
        let mut last_path = self.container.clone();
        last_path.push(e);
        let mut end_work = Work::of(&last, scope_above(root, &last_path));
        let end_node =
            locate(&end_work, &last, self.end.get(last_path.len()..).unwrap_or_default()).ok_or_else(path_error)?;
        let end_run = work_run_of(&end_work, end_node);
        trim_before(&mut end_work, end_run);
        let end_root = end_work.root();
        let fresh = build::paragraphs(&mut end_work, end_root, looks, entries);
        let mut nodes: Vec<Node> = Vec::new();
        let mut detached: Vec<Arc<Element>> = Vec::new();
        let mut kept_begin = None;
        if b != e {
            let first = child(b).ok_or_else(path_error)?;
            let mut first_path = self.container.clone();
            first_path.push(b);
            let mut begin_work = Work::of(&first, scope_above(root, &first_path));
            let begin_node = locate(
                &begin_work,
                &first,
                self.begin.get(first_path.len()..).unwrap_or_default(),
            )
            .ok_or_else(path_error)?;
            let begin_run = work_run_of(&begin_work, begin_node);
            if trim_after(&mut begin_work, begin_run) {
                kept_begin = Some(begin_work.build().ok_or_else(path_error)?);
            } else {
                detached.push(Arc::clone(&first));
            }
            for k in b + 1..e {
                match parent.children().get(k as usize) {
                    Some(Node::Element(c)) if c.name().is(Some(W), "p") => detached.push(Arc::clone(c)),
                    Some(other) => nodes.push(other.clone()),
                    None => {}
                }
            }
        }
        let mut kept = detached.iter().filter_map(|p| {
            let para = p.attribute(Some(W14), "paraId")?;
            let text = p.attribute(Some(W14), "textId")?;
            (valid_long_hex(para) && valid_long_hex(text)).then(|| (para.to_owned(), text.to_owned()))
        });
        let prefix = w14_prefix(root, &last_path);
        let para_name = Arc::new(QName::new(Some(&prefix), "paraId", Some(W14)));
        let text_name = Arc::new(QName::new(Some(&prefix), "textId", Some(W14)));
        let mut out: Vec<Node> = Vec::new();
        if let Some(begin) = kept_begin {
            out.push(Node::Element(begin));
        }
        let mut draws = Vec::new();
        for paragraph in fresh {
            match kept.next() {
                Some((para, text)) => {
                    end_work.set(paragraph, Arc::clone(&para_name), &para);
                    end_work.set(paragraph, Arc::clone(&text_name), &text);
                }
                None => {
                    let node = u32::try_from(out.len()).unwrap_or(u32::MAX);
                    for name in [&para_name, &text_name] {
                        draws.push(Draw {
                            from: DrawFrom::Seed(Arc::from(format!("{part}\0toc"))),
                            node,
                            at: Vec::new(),
                            attribute: Arc::clone(name),
                        });
                    }
                }
            }
            out.push(Node::Element(end_work.build_element(paragraph).ok_or_else(path_error)?));
        }
        out.extend(nodes);
        out.push(Node::Element(end_work.build().ok_or_else(path_error)?));
        let keys = vec![Key::Part(Arc::from(part))];
        let count = (e - b) as usize + 1;
        sink.splice_children(part, (&self.container, b as usize, count), out, keys, draws)
    }
}

/// `paragraph`'s (the root's) children before `end_run` taken out, but its properties and
/// what holds `end_run`.
fn trim_before(work: &mut Work, end_run: Id) {
    let root = work.root();
    let holding = ancestors(work, end_run);
    for child in work.children(root).to_vec() {
        if child == end_run || work.is(child, W, "pPr") || holding.contains(&child) || !work.is_element(child) {
            continue;
        }
        if precedes(work, child, end_run) {
            work.remove(child);
        }
    }
}

/// The begin's paragraph (the root) cut at `begin_run` when something comes before it:
/// whether it is kept (else it goes whole).
fn trim_after(work: &mut Work, begin_run: Id) -> bool {
    let root = work.root();
    let kept_before = work
        .children(root)
        .iter()
        .any(|c| work.is_element(*c) && !work.is(*c, W, "pPr") && *c != begin_run && precedes(work, *c, begin_run));
    if !kept_before {
        return false;
    }
    for child in work.children(root).to_vec() {
        if child == begin_run || precedes(work, begin_run, child) {
            work.remove(child);
        }
    }
    true
}

fn ancestors(work: &Work, node: Id) -> Vec<Id> {
    let mut out = Vec::new();
    let mut at = work.parent(node);
    while let Some(parent) = at {
        out.push(parent);
        at = work.parent(parent);
    }
    out
}

/// Whether `left` comes before `right` in document order (an ancestor first).
fn precedes(work: &Work, left: Id, right: Id) -> bool {
    let mut order = Vec::new();
    preorder(work, work.root(), &mut order);
    let at = |n: Id| order.iter().position(|o| *o == n);
    matches!((at(left), at(right)), (Some(a), Some(b)) if a < b)
}

fn preorder(work: &Work, node: Id, out: &mut Vec<Id>) {
    out.push(node);
    for child in work.children(node).to_vec() {
        preorder(work, child, out);
    }
}
