"""Round 57 (R125): a table whose last row has a cell over three columns, and the chain of
edits that made it ragged.  Made-up documents only: a 4-column quarter plan, column 0 merged
down over rows 1-2, the last row ``[Totals | one cell over columns 1-3]``.

* **insert_row beside a spanned row** takes the nearest row's cells without a span (else a
  cell per grid column), and says which -- Word's Insert Rows copies the span (measured,
  ``feedback-round57/probe-T``), which made a row of one cell over three columns for four
  values.
* **set_cells giving one merged cell several values** is refused, each cell named with its
  positions, values and the split that gives each value a cell; one value repeated down a
  merged cell is written once.
* **set_cells from over a merged row** offers the way that leaves the row as it is first:
  rows inserted above it, then the call again.
* **split** of a cell over several grid columns: by default into its own columns; into
  fewer along the grid's lines (Word's Split Cells divides the width evenly, adding a grid
  column -- measured -- the result says so); into more than it spans refused with the counts
  that keep the grid.
* **delete_column through a tracked split's new cell** deletes its inserted mark too: Accept
  All then leaves rows of one width (Word's Accept All gave a row a column wider, measured),
  Reject All the table as it was (Word's too).
* **check's table_grid_mismatch** lists a table ragged as Accept All leaves it (``drawn:
  "ragged"``).

Tool-design rules: 1 (a row for values gets a cell per value; a split's count read as the
cell's grid), 2 (refusals name the cells, the values and the calls that work), 3 (one value
repeated is not a clash), 7 (a ragged table is a check fact, with the edit that made it).
"""

from __future__ import annotations

import datetime as dt

import pytest
from ooxml_edit.tools import Dataset, Result, Toolbox, string, tool

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
CLOCK = lambda: dt.datetime(2026, 11, 13, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
ROWS = [("Region", "Owner", "Quarter", "Note"), ("Lowland", "Mira", "Q1", "steady"),
        ("Lowland", "Joost", "Q2", "rising"), ("Upland", "Sanne", "Q1", "flat"),
        ("Totals", "to follow", "", "")]
EXTRA = [("Coast", "Pim", "Q3", "new"), ("Delta", "Fenna", "Q4", "late")]


@tool("plan_rows", "Keep a made-up dataset.", {"id": string("Its id.")}, documents=())
def plan_rows(call, id):
    call.define_dataset(id, Dataset(ROWS[0], [list(r) for r in EXTRA]), source=None)
    return Result(summary=f"Kept d:{id}")


@pytest.fixture(scope="module")
def box():
    with Toolbox([*TOOLS, plan_rows], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def call(box, session, name, /, **arguments):
    return box.dispatch(session, name, arguments)


def ok(box, session, name, /, **arguments):
    result = call(box, session, name, **arguments)
    assert result.ok, (name, result.to_json())
    return result


def _plan(box, *, tracked: bool = False, every_row_spanned: bool = False):
    """The quarter plan: its session and table id (tracking on when ``tracked``)."""
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "plan.docx")
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    markdown = "\n".join(["| " + " | ".join(ROWS[0]) + " |", "|---|---|---|---|"]
                         + ["| " + " | ".join(r) + " |" for r in ROWS[1:]])
    ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{first}", markdown=f"Plan.\n\n{markdown}\n\nEnd.")
    table = _table(session)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="merge", first={"row": 1, "col": 0},
       last={"row": 2, "col": 0})
    ok(box, session, "word_edit_table", doc="d1", table=table, action="merge", first={"row": 4, "col": 1},
       last={"row": 4, "col": 3})
    if every_row_spanned:
        for row in range(4):
            ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="merge",
               first={"row": row, "col": 2}, last={"row": row, "col": 3})
    if tracked:
        ok(box, session, "word_set_tracking", doc="d1", on=True, author="Editor A")
    return session, _table(session)


def _element(session):
    return word_docs.tables(session, "d1")[0]


def _table(session) -> str:
    return f"t:{word_docs.rows_of(_element(session))[0].get(word_docs.W14_PARA)}"


def _span(cell) -> int:
    found = cell.find(f"{_W}tcPr/{_W}gridSpan")
    return int(found.get(_W + "val")) if found is not None else 1


def _shape(session) -> list[list[int]]:
    """Each row's cells as their grid spans."""
    return [[_span(cell) for cell in word_docs.cells_of(row)] for row in word_docs.rows_of(_element(session))]


