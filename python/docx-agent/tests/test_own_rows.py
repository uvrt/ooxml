"""Round 51 (R103): a tracked deletion of a row or table that is the author's own pending
insertion removes it outright, as Word does (the rows probe, :mod:`rows_cases`,
``crates/ooxml-oracle/word/observations/rows-word.json``):

* a row wholly one's own insertion (its mark, its cells' marks, every run) goes, no record
  left -- deleted by ``delete_row``, a table's deletion, a ``sync`` removal;
* another author's inserted row is marked deleted beside their insertion, each cell's mark
  too;
* one's own row holding another author's insertion goes, and what was theirs stays, deleted,
  at the start of the next paragraph; one's own row holding what was there before keeps its
  insertion, its content and marks deleted;
* a table goes with its last row.

The same edits Word made in the probe are made here on the same made-up document, and the
two compared shape for shape (records by author, text); Word's Accept All and Reject All of
its result are compared with ours."""

from __future__ import annotations

import datetime as dt
import json
import re
from pathlib import Path

import pytest
from lxml import etree
from ooxml_edit.tools import Dataset, Result, Toolbox, string, tool

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

ROOT = Path(__file__).resolve().parent.parent
import rows_cases as rows_probe  # noqa: E402
from conftest import OBSERVATIONS  # noqa: E402

