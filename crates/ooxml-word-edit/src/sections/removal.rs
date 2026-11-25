//! Owns: [`RemoveSectionBreak`]'s edit, as docx-agent's `remove_section_break` makes it:
//! the break's paragraph mark deleted -- the paragraph joins the one after it (an empty
//! one simply goes), what the section stated that the next does not handed on, the rest
//! released; tracked, the mark a deletion.
//!
//! Must not: write but through the sink.

use super::breaks::RemoveSectionBreak;
use super::breaks::{hand_on, next_block, next_sibling_path, REFERENCES};
use super::{find_section, SectionEdited};
use crate::delete::{editable, stretch_keys};
use crate::doc::Key;
use crate::names::{W, W14, W16DU};
use crate::policy::RevisionPolicy;
use crate::sink::EditSink;
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::sections::sections;

impl Command<WordDoc> for RemoveSectionBreak {
    type Out = SectionEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        let found = find_section(doc, &*doc.docx()?, &self.section)?;
        if found.section.id == "s:body" {
            return Err(EditError::Refused(
                "the body's last section has no break to remove".to_owned(),
            ));
        }
        Ok(())
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<SectionEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl RemoveSectionBreak {
    /// The edit, through `sink`.
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<SectionEdited> {
        sink.read_everything();
        let docx = sink.docx()?;
        let Planned {
            wanted_id,
            paragraph_path,
            next_path,
            main,
            entry_id,
            following_path,
        } = self.plan(sink, &docx)?;
        let mut paths = vec![paragraph_path.clone(), next_path.clone()];
        paths.extend(following_path.clone());
        let tracked = matches!(self.policy, RevisionPolicy::Tracked(_));
        let renamed = stamp_document(&mut *sink, &docx)?;
        let entry_id = crate::ranges::renamed_id(&renamed, &entry_id);
        let tree = sink
            .doc()
            .tree(&main)?
            .ok_or_else(|| EditError::no_part(main.clone()))?;
        let wanted: Vec<&[u32]> = paths.iter().map(Vec::as_slice).collect();
        let path_error = || EditError::no_element(main.clone());
        let (mut work, parent, stretch, nodes) = editable(tree.root(), &paths, &wanted).ok_or_else(path_error)?;
        let (&paragraph, &next_sect) = (
            nodes.first().ok_or_else(path_error)?,
            nodes.get(1).ok_or_else(path_error)?,
        );
        let following = next_block(&work, paragraph);
        let following_paragraph = following.filter(|f| work.is(*f, W, "p"));
        if tracked && following_paragraph.is_none() {
            return Err(EditError::Refused(format!(
                "{}'s break is not followed by a paragraph: Word cannot join a tracked break's mark into a table",
                self.section
            )));
        }
        let empty = !work
            .children(paragraph)
            .iter()
            .any(|c| work.is_element(*c) && !work.is(*c, W, "pPr"));
        let survives = crate::block_checks::check_container_survives(tree.root(), &paragraph_path).is_ok();
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let sect = work
            .find(paragraph, W, "pPr")
            .and_then(|p| work.find(p, W, "sectPr"))
            .ok_or_else(path_error)?;
        hand_on(&mut work, sect, next_sect, orders);
        let (gone, numbered, dated, released) = match sink.policy().clone() {
            RevisionPolicy::Tracked(tracking) => {
                let (gone, n, d) = sink.stamped(tracking, |stamp| {
                    Ok(crate::marks::delete_mark(&mut work, paragraph, stamp, orders).is_some())
                })?;
                (gone, Some(n), d, Released::Stories)
            }
            RevisionPolicy::Untracked => {
                let goes = empty && survives;
                let (gone, refs) = remove_untracked(&mut work, (paragraph, sect, following_paragraph), goes);
                (gone, None, false, Released::These(refs))
            }
        };
        let removed: Vec<String> = if gone { vec![entry_id] } else { Vec::new() };
        let para_of = |work: &Work, node: Id| work.get(node, Some(W14), "paraId").map(|p| format!("p:{p}"));
        let survivor = following_paragraph.and_then(|f| para_of(&work, f));
        let kept = para_of(&work, paragraph);
        let built = work.build_content(work.root()).ok_or_else(path_error)?;
        let draws = crate::delete::draws_of(&built, numbered);
        let docx = sink.docx()?;
        let mut keys = stretch_keys(&docx, &main, &parent, stretch);
        keys.push(Key::Annotations);
        let count = (stretch.1 - stretch.0) as usize + 1;
        sink.splice_children(&main, (&parent, stretch.0 as usize, count), built, keys, draws)?;
        finish(sink, &main, released, dated)?;
        outcome(sink, (wanted_id, renamed, removed), (survivor, kept))
    }
}

/// What a removal works on: the section's id, its paragraph's place and id, the next
/// section's `w:sectPr`'s place, the main part, and the place of the block after.
struct Planned {
    wanted_id: String,
    paragraph_path: Vec<u32>,
    next_path: Vec<u32>,
    main: String,
    entry_id: String,
    following_path: Option<Vec<u32>>,
}

impl RemoveSectionBreak {
    fn plan(&self, sink: &EditSink<'_, '_>, docx: &ooxml_word_model::docx::Docx) -> Result<Planned> {
        let found = find_section(sink.doc(), docx, &self.section)?;
        if found.section.id == "s:body" {
            return Err(EditError::Refused(
                "the body's last section has no break to remove".to_owned(),
            ));
        }
        let wanted_id = found.section.id.clone();
        let all = sections(docx);
        let next = all
            .get(found.index + 1)
            .ok_or_else(|| EditError::no_element(docx.main.clone()))?;
        let paragraph_path = found
            .paragraph_path()
            .ok_or_else(|| EditError::no_element(docx.main.clone()))?
            .to_vec();
        let main = docx.main.clone();
        let entry_id = docx
            .body()
            .and_then(|b| b.ids.paragraph_at(&paragraph_path))
            .map(|p| p.id)
            .unwrap_or_default();
        let next_path = next.path.to_vec();
        let tree = sink
            .doc()
            .tree(&main)?
            .ok_or_else(|| EditError::no_part(main.clone()))?;
        let following_path = next_sibling_path(tree.root(), &paragraph_path);
        Ok(Planned {
            wanted_id,
            paragraph_path,
            next_path,
            main,
            entry_id,
            following_path,
        })
    }
}

/// What a removal says: the section and the paragraph gone, the paragraph after the break
/// named for the one that joined it (or the break's paragraph, kept).
fn outcome(
    sink: &mut EditSink<'_, '_>,
    (wanted_id, renamed, removed): (String, Vec<(String, String)>, Vec<String>),
    (survivor, kept): (Option<String>, Option<String>),
) -> Result<SectionEdited> {
    let renamed: Vec<(String, String)> = renamed.into_iter().filter(|(old, _)| !removed.contains(old)).collect();
    let mut out = SectionEdited {
        id: None,
        changed: true,
        created: Vec::new(),
        removed: std::iter::once(wanted_id).chain(removed.iter().cloned()).collect(),
        renamed,
        warnings: Vec::new(),
    };
    if let (Some(first), Some(survivor)) = (removed.first(), survivor) {
        // the paragraph after the break took its place
        sink.rename(vec![(first.clone(), survivor.clone())])?;
        out.id = Some(survivor);
    } else if removed.is_empty() {
        out.id = kept;
    }
    Ok(out)
}

/// The break taken out untracked: an empty break paragraph goes (`goes`), else it joins the
/// paragraph after it, else only its section properties go. Whether the paragraph went,
/// and the relationships its references used.
fn remove_untracked(
    work: &mut Work,
    (paragraph, sect, following): (Id, Id, Option<Id>),
    goes: bool,
) -> (bool, Vec<String>) {
    let released = work
        .children(sect)
        .iter()
        .filter(|n| work.is_any(**n, W, &REFERENCES))
        .filter_map(|n| work.get(*n, Some(crate::names::R), "id").map(str::to_owned))
        .collect();
    if goes {
        crate::across::rehome_markers(work, paragraph);
        work.remove(paragraph);
        return (true, released);
    }
    if let Some(into) = following {
        work.remove(sect);
        let mut at = usize::from(work.find(into, W, "pPr").is_some());
        for child in work.children(paragraph).to_vec() {
            if work.is(child, W, "pPr") {
                continue;
            }
            work.insert(into, at, child);
            at += 1;
        }
        work.remove(paragraph);
        return (true, released);
    }
    work.remove(sect);
    if let Some(properties) = work.find(paragraph, W, "pPr") {
        if work.children(properties).is_empty() {
            work.remove(properties);
        }
    }
    (false, released)
}

/// The relationships a removal lets go.
enum Released {
    /// The removed section's references'.
    These(Vec<String>),
    /// Every header and footer relationship no section uses any more (tracked).
    Stories,
}

/// The relationships let go, `w16du` declared, the people part kept.
fn finish(sink: &mut EditSink<'_, '_>, main: &str, released: Released, dated: bool) -> Result<()> {
    let ids = match released {
        Released::These(ids) => ids,
        Released::Stories => sink
            .doc()
            .package()
            .relationships(main)?
            .iter()
            .filter(|r| r.rel_type.ends_with("/header") || r.rel_type.ends_with("/footer"))
            .map(|r| r.id.clone())
            .collect(),
    };
    sink.release(main, &ids)?;
    if dated {
        sink.ensure_w14(main, false, &[("w16du", W16DU)])?;
    }
    if let RevisionPolicy::Tracked(tracking) = sink.policy().clone() {
        sink.people(Some(&tracking.author), Some(&tracking.author))?;
    }
    Ok(())
}
