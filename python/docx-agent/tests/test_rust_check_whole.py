"""``check`` answered whole by the Rust session (lane P5a) against the Python path it replaces.

A check of a document the session owns that reads the document alone -- ``validate``,
``fields``, ``app`` -- is the session's ``check`` (``ooxml_tools_word::check``), and its
``validation`` (the save gate's comparison) the session's too
(:func:`docx_agent.rust.owner.session_validation`): neither makes the Python view current.
Every fixture, before and after an edit, answers as the Python path does but for the
seconds in ``timing``; a check that needs the layout goes to the Python path, counted.
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
FIXTURES = sorted((HERE / "fixtures").rglob("*.docx"))[::3]

INCLUDES = (["validate"], ["fields"], ["validate", "fields", "app"], ["fields", "validate"])


def _fields_made_up() -> bytes:
    """A document whose fields say something: an empty table of contents, a reference to a
    bookmark held (named in another case), and references to missing bookmarks."""
    from lxml import etree

    from docx_agent import Document

    w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
    document = Document.new()
    part = document.package.document_part()
    body = document.package.tree(part).find(f"{{{w}}}body")

    def field(instruction: str, result: str) -> str:
        return (f'<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve">'
                f' {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>'
                f'<w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>')

    paragraphs = [
        field('TOC \\o "1-3" \\h', "No table of contents entries found."),
        '<w:bookmarkStart w:id="7" w:name="Here"/><w:r><w:t>held</w:t></w:r><w:bookmarkEnd w:id="7"/>',
        field("REF HERE \\h", "held") + field("PAGEREF bm:Missing \\h", "3"),
        field("NOTEREF Gone", "1") + field("PAGE", "1"),
    ]
    for inner in reversed(paragraphs):
        body.insert(0, etree.fromstring(f'<w:p xmlns:w="{w}">{inner}</w:p>'))
    document.package.mark_dirty(part)
    return document.to_bytes()


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def _seconds_out(answer: dict) -> dict:
    """``answer`` with each part's seconds in ``timing`` left out (their names kept)."""
    data = answer.get("data")
    if isinstance(data, dict) and isinstance(data.get("timing"), dict):
        data["timing"] = sorted(data["timing"])
    for key in ("duration_ms", "timing"):
        answer.pop(key, None)
    return answer


def _answer(toolbox, session, arguments: dict, monkeypatch, *, rust: bool) -> dict:
    with monkeypatch.context() as m:
        if not rust:
            m.setattr(reads, "serve_check", lambda call, arguments: None)
            m.setattr(rust_owner, "session_validation", lambda entry: None)
        reply = toolbox.dispatch(session, "check", arguments).to_json()
    return _seconds_out(json.loads(json.dumps(reply)))


def _owned(session, doc: str) -> bool:
    return reads._owned(session.entry(doc)) is not None


@pytest.mark.parametrize("path", [*FIXTURES, None], ids=lambda p: p.name if p else "fields-made-up")
def test_the_session_answers_a_check_of_the_document_alone_as_python_does(toolbox, path, monkeypatch):
    session = toolbox.session()
    doc = (session.open(path.read_bytes(), name=path.name) if path is not None else
           session.open(_fields_made_up(), name="fields.docx"))
    if not _owned(session, doc):
        pytest.skip("the session does not own this document")
    for edited in (False, True):
        if edited:
            paragraphs = session.entry(doc).document.paragraphs()
            if not paragraphs:
                break
            done = toolbox.dispatch(session, "word_insert_text", {"doc": doc, "target": paragraphs[0].id,
                                                                   "text": " checked"})
            assert done.ok, done.to_json()
        for include in INCLUDES:
            arguments = {"doc": doc, "include": include}
            served = reads.served().get("check", 0)
            rust = _answer(toolbox, session, arguments, monkeypatch, rust=True)
            assert reads.served().get("check", 0) == served + 1, reads.fallbacks().get("check")
            python = _answer(toolbox, session, arguments, monkeypatch, rust=False)
            assert rust == python, (doc, edited, include)


def test_the_made_up_fields_are_told(toolbox):
    session = toolbox.session()
    doc = session.open(_fields_made_up(), name="fields.docx")
    facts = toolbox.dispatch(session, "check", {"doc": doc, "include": ["fields"]}).data["fields"]
    assert facts["counts"] == {"TOC": 1, "REF": 1, "PAGEREF": 1, "NOTEREF": 1, "PAGE": 1}
    assert len(facts["empty_toc"]) == 1
    assert [m.split(": ", 1)[1] for m in facts["missing_bookmarks"]] == [
        "PAGEREF to a missing bookmark 'bm:Missing'", "NOTEREF to a missing bookmark 'Gone'"]


def test_a_check_that_needs_the_layout_is_pythons_counted(toolbox, monkeypatch):
    session = toolbox.session()
    doc = session.open(FIXTURES[0].read_bytes(), name=FIXTURES[0].name)
    if not _owned(session, doc):
        pytest.skip("the session does not own this document")
    before = dict(reads.fallbacks().get("check", {}))
    toolbox.dispatch(session, "check", {"doc": doc, "include": ["validate", "layout"]})
    after = reads.fallbacks().get("check", {})
    assert after.get("check: layout facts", 0) == before.get("check: layout facts", 0) + 1
