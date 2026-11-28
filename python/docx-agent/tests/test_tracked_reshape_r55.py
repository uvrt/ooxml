"""Round 55 (R115, R117): a tracked merge or split over pending tracked structure changes, in
a form Word's Accept All and Reject All settle exactly, whatever the order of the edits;
and a tracked text replacement diffed at word boundaries.  Made-up documents only.

* **R115** (measured, Word 16.106 for Mac, ``feedback-round55/probe-C``): a merge over a
  pending column deletion leaves the deleted column's cells out (they keep their deletion,
  Word's Accept All removes them, the cells merged away beyond them go to the merge's first
  cell); merged down over a row whose deletion is pending, that row is passed over; a merge
  reaching a pending merge takes it in whole; a split down beside a pending column deletion
  writes the new row's cell under it deleted too.  Delete-then-merge and merge-then-delete
  settle to the same tables: deleting a column or row a pending merge took a cell from takes
  that cell's text out of the merged cell again.  A column or row holding a pending merge's
  first cell is refused, the fix accepting that merge alone, by id.  ``word_changes`` lists
  the merge as its own change.
* **R117**: a tracked replacement marks whole words -- "Quarterly" deleted, "Half-yearly"
  inserted -- never "Quarte" / "Half-yea" around a shared "rly"; the deletion before the
  insertion.  Untracked edits are unchanged.

Tool-design rules: 1 (an edit made in either order means one table), 2 (a refusal names the
blocking change by id, with the call that settles it), 3 (a merge already made is a no-op),
8 (the forms follow Word's Accept All / Reject All, measured).
"""

from __future__ import annotations

import datetime as dt
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


def _opened(box, tracked: bool = True):
    session = box.session(clock=CLOCK)
    session.open(word_docs.blank(), "report.docx")
    first = word_docs.paragraphs(session, "d1")[0][0]
    ok(box, session, "word_insert_markdown", doc="d1", at=f"after:{first}",
       markdown=f"Field report.\n\n{MARKDOWN}\n\nAfter the table.")
    if tracked:
        ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    return session, _last_table(_document(session))


def _document(session) -> bytes:
    """The document's bytes as they are now."""
    return word_docs.saved(session, "d1")


def _tables(data: bytes) -> list:
    return [b for b in word_docs.body_blocks(word_docs.tree(data)) if b.tag == _W + "tbl"]


def _last_table(data: bytes) -> str:
    return f"t:{word_docs.rows_of(_tables(data)[-1])[0].get(word_docs.W14_PARA)}"


def _opened_data(box, data: bytes):
    session = box.session(clock=CLOCK)
    session.open(data, "report.docx")
    return session


def _check(box, data: bytes) -> dict:
    return ok(box, _opened_data(box, data), "check", doc="d1", include=["validate", "layout"]).data


def _no_grid_mismatch(box, data: bytes) -> None:
    assert _check(box, data)["layout"].get("table_grid_mismatch", []) == []


def _markdown(box, data: bytes, target: str | None = None) -> str:
    session = _opened_data(box, data)
    text = word_docs.read(box, session, "d1", **({"range": target} if target else {}))
    return re.sub(r" ?<!-- [^>]*-->\n?| data-id=\"[^\"]*\"", "", text)


def _shape(data: bytes) -> list[list[tuple]]:
    """Each row's cells of the body's last table: (grid column, span, vertical merge, text)."""
    out = []
    for row in word_docs.rows_of(_tables(data)[-1]):
        column, line = 0, []
        for cell in row.iter(_W + "tc"):
            properties = cell.find(_W + "tcPr")
            span = properties.find(_W + "gridSpan") if properties is not None else None
            merge = properties.find(_W + "vMerge") if properties is not None else None
            width = int(span.get(_W + "val")) if span is not None else 1
            text = " / ".join("".join(t.text or "" for t in p.iter(_W + "t")) for p in cell.iter(_W + "p"))
            line.append((column, width, None if merge is None else merge.get(_W + "val", "continue"), text))
            column += width
        out.append(line)
    return out


def M(r1, c1, r2, c2):
    return {"action": "merge", "first": {"row": r1, "col": c1}, "last": {"row": r2, "col": c2}}


def DC(index):
    return {"action": "delete_column", "index": index}


def DR(index):
    return {"action": "delete_row", "index": index}


