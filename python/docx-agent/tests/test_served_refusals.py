"""Lane P2e1: a refusal is not a fallback.  A served tool the Rust session refuses answers
with that refusal -- docx-agent's code, message and field, with Rust's way out
(``valid_options``, ``try_call``) -- never handed to the Python handler for its wording; an
id a change removed is refused as docx-agent's view refuses it (which change removed it, and
the way on), from the session's record.  Made-up documents only."""

from __future__ import annotations

import datetime as dt
import pickle

import pytest

from docx_agent import Document
from docx_agent.rust import edits, reads
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox, ToolError
from ooxml_edit.tools.owned import OwnedEntry

CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
GONE = "p:0BADF00D"


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture(scope="module")
def report() -> bytes:
    doc = Document.new()
    doc.insert_markdown("# A made-up report\n\nLantern meadow by the harbour.\n\nRiver stone, river stone.\n\n"
                        "| Crop | Yield |\n| --- | --- |\n| Oats | 4 |\n\nThe last line.")
    return doc.to_bytes()


def _open(toolbox, data: bytes, served: bool):
    """A session that serves (the system's clock) or one the Python handler answers (its own)."""
    session = toolbox.session() if served else toolbox.session(clock=CLOCK)
    doc = session.open(data, "report.docx")
    if served and not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    return session, doc


def _ids(session, doc) -> dict[str, str]:
    return {p.text: p.id for p in session.entry(doc).document.paragraphs()}


def _refusing(ids: dict[str, str]) -> list[tuple[str, dict]]:
    meadow, stone = ids["Lantern meadow by the harbour."], ids["River stone, river stone."]
    return [
        ("word_set_text", {"target": GONE, "text": "x"}),
        ("word_set_text", {"target": f"{meadow}@99:120", "text": "x"}),
        ("replace_text", {"find": "zz-nowhere-zz", "replace": "x"}),
        ("replace_text", {"find": "river stone", "replace": "x", "expect": 5}),
        ("word_insert_text", {"at": f"{GONE}@0", "text": "x"}),
        ("word_insert_text", {"find": "nowhere at all", "text": "x"}),
        ("word_delete", {"target": GONE}),
        ("word_delete", {"find": "River", "target": stone, "within": stone}),
        ("word_format", {"targets": [GONE], "bold": True}),
        ("word_format", {"targets": [meadow], "style": "No Such Style"}),
        ("word_lists", {"action": "restart", "targets": [meadow]}),
        ("word_styles", {"action": "delete", "name": "Normal"}),
        ("word_insert_markdown", {"at": f"after:{GONE}", "markdown": "Text."}),
        ("word_fields", {"action": "delete", "ids": ["fld:body/999"]}),
        ("word_move", {"blocks": GONE, "after": meadow}),
        ("word_sections", {"action": "set", "section": "s:nowhere", "orientation": "landscape"}),
        ("word_notes", {"action": "edit", "note": "fn:99", "text": "x"}),
        ("word_links", {"action": "add_bookmark", "target": meadow, "bookmark": "1bad"}),
        ("word_drawings", {"action": "set", "target": "d:999999", "wrap": "square"}),
        ("word_headers_footers", {"action": "set", "section": "s:nowhere", "kind": "header", "text": "x"}),
    ]


def _reply(result) -> dict:
    error = result.to_json()["error"]
    return {key: error.get(key) for key in ("code", "message", "field")}


def test_refusals_are_the_sessions_answer(toolbox, report):
    """Each refusal served by the session (counted served, no fallback) says what the Python
    handler says: code, message and field."""
    session, doc = _open(toolbox, report, served=False)
    python = {}
    for tool, arguments in _refusing(_ids(session, doc)):
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        if result.ok:
            continue
        python[(tool, repr(arguments))] = _reply(result)
    assert len(python) >= 15, python
    session, doc = _open(toolbox, report, served=True)
    differ = {}
    for tool, arguments in _refusing(_ids(session, doc)):
        if (tool, repr(arguments)) not in python:
            continue
        before, fell = dict(edits.served()), edits.fallbacks()
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert not result.ok, (tool, arguments)
        assert edits.served().get(tool, 0) == before.get(tool, 0) + 1, (tool, edits.fallbacks(), fell)
        if _reply(result) != python[(tool, repr(arguments))]:
            differ[tool, repr(arguments)] = (_reply(result), python[(tool, repr(arguments))])
    assert not differ, differ


def test_a_refusal_carries_its_way_out(toolbox, report):
    """The session's way out reaches the reply: the call that would work (tool rule 2)."""
    session, doc = _open(toolbox, report, served=True)
    result = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": GONE, "text": "x"})
    error = result.to_json()["error"]
    assert error["code"] == "not_found" and error["try_call"]["tool"], error


def test_a_deleted_id_names_the_change(toolbox, report):
    """An id a served call deleted is refused as docx-agent's view refuses it: which change
    removed it, and the way on -- served, from the session's record."""
    replies = []
    for served in (True, False):
        session, doc = _open(toolbox, report, served=served)
        target = _ids(session, doc)["The last line."]
        assert toolbox.dispatch(session, "word_delete", {"doc": doc, "target": target}).ok
        before = dict(edits.served())
        result = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": "x"})
        assert edits.served().get("word_set_text", 0) == before.get("word_set_text", 0) + served
        replies.append(_reply(result))
        assert f"{target} was deleted by word_delete" in result.error.message, result.error.message
    assert replies[0] == replies[1]


def test_a_read_refusal_is_the_sessions_answer(toolbox, report):
    """A read the session serves answers its refusal too (no Python fallback)."""
    session, doc = _open(toolbox, report, served=True)
    before = reads.fallbacks()
    result = toolbox.dispatch(session, "find_text", {"doc": doc, "text": "meadow", "range": GONE})
    assert not result.ok and result.error.code == "not_found", result.to_json()
    assert reads.fallbacks() == before


def test_try_call_pickles_and_serialises():
    error = ToolError("not_found", "no such", valid_options=["p:1"],
                      try_call={"tool": "word_read", "arguments": {"doc": "d1"}})
    again = pickle.loads(pickle.dumps(error))
    assert again.to_json() == error.to_json()
    assert list(error.to_json()) == ["code", "message", "valid_options", "try_call"]
    assert "try_call" not in ToolError("not_found", "no such").to_json()
