"""Round 45 (R67): a table filled from a dataset in one pass, and ``word_fields`` update's
reading done once, with a ``timing`` block saying where the time went.

``word_edit_table`` ``set_cells`` ``from`` once wrote its block as one edit per row added and
one or more per cell (a 431-row block took minutes); on many table shapes here, a fill writes
the same bytes each time and its undo gives the document back as it was.  Made-up documents
and data only."""

from __future__ import annotations

import datetime as dt
import hashlib
import io
import json
import sys
import zipfile
from pathlib import Path

import pytest

pytest.importorskip("ooxml_edit.tools.sources", reason="needs ooxml-edit 0.22 (sources)")

from ooxml_edit.tools import Dataset, Result, Toolbox, string, tool  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

import word_docs  # noqa: E402

sys.path.insert(0, str(Path(__file__).parent))
import toc_number_cases  # noqa: E402

CLOCK = lambda: dt.datetime(2026, 11, 8, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731

#: A made-up stock list: a code, a name (some empty), an amount (some missing), a date, a note
#: (some missing, some of two lines, some long, some with French punctuation).
COLUMNS = ("code", "name", "amount", "due", "note")
TYPES = ("text", "text", "number", "date", "text")


def _rows(count: int, long: bool = False) -> list[tuple]:
    out = []
    for i in range(count):
        note = None if i % 4 == 1 else ("line one\nline two" if i % 4 == 2 else f"note {i} : fin !")
        if long and i % 3 == 0:
            note = ("lantern meadow river stone harbour quiet orchard copper " * 12).strip()
        out.append((f"K-{i:03d}", f"Item {i}" if i % 5 else "", 1.5 * i if i % 6 else None,
                    f"2026-0{1 + i % 9}-1{i % 10}", note))
    return out


@tool("stock_put", "Keep a made-up dataset.", {"id": string("Its id."), "rows": string("How many."),
                                                 "long": string("Long notes.", optional=True),
                                                 "typed": string("Typed columns.", optional=True)},
      documents=())
def stock_put(call, id, rows, long=None, typed=None):
    call.define_dataset(id, Dataset(COLUMNS, _rows(int(rows), bool(long)), types=TYPES if typed else None),
                        source=None)
    return Result(summary=f"Kept d:{id}")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox([*TOOLS, stock_put], formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _last_table(session) -> str:
    return f"t:{word_docs.rows_of(word_docs.tables(session, 'd1')[-1])[0].get(word_docs.W14_PARA)}"


def digest(data: bytes) -> str:
    """The package's parts, but its core properties (a new document's creation time)."""
    archive = zipfile.ZipFile(io.BytesIO(data))
    found = hashlib.sha256()
    for info in archive.infolist():
        if info.filename != "docProps/core.xml":
            found.update(info.filename.encode() + b"\0" + archive.read(info.filename))
    return found.hexdigest()


def plain(result) -> dict:
    """A result's JSON without what differs from run to run: its duration and timing."""
    out = result.to_json()
    out.pop("duration", None)
    if isinstance(out.get("data"), dict):
        out["data"] = {k: v for k, v in out["data"].items() if k != "timing"}
    return out


HEAD = "| code | name | amount | due | note |\n|---|---|---|---|---|\n"
ROW = "| a | b | c | d | e |"

#: name -> (setup, fills, dataset): the table shapes compared.
SHAPES = {
    "grow": ({}, [{"grow": True}], {}),
    "header-repeated": ({"header_rows": True}, [{"grow": True}], {}),
    "header-row-only": ({"header_rows": True, "delete_row": True}, [{"grow": True}], {}),
    "number-formats": ({}, [{"grow": True, "date_format": "dd/MM/yyyy"}], {"typed": "1"}),
    "some-columns": ({"markdown": "| x | y |\n|---|---|\n| 1 | 2 |"},
                     [{"grow": True, "from_columns": ["note", "code"]}], {}),
    "below-the-header": ({}, [{"grow": True, "header": [], "index": 1}], {}),
    "rows-left-below": ({"markdown": HEAD + "\n".join([ROW] * 20)}, [{}], {"rows": "12"}),
    "long-text": ({}, [{"grow": True}], {"long": "1"}),
    "cells-of-paragraphs": ({"paragraphs": True}, [{"grow": True}], {}),
    "filled-again": ({}, [{"grow": True}, {}], {}),
    "tracked": ({"tracked": True}, [{"grow": True}], {}),
    "tracked-header": ({"tracked": True, "header_rows": True}, [{"grow": True}], {}),
    "merged-header": ({"merge_header": True}, [{"grow": True, "header": [], "index": 1}], {}),
    "merged-down-above": ({"markdown": HEAD + ROW + "\n" + ROW, "merge_down": True},
                          [{"grow": True, "header": [], "index": 2}], {}),
    "shaded-rows": ({"shade": True}, [{"grow": True}], {}),
    "french": ({"language": "fr-FR"}, [{"grow": True}], {}),
}


def _run(toolbox, setup: dict, fills: list[dict], dataset: dict) -> tuple:
    session = toolbox.session(clock=CLOCK)
    session.open(word_docs.with_body(word_docs.paragraph("Stock report")), "report.docx")
    results = []

    def call(name, **arguments):
        result = toolbox.dispatch(session, name, arguments)
        assert result.ok, json.dumps(result.to_json(), indent=1, default=str)
        results.append(plain(result))
        return result

    if setup.get("language"):
        call("word_styles", doc="d1", action="set_language", language=setup["language"])
    first = word_docs.paragraph_ids(toolbox, session, "d1")[0]
    call("word_insert_markdown", doc="d1", at=f"after:{first}", markdown=setup.get("markdown", HEAD + ROW))
    table = _last_table(session)
    if setup.get("header_rows"):
        call("word_format_table", doc="d1", table=table, scope="table", header_rows=1)
    if setup.get("delete_row"):
        call("word_edit_table", doc="d1", table=table, action="delete_row", index=1)
    if setup.get("shade"):
        call("word_format_table", doc="d1", table=table, scope="row", row=1, shading="FFCC00", height=20)
    if setup.get("merge_header"):
        call("word_edit_table", doc="d1", table=table, action="merge", first={"row": 0, "col": 0},
             last={"row": 0, "col": 1})
    if setup.get("merge_down"):
        call("word_edit_table", doc="d1", table=table, action="merge", first={"row": 0, "col": 4},
             last={"row": 1, "col": 4})
    if setup.get("paragraphs"):
        call("word_edit_table", doc="d1", table=table, action="set_cells",
             cells=[{"row": 1, "col": k, "text": "one\ntwo\nthree"} for k in range(5)])
    if setup.get("tracked"):
        call("word_set_tracking", doc="d1", on=True, author="Reviewer")
    call("stock_put", id="stock", rows=dataset.get("rows", "23"), long=dataset.get("long"),
         typed=dataset.get("typed"))
    before = word_docs.read(toolbox, session, "d1", view="markup")
    timings, versions = [], set()
    for fill in fills:
        table = _last_table(session)
        done = call("word_edit_table", doc="d1", table=table, action="set_cells", **{"from": "d:stock", **fill})
        timings.append(done.data["timing"])
        versions.add(done.version)                  # (a fill writing what is there makes no step)
    steps = len(versions)
    filled = word_docs.table_texts(session, "d1", _last_table(session))
    for _ in range(steps):
        call("undo", doc="d1")
    return filled, word_docs.read(toolbox, session, "d1", view="markup"), results, timings, before


@pytest.mark.parametrize("name", sorted(SHAPES))
def test_a_fill_writes_the_same_each_time_and_its_undo_gives_the_table_back(toolbox, name):
    """The same table and results each time, and the document as it was after the undo (as
    word_read says it, markup and ids: the parts an undo wrote again are serialised anew)."""
    setup, fills, dataset = SHAPES[name]
    first = _run(toolbox, setup, fills, dataset)
    again = _run(toolbox, setup, fills, dataset)
    assert first[:3] == again[:3]
    assert first[1] == first[4]                                # undone: as before the fill


def test_the_result_says_where_the_time_went(toolbox):
    _, _, results, timings, _ = _run(toolbox, {}, [{"grow": True}], {})
    timing = timings[0]
    assert list(timing)[:6] == ["resolve", "check", "rows", "cells", "record", "result"]
    # (the Rust session spaces the text as it writes the cells: its "spacing" is in "cells"; on the
    # Rust host the checks after the edit, validation among them, are in the result's own timing)
    assert "validate" not in timing
    assert all(isinstance(v, float) and v >= 0 for v in timing.values())
    assert len(json.dumps(timing)) < 300                       # a small part of the result's budget


def test_a_block_of_many_rows_is_one_fill(toolbox):
    """A block of 120 rows from a dataset: one call, every row in the table."""
    session = toolbox.session(clock=CLOCK)
    session.open(word_docs.blank(), "report.docx")
    first = word_docs.paragraph_ids(toolbox, session, "d1")[0]
    assert toolbox.dispatch(session, "word_insert_markdown", {"doc": "d1", "at": f"after:{first}",
                                                              "markdown": HEAD + ROW}).ok
    assert toolbox.dispatch(session, "stock_put", {"id": "stock", "rows": "120"}).ok
    table = _last_table(session)
    result = toolbox.dispatch(session, "word_edit_table", {"doc": "d1", "table": table, "action": "set_cells",
                                                           "from": "d:stock", "grow": True})
    assert result.ok and result.data["rows"] == 121
    assert len(word_docs.table_texts(session, "d1", table)) == 121


# -- word_fields update ------------------------------------------------------------------------


def test_the_update_says_where_its_time_went(toolbox):
    session = toolbox.session(clock=CLOCK)
    session.open(toc_number_cases.CASES["n-tab"], "toc.docx")
    result = toolbox.dispatch(session, "word_fields", {"doc": "d1", "action": "update"})
    assert result.ok, result.to_json()
    # the Rust host's update says where its time went in the result's own timing (lane P2e6: no
    # part ahead of the lock), not in its data
    assert "timing" not in result.data
    assert result.timing["total"] >= result.timing["work"] > 0


# -- set_cells by position: one pass too (a thousand cells took 80 s one by one) ----------------


def _cells_run(toolbox, tracked: bool) -> tuple:
    session = toolbox.session(clock=CLOCK)
    session.open(word_docs.blank(), "report.docx")
    first = word_docs.paragraph_ids(toolbox, session, "d1")[0]
    results = []

    def call(name, **arguments):
        result = toolbox.dispatch(session, name, arguments)
        assert result.ok, json.dumps(result.to_json(), indent=1, default=str)
        results.append(plain(result))

    call("word_insert_markdown", doc="d1", at=f"after:{first}", markdown=HEAD + "\n".join([ROW] * 6))
    table = _last_table(session)
    call("word_edit_table", doc="d1", table=table, action="merge", first={"row": 1, "col": 0},
         last={"row": 2, "col": 1})
    if tracked:
        call("word_set_tracking", doc="d1", on=True, author="Reviewer")
    # Round 57 (R125): the merged cell (rows 1-2, columns 0-1) given one value at each of its
    # positions -- different values for one merged cell are refused -- and a plain cell twice.
    cells = [{"row": r, "col": c, "text": ("merged : ok\nsecond line" if r in (1, 2) and c < 2 else
                                           f"r{r} c{c} : ok" + ("\nsecond line" if (r + c) % 3 == 0 else ""))}
             for r in range(7) for c in range(5)]
    cells += [{"row": 2, "col": 1, "text": "merged : ok\nsecond line"}, {"row": 3, "col": 3, "text": "d"}]
    call("word_edit_table", doc="d1", table=table, action="set_cells", cells=cells)
    call("word_edit_table", doc="d1", table=table, action="set_cells",
         cells=[{"row": 4, "col": 0, "text": "one"}, {"row": 4, "col_label": "r0 c4 : ok", "text": "by label"}])
    filled = word_docs.table_texts(session, "d1", _last_table(session))
    call("undo", doc="d1")
    return filled, digest(word_docs.saved(session, "d1")), results


@pytest.mark.parametrize("tracked", [False, True])
def test_cells_by_position_write_what_they_wrote_one_by_one(toolbox, tracked):
    assert _cells_run(toolbox, tracked) == _cells_run(toolbox, tracked)


def test_a_cell_the_table_lacks_is_refused_as_before(toolbox):
    session = toolbox.session(clock=CLOCK)
    session.open(word_docs.blank(), "report.docx")
    first = word_docs.paragraph_ids(toolbox, session, "d1")[0]
    assert toolbox.dispatch(session, "word_insert_markdown", {"doc": "d1", "at": f"after:{first}",
                                                              "markdown": HEAD + ROW}).ok
    table = _last_table(session)
    result = toolbox.dispatch(session, "word_edit_table", {"doc": "d1", "table": table, "action": "set_cells",
                                                           "cells": [{"row": 0, "col": 0, "text": "x"},
                                                                     {"row": 9, "col": 0, "text": "y"}]})
    assert not result.ok and "no cell at (9, 0)" in result.error.message
    assert word_docs.table_texts(session, "d1", table)[0][0] == "code"
