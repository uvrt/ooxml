"""Lane P2c17: the last editing group served by the Rust session -- ``word_set_tracking``,
``word_changes``, ``word_comments``, ``word_controls``, ``word_edit_table`` and
``word_copy_from`` (from the document itself or another the session holds): each call of a
review and of a table's reshaping answered by the session, refs they define and use
included, and the document valid after.  Made-up documents only."""

from __future__ import annotations

import json

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _report(title: str) -> bytes:
    parts = [f"# {title}"]
    for c in range(1, 4):
        parts += [f"# Part {c}", f"Lantern meadow {c} by the harbour.", f"River stone number {c}."]
    parts.append("| Name | Value | Note |\n|---|---|---|\n| Lantern | 1 | a |\n| Meadow | 2 | b |\n"
                 "| River | 3 | c |")
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "made.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown="\n\n".join(parts))
        return word_docs.saved(session, doc)


@pytest.fixture(scope="module")
def report() -> bytes:
    return _report("A made-up report")


@pytest.fixture(scope="module")
def other() -> bytes:
    return _report("Another made-up report")


def _calls(toolbox, data: bytes, other: bytes, calls) -> None:
    """Each call ok; the document as valid after as it opened (no new validation problem)."""
    session = toolbox.session()
    doc = session.open(data, "report.docx")
    second = session.open(other, "other.docx")
    for tool, arguments, _changing in calls(toolbox, session, doc, second):
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, (tool, arguments, result.error)
    assert word_docs.validation(toolbox, session, doc)["new"] == []


def _ids(toolbox, session, doc) -> dict[str, str]:
    """Each block's id by its text (a heading's without its ``#``)."""
    out: dict[str, str] = {}
    for block, text in word_docs.paragraphs(session, doc):
        out.setdefault(text, block)
    for table in word_docs.tables(session, doc):
        out.setdefault(f"table {len(out)}", f"t:{word_docs.rows_of(table)[0].get(word_docs.W14_PARA)}")
    return out


def _review_calls(toolbox, session, doc, second):
    ids = _ids(toolbox, session, doc)
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


def _table_calls(toolbox, session, doc, second):
    ids = _ids(toolbox, session, doc)
    table = next(block for block in ids.values() if block.startswith("t:"))
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
    their = _ids(toolbox, session, second)
    yield "word_copy_from", {"source_doc": second, "range": their["Part 2"] + ".." + their["River stone number 2."],
                             "at": "after:" + ids["Part 1"], "ref": "copied"}, False
    yield "word_copy_from", {"source_doc": doc, "section_heading": ids["Part 3"], "at": "end"}, False
    yield "word_delete", {"target": "$copied", "force": True}, False


@pytest.mark.parametrize("calls", [_review_calls, _table_calls], ids=["review", "tables"])
def test_the_review_and_table_groups(toolbox, report, other, calls):
    _calls(toolbox, report, other, calls)


def test_tracking_set_by_a_call_is_the_documents(toolbox, report):
    session = toolbox.session()
    doc = session.open(report, "report.docx")
    on = toolbox.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Reviewer B"})
    assert on.ok, on.error
    # the mode is the session's alone: describe says it
    described = toolbox.dispatch(session, "describe", {"doc": doc})
    assert described.data["tracking"] == {"author": "Reviewer B"}, described.data
    target = _ids(toolbox, session, doc)["River stone number 3."]
    written = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": target, "text": "River pebble."})
    assert written.ok, written.error
    listed = toolbox.dispatch(session, "word_changes", {"doc": doc, "action": "list"})
    assert "Reviewer B" in json.dumps(listed.data)
    off = toolbox.dispatch(session, "word_set_tracking", {"doc": doc, "on": False})
    assert off.ok, off.error
    assert toolbox.dispatch(session, "describe", {"doc": doc}).data["tracking"] is None
