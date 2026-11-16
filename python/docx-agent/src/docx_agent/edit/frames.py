"""A table's frame, for new tables that look like it (round 58, R129): ``insert_markdown``'s
``table_like`` (``word_insert_markdown`` ``like`` a table).

A table style formats regions -- the header and total rows, the first and last column,
banded rows and columns, the corners (:mod:`.table_styles`) -- and no other column: a look
with a middle column of its own (a shaded third column) is the table's own cell formatting.
A new table made from such a table's **frame** carries it: the table's properties and grid
(its style, look, widths, borders, alignment), each row's properties and each cell's
(shading, borders, widths, merges across, vertical alignment), each cell's first paragraph's
properties, its mark formatted as that paragraph's first run of text, and that formatting
on the text written into it -- as Word's Insert Rows gives a new row (measured, round 58:
``tools/row_format_probe.py``). No text, pictures, bookmarks or comments come with it.

Rows: the new table's rows, in order, take the sample's -- its header rows first (the rows
at its top that repeat as header rows, or its first row where its look has the header row
on), its total row last (its last row, where its look has the total row on), its body rows
repeated in their order between: a sample whose body rows alternate two looks gives rows
that alternate them.  The sample as Accept All leaves it (tracked rows and property
changes settled).

What a frame cannot carry faithfully is refused, with what would work: a sample whose body
rows are merged down (repeating them would cut the merges), with tables nested in cells,
with content controls around rows or cells, rows off its grid, or pending tracked changes
to its cells' structure; and text the sample's merged cells would hide.
"""

from __future__ import annotations

import copy
from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from ..oxml.xml import Element, append_in_order, make, remove
from .errors import EditError

if TYPE_CHECKING:  # pragma: no cover
    from .document import Document

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_ROW_RECORDS = (_W + "ins", _W + "del", _W + "trPrChange")


class FrameError(EditError):
    """A sample table whose frame cannot be copied faithfully, or text that does not fit
    it: why, the sample's shape and a Markdown skeleton that fits."""

    def __init__(self, message: str, shape: dict | None = None, skeleton: str | None = None) -> None:
        super().__init__(message)
        self.shape = shape or {}
        self.skeleton = skeleton


@dataclass
class CellFrame:
    #: Its first grid column, and how many it spans.
    column: int
    span: int
    #: ``w:vMerge``'s value (``restart``, ``continue``) or ``None``.
    merge: str | None
    properties: Element
    #: Its first paragraph, empty: its properties, its mark formatted as its first run of text.
    paragraph: Element
    #: The run properties text written into it takes (the mark's), or ``None``.
    run: Element | None


@dataclass
class RowFrame:
    index: int
    properties: Element | None
    cells: list[CellFrame] = field(default_factory=list)


