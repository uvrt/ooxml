//! Owns: the lists copied blocks number with, brought over as docx-agent's importer brings
//! them (`_lists`, `num`, `abstract`, `_continued`): each source instance a paragraph (or an
//! imported style) names becomes a new instance in this document -- over a copy of the
//! source's definition under a new number (its `w:nsid` kept unless this document uses it,
//! its picture bullets dropped, the styles it links brought as the copy brings styles), the
//! instance's level overrides copied -- each source instance and definition brought once;
//! under [`ListPolicy::Continue`] a list whose first level counts as the list before the
//! place does joins that list instead.
//!
//! Must not: write but through [`crate::lists::numbering`] and [`Styles`].

use super::styles::{detached_copy, Styles};
use super::ListPolicy;
use crate::doc::Key;
use crate::lists::numbering;
use crate::names::W;
use crate::placing::{Place, BLOCKS};
use crate::review::space::Space;
use crate::review::xml::make;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_word_model::docx::numbering::Lists;
use ooxml_word_model::docx::{at_path, find_path, Docx};
use ooxml_word_model::xmlread::py_int;
use ooxml_xml::{Element, Node};
use std::collections::HashMap;
use std::sync::Arc;

/// What one copy did with lists: the instances and definitions brought, by source number.
pub(super) struct ListMaps {
    policy: ListPolicy,
    /// Where the copies go (`_continued` reads the list before it).
    place: Place,
    /// The source's numbering, once read.
    source: Option<Lists>,
    /// Source instance -> this document's.
    nums: HashMap<i64, i64>,
    /// Source definition (`None`: an instance naming none) -> this document's.
    abstracts: HashMap<Option<i64>, i64>,
    /// Whether this document's numbering part was declared read.
    read: bool,
}

impl ListMaps {
    pub(super) fn new(policy: ListPolicy, place: Place) -> ListMaps {
        ListMaps {
            policy,
            place,
            source: None,
            nums: HashMap::new(),
            abstracts: HashMap::new(),
            read: false,
        }
    }
}

/// A list number as docx-agent's `int(value or 0)` reads it.
fn number(value: Option<&str>) -> Result<i64> {
    match value {
        None | Some("") => Ok(0),
        Some(v) => py_int(v).ok_or_else(|| EditError::Unsupported("a list number docx-agent cannot read".to_owned())),
    }
}

/// `value` as Python's f-string writes an optional number.
fn py_number(value: Option<i64>) -> String {
    value.map_or_else(|| "None".to_owned(), |v| v.to_string())
}

/// A level's `w:numFmt` value.
fn number_format(level: Option<&Arc<Element>>) -> Option<String> {
    level
        .and_then(|l| l.find(Some(W), "numFmt"))
        .and_then(|f| f.attribute(Some(W), "val"))
        .map(str::to_owned)
}