def S(row, col, rows=1, columns=2):
    return {"action": "split", "first": {"row": row, "col": col}, "rows": rows, "columns": columns}


def _made(box, steps, tracked: bool = True) -> bytes:
    session, _ = _opened(box, tracked)
    for step in steps:
        ok(box, session, "word_edit_table", doc="d1", table=_last_table(_document(session)), **step)
    return _document(session)


def _settled(box, data: bytes, action: str) -> list[list[tuple]]:
    session = _opened_data(box, data)
    ok(box, session, "word_changes", doc="d1", action=action, all=True)
    settled = _document(session)
    assert _check(box, settled)["validate"]["problems"] == []
    return _shape(settled)


#: Each case written in both orders, and the table its acceptance must give (the same edits
#: made untracked, in the first order -- the accepted table as a reader means it).
ORDERS = {
    "merge left of the deleted column": ([DC(2), M(1, 0, 1, 1)], [M(1, 0, 1, 1), DC(2)]),
    "deleted column inside the merge": ([DC(1), M(1, 0, 1, 1)], [M(1, 0, 1, 2), DC(1)]),
    "header row over the deleted column": ([DC(1), M(0, 0, 0, 2)], [M(0, 0, 0, 3), DC(1)]),
    "rectangle over the deleted column": ([DC(2), M(1, 1, 2, 2)], [M(1, 1, 2, 3), DC(2)]),
    "merged down over a deleted row": ([DR(2), M(1, 0, 2, 0)], [M(1, 0, 3, 0), DR(2)]),
    "split down beside a deleted column": ([DC(2), S(1, 0, rows=2, columns=1)],
                                           [S(1, 0, rows=2, columns=1), DC(2)]),
}

#: Cases written one way (a merge right of the deleted column, down beside it, a split,
#: two merges), each accepted as the same edits untracked.
CASES = {
    "merge right of the deleted column": [DC(1), M(1, 1, 1, 2)],
    "merge down beside the deleted column": [DC(2), M(1, 0, 2, 0)],
    "first column deleted": [DC(0), M(1, 0, 1, 1)],
    "split across beside the deleted column": [DC(2), S(1, 1)],
    "split of the column after the deleted one": [DC(2), S(1, 2)],
    "split beside a deleted row": [DR(2), S(1, 0)],
    "two merges apart": [M(1, 0, 1, 1), M(2, 2, 2, 3)],
    "two merges, one row each": [M(1, 0, 1, 1), M(2, 0, 2, 1)],
    "a merge grown over a pending merge": [M(1, 0, 1, 1), M(1, 0, 1, 2)],
    "a merge taking in a pending merge's first cell": [M(1, 1, 1, 2), M(1, 0, 1, 2)],
    "merged across, then down": [M(1, 0, 1, 1), M(1, 0, 2, 0)],
    "merged down, then further down": [M(1, 0, 2, 0), M(1, 0, 3, 0)],
    "column deleted, two merges": [DC(2), M(1, 0, 1, 1), M(2, 0, 2, 1)],
}


@pytest.mark.parametrize("case", list(ORDERS))
def test_the_order_of_the_edits_does_not_matter(box, case):
    one, other = ORDERS[case]
    first, second = _made(box, one), _made(box, other)
    plain = _made(box, one, tracked=False)
    for document in (first, second):
        _no_grid_mismatch(box, document)
        assert _settled(box, document, "accept") == _shape(plain)
        assert _settled(box, document, "reject") == _shape(_made(box, [], tracked=False))


@pytest.mark.parametrize("case", list(CASES))
def test_a_reshape_over_pending_changes_accepts_as_made_untracked(box, case):
    document = _made(box, CASES[case])
    _no_grid_mismatch(box, document)
    assert _settled(box, document, "accept") == _shape(_made(box, CASES[case], tracked=False))
    assert _settled(box, document, "reject") == _shape(_made(box, [], tracked=False))


def test_the_final_view_reads_the_merge_over_a_deleted_column_as_accepted(box):
    document = _made(box, [DC(1), M(0, 0, 0, 2)])
    session = _opened_data(box, document)
    ok(box, session, "word_changes", doc="d1", action="accept", all=True)
    accepted = _document(session)
    assert _markdown(box, document, _last_table(document)) == _markdown(box, accepted, _last_table(accepted))


