//! Owns: a page's (or column's) foot as its stack fills ([`Foot`], docx2svg's `_scan`,
//! its notes' part): the room a line's new footnotes need under it -- the separator
//! where the foot is empty, every new note but the last whole, of the last the least
//! widow control lets stay ([`note_minimum`]) -- and, once the line is placed, how many
//! of the last note's lines go on the page ([`note_prefix`]); below mode 15 in a column
//! of several, of a line's new notes only the first need keep its least, the others
//! taking what is left in turn; in mode 15 in such a column the notes take no room of the
//! column's (the page's note area does). In mode 15 a paragraph's last line keeps its
//! space after above the notes.
//!
//! Must not: decide where a page ends past an overflow (`keeps` does), or measure a note.

use super::scan::Frame;
use super::{NoteRest, PageContext, PlacedNote, Position};
use crate::heights::LineHeight;
use crate::notes::lines::heights;
use crate::notes::{note_minimum, note_prefix, NoteLines};
use crate::px::Px;

/// A line as the foot sees it: where its pitch starts, its height, the border and space
/// after that go with it, and where the page's text ends.
pub(super) struct Fitted<'h> {
    pub y: Px,
    pub height: &'h LineHeight,
    pub border: Px,
    pub after: Px,
    pub bottom: Px,
}

impl Fitted<'_> {
    /// Where the line's text and border end.
    fn end(&self) -> Px {
        self.y + self.height.fit() + self.border
    }
}

/// How the column's notes stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// At a page's foot.
    Page,
    /// At a column's own foot (below mode 15).
    Column,
    /// In the page's note area (mode 15): no room of the column's.
    Pooled,
}

/// The foot of a page or column as its stack fills.
pub(super) struct Foot {
    kind: Kind,
    mode15: bool,
    separator: Px,
    carried: bool,
    /// The notes placed so far.
    notes: Vec<PlacedNote>,
    /// The room they take, with the separator and the lines carried.
    taken: Px,
    /// Per line placed, how many notes the foot held after it.
    marks: Vec<(Position, usize)>,
}

impl Foot {
    /// The foot of a page (`frame` none) or column of section `section`, the lines of a
    /// note the one before could not hold, `carry`, first.
    pub fn new(cx: &PageContext<'_>, section: usize, frame: Option<&Frame>, carry: NoteRest) -> Foot {
        let (separator, continuation) = cx.separators(section);
        let kind = match frame {
            Some(f) if f.pooled => Kind::Pooled,
            Some(f) if f.multi => Kind::Column,
            _ => Kind::Page,
        };
        let taken = if carry.is_empty() {
            Px::ZERO
        } else {
            continuation + heights(&carry)
        };
        Foot {
            kind,
            mode15: cx.mode15,
            separator,
            carried: !carry.is_empty(),
            notes: Vec::new(),
            taken,
            marks: Vec::new(),
        }
    }

    /// The foot's room before a line's new notes: with the separator where they are its
    /// first, and below a column's or a page's own foot the new notes' that stand whole.
    fn below(&self, new: &[&NoteLines]) -> Px {
        let opens = if !new.is_empty() && self.notes.is_empty() && !self.carried {
            self.separator
        } else {
            Px::ZERO
        };
        match self.kind {
            Kind::Pooled => self.taken,
            Kind::Column => self.taken + opens,
            Kind::Page => {
                let whole = new.iter().take(new.len().saturating_sub(1)).map(|n| heights(n));
                self.taken + opens + whole.fold(Px::ZERO, |a, b| a + b)
            }
        }
    }

    /// Whether the last note placed did not go on the page whole: nothing more may.
    fn partial(&self) -> bool {
        self.notes.last().is_some_and(|n| n.taken < n.lines.len())
    }

    /// The room the foot needs below the line `fitted`, its new notes `new` included.
    pub fn need<'n>(&self, new: impl Iterator<Item = &'n NoteLines>, fitted: &Fitted<'_>) -> Px {
        let new: Vec<&NoteLines> = new.collect();
        let below = self.below(&new);
        let mut need = match (new.first(), new.last(), self.kind) {
            (Some(_), _, Kind::Pooled) | (None, _, _) => below,
            (Some(first), _, Kind::Column) if !self.partial() => below + note_minimum(first),
            (_, Some(last), Kind::Page) if !self.partial() => below + note_minimum(last),
            _ => fitted.bottom - fitted.y + Px::int(1),
        };
        let any = !self.notes.is_empty() || self.carried || !new.is_empty();
        if !fitted.after.is_zero() && self.mode15 && self.kind == Kind::Page && any {
            // In mode 15 a paragraph's last line keeps its space after above the notes.
            need += fitted.after;
        }
        need
    }

    /// The line `fitted` placed: its new notes `new` at the foot, the last (or below mode
    /// 15 in a column, each in turn) with as many lines as then fit.
    pub fn place<'n>(&mut self, new: impl Iterator<Item = &'n NoteLines>, fitted: &Fitted<'_>) {
        let new: Vec<&NoteLines> = new.collect();
        if new.is_empty() {
            return;
        }
        let mut below = self.below(&new);
        match self.kind {
            Kind::Pooled => self.notes.extend(new.iter().map(|n| PlacedNote {
                lines: NoteLines::clone(n),
                taken: n.len(),
            })),
            Kind::Column => {
                let mut room = fitted.bottom - fitted.end() - below;
                let mut stopped = false;
                for note in new {
                    let taken = if stopped { 0 } else { note_prefix(note, room) };
                    stopped = stopped || taken < note.len();
                    let used = heights(note.get(..taken).unwrap_or_default());
                    room -= used;
                    below += used;
                    self.notes.push(PlacedNote {
                        lines: NoteLines::clone(note),
                        taken,
                    });
                }
                self.taken = below;
            }
            Kind::Page => {
                let Some((last, whole)) = new.split_last() else {
                    return;
                };
                for note in whole {
                    self.notes.push(PlacedNote {
                        lines: NoteLines::clone(note),
                        taken: note.len(),
                    });
                }
                let taken = note_prefix(last, fitted.bottom - fitted.end() - below);
                self.notes.push(PlacedNote {
                    lines: NoteLines::clone(last),
                    taken,
                });
                self.taken = below + heights(last.get(..taken).unwrap_or_default());
            }
        }
    }

    /// A line (or a table's row) placed at `position`.
    pub fn mark(&mut self, position: Position) {
        self.marks.push((position, self.notes.len()));
    }

    /// The notes at the foot of the page or column that ends before `end`: those of the
    /// last line it keeps (`_page_notes`).
    pub fn kept(&self, end: &Position) -> Vec<PlacedNote> {
        let count = self
            .marks
            .iter()
            .rev()
            .find(|(position, _)| position < end)
            .map_or(0, |(_, n)| *n);
        self.notes.get(..count).unwrap_or_default().to_vec()
    }
}
