"""Round 55 (R116, R119): ``word_changes`` list gives one change per edit as it was made, and
a sample document with the shape ``accept_loses_text`` names ships with the package.
Made-up documents only.

* R116: a tracked column deletion, a merge and a cell's new text, made by one author in one
  minute, were one listed "replacement" -- every record in a table joined every structure
  record there.  Now each edit is its own entry, with its ids and what it was
  (``structure``: column deletion, column insertion, merge, split, row deletion, row
  insertion), and accepting or rejecting one leaves the others; a row or column inserted or
  deleted whole, a merge, a split stay one entry each.
* Two records sharing one ``w:id`` are listed ``rev:95`` and ``rev:95#1``; accepting several
  ids in one call resolves them all first (accepting the first had renamed the second, which
  was then refused), and an id that is not there is refused with the ids that are.
* R119: the sample docx-agent shipped (``accept_loses_text.docx``, its body inlined here):
  check names its two cells, accept moves their text after the row, reject keeps it.

Tool-design rules: 2 (a refused id comes with the listed ids and the list call), 5 (several
ids, one call, one undo step), 7 (the sample shows check's fact).
"""

from __future__ import annotations

import datetime as dt
import itertools

import re

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
ME = "Editor A"
CLOCK = lambda: dt.datetime(2026, 11, 13, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
ROWS = [("Area", "Lead", "Phase", "Remark"), ("Harbour", "Ines", "draft", "awaits figures"),
        ("Orchard", "Tomas", "done", "signed off"), ("Quarry", "Aleid", "open", "two gaps")]
MARKDOWN = "\n".join(["| " + " | ".join(ROWS[0]) + " |", "|---|---|---|---|"]
                     + ["| " + " | ".join(r) + " |" for r in ROWS[1:]])
EDITS = {
    "delete_column": dict(action="delete_column", index=2),
    "set_cells": dict(action="set_cells", cells=[{"row": 3, "col": 0, "text": "Quarry North"}]),
    "merge": dict(action="merge", first={"row": 1, "col": 0}, last={"row": 1, "col": 1}),
    "merge_down": dict(action="merge", first={"row": 1, "col": 0}, last={"row": 2, "col": 0}),
    "split": dict(action="split", first={"row": 2, "col": 1}),
    "split_rows": dict(action="split", first={"row": 2, "col": 1}, rows=2, columns=1),
    "insert_column": dict(action="insert_column", index=1),
    "insert_row": dict(action="insert_row", index=1),
    "delete_row": dict(action="delete_row", index=2),
    "first_cell": dict(action="set_cells", cells=[{"row": 1, "col": 0, "text": "Harbour East"}]),
}


@pytest.fixture(scope="module")
def box():
    with Toolbox(list(TOOLS), formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def call(box, session, name, /, **arguments):
    return box.dispatch(session, name, arguments)


def ok(box, session, name, /, **arguments):
    result = call(box, session, name, **arguments)
    assert result.ok, (name, result.to_json())
    return result


def _first(session) -> str:
    return word_docs.paragraphs(session, "d1")[0][0]


def _edited(box, *edits: str):
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "report.docx")
    made = ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{_first(session)}",
              markdown=f"Field report.\n\n{MARKDOWN}\n\nAfter the table.")
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    table = [block for block in made.data["blocks"] if block.startswith("t:")][-1]
    for edit in edits:
        ok(box, session, "word_edit_table", doc="d1", table=table, **EDITS[edit])
    return session


def _listed(box, session) -> list[dict]:
    return ok(box, session, "word_changes", doc="d1", action="list").data


def _shape(listed: list[dict]) -> list[tuple]:
    return sorted((c["kind"], c.get("structure"), c.get("old_text", ""), c.get("text", "")) for c in listed)


# -- R116: one change per edit ---------------------------------------------------------------------


def test_a_column_deletion_and_a_cells_text_are_two_changes(box):
    listed = _listed(box, _edited(box, "delete_column", "set_cells"))
    assert _shape(listed) == [("deletion", "column deletion", "Phase draft done open", ""),
                              ("insertion", None, "", "North")]


@pytest.mark.parametrize("order", [("merge", "set_cells", "delete_column"), ("set_cells", "merge", "delete_column"),
                                   ("merge", "delete_column", "set_cells")])  # (a merge after it: lane C)
def test_a_merge_a_column_deletion_and_a_cells_text_are_three_changes(box, order):
    listed = _listed(box, _edited(box, *order))
    assert _shape(listed) == [("deletion", "column deletion", "Phase draft done open", ""),
                              ("insertion", None, "", "North"),
                              ("table", "merge", "", "Ines")]
    assert len({r for c in listed for r in c.get("records", [c["id"]])}) == sum(
        len(c.get("records", [c["id"]])) for c in listed)                    # no record in two


@pytest.mark.parametrize("edit, kind, structure", [
    ("delete_column", "deletion", "column deletion"), ("insert_column", "insertion", "column insertion"),
    ("merge", "table", "merge"), ("merge_down", "table", "merge"), ("split", "table", "split"),
    ("split_rows", "table", "split"), ("insert_row", "insertion", "row insertion"),
    ("delete_row", "deletion", "row deletion")])
