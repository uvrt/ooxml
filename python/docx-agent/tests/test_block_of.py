"""``block_of`` and ``blocks_between`` (production round 22, U1; :mod:`docx_agent.tools`): the
body's top-level block holding any id the tools take, and the inclusive span of top-level
blocks between two such ids, as the application asks them of an open document
(``block_of(session, doc, id)``) or a handler of its call.  A made-up document."""

from __future__ import annotations

import pytest
from ooxml_edit.tools import ToolError, Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS, block_of, blocks_between
import word_docs

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def _p(identifier: str, inner: str = "") -> str:
    return f'<w:p w14:paraId="{identifier}" w14:textId="77777777">{inner}</w:p>'


def _r(text: str) -> str:
    return f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'


def _tc(inner: str) -> str:
    return f'<w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr>{inner}</w:tc>'


#: A heading line, two body paragraphs (a bookmark in the second), a table whose cell (0, 1)
#: holds a nested table, a rich-text content control, and a closing paragraph with a link and
#: a PAGE field.
NESTED = ('<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>'
          f'<w:tr w14:paraId="000000A1" w14:textId="77777777">{_tc(_p("000000A2", _r("inner")))}</w:tr></w:tbl>')
TABLE = ('<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/>'
         '<w:gridCol w:w="4000"/></w:tblGrid><w:tr w14:paraId="000000B1" w14:textId="77777777">'
         + _tc(_p("000000B2")) + _tc(_p("000000B3", _r("cell text")) + NESTED + _p("000000B4"))
         + '</w:tr><w:tr w14:paraId="000000B5" w14:textId="77777777">' + _tc(_p("000000B6")) + _tc(_p("000000B7"))
         + "</w:tr></w:tbl>")
BODY = (_p("00000011", _r("Report title")) + _p("00000012", _r("Some body text here."))
        + _p("00000013", _r("Third ") + '<w:bookmarkStart w:id="0" w:name="Marked"/>' + _r("paragraph")
             + '<w:bookmarkEnd w:id="0"/>' + _r(" text."))
        + TABLE + '<w:sdt><w:sdtPr><w:id w:val="123"/><w:richText/></w:sdtPr><w:sdtContent>'
        + _p("00000014", _r("Inside")) + "</w:sdtContent></w:sdt>"
        + _p("00000015", '<w:hyperlink w:anchor="Marked">' + _r("Site") + "</w:hyperlink>"
             '<w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple>' + _r("Closing words.")))


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.to_json())
    return result


@pytest.fixture
def made(box) -> dict:
    """The made-up document open, a note, a comment and a text box added in its body, and a
    header: the session, the document and the ids."""
    session = box.session()
    d = session.open(word_docs.with_body(BODY), "made.docx")
    body, later = "p:00000012", "p:00000013"
    note = _ok(box, session, "word_notes", doc=d, action="insert", at=f"{body}@4", text="A note.").created[0]
    comment = _ok(box, session, "word_comments", doc=d, action="add", author="Reviewer",
                  items=[{"target": f"{later}@0:5", "text": "A remark."}]).created[0]
    drawing = _ok(box, session, "word_drawings", doc=d, action="insert_text_box", at=f"{body}@2",
                  text="boxed text").data["id"]
    header = _ok(box, session, "word_headers_footers", doc=d, section="s:body", which="header", action="set",
                 text="Header text")
    root = word_docs.tree(word_docs.saved(session, d))
    boxed = next(p for p in root.iter(W + "p") if word_docs.text_of(p) == "boxed text"
                 and any(a.tag == W + "txbxContent" for a in p.iterancestors()))
    link = _ok(box, session, "word_links", doc=d, action="list").data["hyperlinks"][0]["id"]
    field = _ok(box, session, "word_fields", doc=d, action="list").data[0]["id"]
    return {"session": session, "doc": d, "title": "p:00000011", "body": body, "later": later,
            "table": "t:000000B1", "cell": "p:000000B3", "nested": "t:000000A1", "inner": "p:000000A2",
            "control": "cc:123", "inside": "p:00000014", "closing": "p:00000015", "note": note,
            "comment": comment, "box": f"p:{boxed.get(word_docs.W14_PARA)}", "link": link, "field": field,
            "header": next(c for c in header.created if "/p:" in c),          # header1/p:...
            "header_story": header.data["story"], "drawing": drawing}


def _of(made, identifier: str):
    return block_of(made["session"], made["doc"], identifier)


def test_a_top_level_block_is_its_own(made):
    for block in ("title", "body", "later", "table", "control", "closing"):
        assert _of(made, made[block]) == made[block], block


