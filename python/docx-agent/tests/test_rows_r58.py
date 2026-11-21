"""Round 58 (R126, R128): rows added beside a table's rows look like them, a row copied with
its content, check says a written row unlike the rows around it, and a table off its grid
is settled by naming only the changes that put it there.

* R126 (a): a row ``insert_row``, ``set_cells`` ``grow`` and ``sync`` add takes, in each
  cell, the formatting of the cell beside it -- its paragraph mark formatted as that cell's
  first run of text, as Word's Insert Rows does (``tools/row_format_probe.py``,
  ``tests/observations/row-format-word.json``) -- so ``set_cells`` text written into it
  looks like its column (rule 6).
* R126 (b): text written into an empty cell takes its mark's formatting, as Word gives
  typed text (measured: the document default where the mark has none, not the column's);
  the call's warning and ``check``'s ``row_text_format`` say where that leaves a row
  unlike the rows around it, with the ``word_format`` call that makes it match (rules 2, 7).
* R126 (c): ``word_edit_table`` ``copy_row`` -- a row copied with its content, tracked as
  an insertion; refused, with the calls that work, where a copy would be wrong (rule 2).
* ``word_inspect``'s ``new_text``: the formatting new text over a paragraph gets, where it
  is not the paragraph's baseline.
* R128: ``table_grid_mismatch`` names the tracked changes that put the table off its grid
  and settles only them (``fix``), never every change within the table.

Made-up "Shelf" tables only.
"""

from __future__ import annotations

import datetime as dt
import io
import json
import sys
import zipfile
from pathlib import Path

import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent import Document
from docx_agent.tools import FORMAT, GROUPS, TOOLS

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))

import row_format_probe as probe  # noqa: E402
from shelf_tables import ROWS, _shelf  # noqa: E402

CLOCK = lambda: dt.datetime(2026, 11, 16, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
OBSERVED = json.loads((ROOT / "tests" / "observations" / "row-format-word.json").read_text(encoding="utf-8"))


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _open(toolbox, data: bytes):
    session = toolbox.session(clock=CLOCK)
    session.open(data, "shelf.docx")
    return session, session.entry("d1").document


def _now(session) -> Document:
    """The document as it is now: a served edit makes a new version, read afresh."""
    return session.entry("d1").document


def _edit(toolbox, session, **arguments):
    result = toolbox.dispatch(session, "word_edit_table", {"doc": "d1", **arguments})
    assert result.ok, result.to_json()
    return result


def _rows(document: Document) -> list:
    return document._resolve(document.tables()[0].id)[1].element.findall(_W + "tr")


def _children(node) -> list:
    """A ``w:rPr``'s children as ``(tag, attributes)``, revision marks left out."""
    if node is None:
        return []
    return [(c.tag, sorted(c.attrib.items())) for c in node if c.tag not in (_W + "ins", _W + "del")]


def _mark(cell) -> list:
    """A cell's first paragraph mark's run properties, revision marks left out."""
    return _children(cell.find(f"{_W}p/{_W}pPr/{_W}rPr"))


def _look(document: Document, cell_id: str) -> tuple:
    """The effective font, size and colour of a cell's text's first run."""
    from docx_agent.edit import effective

    cell = document.get(cell_id)
    paragraph = cell.paragraphs[0]
    element = paragraph._element if hasattr(paragraph, "_element") else document._resolve(paragraph.id)[1].element
    run = element.find(f".//{_W}r")
    got = effective.run(document, element, run)
    return got.font, got.size, got.color, got.bold


def _check(toolbox, session) -> dict:
    result = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["layout"]})
    assert result.ok and "error" not in result.data["layout"], result.to_json()
    return result.data["layout"]


# -- R126 (a): a row added looks like its table (rule 6, measured) -----------------------------


@pytest.mark.parametrize("key", ["Below", "Above", "Blank", "Marked", "Mixed", "Tracked"])
def test_a_row_inserted_has_the_marks_word_gives_it(toolbox, key):
    """Each new cell's mark as Word's Insert Rows wrote it for the probe's table (its first
    run's formatting -- over a mark formatted otherwise too)."""
    session, document = _open(toolbox, probe.document())
    k = probe.TABLES.index(key)
    table = document.tables()[k].id
    if key == "Tracked":
        assert toolbox.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Editor A"}).ok
    above = key == "Above"
    _edit(toolbox, session, table=table, action="insert_row", index=0 if above else 1,
          side="before" if above else "after")
    document = _now(session)
    rows = document._resolve(document.tables()[k].id)[1].element.findall(_W + "tr")
    new = rows[0 if above else 2]
    word = OBSERVED[key][0 if above else 2]["cells"]
    for cell, seen in zip(new.findall(_W + "tc"), word):
        expected = etree.fromstring(f'<w:pPr xmlns:w="{W}">{_normalised_ppr(seen["pPr"])}'
                                    "</w:pPr>")
        assert _mark(cell) == _children(expected.find(_W + "rPr"))


