//! Owns: the model's paragraphs joined in the final view (docx2svg's `_joined`,
//! `_joined_into` and `_following_properties`), as [`crate::view::join_plan`] plans them: a
//! paragraph whose mark is deleted made one with the paragraph after it, or with the first
//! paragraph of the table after it; one with nothing to join takes the properties of the
//! paragraph after it in the story.
//!
//! Must not: read XML.

use crate::model::{Block, ParaProps, Paragraph, Table};
use crate::view::{join_plan, Joined, Joining, View};
use std::sync::Arc;

/// Whether nothing of the paragraph is drawn but its mark.
fn empty(paragraph: &Paragraph) -> bool {
    paragraph
        .runs
        .iter()
        .all(|r| r.text.is_empty() && r.breaks.is_empty() && r.field.is_none())
}

/// `first` (its mark deleted) joined to `following`: both's runs under `following`'s
/// properties and section, `first`'s path and id kept.
pub(crate) fn joined(first: &Paragraph, following: &Paragraph) -> Paragraph {
    let count = first.runs.len();
    let mut joins: Vec<usize> = if following.runs.is_empty() {
        Vec::new()
    } else {
        vec![count]
    };
    joins.extend(following.joins.iter().map(|k| count + k));
    let mut bookmarks = first.bookmarks.to_vec();
    bookmarks.extend(following.bookmarks.iter().map(|b| crate::model::Bookmark {
        run: count + b.run,
        ..b.clone()
    }));
    let mut runs = first.runs.to_vec();
    runs.extend(following.runs.iter().cloned());
    Paragraph {
        runs: Arc::from(runs),
        key: first.key,
        unkeyed: first.unkeyed.clone(),
        para_id: first.para_id.clone(),
        joins: Arc::from(joins),
        bookmarks: Arc::from(bookmarks),
        ..following.clone()
    }
}

/// `table` with `first` joined to the first paragraph of its first cell (a nested table's
/// where the cell begins with one); `None` where that cell holds no paragraph.
pub(crate) fn joined_into(first: &Paragraph, table: &Table) -> Option<Table> {
    let row = table.rows.first()?;
    let cell = row.first()?;
    let head = match cell.first()? {
        Block::Paragraph(p) => {
            let made = joined(first, p);
            Block::Paragraph(Arc::new(Paragraph {
                key: p.key,
                unkeyed: p.unkeyed.clone(),
                para_id: p.para_id.clone(),
                ..made
            }))
        }
        Block::Table(t) => Block::Table(Arc::new(joined_into(first, t)?)),
    };
    let new_cell: Vec<Block> = std::iter::once(head).chain(cell.iter().skip(1).cloned()).collect();
    let new_row: Vec<Arc<[Block]>> = std::iter::once(Arc::from(new_cell))
        .chain(row.iter().skip(1).cloned())
        .collect();
    let rows: Vec<Arc<[Arc<[Block]>]>> = std::iter::once(Arc::from(new_row))
        .chain(table.rows.iter().skip(1).cloned())
        .collect();
    Some(Table {
        rows: Arc::from(rows),
        joined: true,
        ..table.clone()
    })
}

/// The final view of a container's blocks, joined as [`join_plan`] says: each paragraph
/// whose mark is deleted joined to the paragraph after it (a chain into the last), or into
/// the table after it. One that cannot join the table (its first cell holds no paragraph)
/// stays, unless nothing of it is drawn.
pub(crate) fn join(blocks: Vec<Block>) -> Vec<Block> {
    let kinds: Vec<Joining> = blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(p) => Joining::Paragraph {
                removed: p.mark_deleted,
                section: p.section.is_some(),
                shown: false,
            },
            Block::Table(_) => Joining::Table,
        })
        .collect();
    if !kinds
        .iter()
        .any(|k| matches!(k, Joining::Paragraph { removed: true, .. }))
    {
        return blocks;
    }
    let mut slots: Vec<Option<Block>> = blocks.into_iter().map(Some).collect();
    let mut take = |k: usize| slots.get_mut(k).and_then(Option::take);
    let mut out: Vec<Block> = Vec::new();
    for group in join_plan(View::Final, &kinds) {
        let (ks, mut tail) = match group {
            Joined::Paragraph(mut ks) => {
                let Some(Block::Paragraph(last)) = ks.pop().and_then(&mut take) else {
                    continue;
                };
                (ks, vec![Block::Paragraph(last)])
            }
            Joined::Table { at, joins } => {
                let Some(table) = take(at) else { continue };
                (joins, vec![table])
            }
            Joined::Open { .. } | Joined::Close(_) => continue,
        };
        // `tail` holds the blocks after the paragraph being joined, the next one last
        for k in ks.into_iter().rev() {
            let Some(Block::Paragraph(p)) = take(k) else { continue };
            match tail.last_mut() {
                Some(Block::Paragraph(following)) => *following = Arc::new(joined(&p, following)),
                Some(Block::Table(table)) => match joined_into(&p, table) {
                    Some(made) => *table = Arc::new(made),
                    None if empty(&p) => {}
                    None => tail.push(Block::Paragraph(p)),
                },
                None => {}
            }
        }
        out.extend(tail.into_iter().rev());
    }
    out
}

