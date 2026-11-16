"""Splits: a column's text made one row per part, the row's other cells repeated (round 58,
R127; :mod:`xlsx_agent.derive`'s ``split``)::

    ds = wb.derive("t:Tasks", split={"column": "Owners"}, group_by=["Owners"],
                   measures=[{"fn": "count", "as": "tasks"}], id="by_owner")
    ds.split.as_dict()      # {"column": "Owners", "by": "line breaks", "rows": 9,
                            #  "from_rows": 4, "split_rows": 3, ...}

**Parts.**  A text cell of ``column`` is cut at each line break (``\\n``, ``\\r\\n`` or
``\\r``: a cell's Alt+Enter), or, with ``by``, at each occurrence of that text (``";"``,
``", "``; matched exactly, case counting); each part is trimmed of white space at both
ends, and a part left empty is dropped (``"Ann;;Bo; "`` is ``Ann`` and ``Bo``).  A part
that comes twice in one cell is kept twice, as Excel's Power Query splits a column into
rows, and counted (``repeated``).  A ``list`` cell (a declared split, :mod:`xlsx_agent.
clean`) gives its items, each trimmed and the empty ones dropped alike; ``by`` does not cut
them again.

**Rows.**  A row whose cell has two or more parts becomes that many rows, in the order of
the parts, each with the row's other cells as they are; a row with one part is one row,
its cell that part (trimmed); a row whose cell is blank or holds only separators and
white space is kept once, its cell blank (``blank``); a cell of another type (a number, a
date, an error) is kept as it is (``not_text``).  So no row is lost: every source row is
at least one row, and :attr:`Split` says how many rows came from how many.

**Then the pipeline.**  A split comes after the join and the fills and before everything
else: ``where`` filters the split rows, and ``group_by`` on the split column groups by
part -- one row per part, the measures over the source rows holding it (``distinct`` of
another column counts its different values among them).  Without ``group_by``,
``measures``, ``bins`` or ``pivot`` the result is the split rows themselves.

**Provenance.**  A part's sources are its cell's (by key where the table has a unique key
column, as a derive writes them); every other value's are its own cell's, repeated on each
row made from it.  A split table has no key column: its key values repeat.

**Bounded.**  The rows are read once, whole, as a join's; at most
:data:`~xlsx_agent.join.MAX_JOINED` values (rows times columns) before and after the
split, else :class:`~xlsx_agent.derive.DeriveError` ``too_many_cells``.

**Content.**  Errors name the column and ``by``, never a cell's value.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Mapping, Sequence, cast

from .address import NameNotFound, nearest
from .dataset import DatasetTable
from .fill import FilledTable
from .join import MAX_JOINED, JoinedTable, JoinedValue, _date1904, _moved, _read
from .profile import FIRST_ADDRESSES, Count
from .read.regions import Region

if TYPE_CHECKING:  # pragma: no cover
    from .workbook import Workbook

#: What a split takes.
SPLIT_KEYS = ("column", "by")

#: A cell's line break, as Excel stores one (Alt+Enter) and as text pasted in may hold one.
LINE_BREAKS = re.compile(r"\r\n|\r|\n")

#: ``Split.by`` when no ``by`` is given.
BY_LINES = "line breaks"


class _Counter:
    __slots__ = ("count", "first")

    def __init__(self) -> None:
        self.count = 0
        self.first: list[str] = []

    def add(self, address: str, n: int = 1) -> None:
        self.count += n
        if len(self.first) < FIRST_ADDRESSES:
            self.first.append(address)

    def done(self) -> Count:
        return Count(self.count, tuple(self.first))


@dataclass(frozen=True)
class Split:
    """What a split did: ``column`` the column split, ``by`` what it was cut at
    (:data:`BY_LINES`, or the text given); ``rows`` how many rows it made from
    ``from_rows`` source rows; ``split_rows`` the source rows cut into two or more (their
    first addresses: the cells of ``column``), ``blank`` those kept once with the cell
    blank, ``not_text`` those whose cell was not text (kept as it is), ``repeated`` the
    parts that came again in their cell (kept)."""

    column: str
    by: str
    rows: int
    from_rows: int
    split_rows: Count
    blank: Count
    not_text: Count
    repeated: Count

    def as_dict(self) -> dict:
        out: dict[str, Any] = {"column": self.column, "by": self.by, "rows": self.rows,
                               "from_rows": self.from_rows,
                               "split_rows": self.split_rows.as_dict()}
        for key in ("blank", "not_text", "repeated"):
            count = getattr(self, key)
            if count.count:
                out[key] = count.as_dict()
        return out


class SplitTable(FilledTable):
    """A table's (or a joined, filled table's, or a dataset's) rows with one column split
    into a row per part: what the derive engine reads, as it reads a fill
    (:class:`~xlsx_agent.fill.FilledTable`), whose fills and marks it keeps; ``split``
    what the split did."""

    def __init__(self, columns: Sequence[str], values: list[list[JoinedValue]],
                 sources: list[list[tuple[str, ...]]], address: Any, date1904: bool,
                 joined: Any, filled: list, marks: dict[tuple[int, int], str],
                 split: Split) -> None:
        super().__init__(columns, values, sources, address, 0, date1904, joined, filled, marks)
        self.split = split


def _bad(message: str, **options: Any) -> Exception:
    from .derive import DeriveError

    return DeriveError("bad_split", message, field="split", **options)


def splits(split: Any) -> "dict[str, Any] | None":
    """The caller's split, checked for its form: ``{column, by}`` (a list of one such
    mapping is taken as it; the empty values a strict call sends -- an empty list, an entry
    without a column, ``by`` ``""`` -- are none)."""
    if split is None:
        return None
    if isinstance(split, Sequence) and not isinstance(split, (str, bytes)):
        items = [item for item in split if not (isinstance(item, Mapping) and not item.get("column"))]
        if not items:
            return None
        if len(items) > 1:
            raise _bad("one split per derive: derive from the split dataset (d:) to split "
                       "another column")
        split = items[0]
    if isinstance(split, str):
        split = {"column": split}            # a column's name alone: split at line breaks
    if not isinstance(split, Mapping) or set(split) - set(SPLIT_KEYS):
        raise _bad("split is {column: the column whose text becomes a row per part, by: the "
                   "text between parts (default: line breaks)}", valid_options=SPLIT_KEYS)
    column = split.get("column") or None
    if column is None:
        return None
    by = split.get("by") or None
    if not isinstance(column, str) or (by is not None and not isinstance(by, str)):
        raise _bad("split's column and by are text: {column: \"Owners\", by: \";\"}")
    return {"column": column, "by": by}


def parts(text: str, by: str | None) -> list[str]:
    """``text`` cut at line breaks (or at ``by``), each part trimmed, empty ones dropped."""
    pieces = LINE_BREAKS.split(text) if by is None else text.split(by)
    return [piece.strip() for piece in pieces if piece.strip()]


def split_table(workbook: "Workbook | None", table: "Region | DatasetTable",
                split: Any) -> "SplitTable | Region | DatasetTable":
    """``table``'s rows with ``split``'s column made a row per part (see the module's
    notes); ``table`` itself when ``split`` names none."""
    wanted = splits(split)
    if wanted is None:
        return table
    try:
        target = table.column_index(wanted["column"])
    except NameNotFound:
        raise _bad(f"split's column names a column of the rows (after a join, its columns "
                   f"too); there are {len(table.columns)}: {', '.join(table.columns[:50])}",
                   valid_options=nearest(wanted["column"], table.columns) or list(table.columns)[:50]
                   ) from None
    from .derive import DeriveError

    rows, sources = _read(workbook, table)
    width = len(table.columns)
    if len(rows) * width > MAX_JOINED:
        raise DeriveError("too_many_cells", f"a split of {len(rows) * width} values, more "
                          f"than {MAX_JOINED}: split a smaller table (a derived dataset)",
                          field="split")
    by = wanted["by"]
    values: list[list[JoinedValue]] = []
    where: list[list[tuple[str, ...]]] = []
    old_marks = table.marks if isinstance(table, FilledTable) else {}
    marks: dict[tuple[int, int], str] = {}
    cut, blank, not_text, repeated = _Counter(), _Counter(), _Counter(), _Counter()
    for p, row in enumerate(rows, 1):
        cells = [v if isinstance(v, JoinedValue) else _moved(v, p, o) for o, v in enumerate(row)]
        cell = cells[target]
        if cell.t == "text":
            found = parts(cast(str, cell.v), by)
        elif cell.t == "list":
            found = [str(item).strip() for item in cast(tuple, cell.v) if str(item).strip()]
        else:
            found = None
        if found is None:
            pieces: list[Any] = [None]
            (blank if cell.t == "blank" else not_text).add(cell.a)
        elif not found:
            pieces = [""]
            blank.add(cell.a)
        else:
            pieces = found
            if len(found) > 1:
                cut.add(cell.a)
            again = len(found) - len(set(found))
            if again:
                repeated.add(cell.a, again)
        for piece in pieces:
            position = len(values) + 1
            made = [_at(v, position) for v in cells]
            if piece is not None:
                made[target] = JoinedValue(
                    sheet="", row=position, column=target + 1, v=piece or None,
                    t="text" if piece else "blank", formula=cell.formula, hidden=cell.hidden,
                    cache=cell.cache, external=cell.external,
                    cleaned=cell.cleaned, at=cell.a)
            values.append(made)
            where.append(list(sources[p - 1]))
            for o in range(width):
                if (p, o) in old_marks:
                    marks[(position, o)] = old_marks[(p, o)]
    if len(values) * width > MAX_JOINED:
        raise DeriveError("too_many_cells", f"a split making {len(values) * width} values, "
                          f"more than {MAX_JOINED}: split a smaller table (a derived dataset)",
                          field="split")
    done = Split(table.columns[target], BY_LINES if by is None else by, len(values), len(rows),
                 cut.done(), blank.done(), not_text.done(), repeated.done())
    filled = list(table.filled) if isinstance(table, FilledTable) else []
    joined = getattr(table, "joined", None) if isinstance(table, JoinedTable) else None
    out = SplitTable(list(table.columns), values, where, table.address,
                     _date1904(workbook, table), joined, filled, marks, done)
    from .dataset import column_formats

    out.column_formats = column_formats(workbook, table, table.columns)
    return out


def _at(value: JoinedValue, position: int) -> JoinedValue:
    """``value`` on the split table's row ``position``, named by its own cell."""
    return JoinedValue(sheet="", row=position, column=value.column, v=value.v, t=value.t,
                       formula=value.formula, issue=value.issue, hidden=value.hidden,
                       cache=value.cache, external=value.external, cleaned=value.cleaned,
                       at=value.a)


__all__ = ["BY_LINES", "SPLIT_KEYS", "Split", "SplitTable", "parts", "split_table", "splits"]
