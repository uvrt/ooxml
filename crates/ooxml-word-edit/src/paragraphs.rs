//! Owns: new paragraphs beside a block, as docx-agent writes them -- [`InsertParagraph`]
//! (`Document.insert_paragraph`: the paragraph it is put beside continued, its properties
//! without a section break or revision marks, its nearest text run's formatting; stamped
//! at once; tracked, recorded as inserted) -- and a paragraph's text given in lines
//! (`set_text_paragraphs`: the first line the paragraph's, each later one a new paragraph
//! after it).
//!
//! Must not: write but through the sink, or read outside what it records reading: the
//! block it inserts beside, the paragraphs it compares a text's lines with, the ledger
//! seeds it draws from, the annotation ids when tracked.

use crate::doc::{Key, WordDoc};
use crate::names::{W, W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::ranges::{renamed_id, stretch};
use crate::set_text::{block_key, cell_paragraph, find, paragraph_has_annotations, write_text, TextEdited};
use crate::sink::{annotation_draws, EditSink};
use crate::stamping::{stamp_document, w14_prefix};
use crate::text::write::{check_text, make_run, run_properties_for};
use crate::text::{paragraph_text, walk};
use crate::work::{Id, Work};
use crate::{EditError, Result};
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::direct;
use ooxml_word_model::docx::{at_path, Docx};
use ooxml_word_model::ids::Entry;
use ooxml_xml::{Element, Node, QName};
use std::sync::Arc;

/// Insert a new paragraph beside a block.
#[derive(Debug, Clone)]
pub struct InsertParagraph {
    /// The paragraph or table it goes beside (an id, an alias, a cell's first paragraph).
    pub anchor: String,
    /// After the anchor (`true`) or before it.
    pub after: bool,
    /// Its text: one line (`\v` a line break, `\t` a tab); empty for an empty paragraph.
    pub text: String,
    /// A paragraph style (name, alias or id; a built-in one the document lacks added as
    /// Word writes it) in place of the properties it would continue.
    pub style: Option<String>,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

/// What an insert_paragraph did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphInserted {
    /// The new paragraph's id.
    pub id: ooxml_word_model::ids::BlockId,
    /// Ids the edit's stamping renamed, old to new, in the order made.
    pub renamed: ooxml_word_model::ids::Renames,
    /// Its part.
    pub part: ooxml_opc::PartName,
}

/// A block a paragraph goes beside.
pub(crate) struct Anchor {
    pub(crate) part: String,
    pub(crate) story: String,
    pub(crate) path: Box<[u32]>,
    pub(crate) id: String,
    pub(crate) paragraph: bool,
}

/// The block `target` names now: an alias followed while its target exists; a cell's
/// first paragraph.
pub(crate) fn anchor(doc: &WordDoc, docx: &Docx, target: &str) -> Result<Anchor> {
    let followed = doc.ids().follow(target);
    let block = (followed != target)
        .then(|| direct(docx, &followed))
        .flatten()
        .or_else(|| direct(docx, target));
    let Some((story, entry)) = block else {
        let found = cell_paragraph(docx, target)?;
        let story = docx
            .part(&found.part)
            .map(|s| s.story.clone())
            .ok_or_else(|| EditError::no_part(found.part.clone()))?;
        return Ok(Anchor {
            part: found.part,
            story,
            path: found.path,
            id: found.id,
            paragraph: true,
        });
    };
    let missing = || EditError::Refused(format!("{target} is not a paragraph or a table"));
    let (path, id, paragraph) = match entry {
        Entry::Paragraph(k) => {
            let p = story.ids.paragraphs().get(k).ok_or_else(missing)?;
            (p.path.clone(), p.id.clone(), true)
        }
        Entry::Table(k) => {
            let t = story.ids.tables().get(k).ok_or_else(missing)?;
            (t.path.clone(), t.id.clone(), false)
        }
    };
    Ok(Anchor {
        part: story.part.clone(),
        story: story.story.clone(),
        path,
        id,
        paragraph,
    })
}

/// Whether a block (a paragraph, a table, a content control, custom XML) follows the
/// child `index` of `parent`: docx-agent's `is_last_in_container`, negated.
pub(crate) fn followed(parent: &Element, index: usize) -> bool {
    parent.children().iter().skip(index + 1).any(|node| {
        matches!(node, Node::Element(e) if e.name().namespace() == Some(W)
            && matches!(e.name().local(), "p" | "tbl" | "sdt" | "customXml"))
    })
}

/// The new paragraph, detached: the anchor paragraph's properties (no section break,
/// change record or revision marks), a run of `text` formatted as the anchor's nearest
/// text run (the last after it, the first before it); with `style`, that paragraph style
/// alone and a run without properties.
fn made(work: &mut Work, anchor: Id, template: bool, after: bool, text: &str, style: Option<&str>) -> Id {
    let paragraph = work.new_element(work.element_name(work.root(), W, "p", "w"));
    if let Some(style) = style {
        let properties = work.new_element(work.element_name(paragraph, W, "pPr", "w"));
        let node = work.new_element(work.element_name(properties, W, "pStyle", "w"));
        let val = work.qname(W, "val", "w");
        work.set(node, val, style);
        work.append(properties, node);
        work.append(paragraph, properties);
    }
    let template = template && style.is_none();
    let properties = template.then(|| work.find(anchor, W, "pPr")).flatten();
    if let Some(properties) = properties {
        let copy = work.deep_copy(properties);
        for child in work.children(copy).to_vec() {
            if work.is_any(child, W, &["sectPr", "pPrChange"]) {
                work.remove(child);
            } else if work.is(child, W, "rPr") {
                for mark in work.children(child).to_vec() {
                    if work.is_any(mark, W, &["ins", "del", "moveFrom", "moveTo", "rPrChange"]) {
                        work.remove(mark);
                    }
                }
                if work.children(child).is_empty() {
                    work.remove(child);
                }
            }
        }
        if !work.children(copy).is_empty() {
            work.append(paragraph, copy);
        }
    }
    if !text.is_empty() {
        let run_properties = template.then(|| {
            let read = walk(work, anchor);
            let with_text: Vec<Id> = read
                .runs
                .iter()
                .copied()
                .filter(|r| read.atoms.iter().any(|a| a.run == Some(*r)))
                .collect();
            let reference = if after { with_text.last() } else { with_text.first() };
            run_properties_for(work, anchor, reference.copied())
        });
        let run = make_run(work, text, run_properties.flatten());
        work.append(paragraph, run);
    }
    paragraph
}

/// docx-agent's `_insert_paragraph` inside an edit: a new paragraph of `text` after (or
/// before) the block `target` names; its id, the stamping's renames and its part.
///
/// # Errors
/// [`EditError::Text`] for text a paragraph cannot hold; a refusal for an anchor that is
/// not a paragraph or a table.
pub(crate) fn insert_paragraph(
    sink: &mut EditSink<'_, '_>,
    target: &str,
    (after, text, style): (bool, &str, Option<&str>),
) -> Result<ParagraphInserted> {
    check_text(text)?;
    let docx = sink.docx()?;
    let anchor = anchor(sink.doc(), &docx, target)?;
    if let Some(name) = style {
        crate::builtin::resolve(&docx, name, "paragraph")?;
    }
    let part = anchor.part.clone();
    sink.read(block_key(&part, &anchor.id));
    let renamed = stamp_document(sink, &docx)?;
    let anchor_id = renamed_id(&renamed, &anchor.id);
    let style = match style {
        Some(name) => Some(crate::builtin::ensure(sink, name, "paragraph")?),
        None => None,
    };
    let tree = sink
        .doc()
        .tree(&part)?
        .ok_or_else(|| EditError::no_part(part.clone()))?;
    let path_error = || EditError::no_element(part.clone());
    let (&index, above) = anchor.path.split_last().ok_or_else(path_error)?;
    let parent = if above.is_empty() {
        tree.root()
    } else {
        at_path(tree.root(), above).ok_or_else(path_error)?
    };
    let last = after && !followed(parent, index as usize);
    let (mut work, placed) = stretch(tree.root(), std::slice::from_ref(&anchor.path)).ok_or_else(path_error)?;
    let &anchor_node = placed.first().ok_or_else(path_error)?;
    let paragraph = made(&mut work, anchor_node, anchor.paragraph, after, text, style.as_deref());
    let drawn = sink.draw(&part, 2)?;
    let (Some(para_id), Some(text_id)) = (drawn.first(), drawn.get(1)) else {
        return Err(path_error());
    };
    let prefix = w14_prefix(tree.root(), &anchor.path);
    for (local, value) in [("paraId", para_id), ("textId", text_id)] {
        work.set(paragraph, Arc::new(QName::new(Some(&prefix), local, Some(W14))), value);
    }
    if after {
        work.add_next(anchor_node, paragraph);
    } else {
        work.add_previous(anchor_node, paragraph);
    }
    let wrote = sink.new_paragraph(&mut work, paragraph, last)?;
    sink.keep_written(&part, None, &work);
    let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
    let mut draws = Vec::new();
    let mut annotated = false;
    for (k, node) in nodes.iter().enumerate() {
        if let Node::Element(e) = node {
            annotated |= paragraph_has_annotations(e);
            if let (Some(numbered), Ok(k)) = (wrote.annotations, u32::try_from(k)) {
                draws.extend(annotation_draws(e, numbered, k));
            }
        }
    }
    let new_id = if anchor.story == "body" {
        format!("p:{para_id}")
    } else {
        format!("{}/p:{para_id}", anchor.story)
    };
    let mut keys = vec![block_key(&part, &anchor_id), block_key(&part, &new_id)];
    if annotated {
        keys.push(Key::Annotations);
    }
    sink.splice_children(&part, (above, index as usize, 1), nodes, keys, draws)?;
    if wrote.dated {
        sink.ensure_w14(&part, false, &[("w16du", W16DU)])?;
    }
    sink.ensure_w14(&part, false, &[])?;
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        let gave = wrote.annotations.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
    }
    Ok(ParagraphInserted {
        id: new_id.into(),
        renamed,
        part: part.into(),
    })
}

