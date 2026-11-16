"""Round 58, R127: ``split`` in a derive -- a column's text made one row per part (a line
break, or a given separator), the row's other cells repeated, parts trimmed and empty ones
dropped, before ``where`` and ``group_by`` (:mod:`xlsx_agent.split`); the result says how
many rows it made from how many.

Every workbook is made up here: a table of chores whose ``Helpers`` cell lists people on
lines of their own (Alt+Enter), or with ``;`` between them, with empty lines, a name twice,
blanks and a number among them.
"""

from __future__ import annotations

import pytest
from build import WorkbookBuilder
from ooxml_edit.tools import Toolbox

from xlsx_agent import DeriveError, Workbook
from xlsx_agent.split import BY_LINES, parts, splits
from xlsx_agent.tools import GROUPS, TOOLS, format as xlsx_format

COLUMNS = ["Chore", "Helpers", "Room", "Tags"]
ROWS = [
    ["Sweep", "Ann\nBo", "hall", "dust; floor"],
    ["Dust", " Ann \r\n\r\nCy\r\n", "den", "dust"],
    ["Mop", "Bo\nBo", "hall", "floor;;  wet ; "],
    ["Rest", None, "den", None],
    ["Count", 3, "loft", "  ;  "],
    ["Wipe", "Dee", "loft", "glass"],
]


def book() -> WorkbookBuilder:
    out = WorkbookBuilder()
    sheet = out.sheet("Chores")
    sheet.rows("A1", [COLUMNS, *ROWS])
    sheet.table("Chores", "A1:D7", COLUMNS)
    return out


@pytest.fixture(scope="module")
def wb() -> Workbook:
    opened = Workbook.open(book().to_bytes())
    opened.declare_key("t:Chores", "Chore")
    return opened


def test_parts_are_cut_at_line_breaks_or_by_trimmed_and_never_empty():
    assert parts("Ann\nBo", None) == ["Ann", "Bo"]
    assert parts(" Ann \r\n\r\nCy\r\n", None) == ["Ann", "Cy"]
    assert parts("Ann\rBo", None) == ["Ann", "Bo"]
    assert parts("floor;;  wet ; ", ";") == ["floor", "wet"]
    assert parts("  ;  ", ";") == []
    assert parts("a, b,c", ", ") == ["a", "b,c"]               # by is matched exactly
    assert parts("Bo\nBo", None) == ["Bo", "Bo"]                # a part twice is kept twice


def test_a_split_makes_a_row_per_part_and_says_how_many_from_how_many(wb):
    derived = wb.derive("t:Chores", split={"column": "Helpers"}, id="helpers")
    assert [(r["Chore"], r["Helpers"]) for r in derived.rows] == [
        ("Sweep", "Ann"), ("Sweep", "Bo"), ("Dust", "Ann"), ("Dust", "Cy"), ("Mop", "Bo"),
        ("Mop", "Bo"), ("Rest", None), ("Count", 3), ("Wipe", "Dee")]
    assert [r["Room"] for r in derived.rows][:2] == ["hall", "hall"]   # the other cells repeated
    said = derived.split.as_dict()
    assert said["column"] == "Helpers" and said["by"] == BY_LINES
    assert said["rows"] == 9 and said["from_rows"] == 6
    assert said["split_rows"]["count"] == 3
    assert said["blank"]["count"] == 1 and said["not_text"]["count"] == 1
    assert said["repeated"]["count"] == 1
    assert derived.key_columns == 0                      # the keys repeat now


def test_a_split_by_a_separator_drops_empty_parts_and_keeps_an_empty_cell_once(wb):
    derived = wb.derive("t:Chores", split={"column": "Tags", "by": ";"}, id="tags")
    assert [(r["Chore"], r["Tags"]) for r in derived.rows] == [
        ("Sweep", "dust"), ("Sweep", "floor"), ("Dust", "dust"), ("Mop", "floor"), ("Mop", "wet"),
        ("Rest", None), ("Count", None), ("Wipe", "glass")]
    said = derived.split.as_dict()
    assert said["rows"] == 8 and said["from_rows"] == 6 and said["blank"]["count"] == 2