def test_the_deleted_columns_text_stays_out_of_the_merged_cell(box):
    document = _made(box, [DC(1), M(1, 0, 1, 1)])
    merged = _settled(box, document, "accept")[1][0]
    assert merged[3] == "Harbour / draft" and merged[1] == 2


def test_a_merge_is_listed_as_its_own_change(box):
    session, table = _opened(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, **DC(2))
    ok(box, session, "word_edit_table", doc="d1", table=table, **M(1, 0, 1, 1))
    changes = ok(box, session, "word_changes", doc="d1", action="list").data
    assert [c["kind"] for c in changes] == ["deletion", "table"]
    deletion, merge = changes
    assert deletion["old_text"] == "Phase draft done open"
    assert merge["text"] == "Ines" and merge["author"] == ME
    assert not set(deletion["records"]) & set(merge["records"])
    # Accepting the merge alone leaves the column deletion pending, and the other way round.
    ok(box, session, "word_changes", doc="d1", action="accept", ids=[merge["id"]])
    left = ok(box, session, "word_changes", doc="d1", action="list").data
    assert [c["old_text"] for c in left] == ["Phase draft done open"]


def test_a_repeated_merge_is_a_no_op(box):
    session, table = _opened(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, **M(1, 0, 1, 1))
    before = _document(session)
    again = ok(box, session, "word_edit_table", doc="d1", table=table, **M(1, 0, 1, 1))
    assert again.changed == [] and "nothing changed" in again.warnings[0]
    assert _document(session) == before
    assert len(ok(box, session, "word_changes", doc="d1", action="list").data) == 1


@pytest.mark.parametrize("step,where", [(DC(0), "column 0"), (DR(1), "row 1")])
def test_deleting_the_first_cell_of_a_pending_merge_is_refused_with_its_id(box, step, where):
    session, table = _opened(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, **DC(2))
    ok(box, session, "word_edit_table", doc="d1", table=table, **M(1, 0, 1, 1))
    merge = ok(box, session, "word_changes", doc="d1", action="list").data[-1]
    before = _document(session)
    refused = call(box, session, "word_edit_table", doc="d1", table=table, **step)
    assert not refused.ok
    error = refused.to_json()["error"]
    assert error["code"] == "refused" and where in error["message"]
    assert "does not track" not in error["message"]
    # The fix accepts the merge alone -- not the column deletion, not "within" the table.
    assert error["details"]["fix"] == {"tool": "word_changes", "calls": [{"action": "accept", "ids": [merge["id"]]}]}
    assert _document(session) == before
    way = error["details"]["call"]
    ok(box, session, way["tool"], **way["arguments"])
    assert [c["kind"] for c in ok(box, session, "word_changes", doc="d1", action="list").data] == ["deletion"]
    if step["action"] == "delete_column":
        ok(box, session, "word_edit_table", doc="d1", table=table, **step)


def test_a_split_of_a_pending_merged_cell_is_refused_with_its_id(box):
    session, table = _opened(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, **M(1, 0, 1, 1))
    merge = ok(box, session, "word_changes", doc="d1", action="list").data[-1]
    refused = call(box, session, "word_edit_table", doc="d1", table=table, **S(1, 0))
    assert not refused.ok
    error = refused.to_json()["error"]
    assert error["details"]["fix"]["calls"] == [{"action": "accept", "ids": [merge["id"]]}]


def SC(row, col, text):
    return {"action": "set_cells", "cells": [{"row": row, "col": col, "text": text}]}


@pytest.mark.parametrize("steps", [
    [M(1, 0, 1, 2), SC(1, 0, "Haven"), DC(1)],
    [M(1, 0, 1, 1), SC(1, 0, "Haven"), DC(1)],
    [M(1, 1, 2, 2), SC(1, 1, "Bo"), DC(2)],
    [DC(1), M(1, 0, 1, 1), SC(1, 0, "Haven")],
    [S(1, 0, rows=2, columns=1), DC(2), M(1, 1, 2, 1)],
], ids=lambda steps: "-".join(s["action"] for s in steps))
def test_a_cell_never_carries_two_deletion_records_and_the_file_validates(box, steps):
    """A merge, a cell's text and a column deletion in any order: one w:cellDel per cell at
    most (the schema allows one), and nothing for validation to find."""
    document = _made(box, steps)
    for row in word_docs.rows_of(_tables(document)[-1]):
        for cell in row.iter(_W + "tc"):
            assert len(cell.findall(f"{_W}tcPr/{_W}cellDel")) <= 1
    assert _check(box, document)["validate"]["problems"] == []


