"""Lane P2e1: a served tool the Rust session refuses answers with that refusal -- its code,
message and field, with its way out (``valid_options``, ``try_call``); an id a change removed
is refused naming which change removed it, and the way on, worded by the session from its
retired-id record.  Made-up documents only."""

from __future__ import annotations

import datetime as dt
import pickle

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox, ToolError

import word_docs


CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
GONE = "p:0BADF00D"


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _markdown_document(markdown: str) -> bytes:
    """A blank document with ``markdown`` inserted by the toolbox, as bytes."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "made.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
        return word_docs.saved(session, doc)


@pytest.fixture(scope="module")
def report() -> bytes:
    return _markdown_document("# A made-up report\n\nLantern meadow by the harbour.\n\nRiver stone, river stone.\n\n"
                              "| Crop | Yield |\n| --- | --- |\n| Oats | 4 |\n\nThe last line.")


def _open(toolbox, data: bytes):
    session = toolbox.session(clock=CLOCK)
    return session, session.open(data, "report.docx")


def _ids(toolbox, session, doc) -> dict[str, str]:
    out: dict[str, str] = {}
    for block, text in word_docs.paragraphs(session, doc):
        out.setdefault(text, block)
    return out


def _refusing(ids: dict[str, str]) -> list[tuple[str, dict]]:
    """Calls the session refuses itself (its handler runs and refuses)."""
    meadow = ids["Lantern meadow by the harbour."]
    return [
        ("word_set_text", {"target": GONE, "text": "x"}),
        ("word_set_text", {"target": f"{meadow}@99:120", "text": "x"}),
        ("replace_text", {"find": "zz-nowhere-zz", "replace": "x"}),
        ("word_insert_text", {"at": f"{GONE}@0", "text": "x"}),
        ("word_insert_text", {"find": "nowhere at all", "text": "x"}),
        ("word_delete", {"target": GONE}),
        ("word_format", {"targets": [GONE], "bold": True}),
        ("word_lists", {"action": "restart", "targets": [meadow]}),
        ("word_insert_markdown", {"at": f"after:{GONE}", "markdown": "Text."}),
        ("word_move", {"blocks": GONE, "after": meadow}),
        ("word_sections", {"action": "set", "section": "s:nowhere", "orientation": "landscape"}),
        ("word_notes", {"action": "edit", "note": "fn:99", "text": "x"}),
        ("word_links", {"action": "add_bookmark", "target": meadow, "bookmark": "1bad"}),
        ("word_drawings", {"action": "set", "target": "d:999999", "wrap": "square"}),
    ]


def _reply(result) -> dict:
    error = result.to_json()["error"]
    return {key: error.get(key) for key in ("code", "message", "field")}


def test_refusals_are_the_sessions_answer(toolbox, report):
    """Each refusal answered by the session itself (its handler runs and refuses), with its
    code and message."""
    session, doc = _open(toolbox, report)
    for tool, arguments in _refusing(_ids(toolbox, session, doc)):
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert not result.ok, (tool, arguments)
        assert result.error.code and result.error.message, (tool, arguments, result.to_json())


def test_a_refusal_carries_its_way_out(toolbox, report):
    """The session's way out reaches the reply: the call that would work (tool rule 2)."""
    session, doc = _open(toolbox, report)
    result = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": GONE, "text": "x"})
    error = result.to_json()["error"]
    assert error["code"] == "not_found" and error["try_call"]["tool"], error


def test_a_deleted_id_names_the_change(toolbox, report):
    """An id a served call deleted is refused as docx-agent's view refused it: which change
    removed it, and the way on -- from the session's record."""
    session, doc = _open(toolbox, report)
    target = _ids(toolbox, session, doc)["Lantern meadow by the harbour."]
    assert toolbox.dispatch(session, "word_delete", {"doc": doc, "target": target}).ok
    result = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": "x"})
    assert not result.ok and result.error.code == "not_found", result.to_json()
    assert f"{target} was deleted by word_delete" in result.error.message, result.error.message


def test_a_read_refusal_is_the_sessions_answer(toolbox, report):
    """A read the session serves answers its refusal too."""
    session, doc = _open(toolbox, report)
    result = toolbox.dispatch(session, "find_text", {"doc": doc, "text": "meadow", "range": GONE})
    assert not result.ok and result.error.code == "not_found", result.to_json()


def test_try_call_pickles_and_serialises():
    error = ToolError("not_found", "no such", valid_options=["p:1"],
                      try_call={"tool": "word_read", "arguments": {"doc": "d1"}})
    again = pickle.loads(pickle.dumps(error))
    assert again.to_json() == error.to_json()
    assert list(error.to_json()) == ["code", "message", "valid_options", "try_call"]
    assert "try_call" not in ToolError("not_found", "no such").to_json()
