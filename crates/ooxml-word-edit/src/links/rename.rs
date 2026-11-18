//! Owns: [`RenameBookmark`] and [`RemoveBookmark`], as docx-agent's `rename_bookmark` and
//! `remove_bookmark` do them: a rename renames what points at the bookmark too -- internal
//! hyperlinks and `REF`, `PAGEREF` and `NOTEREF` field instructions in every story; a
//! removal takes its markers out (its text stays) and says what still points at it.
//!
//! Must not: write but through the sink; it reads every story.

use super::LinkEdited;
use crate::delete::{editable, stretch_keys};
use crate::doc::Key;
use crate::names::W;
use crate::sink::EditSink;
use crate::stamping::{stamp_document, with_attribute};
use crate::{EditError, Result, WordDoc};
use ooxml_base::pyfmt::repr_str;
use ooxml_store::{Command, Txn};
use ooxml_word_model::address::bookmark;
use ooxml_word_model::address::marks::bookmark_markers;
use ooxml_word_model::docx::{casefold, Docx};
use ooxml_xml::{Children, Element, HeadPatch, Node, NodePath};
use std::sync::Arc;

/// A bookmark renamed, with what points at it.
#[derive(Debug, Clone)]
pub struct RenameBookmark {
    /// The bookmark (`bm:` optional; any case).
    pub name: String,
    /// Its new name.
    pub new: String,
}

/// A bookmark's markers removed.
#[derive(Debug, Clone)]
pub struct RemoveBookmark {
    /// The bookmark (`bm:` optional; any case).
    pub name: String,
}

/// The bookmark's name as written.
fn existing(docx: &Docx, name: &str) -> Result<String> {
    Ok(bookmark(docx, name).map_err(EditError::Address)?.name)
}

impl Command<WordDoc> for RenameBookmark {
    type Out = LinkEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        existing(&*doc.docx()?, &self.name)?;
        super::check_name(&self.new)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<LinkEdited> {
        let mut sink = EditSink::new(tx, crate::RevisionPolicy::Untracked);
        sink.read_everything();
        let docx = sink.docx()?;
        let old = existing(&docx, &self.name)?;
        super::check_name(&self.new)?;
        let new_folded = casefold(&self.new);
        if bookmark_markers(&docx)
            .iter()
            .any(|b| casefold(&b.name) == new_folded && b.name != old)
        {
            return Err(EditError::Refused(format!(
                "the document already has a bookmark {}",
                repr_str(&self.new)
            )));
        }
        let stamped = stamp_document(&mut sink, &docx)?;
        let docx = sink.docx()?;
        let marked = bookmark(&docx, &old).map_err(EditError::Address)?;
        let (marked_part, start) = (marked.part.part.clone(), marked.start.clone());
        for story in &docx.parts {
            let mut edits = Vec::new();
            walk(story.root(), &mut Vec::new(), &mut |element, path| {
                let rename = if story.part == marked_part && path == &start[..] {
                    Some(("name", self.new.clone()))
                } else if is(element, "hyperlink") && element.attribute(Some(W), "anchor") == Some(old.as_str()) {
                    Some(("anchor", self.new.clone()))
                } else if is(element, "fldSimple") {
                    let instruction = element.attribute(Some(W), "instr").unwrap_or("");
                    let renamed = rename_in_instruction(instruction, &old, &self.new);
                    (renamed != instruction).then_some(("instr", renamed))
                } else {
                    None
                };
                if let Some(rename) = rename {
                    edits.push(Edit::Attribute(path.to_vec(), Arc::clone(element.head()), rename));
                } else if is(element, "instrText") {
                    let text = element.text_content();
                    let renamed = rename_in_instruction(&text, &old, &self.new);
                    if renamed != text {
                        edits.push(Edit::Text(path.to_vec(), Arc::clone(element.head()), renamed));
                    }
                }
            });
            for edit in edits {
                edit.write(&mut sink, &story.part)?;
            }
        }
        let mut renamed = vec![(format!("bm:{old}"), format!("bm:{}", self.new))];
        renamed.extend(stamped);
        let id = format!("bm:{}", self.new);
        Ok(LinkEdited {
            id: Some(id),
            changed: true,
            renamed,
            ..LinkEdited::default()
        })
    }
}