def _grid(session) -> list[list[tuple[int, int, str]]]:
    """Each grid position's cell: its origin's (row, column) and text -- a cell merged down
    (``w:vMerge`` continuing) is the one it continues."""
    out: list[list[tuple[int, int, str]]] = []
    for r, row in enumerate(word_docs.rows_of(_element(session))):
        line: list[tuple[int, int, str]] = []
        for cell in word_docs.cells_of(row):
            merge = cell.find(f"{_W}tcPr/{_W}vMerge")
            column = len(line)
            if merge is not None and merge.get(_W + "val") in (None, "continue") and r:
                found = out[r - 1][column]
            else:
                found = (r, column, word_docs.cell_text(cell))
            line += [found] * _span(cell)
        out.append(line)
    return out


def _texts(session, row: int) -> list[str]:
    return [text for _, _, text in _grid(session)[row]]


def _ragged(box, session) -> list[dict]:
    facts = ok(box, session, "check", doc="d1", include=["layout"]).data["layout"].get("table_grid_mismatch", [])
    return [e for e in facts if e.get("drawn") == "ragged"]


def _review(box, session, action: str) -> None:
    ok(box, session, "word_changes", doc="d1", action=action, all=True)


# -- insert_row beside a spanned row --------------------------------------------------------------


@pytest.mark.parametrize("tracked", [False, True])
@pytest.mark.parametrize("side", ["before", "after"])
def test_insert_row_beside_a_spanned_row_takes_the_nearest_rows_cells(box, tracked, side):
    session, table = _plan(box, tracked=tracked)
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="insert_row", index=4, side=side)
    shape = _shape(session)
    new = 4 if side == "before" else 5
    assert shape[new] == [1, 1, 1, 1], shape
    assert shape[9 - new] == [1, 3]                       # the spanned row as it was
    assert any("the new row has row 3's cells, one per column" in w for w in result.warnings), result.warnings
    # and a value per column goes in (the chain's step 2)
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="set_cells",
       cells=[{"row": new, "col": c, "text": v} for c, v in enumerate(EXTRA[0])])
    assert _texts(session, new) == list(EXTRA[0])


def test_insert_row_beside_an_unspanned_row_is_as_before(box):
    session, table = _plan(box)
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="insert_row", index=3)
    assert _shape(session)[4] == [1, 1, 1, 1]
    assert not any("cells" in w and "new row" in w for w in result.warnings)


def test_insert_row_where_every_row_has_a_span_gets_a_cell_per_column(box):
    session, table = _plan(box, every_row_spanned=True)
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="insert_row", index=4, side="before")
    assert _shape(session)[4] == [1, 1, 1, 1]
    assert any("a cell for each of the table's 4 grid columns" in w for w in result.warnings), result.warnings


def test_a_new_row_inside_a_vertical_merge_carries_it_on(box):
    """The new row takes row 3's cells, but sits between rows 1 and 2, merged down in
    column 0: its column-0 cell continues that merge (the shape row's has none)."""
    session, table = _plan(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="merge", first={"row": 3, "col": 1},
       last={"row": 3, "col": 2})      # row 3 now spanned too: the nearest plain row is 1 or 4
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="insert_row", index=1)
    grid = _grid(session)
    assert grid[2][0][:2] == (1, 0)                           # inside the merge, as Word does


# -- set_cells: several values for one merged cell -------------------------------------------------


@pytest.mark.parametrize("tracked", [False, True])
def test_several_values_into_one_merged_cell_are_refused(box, tracked):
    session, table = _plan(box, tracked=tracked)
    before = word_docs.saved(session, "d1")
    result = call(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
                  cells=[{"row": 4, "col": c, "text": v} for c, v in enumerate(EXTRA[0])])
    assert not result.ok
    error = result.to_json()["error"]
    assert error["code"] == "refused" and set(error["valid_options"]) == {"split", "insert_row"}
    assert "(4, 1), (4, 2), (4, 3) are one merged cell" in error["message"]
    assert "'Pim', 'Q3', 'new'" in error["message"]
    clash = error["details"]["merged"][0]
    assert clash["values"] == ["Pim", "Q3", "new"] and clash["cells"] == [1, 2, 3]
    assert clash["split"]["arguments"]["columns"] == 3 and clash["split"]["arguments"]["rows"] == 1
    assert word_docs.saved(session, "d1") == before                # nothing written
    # the way out works: the split given, then the four values
    ok(box, session, "word_edit_table", doc="d1", **clash["split"]["arguments"])
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="set_cells",
       cells=[{"row": 4, "col": c, "text": v} for c, v in enumerate(EXTRA[0])])
    assert _texts(session, 4) == list(EXTRA[0])


