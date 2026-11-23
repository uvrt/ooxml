"""docx-agent's reading tools answered by the Rust session (lane P1c): the version a call
reads handed to the session as the parts that changed, the call run there with the
interpreter lock let go -- ``describe``, ``word_read``, ``word_inspect``, ``find_text`` read
the version an edit made; an id a served edit deleted is refused naming the edit; and a
placer callback holds no lock another thread needs.
"""
from __future__ import annotations

import json

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Version"):          # an ooxml-core before 0.10.0
    pytest.skip("ooxml_core has no Version", allow_module_level=True)

from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent.rust import reads  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from conftest import FIXTURE_DIR  # noqa: E402
import word_docs  # noqa: E402


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def _answer(toolbox, session, tool, arguments) -> dict:
    """``tool``'s answer through the toolbox, counted served."""
    served = reads.served().get(tool, 0)
    answer = json.loads(json.dumps(toolbox.dispatch(session, tool, arguments).to_json()))
    assert reads.served().get(tool, 0) == served + 1, tool
    return answer


def _first(session, doc) -> str:
    return next(block for block, text in word_docs.paragraphs(session, doc) if block != "p:None" and text)


def test_a_new_version_is_handed_over_as_the_parts_that_changed(toolbox):
    path = FIXTURE_DIR / "generated" / "ids-and-markup.docx"
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        first = _first(session, doc)
        before = _answer(toolbox, session, "word_read", {"doc": doc})
        edited = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": first, "text": "Changed here."})
        assert edited.ok, edited.to_json()
        for tool, arguments in [("word_read", {"doc": doc}), ("describe", {"doc": doc}),
                                ("word_inspect", {"doc": doc, "target": first}),
                                ("find_text", {"doc": doc, "text": "Changed"})]:
            assert _answer(toolbox, session, tool, arguments)["ok"], tool
        assert "Changed here." in _answer(toolbox, session, "word_read", {"doc": doc})["data"]
        assert before != _answer(toolbox, session, "word_read", {"doc": doc})
        found = _answer(toolbox, session, "find_text", {"doc": doc, "text": "Changed"})
        assert found["data"], found
        assert session.entry(doc).version in reads.owned(session.entry(doc)).handle.versions
    finally:
        session.close(doc)


def test_an_id_a_served_edit_deleted_is_refused_naming_it(toolbox):
    path = FIXTURE_DIR / "generated" / "ids-and-markup.docx"
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        first = _first(session, doc)
        deleted = toolbox.dispatch(session, "word_delete", {"doc": doc, "target": first, "force": True})
        assert deleted.ok, deleted.to_json()
        # an id a served edit deleted: refused by the session, worded as the view worded it (lane P2e1)
        gone = _answer(toolbox, session, "word_inspect", {"doc": doc, "target": first})
        assert f"{first} was deleted by word_delete" in gone["error"]["message"]
    finally:
        session.close(doc)


def test_a_placer_callback_holds_no_rust_lock_another_thread_needs():
    """While Rust waits for a placer (the interpreter lock taken back), another thread's
    calls on the same document and session -- a commit, a read, a read_batch -- finish:
    no session, document or memo lock is held across the callback (a deadlock pinned)."""
    import threading

    path = FIXTURE_DIR / "generated" / "ids-and-markup.docx"
    data = path.read_bytes()
    session = ooxml_core.Session()
    doc = session.open(data, kind="docx", name=path.name)
    head = session.head(doc)
    done: list[str] = []

    def other() -> None:
        session.commit(doc, {"word/document.xml": _part(data, "word/document.xml")})
        at_head = ooxml_core.CallOptions(documents=ooxml_core.CallDocuments(at=[head]))
        session.call("word_read", json.dumps({"doc": doc}), at_head)
        session.call("read_batch", json.dumps({"ops": [{"tool": "describe", "arguments": {"doc": doc}},
                                                       {"tool": "word_read", "arguments": {"doc": doc}}]}))
        done.append("other")

    def placer(asked: str) -> str:
        # Called by Rust in the middle of word_inspect: the other thread runs to its end
        # meanwhile, through the same session and document.
        thread = threading.Thread(target=other)
        thread.start()
        thread.join(timeout=60)
        assert not thread.is_alive(), "a call on the same document waited for the placer's caller"
        return json.dumps([{"unknown": {"reason": "not-drawn", "after_page": None}} for _ in json.loads(asked)])

    reply, _images, _app = session.call("word_inspect", json.dumps({"doc": doc, "target": "body", "layout": True}),
                                        ooxml_core.CallOptions(documents=ooxml_core.CallDocuments(at=[head]),
                                                               layout=ooxml_core.CallLayout(placer=placer)))
    assert json.loads(reply)["ok"] and done == ["other"]

    # Two threads at once, one inspecting with a placer, one committing and reading.
    errors: list[BaseException] = []

    def inspect() -> None:
        try:
            for _ in range(5):
                session.call("word_inspect", json.dumps({"doc": doc, "target": "body", "layout": True}),
                             ooxml_core.CallOptions(layout=ooxml_core.CallLayout(placer=lambda asked: json.dumps(
                                 [{"placements": []} for _ in json.loads(asked)]))))
        except BaseException as error:  # noqa: BLE001
            errors.append(error)

    def write() -> None:
        try:
            for _ in range(5):
                session.commit(doc, {"word/document.xml": _part(data, "word/document.xml")})
                session.call("describe", json.dumps({"doc": doc}))
        except BaseException as error:  # noqa: BLE001
            errors.append(error)

    threads = [threading.Thread(target=inspect), threading.Thread(target=write)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=120)
    assert not any(t.is_alive() for t in threads) and not errors, errors


def _part(data: bytes, name: str) -> bytes:
    import io
    import zipfile

    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return archive.read(name)