def test_each_structure_edit_is_one_change(box, edit, kind, structure):
    listed = _listed(box, _edited(box, edit))
    assert [(c["kind"], c.get("structure")) for c in listed] == [(kind, structure)]


@pytest.mark.parametrize("edit", ["merge", "merge_down", "split", "insert_column", "delete_row"])
def test_a_cells_new_text_beside_a_structure_edit_is_its_own_change(box, edit):
    # set_cells on row 1's first cell before the edit: a cell the merges take text into.
    listed = _listed(box, _edited(box, "first_cell", edit))
    assert len(listed) == 2 and len([c for c in listed if c.get("structure")]) == 1
    text = next(c for c in listed if not c.get("structure"))
    assert "East" in text.get("text", "") and "Ines" not in text.get("text", "")


def test_each_change_is_accepted_alone_and_in_any_order_gives_accept_all(box):
    edits = ("merge", "set_cells", "delete_column")
    all_at_once = _edited(box, *edits)
    ok(box, all_at_once, "word_changes", doc="d1", action="accept", all=True)
    expected = ok(box, all_at_once, "word_read", doc="d1").data
    for k in range(len(edits)):
        order = list(itertools.permutations(range(3)))[k * 2]
        session = _edited(box, *edits)
        ids = [c["id"] for c in _listed(box, session)]
        for step, position in enumerate(order):
            result = ok(box, session, "word_changes", doc="d1", action="accept", ids=[ids[position]])
            assert result.to_json()["checks"]["validate"]["new"] == []            # a valid file each time
            assert len(_listed(box, session)) == 2 - step                    # one gone, the rest kept
        assert ok(box, session, "word_read", doc="d1").data == expected


def test_rejecting_one_change_keeps_the_others(box):
    session = _edited(box, "merge", "set_cells", "delete_column")
    listed = _listed(box, session)
    column = next(c for c in listed if c.get("structure") == "column deletion")
    ok(box, session, "word_changes", doc="d1", action="reject", ids=[column["id"]])
    left = _listed(box, session)
    assert sorted(c.get("structure") or c["kind"] for c in left) == ["insertion", "merge"]
    read = ok(box, session, "word_read", doc="d1", view="original").data
    assert "Phase" in read and "| Area | Lead | Phase | Remark |" in read


# -- two records, one w:id -----------------------------------------------------------------------------


def _shared_ids(box):
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "notes.docx")
    ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{_first(session)}",
       markdown="Alpha one.\n\nBravo two.")
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    ok(box, session, "replace_text", doc="d1", find="one", replace="uno")
    ok(box, session, "replace_text", doc="d1", find="two", replace="dos")
    # (The ids rewritten on a document of its bytes: the session's own is its to change.)
    data = word_docs.replaced(word_docs.saved(session, "d1"), {"word/document.xml": lambda xml: re.sub(
        r'(<w:(?:ins|del) w:id=")\d+"', r'\g<1>95"', xml)})
    again = box.session(clock=CLOCK)
    again.open(data, "notes.docx")
    return again


@pytest.mark.parametrize("ids", [["rev:95", "rev:95#2"], ["rev:95#2", "rev:95"], ["rev:95#1", "rev:95#3"]])
def test_ids_sharing_one_w_id_are_accepted_together(box, ids):
    session = _shared_ids(box)
    assert [c["id"] for c in _listed(box, session)] == ["rev:95", "rev:95#2"]
    result = ok(box, session, "word_changes", doc="d1", action="accept", ids=ids)
    assert result.data["records"] == (4 if "rev:95" in ids else 2)
    read = ok(box, session, "word_read", doc="d1", view="markup").data
    assert "uno" in read and "dos" in read and "{++" not in read


def test_an_id_not_there_is_refused_with_the_ids_that_are(box):
    session = _shared_ids(box)
    before = word_docs.saved(session, "d1")
    refused = call(box, session, "word_changes", doc="d1", action="accept", ids=["rev:95", "rev:96"])
    assert not refused.ok
    error = refused.to_json()["error"]
    assert error["code"] == "not_found" and "'rev:96'" in error["message"]
    assert error["valid_options"] == ["rev:95", "rev:95#2"]
    # the way out: the call that lists the ids (the session's try_call, tool rule 2; docx-agent's
    # own handler names it in details.call)
    way_out = error.get("try_call") or error["details"]["call"]
    assert {key: way_out[key] for key in ("tool", "arguments")} == \
        {"tool": "word_changes", "arguments": {"doc": "d1", "action": "list"}}
    assert word_docs.saved(session, "d1") == before            # nothing accepted


# -- R119: the sample ---------------------------------------------------------------------------------