def test_nested_ids_give_the_top_level_block(made):
    table = made["table"]
    assert _of(made, made["cell"]) == table
    assert _of(made, made["nested"]) == table
    assert _of(made, made["inner"]) == table
    assert _of(made, f"{table}/c0,1") == table
    assert _of(made, "tr:000000B5") == table
    assert _of(made, f"{made['cell']}/r0") == table
    assert _of(made, f"{made['cell']}@1:3") == table                       # a range: its start
    assert _of(made, f"{made['body']}@1..{made['cell']}@2") == made["body"]
    assert _of(made, made["box"]) == made["body"]                          # a text box's paragraph
    assert _of(made, made["drawing"]) == made["body"]
    assert _of(made, made["inside"]) == made["control"]                    # a control's paragraph


def test_annotations_give_the_block_that_holds_them(made):
    assert _of(made, made["note"]) == made["body"]                         # its reference
    assert _of(made, made["comment"]) == made["later"]                     # its range's start
    assert _of(made, "bm:Marked") == made["later"]
    assert _of(made, "Marked") == made["later"]                            # the plain name
    assert _of(made, made["link"]) == made["closing"]
    assert made["field"].startswith("fld:body/") and _of(made, made["field"]) == made["closing"]
    assert _of(made, made["control"]) == made["control"]


def test_what_is_not_in_the_body_is_none(made):
    assert _of(made, made["header"]) is None
    assert _of(made, made["header_story"]) is None
    assert _of(made, "body") is None
    assert _of(made, "s:body") is None


def test_a_section_break_is_its_paragraphs_block(box, made):
    session, d = made["session"], made["doc"]
    _ok(box, session, "word_sections", doc=d, action="insert_break", after=made["body"], start="nextPage")
    section = next(s["id"] for s in _ok(box, session, "word_sections", doc=d, action="list").data
                   if s["id"] != "s:body")
    assert _of(made, section) == _of(made, "p" + section[1:])


def test_an_unknown_id_is_not_found_and_a_span_is_refused(made):
    for unknown in ("p:DEADBEEF", "fn:99", "c#99", "bm:Nowhere", "Nowhere", "d:99", f"{made['table']}/c9,9"):
        with pytest.raises((KeyError, IndexError)):
            _of(made, unknown)
    with pytest.raises(Exception, match="blocks_between"):
        _of(made, f"{made['body']}..{made['later']}")


def test_blocks_between_spans_top_level_blocks_from_nested_ends(made):
    session, d = made["session"], made["doc"]
    assert blocks_between(session, d, made["note"], made["inner"]) == [made["body"], made["later"], made["table"]]
    assert blocks_between(session, d, made["title"], made["title"]) == [made["title"]]
    assert blocks_between(session, d, made["cell"], made["link"]) == [made["table"], made["control"],
                                                                     made["closing"]]


def test_blocks_between_refuses_a_reversed_span_as_the_span_tools_do(made):
    session, d = made["session"], made["doc"]
    with pytest.raises(Exception, match="ends before it starts"):
        blocks_between(session, d, made["inner"], made["note"])
    with pytest.raises(Exception, match="not in the body"):
        blocks_between(session, d, made["header"], made["body"])


def test_a_revision_and_an_endnote_give_their_block(box, made):
    session, d = made["session"], made["doc"]
    _ok(box, session, "word_set_tracking", doc=d, on=True, author="Reviewer")
    _ok(box, session, "word_insert_text", doc=d, at=f"{made['inner']}@0", text="new ")
    revision = _ok(box, session, "word_changes", doc=d, action="list").data[0]["id"]
    assert _of(made, revision) == made["table"]
    endnote = _ok(box, session, "word_notes", doc=d, action="insert", kind="endnote", at=f"{made['closing']}@3",
                  text="An endnote.").created[0]
    assert _of(made, endnote) == made["closing"]


def test_the_application_resolves_refs_outside_a_call(box, made):
    """Round 23, V5: block_of(session, doc, id) and blocks_between(session, doc, a, b), the
    same helpers as a handler's (call, ...) forms, resolve a ref with Session.resolve_ref."""
    session, doc = made["session"], made["doc"]
    result = box.dispatch(session, "word_insert_text", {"doc": doc, "items": [
        {"target": made["cell"], "text": " more", "ref": "cell_text"},
        {"target": f"{made['body']}@0", "where": "before", "text": "So ", "ref": "opening"}]})
    assert result.ok, result.summary
    assert set(session.refs(doc)) == {"cell_text", "opening"}
    assert block_of(session, doc, "$cell_text") == made["table"]
    assert block_of(session, doc, made["box"]) == made["body"]         # an id as given
    assert blocks_between(session, doc, "$opening", "$cell_text") == [made["body"], made["later"], made["table"]]
    with pytest.raises(ToolError) as missing:
        block_of(session, doc, "$nowhere")
    assert missing.value.code == "not_found" and missing.value.valid_options == ["$cell_text", "$opening"]
    with pytest.raises(ToolError) as closed:
        block_of(session, "d9", "$opening")
    assert closed.value.code == "not_found"
    with pytest.raises(TypeError, match=r"\(session, doc, first, last\)"):
        blocks_between(session, doc, "$opening")
    box.dispatch(session, "undo", {"doc": doc})                    # the refs go with it
    assert session.refs(doc) == {}
