"""``ooxml_core.Session``: the Rust session from Python (lane P0d).

Documents opened from bytes, ``describe`` called as JSON, edited (``edit``: a Word tool's
change), saved and closed; refusals as replies and the session's own errors as
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
    text, images, _app = session.call(name, json.dumps(arguments), ooxml_core.CallOptions(**options))
    assert images == []
    return json.loads(text)


def edit(session, name: str, arguments, **options) -> tuple[dict, object]:
    """A changing call: its reply and what it made (``None``: nothing)."""
    text, _app, made = session.edit(name, json.dumps(arguments), ooxml_core.CallOptions(**options))
    return json.loads(text), made


def insert(session, doc: str, text: str, **options) -> dict:
    reply, made = edit(session, "word_insert_markdown", {"doc": doc, "markdown": text, "at": "end"}, **options)
    assert reply["ok"] and made is not None, reply
    return reply


def body(data: bytes) -> str:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return archive.read("word/document.xml").decode()


def test_open_describe_edit_save_close():
    session = ooxml_core.Session()
    data = docx()
    doc = session.open(data, name="a.docx")
    assert doc == "d1" and session.documents() == ["d1"] and session.version(doc) == 0
    reply = call(session, "describe", {"doc": doc})
    assert (reply["ok"], reply["doc"], reply["version"]) == (True, "d1", 0)
    assert reply["data"]["controls"] == 1 and reply["summary"] == "Described a.docx"
    assert session.save(doc) == data              # nothing changed: the same bytes
    assert insert(session, doc, "One.")["version"] == 1 and session.version(doc) == 1
    assert call(session, "describe", {"doc": doc})["data"]["controls"] == 1
    again = session.open(session.save(doc), name="b.docx")
    assert "One." in body(session.save(again))
    assert insert(session, doc, "Two.")["version"] == 2
    session.close(doc)
    assert session.documents() == [again]
    assert dict(session.stats())["calls"] == 4
    assert dict(session.store_stats(again))["commits"] == 0


def test_refusals_are_replies_and_session_errors_are_exceptions():
    session = ooxml_core.Session()
    doc = session.open(docx(), name="a.docx")
    missing = call(session, "describe", {"doc": "d5"})
    assert missing["error"] == {"code": "not_found", "message": "no open document 'd5'", "field": "doc",
                                "valid_options": ["d1"]}
    # every tool the session dispatches (the registry's, as tool_specs exports them), sorted
    names = sorted(s["name"] for s in json.loads(ooxml_core.tool_specs("docx")))
    assert call(session, "nope", {})["error"]["valid_options"] == names
    text, _, _ = session.call("describe", "{not json")
    assert json.loads(text)["error"]["code"] == "invalid_arguments"
    assert len(session.call("describe", json.dumps({"doc": doc}), ooxml_core.CallOptions(limit=60))[0]) <= 60 + 100
    with pytest.raises(KeyError):
        session.version("d9")
    with pytest.raises(ValueError):
        session.open(docx(), kind="doc")
    with pytest.raises(ValueError):
        session.open(b"not a zip")
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
    reply, made = edit(session, "word_insert_markdown", {"doc": doc, "markdown": "Late.", "at": "end"}, context=late)
    assert reply["error"]["code"] == "timeout" and made is None
    assert session.version(doc) == 0
    with pytest.raises(ValueError):
        ooxml_core.CallContext(deadline=-1.0)
    session.shutdown()
    assert call(session, "describe", {"doc": doc})["error"]["code"] == "timeout"


def test_a_locale_is_a_tag_or_a_locale_with_its_five_fields():
    from types import SimpleNamespace

    dutch = SimpleNamespace(decimal_separator=",", thousands_separator=".", date_order="dmy", language="nl-NL",
                            list_separator=";")
    session = ooxml_core.Session(locale=dutch)
    doc = session.open(docx(), name="a.docx")
    for locale in (dutch, "nl_be", None):
        context = ooxml_core.CallContext(writing=ooxml_core.CallWriting(locale=locale))
        assert "error" not in call(session, "describe", {"doc": doc}, context=context)
    tagged = ooxml_core.Session(locale="en-GB")
    assert "error" not in call(tagged, "describe", {"doc": tagged.open(docx(), name="b.docx")})
    with pytest.raises(ValueError):
        ooxml_core.CallWriting(locale="english")
    with pytest.raises(ValueError):
        ooxml_core.Session(locale=SimpleNamespace(**{**vars(dutch), "list_separator": ","}))
    with pytest.raises(AttributeError):
        ooxml_core.CallWriting(locale=SimpleNamespace(language="nl-NL"))


def test_limits_are_given_as_one_and_checked():
    """``Limits``: the host's limits on a call's reply, carried by a ``CallContext``;
    ``CallLayout(image_budgets=)``: the session's and the round's, by name."""
    limits = ooxml_core.Limits(result_chars=3000, images_per_call=2, image_edge=1200)
    assert (limits.result_chars, limits.list_items, limits.images_per_call, limits.image_edge) == (3000, 50, 2, 1200)
    default = ooxml_core.Limits()
    assert (default.result_chars, default.list_items, default.images_per_call,
            default.image_edge) == (24_000, 50, 4, 2576)
    for given in ({"list_items": 0}, {"images_per_call": 0}, {"image_edge": 0}):
        with pytest.raises(ValueError, match=next(iter(given))):
            ooxml_core.Limits(**given)
    ooxml_core.CallContext(limits=limits, caller="w1")
    with pytest.raises(TypeError):
        ooxml_core.CallContext(max_result_chars=3000)
    ooxml_core.CallLayout(image_budgets=[("session", 10, 3), ("round", 4, 4)])
    with pytest.raises(ValueError, match="week"):
        ooxml_core.CallLayout(image_budgets=[("week", 1, 0)])


def test_tool_specs_are_the_exports():
    specs = json.loads(ooxml_core.tool_specs("docx"))
    # the registry's tools in registration order (ooxml-session's tests/integration/registry.rs holds each
    # to its golden export): describe first, read_batch over the others last, each once
    names = [s["name"] for s in specs]
    assert names[0] == "describe" and names[-1] == "read_batch" and len(set(names)) == len(names)
    assert {"word_read", "word_set_text"} <= set(names)
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
                insert(session, doc, f"Writer {n}, line {k}.", context=context)
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
    insert(session, doc, "Ann's.", context=ann)
    insert(session, doc, "Bo's.", context=bo)
    with pytest.raises(ValueError, match="not the latest"):
        session.undo(doc, context=ann)             # bo changed it after ann
    # (steps, state number): a plain undo or redo gives the number of the state it brings back,
    # as docx-agent's undo replies number them (bo's undo restores ann's state 1, the next the
    # opened state 0, the redo of both state 2) -- not a count of versions
    assert session.undo(doc, 5, context=bo) == (1, 1)
    assert "Ann's." in body(session.save(doc)) and "Bo's." not in body(session.save(doc))
    assert session.undo(doc) == (1, 0)
    assert "Ann's." not in body(session.save(doc))
    assert session.redo(doc, 2) == (2, 2)
    assert "Bo's." in body(session.save(doc))
    assert session.validation(doc)[:2] == ([], [])
    assert session.save(doc, checked=True) == session.save(doc)
    reply = call(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Ann"})
    assert reply["ok"] and reply["data"]["author"] == "Ann"


def page_docx(result: str) -> bytes:
    """A one-paragraph document: "Page " and a `PAGE` field whose cached result is `result`."""
    run = '<w:r><w:fldChar w:fldCharType="{}"/></w:r>'
    body = (f'<w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r>{run.format("begin")}'
            '<w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>'
            f'{run.format("separate")}<w:r><w:t>{result}</w:t></w:r>{run.format("end")}</w:p>')
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("_rels/.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/'
                         'relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/'
                         'officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
                         "</Relationships>")
        archive.writestr("word/document.xml", f"<w:document {_W}><w:body>{body}</w:body></w:document>")
    return out.getvalue()


def test_a_field_update_lays_out_with_the_sessions_own_engine():
    """``Session(pages=PageSetup(...))`` (lane H7b): the session keeps each document's engine
    (``Session.pages``, one per document) and a call whose host gives no layout is laid out by
    it; ``set_pages`` gives a document an engine of its own."""
    from pathlib import Path

    files = Path(__file__).resolve().parents[2] / "pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
    setup = ooxml_core.PageSetup(ooxml_core.WordFonts([("folder", str(files))]))
    bare = ooxml_core.Session()
    assert bare.pages(bare.open(page_docx("9"), name="a.docx")) is None
    session = ooxml_core.Session(pages=setup)
    doc = session.open(page_docx("9"), name="a.docx")
    assert isinstance(session.pages(doc), ooxml_core.WordPages)
    reply = call(session, "word_fields", {"doc": doc, "action": "update"})
    assert reply["ok"], reply
    assert call(session, "word_fields", {"doc": doc, "action": "list"})["data"][0]["result"] == "1"
    other = ooxml_core.Session()
    doc = other.open(page_docx("9"), name="a.docx")
    other.set_pages(doc, setup)
    assert call(other, "word_fields", {"doc": doc, "action": "update"})["ok"]
    assert call(other, "word_fields", {"doc": doc, "action": "list"})["data"][0]["result"] == "1"


def test_a_field_update_lays_out_with_the_hosts_engine():
    """``layout=CallLayout(pages)``: the host's page engine for the document, which ``word_fields`` lays the
    versions it updates out with; without it a page number is left to the host."""
    from pathlib import Path

    files = Path(__file__).resolve().parents[2] / "pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"
    pages = ooxml_core.WordPages(ooxml_core.WordFonts([("folder", str(files))]))
    session = ooxml_core.Session()
    doc = session.open(page_docx("9"), name="a.docx")
    left = call(session, "word_fields", {"doc": doc, "action": "update"})
    assert call(session, "word_fields", {"doc": doc, "action": "list"})["data"][0]["result"] == "9", left
    reply = call(session, "word_fields", {"doc": doc, "action": "update"}, layout=ooxml_core.CallLayout(pages))
    assert reply["ok"], reply
    assert call(session, "word_fields", {"doc": doc, "action": "list"})["data"][0]["result"] == "1"



def test_a_batch_is_one_step_read_by_its_thread_and_a_dropped_group_lets_go():
    session = ooxml_core.Session()
    doc = session.open(docx(), name="a.docx")
    group = session.open_group(doc)
    with pytest.raises(RuntimeError, match="open already"):
        session.open_group(doc)                   # this thread's batch is open already
    insert(session, doc, "One.")
    insert(session, doc, "Two.")
    seen: list[int] = []
    other = threading.Thread(target=lambda: seen.append(session.head(doc).state))
    other.start()
    other.join()
    assert seen == [0] and session.head(doc).state == 1   # the ops' version: this thread's alone
    head = group.close()
    assert head is not None and head.state == 1 and session.version(doc) == head.number
    assert len(session.numbers(doc)[1]) == 1     # one step
    dropped = session.open_group(doc)
    insert(session, doc, "Three.")
    del dropped                                   # unclosed: its work taken back, the document let go
    again = session.open_group(doc)
    assert "Three." not in body(session.save(doc))
    assert again.close() is None