#: The body of the sample docx-agent shipped (``docx_agent/samples/accept_loses_text.docx``): a
#: row's last cell, and a table's last cell, whose last paragraph's mark is a pending tracked
#: deletion by Editor B.
SAMPLE_BODY = (
    '<w:p w14:paraId="1A550001" w14:textId="2B660001"><w:r>'
    '<w:t xml:space="preserve">A row\'s last cell whose last paragraph mark is a pending tracked deletion:</w:t>'
    '</w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>'
    '<w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr><w:tblGrid>'
    '<w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
    '<w:tr w14:paraId="3C773850" w14:textId="4D883850"><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/>'
    '</w:tcPr><w:p w14:paraId="1A550002" w14:textId="2B660002"><w:r><w:t xml:space="preserve">r1c1</w:t>'
    '</w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>'
    '<w:p w14:paraId="1A550006" w14:textId="2B660006"><w:r><w:t xml:space="preserve">Kept.</w:t></w:r>'
    '</w:p><w:p w14:paraId="1A550007" w14:textId="2B660007"><w:pPr><w:rPr>'
    '<w:del w:id="901" w:author="Editor B" w:date="2026-11-13T09:00:00Z"/></w:rPr></w:pPr><w:r>'
    '<w:t xml:space="preserve">Last.</w:t></w:r></w:p></w:tc></w:tr>'
    '<w:tr w14:paraId="3C773851" w14:textId="4D883851"><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/>'
    '</w:tcPr><w:p w14:paraId="1A550004" w14:textId="2B660004"><w:r><w:t xml:space="preserve">r2c1</w:t>'
    '</w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>'
    '<w:p w14:paraId="1A550005" w14:textId="2B660005"><w:r><w:t xml:space="preserve">r2c2</w:t></w:r>'
    '</w:p></w:tc></w:tr></w:tbl><w:p w14:paraId="1A550008" w14:textId="2B660008"><w:r>'
    '<w:t xml:space="preserve">The table\'s last cell, the same:</w:t></w:r></w:p><w:tbl><w:tblPr>'
    '<w:tblW w:w="0" w:type="auto"/><w:tblBorders>'
    '<w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
    '<w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr><w:tblGrid>'
    '<w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
    '<w:tr w14:paraId="3C773860" w14:textId="4D883860"><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/>'
    '</w:tcPr><w:p w14:paraId="1A550009" w14:textId="2B660009"><w:r><w:t xml:space="preserve">r1c1</w:t>'
    '</w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>'
    '<w:p w14:paraId="1A55000A" w14:textId="2B66000A"><w:r><w:t xml:space="preserve">r1c2</w:t></w:r>'
    '</w:p></w:tc></w:tr><w:tr w14:paraId="3C773861" w14:textId="4D883861"><w:tc><w:tcPr>'
    '<w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p w14:paraId="1A55000B" w14:textId="2B66000B"><w:r>'
    '<w:t xml:space="preserve">r2c1</w:t></w:r></w:p></w:tc><w:tc><w:tcPr>'
    '<w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p w14:paraId="1A55000D" w14:textId="2B66000D"><w:r>'
    '<w:t xml:space="preserve">Kept.</w:t></w:r></w:p><w:p w14:paraId="1A55000E" w14:textId="2B66000E">'
    '<w:pPr><w:rPr><w:del w:id="902" w:author="Editor B" w:date="2026-11-13T09:00:00Z"/></w:rPr></w:pPr>'
    '<w:r><w:t xml:space="preserve">Last.</w:t></w:r></w:p></w:tc></w:tr></w:tbl>'
    '<w:p w14:paraId="1A55000F" w14:textId="2B66000F"><w:r><w:t xml:space="preserve">End.</w:t></w:r>'
    '</w:p>')


def _loses_text(box, session) -> list[dict]:
    return ok(box, session, "check", doc="d1", include=["layout"]).data["layout"].get("accept_loses_text", [])


def test_check_names_the_samples_two_cells(box):
    session = box.session(clock=CLOCK)
    session.open(word_docs.with_body(SAMPLE_BODY), "sample.docx")
    facts = _loses_text(box, session)
    assert [f["fix"]["calls"][0] for f in facts] == [{"action": "reject", "ids": ["rev:901"]},
                                                     {"action": "reject", "ids": ["rev:902"]}]
    cells = [f["cell"] for f in facts]
    assert [cell.rsplit("/", 1)[1] for cell in cells] == ["c0,1", "c1,1"]
    # The table ids are stable (rows carry paraIds), so the facts name them across calls.
    assert all(cell.startswith("t:") for cell in cells)


def test_check_accept_and_reject_on_the_sample(box):
    session = box.session(clock=CLOCK)
    session.open(word_docs.with_body(SAMPLE_BODY), "sample.docx")
    accepted = ok(box, session, "word_changes", doc="d1", action="accept", all=True)
    assert len([w for w in accepted.warnings if "now after the row" in w]) == 2
    read = ok(box, session, "word_read", doc="d1").data
    assert read.count("Kept.") == 2 and read.count("Last.") == 2
    ok(box, session, "word_undo", doc="d1")
    ok(box, session, "word_changes", doc="d1", action="reject", all=True)
    assert _loses_text(box, session) == []
    tree = word_docs.tree(word_docs.saved(session, "d1"))
    cells = ["".join(t.text for t in tc.iter(_W + "t")) for tc in tree.iter(_W + "tc")]
    assert cells.count("Kept.Last.") == 2