def _normalised_ppr(xml: str) -> str:
    """The inside of Word's recorded ``w:pPr`` (its ``w:ins`` attributes dropped)."""
    return xml[len("<w:pPr>"):-len("</w:pPr>")] if xml.startswith("<w:pPr>") else ""


def test_text_written_into_a_row_inserted_looks_like_its_column(toolbox):
    session, document = _open(toolbox, _shelf(ROWS))
    table = document.tables()[0].id
    _edit(toolbox, session, table=table, action="insert_row", index=2)
    result = _edit(toolbox, session, table=table, action="set_cells",
                   cells=[{"row": 3, "col": 0, "text": "Fan"}, {"row": 3, "col": 2, "text": "9"}])
    assert not [w for w in result.warnings if "unlike" in w or "word_format" in w], result.warnings
    assert _look(document, f"{document.tables()[0].id}/c3,0") == ("Arial", 8.0, "1F4E79", False)
    assert _look(document, f"{document.tables()[0].id}/c3,2") == ("Arial", 8.0, "1F4E79", True)
    assert _check(toolbox, session)["row_text_format"] == []


def test_rows_grow_and_sync_add_look_like_the_last_row(toolbox):
    from ooxml_edit.tools import Dataset, Result, string, tool

    @tool("shelf_rows", "Keep made-up rows.", {"id": string("Its id.")}, documents=())
    def shelf_rows(call, id):
        call.define_dataset(id, Dataset(("item", "room", "count"),
                                        [("Lamp", "hall", "3"), ("Rug", "den", "5"), ("Vase", "hall", "6")]),
                            source=None)
        return Result(summary="kept")

    with Toolbox([*TOOLS, shelf_rows], formats=[FORMAT], groups=GROUPS, workers=0) as box:
        for how in ("grow", "sync"):
            session, document = _open(box, _shelf(ROWS[:1]))
            assert box.dispatch(session, "shelf_rows", {"id": "next"}).ok
            table = document.tables()[0].id
            if how == "grow":
                _edit(box, session, table=table, action="set_cells", **{"from": "d:next"}, index=1, grow=True)
            else:
                _edit(box, session, table=table, action="sync", **{"from": "d:next"}, key="item")
            now = document.tables()[0].id
            for row in (2, 3):
                assert _look(document, f"{now}/c{row},0") == ("Arial", 8.0, "1F4E79", False), how
                assert _look(document, f"{now}/c{row},2") == ("Arial", 8.0, "1F4E79", True), how


# -- R126 (b): an empty cell, as Word types into it; check says the row (rule 7) ---------------


@pytest.mark.parametrize("tracked", [False, True])
def test_text_in_an_empty_cell_takes_its_columns_formatting_where_word_gives_the_default(toolbox, tracked):
    """Word types into an empty cell with a bare mark in the document default (measured,
    ``Empty``); set_cells writes it in the formatting the same column's rows around it
    share (the team's ask, a departure from Word), and says so, naming the row."""
    assert OBSERVED["Empty"][1]["cells"][1]["runs"] == [["", "typed"]]
    session, document = _open(toolbox, _shelf([ROWS[0], ROWS[1], None, ROWS[2]]))
    table = document.tables()[0].id
    if tracked:
        assert toolbox.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Editor A"}).ok
    result = _edit(toolbox, session, table=table, action="set_cells",
                   cells=[{"row": 3, "col": 0, "text": "Rug"}, {"row": 3, "col": 2, "text": "7"}])
    document = _now(session)
    table = document.tables()[0].id
    assert _look(document, f"{table}/c3,0") == ("Arial", 8.0, "1F4E79", False)
    assert _look(document, f"{table}/c3,2") == ("Arial", 8.0, "1F4E79", True)
    notes = [w for w in result.warnings if "column's formatting" in w]
    assert len(notes) == 2 and "row 2's (Arial 8 pt colour 1F4E79)" in notes[0]
    assert "Word would give it the document default (Calibri 11 pt colour auto)" in notes[0]
    assert not [w for w in result.warnings if "word_format makes it match" in w]
    assert _check(toolbox, session)["row_text_format"] == []
    validate = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["validate"]}).data["validate"]
    assert validate["new"] == [], validate
    if tracked:
        rejected = Document.open(_now(session).to_bytes())
        rejected.reject()
        assert "Rug" not in rejected.to_markdown()