OBSERVED = json.loads((OBSERVATIONS / "rows-word.json").read_text(encoding="utf-8"))
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
ME, OTHER = "Writer", rows_probe.OTHER
CLOCK = lambda: dt.datetime(2026, 11, 11, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
_NS = (f'xmlns:w="{W}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" '
       'xmlns:w16du="http://schemas.microsoft.com/office/word/2023/wordml/word16du"')


# -- shapes --------------------------------------------------------------------------------------


def _who(element) -> str:
    author = element.get(_W + "author")
    return "me" if author in ("{author}", ME) else "other"


def _records(properties) -> tuple:
    if properties is None:
        return ()
    return tuple(f"{c.tag[len(_W):]}:{_who(c)}" for c in properties if c.tag in (_W + "ins", _W + "del"))


def _paragraph(p) -> tuple:
    runs = []
    for run in p.iter(_W + "r"):
        chain, node = [], run.getparent()
        while node is not p:
            if node.tag in (_W + "ins", _W + "del"):
                chain.append(f"{node.tag[len(_W):]}:{_who(node)}")
            node = node.getparent()
        text = "".join(x.text or "" for x in run if x.tag in (_W + "t", _W + "delText"))
        if text:
            runs.append((">".join(reversed(chain)), text))
    return _records(p.find(f"{_W}pPr/{_W}rPr")), tuple(runs)


def _block(element) -> tuple:
    if element.tag == _W + "tbl":
        return ("table", tuple((_records(row.find(_W + "trPr")), tuple(_paragraph(p) for p in row.iter(_W + "p")))
                               for row in element.iter(_W + "tr")))
    return ("p", _paragraph(element))


def _observed(name: str) -> list[tuple]:
    out = []
    for block in OBSERVED[name]:
        element = etree.fromstring(re.sub(r"^<(w:\w+)", r"<\1 " + _NS, block, count=1))
        if element.tag != _W + "sectPr":
            out.append(_block(element))
    return out


def _ours(session, doc: str = "d1") -> list[tuple]:
    body = word_docs.tree(word_docs.saved(session, doc)).find(_W + "body")
    return [_block(child) for child in body if child.tag != _W + "sectPr"]


# -- Word's edits, made here ---------------------------------------------------------------------


@pytest.fixture(scope="module")
def box():
    with Toolbox([*TOOLS, rows_keep], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def ok(box, session, name, /, **arguments):
    result = box.dispatch(session, name, arguments)
    assert result.ok, (name, result.to_json())
    return result


def _table_ids(session, doc="d1") -> list[str]:
    return [f"t:{word_docs.rows_of(t)[0].get(word_docs.W14_PARA)}" for t in word_docs.tables(session, doc)]


def _cell_paragraph(session, table, row, column, doc="d1") -> str:
    cell = word_docs.cells_of(word_docs.rows_of(word_docs.element(session, doc, table))[row])[column]
    return f"p:{cell.find(_W + 'p').get(word_docs.W14_PARA)}"


def _made_as_word_made_them(box):
    """The probe's document, and the probe's edits (once ``crates/ooxml-oracle/word/probes/rows_probe.py``'s script),
    tracked by the probe's Word user, made through the tools: the session."""
    session = box.session(clock=CLOCK)
    session.open(rows_probe.document(ME), "rows.docx")

    def edit(tool, **arguments):
        ok(box, session, tool, doc="d1", **arguments)

    tables = dict(zip([k for k in rows_probe.TABLES if k != "TableLive"], _table_ids(session)))
    edit("word_set_tracking", on=True, author=ME)
    here = next(block for block, text in word_docs.paragraphs(session, "d1") if text == "Here")
    made = ok(box, session, "word_insert_markdown", doc="d1", at=f"before:{here}",
              markdown="| x | y |\n|---|---|\n| TableLive typed | |")
    edit("word_delete", target=next(i for i in made.created if i.startswith("t:")))
    edit("word_delete", target=tables["TableMixed"])
    edit("word_delete", target=tables["TableMine"])
    second = _cell_paragraph(session, tables["Typed"], 1, 1)
    edit("word_set_text", target=second, text="Typed r2c2 TYPEDTWO")
    edit("word_set_text", target=second, text="")
    first = _cell_paragraph(session, tables["Typed"], 1, 0)
    edit("word_set_text", target=first, text="Typed r2c1 TYPEDONE")
    edit("word_set_text", target=first, text="Typed r2c1")

    def delete_row(key, index):
        edit("word_edit_table", table=tables[key], action="delete_row", index=index)

    def insert_row(key, index):
        edit("word_edit_table", table=tables[key], action="insert_row", index=index)

    delete_row("Two", 2)
    delete_row("Two", 1)
    for key in ("MinePlain", "OtherMineText"):
        delete_row(key, 2)
    delete_row("MineOtherMid", 1)
    for key in ("MineOtherText", "Other", "Mine"):
        delete_row(key, 2)
    insert_row("LiveClear", 1)
    cell = _cell_paragraph(session, tables["LiveClear"], 2, 0)
    edit("word_set_text", target=cell, text="CLEARED")
    edit("word_set_text", target=cell, text="")
    insert_row("LiveEmpty", 1)
    delete_row("LiveEmpty", 2)
    insert_row("Live", 1)
    edit("word_set_text", target=_cell_paragraph(session, tables["Live"], 2, 0), text="LIVENEW")
    delete_row("Live", 2)
    return session


def test_word_removes_ones_own_pending_row_and_table_outright():
    """The measured facts this follows (Word 16.106 for Mac)."""
    tracked = _observed("tracked")
    tables = [b for b in tracked if b[0] == "table"]
    live, empty, clear, mine, other, mine_other, mine_mid, other_mine, plain, two, typed, mixed = tables
    for table in (live, empty, mine):             # inserted and deleted: no record left
        assert all(row[0] == () for row in table[1])
    assert len(live[1]) == len(empty[1]) == 3 and len(mine[1]) == 2
    assert other[1][2][0] == ("ins:other", "del:me")           # another's row: marked deleted
    assert other[1][2][1][0] == (("ins:other", "del:me"), (("ins:other>del:me", "Other r3c1"),))
    assert other_mine[1][2][1][0] == (("ins:other", "del:me"), ())  # one's own text in it: gone
    assert plain[1][2][0] == ("ins:me",)                       # what was there before: deleted
    assert plain[1][2][1][0] == (("del:me",), (("del:me", "MinePlain r3c1"),))
    assert len(mine_other[1]) == 2 and len(mine_mid[1]) == 2   # one's own row holding another's
    after = tracked[tracked.index(mine_other) + 1]
    assert after == ("p", ((), (("ins:other>del:me", "MineOtherText r3c1"), ("", "After MineOtherText."))))
    assert mine_mid[1][1][1][0][1][0] == ("ins:other>del:me", "MineOtherMid r2c1")
    assert [row[0] for row in two[1]] == [(), ("del:me",)]     # own row gone, the other marked
    assert [row[0] for row in mixed[1]] == [("ins:other", "del:me")]
    assert not any(b[0] == "table" and "TableMine" in str(b) for b in tracked)
    assert typed[1][1][1] == (((), (("", "Typed r2c1"),)), ((), (("del:me", "Typed r2c2"),)))


def test_the_probes_edits_made_here_give_words_result(box):
    session = _made_as_word_made_them(box)
    assert _ours(session) == _observed("tracked")
    assert word_docs.validation(box, session, "d1")["new"] == []


@pytest.mark.parametrize("review", ["accept", "reject"])
def test_accepting_or_rejecting_gives_words_review(box, review):
    session = _made_as_word_made_them(box)
    ok(box, session, "word_changes", doc="d1", action=review, all=True)
    assert _ours(session) == _observed(review)


# -- through the tools ---------------------------------------------------------------------------


SETS: dict[str, list[tuple]] = {}


@tool("rows_keep", "Keep a made-up dataset.", {"id": string("Its id.")}, documents=())
def rows_keep(call, id):
    call.define_dataset(id, Dataset(("code", "title"), SETS[id]), source=None)
    return Result(summary=f"Kept d:{id}")


def _opened(box, rows: list[tuple]):
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "plan.docx")
    first = word_docs.paragraph_ids(box, session, "d1")[0]
    ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{first}",
       markdown="| code | title |\n|---|---|\n" + "\n".join(f"| {a} | {b} |" for a, b in rows))
    ok(box, session, "word_insert_markdown", doc="d1", at="end", markdown="Middle.\n\nTail.")
    return session, _table_ids(session)[-1]


