"""Round 53 (R107): a table whose row reaches past its grid.

A row with more cells (or columns spanned) than ``w:tblGrid`` -- Word's own form of a merge or
a column deletion tracked (the cell merged away kept, ``w:cellDel``), a row from a table of
another grid -- stopped docx2svg's layout with an ``IndexError``, which the tool layer reported
as ``not_found``: render, check and the live view of the document all failed, again and again.
docx2svg (0.11.11) lays such a table out as Word does; here:

* ``check`` says it (``layout``'s ``table_grid_mismatch``: the table, its rows, how Word draws
  it, the call that settles a tracked one), from the document alone, so even where the layout
  failed (rule 7);
* the edits that left a table off its grid keep it in step: a row inserted beside a column
  whose tracked deletion or insertion is pending goes with that change, and a review leaves
  every table it touched fitting its grid.

Made-up "Shelf" reports only.
"""

from __future__ import annotations

import datetime as dt
import functools
import io
import zipfile

import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox
from layout_reads import table_element, table_ids, written

from docx_agent.tools import FORMAT, GROUPS, TOOLS
import word_docs

CLOCK = lambda: dt.datetime(2026, 11, 12, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
SHELF = ("| item | room | notes |\n|---|---|---|\n| Lamp | hall | Outcome: partly done |\n"
         "| Kettle | kitchen | Outcome: done |\n| Stool | attic | Outcome: open |")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@functools.lru_cache(maxsize=1)
def _shelf() -> bytes:
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK)
        return word_docs.saved(session, written(box, session, "Shelf report.\n\n" + SHELF + "\n\nThe end.",
                                                "shelf.docx"))


def _with_extra_cells(data: bytes, row: int = 1, extra: int = 1) -> bytes:
    """``data`` with ``extra`` cells more in its first table's row ``row``: a row off its
    grid, as a row copied from a table of another grid leaves it."""
    archive = zipfile.ZipFile(io.BytesIO(data))
    root = etree.fromstring(archive.read("word/document.xml"))
    table = root.find(f".//{_W}tbl")
    target = table.findall(f"{_W}tr")[row]
    for _ in range(extra):
        cell = etree.fromstring(f'<w:tc xmlns:w="{_W[1:-1]}"><w:tcPr><w:tcW w:w="1500" w:type="dxa"/></w:tcPr>'
                                "<w:p><w:r><w:t>Spare</w:t></w:r></w:p></w:tc>")
        target.append(cell)
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as written_:
        for info in archive.infolist():
            body = etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True) \
                if info.filename == "word/document.xml" else archive.read(info.filename)
            written_.writestr(info, body)
    return out.getvalue()


def _open(toolbox, data: bytes):
    session = toolbox.session(clock=CLOCK)
    return session, session.open(data, "shelf.docx")


def _ok(toolbox, session, tool: str, **arguments):
    result = toolbox.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.to_json())
    return result


def _fits(toolbox, session, doc: str) -> bool:
    """No table of the document reaches past its grid: no grid problem in its validation."""
    problems = _ok(toolbox, session, "check", doc=doc, include=["validate"]).data["validate"]["problems"]
    return not any(problem.startswith("[grid-") for problem in problems)


def _reviewed(toolbox, session, doc: str, how: str) -> str:
    """A copy of the document with every change accepted (``how`` "accept") or rejected."""
    copy = session.open(word_docs.saved(session, doc), f"{how}.docx")
    _ok(toolbox, session, "word_changes", doc=copy, action=how, all=True)
    return copy


def _mismatch(toolbox, session, doc: str) -> list:
    return _ok(toolbox, session, "check", doc=doc, include=["layout"]).data["layout"]["table_grid_mismatch"]


# -- check says it (rule 7) ------------------------------------------------------------------------


