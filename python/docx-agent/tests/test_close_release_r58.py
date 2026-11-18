"""Round 58 (Q3): a closed document -- and one let go without closing -- leaves nothing in the
calling process: the entry's history, read copy and caches are emptied on ``Session.close``,
and nothing keeps the document from being collected (the Rust owner's finalizer held it,
through the owner's mirror and its read copy, for as long as the process ran)."""

from __future__ import annotations

import gc
import sys
import weakref
from pathlib import Path

import pytest

from ooxml_edit import opc
from ooxml_edit.tools import Toolbox

from docx_agent import blocks as blocks_module
from docx_agent import finalizers
from docx_agent.rust import check as rust_check
from docx_agent.tools import FORMAT, GROUPS, TOOLS

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))

from tracked_report import build  # noqa: E402
from tracked_report_bench import chapters_of  # noqa: E402

pytestmark = pytest.mark.any_faces


@pytest.fixture(scope="module")
def report() -> bytes:
    return build(chapters=2, seed=3)[0]


def _edited_and_checked(box, session, report):
    """A document opened with tracking on, edited twice, described and one chapter checked
    (a Rust owner made for it where ooxml-core is there); its id and its document."""
    doc = session.open(report, "report.docx")
    assert box.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "W"}).ok
    entry = session.entry(doc)
    chapter = chapters_of(entry.document)[0]
    table = chapter["tables"][0]
    assert box.dispatch(session, "word_edit_table", {"doc": doc, "table": table["id"],
                                                     "action": "insert_row", "index": 2}).ok
    assert box.dispatch(session, "word_format", {"doc": doc, "targets": table["cells"][:6],
                                                 "paragraph_style": "Table Number"}).ok
    assert box.dispatch(session, "describe", {"doc": doc}).ok
    assert box.dispatch(session, "check", {"doc": doc, "range": [f"{chapter['head']}..{chapter['end']}"]}).ok
    return doc, entry.document


def test_closing_empties_what_the_entry_keeps(report):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc, document = _edited_and_checked(box, session, report)
        entry = session.entry(doc)
        assert document.history.can_undo() and entry.committed is not None and len(entry.check_cache)
        session.close(doc)
        # What the application still holds stays usable, emptied of its history and copies.
        assert not document.history.can_undo() and entry.committed is None
        assert entry._snapshot is None and not len(entry.check_cache) and not entry.changes_kept
        assert not blocks_module._BODIES and not opc._JOINED and "_rust_doc" not in document.__dict__
        assert document.paragraphs()


def test_a_closed_document_is_collected(report):
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc, document = _edited_and_checked(box, session, report)
        if rust_check.core() is not None:
            assert document.__dict__.get("_rust_doc") is not None     # the owner that held it
        gone = weakref.ref(document)
        del document
        session.close(doc)
        gc.collect()
        assert gone() is None


def test_a_document_let_go_without_closing_is_collected(report):
    if rust_check.core() is None:
        pytest.skip("ooxml-core is not installed: no Rust owner")
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc, document = _edited_and_checked(box, session, report)
        owner = document.__dict__["_rust_doc"]
        finalizer = owner.finalizer
        _, func, args, _ = finalizer.peek()
        # The key alone, handed over: a finalizer does no work itself (lane L4b).
        assert func is finalizers.later and args == (rust_check._close_owner, owner.key)
        gone = weakref.ref(document)
        del owner, document
        session.documents.pop(doc)          # dropped by the application, never closed
        gc.collect()
        assert gone() is None and not finalizer.alive
        finalizers.drain()                                                  # the owner closed