def test_each_part_keeps_its_cells_sources(wb):
    derived = wb.derive("t:Chores", split={"column": "Helpers"}, id="helpers")
    first, second = derived.provenance[0], derived.provenance[1]
    assert [str(s) for s in first["Helpers"].sources] == [str(s) for s in second["Helpers"].sources]
    assert "Helpers" in str(first["Helpers"].sources[0])
    assert "Room" in str(second["Room"].sources[0])


def test_group_by_the_split_column_gives_a_row_per_part(wb):
    derived = wb.derive("t:Chores", split={"column": "Helpers"}, group_by=["Helpers"],
                        measures=[{"fn": "count", "as": "chores"},
                                  {"fn": "distinct", "column": "Room", "as": "rooms"}], id="by_helper")
    got = {r["Helpers"]: (r["chores"], r["rooms"]) for r in derived.rows}
    assert got == {"Ann": (2, 2), "Bo": (3, 1), "Cy": (1, 1), None: (1, 1), 3: (1, 1), "Dee": (1, 1)}
    assert derived.split.rows == 9
    # where filters the split rows: one part at a time
    only = wb.derive("t:Chores", split={"column": "Helpers"}, where={"Helpers": {"eq": "Ann"}},
                     id="ann")
    assert [r["Chore"] for r in only.rows] == ["Sweep", "Dust"]


def test_a_split_from_a_dataset_and_after_a_fill(wb):
    first = wb.derive("t:Chores", where={"Room": {"in": ["hall", "den"]}}, id="near")
    derived = wb.derive(first, split={"column": "Helpers"}, id="near_helpers")
    assert derived.split.from_rows == 4 and derived.split.rows == 7
    filled = wb.derive("t:Chores", fill=[{"column": "Helpers", "from": "Room"}],
                       split={"column": "Helpers"}, id="filled")
    assert ("Rest", "den") in [(r["Chore"], r["Helpers"]) for r in filled.rows]
    assert filled.filled and filled.split.blank.count == 0


def test_the_split_forms_read_as_meant_and_refusals_say_the_way_out(wb):
    assert splits(None) is None and splits([]) is None
    assert splits([{"column": None, "by": None}]) is None                # a strict call's empty entry
    assert splits("Helpers") == {"column": "Helpers", "by": None}
    assert splits([{"column": "Helpers", "by": ""}]) == {"column": "Helpers", "by": None}
    with pytest.raises(DeriveError) as error:
        wb.derive("t:Chores", split={"column": "Helper"}, id="x")
    assert error.value.code == "bad_split" and "Helpers" in (error.value.valid_options or [])
    with pytest.raises(DeriveError) as error:
        wb.derive("t:Chores", split=[{"column": "Helpers"}, {"column": "Tags"}], id="x")
    assert "one split" in error.value.message
    with pytest.raises(DeriveError) as error:
        wb.derive("t:Chores", split={"column": "Helpers", "at": ";"}, id="x")
    assert error.value.code == "bad_split"


def test_xlsx_derive_split_in_a_tool_session():
    with Toolbox(TOOLS, formats=[xlsx_format()], groups=GROUPS, workers=0) as box:
        session = box.session()
        session.open(book().to_bytes(), "chores.xlsx")
        result = box.dispatch(session, "xlsx_derive", {
            "doc": "d1", "from": "t:Chores", "id": "by_helper", "split": {"column": "Helpers", "by": None},
            "group_by": ["Helpers"], "measures": [{"fn": "count", "as": "chores"}]})
        assert result.ok, result.to_json()
        assert result.data["split"]["rows"] == 9 and result.data["split"]["from_rows"] == 6
        assert [row[0]["v"] for row in result.data["rows"]] == ["Ann", "Bo", "Cy", None, 3, "Dee"]
        assert [row[1]["v"] for row in result.data["rows"]] == [2, 3, 1, 1, 1, 1]
        refused = box.dispatch(session, "xlsx_derive", {"doc": "d1", "from": "t:Chores", "id": "x",
                                                        "split": {"column": "Helper"}})
        assert not refused.ok and "Helpers" in str(refused.to_json())