def test_check_names_the_table_and_row_off_the_grid_and_render_draws_it(toolbox):
    session, doc = _open(toolbox, _with_extra_cells(_shelf(), extra=2))
    table = table_ids(toolbox, session, doc)[0]
    result = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["validate", "reflow", "layout"]})
    assert result.ok, result.to_json()
    assert "error" not in result.data["reflow"]
    entry = result.data["layout"]["table_grid_mismatch"]
    assert entry == [{"table": table, "grid": 3, "rows": [{"row": 1, "reaches": 5}], "drawn": "widened",
                      "pages": [1]}]
    assert any("grid-row-mismatch" in problem for problem in result.data["validate"]["problems"])
    assert any("table-grid-mismatch" in w for w in result.data["reflow"]["layout_warnings"])
    rendered = toolbox.dispatch(session, "render", {"doc": "d1", "pages": [1]})
    assert rendered.ok, rendered.to_json()


def test_a_tracked_merge_stays_on_its_grid(toolbox):
    """Round 54 (lane M): docx-agent writes a tracked merge as Word accepts and rejects it exactly -- the
    first cell keeping its span, the cells merged away w:cellDel with their content -- so the
    row stays on its grid while pending (test_tracked_merge_r54m.py)."""
    session, doc = _open(toolbox, _shelf())
    table = table_ids(toolbox, session, doc)[0]
    assert toolbox.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Shelf Editor"}).ok
    merged = toolbox.dispatch(session, "word_edit_table", {"doc": "d1", "table": table, "action": "merge",
                                                           "first": {"row": 1, "col": 0}, "last": {"row": 1, "col": 2}})
    assert merged.ok, merged.to_json()
    result = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["layout"]})
    assert result.ok and "error" not in result.data["layout"], result.to_json()
    assert result.data["layout"]["table_grid_mismatch"] == [] and _fits(toolbox, session, doc)


def _earlier_merge_form(data: bytes) -> bytes:
    """``data`` with its first table's row 1 merged across as docx-agent wrote a tracked
    merge before round 54 lane M: the first cell's span applied, the others w:cellDel."""
    archive = zipfile.ZipFile(io.BytesIO(data))
    root = etree.fromstring(archive.read("word/document.xml"))
    row = root.find(f".//{_W}tbl").findall(f"{_W}tr")[1]
    cells = row.findall(f"{_W}tc")
    span = etree.SubElement(cells[0].find(f"{_W}tcPr"), f"{_W}gridSpan")
    span.set(f"{_W}val", "3")
    said = {f"{_W}author": "Shelf Editor", f"{_W}date": "2026-11-12T09:00:00Z"}
    for k, cell in enumerate(cells[1:]):
        etree.SubElement(cell.find(f"{_W}tcPr"), f"{_W}cellDel", {f"{_W}id": str(900 + k), **said})
        # their content and marks deleted (the text inserted in the first cell left out here)
        for paragraph in cell.findall(f"{_W}p"):
            for run in paragraph.findall(f"{_W}r"):
                wrapper = etree.Element(f"{_W}del", {f"{_W}id": str(910 + k), **said})
                run.addprevious(wrapper)
                wrapper.append(run)
                for text in run.iter(f"{_W}t"):
                    text.tag = f"{_W}delText"
            properties = paragraph.find(f"{_W}pPr")
            if properties is None:
                properties = etree.Element(f"{_W}pPr")
                paragraph.insert(0, properties)
            etree.SubElement(etree.SubElement(properties, f"{_W}rPr"), f"{_W}del",
                             {f"{_W}id": str(920 + k), **said})
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as written_:
        for info in archive.infolist():
            body = etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True) \
                if info.filename == "word/document.xml" else archive.read(info.filename)
            written_.writestr(info, body)
    return out.getvalue()