def _rows(session, table) -> tuple[int, int]:
    """The table's rows in the document, and in the final view (less those deleted)."""
    rows = word_docs.rows_of(word_docs.element(session, "d1", table))
    return len(rows), sum(1 for row in rows if row.find(f"{_W}trPr/{_W}del") is None)


def _xml(session) -> bytes:
    return etree.tostring(word_docs.tree(word_docs.saved(session, "d1")))


def test_a_row_inserted_then_deleted_leaves_no_change(box):
    """The reported case: tracking on, a 3-row table, a row inserted at 2, row 3 deleted."""
    session, table = _opened(box, [("A1", "one"), ("A2", "two")])
    before = _xml(session)
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="insert_row", index=2)
    deleted = ok(box, session, "word_edit_table", doc="d1", table=table, action="delete_row", index=3)
    table = _table_ids(session)[-1]
    assert _rows(session, table) == (3, 3)
    assert ok(box, session, "word_changes", doc="d1", action="list").data == []
    assert _xml(session) == before                    # as it was: no trace of either edit
    assert any(i.startswith("tr:") for i in deleted.removed)


def test_a_row_with_text_typed_into_it_goes_too(box):
    session, table = _opened(box, [("A1", "one"), ("A2", "two")])
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="insert_row", index=1)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
       cells=[{"row": 2, "col": 0, "text": "A9"}, {"row": 2, "col": 1, "text": "nine"}])
    ok(box, session, "word_edit_table", doc="d1", table=table, action="delete_row", index=2)
    assert _rows(session, table) == (3, 3)
    assert word_docs.revisions(session, "d1") == []


def test_sync_removing_rows_it_added_earlier_leaves_final_rows(box):
    """Rows a sync added, removed by a later sync in the same pending session: the document's
    rows are the final view's; a row there before is marked deleted (it stays until
    accepted)."""
    base = [(f"K{i:03d}", f"Item {i}") for i in range(1, 21)]
    session, table = _opened(box, base)
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    grown = base + [(f"K{i:03d}", f"Item {i}") for i in range(21, 61)]
    SETS["grown"] = grown
    ok(box, session, "rows_keep", id="grown")
    ok(box, session, "word_edit_table", doc="d1", table=table, action="sync", **{"from": "d:grown", "remove": True})
    table = _table_ids(session)[-1]
    assert _rows(session, table) == (61, 61)
    kept = [r for r in grown if not 30 <= int(r[0][1:]) < 45]
    SETS["kept"] = kept
    ok(box, session, "rows_keep", id="kept")
    result = ok(box, session, "word_edit_table", doc="d1", table=table, action="sync",
                **{"from": "d:kept", "remove": True})
    table = _table_ids(session)[-1]
    assert _rows(session, table) == (len(kept) + 1, len(kept) + 1)
    assert result.data["rows"] == len(kept) + 1
    assert len([i for i in result.removed if i.startswith("tr:")]) == 15
    # One that was there before: marked, as before.
    SETS["less"] = [r for r in kept if r[0] != "K003"]
    ok(box, session, "rows_keep", id="less")
    ok(box, session, "word_edit_table", doc="d1", table=table, action="sync", **{"from": "d:less", "remove": True})
    table = _table_ids(session)[-1]
    assert _rows(session, table) == (len(kept) + 1, len(kept))


