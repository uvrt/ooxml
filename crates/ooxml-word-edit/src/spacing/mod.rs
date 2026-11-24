//! Owns: the spacing around punctuation an edit gives the text it wrote, as docx-agent's
//! `edit/typography.py` writes it (`Recorder`, `apply_written`; the rules and the language
//! cascade are `ooxml_word_model::typography`'s):
//! - [`Written`], the record of what an edit wrote, by stable place: a paragraph's part and
//!   `w14:paraId`, and the places of the written characters in its text (current view). A
//!   [`crate::work::Work`] carries the marks of what is written in it; the sink takes them
//!   into the record when the work is handed back, and puts them on a later work of the same
//!   paragraph, so a later write in the same edit shifts them as it changes the text;
//! - [`space_paragraph`]: where the rule of the text's language is not met beside written
//!   text, an ordinary space the edit wrote becomes U+00A0, and where there is none U+00A0
//!   goes in next to what it wrote. Nothing is removed; a space that was there stays.
//!
//! The sink applies it when the edit ends (`EditSink::space_written`), before text boxes
//! are synced and revision ids made unique, as docx-agent's `_edit` does.
//!
//! Must not: identify text by pointer (equal elements share one allocation), write but
//! through the sink, or space text the edit did not write.

use crate::names::W;
use crate::text::{atoms, AtomKind};
use crate::work::{Id, Work};
use ooxml_word_model::docx::at_path;
use ooxml_word_model::typography::{find, Action, Cascade, RunContext, RunLanguage, NBSP};
use ooxml_xml::Element;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// What an edit wrote: per paragraph (its part and `w14:paraId`), the places of the
/// written characters in its text.
#[derive(Debug, Default, Clone)]
pub(crate) struct Written {
    places: BTreeMap<(Arc<str>, String), BTreeSet<usize>>,
    /// The paragraphs whose places were put on a work not handed back yet.
    loaded: BTreeSet<(Arc<str>, String)>,
}

impl Written {
    /// Whether nothing is recorded.
    pub(crate) fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    /// Whether anything is recorded in `part`.
    pub(crate) fn in_part(&self, part: &str) -> bool {
        self.places.keys().any(|(p, _)| &**p == part)
    }

    /// The written places of the paragraph `id` of `part`.
    pub(crate) fn of(&self, part: &str, id: &str) -> Option<&BTreeSet<usize>> {
        self.places.get(&(Arc::from(part), id.to_owned()))
    }

    /// The written places of the paragraph `id` of `part` are now `places`.
    pub(crate) fn set(&mut self, part: &str, id: &str, places: BTreeSet<usize>) {
        let key = (Arc::from(part), id.to_owned());
        if places.is_empty() {
            self.places.remove(&key);
        } else {
            self.places.insert(key, places);
        }
    }

    /// The places of the paragraph `id` of `part` were put on a work.
    pub(crate) fn loaded(&mut self, part: &str, id: &str) {
        self.loaded.insert((Arc::from(part), id.to_owned()));
    }

    /// Whether the places of the paragraph `id` of `part` were put on a work, which now
    /// hands them back (forgotten).
    pub(crate) fn take_loaded(&mut self, part: &str, id: &str) -> bool {
        self.loaded.remove(&(Arc::from(part), id.to_owned()))
    }

    /// Every paragraph recorded, by part and id.
    pub(crate) fn paragraphs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.places.keys().map(|(part, id)| (&**part, id.as_str()))
    }
}

/// The places of `paragraph`'s characters (current view) that `work` marks written.
pub(crate) fn marked_places(work: &Work, paragraph: Id) -> BTreeSet<usize> {
    atoms(work, paragraph)
        .iter()
        .enumerate()
        .filter(|(_, a)| a.kind == AtomKind::Text && work.marked(a.node, a.offset))
        .map(|(i, _)| i)
        .collect()
}

/// Mark written in `work` the characters of `paragraph` at `places` (current view).
pub(crate) fn mark_places(work: &mut Work, paragraph: Id, places: &BTreeSet<usize>) {
    let mut by_node: BTreeMap<Id, Vec<usize>> = BTreeMap::new();
    for (i, atom) in atoms(work, paragraph).iter().enumerate() {
        if atom.kind == AtomKind::Text && (places.contains(&i) || work.marked(atom.node, atom.offset)) {
            by_node.entry(atom.node).or_default().push(atom.offset);
        }
    }
    for (node, offsets) in by_node {
        work.mark(node, offsets);
    }
}

/// What the rule reads of the paragraph a [`Work`] holds beyond its own content.
pub(crate) struct Context<'a> {
    /// The styles' cascade.
    pub(crate) cascade: &'a Cascade,
    /// `Some` when the paragraph is in a table: that table's `w:tblPr`.
    pub(crate) table: Option<Option<Arc<Element>>>,
}

/// `Some` with the `w:tblPr` of the innermost table above the element at `path` of `root`
/// when there is one.
pub(crate) fn table_of(root: &Arc<Element>, path: &[u32]) -> Option<Option<Arc<Element>>> {
    let (_, above) = path.split_last()?;
    let mut node = root;
    let mut table = None;
    for k in std::iter::once(None).chain(above.iter().map(Some)) {
        if let Some(k) = k {
            node = node.children().get(*k as usize)?.as_element()?;
        }
        if node.name().is(Some(W), "tbl") {
            table = Some(node.find(Some(W), "tblPr").cloned());
        }
    }
    table
}

