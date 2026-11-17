"""Round 58: where a call's time went (``Result.timing``, :mod:`ooxml_edit.tools.timing`),
what a history keeps (``History.kept_bytes``), and a format letting go of a closed document
(``DocumentFormat.closed``).  Toy documents only."""

from __future__ import annotations

import dataclasses

import synthetic
from ooxml_edit import opc
import tools_toys
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.memory import process_memory
from ooxml_edit.tools.timing import TimedLock, clock


def test_a_result_says_where_its_time_went_and_the_model_never_sees_it():
    with Toolbox(tools_toys.TOOLS, formats=tools_toys.FORMATS, groups=tools_toys.GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(synthetic.outer_package(), "a.docx")
        result = box.dispatch(session, "toy_word_insert", {"doc": doc, "text": "one", "at": "end"})
        assert result.ok
        timing = result.timing
        assert set(timing) == {"total", "lock_wait", "work", "cpu", "held"}
        assert 0 < timing["held"] <= timing["total"]
        assert "timing" not in result.to_json()
        assert isinstance(session.entry(doc).lock, TimedLock)


def test_a_thread_holding_a_document_lock_is_counted_until_it_lets_go():
    lock = TimedLock()
    _, before = clock()
    with lock:
        with lock:
            assert lock._is_owned()
        inside = clock()[1]
    after = clock()[1]
    assert before <= inside <= after
    assert not lock._is_owned()


def test_a_history_counts_each_kept_piece_once():
    document = tools_toys.ToyDocument(synthetic.outer_package())
    assert document.history.kept_bytes() == 0
    for k in range(3):
        document.insert(f"item {k}", "end")
    kept = document.history.kept_bytes()
    assert 0 < kept
    # Each step's snapshot shares the pieces of the one before: three steps hold less than
    # three whole copies of the part.
    whole = len(document.package.read(synthetic.MAIN))
    assert kept < 3 * whole


def test_closing_a_document_tells_its_format():
    closed = []
    text = dataclasses.replace(tools_toys.TEXT, closed=closed.append)
    with Toolbox(tools_toys.TOOLS, formats=[tools_toys.DECK, text], groups=tools_toys.GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(synthetic.outer_package(), "a.docx")
        document = session.entry(doc).document
        session.close(doc)
    assert closed == [document]


def test_the_process_memory_is_a_figure_in_megabytes():
    found = process_memory()
    assert all(isinstance(value, float) and value > 0 for value in found.values())


def test_closing_a_document_empties_its_entry_and_gives_the_memory_back(monkeypatch):
    """Q3: the entry's undo history, read copy, bytes and caches go on close; then the
    memory is given back (a collection, the allocator's free pages), the entry let go."""
    from ooxml_edit.tools import memory, session as session_module

    reclaimed = []
    monkeypatch.setattr(session_module, "reclaim", lambda: reclaimed.append(True))
    with Toolbox(tools_toys.TOOLS, formats=tools_toys.FORMATS, groups=tools_toys.GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(synthetic.outer_package(), "a.docx")
        for k in range(2):
            assert box.dispatch(session, "toy_word_insert", {"doc": doc, "text": f"item {k}", "at": "end"}).ok
        entry = session.entry(doc)
        entry.snapshot()
        entry.check_cache.put("key", "found")
        assert entry.history.can_undo() and entry._snapshot is not None
        opc._JOINED.append((opc.Pieces((b"<a>", b"</a>")), b"<a></a>"))   # kept by bytes, any document's
        session.close(doc)
        assert reclaimed == [True] and not opc._JOINED
        assert not entry.history.can_undo() and entry.committed is None and entry._snapshot is None
        assert not len(entry.check_cache) and not entry.changes_kept
        assert entry.log                                  # the transcript's records stay
    memory.reclaim()                                      # runs anywhere: nothing to ask is no error
