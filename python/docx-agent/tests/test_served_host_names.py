"""Lane P2c17: a call the Rust session answers names documents as its host does.  One Rust
session serves every host session of the process and numbers documents across them, so a
host's ``d1`` is the session's ``d3`` from the second host session on; every name a served
reply carries (its undo and way-out calls, its summary, its data, its refs) is the host's.
Made-up documents only."""

from __future__ import annotations

import json
import re

import pytest

from docx_agent import Document
from docx_agent.rust import edits, reads
from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.owned import OwnedEntry

_NAME = re.compile(r"\bd\d+\b")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _report(title: str) -> bytes:
    doc = Document.new()
    parts = [f"# {title}"]
    for c in range(1, 4):
        parts += [f"# Part {c}", f"Lantern meadow {c} by the harbour.", f"River stone number {c}."]
    parts.append("| Name | Value |\n|---|---|\n| Lantern | 1 |\n| Meadow | 2 |")
    doc.insert_markdown("\n\n".join(parts))
    return doc.to_bytes()


def _names_in(result) -> set[str]:
    reply = [result.summary, result.data, result.warnings, dict(result.refs), sorted(result.changed)]
    return set(_NAME.findall(json.dumps(reply, default=str)))


def test_a_served_reply_names_documents_as_its_host_does(toolbox):
    first = toolbox.session()
    for k in range(2):
        first.open(_report(f"First session {k}"), f"first{k}.docx")
    session = toolbox.session()
    doc = session.open(_report("A made-up report"), "report.docx")
    other = session.open(_report("Another made-up report"), "other.docx")
    entry = session.entry(doc)
    if not isinstance(entry, OwnedEntry):
        pytest.skip("no Rust session to own the document")
    assert entry.handle.doc != doc, "the second host session's names are not the session's ids"
    hosts = {doc, other}
    ids = {p.text: p.id for p in entry.document.paragraphs()}
    table = entry.document.tables()[0].id
    theirs = session.entry(other).document.tables()[0].id
    calls = [
        ("word_set_text", {"target": ids["River stone number 1."], "text": "River pebble number 1."}),
        # text the file had when opened, deleted: the reply's undo call names the document
        ("word_delete", {"target": ids["Lantern meadow 2 by the harbour."]}),
        ("word_notes", {"action": "insert", "find": "meadow 3", "text": "A made-up note.", "ref": "note"}),
        ("word_links", {"action": "add_bookmark", "target": ids["Part 2"], "bookmark": "part_two"}),
        ("word_format_table", {"table": table, "scope": "table", "style": "Table Grid"}),
        ("word_insert_markdown", {"at": "end", "markdown": "A closing line.", "ref": "closing"}),
        ("word_compare_blocks", {"blocks": [table], "reference": {"doc": other, "block": theirs}}),
        ("word_design_facts", {}),
        ("word_read", {}),
    ]
    edits_before, reads_before = dict(edits.served()), dict(reads.served())
    seen: set[str] = set()
    for tool, arguments in calls:
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, (tool, result.error)
        assert _names_in(result) <= hosts, (tool, _names_in(result), result.summary, result.data)
        seen |= _names_in(result)
    assert doc in seen, "the replies name the document (the deletion's undo call)"
    answered = sum(edits.served().get(t, 0) - edits_before.get(t, 0) for t, _ in calls)
    answered += sum(reads.served().get(t, 0) - reads_before.get(t, 0) for t, _ in calls)
    assert answered >= 6, (edits.fallbacks(), reads.fallbacks())


def test_a_reply_without_names_keeps_the_sessions_ids():
    """The session alone (no host names) still names documents by its own ids."""
    ooxml_core = pytest.importorskip("ooxml_core")
    session = ooxml_core.Session()
    doc = session.open(_report("A made-up report"), kind="docx", name="report.docx")
    reply, _images, _app = session.call("word_read", json.dumps({"doc": doc}))
    assert json.loads(reply)["doc"] == doc
    named, _images, _app = session.call("word_read", json.dumps({"doc": "first"}), names={"first": doc})
    assert json.loads(named)["doc"] == "first"
    refused, _images, _app = session.call("word_read", json.dumps({"doc": doc}), names={"first": doc})
    assert json.loads(refused)["error"]["code"] == "not_found"
