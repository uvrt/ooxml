"""Lane U4: ``undo`` of a Word document is the Rust session's own tool -- its definition the
shared one for a Word document (no ``scope``: a body is one scope every change shares), its
replies today's, word for word: the session's own undo and redo, a caller's own (refused
with the later change and who made it while it is not the latest), and a caller's own redo
refused while another caller's undo is the next to redo.  Lane U5: nothing (of the caller's)
to take is refused with its way out (tool rule 2), which today's reply lacks: the document's
steps and who made each, and the undo or redo the other way where that one works; a session
with its own clock undoes through the same tool."""

import datetime as dt
import json
from pathlib import Path

import pytest

from docx_agent import Document
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

GOLDEN = Path(__file__).resolve().parents[3] / "crates" / "ooxml-tools-word" / "tests" / "golden" / "undo.json"


CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 41, 17, tzinfo=dt.timezone.utc)  # noqa: E731


@pytest.fixture(params=["system clock", "own clock"])
def opened(request):
    document = Document.new()
    document.paragraphs()[0].set_text("First")
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=CLOCK) if request.param == "own clock" else box.session()
        doc = session.open(document.to_bytes(), "report.docx")
        if not isinstance(session.entry(doc), OwnedEntry):
            pytest.skip("no Rust session to own the document")
        first = session.entry(doc).document.paragraphs()[0].id

        def call(writer, tool, **arguments):
            view = session.view(context={"agent": writer}) if writer else session
            named = {} if tool == "batch" else {"doc": doc}
            out = box.dispatch(view, tool, {**named, **arguments}).to_json()
            out.pop("timing", None)
            return out

        yield call, first, session.entry(doc)


def test_a_callers_own_undo_names_the_later_change_and_who_made_it(opened):
    call, first, entry = opened
    assert call("a", "undo")["error"]["message"] == (
        "nothing of yours to undo in d1: undo here takes back your own changes only")
    assert call(None, "undo", redo=True)["error"] == {
        "code": "refused", "message": "nothing to redo in d1",
        "valid_options": [{"version": 0, "oldest": True}]}
    call("a", "word_insert_text", at=f"{first}@0", text="A ")
    call("b", "word_insert_text", at=f"{first}@0", text="B ")
    refused = call("a", "undo")
    assert refused["version"] == 2
    assert refused["error"] == {
        "code": "entangled", "valid_options": [],
        "message": "your latest change to d1 (word_insert_text, version 1) is not the latest: "
                   "word_insert_text by agent=b changed d1 after it (version 2), and undoing yours "
                   "would take back what came after. Change it back with an edit, or ask whoever "
                   "made the later change to undo it first",
        "details": {"version": 1, "later": [
            {"version": 2, "tool": "word_insert_text", "context": {"agent": "b"}}]}}
    undone = call("b", "undo", steps=3)
    assert (undone["summary"], undone["data"], undone["version"]) == (
        "Undid 1 of your own step(s) of d1", {"steps": 1, "version": 1, "own": True}, 1)
    assert undone["warnings"] == [
        "1 of 3 step(s): no more of yours, or the next depends on a later change by another caller"]
    assert entry.version == 1 and entry.document.paragraphs()[0].text == "A First"
    # a's own undo now, then b's own redo is not the next: a's undo is
    assert call("a", "undo")["data"] == {"steps": 1, "version": 0, "own": True}
    assert call("b", "undo", redo=True)["error"]["message"] == (
        "your undone change to d1 (version 2) is not the next to redo: another caller undid a "
        "change since. Make your change again instead")
    assert call("a", "undo", redo=True)["data"] == {"steps": 1, "version": 1, "own": True}
    assert call("b", "undo", redo=True)["data"] == {"steps": 1, "version": 2, "own": True}
    # the session's own undo and redo take any caller's steps
    assert call(None, "undo", steps=2)["data"] == {"steps": 2, "version": 0}
    redone = call(None, "undo", redo=True, steps=5)
    assert (redone["summary"], redone["warnings"], entry.version) == ("Redid 2 step(s) of d1", [], 2)
    assert entry.document.paragraphs()[0].text == "B A First"
    assert call(None, "undo", scope="256")["error"]["field"] == "scope"


def test_the_session_api_counts_and_a_batch_is_one_step_of_its_caller(opened):
    call, first, entry = opened
    out = call("a", "batch", ops=[
        {"tool": "word_insert_text", "arguments": {"doc": "d1", "at": f"{first}@0", "text": "1 "}},
        {"tool": "word_insert_text", "arguments": {"doc": "d1", "at": f"{first}@0", "text": "2 "}}])
    assert out["ok"], json.dumps(out)
    call("b", "word_insert_text", at=f"{first}@0", text="B ")
    error = call("a", "undo")["error"]
    assert "your latest change to d1 (batch, version 1)" in error["message"]
    details = error["details"]
    assert details["later"] == [{"version": 2, "tool": "word_insert_text", "context": {"agent": "b"}}]
    assert call("b", "undo")["data"]["steps"] == 1
    assert call("a", "undo")["data"] == {"steps": 1, "version": 0, "own": True}
    assert entry.document.paragraphs()[0].text == "First"


def test_nothing_to_take_lists_the_steps_and_who_made_each_and_the_call_that_works(opened):
    call, first, _entry = opened
    call("a", "word_insert_text", at=f"{first}@0", text="A ")
    call("b", "word_insert_text", at=f"{first}@0", text="B ")
    by_a = {"version": 1, "tool": "word_insert_text", "context": {"agent": "a"}}
    by_b = {"version": 2, "tool": "word_insert_text", "context": {"agent": "b"}}
    oldest = {"version": 0, "oldest": True}
    # c made none of them: the steps, who made each, and no call of c's that would work
    error = call("c", "undo")["error"]
    assert error["code"] == "refused" and "try_call" not in error
    assert error["valid_options"] == [by_b, by_a, oldest]
    # b's own undone: its redo is the way out of a second undo, its step marked
    assert call("b", "undo")["ok"]
    error = call("b", "undo")["error"]
    assert error["message"] == "nothing of yours to undo in d1: undo here takes back your own changes only"
    assert error["valid_options"] == [{**by_b, "yours": True, "undone": True}, by_a, oldest]
    assert error["try_call"] == {"tool": "undo", "arguments": {"doc": "d1", "redo": True}}
    assert call("b", "undo", redo=True)["ok"]
    # the session's own redo with nothing to redo: its undo works
    error = call(None, "undo", redo=True)["error"]
    assert error["valid_options"] == [by_b, by_a, oldest]
    assert error["try_call"] == {"tool": "undo", "arguments": {"doc": "d1"}}
