//! Owns: where two versions' top-level blocks differ ([`aligned`]): each run of blocks
//! changed, added or removed, the blocks between them the same -- the same `Arc`, or a
//! paragraph that lays out as it did (the model moved it to another place, or only its
//! bookmarks changed: [`Paragraph::lays_out_as`]). Blocks added near the start move every
//! later paragraph's path; they are still the same blocks, so only the runs around the
//! blocks added are made again.
//!
//! Must not: call two blocks the same that lay out otherwise wherever they stand: what
//! reads a block's neighbours (spacing, borders, keeps, list labels, places in the endnotes)
//! is the window's widening's ([`super::again`]).

use ooxml_word_model::model::Block;
#[cfg(doc)]
use ooxml_word_model::model::Paragraph;
use std::collections::HashMap;
use std::sync::Arc;

/// Blocks `[a - shift, b_old)` of the old body are `[a, b_new)` of the new one, `shift`
/// being what the windows before it added: `a` and `b_new` index the new body, `b_old` the
/// old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Window {
    pub(super) a: usize,
    pub(super) b_old: usize,
    pub(super) b_new: usize,
}

/// Whether `old` is `new`, perhaps at another place.
fn same(old: &Block, new: &Block) -> bool {
    match (old, new) {
        (Block::Paragraph(p), Block::Paragraph(q)) => Arc::ptr_eq(p, q) || p.lays_out_as(q),
        (Block::Table(t), Block::Table(u)) => Arc::ptr_eq(t, u),
        _ => false,
    }
}

/// What a block is found by among the other version's: its runs' `Arc` (a moved paragraph
/// shares them), a table's own.
fn key(block: &Block) -> usize {
    match block {
        Block::Paragraph(p) => Arc::as_ptr(&p.runs).cast::<()>() as usize,
        Block::Table(t) => Arc::as_ptr(t).cast::<()>() as usize,
    }
}

/// Each key's places among `blocks[from..]`, in order.
fn places(blocks: &[Block], from: usize) -> HashMap<usize, Vec<usize>> {
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, block) in blocks.iter().enumerate().skip(from) {
        out.entry(key(block)).or_default().push(k);
    }
    out
}

/// The first place at or after `from`, below `end`, of a block of `blocks` that is
/// `block` (by `is`).
fn next_of(
    found: &HashMap<usize, Vec<usize>>,
    blocks: &[Block],
    block: &Block,
    (from, end): (usize, usize),
    is: impl Fn(&Block) -> bool,
) -> Option<usize> {
    let list = found.get(&key(block))?;
    let start = list.partition_point(|&k| k < from);
    list.get(start..)?
        .iter()
        .take_while(|&&k| k < end)
        .copied()
        .find(|&k| blocks.get(k).is_some_and(&is))
}

