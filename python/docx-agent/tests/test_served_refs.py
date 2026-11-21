"""Lane H2: a ref name the tool layer folds (camelCase, hyphenated) or refuses, in a call the
Rust session serves: served, with the reply -- refs, warnings, refusals -- the Python
handler's.  A name that folds to none, or to another address's ref, is refused before the
edit, so nothing changes.  Made-up documents only."""

from __future__ import annotations

import pytest

from docx_agent import Document
from docx_agent.rust import edits, reads
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry


def _report() -> bytes:
    doc = Document.new()
    doc.insert_markdown("# A made-up report\n\nLantern meadow by the harbour.\n\nRiver stone number one.")
    return doc.to_bytes()


def _run(calls: list[tuple[str, dict]], owned: bool) -> list[dict]:
    """Each call's reply, as compared: ok, error, warnings, refs; and the document's version
    after it."""
    before = None if owned else reads.use_session(None)
    try:
        with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
            session = box.session()
            doc = session.open(_report(), "report.docx")
            if owned and not isinstance(session.entry(doc), OwnedEntry):
                pytest.skip("no Rust session to own the document")
            out = []
            for tool, arguments in calls:
                result = box.dispatch(session, tool, {"doc": doc, **arguments})
                out.append({"ok": result.ok, "error": None if result.error is None else result.error.to_json(),
                            "warnings": [w for w in result.warnings if "ref" in w], "refs": sorted(result.refs),
                            "version": session.entry(doc).version})
            return out
    finally:
        if not owned:
            reads.use_session(before)


CALLS = [
    ("word_insert_markdown", {"at": "end", "markdown": "A closing line.", "ref": "closingLine"}),
    ("word_set_text", {"target": "$closing_line", "text": "A closing line, again."}),
    ("word_insert_text", {"items": [{"find": "meadow", "text": " green", "ref": "green-word"},
                                    {"find": "stone", "text": " grey", "ref": "grey_word"}]}),
    # folds to none: refused, nothing made
    ("word_insert_markdown", {"at": "end", "markdown": "Never made.", "ref": "Closing Line!"}),
    # folds to a ref naming another address: refused, nothing made
    ("word_insert_markdown", {"at": "end", "markdown": "Never made either.", "ref": "closing-line"}),
]


def test_folded_ref_names_are_served_as_the_python_handler_answers():
    served = sum(edits.served().values())
    rust = _run(CALLS, owned=True)
    # every call answered there: the two refused too -- a refusal is the call's answer, not a
    # fallback (lane P2e1)
    assert sum(edits.served().values()) - served == 5, edits.fallbacks()
    assert not any("ref name" in why for reasons in edits.fallbacks().values() for why in reasons)
    python = _run(CALLS, owned=False)
    for (tool, _arguments), ours, theirs in zip(CALLS, rust, python):
        ours, theirs = dict(ours), dict(theirs)
        assert ours.pop("version") - rust[0]["version"] == theirs.pop("version") - python[0]["version"], tool
        assert ours == theirs, tool
    assert rust[0]["warnings"] == ["ref 'closingLine' read as 'closing_line': write $closing_line"]
    assert rust[0]["refs"] == ["closing_line"]
    assert [r["ok"] for r in rust] == [True, True, True, False, False]
    assert rust[3]["version"] == rust[4]["version"] == rust[2]["version"], "a refused ref changes nothing"


def test_a_tracking_mode_the_session_refuses_is_the_calls_refusal():
    """(Lane H2.)  The host keeps only a mode ``word_set_tracking`` took, so the session never
    refuses the one it is given; were it to, its refusal is the call's -- nothing changed, no
    Python handler."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(_report(), "report.docx")
        entry = session.entry(doc)
        if not isinstance(entry, OwnedEntry):
            pytest.skip("no Rust session to own the document")
        target = entry.document.paragraphs()[1].id
        version, fallbacks = entry.version, edits.fallbacks()
        entry.tracking = {"on": True}           # (no author: what word_set_tracking refuses)
        result = box.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": "Changed."})
        assert not result.ok and result.error.field == "author", result.to_json()
        assert entry.version == version and edits.fallbacks() == fallbacks
