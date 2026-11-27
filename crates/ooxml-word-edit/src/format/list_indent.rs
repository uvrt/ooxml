//! Owns: Word's list indent of an item in a one-level list (a definition with
//! `w:multiLevelType="singleLevel"`, Word's built-in List Bullet and List Number among
//! them), the rule stated once ([`ListIndent::deeper`]). Word has no level below, so it
//! keeps the item's style and list and moves the item instead: its number (the first line)
//! goes to the next stop after it, the text follows at the level's hanging, and the item
//! gets a `num` tab stop at the new text position. The stops are the default tab stops
//! (`w:defaultTabStop`, every interval), the item's own text position, and the first-line
//! and text positions of the paragraph right after the new items (measured: Word 16.106 for Mac,
//! `crates/ooxml-oracle/word/observations/like-markdown-word.json`, `word-indents`).
//!
//! Where it stops: a multi-level definition that lacks the level (Word fills that level
//! instead, [`ooxml_word_model::docx::numbering::Lists::fills`]) and an item whose sample has
//! an indent of its own are not measured, and are not moved (the edit says so). Known and
//! not followed: Word's list indent of an existing item where it stands stops at the first
//! line of the paragraph before it, not after it; no tool moves an existing item so.
//!
//! Must not: read the document (the caller gives the positions), or know of Markdown.

use ooxml_word_model::docx::formatting::EffectiveParagraph;

/// Word's tab interval where the settings state none (or none above zero).
pub(crate) fn default_tab(stated: Option<i64>) -> i64 {
    stated
        .filter(|v| *v > 0)
        .unwrap_or(ooxml_text::geometry::APPLICATION_DEFAULT_TAB)
}

/// Twips from points (the formatting cascade's unit).
fn twips(points: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let out = (points * 20.0).round() as i64;
    out
}

/// A list item's position, twips: its number's (the first line's) and its hanging (text
/// position less number position).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListIndent {
    /// Where the number stands (`left` plus `firstLine` less `hanging`).
    pub(crate) number: i64,
    /// From the number to the text.
    pub(crate) hanging: i64,
}

impl ListIndent {
    /// The position of a paragraph formatted `effective`.
    pub(crate) fn of(effective: &EffectiveParagraph) -> ListIndent {
        let left = twips(effective.indent_left);
        let number = left + twips(effective.first_line) - twips(effective.hanging);
        ListIndent {
            number,
            hanging: left - number,
        }
    }

    /// Where the text stands.
    pub(crate) fn left(self) -> i64 {
        self.number + self.hanging
    }

    /// The item `levels` deeper, as Word's list indent moves it that many times: each time
    /// its number to the next stop after it -- the next default tab stop (every `tab`
    /// twips), its own text position, or the first-line or text position of `next`, the
    /// paragraph after the new items, whichever comes first -- the hanging kept.
    pub(crate) fn deeper(self, levels: i64, tab: i64, next: Option<ListIndent>) -> ListIndent {
        let mut at = self;
        let after = next.map_or([None, None], |n| [Some(n.number), Some(n.left())]);
        for _ in 0..levels.max(0) {
            let tabbed = (at.number.div_euclid(tab) + 1) * tab;
            let number = [Some(tabbed), Some(at.left())]
                .into_iter()
                .chain(after)
                .flatten()
                .filter(|stop| *stop > at.number)
                .min()
                .unwrap_or(tabbed);
            at = ListIndent { number, ..at };
        }
        at
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::ListIndent;

    /// Word's built-in List Bullet and List Number (`ind` left 360, hanging 360).
    const LIST_BULLET: ListIndent = ListIndent {
        number: 0,
        hanging: 360,
    };
    /// Word's built-in List Bullet 3 as new_document writes it (left 926, hanging 360).
    const LIST_BULLET_3: ListIndent = ListIndent {
        number: 566,
        hanging: 360,
    };
    /// A paragraph in no list, no indent.
    const PLAIN: ListIndent = ListIndent { number: 0, hanging: 0 };

    /// The text positions of the items one to `levels` levels down.
    fn lefts(start: ListIndent, tab: i64, next: Option<ListIndent>, levels: i64) -> Vec<i64> {
        (1..=levels).map(|n| start.deeper(n, tab, next).left()).collect()
    }

    // Each case is word-indents' (crates/ooxml-oracle/word/observations/like-markdown-word.json):
    // the new items' `ind` left at levels 2 to 5, defaultTabStop 708 unless said.

    #[test]
    fn a_one_level_list_alone_moves_its_items_by_the_default_stops_and_its_text() {
        // alone-bullet, alone-number (After stays. follows)
        assert_eq!(lefts(LIST_BULLET, 708, Some(PLAIN), 4), [720, 1068, 1428, 1776]);
        // alone-bullet3
        assert_eq!(lefts(LIST_BULLET_3, 708, Some(PLAIN), 4), [1068, 1428, 1776, 2136]);
        // nothing after the new items
        assert_eq!(lefts(LIST_BULLET, 708, None, 4), [720, 1068, 1428, 1776]);
    }

    #[test]
    fn the_default_stops_are_the_settings_ones() {
        // tab567-bullet, tab567-bullet3: defaultTabStop 567
        assert_eq!(lefts(LIST_BULLET, 567, Some(PLAIN), 4), [720, 927, 1287, 1494]);
        assert_eq!(lefts(LIST_BULLET_3, 567, Some(PLAIN), 4), [927, 1287, 1494, 1854]);
    }

    #[test]
    fn the_paragraph_after_the_new_items_is_a_stop_at_its_first_line_and_its_text() {
        // next-list: a List Bullet 3 paragraph (first line 566, text 926) right after
        assert_eq!(lefts(LIST_BULLET, 708, Some(LIST_BULLET_3), 4), [720, 926, 1068, 1286]);
        // next-plain: a paragraph in no list, left 926 hanging 360
        let plain = ListIndent {
            number: 566,
            hanging: 360,
        };
        assert_eq!(lefts(LIST_BULLET, 708, Some(plain), 4), [720, 926, 1068, 1286]);
        // next-hanging: List Bullet 3 with left 1000 hanging 100 of its own (first line 900)
        let hanging = ListIndent {
            number: 900,
            hanging: 100,
        };
        assert_eq!(lefts(LIST_BULLET, 708, Some(hanging), 4), [720, 1068, 1260, 1360]);
        // next-gap, before-list: a paragraph in no list right after, the list further away
        assert_eq!(lefts(LIST_BULLET, 708, Some(PLAIN), 4), [720, 1068, 1428, 1776]);
    }

    #[test]
    fn several_lists_in_one_document() {
        // the probe document's one-level cases (word-added): List Bullet before List
        // Paragraph's nine-level list (Echo: first line 360, text 720); List Number before
        // List Bullet 3 (Sierra); List Bullet 3 before a paragraph in no list (After)
        let echo = ListIndent {
            number: 360,
            hanging: 360,
        };
        assert_eq!(lefts(LIST_BULLET, 708, Some(echo), 2), [720, 1068]);
        assert_eq!(lefts(LIST_BULLET, 708, Some(LIST_BULLET_3), 2), [720, 926]);
        assert_eq!(lefts(LIST_BULLET_3, 708, Some(PLAIN), 3), [1068, 1428, 1776]);
    }

    #[test]
    fn no_hanging_moves_by_the_default_stops_alone() {
        assert_eq!(lefts(PLAIN, 708, None, 2), [708, 1416]);
    }
}