/// The runs of blocks where `new` differs from `old`, in order. Between them the blocks
/// are the same, in the same order. A greedy alignment: where two blocks differ, the
/// nearer of the old block found again later in the new body (blocks added before it) and
/// the new block found later in the old (blocks removed) is taken; neither found, the two
/// are one changed block.
pub(super) fn aligned(old: &[Block], new: &[Block]) -> Vec<Window> {
    let prefix = old.iter().zip(new).take_while(|(x, y)| same(x, y)).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(x, y)| same(x, y))
        .count();
    let (old_end, new_end) = (old.len() - suffix, new.len() - suffix);
    if prefix == old_end && prefix == new_end {
        return Vec::new();
    }
    let in_new = places(new.get(..new_end).unwrap_or_default(), prefix);
    let in_old = places(old.get(..old_end).unwrap_or_default(), prefix);
    let mut out = Vec::new();
    let (mut i, mut j) = (prefix, prefix);
    let mut open: Option<usize> = None;
    while i < old_end && j < new_end {
        let (Some(x), Some(y)) = (old.get(i), new.get(j)) else {
            break;
        };
        if same(x, y) {
            if let Some(a) = open.take() {
                out.push(Window { a, b_old: i, b_new: j });
            }
            i += 1;
            j += 1;
            continue;
        }
        open.get_or_insert(j);
        let later_new = next_of(&in_new, new, x, (j + 1, new_end), |b| same(x, b));
        let later_old = next_of(&in_old, old, y, (i + 1, old_end), |b| same(b, y));
        match (later_new, later_old) {
            (Some(jn), Some(io)) if jn - j <= io - i => j = jn,
            (_, Some(io)) => i = io,
            (Some(jn), None) => j = jn,
            (None, None) => {
                i += 1;
                j += 1;
            }
        }
    }
    let a = open.unwrap_or(j);
    if a < new_end || i < old_end {
        out.push(Window {
            a,
            b_old: old_end,
            b_new: new_end,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_word_model::model::{Paragraph, Run};
    use ooxml_word_model::value::text;

    fn paragraph(words: &str, path: &str) -> Block {
        let runs: Arc<[Run]> = Arc::from(vec![Run {
            text: text(words),
            ..Run::default()
        }]);
        Block::Paragraph(Arc::new(Paragraph {
            runs,
            unkeyed: Some(text(path)),
            ..Paragraph::default()
        }))
    }

    /// The block named `path`, sharing its runs (a paragraph read again, or named otherwise).
    fn moved(block: &Block, path: &str) -> Block {
        match block {
            Block::Paragraph(p) => Block::Paragraph(Arc::new(Paragraph {
                unkeyed: Some(text(path)),
                ..(**p).clone()
            })),
            Block::Table(_) => block.clone(),
        }
    }

    #[test]
    fn blocks_added_at_the_start_move_the_rest_unchanged() {
        let old: Vec<Block> = (0..6)
            .map(|k| paragraph(&format!("p{k}"), &format!("p[{k}]")))
            .collect();
        let mut new: Vec<Block> = vec![paragraph("new", "p[0]"), paragraph("new 2", "p[1]")];
        new.extend(old.iter().enumerate().map(|(k, b)| moved(b, &format!("p[{}]", k + 2))));
        assert_eq!(
            aligned(&old, &new),
            vec![Window {
                a: 0,
                b_old: 0,
                b_new: 2
            }]
        );
    }

    #[test]
    fn an_addition_and_a_change_further_on_are_two_runs() {
        let old: Vec<Block> = (0..8)
            .map(|k| paragraph(&format!("p{k}"), &format!("p[{k}]")))
            .collect();
        let mut new: Vec<Block> = old.get(..2).unwrap().to_vec();
        new.push(paragraph("added", "p[2]"));
        new.extend(old.get(2..6).unwrap().iter().map(|b| moved(b, "elsewhere")));
        new.push(paragraph("changed", "p[7]"));
        new.push(moved(old.get(7).unwrap(), "p[8]"));
        assert_eq!(
            aligned(&old, &new),
            vec![
                Window {
                    a: 2,
                    b_old: 2,
                    b_new: 3
                },
                Window {
                    a: 7,
                    b_old: 7,
                    b_new: 8
                }
            ]
        );
    }

    #[test]
    fn two_alike_paragraphs_stay_apart() {
        // Two paragraphs of the same content share their runs (the model keeps them by
        // content).
        let same = paragraph("alike", "p[0]");
        let old = vec![same.clone(), paragraph("between", "p[1]"), moved(&same, "p[2]")];
        let new = vec![
            moved(&same, "p[0]"),
            paragraph("between", "p[1]"),
            paragraph("added", "p[2]"),
            moved(&same, "p[3]"),
        ];
        // Each alike paragraph is matched with its own, in order ("between", read again, is
        // the same by content): only the paragraph added differs.
        assert_eq!(
            aligned(&old, &new),
            vec![Window {
                a: 2,
                b_old: 2,
                b_new: 3
            }]
        );
    }
}
