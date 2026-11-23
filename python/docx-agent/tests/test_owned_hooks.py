"""Lane H7a: the docx format's hooks for a document the Rust session owns read the session.

ooxml-edit gives the hooks an ``OwnedDocument`` (the owner, its handle, the version) for such a
document -- never the entry or a Python view -- and each answers from the session's reading
of the version: ``summary``, ``problems``, ``state``, ``closed`` and ``restored``.  The id
state is a record the session opens its document with again, as the same record.
"""

from __future__ import annotations

import base64
import json
import zlib

import pytest

ooxml_core = pytest.importorskip("ooxml_core")

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from ooxml_edit.tools import OwnedDocument, Toolbox  # noqa: E402

import word_docs  # noqa: E402

REPORT = "# Part one\n\n" + "\n\n".join(f"Text {k}." for k in range(6)) + "\n\n| a | b |\n|---|---|\n| 1 | 2 |"


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _owned(session, doc):
    entry = session.entry(doc)
    assert entry.owned
    subject = entry.subject
    assert isinstance(subject, OwnedDocument)
    return entry, subject


def _record(state: str) -> dict:
    """An id state's record: what it says, whatever bytes deflate wrote."""
    prefix = "docx-ids.1."
    assert state.startswith(prefix)
    body = state[len(prefix):]
    return json.loads(zlib.decompress(base64.urlsafe_b64decode(body + "=" * (-len(body) % 4))))


def _report(box) -> bytes:
    session = box.session()
    doc = session.open(word_docs.blank(), "report.docx")
    word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=REPORT)
    return word_docs.saved(session, doc)


def test_the_state_of_an_owned_document_opens_it_again_as_the_same_record(box):
    session = box.session()
    doc = session.open(_report(box), "report.docx")
    writer = session.view(context={"agent": "writer-2"})
    entry, _subject = _owned(session, doc)
    blocks = [block for block, text in word_docs.blocks(box, session, doc) if block.startswith("p:") and text]
    assert box.dispatch(writer, "word_delete", {"doc": doc, "target": blocks[3]}).ok
    assert box.dispatch(session, "word_set_text", {"doc": doc, "target": blocks[2], "text": "Changed."}).ok
    made = box.dispatch(session, "word_insert_markdown", {"doc": doc, "markdown": "More.", "at": "end"})
    assert made.ok, made.to_json()
    state = FORMAT.state(entry.subject)
    ours = _record(state)
    # The session drew the new ids, so it knows where its seeds' draws got to.
    assert ours["seeds"]
    data = session.save(doc).data
    other = box.session()
    assert other.open(data, "report.docx", doc_id=doc, state=state) == doc
    assert _record(FORMAT.state(other.entry(doc).subject)) == ours
    # The deleted paragraph's id is still refused as deleted after the reopen.
    refused = box.dispatch(other, "word_set_text", {"doc": doc, "target": blocks[3], "text": "x"})
    assert not refused.ok and refused.error.code == "not_found", refused.to_json()


def test_save_close_and_open_document_s_summary_never_read_the_view(box, monkeypatch):
    from ooxml_edit.tools.owned import OwnedEntry

    session = box.session()
    doc = session.open(_report(box), "report.docx")
    entry, _subject = _owned(session, doc)

    def no_view(self):
        raise AssertionError("a format hook read the Python view")

    monkeypatch.setattr(OwnedEntry, "document", property(no_view))
    saved = session.save(doc)
    assert saved.state
    summary = FORMAT.summary(entry.subject)
    assert summary["tables"] == 1 and summary["headings"] == 1 and summary["template"] is False
    session.close(doc)
