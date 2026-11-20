"""``word_read`` served by the Rust session (lane P1a) against today's.

The Rust definition (``crates/ooxml-tools-word``, golden ``tests/golden/word_read.json``
there) equals today's byte for byte, and the reply of ``ooxml_core.Session.call("word_read",
...)`` equals today's envelope on every fixture, in every view and with every story -- but
for one thing the Rust core does not read yet: a SmartArt diagram's one-line summary in its
drawing's comment (``[smartart: ...]``), which waits for the diagram's data model.  ``KNOWN``
takes those summaries out of today's text before the two are compared, so any other
difference fails.
"""
from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from ooxml_edit.tools import Toolbox

from docx_agent.rust.session import RustTools
from docx_agent.tools import FORMAT, GROUPS, TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden" / "word_read.json"
FIXTURES = sorted((HERE / "fixtures").rglob("*.docx"))

#: What the Rust core does not write yet: a diagram's summary in its comment.
KNOWN = re.compile(r" \[(?:d:[^ \]]+ )?smartart: [^\n]*?\](?= -->| \[| empty -->)")

CASES = {"final": {}, "markup": {"view": "markup"}, "original": {"view": "original"}, "all": {"stories": "all"}}


@pytest.fixture(scope="module")
def toolbox():
    """Today's tools, every read answered by the Python path (lane P1c: the toolbox's reads
    are the Rust session's by default; this compares the two)."""
    from docx_agent.rust import reads

    before = reads.use_session(None)
    try:
        yield Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)
    finally:
        reads.use_session(before)


def test_the_rust_definition_is_todays_byte_for_byte():
    from test_rust_tool_specs import export, text

    today = {tool.name: tool for tool in TOOLS}["word_read"]
    assert GOLDEN.read_text(encoding="utf-8") == text(export(today))
    specs = json.loads(ooxml_core.tool_specs("docx"))
    assert json.loads(GOLDEN.read_text(encoding="utf-8")) in specs


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: str(p.relative_to(HERE / "fixtures")))
def test_word_read_in_rust_equals_todays_on_the_fixtures(toolbox, path):
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    rust = RustTools()
    rust_doc = rust.open(path.read_bytes(), path.name)
    try:
        for case, arguments in CASES.items():
            today = json.loads(json.dumps(toolbox.dispatch(session, "word_read", {"doc": doc, **arguments}).to_json()))
            reply = rust.dispatch("word_read", {"doc": rust_doc, **arguments}).to_json()
            assert today["ok"] and reply["ok"], (case, today, reply)
            kept = KNOWN.sub("", today["data"])
            today["total"] -= len(today["data"]) - len(kept)     # each fixture reads in one page
            today["data"] = kept
            assert reply == today, case
    finally:
        session.close(doc)


def test_a_refusal_carries_its_way_out(toolbox):
    rust = RustTools()
    doc = rust.open(FIXTURES[0].read_bytes(), FIXTURES[0].name)
    refused = rust.dispatch("word_read", {"doc": doc, "range": "p:0000FFFF"})
    assert not refused.ok and refused.error.code == "not_found"
    envelope = refused.to_json()["error"]
    assert envelope["field"] == "range" and envelope["try_call"]["arguments"] == {"doc": doc}
