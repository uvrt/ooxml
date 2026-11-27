"""docx-agent's reading tools on the Rust host (lane P1c): ``describe``, ``word_read``,
``word_inspect``, ``find_text`` read the version an edit made; an id an edit deleted is refused
naming the edit.
"""
from __future__ import annotations

import json

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR
import word_docs


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def _answer(toolbox, session, tool, arguments) -> dict:
    """``tool``'s answer through the toolbox."""
    return json.loads(json.dumps(toolbox.dispatch(session, tool, arguments).to_json()))


def _first(session, doc) -> str:
    return next(block for block, text in word_docs.paragraphs(session, doc) if block != "p:None" and text)


def test_a_new_version_is_handed_over_as_the_parts_that_changed(toolbox):
    path = FIXTURE_DIR / "generated" / "ids-and-markup.docx"
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        first = _first(session, doc)
        before = _answer(toolbox, session, "word_read", {"doc": doc})
        edited = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": first, "text": "Changed here."})
        assert edited.ok, edited.to_json()
        for tool, arguments in [("word_read", {"doc": doc}), ("describe", {"doc": doc}),
                                ("word_inspect", {"doc": doc, "target": first}),
                                ("find_text", {"doc": doc, "text": "Changed"})]:
            assert _answer(toolbox, session, tool, arguments)["ok"], tool
        assert "Changed here." in _answer(toolbox, session, "word_read", {"doc": doc})["data"]
        assert before != _answer(toolbox, session, "word_read", {"doc": doc})
        found = _answer(toolbox, session, "find_text", {"doc": doc, "text": "Changed"})
        assert found["data"], found
    finally:
        session.close(doc)


def test_an_id_a_served_edit_deleted_is_refused_naming_it(toolbox):
    path = FIXTURE_DIR / "generated" / "ids-and-markup.docx"
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        first = _first(session, doc)
        deleted = toolbox.dispatch(session, "word_delete", {"doc": doc, "target": first, "force": True})
        assert deleted.ok, deleted.to_json()
        # an id an edit deleted: refused naming the edit (lane P2e1)
        gone = _answer(toolbox, session, "word_inspect", {"doc": doc, "target": first})
        assert f"{first} was deleted by word_delete" in gone["error"]["message"]
    finally:
        session.close(doc)