def test_a_row_unlike_its_table_the_fallback_does_not_cover_is_said_with_the_call(toolbox):
    """Text written over a cell's own formatting keeps it (set_text): a red row among blue
    ones stays red, and the warning and row_text_format name it with the word_format call
    that makes it match -- which clears it."""
    colors = ["1F4E79", "1F4E79", "C00000", "1F4E79"]
    session, document = _open(toolbox, _shelf(ROWS, colors=colors))
    table = document.tables()[0].id
    result = _edit(toolbox, session, table=table, action="set_cells", cells=[{"row": 3, "col": 0, "text": "Rug"}])
    assert any("row 3 of" in w and "colour C00000" in w and "word_format" in w for w in result.warnings), \
        result.warnings
    table = _now(session).tables()[0].id
    entry, = _check(toolbox, session)["row_text_format"]
    assert entry["table"] == table and entry["row"] == 3
    cells = [f"{table}/c3,{c}" for c in range(3)]
    assert entry["cells"] == [{"cell": c, "has": {"color": "C00000"}, "around": {"color": "1F4E79"}} for c in cells]
    fix = entry["fix"]
    assert fix == {"tool": "word_format", "calls": [{"targets": cells, "color": "1F4E79"}]}
    for call in fix["calls"]:
        assert toolbox.dispatch(session, fix["tool"], {"doc": "d1", **call}).ok
    assert _check(toolbox, session)["row_text_format"] == []


def test_neighbours_that_disagree_give_no_formatting(toolbox):
    """Rows above and below in two colours: the empty cell's text keeps Word's default."""
    colors = ["1F4E79", "C00000", "1F4E79", "7F7F7F"]
    session, document = _open(toolbox, _shelf([ROWS[0], ROWS[1], None, ROWS[2]], colors=colors))
    table = document.tables()[0].id
    result = _edit(toolbox, session, table=table, action="set_cells", cells=[{"row": 3, "col": 0, "text": "Rug"}])
    assert not [w for w in result.warnings if "column's formatting" in w]
    document = _now(session)
    assert _look(document, f"{document.tables()[0].id}/c3,0")[0] == "Calibri"


def test_rows_that_legitimately_differ_are_not_said(toolbox):
    """No noise: a header row (Georgia, bold), alternating row colours, rows the session did
    not write, and a written row like the rows around it are not entries."""
    colors = ["1F4E79", "7F7F7F", "1F4E79", "7F7F7F"]
    session, document = _open(toolbox, _shelf(ROWS, colors=colors))
    table = document.tables()[0].id
    assert _check(toolbox, session)["row_text_format"] == []                  # nothing written yet
    _edit(toolbox, session, table=table, action="set_cells",
          cells=[{"row": 0, "col": 0, "text": "Thing"}, {"row": 2, "col": 1, "text": "loft"}])
    assert _check(toolbox, session)["row_text_format"] == []


def test_rows_written_one_after_another_each_take_the_unwritten_rows_formatting(toolbox):
    """Rows written in the session are not the rows a written row takes its formatting from."""
    session, document = _open(toolbox, _shelf([ROWS[0], None, None, ROWS[1]]))
    table = document.tables()[0].id
    _edit(toolbox, session, table=table, action="set_cells", cells=[{"row": 2, "col": 0, "text": "Rug"}])
    _edit(toolbox, session, table=table, action="set_cells", cells=[{"row": 3, "col": 0, "text": "Vase"}])
    document = _now(session)
    table = document.tables()[0].id
    assert _look(document, f"{table}/c2,0")[:3] == _look(document, f"{table}/c3,0")[:3] == ("Arial", 8.0, "1F4E79")
    assert _check(toolbox, session)["row_text_format"] == []


def test_a_check_of_a_range_elsewhere_leaves_the_table_out(toolbox):
    session, document = _open(toolbox, _shelf([ROWS[0], None, ROWS[1]]))
    table = document.tables()[0].id
    _edit(toolbox, session, table=table, action="set_cells", cells=[{"row": 2, "col": 0, "text": "Rug"}])
    first = document.paragraphs()[0].id
    result = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["layout"], "range": first})
    assert result.ok, result.to_json()
    assert result.data["layout"]["row_text_format"] == []


# -- word_inspect's new_text -------------------------------------------------------------------