def test_a_column_no_one_grid_column_as_accepted_is_refused(box):
    """After a merge and a column deletion, row 1 as accepted is [Harbour over two columns]
    [awaits figures]: Word's Accept All drops the deleted cell that keeps nothing and merges
    the one that keeps its text into Harbour (Word 16.106, lane fix-grid-moved-left), so
    column 1 is Harbour's second half there and Phase in the other rows.  Deleting it is
    refused as not supported, naming row 1, and the document stays valid."""
    session, _ = _opened(box)
    for step in (M(1, 0, 1, 2), DC(1)):
        ok(box, session, "word_edit_table", doc="d1", table=_last_table(_document(session)), **step)
    refused = call(box, session, "word_edit_table", doc="d1", table=_last_table(_document(session)), **DC(1))
    assert not refused.ok
    error = refused.to_json()["error"]
    assert error["details"]["not_supported"] == "a column position whose cells pending cell changes move"
    assert "in row(s) 1 " in error["message"] and "edit the cells by row" in error["message"]
    document = _document(session)
    for row in word_docs.rows_of(_tables(document)[-1]):
        for cell in row.iter(_W + "tc"):
            assert len(cell.findall(f"{_W}tcPr/{_W}cellDel")) <= 1
    assert _check(box, document)["validate"]["problems"] == []


def test_untracked_a_merge_over_a_pending_deletion_is_still_refused(box):
    session, table = _opened(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, **DC(1))
    ok(box, session, "word_set_tracking", doc="d1", on=False)
    refused = call(box, session, "word_edit_table", doc="d1", table=table, **M(1, 0, 1, 1))
    assert not refused.ok and refused.to_json()["error"]["code"] == "refused"


# -- R117 ----------------------------------------------------------------------------------------


def _marked(data: bytes, paragraph: str) -> str:
    """The paragraph as kept text, [-deleted-] and {+inserted+}."""
    out = []
    for run in word_docs.find(word_docs.tree(data), paragraph).iter(_W + "r"):
        text = "".join(t.text or "" for t in run.iter(_W + "t", _W + "delText"))
        above = {a.tag for a in run.iterancestors()}
        out.append(f"[-{text}-]" if _W + "del" in above else f"{{+{text}+}}" if _W + "ins" in above else text)
    return "".join(out)


