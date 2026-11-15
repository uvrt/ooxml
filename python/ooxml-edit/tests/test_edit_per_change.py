"""0.117.0: a version costs what it changed -- one serialization per version a call leaves,
a long part's versions sharing their pieces, a committed version's bytes made when first
asked, and reads that wait for a commit, never for the lock an edit holds."""

from __future__ import annotations

import threading
import time

from ooxml_edit.history import History
from ooxml_edit.opc import OpcPackage, Pieces
from ooxml_edit.tools.session import Committed, DocumentEntry, DocumentFormat
from ooxml_edit.xml import make, qn

import synthetic
import tools_toys


def _package(items: int = 2000) -> OpcPackage:
    package = OpcPackage.open(synthetic.outer_package())
    root = package.tree(synthetic.MAIN)
    for k in range(items):
        root.append(make("tst:item", val=f"item {k} " + "x" * 40))
    package.mark_dirty(synthetic.MAIN)
    return package


# -- pieces ------------------------------------------------------------------------------------


def test_pieces_share_what_did_not_change_and_read_back_the_bytes():
    data = bytes(range(256)) * 4000                       # ~1 MB
    first = Pieces.of(data)
    assert bytes(first) == data and len(first) == len(data)
    changed = data[:500_000] + b"CHANGED" + data[500_010:]
    second = Pieces.of(changed, first)
    assert bytes(second) == changed and second != first
    shared = sum(1 for piece in second.pieces if any(piece is old for old in first.pieces))
    assert shared >= len(first.pieces) - 2             # only the changed piece(s) copied
    assert Pieces.of(data, first) is first             # the same bytes: the same pieces
    assert Pieces.of(changed, second) == second == changed
    assert Pieces.of(b"") == b"" and bytes(Pieces.of(b"abc", Pieces.of(b""))) == b"abc"


def test_a_long_parts_snapshots_share_pieces_and_undo_gives_its_bytes_back():
    package = _package(20000)
    history = History(package)
    states = [package.read(synthetic.MAIN)]
    for k in range(5):
        with history.batch():
            root = package.tree(synthetic.MAIN)
            list(root.iter(qn("tst:item")))[k * 100].set("val", f"edited {k}")
            package.mark_dirty(synthetic.MAIN)
        states.append(package.read(synthetic.MAIN))
    snapshots = [snapshot.trees[synthetic.MAIN] for snapshot, _ in history._undo]
    assert all(isinstance(found, Pieces) for found in snapshots)
    held = {id(piece) for found in snapshots for piece in found.pieces}
    whole = sum(len(found.pieces) for found in snapshots)
    assert len(held) < whole / 2                       # most pieces are shared
    for expected in reversed(states[:-1]):
        assert history.undo()
        assert package.read(synthetic.MAIN) == expected
    for expected in states[1:]:
        assert history.redo()
        assert package.read(synthetic.MAIN) == expected


# -- one serialization per version, inside a call ----------------------------------------------


def test_a_tree_changed_between_calls_is_written_out_again():
    package = _package(10)
    history = History(package)
    with package.serializing():
        with history.batch():
            package.tree(synthetic.MAIN).append(make("tst:item", val="in the call"))
            package.mark_dirty(synthetic.MAIN)
        kept = package.read(synthetic.MAIN)
        assert package.read(synthetic.MAIN) is kept    # one serialization, shared
    # Outside a call: an application's own edit of the tree, without marking it dirty again.
    package.tree(synthetic.MAIN).append(make("tst:item", val="between calls"))
    assert b"between calls" in package.read(synthetic.MAIN)
    assert package.read(synthetic.MAIN) is not package.read(synthetic.MAIN)


def test_nothing_is_kept_while_a_batch_is_open():
    package = _package(10)
    history = History(package)
    with package.serializing():
        with history.batch():
            package.tree(synthetic.MAIN).append(make("tst:item", val="one"))
            package.mark_dirty(synthetic.MAIN)
            assert b"one" in package.read(synthetic.MAIN)
            package.tree(synthetic.MAIN).append(make("tst:item", val="two"))     # not marked again
            assert b"two" in package.read(synthetic.MAIN)
        assert b"two" in package.read(synthetic.MAIN)


# -- one serialization per version, from one call to the next (0.119.0) --------------------------


