"""Lane P2c17: the last editing group served by the Rust session -- ``word_set_tracking``,
``word_changes``, ``word_comments``, ``word_controls``, ``word_edit_table`` (the actions
that write no rows) and ``word_copy_from`` (from the document itself or another the session
holds); their replies as docx-agent's own handler gives them, refs they define and use
included.  Both sides date with the system's clock (the Python side with the tools taken
out of the served set), so dates are compared blanked.  Made-up documents only."""

from __future__ import annotations

import dataclasses
import json
import re

import pytest

from docx_agent import Document
from docx_agent.rust import edits
from docx_agent.rust import served as served_tools
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

#: The tools this file serves.
GROUP = ("word_set_tracking", "word_changes", "word_comments", "word_controls", "word_edit_table",
         "word_copy_from")

_DATE = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ?")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _report(title: str) -> bytes:
    doc = Document.new()
    parts = [f"# {title}"]
    for c in range(1, 4):
        parts += [f"# Part {c}", f"Lantern meadow {c} by the harbour.", f"River stone number {c}."]
    parts.append("| Name | Value | Note |\n|---|---|---|\n| Lantern | 1 | a |\n| Meadow | 2 | b |\n"
                 "| River | 3 | c |")
    doc.insert_markdown("\n\n".join(parts))
    return doc.to_bytes()


@pytest.fixture(scope="module")
def report() -> bytes:
    return _report("A made-up report")


@pytest.fixture(scope="module")
def other() -> bytes:
    return _report("Another made-up report")


def _calls(toolbox, monkeypatch, data: bytes, other: bytes, served: bool, calls) -> list:
    """Each call's reply (dates blanked); where ``served``, each changing one the session
    answered (counted) but those ``calls`` says are the Python handler's (``python``: true;
    ``None`` for a reading call, which is not an edit)."""
    with monkeypatch.context() as patched:
        if not served:
            hosting = served_tools.hosting()
            fewer = dataclasses.replace(hosting, served=hosting.served - set(GROUP))
            patched.setattr(served_tools, "_HOSTING", [fewer])
        session = toolbox.session()
        doc = session.open(data, "report.docx")
        second = session.open(other, "other.docx")
        if served and not isinstance(session.entry(doc), OwnedEntry):
            pytest.skip("no Rust session to own the document")
        out = []
        for tool, arguments, python in calls(session, doc, second):
            before = dict(edits.served())
            result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
            assert result.ok, (tool, arguments, result.error)
            if served and python is not None:
                answered = edits.served().get(tool, 0) - before.get(tool, 0)
                assert answered == (0 if python else 1), (tool, arguments, edits.fallbacks())
            reply = [tool, result.summary, result.data, sorted(result.changed), result.warnings,
                     dict(result.refs)]
            out.append(json.loads(_DATE.sub("<date>", json.dumps(reply, sort_keys=True))))
        out.append(json.loads(_DATE.sub("<date>", json.dumps(session.entry(doc).document.state(),
                                                                 sort_keys=True, default=str))))
        return out


def _ids(session, doc) -> dict[str, str]:
    """Each paragraph's id by its text."""
    return {p.text: p.id for p in session.entry(doc).document.paragraphs()}


def _review_calls(session, doc, second):
    ids = _ids(session, doc)
    yield "word_set_tracking", {"on": True, "author": "Reviewer A"}, False
    yield "word_set_text", {"target": ids["River stone number 1."], "text": "River pebble number 1."}, False
    yield "word_set_text", {"target": ids["River stone number 2."], "text": "River pebble number 2."}, False
    yield "word_changes", {"action": "list"}, None
    yield "word_changes", {"action": "preview", "all": True}, None
    yield "word_changes", {"action": "accept", "all": True, "within": ids["River stone number 1."]}, False
    yield "word_comments", {"action": "add", "items": [
        {"find": "meadow 2", "text": "A made-up comment.", "ref": "first"},
        {"target": ids["Part 3"], "text": "Another one."}]}, False
    yield "word_set_tracking", {"on": False, "word_switch": True}, False
    yield "word_changes", {"action": "reject", "all": True}, False
    yield "word_comments", {"action": "reply", "items": [{"comment": "$first", "text": "A reply.",
                                                          "resolve": True}]}, False
    yield "word_comments", {"action": "list"}, None
    yield "word_comments", {"action": "reopen", "items": [{"comment": "$first"}]}, False
    yield "word_comments", {"action": "edit", "items": [{"comment": "$first", "text": "Edited."}]}, False
    yield "word_controls", {"action": "insert", "at": ids["Lantern meadow 3 by the harbour."], "tag": "made-up",
                            "title": "A field"}, False
    yield "word_controls", {"action": "insert", "find": "number 3", "type": "checkbox"}, False
    yield "word_controls", {"action": "list"}, None


def _table_calls(session, doc, second):
    ids = _ids(session, doc)
    table = session.entry(doc).document.tables()[0].id
    yield "word_edit_table", {"table": table, "action": "insert_column", "index": 1}, False
    yield "word_edit_table", {"table": table, "action": "delete_row", "index": 2}, False
    yield "word_edit_table", {"table": table, "action": "merge", "first": {"row": 1, "col": 0},
                              "last": {"row": 1, "col": 1}}, False
    yield "word_edit_table", {"table": table, "action": "split", "first": {"row": 1, "col": 0}}, False
    yield "word_edit_table", {"table": table, "action": "delete_column", "index": 3}, False
    # rows written, looked at against the rows around them in the session too (R126)
    yield "word_edit_table", {"table": table, "action": "insert_row", "index": 0}, False
    yield "word_edit_table", {"table": table, "action": "copy_row", "index": 2}, False
    yield "word_edit_table", {"table": table, "action": "set_cells",
                              "cells": [{"row": 1, "col": 0, "text": "Pebble"}, {"row": 3, "col": 1, "text": "7"}]}, False
    their = _ids(session, second)
    yield "word_copy_from", {"source_doc": second, "range": their["Part 2"] + ".." + their["River stone number 2."],
                             "at": "after:" + ids["Part 1"], "ref": "copied"}, False
    yield "word_copy_from", {"source_doc": doc, "section_heading": ids["Part 3"], "at": "end"}, False
    yield "word_delete", {"target": "$copied", "force": True}, False


@pytest.mark.parametrize("calls", [_review_calls, _table_calls], ids=["review", "tables"])
def test_the_last_group_served_as_the_python_handler_answers(toolbox, monkeypatch, report, other, calls):
    served = _calls(toolbox, monkeypatch, report, other, True, calls)
    assert served == _calls(toolbox, monkeypatch, report, other, False, calls)


def test_tracking_set_by_a_served_call_is_the_documents(toolbox, report):
    session = toolbox.session()
    doc = session.open(report, "report.docx")
    entry = session.entry(doc)
    if not isinstance(entry, OwnedEntry):
        pytest.skip("no Rust session to own the document")
    on = toolbox.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Reviewer B"})
    assert on.ok, on.error
    assert entry.tracking == {"on": True, "author": "Reviewer B"}
    assert entry.handle.tracking == entry.tracking
    target = _ids(session, doc)["River stone number 3."]
    written = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": "River pebble."})
    assert written.ok, written.error
    listed = toolbox.dispatch(session, "word_changes", {"doc": doc, "action": "list"})
    assert "Reviewer B" in json.dumps(listed.data)
    off = toolbox.dispatch(session, "word_set_tracking", {"doc": doc, "on": False})
    assert off.ok, off.error
    assert entry.tracking is None and entry.handle.tracking is None

