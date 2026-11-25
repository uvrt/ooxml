//! Owns: blocks moved while tracking changes, as docx-agent's `move_block` moves each one
//! under tracking (`_track_move`), block after block, each later one beside the one before.
//! The block stays where it was and a copy goes where it moves. The copy's paragraphs and rows
//! take fresh paraIds and textIds (the part's move sequence) and its drawings new numbers. It
//! holds none of the original's bookmark, comment, move or permission ranges. Its id is the
//! block's from then on (an alias).
//!
//! A paragraph with no revision, whose original and copy are each followed by a block of
//! their container, takes Word's move form: `w:moveFrom` where it was and `w:moveTo` where
//! it goes (its runs and its mark), each between a named range start (after its properties)
//! and the range's end (after it). Any other block is a deletion where it was and an
//! insertion where it goes, as Word records each. A content control, a block holding a note
//! or comment reference, and one's own insertion move untracked ([`crate::MoveBlocks`]).
//!
//! Must not: write but through the sink, or read past the stretch holding the blocks, their
//! target and the paragraphs their deletions join into.

use crate::block_checks::neighbour_path;
use crate::blocks::{BlockAt, BlockKind};
use crate::delete::{declare, draws_of, editable, join_path, stretch_keys};
use crate::doc::Key;
use crate::marks::{
    clear_revisions, insert_in_order, insert_paragraph_content, mark, mark_record, track_new_paragraph,
};
use crate::moving::{outside_blocks, survives, Outside};
use crate::names::{W, W14};
use crate::policy::{Stamp, Tracking};
use crate::review::xml::make;
use crate::sink::{EditSink, Numbered};
use crate::work::{Id, Work};
use crate::{EditError, MoveBlocks, Result};

/// Range markers a moved block's copy does not take (`_MARKERS`, permissions too).
const MARKERS: &[&str] = &[
    "bookmarkStart",
    "bookmarkEnd",
    "commentRangeStart",
    "commentRangeEnd",
    "moveFromRangeStart",
    "moveFromRangeEnd",
    "moveToRangeStart",
    "moveToRangeEnd",
    "permStart",
    "permEnd",
];

/// A tracked move as planned: the story, the blocks with their ids (as stamped) and
/// whether each moves tracked, and the target.
pub(crate) struct Planned<'a> {
    pub(crate) part: &'a str,
    pub(crate) story: &'a str,
    pub(crate) blocks: &'a [BlockAt],
    pub(crate) ids: &'a [String],
    pub(crate) tracked: &'a [bool],
    pub(crate) anchor: &'a BlockAt,
}

/// What a tracked move did: each block's id where it is now, the copies made and the
/// renames (the stamping's, then each copy's).
#[derive(Default)]
pub(crate) struct Done {
    pub(crate) moved: Vec<String>,
    pub(crate) created: Vec<String>,
    pub(crate) renamed: Vec<(String, String)>,
}

/// The revision records written so far: the ids, whether any is dated, and the
/// relationships the deletions let go.
#[derive(Default)]
struct Records {
    numbered: Option<Numbered>,
    dated: bool,
    released: Vec<String>,
}

impl Records {
    fn add(&mut self, numbered: Numbered, dated: bool) {
        self.numbered = Some(match self.numbered {
            Some(n) => Numbered { to: numbered.to, ..n },
            None => numbered,
        });
        self.dated |= dated;
    }
}