def _one_paragraph(box, text: str, run_properties: str = ""):
    """A document of one paragraph (``p:1A000001``) of one run of ``text``."""
    props = f"<w:rPr>{run_properties}</w:rPr>" if run_properties else ""
    data = word_docs.with_body(f'<w:p w14:paraId="1A000001" w14:textId="77777777"><w:r>{props}'
                               f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p>')
    return _opened_data(box, data), "p:1A000001"


def _text_after(box, data: bytes, action: str) -> str:
    session = _opened_data(box, data)
    ok(box, session, "word_changes", doc="d1", action=action, all=True)
    return word_docs.paragraphs(session, "d1")[0][1]


WORDS = [
    ("Quarterly report", "Half-yearly report", "[-Quarterly-]{+Half-yearly+} report"),
    ("The Quarterly report, 2025.", "The Half-yearly report, 2026.",
     "The [-Quarterly-]{+Half-yearly+} report, [-2025-]{+2026+}."),
    ("Revenue rose by 12 percent.", "Revenue fell by 15 percent.",
     "Revenue [-rose-]{+fell+} by [-12-]{+15+} percent."),
    ("Sent to the north region office.", "Sent to the south coastal office.",
     "Sent to the [-north region-]{+south coastal+} office."),
    ("Status: draft", "Status: Draft", "Status: [-draft-]{+Draft+}"),
    ("awaits figures", "awaits the final figures", "awaits {+the final +}figures"),
    # Word's Compare splits at punctuation too (measured): a hyphen, a comma, a full stop.
    ("Costs went from 1,200 to 1,250 euros.", "Costs went from 1,200 to 1,350 euros.",
     "Costs went from 1,200 to 1,[-250-]{+350+} euros."),
    ("e-mail the team-lead today", "e-mail the project-lead today", "e-mail the [-team-]{+project+}-lead today"),
    ("Signed off by the board.", "Signed off by the whole board!", "Signed off by the {+whole +}board[-.-]{+!+}"),
    ("Planned for Q3 2026", "Planned for Q4 2026", "Planned for [-Q3-]{+Q4+} 2026"),
]


@pytest.mark.parametrize("old,new,marked", WORDS)
def test_a_tracked_replacement_marks_whole_words(box, old, new, marked):
    session, paragraph = _one_paragraph(box, old)
    ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
    ok(box, session, "word_set_text", doc="d1", target=paragraph, text=new)
    data = _document(session)
    assert _marked(data, paragraph) == marked
    assert _text_after(box, data, "accept") == new
    assert _text_after(box, data, "reject") == old


def test_set_cells_tracked_lists_the_whole_words(box):
    session, table = _opened(box)
    ok(box, session, "word_edit_table", doc="d1", table=table, action="set_cells",
       cells=[{"row": 1, "col": 3, "text": "awaits the final figures"}, {"row": 1, "col": 2, "text": "Draft"}])
    changes = ok(box, session, "word_changes", doc="d1", action="list").data
    assert sorted((c.get("old_text", ""), c.get("text", "")) for c in changes) == \
        [("", "the final"), ("draft", "Draft")]


def test_an_untracked_replacement_keeps_the_characters_it_shares(box):
    """Untracked, a character that survives keeps its run (its formatting): unchanged."""
    session, paragraph = _one_paragraph(box, "Quarterly report", "<w:b/>")
    ok(box, session, "word_set_text", doc="d1", target=paragraph, text="Half-yearly report")
    found = word_docs.find(word_docs.tree(_document(session)), paragraph)
    assert all(r.find(f"{_W}rPr/{_W}b") is not None for r in found.iter(_W + "r"))
    assert word_docs.text_of(found) == "Half-yearly report"


def test_a_tracked_replacement_accepts_with_the_untracked_edits_formatting(box):
    """A word partly kept ("bold" -> "...d") is replaced whole, its new text formatted per
    character as the untracked edit formats it: accepted, the same Markdown."""
    def made(tracked: bool) -> tuple[bytes, str]:
        session = _opened_data(box, word_docs.blank())
        ok(box, session, "word_insert_markdown", doc="d1", at="end", markdown="Plain **bold** and *italic* runs.")
        paragraph, text = next((p, t) for p, t in word_docs.paragraphs(session, "d1") if t.startswith("Plain"))
        if tracked:
            ok(box, session, "word_set_tracking", doc="d1", on=True, author=ME)
        ok(box, session, "word_set_text", doc="d1", target=paragraph, text=text[:5] + "EDITED" + text[9:])
        return _document(session), paragraph

    tracked, paragraph = made(True)
    assert _marked(tracked, paragraph).startswith("[-Plain -][-bold-]{+PlainEDITED+}{+d+}")
    session = _opened_data(box, tracked)
    ok(box, session, "word_changes", doc="d1", action="accept", all=True)
    assert _markdown(box, _document(session)) == _markdown(box, made(False)[0])


# -- Word itself (opt-in: pytest -m oracle) --------------------------------------------------------


@pytest.mark.oracle
@pytest.mark.parametrize("case", [*ORDERS, *CASES])
@pytest.mark.parametrize("action", ["accept", "reject"])
def test_words_accept_and_reject_equal_ours(box, case, action):
    """Word's own Accept All / Reject All of each form give the table docx-agent's own give:
    the same cells, spans, merges and text (widths Word re-saves its own way)."""

    import oracle

    from oracle import review_script

    steps = ORDERS[case][0] if case in ORDERS else CASES[case]
    data = _made(box, steps)
    try:
        with oracle.session() as word:
            outcome = word.run_script(review_script(action), data, name="r55c", tag=action, timeout=200)
            assert outcome, f"Word did not {action}: {outcome.outcome} {outcome.detail}"
            theirs = outcome.path.read_bytes()
            outcome.path.unlink(missing_ok=True)
    except oracle.WordBusy as busy:
        pytest.skip(str(busy))
    assert _shape(theirs) == _settled(box, data, action)
