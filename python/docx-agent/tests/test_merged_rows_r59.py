"""Round 59 (R136): rows beside cells merged down.

* ``insert_row`` inside a merge down joins it -- the new row's cell continues the merge --
  and above its first row or below its last stays outside it, as Word's Insert Rows does
  (``tools/merged_rows_probe.py``, ``tests/observations/merged-rows-word.json``: every case
  run here on the document Word was given, and compared with what Word saved).  Beside a
  merge, the result says so with the merge call that joins the row (rule 2).
* ``copy_row`` copies a row with a cell merged down (refused until now): inside the merge
  the copy's cell joins it, empty; outside, it is its own cell with the text it had -- as
  Word pastes a row there (measured).
* ``word_insert_markdown`` ``like`` a table merged down in its body rows (refused until
  now): in each column the sample merges down, an empty cell under another joins its merge
  -- as rows Word adds inside a merge do -- and a cell with text starts one (rule 1).

Made-up tables only.
"""

from __future__ import annotations

import contextlib
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
from docx_agent.edit.errors import EditError
from docx_agent.tools import FORMAT, GROUPS, TOOLS

ROOT = Path(__file__).resolve().parent.parent

import merged_rows_cases as probe  # noqa: E402

CLOCK = lambda: dt.datetime(2026, 11, 17, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
OBSERVED = json.loads((ROOT / "tests" / "observations" / "merged-rows-word.json").read_text(encoding="utf-8"))

#: Each Word case as docx-agent's call: ``(row index from 0, below)``.  Word's insertion point
#: in a merged cell is in the merge's first row (``…Merged``: row 2 of the probe's).
INSERTS = {"BelowLast": (3, True), "BelowLastMerged": (1, True), "BelowFirst": (1, True), "Inside": (2, True),
           "InsideAbove": (2, False), "AboveFirst": (1, False), "AboveFirstMerged": (1, False), "End": (4, True),
           "TrackedInside": (2, True), "TrackedBelowLast": (3, True)}


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _call(box, session, tool: str, /, **arguments):
    return box.dispatch(session, tool, arguments)


def _shape(table: etree._Element) -> list[list[tuple]]:
    """Per row, per cell: ``(vMerge, text, mark inserted)``; and whether the row is inserted."""
    out = []
    for row in table.findall(_W + "tr"):
        cells = [(None if (m := cell.find(f"{_W}tcPr/{_W}vMerge")) is None else (m.get(_W + "val") or "continue"),
                  "".join(t.text or "" for t in cell.iter(_W + "t")),
                  cell.find(f"{_W}p/{_W}pPr/{_W}rPr/{_W}ins") is not None)
                 for cell in row.findall(_W + "tc")]
        out.append([row.find(f"{_W}trPr/{_W}ins") is not None] + cells)
    return out


def _observed(key: str) -> list[list[tuple]]:
    return [[row["trPr"].startswith("<w:trPr><w:ins")]
            + [(c["vMerge"], c["text"], c["mark_inserted"]) for c in row["cells"]] for row in OBSERVED[key]]


def _tables(data: bytes) -> list[etree._Element]:
    root = etree.fromstring(zipfile.ZipFile(io.BytesIO(data)).read("word/document.xml"))
    return list(root.iter(_W + "tbl"))


@pytest.mark.parametrize("key", list(INSERTS))
def test_insert_row_beside_a_merge_down_is_what_word_makes(key):
    document = Document.open(probe.document())
    table = document.tables()[probe.TABLES.index(key)].id
    index, below = INSERTS[key]
    with document.tracking(author="Editor A") if key.startswith("Tracked") else contextlib.nullcontext():
        document.insert_row(table, index, below=below)
    assert _shape(_tables(document.to_bytes())[probe.TABLES.index(key)]) == _observed(key)


@pytest.mark.parametrize("key", ["BelowLast", "AboveFirst", "End"])
def test_a_row_beside_a_merge_says_so_with_the_merge_that_joins_it(toolbox, key):
    session = toolbox.session()
    d = session.open(probe.document(), "merged.docx")
    table = session.entry(d).document.tables()[probe.TABLES.index(key)].id
    index, below = INSERTS[key]
    result = _call(toolbox, session, "word_edit_table", doc=d, table=table, action="insert_row", index=index,
                          side="after" if below else "before")
    assert result.ok, result
    note = next(w for w in result.warnings if "merge down" in w)
    first, last = {"BelowLast": ((1, 0), (4, 0)), "AboveFirst": ((1, 0), (4, 0)), "End": ((2, 0), (5, 0))}[key]
    assert f"first {{row: {first[0]}, col: 0}} last {{row: {last[0]}, col: 0}}" in note
    # The call it names joins the row: one merge down over the group and the new row.
    merged = _call(toolbox, session, "word_edit_table", doc=d, table=table, action="merge",
                          first={"row": first[0], "col": 0}, last={"row": last[0], "col": 0})
    assert merged.ok, merged
    shape = _shape(_tables(session.entry(d).document.to_bytes())[probe.TABLES.index(key)])
    assert [row[1][0] for row in shape[first[0]:last[0] + 1]] == ["restart"] + ["continue"] * (last[0] - first[0])
    # Rule 3: the same call again changes nothing and is no error.
    again = _call(toolbox, session, "word_edit_table", doc=d, table=table, action="merge",
                  first={"row": first[0], "col": 0}, last={"row": last[0], "col": 0})
    assert again.ok and "nothing changed" in again.warnings[0]


def test_a_row_inside_a_merge_says_nothing_of_it(toolbox):
    session = toolbox.session()
    d = session.open(probe.document(), "merged.docx")
    table = session.entry(d).document.tables()[probe.TABLES.index("Inside")].id
    result = _call(toolbox, session, "word_edit_table", doc=d, table=table, action="insert_row", index=2)
    assert result.ok and not any("merge down" in w for w in result.warnings)


#: copy_row against Word's paste of a whole row (measured): ``(row copied, below)`` and the
#: Word case whose pasted row (index in its table) it must equal.
COPIES = {"first above": ((1, False), ("PasteAboveFirst", 1)), "last below": ((3, True), ("PasteAfterGroup", 4)),
          "end": ((4, True), ("PasteEnd", 5))}


@pytest.mark.parametrize("case", list(COPIES))
def test_copy_row_outside_a_merge_is_the_row_word_pastes_there(case):
    (index, below), (key, at) = COPIES[case]
    document = Document.open(probe.document())
    table = document.tables()[probe.TABLES.index("Inside")].id
    result = document.copy_row(table, index, below=below)
    rows = _shape(_tables(document.to_bytes())[probe.TABLES.index("Inside")])
    new = rows[index + 1 if below else index]
    word = _observed(key)[at]
    assert [c[0] for c in new[1:]] == [c[0] for c in word[1:]] == [None, None, None]
    assert [c[1].replace("Inside", key) for c in new[1:]] == [c[1] for c in word[1:]]
    assert any("merge down" in w for w in result.warnings) == (case != "end")


@pytest.mark.parametrize("tracked", [False, True])
@pytest.mark.parametrize("index, below", [(1, True), (2, True), (2, False), (3, False)])
def test_copy_row_inside_a_merge_joins_it(index, below, tracked):
    document = Document.open(probe.document())
    table = document.tables()[probe.TABLES.index("Inside")].id
    with document.tracking(author="Editor A") if tracked else contextlib.nullcontext():
        document.copy_row(table, index, below=below)
    rows = _shape(_tables(document.to_bytes())[probe.TABLES.index("Inside")])
    at = index + 1 if below else index
    assert [row[1][0] for row in rows] == [None, "restart", "continue", "continue", "continue", None]
    copied = _shape(_tables(probe.document())[probe.TABLES.index("Inside")])[index]
    assert rows[at][1][1] == "" and [c[1] for c in rows[at][2:]] == [c[1] for c in copied[2:]]
    # Tracked, as Word tracks a row it inserts there: the row and every mark inserted.
    assert rows[at][0] is tracked and all(c[2] is tracked for c in rows[at][1:])
    # Rule 6: the joined cell's paragraph is formatted as the row copied had it.
    table_element = _tables(document.to_bytes())[probe.TABLES.index("Inside")]
    spacing = table_element.findall(_W + "tr")[at].find(f"{_W}tc/{_W}p/{_W}pPr/{_W}spacing")
    assert spacing is not None and spacing.get(_W + "after") == "0"


def test_copy_row_is_one_undo_step_and_no_longer_refused(toolbox):
    session = toolbox.session(clock=CLOCK)
    d = session.open(probe.document(), "merged.docx")
    before = session.entry(d).document.to_bytes()
    table = session.entry(d).document.tables()[probe.TABLES.index("Inside")].id
    assert _call(toolbox, session, "word_set_tracking", doc=d, on=True, author="Editor A").ok
    result = _call(toolbox, session, "word_edit_table", doc=d, table=table, action="copy_row", index=2)
    assert result.ok, result
    assert _call(toolbox, session, "undo", doc=d).ok
    after = _shape(_tables(session.entry(d).document.to_bytes())[probe.TABLES.index("Inside")])
    assert after == _shape(_tables(before)[probe.TABLES.index("Inside")])


def test_copy_row_keeps_its_other_refusals_with_the_way_out(toolbox):
    document = Document.open(probe.document())
    table = document.tables()[probe.TABLES.index("Inside")].id
    document.insert_footnote(document.table(table).cell(2, 1).paragraphs[0].id, "A note.")
    session = toolbox.session(clock=CLOCK)
    d = session.open(document.to_bytes(), "merged.docx")
    table = session.entry(d).document.tables()[probe.TABLES.index("Inside")].id
    result = _call(toolbox, session, "word_edit_table", doc=d, table=table, action="copy_row", index=2)
    assert not result.ok and "footnote" in result.error.message
    assert [c["arguments"]["action"] for c in result.error.details["calls"]] == ["insert_row", "set_cells"]


# -- frames ------------------------------------------------------------------------------------

def merged_sample() -> tuple[Document, str]:
    """A made-up three-column table: a header row, body rows whose first column is merged down
    over two groups (rows 1-2, rows 3-5), a shaded third column."""
    document = Document.new()
    table = document.insert_table(6, 3, after=document.paragraphs()[0].id, style="Table Grid").id
    document.set_table(table, look={"first_row": True, "last_row": False, "first_column": False,
                                    "last_column": False, "banded_rows": False, "banded_columns": False})
    texts = [["Shelf", "Item", "Count"], ["Top", "jars", "4"], ["", "tins", "2"], ["Low", "bags", "7"],
             ["", "boxes", "1"], ["", "crates", "3"]]
    for r, row in enumerate(texts):
        for c, text in enumerate(row):
            if text:
                document.table(table).cell(r, c).paragraphs[0].set_text(text)
        document.format_cell(table, r, 2, shading="DEEAF6")
    document.set_row(table, 0, header=True)
    document.merge_cells(table, (1, 0), (2, 0))
    document.merge_cells(table, (3, 0), (5, 0))
    return Document.open(document.to_bytes()), table


def _merges(document: Document, table: str) -> list[str | None]:
    rows = document._resolve(table)[1].element.findall(_W + "tr")
    out = []
    for row in rows:
        merge = row.find(f"{_W}tc/{_W}tcPr/{_W}vMerge")
        out.append(None if merge is None else (merge.get(_W + "val") or "continue"))
    return out


@pytest.mark.parametrize("markdown, merges", [
    # As the sample: the same groups.
    ("| Top | jars | 4 |\n|  | tins | 2 |\n| Low | bags | 7 |\n|  | boxes | 1 |\n|  | crates | 3 |",
     [None, "restart", "continue", "restart", "continue", "continue"]),
    # More body rows: each group as long as its empty cells go -- rows Word adds inside a merge join it.
    ("| A | 1 | 1 |\n|  | 2 | 2 |\n|  | 3 | 3 |\n|  | 4 | 4 |\n| B | 5 | 5 |\n|  | 6 | 6 |\n| C | 7 | 7 |",
     [None, "restart", "continue", "continue", "continue", "restart", "continue", None]),
    # Fewer: a group of one is a cell of its own.
    ("| A | 1 | 1 |\n| B | 2 | 2 |\n|  | 3 | 3 |", [None, None, "restart", "continue"]),
    # Every cell given: no merges.
    ("| A | 1 | 1 |\n| B | 2 | 2 |", [None, None, None]),
])
def test_a_frame_copy_merges_its_body_down_as_the_text_says(markdown, merges):
    document, sample = merged_sample()
    head = "| Shelf | Item | Count |\n|---|---|---|\n"
    result = document.insert_markdown(head + markdown + "\n", at="end", table_like=sample)
    new = next(b for b in result.blocks if b.startswith("t:"))
    assert _merges(document, new) == merges
    # Each continuing cell is empty; the third column keeps the sample's shading.
    rows = document._resolve(new)[1].element.findall(_W + "tr")
    for row, merge in zip(rows, merges):
        if merge == "continue":
            assert not "".join(t.text or "" for t in row.find(_W + "tc").iter(_W + "t"))
        assert row.findall(_W + "tc")[2].find(f"{_W}tcPr/{_W}shd").get(_W + "fill") == "DEEAF6"
    if any(merges):
        assert any("merged down" in w and "column 0" in w for w in result.warnings)


@pytest.mark.any_faces
def test_a_frame_copy_with_merges_renders_like_the_sample():
    # The sample and its copy, one after the other on a page, drawn by docx2svg: the same
    # rules, shading and text, each relative to its table's top.
    import docx2svg

    document, sample = merged_sample()
    markdown = ("| Shelf | Item | Count |\n|---|---|---|\n| Top | jars | 4 |\n|  | tins | 2 |\n| Low | bags | 7 |\n"
                "|  | boxes | 1 |\n|  | crates | 3 |\n")
    document.insert_markdown(markdown, at="end", table_like=sample)
    svg = docx2svg.convert_docx_to_svg(document.to_bytes())[0]
    first, second = _drawn(svg, 1), _drawn(svg, 2)
    assert first and len(first) == len(second)
    for a, b in zip(first, second):   # within a pixel: the two tables' tops round differently
        assert (a[0], a[5], a[6]) == (b[0], b[5], b[6])
        numbers = lambda x: [*x[1], x[2], float(x[3] or 0), float(x[4] or 0)]  # noqa: E731
        assert all(abs(u - v) <= 1 for u, v in zip(numbers(a), numbers(b))), (a, b)
    # The merges drawn: no rule across the first column inside a group (rows 1-2, 3-5).
    assert sum(1 for kind, *_ in first if kind == "table-border") == 70


def _drawn(svg: str, table: int) -> list[tuple]:
    """What docx2svg draws for the body's ``table``-th table: each rule, shading and text,
    its position relative to the table's top left."""
    root = etree.fromstring(svg.encode())
    out = []
    for node in root.iter():
        path = node.get("data-docx-path") or ""
        if not path.startswith(f"w:body/w:tbl[{table}]/") or node.get("y") is None:
            continue
        kind = node.get("data-docx-kind") or etree.QName(node).localname
        xs = [float(v) for v in node.get("x").split()]
        out.append((kind, xs, float(node.get("y")), node.get("width"), node.get("height"), node.get("fill"),
                    "".join(node.itertext())))
    if not out:
        return out
    left, top = min(x[1][0] for x in out), min(x[2] for x in out)
    return sorted((k, [round(v - left, 2) for v in xs], round(y - top, 2), w, h, f, t)
                  for k, xs, y, w, h, f, t in out)


def test_a_frame_copy_keeps_the_refusals_with_the_way_out(toolbox):
    document, sample = merged_sample()
    head = "| Shelf | Item | Count |\n|---|---|---|\n"
    with pytest.raises(EditError, match="2 column"):
        document.insert_markdown("| Shelf | Item |\n|---|---|\n| A | 1 |\n", at="end", table_like=sample)
    # Text in a header row's cell merged into the one above (the sample's two header rows,
    # their second column merged down) is refused: the text goes in the merge's first cell.
    header, table = merged_sample()
    header.set_row(table, 1, header=True)
    header.merge_cells(table, (0, 1), (1, 1))
    with pytest.raises(EditError, match="merged into the cell above"):
        header.insert_markdown(head + "| A | 1 | 1 |\n| B | 2 | 2 |\n", at="end", table_like=table)
    made = header.insert_markdown(head + "| A |  | 1 |\n| B | 2 | 2 |\n", at="end", table_like=table)
    new = next(b for b in made.blocks if b.startswith("t:"))
    rows = header._resolve(new)[1].element.findall(_W + "tr")
    assert rows[1].findall(_W + "tc")[1].find(f"{_W}tcPr/{_W}vMerge") is not None


def test_the_tool_descriptions_say_the_rules():
    from docx_agent.tools import TOOLS as tools

    by_name = {tool.name: tool for tool in tools}
    like = json.dumps(by_name["word_insert_markdown"].schema)
    assert "empty cells joining merges" in like
