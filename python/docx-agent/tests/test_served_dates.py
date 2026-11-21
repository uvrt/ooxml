"""Lane P2e1: a served call's writes are dated by the session's clock, to the second, as
docx-agent's handlers date them (``Session.edit(date=...)``, every dated write); a session
with its own clock is served (no fallback).  Made-up documents only."""

from __future__ import annotations

import datetime as dt
import io
import zipfile

import pytest

from docx_agent import Document
from docx_agent.rust import edits
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry
from python_handler import python_session

CLOCK = lambda: dt.datetime(2026, 11, 20, 9, 41, 17, tzinfo=dt.timezone.utc)  # noqa: E731
STAMP = 'w:date="2026-11-20T09:41:17Z"'


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


@pytest.fixture(scope="module")
def report() -> bytes:
    doc = Document.new()
    doc.insert_markdown("# A made-up report\n\nLantern meadow by the harbour.\n")
    return doc.to_bytes()


def _tracked_edit(toolbox, session, doc) -> tuple[str, int]:
    assert toolbox.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Ann"}).ok
    ids = {p.text: p.id for p in session.entry(doc).document.paragraphs()}
    before = dict(edits.served())
    result = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": ids["Lantern meadow by the harbour."],
                                                         "text": "Lantern meadow by the river."})
    assert result.ok, result.error
    served = edits.served().get("word_set_text", 0) - before.get("word_set_text", 0)
    data = session.save(doc).data
    xml = zipfile.ZipFile(io.BytesIO(data)).read("word/document.xml").decode()
    return xml, served


def test_a_served_revision_is_dated_to_the_second(toolbox, report):
    session = toolbox.session(clock=CLOCK)
    doc = session.open(report, "report.docx")
    if not isinstance(session.entry(doc), OwnedEntry):
        pytest.skip("no Rust session to own the document")
    xml, served = _tracked_edit(toolbox, session, doc)
    assert served == 1, edits.fallbacks()
    assert STAMP in xml
    session, doc = python_session(toolbox, report, "report.docx", clock=CLOCK)
    python, _ = _tracked_edit(toolbox, session, doc)
    assert STAMP in python
