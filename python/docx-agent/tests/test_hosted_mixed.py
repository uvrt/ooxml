"""A session holding Word documents on the Rust host beside a workbook the Python host holds
(Phase 3 E1b-2, decisions D2 and D3; the values channel): one id counter and one room, the
workbook's rows read by a Word call as the Python host resolved them, and every call routed by the
documents it names.

- The Python host's document is a stand-in in the Rust session: its id from the one counter, the
  room counted once, its state numbers told as they move.
- A Word call reads the workbook's ``t:`` rows, and a reference to one of its values, as resolved
  in Python first; a check of numbers reads its tables too.
- A Word tool aimed at the workbook is refused as on any document of another kind; a workbook's
  tool runs on the Python host.
- A ``read_batch`` of both runs each op where its document is held; a ``batch`` of both is
  refused with the way out, two batches.
- A toolbox with a Python format of a kind the Rust session cannot name stays on the Python host.

Made-up documents and values only.
"""

from __future__ import annotations

import dataclasses

import pytest
from ooxml_edit.tools import Dataset, DocumentFormat, Result, Session, Toolbox, integer, string, tool
from ooxml_edit.tools.hosted import HostDocument, HostedSession

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from word_docs import blank, paragraphs


class Grid:
    """A made-up workbook: ``t:stock``, a table of three rows; ``version`` moves as it changes."""

    def __init__(self) -> None:
        self.tables = {"stock": (("item", "count"), [("bolts", 12), ("nuts", 7), ("pins", 3)])}


def _table(grid: Grid, address: str) -> Dataset:
    """``t:<name>``, or one of its rows by key (``t:stock/nuts``) or number (``t:stock/#2``)."""
    from ooxml_edit.tools import ToolError

    name, _, row = address[len("t:"):].partition("/")
    if name not in grid.tables:
        raise ToolError("not_found", f"no table t:{name}", valid_options=[f"t:{n}" for n in grid.tables])
    header, rows = grid.tables[name]
    if row.startswith("#"):
        rows = rows[int(row[1:]) - 1:int(row[1:])]
    elif row:
        rows = [r for r in rows if r[0] == row]
    return Dataset(header, rows, keys=1)


GRID = DocumentFormat(kind="xlsx", open=lambda data: Grid(), detect=lambda data, name: False,
                      datasets={"t:": _table}, value_sources=lambda grid: [f"t:{n}" for n in grid.tables])


@tool("grid_count", "Test only: set an item's count in a made-up workbook.",
      {"doc": string("Document id."), "item": string("The item."), "count": integer("Its count.")},
      kind="xlsx", mutates=True)
def grid_count(call, doc, item, count):
    header, rows = call.entry.document.tables["stock"]
    call.entry.document.tables["stock"] = (header, [(i, count if i == item else c) for i, c in rows])
    return Result(summary=f"{item}: {count}")


@tool("grid_items", "Test only: list a made-up workbook's items.", {"doc": string("Document id.")}, kind="xlsx")
def grid_items(call, doc):
    return Result(summary="items", data=[row[0] for row in call.entry.document.tables["stock"][1]])


