"""``word_inspect``, ``find_text`` and ``read_batch`` served by the Rust session (lane P1b)
against today's.

The Rust definitions (``crates/ooxml-tools-word/tests/golden``) equal today's byte for byte,
and the replies of ``ooxml_core.Session.call`` equal today's on every fixture: ``word_inspect``
of the body and of a cell, a table, a span and a missing id (a drawing's chart and SmartArt
JSON included), ``find_text`` of a letter in the body and in
every story, ``read_batch`` of several reads with a failing one among them.
"""
from __future__ import annotations

import dataclasses
import json
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from ooxml_edit.tools import Toolbox

from docx_agent import Document
from docx_agent.rust.session import RustTools
from docx_agent.tools import FORMAT, GROUPS, TOOLS

HERE = Path(__file__).resolve().parent
GOLDEN = HERE.parents[2] / "crates" / "ooxml-tools-word" / "tests" / "golden"
FIXTURES = sorted((HERE / "fixtures").rglob("*.docx"))


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


@pytest.mark.parametrize("name", ["word_inspect", "find_text", "read_batch"])
def test_the_rust_definitions_are_todays_byte_for_byte(name):
    from test_rust_tool_specs import export, text

    today = {tool.name: tool for tool in TOOLS}[name]
    if name == "read_batch":     # documents named by a function today: none
        today = dataclasses.replace(today, documents=())
    golden = (GOLDEN / f"{name}.json").read_text(encoding="utf-8")
    assert golden == text(export(today))
    specs = json.loads(ooxml_core.tool_specs("docx"))
    assert json.loads(golden)["schema"] in [s["schema"] for s in specs if s["name"] == name]


def _targets(data: bytes) -> list[str]:
    document = Document.open(data)
    out = ["body", "p:0BADF00D"]
    paragraphs = document.paragraphs()
    if paragraphs:
        out.append(paragraphs[len(paragraphs) // 2].id)
    if len(paragraphs) >= 3:
        out.append(f"{paragraphs[0].id}..{paragraphs[2].id}")
    tables = document.tables()
    if tables:
        out += [tables[0].id, f"{tables[0].id}/c0,0"]
    return out


def _calls(data: bytes) -> list[tuple[str, dict]]:
    calls = [("word_inspect", {"target": t}) for t in _targets(data)]
    calls += [("find_text", {"text": "e"}), ("find_text", {"text": "e", "stories": "all"})]
    calls.append(("read_batch", {"ops": [{"tool": "find_text", "arguments": {"doc": "d1", "text": "a"}},
                                         {"tool": "word_read", "arguments": {"doc": "d1", "range": "p:0BADF00D"}},
                                         {"tool": "word_inspect", "arguments": {"doc": "d1", "target": "body"}}]}))
    return calls


def _comparable(tool: str, reply: dict) -> dict:
    if reply.get("error") is not None:
        return {"code": reply["error"]["code"], "message": reply["error"]["message"]}
    data = reply.get("data")
    if tool == "read_batch":
        data = {**data, "ops": [_comparable(op["tool"], {**op, "data": op.get("data")}) for op in data["ops"]]}
    return {"summary": reply.get("summary"), "data": data, "next_cursor": reply.get("next_cursor"),
            "total": reply.get("total")}


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: str(p.relative_to(HERE / "fixtures")))
def test_the_rust_reads_equal_todays_on_the_fixtures(toolbox, path):
    data = path.read_bytes()
    session = toolbox.session()
    doc = session.open(data, name=path.name)
    rust = RustTools()
    rust_doc = rust.open(data, path.name)
    assert rust_doc == doc == "d1"
    try:
        for tool, arguments in _calls(data):
            today = json.loads(json.dumps(toolbox.dispatch(session, tool, {"doc": doc, **arguments}).to_json()))
            if tool == "read_batch":
                today = json.loads(json.dumps(toolbox.dispatch(session, tool, arguments).to_json()))
                reply = rust.dispatch(tool, arguments).to_json()
            else:
                reply = rust.dispatch(tool, {"doc": rust_doc, **arguments}).to_json()
            assert _comparable(tool, reply) == _comparable(tool, today), (tool, arguments)
    finally:
        session.close(doc)


def test_a_refusal_carries_its_way_out():
    path = FIXTURES[0]
    rust = RustTools()
    doc = rust.open(path.read_bytes(), path.name)
    refused = rust.dispatch("word_inspect", {"doc": doc, "target": "p:0000FFFF"}).to_json()["error"]
    assert refused["code"] == "not_found" and refused["field"] == "target"
    assert refused["try_call"]["arguments"] == {"doc": doc, "target": "body"}
    laid = rust.dispatch("word_inspect", {"doc": doc, "target": "body", "layout": True}).to_json()["error"]
    assert laid["code"] == "refused" and laid["try_call"]["arguments"] == {"doc": doc, "target": "body"}
