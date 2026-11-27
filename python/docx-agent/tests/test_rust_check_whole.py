"""``check`` answered whole by the Rust session (lane P5a): a check that reads the document
alone -- ``validate``, ``fields``, ``app`` -- is the session's ``check``
(``ooxml_tools_word::check``), and so is a check that needs the layout.
"""
from __future__ import annotations

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR
import word_docs

FIXTURES = sorted((FIXTURE_DIR).rglob("*.docx"))[::3]


def _fields_made_up() -> bytes:
    """A document whose fields say something: an empty table of contents, a reference to a
    bookmark held (named in another case), and references to missing bookmarks."""

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
    return word_docs.with_body("".join(f"<w:p>{inner}</w:p>" for inner in paragraphs) + "<w:p/>")


@pytest.fixture(scope="module")
def toolbox():
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def test_the_made_up_fields_are_told(toolbox):
    session = toolbox.session()
    doc = session.open(_fields_made_up(), name="fields.docx")
    facts = toolbox.dispatch(session, "check", {"doc": doc, "include": ["fields"]}).data["fields"]
    assert facts["counts"] == {"TOC": 1, "REF": 1, "PAGEREF": 1, "NOTEREF": 1, "PAGE": 1}
    assert len(facts["empty_toc"]) == 1
    assert [m.split(": ", 1)[1] for m in facts["missing_bookmarks"]] == [
        "PAGEREF to a missing bookmark 'bm:Missing'", "NOTEREF to a missing bookmark 'Gone'"]


def test_a_check_that_needs_the_layout_is_the_sessions(toolbox):
    # Lane P5l: the layout's facts, the host's.
    session = toolbox.session()
    doc = session.open(FIXTURES[0].read_bytes(), name=FIXTURES[0].name)
    reply = toolbox.dispatch(session, "check", {"doc": doc, "include": ["validate", "layout"]}).to_json()
    assert "error" not in reply, reply
    assert "layout" in reply["data"], reply
