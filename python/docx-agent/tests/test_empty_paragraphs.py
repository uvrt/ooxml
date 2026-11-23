"""Empty paragraphs between text (production feedback round 18, Q4): ``check`` with
``include: ["layout"]`` lists in ``layout.empty_paragraphs``, per table cell and for the
body, the blank lines left between two paragraphs of text, with the ``word_delete`` calls
that take them out; applying them (tracked too) and checking again finds nothing, and the
text is unchanged."""

from __future__ import annotations

import itertools
import json

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit 0.4 (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402
from layout_reads import table_ids  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_ids = itertools.count(0x700)


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, json.dumps(result.to_json(), indent=1)
    return result


def _p(text: str = "", ppr: str = "", inner: str | None = None) -> tuple[str, str]:
    """A paragraph with a paraId of its own: (its id, its XML)."""
    identifier = f"{next(_ids):08X}"
    runs = inner if inner is not None else (f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else "")
    return f"p:{identifier}", (f'<w:p w14:paraId="{identifier}" w14:textId="77777777">'
                               f'{f"<w:pPr>{ppr}</w:pPr>" if ppr else ""}{runs}</w:p>')


def _cell(*paragraphs: str) -> str:
    return '<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>' + "".join(paragraphs) + "</w:tc>"


def _table(rows: list[list[str]]) -> str:
    """A table of cells (each its paragraphs' XML), its rows with paraIds."""
    grid = '<w:gridCol w:w="4500"/>' * len(rows[0])
    return (f'<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>'
            + "".join(f'<w:tr w14:paraId="{next(_ids):08X}" w14:textId="77777777">' + "".join(row) + "</w:tr>"
                      for row in rows) + "</w:tbl>")


def _document() -> tuple[bytes, list[str]]:
    """A made-up document: an opening line, then two blank lines (one a space only) before
    the next, a table whose cell (1, 0) reads Title, a blank line, Body text, and a closing
    line; an empty paragraph also opens the document and ends the cell (1, 1).  Its blanks."""
    blank0, blank1, blank2 = _p(), _p(" "), _p()
    body = (_p()[1] + _p("Opening line.")[1] + blank0[1] + blank1[1] + _p("Next line.")[1]
            + _table([[_cell(_p("Item")[1]), _cell(_p("Note")[1])],
                      [_cell(_p("Title")[1], blank2[1], _p("Body text")[1]), _cell(_p("y")[1], _p()[1])]])
            + _p("Closing line.")[1])
    return word_docs.with_body(body), [blank0[0], blank1[0], blank2[0]]


def _texts(session, doc: str) -> list[str]:
    """Every paragraph's text, empty ones left out, in order."""
    return [text for _, text in word_docs.paragraphs(session, doc) if text.strip()]


def _found(box, session, doc: str, **arguments) -> list[dict]:
    data = _ok(box, session, "check", doc=doc, include=["layout"], **arguments).data
    return data["layout"]["empty_paragraphs"]


def test_the_repro_lists_the_cell_and_the_body_with_their_paragraphs(box):
    data, blanks = _document()
    session = box.session()
    doc = session.open(data, "notes.docx")
    table = table_ids(box, session, doc)[0]
    found = {entry["at"]: entry for entry in _found(box, session, doc)}
    assert set(found) == {"body", f"{table}/c1,0"}
    assert found[f"{table}/c1,0"] == {
        "kind": "empty_paragraphs", "at": f"{table}/c1,0", "paragraphs": [1], "ids": [blanks[2]],
        "fix": {"tool": "word_delete", "calls": [{"target": blanks[2]}]}}
    # Body: positions among its paragraphs (the empty first one is 0, never listed: it
    # leads); the two blanks are siblings, so one span deletes them.
    body = found["body"]
    assert body["paragraphs"] == [2, 3] and body["ids"] == blanks[:2]
    assert body["fix"]["calls"] == [{"target": f"{blanks[0]}..{blanks[1]}"}]


@pytest.mark.parametrize("tracked", [False, True])
def test_applying_the_fixes_leaves_nothing_and_the_text_unchanged(box, tracked):
    data, blanks = _document()
    session = box.session()
    doc = session.open(data, "notes.docx")
    before = _texts(session, doc)
    if tracked:
        _ok(box, session, "word_set_tracking", doc=doc, on=True, author="Reviewer")
    for entry in _found(box, session, doc):
        for call in entry["fix"]["calls"]:
            _ok(box, session, entry["fix"]["tool"], doc=doc, **call)
    assert _found(box, session, doc) == []
    assert _texts(session, doc) == before
    if tracked:                          # deleted as tracked changes, accepted they are gone
        _ok(box, session, "word_changes", doc=doc, action="accept", all=True)
    gone = set(blanks)
    assert not gone & {identifier for identifier, _ in word_docs.paragraphs(session, doc)}


def test_only_on_request(box):
    data, _ = _document()
    session = box.session()
    doc = session.open(data, "notes.docx")
    assert "layout" not in _ok(box, session, "check", doc=doc).data


def test_a_table_a_section_break_or_a_page_break_before_ends_a_run_unlisted(box):
    section = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1417" w:bottom="1417" '
               'w:left="1417" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')
    body = (_p("One.")[1] + _p()[1] + _p(ppr="<w:pageBreakBefore/>")[1] + _p()[1] + _p("Two.")[1]
            + _p(ppr=section)[1] + _p("Three.")[1] + _p()[1]
            + _table([[_cell(_p("A")[1])], [_cell(_p("b")[1])]]) + _p()[1] + _p("Four.")[1])
    session = box.session()
    doc = session.open(word_docs.with_body(body), "runs.docx")
    assert _found(box, session, doc) == []


