"""``ooxml_core.Session``: the Rust session from Python (lane P0d).

Documents opened from bytes, ``describe`` called as JSON, a version's parts handed over
(``update``), saved and closed; refusals as replies and the session's own errors as
exceptions; cancellation and deadlines; many threads on one document, every Rust call
made with the interpreter lock let go -- nothing here relies on that lock, and on a
free-threaded build the module keeps it off.
"""
from __future__ import annotations

import io
import json
import sys
import sysconfig
import threading
import zipfile

import ooxml_core
import pytest

_W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'


def docx(controls: int = 1) -> bytes:
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("_rels/.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/'
                         'relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/'
                         'officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
                         "</Relationships>")
        archive.writestr("word/document.xml", f"<w:document {_W}><w:body>{'<w:sdt/>' * controls}</w:body></w:document>")
    return out.getvalue()


def call(session, name: str, arguments, **options) -> dict:
    text, images, _app = session.call(name, json.dumps(arguments), **options)
    assert images == []
    return json.loads(text)


def test_open_describe_update_save_close():
    session = ooxml_core.Session()
    data = docx()
    doc = session.open(data, name="a.docx")
    assert doc == "d1" and session.documents() == ["d1"] and session.version(doc) == 0
    reply = call(session, "describe", {"doc": doc})
    assert (reply["ok"], reply["doc"], reply["version"]) == (True, "d1", 0)
    assert reply["data"]["controls"] == 1 and reply["summary"] == "Described a.docx"
    assert session.save(doc) == data              # nothing changed: the same bytes
    version, changed, attempts = session.update(doc, {"word/document.xml": docx_part(3)})
    assert (version, changed, attempts) == (1, True, 1)
    assert call(session, "describe", {"doc": doc})["data"]["controls"] == 3
    again = session.open(session.save(doc), name="b.docx")
    assert call(session, "describe", {"doc": again})["data"]["controls"] == 3
    assert session.update(doc, {"word/media/x.bin": b"\0", "word/x.xml": b"<x/>"})[0] == 2
    assert session.update(doc, {"word/media/x.bin": None})[0] == 3
    session.close(doc)
    assert session.documents() == [again]
    assert dict(session.stats())["calls"] == 3
    assert dict(session.store_stats(again))["commits"] == 0


def docx_part(controls: int) -> bytes:
    return f"<w:document {_W}><w:body>{'<w:sdt/>' * controls}</w:body></w:document>".encode()


def test_refusals_are_replies_and_session_errors_are_exceptions():
    session = ooxml_core.Session()
    doc = session.open(docx(), name="a.docx")
    missing = call(session, "describe", {"doc": "d5"})
    assert missing["error"] == {"code": "not_found", "message": "no open document 'd5'", "field": "doc",
                                "valid_options": ["d1"]}
    assert call(session, "nope", {})["error"]["valid_options"] == [
        "describe", "find_text", "read_batch", "replace_text", "word_changes", "word_comments", "word_inspect",
        "word_read", "word_set_text", "word_set_tracking"]
    text, _, _ = session.call("describe", "{not json")
    assert json.loads(text)["error"]["code"] == "invalid_arguments"
    assert len(session.call("describe", json.dumps({"doc": doc}), limit=60)[0]) <= 60 + 100
    with pytest.raises(KeyError):
        session.version("d9")
    with pytest.raises(ValueError):
        session.open(docx(), kind="doc")
    with pytest.raises(ValueError):
        session.open(b"not a zip")
    with pytest.raises(ValueError):
        session.update(doc, {"word/document.xml": b"<w:document"})
    assert session.version(doc) == 0


