"""``word_comments`` writes no comment into a header or footer, where Word drops comments on its
own save (``crates/ooxml-oracle/word/observations/suite-word.json``, trial ``w2-review-comments``):
such an item is refused with the body as its way out (tool rule 2), and in a call of several
items the others are still written, the refused one reported per item (rule 5).  A reply to a
comment anchored in a header is refused the same way (the golden transcript
``w2-review-comments``, call 11).  Made-up documents only."""

from __future__ import annotations

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _with_header(box):
    """A session holding a document with a body paragraph and a header: the ids of both."""
    session = box.session()
    doc = session.open(word_docs.with_body(word_docs.paragraph("Body text here.")), "h.docx")
    made = word_docs.call(box, session, "word_headers_footers", doc=doc, action="set", section="s:1",
                          which="header", text="Draft header")
    header = next(c for c in made.to_json()["created"] if "/p:" in c)
    body = made.to_json()["renamed"]["p@body/0"]
    return session, doc, header, body


@pytest.mark.parametrize("which", ["header", "footer"])
def test_a_comment_in_a_header_or_footer_is_refused_with_the_body_as_the_way_out(box, which):
    session = box.session()
    doc = session.open(word_docs.with_body(word_docs.paragraph("Body text here.")), "h.docx")
    made = word_docs.call(box, session, "word_headers_footers", doc=doc, action="set", section="s:1",
                          which=which, text="Draft line")
    place = next(c for c in made.to_json()["created"] if "/p:" in c)
    result = box.dispatch(session, "word_comments", {"doc": doc, "action": "add",
                                                     "items": [{"target": place, "text": "Check this."}]})
    error = result.to_json()["error"]
    assert error["code"] == "refused"
    assert f"{which}1" in error["message"] and "in the body" in error["message"]
    assert error["try_call"]["arguments"]["action"] == "list"
    assert word_docs.call(box, session, "word_comments", doc=doc, action="list").data == []


def test_the_other_items_of_a_call_are_written(box):
    session, doc, header, body = _with_header(box)
    result = word_docs.call(box, session, "word_comments", doc=doc, action="add", author="A", items=[
        {"target": header, "text": "In the header."}, {"target": body, "text": "In the body."}]).to_json()
    comments, refused = result["data"]["comments"], result["data"]["refused"]
    assert comments[0] is None and comments[1].startswith("c:")
    assert [r["item"] for r in refused] == [0] and refused[0]["code"] == "refused"
    assert result["summary"] == "add: 1 of 2 comment(s), 1 refused"
    assert result["warnings"] and result["warnings"][0].startswith("items[0] not written: ")
    listed = word_docs.call(box, session, "word_comments", doc=doc, action="list").data
    assert [c["text"] for c in listed] == ["In the body."]
    # one history step: the written item undone with it
    assert box.dispatch(session, "undo", {"doc": doc}).ok
    assert word_docs.call(box, session, "word_comments", doc=doc, action="list").data == []