/// `blocks` with each paragraph whose mark is deleted (and that joined nothing) under the
/// properties of the paragraph after it in the story (or none), and the first paragraph's
/// properties for the blocks before these.
pub(crate) fn following_properties(
    blocks: Vec<Block>,
    mut following: Option<Arc<ParaProps>>,
) -> (Vec<Block>, Option<Arc<ParaProps>>) {
    let mut out = Vec::with_capacity(blocks.len());
    for block in blocks.into_iter().rev() {
        let block = match block {
            Block::Paragraph(p) => {
                let p = if p.mark_deleted && p.properties != following {
                    Arc::new(Paragraph {
                        properties: following.clone(),
                        ..(*p).clone()
                    })
                } else {
                    p
                };
                following = p.properties.clone();
                Block::Paragraph(p)
            }
            Block::Table(t) => {
                let (table, next) = table_following(&t, following);
                following = next;
                Block::Table(table)
            }
        };
        out.push(block);
    }
    out.reverse();
    (out, following)
}

/// What [`following_properties`] would do to `blocks`, without making anything: whether it
/// changes a paragraph, and the properties it hands the blocks before them.
fn following_of(blocks: &[Block], mut following: Option<Arc<ParaProps>>) -> (bool, Option<Arc<ParaProps>>) {
    let mut changed = false;
    for block in blocks.iter().rev() {
        match block {
            Block::Paragraph(p) => {
                if p.mark_deleted && p.properties != following {
                    changed = true;
                } else {
                    following.clone_from(&p.properties);
                }
            }
            Block::Table(t) => {
                for cell in t.rows.iter().rev().flat_map(|row| row.iter().rev()) {
                    let (inner, next) = following_of(cell, following);
                    changed |= inner;
                    following = next;
                }
            }
        }
    }
    (changed, following)
}

fn table_following(table: &Arc<Table>, following: Option<Arc<ParaProps>>) -> (Arc<Table>, Option<Arc<ParaProps>>) {
    // Most tables hold no paragraph whose mark is deleted: they are taken as they are,
    // nothing copied.
    let (changes, next) = following_of(
        std::slice::from_ref(&Block::Table(Arc::clone(table))),
        following.clone(),
    );
    if !changes {
        return (Arc::clone(table), next);
    }
    let mut following = following;
    let mut changed = false;
    let mut rows = Vec::with_capacity(table.rows.len());
    for row in table.rows.iter().rev() {
        let mut cells = Vec::with_capacity(row.len());
        for cell in row.iter().rev() {
            let (blocks, next) = following_properties(cell.to_vec(), following);
            following = next;
            changed |= blocks.iter().zip(cell.iter()).any(|(a, b)| !same_block(a, b));
            cells.push(Arc::from(blocks));
        }
        cells.reverse();
        rows.push(Arc::from(cells));
    }
    if !changed {
        return (Arc::clone(table), following);
    }
    rows.reverse();
    (
        Arc::new(Table {
            rows: Arc::from(rows),
            ..(**table).clone()
        }),
        following,
    )
}

fn same_block(a: &Block, b: &Block) -> bool {
    match (a, b) {
        (Block::Paragraph(x), Block::Paragraph(y)) => Arc::ptr_eq(x, y),
        (Block::Table(x), Block::Table(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}
