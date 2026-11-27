"""The Toolbox over the Rust host (Phase 3, E1a): ``ooxml_edit.tools.Toolbox``, ``Session`` and
``Result`` with today's signatures, docx-agent's documents held and its calls run by the Rust
host (``ooxml_core.Host``, one per toolbox; ``HostSession``, one per session).

- Which toolbox runs on the host: every format the host's (a Python-held format keeps the Python
  host until E1b-2), the application's own tools beside it.
- The application's tools keep their datasets, outputs and images in the Rust session (E1b,
  decision D3): read back as kept, a dataset kept again read fresh.
- The session clock reaches the host as a value: a new document's created date is the clock's.
- A call's ``Result`` is built from the reply's JSON, its ``ToolError`` from the reply's error,
  and its record is appended to the session's log.
- An application's own tool runs in the facade, and inside a Rust batch is refused with the way
  out "call it alone".
- ``dispatch_many`` runs a turn over the facade, each result the one the order given has.
- ``docx_format``'s settings reach the host's Word format (a minimum text size, one number or by kind; typography).

Every document is made up.
"""

from __future__ import annotations

import datetime as dt
import io
import re
import zipfile

import pytest

from ooxml_edit.tools import Result, Session, ToolError, Toolbox, string, tool
from ooxml_edit.tools.hosted import HostedSession, HostResult

from docx_agent.tools import FORMAT, GROUPS, TOOLS, docx_format
import word_docs

CLOCK_AT = dt.datetime(2031, 3, 4, 5, 6, 7, tzinfo=dt.timezone.utc)


def clock() -> dt.datetime:
    return CLOCK_AT


@tool("app_note", "Test only: an application's own tool that names no document.",
      {"text": string("What to note.")}, documents=())
def app_note(call, text):
    return Result(summary=f"noted {text}", data={"text": text})


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def opened(box, **given):
    session = box.session(clock=clock, **given)
    doc = session.open(word_docs.blank(), "report.docx")
    return session, doc


# -- which toolbox runs on the host ---------------------------------------------------------------


def test_a_toolbox_of_the_hosts_formats_and_tools_runs_its_sessions_on_the_host(box):
    session = box.session()
    assert isinstance(session, HostedSession)
    assert isinstance(session.view(roles=["review"]), type(session.view()))


def test_a_toolbox_with_tools_of_its_own_runs_on_the_host():
    with Toolbox([*TOOLS, app_note], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        assert isinstance(toolbox.session(), HostedSession)


def test_until_e1b_2_a_toolbox_with_a_python_format_runs_on_the_python_host():
    import dataclasses

    python_format = dataclasses.replace(FORMAT, kind="grid", host=None, owner=None)
    with Toolbox(TOOLS, formats=[FORMAT, python_format], groups=GROUPS, workers=0) as toolbox:
        assert type(toolbox.session()) is Session


# -- the clock, the result, the log ---------------------------------------------------------------


def test_the_sessions_clock_dates_a_new_document():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session(clock=clock)
        made = toolbox.dispatch(session, "new_document", {"kind": "docx"})
        assert made.ok, made.to_json()
        core = zipfile.ZipFile(io.BytesIO(session.save(made.doc).data)).read("docProps/core.xml").decode()
    assert "2031-03-04T05:06:07Z" in core, core


def test_a_result_is_the_replys_json_and_its_record_is_logged(box):
    session, doc = opened(box)
    done = box.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": "Hello there."})
    assert isinstance(done, HostResult) and done.ok
    assert done.version == 1 and done.doc == doc
    assert done.touched and done.changed_parts, "the application's part"
    assert done.timing and done.timing["total"] >= done.timing["work"]
    refused = box.dispatch(session, "word_set_text", {"doc": doc, "target": "p:00000000", "text": "x"})
    assert not refused.ok and isinstance(refused.error, ToolError)
    assert refused.to_json()["error"]["code"] == refused.error.code
    assert [r.tool for r in session.log] == ["word_insert_markdown", "word_set_text"]
    assert session.log[1].ok is False and session.log[1].code == refused.error.code


def test_a_batchs_op_comments_are_kept_in_the_log(box):
    session, doc = opened(box)
    ops = [{"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": "end", "markdown": "One."},
            "comment": "add the first"}]
    assert box.dispatch(session, "batch", {"ops": ops}).ok
    assert session.log[-1].comments == [{"op": 0, "comment": "add the first"}]


