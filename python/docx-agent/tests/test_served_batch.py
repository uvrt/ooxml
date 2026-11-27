"""A batch of Word ops on the Rust host is one step: one undo takes it back, all or none where
atomic (undo and redo as they were where it fails), a failed op taken back alone where not, its
reads reading what its earlier ops made, a writer beside it losing nothing, and an id one of its
ops deleted refused naming the batch's change (lane H1, on the host)."""

import threading

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as box:
        yield box


@pytest.fixture
def opened(toolbox, constructs_doc):
    session = toolbox.session()
    doc = session.open(constructs_doc.read_bytes(), name=constructs_doc.name)
    paragraphs = [block for block, text in word_docs.paragraphs(session, doc) if text.strip() and block != "p:None"]

    def version() -> int:
        return toolbox.dispatch(session, "describe", {"doc": doc}).version

    yield session, doc, version, paragraphs
    session.close(doc)


def _texts(session, doc, ids):
    """The paragraphs' texts now, by id (``None``: gone)."""
    found = dict(word_docs.paragraphs(session, doc))
    return [found.get(i) for i in ids]


def _set(target, text):
    return {"tool": "word_set_text", "arguments": {"doc": "d1", "target": target, "text": text}}


def test_an_atomic_batch_is_one_step_and_one_undo_takes_it_back(toolbox, opened):
    session, doc, version, ids = opened
    targets = ids[:3]
    before, start = _texts(session, doc, targets), version()
    result = toolbox.dispatch(session, "batch", {"atomic": True,
                                                 "ops": [_set(t, f"Batch {k}") for k, t in enumerate(targets)]})
    assert result.ok, result.error
    assert version() == start + 1 == result.version
    assert _texts(session, doc, targets) == ["Batch 0", "Batch 1", "Batch 2"]
    assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    assert version() == start
    assert _texts(session, doc, targets) == before
    assert toolbox.dispatch(session, "undo", {"doc": doc, "redo": True}).ok
    assert _texts(session, doc, targets) == ["Batch 0", "Batch 1", "Batch 2"]


def test_a_failed_atomic_batch_leaves_undo_and_redo_as_they_were(toolbox, opened):
    session, doc, version, ids = opened
    assert toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": ids[0], "text": "Kept"}).ok
    assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    before, start = _texts(session, doc, ids[:3]), version()
    result = toolbox.dispatch(session, "batch", {"atomic": True, "ops": [
        _set(ids[1], "Gone 1"), _set(ids[2], "Gone 2"), _set("p:FFFFFFFF", "nowhere")]})
    assert not result.ok
    assert "nothing in the batch was applied" in result.error.message
    assert version() == start
    assert _texts(session, doc, ids[:3]) == before
    assert toolbox.dispatch(session, "undo", {"doc": doc, "redo": True}).ok
    assert _texts(session, doc, ids[:1]) == ["Kept"]


def test_a_partial_batch_keeps_the_ops_that_applied(toolbox, opened):
    session, doc, version, ids = opened
    before, start = _texts(session, doc, ids[:3]), version()
    result = toolbox.dispatch(session, "batch", {"ops": [
        _set(ids[0], "First"), _set("p:FFFFFFFF", "nowhere"), _set(ids[2], "Third")]})
    assert result.ok, result.error
    assert result.data["failed"] == [1]
    assert version() == start + 1
    assert _texts(session, doc, ids[:3]) == ["First", before[1], "Third"]
    assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    assert _texts(session, doc, ids[:3]) == before


def test_reads_inside_a_batch_read_its_earlier_writes(toolbox, opened):
    session, doc, version, ids = opened
    result = toolbox.dispatch(session, "batch", {"atomic": True, "ops": [
        _set(ids[0], "Written first"),
        {"tool": "word_read", "arguments": {"doc": "d1", "range": ids[0]}},
        _set(ids[1], "Written after the read")]})
    assert result.ok, result.error
    assert "Written first" in str(result.data["ops"][1]), result.data["ops"][1]


def test_a_writer_arriving_mid_batch_waits_and_loses_nothing(toolbox, opened):
    session, doc, version, ids = opened
    batch_ids, single_ids = ids[:20], ids[20:26]
    start, failures = version(), []
    started = threading.Event()

    def batch() -> None:
        started.set()
        result = toolbox.dispatch(session.view(context={"agent": "b"}), "batch", {
            "atomic": True, "ops": [_set(t, f"Batch {k}") for k, t in enumerate(batch_ids)]})
        if not result.ok:
            failures.append(result.error)

    def writer() -> None:
        started.wait(10)
        view = session.view(context={"agent": "w"})
        for k, target in enumerate(single_ids):
            result = toolbox.dispatch(view, "word_set_text", {"doc": doc, "target": target, "text": f"Alone {k}"})
            if not result.ok:
                failures.append(result.error)

    threads = [threading.Thread(target=batch), threading.Thread(target=writer)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(120)
    assert not any(thread.is_alive() for thread in threads), "deadlock"
    assert not failures, failures[0]
    assert _texts(session, doc, batch_ids) == [f"Batch {k}" for k in range(len(batch_ids))]
    assert _texts(session, doc, single_ids) == [f"Alone {k}" for k in range(len(single_ids))]
    assert version() == start + 1 + len(single_ids)


def test_an_id_a_batch_deleted_is_retired_by_the_batch_s_step(toolbox, opened):
    session, doc, version, ids = opened
    result = toolbox.dispatch(session, "batch", {"atomic": True, "ops": [
        {"tool": "word_delete", "arguments": {"doc": "d1", "target": ids[0]}}, _set(ids[1], "After the delete")]})
    assert result.ok, result.error
    # the deleted paragraph's id is retired by the batch's step, as the session's record says
    refused = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": ids[0], "text": "x"})
    assert not refused.ok and f"the change that made version {version()}" in refused.error.message, \
        refused.error.message
