"""Round 60 R146: a paragraph of the file's text deleted as a tracked change (its mark too)
is listed by ``word_inspect`` as ``joins`` on the block after it, whatever that block is, so
a host that maps the body's blocks to sections still places it.  Made-up documents only.

The production team's steps: ``new_document``, "# Annex", "Intro text.", a 2x2 table,
"After text.", the file saved and opened again, tracking on, ``word_delete`` of the intro.
The final view stays as Word 16.106's Accept All leaves it (measured: the paragraph goes,
the table keeps its text); ``check`` names the deletion in ``accept_loses_original``.

Tool-design rules: 1 (the body's blocks place every paragraph of the file), 8 (the view is
Word's Accept All, measured).
"""

from __future__ import annotations

import datetime as dt

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

CLOCK = lambda: dt.datetime(2026, 11, 25, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
NEXT = {"table": "| A | B |\n|---|---|\n| 1 | 2 |", "paragraph": "Next para."}


@pytest.fixture(scope="module")
def box():
    with Toolbox(list(TOOLS), formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def ok(box, session, name, /, **arguments):
    result = box.dispatch(session, name, arguments)
    assert result.ok, (name, result.to_json())
    return result


def _deleted(box, following: str):
    """The team's document with ``following`` after the intro, opened again from its file,
    the intro deleted tracked: the session and the intro's id."""
    made = box.session(clock=CLOCK)
    ok(box, made, "new_document", kind="docx")
    ok(box, made, "word_insert_markdown", doc="d1", at="end",
       markdown=f"# Annex\n\nIntro text.\n\n{NEXT[following]}\n\nAfter text.")
    session = box.session(clock=CLOCK)
    session.open(made.save("d1").data, "annex.docx")
    blocks = ok(box, session, "word_inspect", doc="d1", target="body").data["blocks"]
    intro = next(b["id"] for b in blocks if b.get("t") == "Intro text.")
    ok(box, session, "word_set_tracking", doc="d1", on=True, author="Reviewer")
    ok(box, session, "word_delete", doc="d1", target=intro)
    return session, intro


def _texts(blocks):
    out = []
    for block in blocks:
        if block["type"] == "paragraph":
            out.append(block["t"])
        for row in block.get("rows", []):
            for cell in row:
                out += _texts(cell["blocks"])
        out += _texts(block.get("blocks", []))
    return out


def test_before_a_table_the_deleted_paragraph_is_the_tables_joins(box):
    session, intro = _deleted(box, "table")
    blocks = ok(box, session, "word_inspect", doc="d1", target="body").data["blocks"]
    assert [b["type"] for b in blocks] == ["paragraph", "table", "paragraph"]
    assert blocks[1]["joins"] == [intro]
    assert "joins" not in blocks[0] and "joins" not in blocks[2]
    # the first cell's paragraph takes it in, as Word's Accept All does; its text is gone
    assert blocks[1]["rows"][0][0]["blocks"][0]["joins"] == [intro]
    assert _texts(blocks) == ["Annex", "A", "B", "1", "2", "After text."]
    # the read of the paragraph is the table's; check names the deletion
    read = ok(box, session, "word_inspect", doc="d1", target=intro).data["blocks"]
    assert read[0]["id"] == blocks[1]["id"]
    lost = ok(box, session, "check", doc="d1", include=["layout"]).data["layout"]["accept_loses_original"]
    assert [entry["paras"] for entry in lost] == [[intro]]


def test_before_a_paragraph_the_deleted_paragraph_stays_its_joins(box):
    session, intro = _deleted(box, "paragraph")
    blocks = ok(box, session, "word_inspect", doc="d1", target="body").data["blocks"]
    assert [(b["t"], b.get("joins")) for b in blocks] == [
        ("Annex", None), ("Next para.", [intro]), ("After text.", None)]
