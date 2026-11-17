//! Owns: where a table goes beside the drawings text wraps around on its page, and in a
//! column right of its section's first (docx2svg's `paginate.table_beside`, `TableBeside`,
//! `table_fits`, `place_item`; measured by `make_wrap_table_probe.py`): the rows that go
//! beside them, the room they are laid out in, and how far down a table too wide for the
//! room moves ([`Beside`]); one page's part of a table placed so ([`Placed`]).
//!
//! Must not: lay a table out (a [`crate::table::TableRooms`] lays it out in a room), or
//! decide where a page ends.

use super::floats::wrap_column;
use super::PageContext;
use crate::flow::TableItem;
use crate::px::Px;
use crate::table::{TablePage, TableRequest, TableRoom, Unsupported};
use crate::wrap::{free_spans, Wrap, MIN_SEGMENT_TWIPS};
use ooxml_base::Q;

/// Where a table goes beside the drawings on its page (`TableBeside`): from row `row` on,
/// laid out in `room` (a narrower column), or where it fits in none moved down to `drop`,
/// the drawings' foot.
#[derive(Debug, Clone)]
pub(super) struct Beside {
    pub row: usize,
    pub room: Option<TableRoom>,
    pub drop: Option<Px>,
}

/// One page's part of a table, and the room its rows from `from` on were laid out in.
pub(super) struct Placed {
    pub page: TablePage,
    pub room: Option<TableRoom>,
    pub from: usize,
}

/// Device px per twip, exactly.
fn twip_px() -> Q {
    Q::new(300, 1440)
}

/// A table placed whole from `y` on a page that never ends, from `row`.
fn far(y: Px, row: usize, first_on_page: bool, mode15: bool) -> TableRequest {
    TableRequest {
        y,
        bottom: y + Px::int(10_000_000),
        row,
        part: Vec::new(),
        first_on_page,
        mode15,
        split_kept: false,
        stop: None,
    }
}

impl PageContext<'_> {
    /// The table laid out in a column `left` twips right of its own and `width` wide.
    pub(super) fn room(&self, index: usize, left: i64, width: i64) -> Option<TableRoom> {
        self.rooms?.room(index, left, width)
    }

    /// Where a table that starts at `y` goes beside `wraps` (`table_beside`), or `None`
    /// where it is beside none: in mode 15 the whole table when any row is beside one,
    /// below mode 15 the rows from the first that is; in the first room it fits in (below
    /// mode 15 the first room only); else moved down to the drawings' foot and tried again.
    pub(super) fn table_beside(
        &self,
        t: &TableItem,
        index: usize,
        y: Px,
        wraps: &[Wrap],
        first_on_page: bool,
    ) -> Result<Option<Beside>, Unsupported> {
        let section = self
            .sections
            .get(t.section)
            .ok_or_else(|| Unsupported("no section".into()))?;
        let column = wrap_column(section, self.mode15, None);
        let laid = t.flow.place(&far(y, 0, first_on_page, self.mode15))?;
        let pieces: Vec<_> = laid.rows.iter().filter(|r| !r.header).collect();
        if pieces.is_empty() {
            return Ok(None);
        }
        let near = |y0: Px, y1: Px| !free_spans(wraps, (y0.value(), y1.value(), None), &column).1.is_empty();
        let start = if self.mode15 {
            if !near(y, laid.end) {
                return Ok(None);
            }
            0
        } else {
            match pieces.iter().position(|p| near(p.top, p.top + p.top_border + p.height)) {
                Some(k) => k,
                None => return Ok(None),
            }
        };
        let Some(first) = pieces.get(start) else {
            return Ok(None);
        };
        let (row, mut top) = (first.row, first.top);
        let mut drop = None;
        for _attempt in 0..64 {
            let rest = t.flow.place(&far(top, row, first_on_page && start == 0, self.mode15))?;
            let (spans, beside) = free_spans(wraps, (top.value(), rest.end.value(), None), &column);
            if beside.is_empty() {
                break;
            }
            let rooms: Vec<(Q, Q)> = spans
                .iter()
                .map(|(a, b, _)| (*a, *b))
                .filter(|(a, b)| (*b - *a) / twip_px() >= Q::from_integer(i128::from(MIN_SEGMENT_TWIPS)))
                .collect();
            let tried = if self.mode15 { rooms.len() } else { rooms.len().min(1) };
            for (a, b) in rooms.into_iter().take(tried) {
                let (left, width) = (a / twip_px(), (b - a) / twip_px());
                if !left.is_integer() || !width.is_integer() {
                    continue;
                }
                let (Ok(l), Ok(w)) = (i64::try_from(left.to_integer()), i64::try_from(width.to_integer())) else {
                    continue;
                };
                if let Some(room) = self.room(index, l, w).filter(|r| r.flow.fits(width)) {
                    return Ok(Some(Beside {
                        row,
                        room: Some(room),
                        drop,
                    }));
                }
            }
            let Some(below) = beside.iter().map(|w| w.bottom).min() else {
                break;
            };
            let below = Px::from(below);
            if below <= top {
                break;
            }
            top = below;
            drop = Some(below);
        }
        Ok(drop.map(|drop| Beside {
            row,
            room: None,
            drop: Some(drop),
        }))
    }

    /// One page's part of a table, beside the drawings where `beside` puts it there
    /// (`place_item`): the rows before `beside.row` in the column, the rest in its room
    /// from where they are moved down to.
    pub(super) fn place_beside(
        &self,
        t: &TableItem,
        beside: Option<&Beside>,
        request: &TableRequest,
    ) -> Result<Placed, Unsupported> {
        let Some(beside) = beside else {
            return Ok(Placed {
                page: t.flow.place(request)?,
                room: None,
                from: 0,
            });
        };
        let mut request = request.clone();
        let mut rows = Vec::new();
        if request.row < beside.row {
            let head = t.flow.place(&TableRequest {
                stop: Some(beside.row),
                ..request.clone()
            })?;
            if head.next != Some((beside.row, Vec::new())) {
                return Ok(Placed {
                    page: head,
                    room: None,
                    from: 0,
                });
            }
            request.first_on_page = request.first_on_page && head.rows.is_empty();
            request.y = head.end;
            request.row = beside.row;
            request.part = Vec::new();
            rows = head.rows;
        }
        if let (Some(drop), true) = (beside.drop, request.row == beside.row && request.part.is_empty()) {
            request.y = request.y.max(drop);
        }
        let flow = beside.room.as_ref().map_or(&t.flow, |r| &r.flow);
        let rest = flow.place(&request)?;
        let from = rows.len();
        rows.extend(rest.rows);
        Ok(Placed {
            page: TablePage {
                end: rest.end,
                next: rest.next,
                rows,
            },
            room: beside.room.clone(),
            from,
        })
    }
}