def test_a_tracked_merge_off_its_grid_is_said_with_the_call_until_accepted(toolbox):
    """A document arriving with a tracked merge off its grid (docx-agent's earlier form, or
    another writer's): check says it, tracked, with the changes that widen it named and
    the call that settles only them (round 58, R128: here their accept -- rejecting them
    keeps the cells beside the first cell's span, a row wider than the others; never an
    accept of every change within the table)."""
    session, doc = _open(toolbox, _earlier_merge_form(_shelf()))
    table = table_ids(toolbox, session, doc)[0]
    result = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["layout"]})
    assert result.ok and "error" not in result.data["layout"], result.to_json()
    entry, = result.data["layout"]["table_grid_mismatch"]
    assert entry["rows"] == [{"row": 1, "reaches": 5}] and entry["tracked"] is True
    fix = entry["fix"]
    assert entry["changes"] == ["rev:900", "rev:901"]
    assert fix == {"tool": "word_changes", "calls": [{"action": "accept", "ids": ["rev:900", "rev:901"]}]}
    assert table not in str(fix)
    settled = toolbox.dispatch(session, fix["tool"], {"doc": "d1", **fix["calls"][0]})
    assert settled.ok, settled.to_json()
    again = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["layout"]})
    assert again.data["layout"]["table_grid_mismatch"] == [] and _fits(toolbox, session, doc)


def test_rows_off_the_grid_are_listed_within_the_cap(toolbox):
    session, doc = _open(toolbox, _with_extra_cells(_shelf(), row=1))
    found = _mismatch(toolbox, session, doc)
    assert len(found) == 1 and found[0]["rows"] == [{"row": 1, "reaches": 4}] and "rows_left_out" not in found[0]


# -- the edits keep the grid in step -----------------------------------------------------------------


def _tracked(toolbox, session, doc: str, *steps) -> None:
    """Each step, ``(tool, arguments)`` made from the first table's id, with tracking on."""
    _ok(toolbox, session, "word_set_tracking", doc=doc, on=True, author="Shelf Editor")
    for step in steps:
        tool, arguments = step(table_ids(toolbox, session, doc)[0])
        _ok(toolbox, session, tool, doc=doc, **arguments)


def _edit(action: str, **arguments):
    return lambda table: ("word_edit_table", {"table": table, "action": action, **arguments})


@pytest.mark.parametrize("name, steps", [
    # a row inserted beside a column whose tracked deletion is pending (accepted: a row a
    # column wider than the grid)
    ("delete column, insert row", [_edit("delete_column", index=1), _edit("insert_row", index=1)]),
    ("delete column, insert row above", [_edit("delete_column", index=2), _edit("insert_row", index=0, side="before")]),
    # a tracked split records no grid (Word's form); a later tracked change recorded the split
    # grid, which rejecting set back under the unsplit cells
    ("split, then cell width", [_edit("split", first={"row": 1, "col": 1}, columns=2),
                                lambda t: ("word_format_table", {"table": t, "scope": "cell", "row": 1, "column": 1,
                                                                 "width": 90})]),
    ("split, then merge", [_edit("split", first={"row": 1, "col": 0}, columns=2),
                           _edit("merge", first={"row": 1, "col": 0}, last={"row": 2, "col": 1})]),
    ("merge across, then insert row", [_edit("merge", first={"row": 1, "col": 0}, last={"row": 1, "col": 2}),
                                       _edit("insert_row", index=1)]),
])
def test_tracked_table_edits_accepted_or_rejected_fit_the_grid(toolbox, name, steps):
    session, doc = _open(toolbox, _shelf())
    _tracked(toolbox, session, doc, *steps)
    for how in ("accept", "reject"):
        assert _fits(toolbox, session, _reviewed(toolbox, session, doc, how)), (name, how)