impl Styles<'_> {
    /// Every list number of `element`'s paragraphs pointed at the instance it becomes here
    /// (`_lists`).
    ///
    /// # Errors
    /// What [`Styles::num`] refuses or leaves to the host.
    pub(crate) fn bring_lists(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        work: &mut Work,
        element: Id,
    ) -> Result<()> {
        for node in work.iter(element, W, &["numId"]) {
            if !work.parent(node).is_some_and(|p| work.is(p, W, "numPr")) {
                continue;
            }
            let value = work.get(node, Some(W), "val").map(str::to_owned);
            let mapped = self.num(sink, space, number(value.as_deref())?)?.to_string();
            if value.as_deref() != Some(mapped.as_str()) {
                let name = work.qname(W, "val", "w");
                work.set(node, name, &mapped);
            }
        }
        Ok(())
    }

    /// Every `w:numId` of the imported style `copy` (in `work`) pointed at the instance it
    /// becomes here.
    ///
    /// # Errors
    /// What [`Styles::num`] refuses or leaves to the host.
    pub(super) fn style_lists(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        work: &mut Work,
        copy: Id,
    ) -> Result<()> {
        for node in work.iter(copy, W, &["numId"]) {
            let value = work.get(node, Some(W), "val").map(str::to_owned);
            let mapped = self.num(sink, space, number(value.as_deref())?)?.to_string();
            if value.as_deref() != Some(mapped.as_str()) {
                let name = work.qname(W, "val", "w");
                work.set(node, name, &mapped);
            }
        }
        Ok(())
    }

    /// The instance here a source list's paragraphs go into (`num`): 0 for none or one the
    /// source lacks.
    fn num(&mut self, sink: &mut EditSink<'_, '_>, space: &mut Space, num_id: i64) -> Result<i64> {
        if num_id == 0 {
            return Ok(0);
        }
        if let Some(&mapped) = self.lists.nums.get(&num_id) {
            return Ok(mapped);
        }
        let main = sink.docx()?.main.clone();
        if !self.lists.read {
            // what is numbered here (and, for this document as the source, there) is read
            if let Some(part) = numbering::part_of(sink, &main)? {
                sink.read(numbering::part_key(&part));
            }
            self.lists.read = true;
        }
        let source = self.lists.source.get_or_insert_with(|| Lists::read(self.source));
        let Some(instance) = source.num_element(num_id).cloned() else {
            return Ok(0);
        };
        let abstract_id = source.abstract_of(num_id);
        if self.lists.policy == ListPolicy::Continue {
            if let Some(joined) = self.continued(sink, space, num_id)? {
                self.lists.nums.insert(num_id, joined);
                return Ok(joined);
            }
        }
        let new_abstract = self.abstract_definition(sink, space, abstract_id)?;
        let mut overrides = Vec::new();
        for over in instance
            .child_elements()
            .filter(|o| o.name().is(Some(W), "lvlOverride"))
        {
            let (work, copy) = detached_copy(over)?;
            overrides.push(
                work.build_element(copy)
                    .ok_or_else(|| EditError::Path("numbering".to_owned()))?,
            );
        }
        let new_num = numbering::add_num_copying(sink, &main, new_abstract, overrides)?;
        self.lists.nums.insert(num_id, new_num);
        Ok(new_num)
    }

    /// The definition here a source definition becomes (`abstract`): a copy of it (Word's
    /// own numbered list for none) under the next number, its `w:nsid` drawn anew where this
    /// document uses it, its picture bullets dropped, the styles it links brought.
    fn abstract_definition(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        space: &mut Space,
        abstract_id: Option<i64>,
    ) -> Result<i64> {
        if let Some(&mapped) = self.lists.abstracts.get(&abstract_id) {
            return Ok(mapped);
        }
        let main = sink.docx()?.main.clone();
        let part = numbering::part_of(sink, &main)?;
        let root = match &part {
            Some(part) => sink.doc().tree(part)?.map(|t| Arc::clone(t.root())),
            None => None,
        };
        let new_id = root
            .as_deref()
            .map(|r| numbering::listed(r, "abstractNum", "abstractNumId"))
            .unwrap_or_default()
            .iter()
            .map(|(n, _)| *n)
            .max()
            .unwrap_or(-1)
            + 1;
        let original = abstract_id.and_then(|id| self.lists.source.as_ref()?.abstract_element(id).cloned());
        let (mut work, node) = match original {
            Some(original) => detached_copy(&original)?,
            None => {
                let made =
                    crate::list_style::parsed(&numbering::builtin_abstract("number", new_id, "00000001", "00000001"))?;
                let work = Work::of(&made, Vec::new());
                let root = work.root();
                (work, root)
            }
        };
        let id_name = work.qname(W, "abstractNumId", "w");
        work.set(node, id_name, &new_id.to_string());
        let used = numbering::used_nsids(sink, part.as_deref())?;
        let found = work.children(node).iter().copied().find(|c| work.is(*c, W, "nsid"));
        let taken = found.is_some_and(|n| used.contains(&work.get(n, Some(W), "val").unwrap_or("").to_uppercase()));
        if found.is_none() || taken {
            let nsid = found.unwrap_or_else(|| {
                let made = make(&mut work, "nsid", &[]);
                work.insert(node, 0, made);
                made
            });
            let value = numbering::nsid(&format!("import\0{}\0{new_id}", py_number(abstract_id)), &used);
            let val = work.qname(W, "val", "w");
            work.set(nsid, val, &value);
        }
        for picture in work.iter(node, W, &["lvlPicBulletId"]) {
            work.remove(picture);
        }
        self.lists.abstracts.insert(abstract_id, new_id);
        for link in work.iter(node, W, &["styleLink", "numStyleLink", "pStyle"]) {
            let kind = if work.is(link, W, "pStyle") {
                "paragraph"
            } else {
                "numbering"
            };
            self.point(sink, space, &mut work, link, kind)?;
        }
        let built = work
            .build_element(node)
            .ok_or_else(|| EditError::Path("numbering".to_owned()))?;
        numbering::add_abstract(sink, &main, built)?;
        Ok(new_id)
    }

    /// The list before the place when its first level counts as source list `num_id`'s does
    /// (`_continued`): the last numbered paragraph at or before the block the copies follow.
    fn continued(&mut self, sink: &mut EditSink<'_, '_>, space: &mut Space, num_id: i64) -> Result<Option<i64>> {
        let place = self.lists.place.clone();
        let Some(tree) = sink.doc().tree(&place.part)? else {
            return Ok(None);
        };
        let root = Arc::clone(tree.root());
        let Some(container) = at_path(&root, &place.container) else {
            return Ok(None);
        };
        let anchor = match place.before {
            Some(k) => {
                let previous = container
                    .children()
                    .iter()
                    .take(k as usize)
                    .filter(|n| !matches!(n, Node::Text(_)))
                    .last();
                match previous {
                    Some(Node::Element(e)) => Some(Arc::clone(e)),
                    _ => None,
                }
            }
            None => container
                .child_elements()
                .filter(|e| e.name().namespace() == Some(W) && BLOCKS.contains(&e.name().local()))
                .last()
                .cloned(),
        };
        let Some(anchor) = anchor else { return Ok(None) };
        sink.read(Key::Part(Arc::from(place.part.as_str())));
        let last = if anchor.name().is(Some(W), "p") {
            Some(anchor)
        } else {
            paragraphs(&anchor).last().cloned()
        };
        let Some(last) = last else { return Ok(None) };
        let all = paragraphs(&root);
        let Some(index) = all.iter().position(|p| Arc::ptr_eq(p, &last)) else {
            return Ok(None);
        };
        let mut previous = None;
        for paragraph in all.iter().take(index + 1).rev() {
            let value = find_path(paragraph, &["pPr", "numPr", "numId"]).and_then(|n| n.attribute(Some(W), "val"));
            if let Some(value) = value.filter(|v| *v != "0") {
                // Python's int(): a value it cannot read raises
                let read = py_int(value)
                    .ok_or_else(|| EditError::Unsupported("a list number docx-agent cannot read".to_owned()))?;
                previous = Some(read);
                break;
            }
        }
        let Some(previous) = previous else { return Ok(None) };
        let source = self.lists.source.get_or_insert_with(|| Lists::read(self.source));
        let source_format = number_format(source.level(self.source, num_id, 0).as_ref());
        // this document's lists as they are now, the styles imported so far in them
        if let Some(part) = self.part.clone() {
            space.put_part(sink, &part)?;
        }
        let docx: Arc<Docx> = sink.docx()?;
        let dest_format = number_format(Lists::read(&docx).level(&docx, previous, 0).as_ref());
        Ok((source_format.is_some() && source_format == dest_format).then_some(previous))
    }
}

/// The paragraphs at or below `element`, in document order.
fn paragraphs(element: &Arc<Element>) -> Vec<Arc<Element>> {
    fn walk(element: &Arc<Element>, out: &mut Vec<Arc<Element>>) {
        if element.name().is(Some(W), "p") {
            out.push(Arc::clone(element));
        }
        for child in element.child_elements() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(element, &mut out);
    out
}