class TableFrame:
    """A sample table's frame, taken (copied) before anything changes."""

    def __init__(self, document: "Document", table: str) -> None:
        from .importing import _current_view
        from .table_format import own_rows, read_look
        from .tables import _new_row_paragraph

        self.sample = table
        try:
            _, entry = document._table_entry(table)
        except KeyError:
            raise EditError(f"like: no table {table!r}") from None
        original = entry.element
        self._refuse_structure(original)
        element = copy.deepcopy(original)
        _current_view(element)
        properties = element.find(_W + "tblPr")
        self.properties = _clean(properties, (_W + "tblPrChange",)) if properties is not None else make("w:tblPr")
        grid = element.find(_W + "tblGrid")
        self.grid = _clean(grid, (_W + "tblGridChange",)) if grid is not None else make("w:tblGrid")
        self.columns = len(self.grid.findall(_W + "gridCol"))
        look = read_look(properties.find(_W + "tblLook") if properties is not None else None)
        rows = own_rows(element)
        self.notes: list[str] = []
        self.rows: list[RowFrame] = []
        several = False
        for k, row in enumerate(rows):
            frame = RowFrame(k, _clean(row.find(_W + "trPr"), _ROW_RECORDS) if row.find(_W + "trPr") is not None
                             else None)
            column = 0
            for cell in row.findall(_W + "tc"):
                cell_properties = cell.find(_W + "tcPr")
                span_node = cell_properties.find(_W + "gridSpan") if cell_properties is not None else None
                span = max(1, int(span_node.get(_W + "val") or 1)) if span_node is not None else 1
                merge_node = cell_properties.find(_W + "vMerge") if cell_properties is not None else None
                merge = None if merge_node is None else (merge_node.get(_W + "val") or "continue")
                paragraph = _new_row_paragraph(cell)
                mark = paragraph.find(f"{_W}pPr/{_W}rPr")
                several = several or len(cell.findall(_W + "p")) > 1
                frame.cells.append(CellFrame(column, span, merge, _clean(cell_properties, ())
                                             if cell_properties is not None else make("w:tcPr"),
                                             paragraph, copy.deepcopy(mark) if mark is not None else None))
                column += span
            if column != self.columns:
                raise FrameError(f"like {table}: its row {k} reaches {column} of its grid's {self.columns} columns "
                                 "(a row off its grid): a frame copies rows on the grid only -- fix the table first "
                                 "(check's table_grid_mismatch says how)", self.shape_of(rows))
            self.rows.append(frame)
        if several:
            self.notes.append(f"like {table}: cells with more than one paragraph -- each new cell has one, formatted "
                              "as the sample cell's first")
        header = 0
        for row in rows:
            flag = row.find(f"{_W}trPr/{_W}tblHeader")
            if flag is None or flag.get(_W + "val") in ("0", "false", "off"):
                break
            header += 1
        if header == 0 and look.get("first_row") and len(rows) > 1:
            header = 1
        self.header = min(header, len(rows))
        self.total = 1 if look.get("last_row") and len(rows) > self.header + 1 else 0
        self.body = list(range(self.header, len(rows) - self.total))
        merged = [r for r in self.body if any(c.merge is not None for c in self.rows[r].cells)]
        if merged:
            raise FrameError(f"like {table}: its body row {merged[0]} has cells merged down: repeating body rows "
                             "would cut the merge -- use a sample whose body rows are not merged down, or copy the "
                             "table whole (word_copy_from) and write its cells (word_edit_table set_cells)",
                             self.shape_of(rows))

    def _refuse_structure(self, element: Element) -> None:
        if any(True for node in element.iter(_W + "tbl") if node is not element):
            raise FrameError(f"like {self.sample}: a table nested in a cell is not part of a frame -- use a sample "
                             "without nested tables")
        for node in element.iter(_W + "sdt", _W + "customXml"):
            parent = node.getparent()
            if parent is not None and parent.tag in (_W + "tbl", _W + "tr"):
                raise FrameError(f"like {self.sample}: content controls around rows or cells are not copied -- "
                                 "use a sample without them")
        if any(True for _ in element.iter(_W + "cellIns", _W + "cellDel", _W + "cellMerge", _W + "tblGridChange")):
            raise FrameError(f"like {self.sample}: pending tracked changes to its cells -- accept or reject them "
                             f"first (word_changes accept within {self.sample})")

    def shape_of(self, rows: list[Element] | None = None) -> dict:
        """What the sample is, for a refusal (rule 2)."""
        out = {"table": self.sample, "columns": self.columns}
        if hasattr(self, "header"):
            out.update(header_rows=self.header, body_rows=len(self.body), total_row=bool(self.total))
        merged = [(r.index, c.column, c.span) for r in getattr(self, "rows", []) for c in r.cells if c.span > 1]
        if merged:
            out["merged_across"] = [f"row {r} columns {c}-{c + s - 1}" for r, c, s in merged[:10]]
        return out

    def skeleton(self, body_rows: int = 2) -> str:
        """A Markdown table that fits the frame: a header row, ``body_rows`` empty rows."""
        line = "|" + " |" * self.columns
        return "\n".join([line, "|" + "---|" * self.columns] + [line] * body_rows)

    def plan(self, count: int) -> list[RowFrame]:
        """The sample's rows ``count`` new rows take: header rows first, the total row last,
        body rows repeated in their order between."""
        heads = [self.rows[k] for k in range(min(self.header, count))]
        left = count - len(heads)
        total = [self.rows[-1]] if self.total and left > 0 else []
        left -= len(total)
        if left > 0 and not self.body:
            raise FrameError(f"like {self.sample}: it has no body row to repeat ({self.header} header row(s)"
                             f"{', a total row' if self.total else ''}): give at most "
                             f"{self.header + self.total} row(s), or use a sample with body rows", self.shape_of(),
                             self.skeleton(0))
        body = [self.rows[self.body[k % len(self.body)]] for k in range(max(left, 0))]
        return heads + body + total

    def table(self, rows: list[list], write_cell) -> Element:
        """The new table: the frame with ``rows`` -- per row, per grid column, what
        ``write_cell(paragraph, run_properties, value)`` writes into the cell's paragraph."""
        widest = max((len(row) for row in rows), default=0)
        if widest != self.columns:
            raise FrameError(f"like {self.sample}: the Markdown table has {widest} column(s), the sample "
                             f"{self.columns}: give each row {self.columns} cells (a merged cell's text in its "
                             "first column, the columns it covers empty)", self.shape_of(), self.skeleton())
        table = make("w:tbl")
        table.append(copy.deepcopy(self.properties))
        table.append(copy.deepcopy(self.grid))
        for k, (values, frame) in enumerate(zip(rows, self.plan(len(rows)))):
            row = make("w:tr")
            table.append(row)
            if frame.properties is not None and len(frame.properties):
                row.append(copy.deepcopy(frame.properties))
            values = list(values) + [None] * (self.columns - len(values))
            for cell in frame.cells:
                covered = [c for c in range(cell.column + 1, cell.column + cell.span) if _has(values[c])]
                if covered:
                    raise FrameError(f"like {self.sample}: row {k}'s cell spans columns {cell.column}-"
                                     f"{cell.column + cell.span - 1} (merged in the sample): its text goes in column "
                                     f"{cell.column}, column(s) {', '.join(map(str, covered))} left empty",
                                     self.shape_of(), self.skeleton())
                if cell.merge == "continue" and _has(values[cell.column]):
                    raise FrameError(f"like {self.sample}: row {k} column {cell.column} is merged into the cell "
                                     "above in the sample: its text goes in that cell, this one left empty",
                                     self.shape_of(), self.skeleton())
                element = make("w:tc")
                row.append(element)
                element.append(copy.deepcopy(cell.properties))
                paragraph = copy.deepcopy(cell.paragraph)
                element.append(paragraph)
                write_cell(paragraph, cell.run, values[cell.column])
        return table


def _has(value) -> bool:
    return bool(value) and not (isinstance(value, str) and not value.strip())


def _clean(node: Element, drop: tuple[str, ...]) -> Element:
    out = copy.deepcopy(node)
    for child in list(out):
        if not isinstance(child.tag, str) or child.tag in drop:
            remove(child)
    return out


def merge_run_properties(base: Element | None, own: Element | None) -> Element | None:
    """``own`` (a run's ``w:rPr``: its emphasis, character style) over ``base`` (the cell's
    text formatting): each of ``own``'s properties replaces ``base``'s of the same name."""
    if base is None:
        return own
    out = copy.deepcopy(base)
    for child in list(out):
        if child.tag in (_W + "ins", _W + "del", _W + "moveFrom", _W + "moveTo", _W + "rPrChange"):
            remove(child)
    if own is not None:
        for child in own:
            if not isinstance(child.tag, str):
                continue
            for old in out.findall(child.tag):
                remove(old)
            append_in_order(out, copy.deepcopy(child))
    return out if len(out) else None