impl MoveBlocks {
    /// The blocks moved as docx-agent moves them under `tracking`, `renamed` the
    /// stamping's renames.
    pub(crate) fn move_tracked(
        &self,
        sink: &mut EditSink<'_, '_>,
        tracking: &Tracking,
        plan: &Planned<'_>,
        renamed: Vec<(String, String)>,
    ) -> Result<Done> {
        let part = plan.part;
        let path_error = || EditError::no_element(part.to_owned());
        let before = crate::revision_ids::document_counts(&*sink.docx()?);
        let tree = sink
            .doc()
            .tree(part)?
            .ok_or_else(|| EditError::no_part(part.to_owned()))?;
        let root = tree.root();
        let (mut work, parent, stretch, nodes) = stretch_of(root, plan).ok_or_else(path_error)?;
        let outside = outside_blocks(root, &parent, stretch);
        let (&target, moving) = nodes.split_last().ok_or_else(path_error)?;
        let mut done = Done {
            renamed,
            ..Done::default()
        };
        let mut records = Records::default();
        let mut previous: Option<Id> = None;
        for ((&node, id), &tracked) in moving.iter().zip(plan.ids).zip(plan.tracked) {
            let (beside, side_after) = previous.map_or((target, self.after.is_some()), |p| (p, true));
            let id = crate::ranges::renamed_id(&done.renamed, id);
            if node == beside {
                return Err(EditError::Refused("a block cannot move beside itself".to_owned()));
            }
            if crate::links::inside(&work, beside, node) {
                return Err(EditError::Refused(format!("{id} holds the block it would move beside")));
            }
            survives(&work, node, &outside)?;
            let already = if side_after {
                work.next(beside) == Some(node)
            } else {
                work.previous(beside) == Some(node)
            };
            if already || !tracked {
                if !already {
                    place(&mut work, beside, side_after, node);
                }
                done.moved.push(id);
                previous = Some(node);
                continue;
            }
            let copy = copy_of(sink, &mut work, node, part)?;
            place(&mut work, beside, side_after, copy);
            let moved = Moved {
                node,
                copy,
                outside: &outside,
            };
            if moved.plain(&work) {
                let (numbered, dated) = moved.move_form(sink, tracking, &mut work)?;
                records.add(numbered, dated);
            } else {
                moved.delete_and_insert(sink, tracking, &mut work, &mut records, plan.story)?;
            }
            let new = copy_id(&work, copy, plan.story).ok_or_else(path_error)?;
            merge(&mut done.renamed, (id, new.clone()));
            done.created.push(new.clone());
            done.moved.push(new);
            previous = Some(copy);
        }
        let built = work.build_content(work.root()).ok_or_else(path_error)?;
        let draws = draws_of(&built, records.numbered);
        let mut keys = stretch_keys(&*sink.docx()?, part, &parent, stretch);
        keys.push(Key::Annotations);
        let count = (stretch.1 - stretch.0) as usize + 1;
        sink.splice_children(part, (&parent, stretch.0 as usize, count), built, keys, draws)?;
        let aliases: Vec<(String, String)> = done
            .renamed
            .iter()
            .filter(|(_, new)| done.created.contains(new))
            .cloned()
            .collect();
        sink.rename(aliases)?;
        declare(sink, part, records.dated)?;
        sink.ensure_w14(part, false, &[])?;
        let gave = records.numbered.is_some_and(|n| n.to > n.from);
        sink.people(gave.then_some(tracking.author.as_str()), Some(tracking.author.as_str()))?;
        sink.release(part, &records.released)?;
        crate::revision_ids::unique_since(sink, &before)?;
        Ok(done)
    }
}

/// The stretch a tracked move rewrites, made editable, its nodes the blocks' and then the
/// target's: the blocks, the target, the paragraph each deletion joins into, and a table's
/// neighbours (its rows' markers rehomed).
fn stretch_of(root: &std::sync::Arc<ooxml_xml::Element>, plan: &Planned<'_>) -> Option<crate::delete::Editable> {
    let mut paths: Vec<Vec<u32>> = plan.blocks.iter().map(|b| b.path.clone()).collect();
    paths.push(plan.anchor.path.clone());
    let wanted: Vec<Vec<u32>> = paths.clone();
    for (block, &tracked) in plan.blocks.iter().zip(plan.tracked) {
        if !tracked {
            continue;
        }
        paths.extend(join_path(root, &block.path));
        if block.kind == BlockKind::Table {
            paths.extend(neighbour_path(root, &block.path, true));
            paths.extend(neighbour_path(root, &block.path, false));
        }
    }
    let wanted: Vec<&[u32]> = wanted.iter().map(Vec::as_slice).collect();
    editable(root, &paths, &wanted)
}

/// `node` put after (or before) `beside`.
fn place(work: &mut Work, beside: Id, after: bool, node: Id) {
    if after {
        work.add_next(beside, node);
    } else {
        work.add_previous(beside, node);
    }
}

/// `renamed` updated as a dict is: a key already there keeps its place.
fn merge(renamed: &mut Vec<(String, String)>, (old, new): (String, String)) {
    match renamed.iter_mut().find(|(o, _)| *o == old) {
        Some(found) => found.1 = new,
        None => renamed.push((old, new)),
    }
}

