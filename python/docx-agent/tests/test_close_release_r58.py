"""Round 58 (Q3): a closed document -- and one let go without closing -- leaves nothing in the
calling process: the entry's history, read copy and caches are emptied on ``Session.close``,
and nothing keeps the entry from being collected (the Rust owner's finalizer held it, through
the owner's mirror and its read copy, for as long as the process ran)."""

from __future__ import annotations

import gc
import weakref

import pytest

from ooxml_edit import opc
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from tracked_report import build

# The Python host's own serving of a Word document (its adapters, owner and mirror), which Phase 3 E2
# deletes with these tests; their calls' answers are the host goldens' (replayed by the Rust host).
pytestmark = [pytest.mark.any_faces, pytest.mark.python_host]


@pytest.fixture(scope="module")
def report() -> bytes:
    return build(chapters=2, seed=3)[0]


def first_chapter(box, session, doc) -> dict:
    """The first chapter's heading and end paragraphs, and its first table with its body rows'
    cells (by address: the report's paragraphs have no paraIds)."""
    out: dict = {"head": None, "end": None, "table": None, "cells": []}
    for block, text in word_docs.blocks(box, session, doc):
        plain = text.lstrip("# ")
        if block.startswith("p") and plain.startswith("Chapter ") and out["head"] is None:
            out["head"] = block
        elif block.startswith("p") and plain.startswith("End of chapter ") and out["end"] is None:
            out["end"] = block
            return out
        elif block.startswith("t") and out["head"] is not None and out["table"] is None:
            out["table"] = block
            out["cells"] = [f"{block}/c{r},{c}" for r in range(1, 10) for c in range(4)]
    return out


def _edited_and_checked(box, session, report):
    """A document opened with tracking on, edited twice, described and one chapter checked;
    its id."""
    doc = session.open(report, "report.docx")
    assert box.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "W"}).ok
    chapter = first_chapter(box, session, doc)
    inserted = box.dispatch(session, "word_edit_table", {"doc": doc, "table": chapter["table"],
                                                         "action": "insert_row", "index": 2})
    assert inserted.ok, inserted.to_json()
    # The first edit stamps the report's ids: its cells by the table's new name.
    table = inserted.renamed.get(chapter["table"], chapter["table"])
    cells = [cell.replace(chapter["table"], table, 1) for cell in chapter["cells"][:6]]
    formatted = box.dispatch(session, "word_format", {"doc": doc, "targets": cells, "paragraph_style": "Table Number"})
    assert formatted.ok, formatted.to_json()
    assert box.dispatch(session, "describe", {"doc": doc}).ok
    assert box.dispatch(session, "check", {"doc": doc, "range": [f"{chapter['head']}..{chapter['end']}"]}).ok
    return doc


def test_closing_empties_what_the_entry_keeps(report):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = _edited_and_checked(box, session, report)
        entry = session.entry(doc)
        # (The steps are the entry's history: the Rust session's.)
        assert entry.history.can_undo() and entry.committed is not None and len(entry.check_cache)
        session.close(doc)
        assert entry.committed is None
        assert entry._snapshot is None and not len(entry.check_cache) and not entry.changes_kept
        assert not opc._JOINED


def test_a_closed_document_is_collected(report):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = _edited_and_checked(box, session, report)
        gone = weakref.ref(session.entry(doc))
        session.close(doc)
        gc.collect()
        assert gone() is None