def test_another_authors_pending_row_is_marked_deleted_beside_their_insertion(box):
    session, table = _opened(box, [("A1", "one"), ("A2", "two")])
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=OTHER)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="insert_row", index=2)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
       cells=[{"row": 3, "col": 0, "text": "A3"}])
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="delete_row", index=3)
    assert _rows(session, table) == (4, 3)
    row = word_docs.rows_of(word_docs.element(session, "d1", table))[3]
    assert _records(row.find(_W + "trPr")) == ("ins:other", "del:me")
    assert [_paragraph(p) for p in row.iter(_W + "p")] == [
        (("ins:other", "del:me"), (("ins:other>del:me", "A3"),)), (("ins:other", "del:me"), ())]


def test_a_whole_table_one_inserted_goes_with_its_rows(box):
    session, _table = _opened(box, [("A1", "one")])
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    last = word_docs.paragraph_ids(box, session, "d1")[-1]
    before = _xml(session)
    made = ok(box, session, "word_insert_markdown", doc="d1", at=f"before:{last}",
              markdown="| new | table |\n|---|---|\n| B1 | b |")
    new = next(i for i in made.created if i.startswith("t:"))
    deleted = ok(box, session, "word_delete", doc="d1", target=new)
    assert new in deleted.removed and len(_table_ids(session)) == 1
    assert _xml(session) == before and ok(box, session, "word_changes", doc="d1", action="list").data == []


def test_a_table_with_another_authors_row_keeps_that_row_marked(box):
    """Word's TableMixed: one's own rows go, another author's is marked deleted, the table
    stays with it."""
    session, _table = _opened(box, [("A1", "one")])
    last = word_docs.paragraph_ids(box, session, "d1")[-1]
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    made = ok(box, session, "word_insert_markdown", doc="d1", at=f"before:{last}",
              markdown="| new | table |\n|---|---|\n| B1 | b |\n| B2 | c |")
    new = next(i for i in made.created if i.startswith("t:"))
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=OTHER)
    ok(box, session, "word_edit_table", doc="d1", table=new, action="insert_row", index=1)
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    deleted = ok(box, session, "word_delete", doc="d1", target=new)
    new = deleted.renamed.get(new) or _table_ids(session)[-1]       # its first row gone: named by the next
    assert _rows(session, new) == (1, 0)
    assert _records(word_docs.rows_of(word_docs.element(session, "d1", new))[0].find(_W + "trPr")) == (
        "ins:other", "del:me")
    assert len([i for i in deleted.removed if i.startswith("tr:")]) == 3


def test_a_control_deleted_with_its_table_deletes_the_table_tracked(box):
    """Before round 51 a table inside a block control deleted with it, tracked, was left as
    it was: its rows are now deleted as Word deletes rows -- one's own pending row outright."""
    table = rows_probe._table([rows_probe._row("Boxed", 1), rows_probe._row("Boxed", 2)])
    data = rows_probe._package(rows_probe._paragraph("Before.") + '<w:sdt><w:sdtPr><w:id w:val="4711"/></w:sdtPr>'
                               f"<w:sdtContent>{table}</w:sdtContent></w:sdt>" + rows_probe._paragraph("After."))
    session = box.session(clock=CLOCK)
    session.open(data, "boxed.docx")
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    ok(box, session, "word_edit_table", doc="d1", table=_table_ids(session)[-1], action="insert_row", index=1)
    ok(box, session, "word_delete", doc="d1", target="cc:4711")
    rows = list(word_docs.tree(word_docs.saved(session, "d1")).iter(_W + "tr"))
    assert len(rows) == 2 and all(_records(row.find(_W + "trPr")) == ("del:me",) for row in rows)
    assert all(_paragraph(p)[0] == ("del:me",) for row in rows for p in row.iter(_W + "p"))
    assert word_docs.validation(box, session, "d1")["new"] == []


