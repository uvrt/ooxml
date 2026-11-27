"""Lane P2c17, on the Rust host: every host session numbers its own documents (a second session's
``d1`` is its own), and every name a reply carries -- its undo and way-out calls, its summary, its
data, its refs -- is one of its session's documents.  (The core's session alone, with no host
names, keeps its own ids.)  Made-up documents only."""

from __future__ import annotations

import json
import re

import pytest

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from ooxml_edit.tools import Toolbox

import word_docs


_NAME = re.compile(r"\bd\d+\b")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        yield box


def _report(title: str) -> bytes:
    parts = [f"# {title}"]
    for c in range(1, 4):
        parts += [f"# Part {c}", f"Lantern meadow {c} by the harbour.", f"River stone number {c}."]
    parts.append("| Name | Value |\n|---|---|\n| Lantern | 1 |\n| Meadow | 2 |")
    return _made("\n\n".join(parts))


def _made(markdown: str) -> bytes:
    """A blank document with ``markdown`` inserted by the toolbox, as bytes."""
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        doc = session.open(word_docs.blank(), "made.docx")
        word_docs.call(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
        return word_docs.saved(session, doc)


def _names_in(result) -> set[str]:
    reply = [result.summary, result.data, result.warnings, dict(result.refs), sorted(result.changed)]
    return set(_NAME.findall(json.dumps(reply, default=str)))


def test_a_reply_names_only_its_sessions_documents(toolbox):
    first = toolbox.session()
    for k in range(2):
        first.open(_report(f"First session {k}"), f"first{k}.docx")
    session = toolbox.session()
    doc = session.open(_report("A made-up report"), "report.docx")
    other = session.open(_report("Another made-up report"), "other.docx")
    hosts = {doc, other}
    ids: dict[str, str] = {}
    for block, text in word_docs.paragraphs(session, doc):
        ids.setdefault(text, block)
    table = f"t:{word_docs.rows_of(word_docs.tables(session, doc)[0])[0].get(word_docs.W14_PARA)}"
    theirs = f"t:{word_docs.rows_of(word_docs.tables(session, other)[0])[0].get(word_docs.W14_PARA)}"
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
    seen: set[str] = set()
    for tool, arguments in calls:
        result = toolbox.dispatch(session, tool, {"doc": doc, **arguments})
        assert result.ok, (tool, result.error)
        assert _names_in(result) <= hosts, (tool, _names_in(result), result.summary, result.data)
        seen |= _names_in(result)
    assert doc in seen, "the replies name the document (the deletion's undo call)"


def test_a_reply_without_names_keeps_the_sessions_ids():
    """The session alone (no host names) still names documents by its own ids."""
    ooxml_core = pytest.importorskip("ooxml_core")
    session = ooxml_core.Session()
    doc = session.open(_report("A made-up report"), kind="docx", name="report.docx")
    reply, _images, _app = session.call("word_read", json.dumps({"doc": doc}))
    assert json.loads(reply)["doc"] == doc
    first = ooxml_core.CallOptions(documents=ooxml_core.CallDocuments(names={"first": doc}))
    named, _images, _app = session.call("word_read", json.dumps({"doc": "first"}), first)
    assert json.loads(named)["doc"] == "first"
    refused, _images, _app = session.call("word_read", json.dumps({"doc": doc}), first)
    assert json.loads(refused)["error"]["code"] == "not_found"