def test_one_value_down_a_merged_cell_is_written(box):
    session, table = _plan(box)
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
                cells=[{"row": r, "col": 0, "text": "Midland"} for r in (1, 2)])
    assert _texts(session, 1)[0] == "Midland"
    refused = call(box, session, "word_edit_table", doc="d1", table=_table(session), action="set_cells",
                   cells=[{"row": 1, "col": 0, "text": "Midland"}, {"row": 2, "col": 0, "text": "Highland"}])
    assert not refused.ok and refused.to_json()["error"]["details"]["merged"][0]["split"]["arguments"]["rows"] == 2
    assert result.ok


# -- set_cells from over a merged row ---------------------------------------------------------------


@pytest.mark.parametrize("grow", [False, True])
def test_a_block_over_a_merged_row_offers_rows_above_it_first(box, grow):
    session, table = _plan(box)
    ok(box, session, "plan_rows", id="extra")
    asked = {"doc": "d1", "table": table, "action": "set_cells", "from": "d:extra", "index": 4}
    if grow:
        asked["grow"] = True
    result = call(box, session, "word_edit_table", **asked)
    assert not result.ok
    error = result.to_json()["error"]
    assert error["valid_options"] == ["insert_row", "split"]
    assert "To keep row 4 as it is, insert the rows above it" in error["message"]
    first, again, split = error["details"]["calls"]
    assert first["arguments"] == {"doc": "d1", "table": table, "action": "insert_row", "index": 4,
                                  "side": "before"} and first["times"] == 3
    assert split["arguments"]["action"] == "split" and split["arguments"]["columns"] == 3
    for _ in range(first["times"]):
        ok(box, session, first["tool"], **first["arguments"])
    ok(box, session, again["tool"], **again["arguments"])
    assert _texts(session, 5) == list(EXTRA[0]) and _texts(session, 6) == list(EXTRA[1])
    assert _shape(session)[-1] == [1, 3]                  # the Totals row left as it was


def test_grow_after_a_spanned_last_row_adds_rows_with_a_cell_per_column(box):
    session, table = _plan(box)
    ok(box, session, "plan_rows", id="extra")
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="set_cells", index=5, grow=True,
                **{"from": "d:extra"})
    assert _shape(session)[5:] == [[1, 1, 1, 1]] * 3
    assert any("the new row has row 3's cells" in w for w in result.warnings), result.warnings


# -- split of a cell over several grid columns -----------------------------------------------------


@pytest.mark.parametrize("tracked", [False, True])
def test_split_by_default_gives_a_spanned_cell_its_own_columns(box, tracked):
    session, table = _plan(box, tracked=tracked)
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="split", first={"row": 4, "col": 1})
    assert result.to_json()["data"]["columns"] == 4
    assert _shape(session) == [[1, 1, 1, 1]] * 5, _shape(session)


def test_split_into_fewer_follows_the_grid_and_says_so(box):
    session, table = _plan(box)
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="split", first={"row": 4, "col": 1},
                columns=2)
    assert _shape(session)[0] == [1, 1, 1, 1] and _shape(session)[4] == [1, 2, 1]
    assert any("along the table's grid, over 2, 1 column(s) each" in w and "Word's Split Cells" in w
               for w in result.warnings), result.warnings


def test_split_into_more_than_its_span_is_refused_with_the_counts_that_keep_the_grid(box):
    session, table = _plan(box)
    result = call(box, session, "word_edit_table", doc="d1", table=table, action="split", first={"row": 4, "col": 1},
                  columns=4)
    error = result.to_json()["error"]
    assert error["code"] == "refused" and error["valid_options"] == [2, 3]
    assert error["details"]["call"]["arguments"]["columns"] == 3
    assert "split into 3, then split one of the new cells" in error["message"]