# -- the paragraph a table came with (round 51) ------------------------------------------------

SEPARATOR = json.loads((OBSERVATIONS / "separator-word.json").read_text(encoding="utf-8"))


def _separator_observed(name: str) -> list[tuple]:
    out = []
    for block in SEPARATOR[name]:
        element = etree.fromstring(re.sub(r"^<(w:\w+)", r"<\1 " + _NS, block, count=1))
        if element.tag != _W + "sectPr":
            out.append(_block(element))
    return out


def test_word_puts_a_new_table_beside_a_table_and_deleting_it_leaves_no_trace():
    """The measured facts (the separator probe, once ``crates/ooxml-oracle/word/probes/separator_probe.py``): Word inserts a table next to a table
    with no paragraph between, and deleting it leaves the document as it was; a table
    inserted at the end of a paragraph's text splits it, and the split stays."""
    made, deleted = _separator_observed("made"), _separator_observed("deleted")
    kinds = [b[0] for b in made]
    assert kinds[1:3] == ["table", "table"] and kinds[-3:] == ["table", "table", "p"]
    split = next(k for k, b in enumerate(deleted) if b[0] == "p" and b[1][1] == (("", "Last paragraph."),))
    assert deleted[split][1][0] == ("ins:me",) and deleted[split + 1] == ("p", ((), ()))
    assert [b[0] for b in deleted] == ["p", "table", "p", "table", "p", "p", "p", "table", "p"]


@pytest.mark.parametrize("where", ["between", "end"])
def test_a_table_one_inserted_beside_a_table_goes_with_the_paragraph_it_came_with(box, where):
    """docx-agent keeps two tables apart with an empty paragraph (inserted, tracked); the
    table deleted, that paragraph goes too -- the document as it was, as Word leaves it."""
    session = box.session(clock=CLOCK)
    session.open(rows_probe._package(
        rows_probe._paragraph("Start.") + rows_probe._table([rows_probe._row("One", 1)])
        + rows_probe._paragraph() + rows_probe._table([rows_probe._row("Two", 1)]) + rows_probe._paragraph()),
        "separated.docx")

    def body():
        found = word_docs.tree(word_docs.saved(session, "d1")).find(_W + "body")
        return [re.sub(r' xmlns:\w+="[^"]*"', "", etree.tostring(child).decode()) for child in found]

    before = body()
    after = word_docs.element(session, "d1", _table_ids(session)[0 if where == "between" else 1]).getnext()
    place = f"p:{after.get(word_docs.W14_PARA)}"
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    made = ok(box, session, "word_insert_markdown", doc="d1", at=f"before:{place}",
              markdown="| new | |\n|---|---|\n| | |")
    new = next(i for i in made.created if i.startswith("t:"))
    assert sum(1 for b in _ours(session) if b == ("p", (("ins:me",), ()))) == 1   # the separator
    result = ok(box, session, "word_delete", doc="d1", target=new)
    assert body() == before      # (the root keeps the w16du declaration the tracking added)
    assert len(result.removed) == len(set(result.removed)) and new in result.removed


def test_a_paragraph_still_keeping_two_tables_apart_stays(box):
    session = box.session(clock=CLOCK)
    session.open(rows_probe._package(
        rows_probe._paragraph("Start.") + rows_probe._table([rows_probe._row("One", 1)]) + rows_probe._paragraph()),
        "separated.docx")
    last = word_docs.paragraph_ids(box, session, "d1")[-1]
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    first = next(i for i in ok(box, session, "word_insert_markdown", doc="d1", at=f"before:{last}",
                               markdown="| a | |\n|---|---|\n| | |").created if i.startswith("t:"))
    ok(box, session, "word_insert_markdown", doc="d1", at=f"before:{last}", markdown="| b | |\n|---|---|\n| | |")
    ok(box, session, "word_delete", doc="d1", target=first)
    blocks = [b[0] for b in _ours(session)]
    assert "table" in blocks and all(not (a == b == "table") for a, b in zip(blocks, blocks[1:]))
    assert word_docs.validation(box, session, "d1")["new"] == []
