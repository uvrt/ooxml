//! Owns: the blocks a review step removed, named as docx-agent names them (round 56, R121:
//! `_blocks_now`, `_blocks_gone`): every table, row, paragraph and content control as the
//! caller knew them before the step, and afterwards the typed list (each removed block
//! once, at its outermost: tables with their rows and paragraphs counted, rows of a table
//! that stays, paragraphs -- those merged into the next by its id -- and controls), the
//! flat list of every id removed, and the tables a removed first row renamed.
//!
//! Must not: change the document.

use super::space::Space;
use crate::work::{Id, Work};
use ooxml_word_model::docx::controls::controls;
use ooxml_word_model::docx::Docx;
use std::collections::HashMap;

/// What a known block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BlockKind {
    Table = 0,
    Row = 1,
    Paragraph = 2,
    Control = 3,
}

/// A block as the caller knew it.
#[derive(Debug, Clone)]
pub(crate) struct Known {
    part: String,
    kind: BlockKind,
    id: String,
    path: Box<[u32]>,
    /// A row's table.
    table: Option<Box<[u32]>>,
}

/// Every block of every story before the step (`_blocks_now`).
pub(crate) fn blocks_now(docx: &Docx) -> Vec<Known> {
    let mut out = Vec::new();
    for story in &docx.parts {
        for table in story.ids.tables().iter() {
            out.push(Known {
                part: story.part.clone(),
                kind: BlockKind::Table,
                id: table.id.clone(),
                path: table.path.clone(),
                table: None,
            });
            for (path, id) in &table.rows {
                out.push(Known {
                    part: story.part.clone(),
                    kind: BlockKind::Row,
                    id: id.clone(),
                    path: path.clone(),
                    table: Some(table.path.clone()),
                });
            }
        }
        for paragraph in story.ids.paragraphs().iter() {
            out.push(Known {
                part: story.part.clone(),
                kind: BlockKind::Paragraph,
                id: paragraph.id.clone(),
                path: paragraph.path.clone(),
                table: None,
            });
        }
    }
    for control in controls(docx) {
        out.push(Known {
            part: control.part,
            kind: BlockKind::Control,
            id: control.id,
            path: control.path,
            table: None,
        });
    }
    out
}

/// A table removed, with its rows and paragraphs counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableGone {
    /// Its id.
    pub id: String,
    /// Its rows.
    pub rows: u64,
    /// Its paragraphs.
    pub paragraphs: u64,
}

/// A row removed from a table that stays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowGone {
    /// Its id.
    pub id: String,
    /// Its paragraphs.
    pub paragraphs: u64,
    /// The table's id now.
    pub table: Option<String>,
}

/// The blocks removed, each once at its outermost (docx-agent's `removed_blocks`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Typed {
    /// Tables.
    pub tables: Vec<TableGone>,
    /// Rows of tables that stay.
    pub rows: Vec<RowGone>,
    /// Paragraphs outside those.
    pub paragraphs: Vec<String>,
    /// Paragraphs whose content went into the next one: their id, the next one's id now.
    pub merged: Vec<(String, String)>,
    /// Content controls.
    pub controls: Vec<String>,
}

/// What a step removed, in docx-agent's three forms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemovedBlocks {
    /// Each removed block once, at its outermost.
    pub typed: Typed,
    /// Every id removed, tables first, then rows, paragraphs and controls.
    pub flat: Vec<String>,
    /// Tables a removed first row renamed: old id, new id.
    pub tables_renamed: Vec<(String, String)>,
}

/// A known block's node in the step's copies, and whether it is still in its tree.
fn state(space: &Space, known: &Known) -> (Option<Id>, bool) {
    match space.get(&known.part) {
        None => (None, true),
        Some(work) => {
            let node = space.locate(&known.part, &known.path);
            let alive = node.is_some_and(|n| super::xml::attached(work, n));
            (node, alive)
        }
    }
}

/// The step's copies and the version after it, to tell what is alive and name it.
struct Naming<'a> {
    space: &'a Space,
    now: &'a dyn Fn(&str, &[u32]) -> Option<String>,
    /// The paragraphs that went into another, by part: the one gone to the one it joined.
    into: HashMap<String, HashMap<Id, Id>>,
}

