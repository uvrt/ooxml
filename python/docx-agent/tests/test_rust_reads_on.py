"""docx-agent's reading tools answered by the Rust session by default (lane P1c), against
the Python path they replace.

Through the toolbox -- the version a call reads handed to the Rust session as the parts that
changed, the call run there with the interpreter lock let go -- ``describe``, ``word_read``,
``word_inspect`` (with ``layout`` too), ``find_text`` (a regular expression too) and
``read_batch`` answer every fixture as the Python path does, before and after edits; what
Rust does not answer the same yet goes to the Python path, counted with why
(``docx_agent.rust.reads.fallbacks``), never silently.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Version"):          # an ooxml-core before 0.10.0
    pytest.skip("ooxml_core has no Version", allow_module_level=True)

from ooxml_edit.tools import Toolbox

from docx_agent.rust import reads
from docx_agent.tools import FORMAT, GROUPS, TOOLS

HERE = Path(__file__).resolve().parent
FIXTURES = sorted((HERE / "fixtures").glob("*/*.docx"))


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


@pytest.fixture(scope="module")
def rust_session():
    session = ooxml_core.Session()
    before = reads.use_session(session)
    try:
        yield session
    finally:
        reads.use_session(before)


def _answer(toolbox, session, tool, arguments, rust) -> dict:
    """``tool``'s answer through the toolbox, by the Rust session (``rust``) or the Python
    path (no session)."""
    before = reads.use_session(rust)
    try:
        return json.loads(json.dumps(toolbox.dispatch(session, tool, arguments).to_json()))
    finally:
        reads.use_session(before)


def _calls(session, doc: str) -> list[tuple[str, dict]]:
    document = session.entry(doc).document
    paragraphs = document.paragraphs()
    tables = document.tables()
    calls: list[tuple[str, dict]] = [("describe", {"doc": doc})]
    for view in ("final", "markup", "original"):
        calls.append(("word_read", {"doc": doc, "view": view}))
    calls.append(("word_read", {"doc": doc, "stories": "all"}))
    targets = ["body", "p:0BADF00D"]
    if paragraphs:
        targets.append(paragraphs[len(paragraphs) // 2].id)
    if len(paragraphs) >= 3:
        targets.append(f"{paragraphs[0].id}..{paragraphs[2].id}")
        calls.append(("word_read", {"doc": doc, "range": f"{paragraphs[0].id}..{paragraphs[2].id}"}))
    if tables:
        targets += [tables[0].id, f"{tables[0].id}/c0,0"]
    calls += [("word_inspect", {"doc": doc, "target": t}) for t in targets]
    calls += [("find_text", {"doc": doc, "text": "e"}),
              ("find_text", {"doc": doc, "text": "e", "stories": "all"}),
              ("find_text", {"doc": doc, "text": r"\b[A-Z]\w+", "regex": True}),
              ("find_text", {"doc": doc, "text": "e*", "regex": True})]
    calls.append(("read_batch", {"ops": [{"tool": "find_text", "arguments": {"doc": doc, "text": "a"}},
                                         {"tool": "word_read", "arguments": {"doc": doc, "range": "p:0BADF00D"}},
                                         {"tool": "describe", "arguments": {"doc": doc}}]}))
    return calls


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: f"{p.parent.name}/{p.stem}")
def test_the_reads_answer_as_the_python_path_does(toolbox, rust_session, path):
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        served = sum(reads.served().values())
        for tool, arguments in _calls(session, doc):
            rust = _answer(toolbox, session, tool, arguments, rust_session)
            python = _answer(toolbox, session, tool, arguments, None)
            assert rust == python, (tool, arguments)
        assert sum(reads.served().values()) > served
    finally:
        session.close(doc)


def test_a_new_version_is_handed_over_as_the_parts_that_changed(toolbox, rust_session):
    path = HERE / "fixtures" / "generated" / "ids-and-markup.docx"
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        first = session.entry(doc).document.paragraphs()[0].id
        before = _answer(toolbox, session, "word_read", {"doc": doc}, rust_session)
        edited = toolbox.dispatch(session, "word_set_text", {"doc": doc, "target": first, "text": "Changed here."})
        assert edited.ok, edited.to_json()
        for tool, arguments in [("word_read", {"doc": doc}), ("describe", {"doc": doc}),
                                ("word_inspect", {"doc": doc, "target": first}),
                                ("find_text", {"doc": doc, "text": "Changed"})]:
            rust = _answer(toolbox, session, tool, arguments, rust_session)
            assert rust == _answer(toolbox, session, tool, arguments, None), tool
        assert "Changed here." in _answer(toolbox, session, "word_read", {"doc": doc}, rust_session)["data"]
        assert before != _answer(toolbox, session, "word_read", {"doc": doc}, rust_session)
        mirror = session.entry(doc).document.__dict__["_rust_reads"]
        assert len(mirror.versions) == 2
    finally:
        session.close(doc)


def test_what_rust_does_not_answer_the_same_falls_back_counted(toolbox, rust_session):
    path = HERE / "fixtures" / "generated" / "ids-and-markup.docx"
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        before = reads.fallbacks().get("find_text", {}).get("regex", 0)
        found = _answer(toolbox, session, "find_text", {"doc": doc, "text": "e*", "regex": True}, rust_session)
        assert found == _answer(toolbox, session, "find_text", {"doc": doc, "text": "e*", "regex": True}, None)
        assert reads.fallbacks()["find_text"]["regex"] == before + 1
        first = session.entry(doc).document.paragraphs()[0].id
        deleted = toolbox.dispatch(session, "word_delete", {"doc": doc, "target": first, "force": True})
        assert deleted.ok, deleted.to_json()
        gone = _answer(toolbox, session, "word_inspect", {"doc": doc, "target": first}, rust_session)
        assert gone == _answer(toolbox, session, "word_inspect", {"doc": doc, "target": first}, None)
        assert reads.fallbacks()["word_inspect"]["renamed or deleted id"] >= 1
    finally:
        session.close(doc)


def test_inspect_with_layout_places_the_blocks_as_the_python_path_does(toolbox, rust_session):
    path = HERE / "fixtures" / "generated" / "ids-and-markup.docx"
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    try:
        served = reads.served().get("word_inspect", 0)
        arguments = {"doc": doc, "target": "body", "layout": True}
        rust = _answer(toolbox, session, "word_inspect", arguments, rust_session)
        assert rust == _answer(toolbox, session, "word_inspect", arguments, None)
        assert "placements" in json.dumps(rust)
        assert reads.served()["word_inspect"] == served + 1
    finally:
        session.close(doc)
