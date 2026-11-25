//! Owns: [`RemoveControl`], a content control taken out as docx-agent's
//! `remove_control(keep_content=True)` takes one out: refused when locked against deletion;
//! its content put where it stood, in the line (its paragraph given a new text id when the
//! control stood right in it) or as blocks, rows or cells; a comment or bookmark mark Word
//! displaced beside it no longer saying so. Word has no revision for a control: tracked,
//! the removal is applied untracked and the result says so.
//!
//! Must not: write but through the sink, or touch more than the control's paragraph, or
//! the control and its neighbours.

use super::{located, rewrite, undisplace, ControlEdited, InParagraph, Rewrite, REMOVED_UNTRACKED};
use crate::delete::{editable, stretch_keys};
use crate::names::W;
use crate::policy::RevisionPolicy;
use crate::sink::{EditSink, TextWrite};
use crate::stamping::stamp_document;
use crate::work::{Id, Work};
use crate::{EditError, Result, WordDoc};
use ooxml_store::{Command, Txn};
use ooxml_word_model::docx::controls::lock;
use ooxml_word_model::docx::{at_path, Docx};

/// A content control removed, its content kept.
#[derive(Debug, Clone)]
pub struct RemoveControl {
    /// The control (`cc:101`).
    pub control: String,
    /// Untracked or tracked.
    pub policy: RevisionPolicy,
}

impl RemoveControl {
    fn check(&self, docx: &Docx) -> Result<()> {
        let found = located(docx, &self.control)?;
        if matches!(lock(&found.element), Some("sdtLocked" | "sdtContentLocked")) {
            return Err(EditError::Refused(format!(
                "{} is locked against deletion",
                self.control
            )));
        }
        Ok(())
    }
}

impl Command<WordDoc> for RemoveControl {
    type Out = ControlEdited;
    type Error = EditError;

    fn validate(&self, doc: &WordDoc) -> Result<()> {
        self.check(&*doc.docx()?)
    }

    fn prepare(&self, tx: &mut Txn<'_, WordDoc>) -> Result<ControlEdited> {
        let mut sink = EditSink::new(tx, self.policy.clone());
        let out = self.edit(&mut sink)?;
        crate::after::after_edit(&mut sink)?;
        Ok(out)
    }
}

impl RemoveControl {
    fn edit(&self, sink: &mut EditSink<'_, '_>) -> Result<ControlEdited> {
        let docx = sink.docx()?;
        self.check(&docx)?;
        let renamed = stamp_document(sink, &docx)?;
        let docx = sink.docx()?;
        let found = located(&docx, &self.control)?;
        let part = found.part.clone();
        if let Some(at) = InParagraph::above(&docx, &part, &found.path) {
            let inside = found.path.get(at.path.len()..).unwrap_or_default().to_vec();
            let how = Rewrite {
                renew: inside.len() == 1,
                ensure_w14: false,
            };
            rewrite(sink, &at, how, |_, work| {
                let sdt = super::at_place(work, &inside).ok_or_else(|| EditError::no_element(at.part.clone()))?;
                unwrap(work, sdt);
                Ok(((), TextWrite::default()))
            })?;
        } else {
            self.unwrap_block(sink, &docx, &part, &found.path)?;
        }
        let warnings = match &self.policy {
            RevisionPolicy::Tracked(_) => vec![REMOVED_UNTRACKED.to_owned()],
            RevisionPolicy::Untracked => Vec::new(),
        };
        Ok(ControlEdited {
            id: None,
            created: Vec::new(),
            removed: vec![self.control.clone()],
            renamed,
            warnings,
        })
    }

    /// A control around blocks, rows or cells taken out: the stretch of its neighbours
    /// rewritten.
    fn unwrap_block(&self, sink: &mut EditSink<'_, '_>, docx: &Docx, part: &str, path: &[u32]) -> Result<()> {
        let path_error = || EditError::no_element(part.to_owned());
        let tree = sink
            .doc()
            .tree(part)?
            .ok_or_else(|| EditError::no_part(part.to_owned()))?;
        let (&k, above) = path.split_last().ok_or_else(path_error)?;
        let count = if above.is_empty() {
            tree.root().children().len()
        } else {
            at_path(tree.root(), above).ok_or_else(path_error)?.children().len()
        };
        let mut paths = vec![path.to_vec()];
        for side in [k.checked_sub(1), k.checked_add(1)].into_iter().flatten() {
            if (side as usize) < count {
                let mut at = above.to_vec();
                at.push(side);
                paths.push(at);
            }
        }
        paths.sort();
        let (mut work, parent, range, found) = editable(tree.root(), &paths, &[path]).ok_or_else(path_error)?;
        let &sdt = found.first().ok_or_else(path_error)?;
        let keys = stretch_keys(docx, part, &parent, range);
        unwrap(&mut work, sdt);
        let nodes = work.build_content(work.root()).ok_or_else(path_error)?;
        let count = (range.1 - range.0) as usize + 1;
        sink.splice_children(part, (&parent, range.0 as usize, count), nodes, keys, Vec::new())
    }
}

/// `sdt`'s content put where it stands, the control gone, marks beside it no longer
/// displaced.
fn unwrap(work: &mut Work, sdt: Id) {
    let children: Vec<Id> = work
        .find(sdt, W, "sdtContent")
        .map(|c| work.children(c).to_vec())
        .unwrap_or_default();
    let mut beside: Vec<Id> = [work.previous(sdt), work.next(sdt)].into_iter().flatten().collect();
    beside.extend(&children);
    for child in children {
        work.add_previous(sdt, child);
    }
    work.remove(sdt);
    undisplace(work, &beside);
}