def test_inspect_says_what_new_text_in_a_cell_looks_like(toolbox):
    session, document = _open(toolbox, _shelf([ROWS[0], None]))
    table = document.tables()[0].id
    # a cell whose runs carry the formatting, its mark bare: effective is the style's
    block = toolbox.dispatch(session, "word_inspect", {"doc": "d1", "target": f"{table}/c1,0"}).data["blocks"][0]
    assert block["effective"]["font"] == "Calibri"
    assert block["new_text"] == {"font": "Arial", "sz": 8.0, "color": "1F4E79"}
    # an empty cell with a bare mark: new text is the baseline, so nothing is said
    block = toolbox.dispatch(session, "word_inspect", {"doc": "d1", "target": f"{table}/c2,0"}).data["blocks"][0]
    assert "new_text" not in block
    # a row inserted beside: its mark carries the column's formatting, new text with it
    _edit(toolbox, session, table=table, action="insert_row", index=1)
    block = toolbox.dispatch(session, "word_inspect", {"doc": "d1", "target": f"{table}/c2,2"}).data["blocks"][0]
    assert block["new_text"] == {"font": "Arial", "sz": 8.0, "color": "1F4E79", "b": True}


# -- R126 (c): copy_row ------------------------------------------------------------------------


@pytest.mark.parametrize("tracked", [False, True])
def test_copy_row_copies_a_row_with_its_content(toolbox, tracked):
    session, document = _open(toolbox, _shelf(ROWS))
    table = document.tables()[0].id
    if tracked:
        assert toolbox.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Editor A"}).ok
    result = _edit(toolbox, session, table=table, action="copy_row", index=2)
    assert result.data["markdown"].count("| Kettle | kitchen | **1** |") == 2
    document = _now(session)
    rows = _rows(document)
    assert len(rows) == 6
    ids = [row.get("{http://schemas.microsoft.com/office/word/2010/wordml}paraId") for row in rows]
    assert len(set(ids)) == len(ids)
    paragraphs = [p.get("{http://schemas.microsoft.com/office/word/2010/wordml}paraId") for p in
                  document._resolve(document.tables()[0].id)[1].element.iter(_W + "p")]
    assert len(set(paragraphs)) == len(paragraphs)
    assert _look(document, f"{document.tables()[0].id}/c3,2") == ("Arial", 8.0, "1F4E79", True)
    validate = toolbox.dispatch(session, "check", {"doc": "d1", "include": ["validate"]}).data["validate"]
    assert validate["new"] == [], validate
    if tracked:
        changes = toolbox.dispatch(session, "word_changes", {"doc": "d1", "action": "list"}).data
        listed = changes["changes"] if isinstance(changes, dict) else changes
        assert [c.get("structure") for c in listed] == ["row insertion"]
        document = _now(session)
        rejected = Document.open(document.to_bytes())
        rejected.reject()
        assert len(rejected.tables()[0].rows) == 5
        accepted = Document.open(document.to_bytes())
        accepted.accept()
        assert len(accepted.tables()[0].rows) == 6


def test_copy_row_before_and_by_default_the_last(toolbox):
    session, document = _open(toolbox, _shelf(ROWS))
    table = document.tables()[0].id
    _edit(toolbox, session, table=table, action="copy_row")
    _edit(toolbox, session, table=table, action="copy_row", index=1, side="before")
    text = _now(session).to_markdown()
    assert text.count("Chair") == 2 and text.count("Lamp") == 2
    assert text.index("Lamp") < text.index("Lamp", text.index("Lamp") + 1) < text.index("Kettle")


def test_copy_row_leaves_bookmarks_and_comments_on_the_row_copied(toolbox):
    document = Document.open(_shelf(ROWS))
    table = document.tables()[0]
    cell = table.cell(1, 0)
    document.add_bookmark(cell.paragraphs[0].id, "lamp_row")
    document.add_comment(cell.paragraphs[0].id, "Check the lamp.")
    result = document.copy_row(table.id, 1)
    assert result.warnings == ["left on row 1, not copied: 1 bookmark(s), 1 comment(s)"]
    data = document.to_bytes()
    body = zipfile.ZipFile(io.BytesIO(data)).read("word/document.xml").decode()
    assert body.count("lamp_row") <= 1


def test_copy_row_of_a_row_merged_down_joins_the_merge(toolbox):
    # Refused until round 59 (R136): now the copy joins the merge it lands in
    # (tests/test_merged_rows_r59.py has the cases measured in Word).
    session, document = _open(toolbox, _shelf(ROWS))
    table = document.tables()[0].id
    _edit(toolbox, session, table=table, action="merge", first={"row": 1, "col": 0}, last={"row": 2, "col": 0})
    copied = toolbox.dispatch(session, "word_edit_table", {"doc": "d1", "table": table, "action": "copy_row",
                                                           "index": 1})
    assert copied.ok, copied.to_json()
    rows = _now(session)._resolve(table)[1].element.findall(_W + "tr")
    merges = [None if (m := row.find(f"{_W}tc/{_W}tcPr/{_W}vMerge")) is None else (m.get(_W + "val") or "continue")
              for row in rows]
    assert merges[1:4] == ["restart", "continue", "continue"]