/// A block's copy (detached): its paragraphs and rows stamped with fresh ids from the
/// part's move sequence, in document order; its range markers dropped; its drawings
/// numbered anew.
fn copy_of(sink: &mut EditSink<'_, '_>, work: &mut Work, node: Id, part: &str) -> Result<Id> {
    let copy = work.deep_copy(node);
    let seed = format!("{part}\0move");
    for target in work.iter(copy, W, &["p", "tr"]) {
        let drawn = sink.draw(&seed, 2)?;
        let (Some(para), Some(text)) = (drawn.first(), drawn.get(1)) else {
            return Err(EditError::no_element(part.to_owned()));
        };
        for (local, value) in [("paraId", para), ("textId", text)] {
            let name = work.qname(W14, local, "w14");
            work.set(target, name, value);
        }
    }
    for marker in work.iter(copy, W, MARKERS) {
        work.remove(marker);
    }
    crate::imported::renumber_drawings(sink, work, copy)?;
    Ok(copy)
}

/// The id a copy goes by: a paragraph's paraId, a table's first row's.
fn copy_id(work: &Work, copy: Id, story: &str) -> Option<String> {
    let prefix = if story == "body" {
        String::new()
    } else {
        format!("{story}/")
    };
    if work.is(copy, W, "p") {
        return Some(format!("{prefix}p:{}", work.get(copy, Some(W14), "paraId")?));
    }
    let row = work.find(copy, W, "tr")?;
    Some(format!("{prefix}t:{}", work.get(row, Some(W14), "paraId")?))
}

/// Whether no block follows `node` in its container (`is_last_in_container`); a node of
/// the stretch's own level counts the blocks after the stretch.
fn last_in_container(work: &Work, node: Id, outside: &Outside) -> bool {
    let mut at = work.next(node);
    while let Some(n) = at {
        if work.is_any(n, W, &["p", "tbl", "sdt", "customXml"]) {
            return false;
        }
        at = work.next(n);
    }
    !(work.parent(node) == Some(work.root()) && outside.after.is_some())
}

/// A block and its copy, placed.
struct Moved<'o> {
    node: Id,
    copy: Id,
    outside: &'o Outside,
}

