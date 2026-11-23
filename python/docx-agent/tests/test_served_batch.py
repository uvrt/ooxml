"""Lane H1: a batch's ops on a document the Rust session owns are served by the session, in
one step group of the session's -- one undo step, all or none where atomic, a failed op
taken back alone where not, its reads reading what its earlier ops made, Python ops beside
served ones -- and the group closed on every path."""

import threading

import pytest

from docx_agent.rust import edits
from docx_agent.rust.owner import retired_ids
from docx_agent.rust.reads import Mirror
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as box:
        yield box


@pytest.fixture
def opened(toolbox, constructs_doc):
    session = toolbox.session()
    doc = session.open(constructs_doc.read_bytes(), name=constructs_doc.name)
    entry = session.entry(doc)
    if not isinstance(entry, OwnedEntry):
        pytest.skip("no Rust session to own the document")
    paragraphs = [p.id for p in entry.document.paragraphs() if p.text.strip() and p.id.startswith("p:")]
    yield session, doc, entry, paragraphs
    session.close(doc)


def _texts(entry, ids):
    found = {p.id: p.text for p in entry.document.paragraphs()}
    return [found.get(i) for i in ids]


def _set(target, text):
    return {"tool": "word_set_text", "arguments": {"doc": "d1", "target": target, "text": text}}


def _batch_fallbacks():
    return sum(n for reasons in edits.fallbacks().values() for why, n in reasons.items() if why == "batch op")


def test_an_atomic_batch_is_served_and_one_undo_takes_it_back(toolbox, opened):
    session, doc, entry, ids = opened
    targets = ids[:3]
    before, start, served = _texts(entry, targets), entry.version, sum(edits.served().values())
    result = toolbox.dispatch(session, "batch", {"atomic": True,
                                                 "ops": [_set(t, f"Batch {k}") for k, t in enumerate(targets)]})
    assert result.ok, result.error
    assert sum(edits.served().values()) - served == 3, edits.fallbacks()
    assert _batch_fallbacks() == 0
    assert entry.version == start + 1 == result.version
    assert _texts(entry, targets) == ["Batch 0", "Batch 1", "Batch 2"]
    assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    assert entry.version == start
    assert _texts(entry, targets) == before
    assert toolbox.dispatch(session, "undo", {"doc": doc, "redo": True}).ok
    assert _texts(entry, targets) == ["Batch 0", "Batch 1", "Batch 2"]


def test_a_failed_atomic_batch_leaves_undo_and_redo_as_they_were(toolbox, opened):
    session, doc, entry, ids = opened
    assert toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": ids[0], "text": "Kept"}).ok
    assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    before, start = _texts(entry, ids[:3]), entry.version
    result = toolbox.dispatch(session, "batch", {"atomic": True, "ops": [
        _set(ids[1], "Gone 1"), _set(ids[2], "Gone 2"), _set("p:FFFFFFFF", "nowhere")]})
    assert not result.ok
    assert "nothing in the batch was applied" in result.error.message
    assert entry.version == start
    assert _texts(entry, ids[:3]) == before
    assert entry.history.can_redo()
    assert toolbox.dispatch(session, "undo", {"doc": doc, "redo": True}).ok
    assert _texts(entry, ids[:1]) == ["Kept"]


def test_a_partial_batch_keeps_the_ops_that_applied(toolbox, opened):
    session, doc, entry, ids = opened
    before, start = _texts(entry, ids[:3]), entry.version
    result = toolbox.dispatch(session, "batch", {"ops": [
        _set(ids[0], "First"), _set("p:FFFFFFFF", "nowhere"), _set(ids[2], "Third")]})
    assert result.ok, result.error
    assert result.data["failed"] == [1]
    assert entry.version == start + 1
    assert _texts(entry, ids[:3]) == ["First", before[1], "Third"]
    assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    assert _texts(entry, ids[:3]) == before


def test_reads_inside_a_batch_read_its_earlier_writes(toolbox, opened):
    session, doc, entry, ids = opened
    result = toolbox.dispatch(session, "batch", {"atomic": True, "ops": [
        _set(ids[0], "Written first"),
        {"tool": "word_read", "arguments": {"doc": "d1", "range": ids[0]}},
        _set(ids[1], "Written after the read")]})
    assert result.ok, result.error
    assert "Written first" in str(result.data["ops"][1]), result.data["ops"][1]


def _part(data, name):
    import io
    import zipfile

    with zipfile.ZipFile(io.BytesIO(data)) as package:
        return package.read(name) if name in package.namelist() else b""


def test_an_error_mid_batch_closes_the_group(toolbox, opened, monkeypatch):
    session, doc, entry, ids = opened
    before, start = _texts(entry, ids[:2]), entry.version
    real, calls = edits._edit, []

    class Stop(BaseException):
        pass

    def failing(call, entry_, arguments, counted):
        calls.append(1)
        if len(calls) == 2:
            raise Stop()
        return real(call, entry_, arguments, counted)

    monkeypatch.setattr(edits, "_edit", failing)
    with pytest.raises(Stop):
        toolbox.dispatch(session, "batch", {"atomic": True,
                                            "ops": [_set(ids[0], "One"), _set(ids[1], "Two")]})
    monkeypatch.setattr(edits, "_edit", real)
    assert entry._group is None and not entry.history.grouped
    assert entry.version == start and _texts(entry, ids[:2]) == before
    # the group was closed: a new batch opens its own, and its step undoes alone
    assert toolbox.dispatch(session, "batch", {"ops": [_set(ids[0], "Again")]}).ok
    assert entry.version == start + 1
    assert toolbox.dispatch(session, "undo", {"doc": doc}).ok
    assert _texts(entry, ids[:2]) == before


def test_a_writer_arriving_mid_batch_waits_and_loses_nothing(toolbox, opened):
    session, doc, entry, ids = opened
    batch_ids, single_ids = ids[:20], ids[20:26]
    start, failures = entry.version, []
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
    assert _texts(entry, batch_ids) == [f"Batch {k}" for k in range(len(batch_ids))]
    assert _texts(entry, single_ids) == [f"Alone {k}" for k in range(len(single_ids))]
    assert entry.version == start + 1 + len(single_ids)


def test_every_version_of_a_batch_is_numbered_as_its_step(toolbox, opened):
    session, doc, entry, ids = opened
    mirror = entry.handle
    first = mirror.session.head(mirror.doc).number + 1
    result = toolbox.dispatch(session, "batch", {"atomic": True, "ops": [
        {"tool": "word_delete", "arguments": {"doc": "d1", "target": ids[0]}}, _set(ids[1], "After the delete")]})
    assert result.ok, result.error
    last = mirror.session.head(mirror.doc).number
    assert last > first
    assert all(mirror.numbers[seq] == entry.version for seq in range(first, last + 1))
    gone = retired_ids(mirror).gone
    assert entry.version in gone.values()


def test_a_step_of_several_versions_numbers_each():
    """The gap found on the way (lane H1): a step the session made in several versions (a
    call's edits, a batch's ops) had only its last numbered, so what its earlier versions
    retired was left out of the record of retired ids."""

    class Head:
        number = 9

        def __init__(self, state: int) -> None:
            self.state = state

    mirror = Mirror(None, "d1", {})
    mirror.keep(Head(4), 7)
    assert mirror.numbers == {7: 4, 8: 4, 9: 4}
    mirror.keep(Head(5))
    assert mirror.numbers[9] == 5