# -- R128: a table off its grid settled by naming only its changes -----------------------------


def _with_inserted_cell(data: bytes, author: str = "Editor B") -> bytes:
    """``data``'s first table with a cell inserted at the end of row 1 as a tracked change
    (``w:cellIns``), off the grid, and a tracked insertion of text in row 3 by another
    author -- a change the fix must leave pending."""
    archive = zipfile.ZipFile(io.BytesIO(data))
    root = etree.fromstring(archive.read("word/document.xml"))
    rows = root.find(f".//{_W}tbl").findall(f"{_W}tr")
    said = {f"{_W}author": author, f"{_W}date": "2026-11-12T09:00:00Z"}
    cell = etree.fromstring(f'<w:tc xmlns:w="{W}"><w:tcPr><w:tcW w:w="1200" w:type="dxa"/></w:tcPr>'
                            "<w:p><w:r><w:t>Spare</w:t></w:r></w:p></w:tc>")
    etree.SubElement(cell.find(f"{_W}tcPr"), f"{_W}cellIns", {f"{_W}id": "700", **said})
    rows[1].append(cell)
    paragraph = rows[3].find(f"{_W}tc/{_W}p")
    inserted = etree.SubElement(paragraph, f"{_W}ins", {f"{_W}id": "710", f"{_W}author": "Editor C",
                                                         f"{_W}date": "2026-11-12T09:00:00Z"})
    run = etree.SubElement(inserted, f"{_W}r")
    etree.SubElement(run, f"{_W}t").text = " extra"
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as written:
        for info in archive.infolist():
            body = etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True) \
                if info.filename == "word/document.xml" else archive.read(info.filename)
            written.writestr(info, body)
    return out.getvalue()


def test_a_table_off_its_grid_names_the_changes_and_rejects_only_them(toolbox):
    session, document = _open(toolbox, _with_inserted_cell(_shelf(ROWS)))
    entry, = [e for e in _check(toolbox, session)["table_grid_mismatch"] if e["drawn"] == "widened"]
    assert entry["tracked"] is True
    assert entry["changes"] == ["rev:700"]
    fix = entry["fix"]
    assert fix == {"tool": "word_changes", "calls": [{"action": "reject", "ids": ["rev:700"]}]}
    assert "within" not in json.dumps(fix) and entry["table"] not in json.dumps(fix)
    assert toolbox.dispatch(session, fix["tool"], {"doc": "d1", **fix["calls"][0]}).ok
    assert _check(toolbox, session)["table_grid_mismatch"] == []
    left = toolbox.dispatch(session, "word_changes", {"doc": "d1", "action": "list"}).data
    assert "extra" in json.dumps(left)                       # the other author's insertion stays pending


def test_a_call_that_puts_a_table_off_its_grid_is_named_with_its_undo(toolbox):
    """A word_edit_table call after which a table is off its grid (made here by hand: no
    action puts one there) says so, and check's entry names the call (made_by, as check
    adds it to the layout's entries)."""
    from types import SimpleNamespace

    from docx_agent.tools.objects import _made_off_grid

    session, document = _open(toolbox, _shelf(ROWS))
    table = _edit(toolbox, session, table=document.tables()[0].id, action="set_cells",
                  cells=[{"row": 1, "col": 1, "text": "hall"}]).changed[0]
    row = _rows(document)[2]
    row.append(etree.fromstring(f'<w:tc xmlns:w="{W}"><w:tcPr><w:tcW w:w="1200" w:type="dxa"/></w:tcPr>'
                                "<w:p/></w:tc>"))
    document._invalidate()
    call = SimpleNamespace(arguments={"doc": "d1", "table": table, "action": "merge", "first": None})
    said = _made_off_grid(call, document, table)
    assert said and "undo" in said
    from docx_agent.layout_facts import table_grid_mismatch
    from docx_agent.tools.objects import ragged_made_by

    layout = {"table_grid_mismatch": table_grid_mismatch(document)}
    ragged_made_by(document, layout)
    entry, = [e for e in layout["table_grid_mismatch"] if e["drawn"] == "widened"]
    assert entry["made_by"] == {"tool": "word_edit_table",
                                "arguments": {"doc": "d1", "table": table, "action": "merge"}}