impl Command<WordDoc> for InsertParagraph {
    type Out = ParagraphInserted;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        check_text(&self.text)?;
        let docx = doc.docx()?;
        anchor(doc, &docx, &self.anchor)?;
        if let Some(name) = &self.style {
            crate::builtin::resolve(&docx, name, "paragraph")?;
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ParagraphInserted> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = insert_paragraph(&mut sink, &self.anchor, (self.after, &self.text, self.style.as_deref()))?;
        // docx-agent's edit, once it ends: the written text spaced, text boxes whose
        // content changed synced, revision ids unique
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

/// A text's lines: split at `\n`, `\r\n` and `\r`.
pub(crate) fn lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(['\n', '\r']) {
        out.push(&rest[..at]);
        let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[at + skip..];
    }
    out.push(rest);
    out
}

/// Whether a text is more than one line.
pub(crate) fn is_lines(text: &str) -> bool {
    text.contains(['\n', '\r'])
}

/// The `w:pStyle` value of a paragraph, if it has one.
pub(crate) fn style_of(paragraph: &Element) -> Option<&str> {
    paragraph
        .child_elements()
        .find(|c| c.name().is(Some(W), "pPr"))?
        .child_elements()
        .find(|c| c.name().is(Some(W), "pStyle"))?
        .attribute(Some(W), "val")
}

/// Whether the paragraph at `path` of `part` and the paragraphs right after it hold
/// `lines`, all in its paragraph style (docx-agent's `_already_lines`); the paragraphs
/// compared are read.
fn already_lines(sink: &mut EditSink<'_, '_>, docx: &Docx, part: &str, path: &[u32], lines: &[&str]) -> Result<bool> {
    let tree = sink
        .doc()
        .tree(part)?
        .ok_or_else(|| EditError::no_part(part.to_owned()))?;
    let path_error = || EditError::no_element(part.to_owned());
    let (&index, above) = path.split_last().ok_or_else(path_error)?;
    let parent = if above.is_empty() {
        tree.root()
    } else {
        at_path(tree.root(), above).ok_or_else(path_error)?
    };
    let first = at_path(tree.root(), path).ok_or_else(path_error)?;
    let wanted = style_of(first).map(str::to_owned);
    let mut blocks = parent
        .children()
        .iter()
        .enumerate()
        .skip(index as usize + 1)
        .filter_map(|(k, node)| match node {
            Node::Element(e) if e.name().namespace() == Some(W) && matches!(e.name().local(), "p" | "tbl") => {
                Some((k, Arc::clone(e)))
            }
            _ => None,
        });
    let story = docx.part(part);
    for (k, line) in lines.iter().enumerate() {
        let (place, element) = if k == 0 {
            (index as usize, Arc::clone(first))
        } else {
            match blocks.next() {
                Some(found) => found,
                None => return Ok(false),
            }
        };
        let mut at = above.to_vec();
        at.push(u32::try_from(place).map_err(|_| path_error())?);
        if let Some(found) = story.and_then(|s| s.ids.paragraph_at(&at)) {
            sink.read(block_key(part, &found.id));
        }
        if !element.name().is(Some(W), "p") {
            return Ok(false);
        }
        let work = Work::of(&element, Vec::new());
        if paragraph_text(&work, work.root()) != *line || style_of(&element).map(str::to_owned) != wanted {
            return Ok(false);
        }
    }
    Ok(true)
}

/// docx-agent's `set_text_paragraphs`: the paragraph `target` names given the first of
/// `text`'s lines, each later line a new paragraph after it, in order; nothing when the
/// paragraph and those after it hold the lines already; refused for a paragraph that ends
/// a section.
///
/// # Errors
/// [`EditError::Text`] for a line a paragraph cannot hold; a refusal for a target that is
/// not a paragraph or ends a section.
pub(crate) fn set_lines(sink: &mut EditSink<'_, '_>, target: &str, text: &str) -> Result<TextEdited> {
    let out = set_lines_in(sink, target, text)?;
    // docx-agent's batch, once it ends: the written text spaced, text boxes whose
    // content changed synced, revision ids unique
    if out.changed && is_lines(text) {
        crate::after::after_edit(sink)?;
    }
    Ok(out)
}

/// [`set_lines`] inside a batch: without making revision ids unique, which the batch does
/// once it ends.
///
/// # Errors
/// As [`set_lines`].
pub(crate) fn set_lines_in(sink: &mut EditSink<'_, '_>, target: &str, text: &str) -> Result<TextEdited> {
    let lines = lines(text);
    for line in &lines {
        check_text(line)?;
    }
    let [head, rest @ ..] = lines.as_slice() else {
        return write_text(sink, target, text);
    };
    if rest.is_empty() {
        return write_text(sink, target, text);
    }
    let docx = sink.docx()?;
    let found = find(sink.doc(), &docx, target)?;
    sink.read(block_key(&found.part, &found.id));
    let ends_section = found
        .element
        .child_elements()
        .find(|c| c.name().is(Some(W), "pPr"))
        .is_some_and(|p| p.child_elements().any(|c| c.name().is(Some(W), "sectPr")));
    if ends_section {
        return Err(EditError::Refused(format!(
            "{} ends a section: paragraphs split after it at \\n would start the next section; give it one \
             line and insert the others before it (word_insert_markdown at before:)",
            found.id
        )));
    }
    if already_lines(sink, &docx, &found.part, &found.path, &lines)? {
        return Ok(TextEdited {
            id: found.id.into(),
            changed: false,
            renamed: Vec::new(),
            part: found.part.into(),
            created: Vec::new(),
        });
    }
    let first = write_text(sink, &found.id, head)?;
    let mut renamed = first.renamed.clone();
    let mut created = Vec::new();
    let mut current = first.id.clone();
    for line in rest {
        let made = insert_paragraph(sink, &current, (true, line, None))?;
        for (old, new) in made.renamed {
            match renamed.iter_mut().find(|(o, _)| *o == old) {
                Some(slot) => slot.1 = new,
                None => renamed.push((old, new)),
            }
        }
        created.push(made.id.to_string());
        current = made.id;
    }
    Ok(TextEdited {
        id: renamed
            .iter()
            .find(|(old, _)| *old == found.id)
            .map_or_else(|| first.id.clone(), |(_, new)| new.to_string().into()),
        changed: true,
        renamed,
        part: found.part.into(),
        created,
    })
}
