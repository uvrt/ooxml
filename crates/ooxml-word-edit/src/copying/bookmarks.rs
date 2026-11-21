//! Owns: the bookmarks a copy holds, brought as docx-agent's importer brings them
//! (`_bookmarks_in`, `_free_bookmark`): each bookmark whose name this document has renamed
//! (`Name_1`, ...) with a warning, or dropped where the copy's [`BookmarkPolicy`] says so
//! (a taken `_GoBack` always); every bookmark kept given the next annotation id, its end
//! following it, an end whose start was dropped or not copied dropped; and once any
//! bookmark was renamed, the copy's internal hyperlinks and `REF`/`PAGEREF`/`NOTEREF`
//! fields that name it renamed with it.
//!
//! One [`Bookmarks`] serves the whole copy (its notes first, then its blocks), so the
//! names it takes and the renames it made carry from one piece of content to the next.
//!
//! Must not: write but into the copies the caller holds; the caller declares the read
//! (names are unique in the document, annotation ids one above the largest).

use super::BookmarkPolicy;
use crate::links::rename_in_instruction;
use crate::names::W;
use crate::sink::EditSink;
use crate::work::{Id, Work};
use crate::Result;
use ooxml_base::pyfmt::repr_str;
use ooxml_word_model::address::marks::bookmark_markers;
use std::collections::HashSet;

/// A bookmark name's longest length (Word's 40 characters).
const LONGEST: usize = 40;

/// The bookmarks one copy brings: the names taken and the renames made.
pub(super) struct Bookmarks {
    policy: BookmarkPolicy,
    /// The names this document has and the copy took, read once the copy meets its first
    /// bookmark.
    taken: Option<HashSet<String>>,
    /// Each source name renamed and its new name, in the order first renamed (a later
    /// rename of a name replaces its earlier one).
    renamed: Vec<(String, String)>,
}

impl Bookmarks {
    pub(super) fn new(policy: BookmarkPolicy) -> Self {
        Self {
            policy,
            taken: None,
            renamed: Vec::new(),
        }
    }

    /// The bookmarks of `elements` (copied content in `work`) brought; the warnings (one per
    /// bookmark renamed).
    ///
    /// # Errors
    /// When the document cannot be read or an annotation id given.
    pub(super) fn bring(
        &mut self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        elements: &[Id],
    ) -> Result<Vec<String>> {
        let starts: Vec<Id> = elements
            .iter()
            .flat_map(|e| work.iter(*e, W, &["bookmarkStart"]))
            .collect();
        let ends: Vec<Id> = elements
            .iter()
            .flat_map(|e| work.iter(*e, W, &["bookmarkEnd"]))
            .collect();
        let mut warnings = Vec::new();
        if !starts.is_empty() && self.taken.is_none() {
            let docx = sink.docx()?;
            self.taken = Some(bookmark_markers(&docx).into_iter().map(|b| b.name).collect());
        }
        let mut ids: Vec<(Option<String>, String)> = Vec::new();
        let mut dropped: HashSet<Option<String>> = HashSet::new();
        for start in starts {
            let mut name = work.get(start, Some(W), "name").unwrap_or("").to_owned();
            let old = work.get(start, Some(W), "id").map(str::to_owned);
            let taken = self.taken.get_or_insert_with(HashSet::new);
            if taken.contains(&name) {
                if self.policy == BookmarkPolicy::Drop || name == "_GoBack" {
                    dropped.insert(old);
                    work.remove(start);
                    continue;
                }
                let new = free_name(taken, &name);
                warnings.push(format!(
                    "bookmark {} renamed {}: the destination has one",
                    repr_str(&name),
                    repr_str(&new)
                ));
                let attribute = work.qname(W, "name", "w");
                work.set(start, attribute, &new);
                match self.renamed.iter_mut().find(|(o, _)| *o == name) {
                    Some(found) => found.1.clone_from(&new),
                    None => self.renamed.push((name, new.clone())),
                }
                name = new;
            }
            taken.insert(name);
            let (value, _) = sink.marker_id()?;
            let value = value.to_string();
            match ids.iter_mut().find(|(o, _)| *o == old) {
                Some(found) => found.1.clone_from(&value),
                None => ids.push((old, value.clone())),
            }
            let attribute = work.qname(W, "id", "w");
            work.set(start, attribute, &value);
        }
        for end in ends {
            let old = work.get(end, Some(W), "id").map(str::to_owned);
            let new = ids.iter().find(|(o, _)| *o == old).map(|(_, n)| n.clone());
            match new.filter(|_| !dropped.contains(&old)) {
                Some(new) => {
                    let attribute = work.qname(W, "id", "w");
                    work.set(end, attribute, &new);
                }
                // dropped, or its start was not copied
                None => work.remove(end),
            }
        }
        self.follow_renames(work, elements);
        Ok(warnings)
    }

    /// Every rename made so far applied to the internal hyperlinks and the field
    /// instructions of `elements`.
    fn follow_renames(&self, work: &mut Work, elements: &[Id]) {
        if self.renamed.is_empty() {
            return;
        }
        for &element in elements {
            for link in work.iter(element, W, &["hyperlink"]) {
                let renamed = work
                    .get(link, Some(W), "anchor")
                    .and_then(|a| self.renamed.iter().find(|(o, _)| o == a))
                    .map(|(_, n)| n.clone());
                if let Some(new) = renamed {
                    let attribute = work.qname(W, "anchor", "w");
                    work.set(link, attribute, &new);
                }
            }
            for node in work.iter(element, W, &["instrText"]) {
                let text = work.text(node).to_owned();
                let renamed = self.rename(&text);
                if renamed != text {
                    work.set_text(node, &renamed);
                }
            }
            for node in work.iter(element, W, &["fldSimple"]) {
                let instruction = work.get(node, Some(W), "instr");
                let missing = instruction.is_none();
                let instruction = instruction.unwrap_or("").to_owned();
                let renamed = self.rename(&instruction);
                if missing || renamed != instruction {
                    let attribute = work.qname(W, "instr", "w");
                    work.set(node, attribute, &renamed);
                }
            }
        }
    }

    /// `instruction` with each rename applied, in order.
    fn rename(&self, instruction: &str) -> String {
        self.renamed.iter().fold(instruction.to_owned(), |text, (old, new)| {
            rename_in_instruction(&text, old, new)
        })
    }
}

/// `name` with the first `_n` suffix no bookmark takes, cut to Word's 40 characters
/// (`_free_bookmark`).
fn free_name(taken: &HashSet<String>, name: &str) -> String {
    (1..)
        .map(|number: u64| {
            let suffix = format!("_{number}");
            let keep = LONGEST.saturating_sub(suffix.chars().count());
            let mut candidate: String = name.chars().take(keep).collect();
            candidate.push_str(&suffix);
            candidate
        })
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::free_name;
    use std::collections::HashSet;

    #[test]
    fn a_free_name_takes_the_first_suffix_no_bookmark_has() {
        let taken: HashSet<String> = ["Total", "Total_1"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(free_name(&taken, "Total"), "Total_2");
        let long = "x".repeat(45);
        assert_eq!(free_name(&HashSet::new(), &long), format!("{}_1", "x".repeat(38)));
    }
}