def _counting(monkeypatch) -> list:
    import ooxml_edit.opc as opc

    calls = []
    real = opc.serialize

    def counted(root):
        calls.append(1)
        return real(root)

    monkeypatch.setattr(opc, "serialize", counted)
    monkeypatch.setenv("OOXML_EDIT_VERIFY_SERIALIZED", "0")
    return calls


def _edit_in_a_call(package, history, value):
    with package.serializing():
        with history.batch():
            package.tree(synthetic.MAIN).append(make("tst:item", val=value))
            package.mark_dirty(synthetic.MAIN)
        return package.read(synthetic.MAIN)


def test_the_next_call_starts_from_the_snapshot_the_last_one_left(monkeypatch):
    package = _package(10)
    history = History(package)
    _edit_in_a_call(package, history, "one")
    calls = _counting(monkeypatch)
    data = _edit_in_a_call(package, history, "two")
    assert len(calls) == 1                             # the version's own, not the state before
    assert b"two" in data and history.undo()
    assert b"two" not in package.read(synthetic.MAIN) and b"one" in package.read(synthetic.MAIN)


def test_a_tree_handed_out_between_calls_or_marked_dirty_is_written_out_again(monkeypatch):
    package = _package(10)
    history = History(package)
    _edit_in_a_call(package, history, "one")
    # An application's own change between calls, through the tree it is handed.
    package.tree(synthetic.MAIN).append(make("tst:item", val="between calls"))
    _edit_in_a_call(package, history, "two")
    assert history.undo()
    assert b"between calls" in package.read(synthetic.MAIN)
    # Through a tree it held, marked dirty as mark_dirty asks.
    root = package.tree(synthetic.MAIN)
    _edit_in_a_call(package, history, "three")
    root.append(make("tst:item", val="held"))
    package.mark_dirty(synthetic.MAIN)
    calls = _counting(monkeypatch)
    _edit_in_a_call(package, history, "four")
    assert len(calls) == 2                             # the state before, and the version
    assert history.undo()
    assert b"held" in package.read(synthetic.MAIN) and b"four" not in package.read(synthetic.MAIN)


# -- committed versions --------------------------------------------------------------------------


def test_a_committed_versions_bytes_are_made_once_when_asked():
    made = []

    def later():
        made.append(1)
        return b"the bytes"

    committed = Committed(3, later, lambda data: data, {}, {})
    assert made == []
    assert committed.data == b"the bytes" and committed.data == b"the bytes"
    assert made == [1]
    again = committed.with_refs({"a": "x"}, {"a": "x"}, {})
    assert again.data == b"the bytes" and made == [1]


def _entry() -> tuple[DocumentEntry, DocumentFormat]:
    def read_copy(document):
        version = document.history.version

        def copy(given):
            found = tools_toys.ToyDocument(given if isinstance(given, bytes) else given.data)
            found.history.continue_from(version)
            return found
        return copy

    fmt = DocumentFormat(kind="docx", open=tools_toys.ToyDocument, detect=lambda data, name: True,
                         read_copy=read_copy)
    document = tools_toys.ToyDocument(synthetic.outer_package())
    entry = DocumentEntry("d1", "docx", "a.docx", "test", document)
    with entry.lock:
        entry.commit(fmt)
    return entry, fmt


def test_a_read_waits_for_the_commit_not_for_the_lock():
    entry, fmt = _entry()
    changed, released = threading.Event(), []

    def edit():
        with entry.lock:
            with entry.batch():
                entry.document.insert("new", "end")
            entry.note_change(None, 0)
            changed.set()
            time.sleep(0.3)
            entry.commit(fmt)                       # the version committed...
            time.sleep(1.0)                         # ...and the lock held on after it
            released.append(time.monotonic())

    thread = threading.Thread(target=edit)
    thread.start()
    changed.wait()
    reader = entry.committed_reader(fmt)
    got = time.monotonic()
    thread.join()
    assert reader is not None and reader.version == entry.version
    assert "new" in reader.document.items()
    assert got < released[0]                         # it did not wait for the lock


def test_a_read_with_the_lock_free_commits_as_before():
    entry, fmt = _entry()
    with entry.lock:
        with entry.batch():
            entry.document.insert("later", "end")
    reader = entry.committed_reader(fmt)
    assert reader is not None and "later" in reader.document.items()