/// Give the paragraph `work` holds (its root) the spacing its text's language wants where
/// the edit wrote (`written`, places in its text): the number of spaces written or made
/// U+00A0 (docx-agent's `apply_written` for one paragraph).
pub(crate) fn space_paragraph(work: &mut Work, written: &BTreeSet<usize>, context: &Context<'_>) -> usize {
    let paragraph = work.root();
    let items = atoms(work, paragraph);
    let text: Vec<char> = items.iter().map(|a| a.ch).collect();
    let properties = work.find(paragraph, W, "pPr").and_then(|p| work.build_element(p));
    let mut runs: BTreeMap<Option<Id>, RunLanguage> = BTreeMap::new();
    let mut of = |i: usize| -> RunLanguage {
        let run = items.get(i).and_then(|a| a.run);
        runs.entry(run)
            .or_insert_with(|| {
                let direct = run
                    .and_then(|r| work.find(r, W, "rPr"))
                    .and_then(|r| work.build_element(r));
                context.cascade.of(&RunContext {
                    paragraph_properties: properties.as_deref(),
                    table: context.table.as_ref().map(|t| t.as_deref()),
                    run_properties: direct.as_deref(),
                })
            })
            .clone()
    };
    let languages: Vec<RunLanguage> = (0..items.len()).map(&mut of).collect();
    let spots = find(
        &text,
        &mut |i| languages.get(i).and_then(|l| l.language.clone()),
        &mut |i| items.get(i).is_some_and(|a| a.plain()) && languages.get(i).is_some_and(|l| l.proofed && !l.code),
        false,
    );
    let is_written = |i: usize| items.get(i).is_some_and(|a| a.kind == AtomKind::Text) && written.contains(&i);
    let mut edits: BTreeMap<Id, Vec<(usize, Action)>> = BTreeMap::new();
    for spot in spots {
        if spot.action == Action::Replace {
            if let Some(atom) = items.get(spot.at).filter(|_| is_written(spot.at)) {
                edits.entry(atom.node).or_default().push((atom.offset, Action::Replace));
            }
            continue;
        }
        // between characters at - 1 and at: into the mark's node where the edit wrote it,
        // else into its neighbour's
        let (left, right) = (spot.at.checked_sub(1), spot.at);
        let sides = if spot.mark == right {
            [Some(right), left]
        } else {
            [left, Some(right)]
        };
        if let Some((side, atom)) = sides
            .into_iter()
            .flatten()
            .find_map(|s| items.get(s).filter(|_| is_written(s)).map(|a| (s, a)))
        {
            let offset = atom.offset + usize::from(Some(side) == left);
            edits.entry(atom.node).or_default().push((offset, Action::Insert));
        }
    }
    let mut made = 0;
    for (node, mut changes) in edits {
        // last first; at one place the replacement before the insertion (docx-agent sorts
        // `(offset, "replace" | "insert")` in reverse)
        changes.sort_by_key(|(offset, action)| (*offset, *action == Action::Replace));
        let mut value: Vec<char> = work.text(node).chars().collect();
        for (offset, action) in changes.into_iter().rev() {
            if action == Action::Replace {
                if let Some(c) = value.get_mut(offset) {
                    *c = NBSP;
                }
            } else {
                value.insert(offset.min(value.len()), NBSP);
            }
            made += 1;
        }
        work.set_text(node, &value.into_iter().collect::<String>());
    }
    made
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::text::paragraph_text;
    use crate::text::write::set_text;
    use ooxml_xml::Limits;

    const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn paragraph(language: &str, text: &str) -> Work {
        let xml = format!(
            r#"<w:p xmlns:w="{W_NS}"><w:r><w:rPr><w:lang w:val="{language}"/></w:rPr><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#
        );
        let tree = ooxml_xml::parse(xml.as_bytes(), &Limits::default()).unwrap();
        Work::of(tree.root(), Vec::new())
    }

    fn spaced(language: &str, old: &str, new: &str) -> String {
        let mut work = paragraph(language, old);
        let root = work.root();
        set_text(&mut work, root, new).unwrap();
        let written = marked_places(&work, root);
        let cascade = Cascade::new(&[], None);
        space_paragraph(
            &mut work,
            &written,
            &Context {
                cascade: &cascade,
                table: None,
            },
        );
        paragraph_text(&work, root)
    }

    #[test]
    fn written_french_text_gets_no_break_spaces() {
        assert_eq!(
            spaced("fr-FR", "x", "Bonjour : le monde!"),
            "Bonjour\u{a0}: le monde\u{a0}!"
        );
        assert_eq!(spaced("fr-CA", "x", "«oui» : non!"), "«\u{a0}oui\u{a0}» : non!");
        assert_eq!(spaced("en-US", "x", "Hello : world!"), "Hello : world!");
    }

    #[test]
    fn a_space_there_before_stays() {
        // the kept " :" was not written by this edit: its space stays breakable
        assert_eq!(spaced("fr-FR", "Fin :", "Fin : oui?"), "Fin : oui\u{a0}?");
    }
}