def test_a_save_document_output_reaches_the_application(box):
    session, doc = opened(box)
    saved = box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx", "format": "docx"})
    assert saved.ok, saved.to_json()
    outputs = session.take_outputs()
    assert [o.name for o in outputs] == ["out.docx"]
    assert outputs[0].data.startswith(b"PK") and outputs[0].state


def test_roles_set_on_the_session_hold_its_calls(box):
    session, doc = opened(box)
    session.roles = ["review"]
    refused = box.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": "No."})
    assert not refused.ok and refused.error.details.get("roles") == ["review"], refused.to_json()
    with pytest.raises(ValueError):
        box.dispatch(session, "word_read", {"doc": doc}, roles=["edit"])


# -- the application's own tools (decision D1) ----------------------------------------------------


def test_an_application_tool_runs_in_the_facade_and_in_a_batch_is_refused_call_it_alone():
    with Toolbox([*TOOLS, app_note], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session, doc = opened(toolbox)
        assert isinstance(session, HostedSession)
        alone = toolbox.dispatch(session, "app_note", {"text": "hi"})
        assert alone.ok and alone.data == {"text": "hi"}, alone.to_json()
        batch = toolbox.dispatch(session, "batch", {"ops": [
            {"tool": "app_note", "arguments": {"text": "hi"}},
            {"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": "end", "markdown": "Kept."}}]})
        assert batch.ok, batch.to_json()
        refused = batch.data["ops"][0]["error"]
        assert refused["message"] == ("app_note is the application's own tool, which cannot run inside a batch: "
                                      "call it alone")
        assert batch.data["applied"] == 1


@tool("app_peek", "Test only: an application's own tool that names a document.", {"doc": string("Document id.")})
def app_peek(call, doc):
    return Result(summary="peeked")


def test_an_application_tool_naming_a_document_the_host_holds_is_refused_with_the_way_out():
    with Toolbox([*TOOLS, app_peek], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session, doc = opened(toolbox)
        refused = toolbox.dispatch(session, "app_peek", {"doc": doc})
        assert not refused.ok and refused.error.code == "refused", refused.to_json()
        assert refused.error.details == {"held_by_host": [doc]}
        assert "read or change it with the host's tools" in refused.error.message
        assert session.log[-1].tool == "app_peek" and session.log[-1].code == "refused"


# -- the application's tools' stores: the Rust session's (E1b, decision D3) ------------------------


ROWS = [{"Region": "North", "Units": 3, "Day": dt.date(2026, 1, 5)},
        {"Region": "South", "Units": 2.5, "Day": None}]


@tool("app_keep", "Test only: keep a made-up dataset.", {"id": string("Its id."), "units": string("North's units.")},
      documents=())
def app_keep(call, id, units):
    from ooxml_edit.tools import Dataset

    rows = [dict(ROWS[0], Units=int(units)), ROWS[1]]
    call.define_dataset(id, Dataset.from_records(rows, types={"Units": "number"}), source=None)
    return Result(summary=f"kept d:{id}")


@tool("app_show", "Test only: show images.", {"count": string("How many.")}, documents=())
def app_show(call, count):
    call.session.reserve_images(int(count))
    return Result(summary="shown")


@pytest.fixture(scope="module")
def own_box():
    from ooxml_edit.tools import Limits

    with Toolbox([*TOOLS, app_keep, app_show], formats=[FORMAT], groups=GROUPS, workers=0,
                 limits=Limits(image_budget=3)) as toolbox:
        yield toolbox


def test_an_application_tools_dataset_is_kept_in_the_rust_session_and_placed_from(own_box):
    session, doc = opened(own_box)
    assert own_box.dispatch(session, "app_keep", {"id": "sales", "units": "3"}).ok
    assert session.dataset_ids() == ["d:sales"]
    entry = session.dataset("d:sales")
    assert entry.dataset.records() == ROWS, "the values exactly as kept"
    assert not entry.stale and not entry.closed and entry.doc is None
    placed = own_box.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end",
                                                               "markdown": "North: {{d:sales/North/Units}}."})
    assert placed.ok, placed.to_json()
    assert "North: 3." in own_box.dispatch(session, "word_read", {"doc": doc}).data
    described = own_box.dispatch(session, "describe", {"doc": doc}).data["datasets"]
    assert described == [{"id": "d:sales", "columns": ["Region", "Units", "Day"], "types": {"Units": "number"},
                          "rows": 2}]


