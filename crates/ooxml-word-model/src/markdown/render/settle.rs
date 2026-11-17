//! Owns: making a block's inline items something Markdown reads back as meant
//! (docx-agent's `render._settle_inline`): whitespace moved out of marks, then
//! strikethrough and then every emphasis dropped from the block where its delimiters would
//! not read back -- each opening delimiter left-flanking and each closing one
//! right-flanking, as CommonMark (and GFM's strikethrough) require.
//!
//! Must not: change text.

use super::inline::{spaced, written};
use crate::markdown::model::{Inline, Mark};

/// `inline` settled.
pub(super) fn settle(inline: &[Inline], heading: bool, cell: bool) -> Vec<Inline> {
    let mut items = spaced(inline);
    for dropped in [
        &[][..],
        &[Mark::Strike][..],
        &[Mark::Strike, Mark::Strong, Mark::Em][..],
    ] {
        if !dropped.is_empty() {
            for item in &mut items {
                if let Inline::Text { marks, .. } = item {
                    marks.retain(|m| !dropped.contains(m));
                }
            }
        }
        if reads_back(&items, heading, cell) {
            break;
        }
    }
    items
}

fn reads_back(items: &[Inline], heading: bool, cell: bool) -> bool {
    let marked = items
        .iter()
        .any(|i| matches!(i, Inline::Text { marks, .. } if marks.iter().any(|m| *m != Mark::Code)));
    if !marked {
        return true;
    }
    let (text, delimiters) = written(items, !(heading || cell), cell);
    delimiters.iter().all(|(at, len, opens)| {
        let before = text.get(..*at).and_then(|t| t.chars().next_back());
        let after = text.get(at + len..).and_then(|t| t.chars().next());
        if *opens {
            left_flanking(before, after)
        } else {
            right_flanking(before, after)
        }
    })
}

fn space(c: Option<char>) -> bool {
    c.is_none_or(char::is_whitespace)
}

fn punctuation(c: Option<char>) -> bool {
    c.is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace())
}

fn left_flanking(before: Option<char>, after: Option<char>) -> bool {
    !space(after) && (!punctuation(after) || space(before) || punctuation(before))
}

fn right_flanking(before: Option<char>, after: Option<char>) -> bool {
    !space(before) && (!punctuation(before) || space(after) || punctuation(after))
}