def test_two_runs_in_one_cell_are_one_entry_the_last_deleted_first(box):
    first_blank, second_blank = _p(), _p()
    body = (_table([[_cell(_p("A")[1])], [_cell(_p("b")[1], first_blank[1], _p("Second")[1], second_blank[1],
                                                _p("Third")[1])]]) + _p("After.")[1])
    session = box.session()
    doc = session.open(word_docs.with_body(body), "cell.docx")
    table = table_ids(box, session, doc)[0]
    (entry,) = _found(box, session, doc)
    assert entry["at"] == f"{table}/c1,0" and entry["paragraphs"] == [1, 3]
    assert entry["fix"]["calls"] == [{"target": second_blank[0]}, {"target": first_blank[0]}]


@pytest.mark.parametrize("inner, state", [
    ("", "empty"),
    ("<w:r><w:t xml:space='preserve'>  </w:t></w:r>", "empty"),
    ("<w:bookmarkStart w:id='0' w:name='here'/><w:bookmarkEnd w:id='0'/>", "empty"),
    ("<w:r><w:t>x</w:t></w:r>", "text"),
    ("<w:r><w:tab/></w:r>", "text"),
    ("<w:r><w:br/></w:r>", "text"),
    ("<w:r><w:drawing/></w:r>", "text"),
    ("<w:fldSimple w:instr='PAGE'/>", "text"),
    ("<w:r><w:footnoteReference w:id='1'/></w:r>", "text"),
    ("<w:r><w:commentReference w:id='1'/></w:r>", "text"),
    ("<w:sdt><w:sdtContent/></w:sdt>", "text"),
    ("<m:oMath xmlns:m='http://schemas.openxmlformats.org/officeDocument/2006/math'/>", "text"),
    ("<w:del w:id='1' w:author='A'><w:r><w:delText>Old</w:delText></w:r></w:del>", "text"),
    ("<w:pPr><w:tabs><w:tab w:val='left' w:pos='720'/></w:tabs></w:pPr>", "empty"),
    ("<w:pPr><w:framePr w:w='2000'/></w:pPr>", "kept"),
    ("<w:pPr><w:rPr><w:del w:id='1' w:author='A'/></w:rPr></w:pPr>", None),
])
def test_what_makes_a_paragraph_empty(box, inner, state):
    """Between two paragraphs of text, a paragraph of ``inner`` is listed where it is empty;
    one with something drawn, kept (a frame) or merged into the next (its mark deleted) not."""
    session = box.session()
    note = "footnoteReference" in inner          # a note's reference: made by the tools, its note there
    middle = _p(inner="" if note else inner)
    body = _p("Text before.")[1] + middle[1] + _p("Text after.")[1]
    doc = session.open(word_docs.with_body(body), "state.docx")
    if note:
        _ok(box, session, "word_notes", doc=doc, action="insert", at=f"{middle[0]}@0", text="A note.")
    assert bool(_found(box, session, doc)) == (state == "empty"), state
