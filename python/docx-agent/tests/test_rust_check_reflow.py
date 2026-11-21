"""``check``'s ``reflow`` answered by the Rust session (lane P5b) against the Python path.

A check of a document the session owns with ``reflow`` (asked, or by default) is laid out
by the document's page engine, handed to the session's ``check``: its page count, coverage
and first warnings, and ``since_last_check`` against what the document's last check saw,
which the session keeps (:func:`docx_agent.rust.reads.last_check`).  The same calls made
by the Python path in a session of their own answer the same, but for the seconds in
``timing`` and its laying out and waits (:data:`LAYING_OUT`); a check here and a check
there in one session measure from each other.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Version"):
    pytest.skip("ooxml_core has no Version", allow_module_level=True)

from ooxml_edit.tools import Toolbox

from docx_agent.rust import owner as rust_owner
from docx_agent.rust import reads
from docx_agent.tools import FORMAT, GROUPS, TOOLS

HERE = Path(__file__).resolve().parent
FIXTURES = sorted((HERE / "fixtures").rglob("*.docx"))[::9]


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


#: Where a check's time went to lay out and to wait: what each path had laid out before
#: (docx-agent keeps a version's check facts; the session asks its engine each time) and the
#: Python path's waits for its workers -- not compared.
LAYING_OUT = {"layout", "worker_wait", "document_wait"}


def _seconds_out(answer: dict) -> dict:
    """``answer`` without seconds, its parts' names in ``timing`` but :data:`LAYING_OUT`."""
    data = answer.get("data")
    if isinstance(data, dict) and isinstance(data.get("timing"), dict):
        data["timing"] = sorted(k for k in data["timing"] if k not in LAYING_OUT)
    for key in ("duration_ms", "timing"):
        answer.pop(key, None)
    return answer


def _steps(toolbox, path: Path, monkeypatch, *, rust: bool) -> list:
    """The answers of a check, the same check again, an edit and two checks after it."""
    out = []
    with monkeypatch.context() as m:
        if not rust:
            m.setattr(reads, "serve_check", lambda call, arguments: None)
            m.setattr(rust_owner, "session_validation", lambda entry: None)
        session = toolbox.session()
        doc = session.open(path.read_bytes(), name=path.name)
        if reads._owned(session.entry(doc)) is None:
            pytest.skip("the session does not own this document")

        def check(**arguments) -> None:
            served = reads.served().get("check", 0)
            reply = toolbox.dispatch(session, "check", {"doc": doc, **arguments}).to_json()
            if rust:
                assert reads.served().get("check", 0) == served + 1, reads.fallbacks().get("check")
            out.append(_seconds_out(json.loads(json.dumps(reply))))

        check()
        check(include=["reflow"])
        paragraphs = session.entry(doc).document.paragraphs()
        if paragraphs:
            done = toolbox.dispatch(session, "word_insert_text", {"doc": doc, "target": paragraphs[0].id,
                                                                   "text": " checked " * 40})
            assert done.ok, done.to_json()
        check(include=["validate", "reflow"])
        check(include=["reflow", "fields"], pages=[1])
    return out


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.name)
def test_the_session_answers_reflow_as_python_does(toolbox, path, monkeypatch):
    rust = _steps(toolbox, path, monkeypatch, rust=True)
    python = _steps(toolbox, path, monkeypatch, rust=False)
    for k, (a, b) in enumerate(zip(rust, python)):
        assert a == b, (path.name, k)


def test_checks_here_and_there_measure_from_each_other(toolbox, monkeypatch):
    path = FIXTURES[0]
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    if reads._owned(session.entry(doc)) is None:
        pytest.skip("the session does not own this document")
    first = toolbox.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).data["reflow"]
    assert "since_last_check" not in first
    with monkeypatch.context() as m:
        m.setattr(reads, "serve_check", lambda call, arguments: None)
        again = toolbox.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).data["reflow"]
    assert "since_last_check" not in again            # the version the served check saw
    target = session.entry(doc).document.paragraphs()[0].id
    assert toolbox.dispatch(session, "word_insert_text", {"doc": doc, "target": target, "text": " x"}).ok
    with monkeypatch.context() as m:
        m.setattr(reads, "serve_check", lambda call, arguments: None)
        python = toolbox.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).data["reflow"]
    assert "since_last_check" in python
    served = toolbox.dispatch(session, "check", {"doc": doc, "include": ["reflow"]}).data["reflow"]
    assert "since_last_check" not in served           # the version Python's check saw