def test_a_plain_cell_still_splits_in_two_by_default(box):
    session, table = _plan(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="split", first={"row": 3, "col": 3})
    assert _shape(session)[3] == [1, 1, 1, 1, 1] and _shape(session)[0] == [1, 1, 1, 2]


# -- a column deleted through a tracked split's new cell -------------------------------------------


def _split_then_delete(box):
    session, table = _plan(box, tracked=True)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="split", first={"row": 4, "col": 1})
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="delete_column", index=3)
    return session


def test_accept_all_after_a_column_deleted_through_a_split_leaves_rows_of_one_width(box):
    session = _split_then_delete(box)
    assert not _ragged(box, session)
    _review(box, session, "accept")
    assert _shape(session) == [[1, 1, 1]] * 5


def test_reject_all_after_a_column_deleted_through_a_split_gives_the_table_back(box):
    session = _split_then_delete(box)
    _review(box, session, "reject")
    assert _shape(session) == [[1, 1, 1, 1]] * 4 + [[1, 3]]


# -- check: a table ragged after Accept All ----------------------------------------------------------


#: A 4-column table whose first row's last cell is deleted as a tracked change, so Accept All
#: leaves that row a column short of the grid: ragged in the file, made by no call here.
RAGGED = ('<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid>'
          + '<w:gridCol w:w="2000"/>' * 4 + "</w:tblGrid>"
          + "".join(f'<w:tr w14:paraId="1C0000{r}0" w14:textId="77777777">'
                    + "".join('<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/>'
                              + ('<w:cellDel w:id="9" w:author="Editor A" w:date="2026-11-13T09:00:00Z"/>'
                                 if (r, c) == (0, 3) else "")
                              + f'</w:tcPr><w:p w14:paraId="1C00{r}{c}01" w14:textId="77777777"><w:r><w:t>r{r}c{c}'
                              "</w:t></w:r></w:p></w:tc>" for c in range(4))
                    + "</w:tr>" for r in range(3))
          + "</w:tbl>")


def test_a_ragged_table_from_the_file_is_named_with_no_made_by(box):
    other = box.session(clock=CLOCK)
    other.open(word_docs.with_body(word_docs.paragraph("Plan.") + RAGGED + word_docs.paragraph("End.")), "again.docx")
    ragged = _ragged(box, other)
    assert ragged and "made_by" not in ragged[0] and ragged[0]["after_accept"] is True, ragged
    assert ragged[0]["grid"] == 4 and ragged[0]["rows"][0]["row"] == 0


def test_rows_of_one_width_are_not_listed(box):
    session, _table_id = _plan(box, tracked=True)
    assert _ragged(box, session) == []


# -- a column this session inserted, deleted again (lane D's find) ------------------------------------


def test_an_own_inserted_column_deleted_goes_outright_and_rows_still_delete(box):
    """Tracking on: insert_column, fill it, delete it again -- gone outright, as one's own rows
    are (round 51); a row inserted after it has no cell there, and deletes."""
    session, table = _plan(box, tracked=True)
    before = _shape(session)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="insert_column", index=3)
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="set_cells",
       cells=[{"row": r, "col": 4, "text": f"n{r}"} for r in range(4)])
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="delete_column", index=4)
    assert _shape(session) == before
    assert not any(True for _ in _element(session).iter(f"{_W}cellDel"))
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="insert_row", index=3)
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="delete_row", index=4)
    assert _shape(session) == before


def test_a_new_row_beside_a_column_deleted_pending_is_deleted_there_too(box):
    """A column of the file deleted, tracked: a row inserted after has its cell in that column
    deleted with it (w:cellDel and its mark), so Accept All leaves rows of one width, and the
    new row deletes (it was read as a pending merge)."""
    session, table = _plan(box, tracked=True)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="delete_column", index=3)
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="insert_row", index=2)
    new = word_docs.rows_of(_element(session))[3]
    last = word_docs.cells_of(new)[-1]
    assert last.find(f"{_W}tcPr/{_W}cellDel") is not None
    assert last.find(f"{_W}p/{_W}pPr/{_W}rPr/{_W}del") is not None
    assert not _ragged(box, session)
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="delete_row", index=3)
    ok(box, session, "word_edit_table", doc="d1", table=_table(session), action="insert_row", index=2)
    _review(box, session, "accept")
    assert _shape(session) == [[1, 1, 1]] * 5 + [[1, 2]]