@pytest.fixture(scope="module")
def box():
    with Toolbox([*TOOLS, grid_count, grid_items], formats=[FORMAT, GRID], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def opened(box):
    session = box.session()
    doc = session.open(blank(), "report.docx")
    grid = session.adopt(Grid(), "xlsx", "stock.xlsx")
    return session, doc, grid


def ok(box, session, name, **arguments):
    result = box.dispatch(session, name, arguments)
    assert result.ok, result.to_json()
    return result


def test_the_workbook_is_named_and_counted_by_the_rust_session(box):
    session, doc, grid = opened(box)
    assert isinstance(session, HostedSession)
    assert (doc, grid) == ("d1", "d2")
    assert list(session.documents) == ["d1", "d2"]
    assert session.documents["d1"] == HostDocument("d1", "docx", "report.docx")
    assert session.documents["d2"].kind == "xlsx" and session.entry("d2").document.tables
    assert session.rust.document("d2")[:2] == ("xlsx", "stock.xlsx")
    session.close("d2")
    assert list(session.documents) == ["d1"]
    assert session.open(blank(), "again.docx") == "d3", "ids are never given twice"


def test_a_word_call_reads_the_workbooks_rows_and_values_as_the_python_host_resolved_them(box):
    session, doc, _grid = opened(box)
    placed = ok(box, session, "word_insert_markdown", doc=doc, at="end",
                markdown="Nuts: {{t:stock/nuts/count}}. Pins: {{d2/t:stock/#3/count}}.")
    assert placed.data["facts"][0]["text"] == "7", placed.to_json()
    assert "Nuts: 7. Pins: 3." in [text for _id, text in paragraphs(session, doc)]
    refused = box.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": "{{t:nope/x/y}}"})
    assert not refused.ok and refused.error.code != "internal", refused.to_json()
    assert "t:stock" in refused.error.message + str(refused.error.valid_options), refused.to_json()


def test_a_workbooks_change_makes_what_was_read_of_it_stale_through_the_rust_session(box):
    session, doc, grid = opened(box)
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="Bolts: {{t:stock/bolts/count}}.")
    version = session.entry(grid).version
    ok(box, session, "grid_count", doc=grid, item="bolts", count=13)
    assert session.entry(grid).version == version + 1
    assert session.rust.document(grid)[2] == version + 1, "told as it moved"
    again = ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="Now {{t:stock/bolts/count}}.")
    assert again.data["facts"][0]["text"] == "13"


def test_a_check_of_numbers_reads_the_workbooks_tables(box):
    session, doc, _grid = opened(box)
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="We hold 12 bolts and 8 nuts.")
    checked = ok(box, session, "check", doc=doc, include=["numbers"])
    assert checked.data["numbers"]["values"] == 3, "t:stock's three counts, read through the values channel"


def test_a_word_tool_aimed_at_the_workbook_is_refused_as_on_any_other_kind(box):
    session, _doc, grid = opened(box)
    refused = box.dispatch(session, "word_read", {"doc": grid})
    assert not refused.ok and refused.error.code == "invalid_arguments", refused.to_json()
    assert refused.error.message == "word_read does not work on a xlsx document"
    assert refused.error.valid_options == ["d1 (docx)"]
    listed = box.dispatch(session, "grid_items", {"doc": grid})
    assert listed.ok and listed.data == ["bolts", "nuts", "pins"], listed.to_json()
    wrong = box.dispatch(session, "grid_items", {"doc": "d1"})
    assert not wrong.ok and wrong.error.message == "grid_items does not work on a docx document", wrong.to_json()


def test_a_read_batch_of_both_runs_each_op_where_its_document_is_held(box):
    session, doc, grid = opened(box)
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="Hello.")
    batch = ok(box, session, "read_batch", ops=[
        {"tool": "word_read", "arguments": {"doc": doc}},
        {"tool": "grid_items", "arguments": {"doc": grid}}])
    ops = batch.data["ops"]
    assert "Hello." in str(ops[0]["data"]) and ops[1]["data"] == ["bolts", "nuts", "pins"], batch.to_json()
    assert [r.tool for r in session.log][-1] == "read_batch"


def test_a_batch_changing_both_is_refused_with_the_way_out_two_batches(box):
    session, doc, grid = opened(box)
    ops = [{"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": "end", "markdown": "One."}},
           {"tool": "grid_count", "arguments": {"doc": grid, "item": "nuts", "count": 1}}]
    refused = box.dispatch(session, "batch", {"ops": ops})
    assert not refused.ok and refused.error.code == "refused", refused.to_json()
    assert "send them as two batches" in refused.error.message
    assert refused.error.details == {"batches": [[0], [1]]}
    assert refused.error.try_call == {"tool": "batch", "arguments": {"ops": [ops[0]]}}
    assert session.log[-1].tool == "batch" and session.log[-1].code == "refused"
    assert ok(box, session, "batch", ops=[ops[0]]).doc == doc
    assert ok(box, session, "batch", ops=[ops[1]]).doc == grid
    assert session.entry(grid).document.tables["stock"][1][1] == ("nuts", 1)


def test_a_python_format_of_a_kind_the_rust_session_cannot_name_stays_on_the_python_host():
    other = dataclasses.replace(GRID, kind="grid")
    with Toolbox(TOOLS, formats=[FORMAT, other], groups=GROUPS, workers=0) as toolbox:
        assert type(toolbox.session()) is Session