def test_a_dataset_kept_again_is_read_fresh_and_one_dropped_is_gone(own_box):
    session, _doc = opened(own_box)
    assert own_box.dispatch(session, "app_keep", {"id": "sales", "units": "3"}).ok
    first = session.dataset("d:sales")
    assert session.dataset("d:sales") is first, "one view while the keeping is the same"
    assert own_box.dispatch(session, "app_keep", {"id": "sales", "units": "7"}).ok
    again = session.dataset("d:sales")
    assert again is not first and again.serial != first.serial
    assert again.dataset.rows[0][1] == 7
    session.drop_dataset("d:sales")
    with pytest.raises(ToolError) as refused:
        session.dataset("d:sales")
    assert refused.value.code == "not_found"
    from ooxml_edit.tools import Dataset, DatasetEntry

    session.keep_datasets([DatasetEntry("sales", Dataset(("Region",), [("West",)]))])
    assert session.dataset("d:sales").dataset.rows == (("West",),), "kept again after a drop: read fresh"


def test_a_dataset_the_host_kept_reads_back_from_its_json(own_box):
    from ooxml_edit.tools import Dataset, DatasetEntry
    from ooxml_edit.tools.hosted_data import HostDatasets, OtherValue

    session, _doc = opened(own_box)
    given = Dataset(("a", "b"), [(dt.datetime(2026, 1, 5, 9, 30), float("nan")), (2 ** 70, "x")],
                    sources=[[("A1",), ()], [(), ("B2",)]], keys=1, formats=(None, "0.0"),
                    markdown={(1, 1): "**x**"})
    session.keep_datasets([DatasetEntry("odd", given)])
    fresh = HostDatasets(session.rust).get("d:odd").dataset      # (no copy of its own: read from the JSON)
    assert fresh.rows == ((dt.datetime(2026, 1, 5, 9, 30), OtherValue("not a finite number")), (2 ** 70, "x"))
    assert fresh.sources == given.sources and fresh.keys == 1 and fresh.formats == (None, "0.0")
    assert fresh.markdown_at(1, 1) == "**x**"


def test_the_datasets_limit_is_the_rust_sessions(own_box):
    from ooxml_edit.tools import Dataset, DatasetEntry, Limits

    session = own_box.session(limits=Limits(max_datasets=1))
    session.keep_datasets([DatasetEntry("one", Dataset(("a",), [(1,)]))])
    with pytest.raises(ToolError) as refused:
        session.keep_datasets([DatasetEntry("two", Dataset(("a",), [(1,)]))])
    assert refused.value.code == "limit" and session.dataset_ids() == ["d:one"]