def test_cancelled_and_late_calls_stop_with_timeout():
    session = ooxml_core.Session()
    doc = session.open(docx(), name="a.docx")
    token = ooxml_core.CancelToken()
    token.cancel()
    assert token.cancelled
    context = ooxml_core.CallContext(cancel=token, caller="w1")
    assert call(session, "describe", {"doc": doc}, context=context)["error"]["code"] == "timeout"
    late = ooxml_core.CallContext(deadline=0.0)
    assert call(session, "describe", {"doc": doc}, context=late)["error"]["code"] == "timeout"
    with pytest.raises(TimeoutError):
        session.update(doc, {"word/x.xml": b"<x/>"}, context=late)
    assert session.version(doc) == 0
    with pytest.raises(ValueError):
        ooxml_core.CallContext(deadline=-1.0)
    session.shutdown()
    assert call(session, "describe", {"doc": doc})["error"]["code"] == "timeout"


def test_tool_specs_are_the_exports():
    specs = json.loads(ooxml_core.tool_specs("docx"))
    assert [s["name"] for s in specs] == ["describe", "word_read", "word_inspect", "find_text", "word_set_tracking",
                                          "word_changes", "word_comments", "word_set_text", "replace_text",
                                          "read_batch"]
    assert specs == json.loads(ooxml_core.Session().tool_specs("docx"))
    assert json.loads(ooxml_core.tool_specs("pptx")) == []
    with pytest.raises(ValueError):
        ooxml_core.tool_specs("doc")


def test_many_threads_on_one_document_commit_every_change():
    session = ooxml_core.Session()
    doc = session.open(docx(), name="a.docx")
    threads, each = 24, 10
    errors: list[BaseException] = []

    def write(n: int) -> None:
        try:
            context = ooxml_core.CallContext(caller=f"w{n}")
            for k in range(each):
                parts = {f"word/w{n}.xml": f'<n v="{k}"/>'.encode()}
                if k % 3 == 0:
                    parts["word/shared.xml"] = f'<n w="{n}" v="{k}"/>'.encode()
                assert session.update(doc, parts, context=context)[1]
                assert call(session, "describe", {"doc": doc})["ok"]
        except BaseException as error:  # noqa: BLE001 -- reported below
            errors.append(error)

    workers = [threading.Thread(target=write, args=(n,)) for n in range(threads)]
    for worker in workers:
        worker.start()
    for worker in workers:
        worker.join()
    assert errors == []
    assert session.version(doc) == threads * each
    assert dict(session.store_stats(doc))["commits"] == threads * each


def test_the_module_keeps_a_free_threaded_build_free():
    if not sysconfig.get_config_var("Py_GIL_DISABLED"):
        pytest.skip("not a free-threaded build")
    assert not sys._is_gil_enabled()


def test_undo_redo_and_the_save_gate():
    session = ooxml_core.Session()
    doc = session.open(docx(), name="a.docx")
    ann = ooxml_core.CallContext(caller="ann")
    bo = ooxml_core.CallContext(caller="bo")
    session.update(doc, {"word/document.xml": docx_part(2)}, context=ann)
    session.update(doc, {"word/document.xml": docx_part(3)}, context=bo)
    with pytest.raises(ValueError, match="not the latest"):
        session.undo(doc, context=ann)             # bo changed it after ann
    assert session.undo(doc, 5, context=bo) == (1, 3)
    assert session.undo(doc) == (1, 4)
    assert call(session, "describe", {"doc": doc})["data"]["controls"] == 1
    assert session.redo(doc, 2) == (2, 6)
    assert call(session, "describe", {"doc": doc})["data"]["controls"] == 3
    assert session.validation(doc)[:2] == ([], [])
    assert session.save(doc, checked=True) == session.save(doc)
    reply = call(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Ann"})
    assert reply["ok"] and reply["data"]["author"] == "Ann"


def test_bridged_calls_are_counted():
    session = ooxml_core.Session()
    doc = session.open(docx(), name="a.docx")
    version = session.bridge(doc, "word_format", {"word/document.xml": docx_part(2)})
    assert version.number == 1
    assert session.bridge_calls() == [("word_format", 1)]
    assert call(session, "describe", {"doc": doc})["data"]["controls"] == 2