/// One element's rename: an attribute set, or an instruction's text.
enum Edit {
    Attribute(Vec<u32>, Arc<ooxml_xml::Head>, (&'static str, String)),
    Text(Vec<u32>, Arc<ooxml_xml::Head>, String),
}

impl Edit {
    fn write(self, sink: &mut EditSink<'_, '_>, part: &str) -> Result<()> {
        match self {
            Edit::Attribute(path, head, (local, value)) => {
                let name = head
                    .attributes
                    .iter()
                    .find(|a| a.name.is(Some(W), local))
                    .map(|a| Arc::clone(&a.name))
                    .ok_or_else(|| EditError::Path(part.to_owned()))?;
                let new = with_attribute(&head, &name, &value);
                let mut node_path = vec![0usize];
                node_path.extend(path.iter().map(|k| *k as usize));
                sink.set_head(part, HeadPatch::new(NodePath(node_path), head, Arc::new(new)))
            }
            Edit::Text(path, head, text) => {
                let element = Element::from_head(head, Children::from_vec(vec![Node::Text(Arc::from(text.as_str()))]));
                sink.replace(part, &path, Arc::new(element), vec![Key::Part(Arc::from(part))])
            }
        }
    }
}

fn is(element: &Element, local: &str) -> bool {
    element.name().is(Some(W), local)
}

/// Every element below `root` with its place (child indexes from the root), in document
/// order.
fn walk(root: &Element, path: &mut Vec<u32>, visit: &mut dyn FnMut(&Arc<Element>, &[u32])) {
    for (k, node) in root.children().iter().enumerate() {
        if let Node::Element(child) = node {
            path.push(u32::try_from(k).unwrap_or(u32::MAX));
            visit(child, path);
            walk(child, path, visit);
            path.pop();
        }
    }
}

impl Command<WordDoc> for RemoveBookmark {
    type Out = LinkEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        existing(&*doc.docx()?, &self.name).map(|_| ())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<LinkEdited> {
        let mut sink = EditSink::new(tx, crate::RevisionPolicy::Untracked);
        sink.read_everything();
        let docx = sink.docx()?;
        let old = existing(&docx, &self.name)?;
        let mut dangling = Vec::new();
        for story in &docx.parts {
            let mut links = 0;
            let mut fields = 0;
            walk(story.root(), &mut Vec::new(), &mut |element, _| {
                if is(element, "hyperlink") && element.attribute(Some(W), "anchor") == Some(old.as_str()) {
                    links += 1;
                } else if is(element, "instrText") {
                    let text = element.text_content();
                    if rename_in_instruction(&text, &old, "x") != text {
                        fields += 1;
                    }
                }
            });
            dangling.extend(std::iter::repeat_n(format!("a hyperlink in {}", story.story), links));
            dangling.extend(std::iter::repeat_n(format!("a field in {}", story.story), fields));
        }
        let renamed = stamp_document(&mut sink, &docx)?;
        let docx = sink.docx()?;
        let marked = bookmark(&docx, &old).map_err(EditError::Address)?;
        let part = marked.part.part.clone();
        let mut paths = vec![marked.start.to_vec()];
        paths.extend(marked.end.as_deref().map(<[u32]>::to_vec));
        let wanted: Vec<&[u32]> = paths.iter().map(Vec::as_slice).collect();
        let root = Arc::clone(marked.part.root());
        let path_error = || EditError::Path(part.clone());
        let (mut work, parent, stretch, found) = editable(&root, &paths, &wanted).ok_or_else(path_error)?;
        for node in found {
            work.remove(node);
        }
        let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
        let mut keys = stretch_keys(&docx, &part, &parent, stretch);
        keys.push(Key::Annotations);
        let count = (stretch.1 - stretch.0) as usize + 1;
        sink.splice_children(&part, (&parent, stretch.0 as usize, count), nodes, keys, Vec::new())?;
        Ok(LinkEdited {
            id: None,
            changed: true,
            removed: vec![format!("bm:{old}")],
            renamed,
            warnings: dangling
                .into_iter()
                .map(|what| format!("{what} still points at the removed bookmark {}", repr_str(&old)))
                .collect(),
            ..LinkEdited::default()
        })
    }
}

/// `instruction` with every `REF`, `PAGEREF` or `NOTEREF` of `old` naming `new`
/// (docx-agent's `_rename_in_instruction`: `(?i)\b(?:REF|PAGEREF|NOTEREF)\s+("?)name\1`).
pub(crate) fn rename_in_instruction(instruction: &str, old: &str, new: &str) -> String {
    let chars: Vec<char> = instruction.chars().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(instruction.len());
    let mut k = 0;
    while k < chars.len() {
        let boundary = k == 0 || chars.get(k - 1).is_some_and(|c| !word(*c));
        let found = if boundary { reference_at(&chars, k) } else { None };
        match found {
            Some((name_start, name_end, end)) => {
                let name: String = chars.get(name_start..name_end).unwrap_or_default().iter().collect();
                out.extend(chars.get(k..name_start).unwrap_or_default());
                out.push_str(if name == old { new } else { &name });
                out.extend(chars.get(name_end..end).unwrap_or_default());
                k = end;
            }
            None => {
                out.extend(chars.get(k));
                k += 1;
            }
        }
    }
    out
}

/// A reference starting at `k`: its name's start and end and the match's end.
fn reference_at(chars: &[char], k: usize) -> Option<(usize, usize, usize)> {
    let upper = |c: &char| c.to_uppercase().next().unwrap_or(*c);
    let keyword = ["REF", "PAGEREF", "NOTEREF"].into_iter().find(|w| {
        let found: Vec<char> = chars
            .get(k..k + w.len())
            .unwrap_or_default()
            .iter()
            .map(upper)
            .collect();
        found.iter().copied().eq(w.chars())
    })?;
    let mut at = k + keyword.len();
    let spaces = chars.get(at..)?.iter().take_while(|c| c.is_whitespace()).count();
    if spaces == 0 {
        return None;
    }
    at += spaces;
    let quoted = chars.get(at) == Some(&'"');
    let start = if quoted { at + 1 } else { at };
    let length = chars
        .get(start..)?
        .iter()
        .take_while(|c| !c.is_whitespace() && **c != '"' && **c != '\\')
        .count();
    if length == 0 {
        return None;
    }
    let end = start + length;
    if quoted {
        return (chars.get(end) == Some(&'"')).then_some((start, end, end + 1));
    }
    Some((start, end, end))
}

#[cfg(test)]
mod tests {
    use super::rename_in_instruction;

    #[test]
    fn references_renamed() {
        assert_eq!(rename_in_instruction(" REF Old \\h ", "Old", "New"), " REF New \\h ");
        assert_eq!(
            rename_in_instruction(" pageref \"Old\" ", "Old", "New"),
            " pageref \"New\" "
        );
        assert_eq!(
            rename_in_instruction(" PAGEREF Older ", "Old", "New"),
            " PAGEREF Older "
        );
        assert_eq!(rename_in_instruction("XREF Old", "Old", "New"), "XREF Old");
    }
}