impl Naming<'_> {
    fn alive(&self, part: &str, node: Id) -> bool {
        self.space.get(part).is_some_and(|w| super::xml::attached(w, node))
    }

    fn now_id(&self, part: &str, node: Id) -> Option<String> {
        self.space.path_of(part, node).and_then(|p| (self.now)(part, &p))
    }

    fn table_of(&self, k: &Known) -> Option<Id> {
        k.table.as_deref().and_then(|t| self.space.locate(&k.part, t))
    }

    /// The paragraph a removed one's content went into, as it is named now.
    fn survivor(&self, part: &str, node: Id) -> Option<String> {
        let into = self.into.get(part)?;
        let mut survivor = into.get(&node).copied();
        let mut steps = 0;
        while let Some(s) = survivor.filter(|s| !self.alive(part, *s)) {
            survivor = into.get(&s).copied();
            steps += 1;
            if steps > into.len() {
                return None;
            }
        }
        survivor.and_then(|s| self.now_id(part, s))
    }

    /// The tables still there that lost their first row's name.
    fn tables_renamed(&self, known: &[Known], states: &[(Option<Id>, bool)]) -> Vec<(String, String)> {
        let shortened: Vec<(String, Id)> = known
            .iter()
            .zip(states)
            .filter(|(k, (_, alive))| k.kind == BlockKind::Row && !alive)
            .filter_map(|(k, _)| Some((k.part.clone(), self.table_of(k)?)))
            .collect();
        let mut out = Vec::new();
        for (k, (node, alive)) in known.iter().zip(states) {
            let Some(node) = node.filter(|_| *alive && k.kind == BlockKind::Table) else {
                continue;
            };
            if shortened.iter().any(|(p, t)| *p == k.part && *t == node) {
                if let Some(new) = self.now_id(&k.part, node).filter(|n| *n != k.id) {
                    out.push((k.id.clone(), new));
                }
            }
        }
        out
    }

    /// Each removed block once, at its outermost.
    fn typed(&self, gone: &[(&Known, Id)]) -> Typed {
        let holders: HashMap<(String, Id), &Known> = gone
            .iter()
            .filter(|(k, _)| matches!(k.kind, BlockKind::Table | BlockKind::Row))
            .map(|(k, n)| ((k.part.clone(), *n), *k))
            .collect();
        let mut tables: Vec<((String, Id), TableGone)> = Vec::new();
        let mut typed = Typed::default();
        for &(k, node) in gone {
            let Some(work) = self.space.get(&k.part) else { continue };
            let owner = self.table_of(k);
            let table_of = |h: &Known| self.table_of(h);
            if let Some((holder, holder_node)) = outer(work, &holders, &k.part, node, owner, &table_of) {
                if holder.kind == BlockKind::Table {
                    let key = (k.part.clone(), holder_node);
                    if let Some((_, counts)) = tables.iter_mut().find(|(t, _)| *t == key) {
                        match k.kind {
                            BlockKind::Row => counts.rows += 1,
                            BlockKind::Paragraph => counts.paragraphs += 1,
                            _ => {}
                        }
                    }
                } else if k.kind == BlockKind::Paragraph {
                    for entry in typed.rows.iter_mut().filter(|e| e.id == holder.id) {
                        entry.paragraphs += 1;
                    }
                }
                continue;
            }
            match k.kind {
                BlockKind::Table => tables.push((
                    (k.part.clone(), node),
                    TableGone {
                        id: k.id.clone(),
                        rows: 0,
                        paragraphs: 0,
                    },
                )),
                BlockKind::Row => typed.rows.push(RowGone {
                    id: k.id.clone(),
                    paragraphs: 0,
                    table: owner
                        .filter(|o| self.alive(&k.part, *o))
                        .and_then(|o| self.now_id(&k.part, o)),
                }),
                BlockKind::Paragraph => match self.survivor(&k.part, node) {
                    Some(id) => match typed.merged.iter_mut().find(|(g, _)| *g == k.id) {
                        Some(slot) => slot.1 = id,
                        None => typed.merged.push((k.id.clone(), id)),
                    },
                    None => typed.paragraphs.push(k.id.clone()),
                },
                BlockKind::Control => typed.controls.push(k.id.clone()),
            }
        }
        typed.tables = tables.into_iter().map(|(_, t)| t).collect();
        typed
    }
}

/// What the step removed of `known` (`_blocks_gone`); `joined` (the paragraphs that went
/// into another, by part) and `now`, a block's id after the step by its part and path.
pub(crate) fn blocks_gone(
    space: &Space,
    known: &[Known],
    joined: &HashMap<String, Vec<(Id, Id)>>,
    now: &dyn Fn(&str, &[u32]) -> Option<String>,
) -> RemovedBlocks {
    let naming = Naming {
        space,
        now,
        into: joined
            .iter()
            .map(|(part, pairs)| (part.clone(), pairs.iter().copied().collect()))
            .collect(),
    };
    let states: Vec<(Option<Id>, bool)> = known.iter().map(|k| state(space, k)).collect();
    let mut out = RemovedBlocks {
        tables_renamed: naming.tables_renamed(known, &states),
        ..RemovedBlocks::default()
    };
    let gone: Vec<(&Known, Id)> = known
        .iter()
        .zip(&states)
        .filter_map(|(k, (node, alive))| (!alive).then_some((k, (*node)?)))
        .collect();
    if gone.is_empty() {
        return out;
    }
    out.typed = naming.typed(&gone);
    let mut ordered: Vec<&(&Known, Id)> = gone.iter().collect();
    ordered.sort_by_key(|(k, _)| k.kind);
    for (k, _) in ordered {
        if !out.flat.contains(&k.id) {
            out.flat.push(k.id.clone());
        }
    }
    out
}

/// The outermost removed table or row holding `node` (a row: its table, as it was);
/// `table_of`, a removed row's table node.
fn outer<'k>(
    work: &Work,
    holders: &HashMap<(String, Id), &'k Known>,
    part: &str,
    node: Id,
    owner: Option<Id>,
    table_of: &dyn Fn(&Known) -> Option<Id>,
) -> Option<(&'k Known, Id)> {
    let mut chain: Vec<Id> = Vec::new();
    let mut at = work.parent(node);
    while let Some(p) = at {
        chain.push(p);
        at = work.parent(p);
    }
    chain.extend(owner);
    let mut found = None;
    let mut k = 0;
    while let Some(&ancestor) = chain.get(k) {
        if let Some(holder) = holders.get(&(part.to_owned(), ancestor)) {
            found = Some((*holder, ancestor));
            if holder.kind == BlockKind::Row {
                chain.extend(table_of(holder));
            }
        }
        k += 1;
    }
    found
}