def test_a_row_inserted_untracked_beside_a_tracked_column_goes_with_it(toolbox):
    session, doc = _open(toolbox, _shelf())
    _tracked(toolbox, session, doc, _edit("insert_column", index=1))
    _ok(toolbox, session, "word_set_tracking", doc=doc, on=False)
    _ok(toolbox, session, "word_edit_table", doc=doc, table=table_ids(toolbox, session, doc)[0], action="insert_row",
        index=1)                                                    # not tracked
    rejected = _reviewed(toolbox, session, doc, "reject")
    assert _fits(toolbox, session, rejected)
    table = table_element(toolbox, session, rejected, table_ids(toolbox, session, rejected)[0])
    spans = [sum(int(tc.find(f"{_W}tcPr/{_W}gridSpan").get(_W + "val")) if tc.find(f"{_W}tcPr/{_W}gridSpan")
                 is not None else 1 for tc in row.findall(_W + "tc")) for row in table.findall(_W + "tr")]
    assert spans == [3] * 5
    assert _fits(toolbox, session, _reviewed(toolbox, session, doc, "accept"))


def test_rows_a_sync_adds_beside_a_tracked_column_deletion_go_with_it(toolbox):
    from ooxml_edit.tools import Dataset, Result, string, tool

    @tool("shelf_rows", "Keep made-up rows.", {"id": string("Its id.")}, documents=())
    def shelf_rows(call, id):
        call.define_dataset(id, Dataset(("item", "room", "notes"),
                                        [("Lamp", "hall", "Outcome: partly done"), ("Rug", "den", "Outcome: open")]),
                            source=None)
        return Result(summary="kept")

    with Toolbox([*TOOLS, shelf_rows], formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK)
        session.open(_shelf(), "shelf.docx")
        assert box.dispatch(session, "shelf_rows", {"id": "next"}).ok
        assert box.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Shelf Editor"}).ok
        table = table_ids(box, session, "d1")[0]
        assert box.dispatch(session, "word_edit_table", {"doc": "d1", "table": table, "action": "delete_column",
                                                         "index": 1}).ok
        synced = box.dispatch(session, "word_edit_table", {"doc": "d1", "table": table, "action": "sync",
                                                           "from": "d:next", "key": "item"})
        assert synced.ok, synced.to_json()
        assert synced.data["added"]["count"] == 1
        for how in ("accept", "reject"):
            assert _fits(box, session, _reviewed(box, session, "d1", how)), how


def test_a_review_leaves_alone_a_table_it_did_not_touch(toolbox):
    session, doc = _open(toolbox, _with_extra_cells(_shelf()))      # off its grid, before any change
    _ok(toolbox, session, "word_insert_markdown", doc=doc, at="end", markdown="Shelf notes.")
    _ok(toolbox, session, "word_set_tracking", doc=doc, on=True, author="Shelf Editor")
    _ok(toolbox, session, "word_insert_markdown", doc=doc, at="end", markdown="A tracked line.")
    accepted = _reviewed(toolbox, session, doc, "accept")
    assert _mismatch(toolbox, session, accepted)[0]["rows"] == [{"row": 1, "reaches": 4}]


def test_accepting_a_column_deletion_beside_a_new_row_adds_no_save_problem(toolbox):
    """R109's repro, from this round: before, the row inserted beside a tracked column
    deletion kept its cell when the deletion was accepted -- four new grid-row-mismatch
    problems in check's validation and a refused save."""
    session, doc = _open(toolbox, _shelf())
    table = table_ids(toolbox, session, doc)[0]
    for name, args in (("word_set_tracking", {"on": True, "author": "Shelf Editor"}),
                       ("word_edit_table", {"table": table, "action": "delete_column", "index": 1}),
                       ("word_edit_table", {"table": table, "action": "insert_row", "index": 1}),
                       ("word_changes", {"action": "accept", "all": True})):
        assert toolbox.dispatch(session, name, {"doc": "d1", **args}).ok, name
    checked = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["validate"]})
    assert checked.data["validate"]["new"] == []
    if "validation" in checked.data:
        assert checked.data["validation"]["new"] == []
    saved = toolbox.dispatch(session, "save_document", {"doc": "d1", "name": "shelf.docx", "format": "docx"})
    assert saved.ok, saved.to_json()
