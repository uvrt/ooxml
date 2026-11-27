"""The Toolbox over the Rust host (Phase 3, E1a): ``ooxml_edit.tools.Toolbox``, ``Session`` and
``Result`` with today's signatures, docx-agent's documents held and its calls run by the Rust
host (``ooxml_core.Host``, one per toolbox; ``HostSession``, one per session).

- Which toolbox runs on the host: the rule until E1b (every format the host's, no tool of the
  application's own), pinned.
- The session clock reaches the host as a value: a new document's created date is the clock's.
- A call's ``Result`` is built from the reply's JSON, its ``ToolError`` from the reply's error,
  and its record is appended to the session's log.
- An application's own tool runs in the facade, and inside a Rust batch is refused with the way
  out "call it alone".
- ``dispatch_many`` runs a turn over the facade, each result the one the order given has.
- ``docx_format``'s settings reach the host's Word format.

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


# -- which toolbox runs on the host (until E1b) ------------------------------------------------


def test_a_toolbox_of_the_hosts_formats_and_tools_runs_its_sessions_on_the_host(box):
    session = box.session()
    assert isinstance(session, HostedSession)
    assert isinstance(session.view(roles=["review"]), type(session.view()))


def test_until_e1b_a_toolbox_with_tools_of_its_own_runs_on_the_python_host():
    with Toolbox([*TOOLS, app_note], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session()
    assert type(session) is Session, "E1b's shared stores move it onto the host"


def test_until_e1b_a_toolbox_with_a_python_format_runs_on_the_python_host():
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
    # (the seam: a toolbox with a tool of its own on the host, which the rule leaves on the
    # Python host until E1b; E1b deletes the seam with the rule)
    with Toolbox([*TOOLS, app_note], formats=[FORMAT], groups=GROUPS, workers=0, _rust_host=True) as toolbox:
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


def test_docx_formats_min_text_size_is_checks_minimum_and_by_kind_is_refused_with_the_way_out():
    small = '<w:p><w:r><w:rPr><w:sz w:val="12"/></w:rPr><w:t>small print</w:t></w:r></w:p>'
    data = word_docs.with_body(small)
    with Toolbox(TOOLS, formats=[docx_format(min_text_size=10)], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session(clock=clock)
        doc = session.open(data, "small.docx")
        checked = toolbox.dispatch(session, "check", {"doc": doc, "include": ["layout"]})
        assert checked.ok, checked.to_json()
        assert "small_text" in str(checked.data)
    with Toolbox(TOOLS, formats=[docx_format(min_text_size={"body": 10})], groups=GROUPS, workers=0) as toolbox:
        session = toolbox.session(clock=clock)
        doc = session.open(data, "small.docx")
        refused = toolbox.dispatch(session, "check", {"doc": doc, "include": ["layout"]})
        assert not refused.ok
        assert refused.error.details["not_supported"] == "check: min_text_size by kind"
        assert refused.error.try_call["arguments"]["min_size"] == 9


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
