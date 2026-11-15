"""The edit path's passes made per change (0.1.116, :mod:`docx_agent.incremental`) give what the
whole passes give: the same tool results, the same saved bytes and id state after every step.

A made-up report is edited by two writers' sequence -- Markdown inserts with lists and a table,
text set, a table's rows added and filled, a paragraph deleted, a batch, undo and redo, checks
(the Rust owner following each version where ``ooxml-core`` is there) -- once with every pass
whole (``DOCX_AGENT_INCREMENTAL=0``, as 0.1.115) and once per change with the whole pass run
beside each and compared (``DOCX_AGENT_FULL_VALIDATE=1``: a difference raises).
"""

from __future__ import annotations

import pytest

pytest.importorskip("ooxml_edit.tools.numbers", reason="needs ooxml-edit (numbers)")

from ooxml_edit.tools import Toolbox  # noqa: E402
from ooxml_edit.tools.limits import Limits  # noqa: E402

from docx_agent import incremental  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS  # noqa: E402

from test_rust_check import CLOCK, INCLUDE, TOOLS, WORDS, _first_difference, _ids, _normal, report  # noqa: E402,F401,F811

pytestmark = pytest.mark.any_faces


def _sequence(session, doc):
    chapters, heads, ends, longs = _ids(session, doc)
    tables = [t for t in session.entry(doc).document.tables() if t.column_count == 3]
    one = f"{chapters[0].id}..{ends[0].id}"
    long_text = " ".join(WORDS * 2) + "."
    return [
        ("a", "check", {"range": [one], "include": INCLUDE}),
        ("a", "word_insert_markdown", {"at": f"before:{heads[3].id}",
                                       "markdown": f"### Note\n\n{long_text}\n\n- one\n- two\n\n1. first\n2. second"}),
        ("b", "set_text", {"target": longs[3].id, "text": longs[3].text + " " + long_text}),
        ("a", "check", {"range": [one], "include": INCLUDE}),
        ("b", "word_edit_table", {"table": tables[1].id, "action": "insert_row", "index": 2}),
        ("b", "word_edit_table", {"table": tables[1].id, "action": "set_cells",
                                  "cells": [{"row": 2, "col": 0, "text": "Added"}, {"row": 1, "col": 1, "text": "7"}]}),
        ("b", "undo", {}),
        ("b", "undo", {"redo": True}),
        ("b", "word_delete", {"target": longs[5].id}),
        ("a", "batch", {"ops": [
            {"tool": "word_insert_markdown", "arguments": {"at": f"before:{ends[1].id}",
                                                           "markdown": "1. again\n2. and again"}},
            {"tool": "set_text", "arguments": {"target": longs[7].id, "text": "Shorter now."}}]}),
        ("b", "check", {"range": [one], "include": INCLUDE}),
        ("a", "undo", {}),
        ("a", "undo", {"redo": True}),
        ("a", "undo", {}),
        ("b", "word_insert_markdown", {"at": f"before:{heads[1].id}",
                                       "markdown": "| A | B |\n|---|---|\n| 1 | 2 |\n\n- item\n- item"}),
        ("a", "check", {"range": [one], "include": INCLUDE}),
    ]


def _run(data: bytes, per_change: bool, monkeypatch, oracle: bool = True) -> tuple[list, dict]:
    monkeypatch.setenv("DOCX_AGENT_INCREMENTAL", "1" if per_change else "0")
    monkeypatch.setenv("DOCX_AGENT_FULL_VALIDATE", "1" if per_change and oracle else "0")
    incremental.STATS.clear()
    out = []
    limits = Limits(render_timeout=600, layout_timeout=600, batch_timeout=900)
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=1, limits=limits) as box:
        session = box.session(clock=CLOCK)
        doc = session.open(data, "report.docx")
        for writer, tool, arguments in _sequence(session, doc):
            view = session.view(context={"agent": writer})
            if tool == "batch":
                arguments = {"ops": [{"tool": op["tool"], "arguments": {"doc": doc, **op["arguments"]}}
                                     for op in arguments["ops"]]}
                result = box.dispatch(view, tool, arguments).to_json()
            else:
                result = box.dispatch(view, tool, {"doc": doc, **arguments}).to_json()
            document = session.entry(doc).document
            out.append((tool, arguments, result, document.to_bytes(), document.id_state()))
    return out, dict(incremental.STATS)


@pytest.mark.parametrize("oracle", [True, False], ids=["compared", "alone"])
def test_passes_per_change_give_what_the_whole_passes_give(report, monkeypatch, oracle):  # noqa: F811
    mine, stats = _run(report, True, monkeypatch, oracle)
    theirs, _ = _run(report, False, monkeypatch)
    assert len(mine) == len(theirs)
    for (tool, arguments, a, data_a, ids_a), (_, _, b, data_b, ids_b) in zip(mine, theirs):
        assert a.get("ok", True), (tool, arguments, a)
        assert _normal(a) == _normal(b), (tool, arguments, _first_difference(_normal(a), _normal(b)))
        assert data_a == data_b, (tool, arguments)
        assert ids_a == ids_b, (tool, arguments)
    # each pass was made per change where it could be
    for name in ("ids note: incremental", "touched ids: incremental", "body index: per change",
                 "restore: per block", *(() if oracle else ("used ids: pool",))):
        assert stats.get(name, 0) >= 1, (name, stats)