def test_an_application_tools_images_count_against_the_rust_sessions_budget(own_box):
    session, _doc = opened(own_box)
    assert session.images_remaining() == 3
    assert own_box.dispatch(session, "app_show", {"count": "2"}).ok
    assert session.images_remaining() == 1
    over = own_box.dispatch(session, "app_show", {"count": "2"})
    assert not over.ok and over.error.code == "limit" and over.error.details["budget"] == "session", over.to_json()
    session.release_images(1)
    assert session.images_remaining() == 2


def test_an_application_output_is_kept_in_the_rust_session_until_taken(own_box):
    from ooxml_edit.tools.session import Output

    session, doc = opened(own_box)
    session.add_output(Output("notes.txt", "txt", b"hello", validate={"new": []}))
    assert own_box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx", "format": "docx"}).ok
    taken = session.take_outputs()
    assert [(o.name, o.data[:2]) for o in taken] == [("notes.txt", b"he"), ("out.docx", b"PK")]
    assert taken[0].validate == {"new": []} and session.take_outputs() == []
    handed = []
    session = own_box.session(on_output=handed.append)
    doc = session.open(word_docs.blank(), "report.docx")
    assert own_box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx", "format": "docx"}).ok
    assert [o.name for o in handed] == ["out.docx"] and session.take_outputs() == []


# -- a turn ------------------------------------------------------------------------------------------


def test_dispatch_many_answers_as_the_calls_in_the_order_given(box):
    session, doc = opened(box)
    other = session.open(word_docs.blank(), "other.docx")
    calls = [("word_insert_markdown", {"doc": doc, "at": "end", "markdown": "First."}),
             ("word_read", {"doc": doc}),
             ("word_insert_markdown", {"doc": other, "at": "end", "markdown": "Elsewhere."}),
             ("word_insert_markdown", {"doc": doc, "at": "end", "markdown": "Second."}),
             ("word_read", {"doc": doc})]
    results = box.dispatch_many(session, calls)
    assert all(r.ok for r in results), [r.to_json() for r in results if not r.ok]
    assert "First." in results[1].data and "Second." not in results[1].data
    assert "Second." in results[4].data
    assert [r.version for r in results] == [1, 1, 1, 2, 2]


# -- the format's settings -----------------------------------------------------------------------


def test_docx_formats_min_text_size_reaches_check_as_one_number_or_by_kind():
    small = '<w:p><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>small print</w:t></w:r></w:p>'
    data = word_docs.with_body(small)
    for minimum in (10, {"body": 10}):
        with Toolbox(TOOLS, formats=[docx_format(min_text_size=minimum)], groups=GROUPS, workers=0) as toolbox:
            session = toolbox.session(clock=clock)
            doc = session.open(data, "small.docx")
            checked = toolbox.dispatch(session, "check", {"doc": doc, "include": ["layout"]})
            assert checked.ok, checked.to_json()
            assert checked.data["layout"].get("small_text"), (minimum, checked.data)
    with Toolbox(TOOLS, formats=[docx_format(min_text_size={"table": 10})], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session(clock=clock)
        doc = session.open(data, "small.docx")
        checked = toolbox.dispatch(session, "check", {"doc": doc, "include": ["layout"]})
        assert checked.ok and not checked.data["layout"].get("small_text"), "a kind left out is not checked"


def test_docx_formats_typography_off_writes_text_as_given():
    text = "Note : voir ci-dessous"
    for typography, spaced in ((True, True), (False, False)):
        with Toolbox(TOOLS, formats=[docx_format(typography=typography)], groups=GROUPS, workers=0) as toolbox:
            session = toolbox.session(clock=clock)
            doc = session.open(word_docs.blank(language="fr-FR"), "note.docx")
            done = toolbox.dispatch(session, "word_insert_markdown", {"doc": doc, "at": "end", "markdown": text})
            assert done.ok, done.to_json()
            body = zipfile.ZipFile(io.BytesIO(session.save(doc).data)).read("word/document.xml").decode()
            assert ("Note :" in body) is spaced, (typography, re.findall(r"Note.{0,3}:", body))