impl Moved<'_> {
    /// Whether the move takes Word's move form: a paragraph holding no revision, it and its
    /// copy each followed by a block of its container (Word cannot accept or reject a last
    /// mark: measured).
    fn plain(&self, work: &Work) -> bool {
        work.is(self.node, W, "p")
            && work.iter(self.node, W, crate::revision_ids::KINDS).is_empty()
            && !last_in_container(work, self.node, self.outside)
            && !last_in_container(work, self.copy, self.outside)
    }

    /// Word's move form: the move's name (`move` and an id of its own), then for the
    /// block and for its copy a range start after its properties, its runs in
    /// `w:moveFrom` (`w:moveTo`), its mark the same, and the range's end after it.
    fn move_form(&self, sink: &mut EditSink<'_, '_>, tracking: &Tracking, work: &mut Work) -> Result<(Numbered, bool)> {
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        let ((), numbered, dated) = sink.stamped(tracking.clone(), |stamp| {
            let name = format!("move{}", stamp.next_id());
            for (paragraph, kind) in [(self.node, "moveFrom"), (self.copy, "moveTo")] {
                let start = stamp.make_named(work, &format!("{kind}RangeStart"), &name);
                match work.find(paragraph, W, "pPr") {
                    Some(properties) => work.add_next(properties, start),
                    None => work.insert(paragraph, 0, start),
                }
                let runs = crate::text::runs(work, paragraph);
                wrap_runs(work, &runs, kind, stamp);
                mark(work, paragraph, kind, stamp, orders);
                let id = work.get(start, Some(W), "id").unwrap_or_default().to_owned();
                let end = make(work, &format!("{kind}RangeEnd"), &[("id", &id)]);
                work.add_next(paragraph, end);
            }
            Ok(())
        })?;
        Ok((numbered, dated))
    }

    /// The block deleted where it was and its copy inserted where it goes, each in the
    /// form Word records it in.
    fn delete_and_insert(
        &self,
        sink: &mut EditSink<'_, '_>,
        tracking: &Tracking,
        work: &mut Work,
        records: &mut Records,
        story: &str,
    ) -> Result<()> {
        let shared = sink.doc().shared_handle();
        let orders = shared.facts.orders();
        if work.is(self.node, W, "tbl") {
            let ((_, released), numbered, dated) = sink.stamped(tracking.clone(), |stamp| {
                Ok(crate::table_delete::track_delete_table(work, self.node, stamp, orders))
            })?;
            records.add(numbered, dated);
            records.released.extend(released);
        } else {
            self.delete_paragraph(sink, work, records, story)?;
        }
        let table = work.is(self.copy, W, "tbl");
        let paragraphs = work.iter(self.copy, W, &["p"]);
        let ((), numbered, dated) = sink.stamped(tracking.clone(), |stamp| {
            if table {
                for row in work.iter(self.copy, W, &["tr"]) {
                    insert_row(work, row, stamp, orders);
                }
            }
            for &paragraph in &paragraphs {
                clear_revisions(work, paragraph);
            }
            if table {
                for &paragraph in &paragraphs {
                    insert_paragraph_content(work, paragraph, stamp);
                    mark(work, paragraph, "ins", stamp, orders);
                }
            }
            Ok(())
        })?;
        records.add(numbered, dated);
        if !table {
            let last = last_in_container(work, self.copy, self.outside);
            let ((), numbered, dated) = sink.stamped(tracking.clone(), |stamp| {
                track_new_paragraph(work, self.copy, last, stamp, orders);
                Ok(())
            })?;
            records.add(numbered, dated);
        }
        Ok(())
    }

    /// The paragraph's tracked deletion (`_track_delete_block`): last in its container,
    /// the paragraph before it -- past those whose mark is deleted already -- gives its
    /// mark; a mark that was one's own insertion joins that paragraph into this one, and
    /// the ids follow what is left.
    fn delete_paragraph(
        &self,
        sink: &mut EditSink<'_, '_>,
        work: &mut Work,
        records: &mut Records,
        story: &str,
    ) -> Result<()> {
        let mut previous = work.previous(self.node);
        while let Some(p) = previous.filter(|p| {
            !work.is_any(*p, W, &["p", "tbl"])
                || (work.is(*p, W, "p")
                    && mark_record(work, *p).is_some_and(|r| work.is_any(r, W, &["del", "moveFrom"])))
        }) {
            previous = work.previous(p);
        }
        let joins = previous.filter(|p| {
            last_in_container(work, self.node, self.outside)
                && work.is(*p, W, "p")
                && !work
                    .find(*p, W, "pPr")
                    .is_some_and(|properties| work.find(properties, W, "sectPr").is_some())
        });
        let own_runs = work.iter(self.node, W, &["r"]);
        let joined = joins.and_then(|p| work.get(p, Some(W14), "paraId").map(str::to_owned));
        let wrote = sink.delete_paragraph(work, self.node, joins)?;
        if let Some(numbered) = wrote.annotations {
            records.add(numbered, wrote.dated);
        }
        records.released.extend(wrote.released);
        let mine = work.get(self.node, Some(W14), "paraId").map(str::to_owned);
        if let (Some(p), Some(joined), Some(mine)) = (joins, joined, mine) {
            if work.parent(p).is_none() {
                let prefix = if story == "body" {
                    String::new()
                } else {
                    format!("{story}/")
                };
                if own_runs.iter().any(|r| crate::links::inside(work, *r, self.node)) {
                    sink.rename(vec![(format!("{prefix}p:{joined}"), format!("{prefix}p:{mine}"))])?;
                } else {
                    let name = work.qname(W14, "paraId", "w14");
                    work.set(self.node, name, &joined);
                    sink.rename(vec![(format!("{prefix}p:{mine}"), format!("{prefix}p:{joined}"))])?;
                }
            }
        }
        Ok(())
    }
}

/// A copied row recorded as inserted: its own row records dropped, `w:trPr/w:ins` added.
fn insert_row(work: &mut Work, row: Id, stamp: &mut Stamp, orders: &crate::validate::Orders) {
    let properties = match work.find(row, W, "trPr") {
        Some(properties) => {
            for record in work.children(properties).to_vec() {
                if work.is_any(record, W, &["del", "ins"]) {
                    work.remove(record);
                }
            }
            properties
        }
        None => {
            let properties = work.new_element(work.element_name(row, W, "trPr", "w"));
            insert_in_order(work, row, properties, orders);
            properties
        }
    };
    let record = stamp.make(work, "ins");
    insert_in_order(work, properties, record, orders);
}

/// Each stretch of sibling `runs` wrapped in a new revision container `kind` where it
/// stands (`wrap_runs`); moved-away text stays `w:t`, as Word writes it.
fn wrap_runs(work: &mut Work, runs: &[Id], kind: &str, stamp: &mut Stamp) {
    let mut current: Option<Id> = None;
    for &run in runs {
        if work.parent(run).is_none() {
            continue;
        }
        if let Some(container) = current.filter(|c| work.previous(run) == Some(*c)) {
            work.append(container, run);
            continue;
        }
        let container = stamp.make(work, kind);
        work.add_previous(run, container);
        work.append(container, run);
        current = Some(container);
    }
}
