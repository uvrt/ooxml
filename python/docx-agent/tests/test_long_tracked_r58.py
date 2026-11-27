"""Round 58: a long report with many tracked changes -- what the reads of it say, and where a
call's time went, on the Rust session.

* ``describe``'s page count, ``render``'s range pages and ``save_document``'s coverage are the
  layout's the session made for the document (``check``'s).
* ``describe``'s revision authors are counted as each record says.
* Every result says where its time went (``Result.timing``).

Made-up documents only (``tests/tracked_report.py``)."""

from __future__ import annotations

import threading
from collections import Counter

import pytest

from ooxml_edit.tools import Toolbox
from ooxml_edit.tools import timing as timing_module

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs
from tracked_report import build

pytestmark = pytest.mark.any_faces


@pytest.fixture(scope="module")
def report() -> bytes:
    return build(chapters=2, seed=3)[0]


def _box(**options):
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, **options)


# -- the reads' facts are the layout's ----------------------------------------------------------


@pytest.mark.parametrize("workers", [0, 1])
def test_describe_render_and_save_read_their_facts_from_the_documents_layout(report, workers):
    with _box(workers=workers) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        table = [block for block, _ in word_docs.blocks(box, session, doc) if block.startswith("t")][1]
        laid = box.dispatch(session, "check", {"doc": doc, "include": ["reflow"]})
        assert laid.ok, laid.to_json()
        reflow = laid.data["reflow"]
        checked = box.dispatch(session, "check", {"doc": doc, "range": [table]})
        assert checked.ok, checked.to_json()
        described = box.dispatch(session, "describe", {"doc": doc})
        assert described.ok, described.to_json()
        assert described.data["pages"] == reflow["pages"]
        rendered = box.dispatch(session, "render", {"doc": doc, "range": [table]})
        assert rendered.ok, rendered.to_json()
        assert rendered.data["range"][0]["pages"] == checked.data["range"][0]["pages"]
        saved = box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx", "format": "docx"})
        assert saved.ok, saved.to_json()
        assert saved.data["coverage"] == reflow["coverage"]


# -- describe's counts --------------------------------------------------------------------------


_RECORDS = ("ins", "del", "rPrChange", "pPrChange", "trPrChange", "tcPrChange", "tblPrChange", "sectPrChange",
            "moveFrom", "moveTo", "cellIns", "cellDel", "cellMerge", "numberingChange")


def test_describe_counts_authors_as_each_revision_says(report):
    root = word_docs.tree(report)
    author = f"{{{word_docs.W}}}author"
    expected = Counter(node.get(author) for node in root.iter()
                       if node.get(author) and node.tag.rpartition("}")[2] in _RECORDS)
    with _box(workers=0) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        described = box.dispatch(session, "describe", {"doc": doc})
    assert described.ok, described.to_json()
    assert described.data["revisions"]["by_author"] == dict(expected)


# -- where a call's time went -------------------------------------------------------------------


def test_every_result_says_where_its_time_went(report):
    with _box(workers=0) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        result = box.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Writer"})
        timing = result.timing
        assert set(timing) == {"total", "lock_wait", "work", "cpu", "held"}
        assert timing["total"] == pytest.approx(timing["lock_wait"] + timing["work"], abs=1e-3)
        assert 0 <= timing["held"] <= timing["total"]       # (the commit lock's, on the Rust host)
        assert "timing" not in result.to_json()          # the application's, not the model's
        failed = box.dispatch(session, "word_format", {"doc": doc, "targets": ["p:NOPE"], "bold": True})
        assert not failed.ok and failed.timing is not None


def test_a_timed_lock_counts_the_wait_and_the_outermost_hold(monkeypatch):
    now = [0.0]
    monkeypatch.setattr(timing_module.time, "perf_counter", lambda: now[0])
    lock = timing_module.TimedLock()
    waited, held = timing_module.clock()
    with lock:
        now[0] += 2.0
        with lock:                      # taken again: no wait, the hold goes on
            now[0] += 1.0
        assert lock._is_owned()
    after = timing_module.clock()
    assert after[0] - waited == 0.0 and after[1] - held == 3.0
    # Another thread holding it: this one's wait is the time until it lets go -- the holder
    # lets go once this thread has read the clock to start waiting.
    taken, letting_go = threading.Event(), threading.Event()
    main = threading.get_ident()

    def ticking() -> float:
        value = now[0]
        if threading.get_ident() == main and taken.is_set():
            letting_go.set()
        return value

    monkeypatch.setattr(timing_module.time, "perf_counter", ticking)

    def holder():
        with lock:
            taken.set()
            letting_go.wait()
            now[0] += 5.0

    thread = threading.Thread(target=holder)
    thread.start()
    taken.wait()
    with lock:
        pass
    thread.join()
    assert timing_module.clock()[0] - after[0] == 5.0


# -- letting go ---------------------------------------------------------------------------------


def test_closing_a_document_lets_go_of_it(report):
    with _box(workers=0) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        described = box.dispatch(session, "describe", {"doc": doc})
        assert described.memory is not None        # the application's figure
        assert "memory" not in described.to_json()["data"]
        session.close(doc)
        with pytest.raises(Exception):
            session.save(doc)
