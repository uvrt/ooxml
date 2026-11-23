"""No call waits for ever while others hold the document (round 34, R17), on the Rust session
(P4e, lane CUT-D: from ``test_layout_deadlock.py``, whose slowed Python layout engine is gone).

Lone checks lay the document out while calls that hold it -- a ``read_batch``'s reads, a
``batch``'s ops, ``word_inspect`` with the layout, a fields update -- ask for the same
layout.  Before round 34, the owner of a layout waited for the lock held by a caller waiting
for that layout, and every call on the document hung.  Here the calls meet by numbers rather
than by a stalled engine: several lone checks and the holding calls start at once, for
several rounds.

Every test runs its calls in daemon threads joined with a deadline: a deadlock fails the test
and never hangs the run.  Made-up documents only (:func:`test_session_checks.report`).
"""

from __future__ import annotations

import threading
import time

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from test_session_checks import long_paragraphs, report
from word_docs import paragraph_ids

pytestmark = pytest.mark.any_faces

#: How long the calls of one test may take in all before it calls them deadlocked.
DEADLINE = 120.0
#: Lone checks started beside the holding calls in each round.
LONE = 4
#: Rounds of each test.
ROUNDS = 3


@pytest.fixture(scope="module")
def made() -> bytes:
    return report(3, 3)


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _all_finish(box, session, first, then, *, lone: int = LONE) -> dict:
    """``lone`` calls ``first`` (name, arguments) and the calls ``then``, all started at
    once.  Every result, by name; fails -- never hangs -- if any call is still running at
    the deadline."""
    out: dict[str, list] = {}
    lock = threading.Lock()
    go = threading.Barrier(lone + len(then))

    def call(name, arguments):
        go.wait(DEADLINE)
        result = box.dispatch(session, name, arguments)
        with lock:
            out.setdefault(name, []).append(result)

    threads = [threading.Thread(target=call, args=first, daemon=True) for _ in range(lone)]
    threads += [threading.Thread(target=call, args=c, daemon=True) for c in then]
    for thread in threads:
        thread.start()
    ends = time.monotonic() + DEADLINE
    for thread in threads:
        thread.join(max(0.0, ends - time.monotonic()))
    hung = [t for t in threads if t.is_alive()]
    if hung:
        pytest.fail(f"deadlock: {len(hung)} of {len(threads)} calls still waiting after {DEADLINE:g} s")
    return out


def _ok(results) -> None:
    bad = [r.to_json() for rs in results.values() for r in rs if not r.ok]
    assert not bad, bad


def test_lone_checks_and_a_read_batch_of_check_and_inspect_all_finish(made, box):
    """The production hang: lone checks lay the version out while a read_batch of check and
    word_inspect, holding the document, asks for the same layout."""
    session = box.session()
    doc = session.open(made, "report.docx")
    target = paragraph_ids(box, session, doc)[2]
    check = {"tool": "check", "arguments": {"doc": doc, "include": ["reflow"]}}
    inspect = {"tool": "word_inspect", "arguments": {"doc": doc, "target": target, "layout": True}}
    for _ in range(ROUNDS):
        out = _all_finish(box, session, ("check", {"doc": doc, "include": ["reflow"]}),
                          [("read_batch", {"ops": [check, inspect]}), ("read_batch", {"ops": [inspect, check]})])
        _ok(out)
        pages = {r.data["reflow"]["pages"] for r in out["check"]}
        for batch in out["read_batch"]:
            answers = batch.data["ops"]
            assert not any("error" in answer for answer in answers), batch.to_json()
            pages |= {a["data"]["reflow"]["pages"] for a in answers if a["tool"] == "check"}
        assert len(pages) == 1


def test_lone_checks_and_a_lone_inspect_all_finish(made, box):
    """word_inspect alone lays out holding the document too (its placements)."""
    session = box.session()
    doc = session.open(made, "report.docx")
    target = paragraph_ids(box, session, doc)[2]
    for _ in range(ROUNDS):
        _ok(_all_finish(box, session, ("check", {"doc": doc, "include": ["reflow"]}),
                        [("word_inspect", {"doc": doc, "target": target, "layout": True})] * 2))


def test_lone_checks_and_a_batch_with_a_check_between_changes_all_finish(made, box):
    """A batch's check before and between its changes lays out holding the document."""
    session = box.session()
    doc = session.open(made, "report.docx")
    body = long_paragraphs(session, doc)
    for k in range(ROUNDS):
        batch = {"ops": [{"tool": "check", "arguments": {"doc": doc, "include": ["reflow"]}},
                         {"tool": "word_set_text", "arguments": {"doc": doc, "target": body[0], "text": f"One {k}."}},
                         {"tool": "check", "arguments": {"doc": doc, "include": ["reflow"]}},
                         {"tool": "word_set_text", "arguments": {"doc": doc, "target": body[1], "text": f"Two {k}."}}]}
        _ok(_all_finish(box, session, ("check", {"doc": doc, "include": ["reflow"]}), [("batch", batch)]))


def test_lone_checks_and_a_fields_update_all_finish(made, box):
    """word_fields update beside lone checks of the same version."""
    session = box.session()
    doc = session.open(made, "report.docx")
    for _ in range(ROUNDS):
        _ok(_all_finish(box, session, ("check", {"doc": doc, "include": ["reflow"]}),
                        [("word_fields", {"doc": doc, "action": "update"}),
                         ("read_batch", {"ops": [{"tool": "check", "arguments": {"doc": doc}}]})]))


def test_many_rounds_of_checks_reads_and_edits_all_finish(made, box):
    """Nine callers at once, as a report's writers: lone checks, read_batches, inspects
    and edits, a few rounds."""
    session = box.session()
    doc = session.open(made, "report.docx")
    body = long_paragraphs(session, doc)
    target = body[0]
    reads = {"ops": [{"tool": "check", "arguments": {"doc": doc, "include": ["reflow"]}},
                     {"tool": "word_inspect", "arguments": {"doc": doc, "target": target, "layout": True}}]}
    for k in range(ROUNDS):
        _ok(_all_finish(box, session, ("check", {"doc": doc, "include": ["reflow"]}), [
            ("read_batch", reads), ("word_inspect", {"doc": doc, "target": target, "layout": True}),
            ("word_set_text", {"doc": doc, "target": body[1 + k], "text": f"Round {k}."}),
            ("check", {"doc": doc}), ("read_batch", reads),
            ("batch", {"ops": [{"tool": "check", "arguments": {"doc": doc}},
                               {"tool": "word_set_text",
                                "arguments": {"doc": doc, "target": body[4 + k], "text": "Shorter."}}]}),
        ], lone=3))
