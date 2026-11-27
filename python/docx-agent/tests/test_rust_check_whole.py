"""``check`` answered whole by the Rust session (lane P5a): a check that reads the document
alone -- ``validate``, ``fields``, ``app`` -- is the session's ``check``
(``ooxml_tools_word::check``), and so is a check that needs the layout.
"""
from __future__ import annotations

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Version"):
    pytest.skip("ooxml_core has no Version", allow_module_level=True)

from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent.rust import reads  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from conftest import FIXTURE_DIR  # noqa: E402
import word_docs  # noqa: E402

# The Python host's own serving of a Word document (its adapters, owner and mirror), which Phase 3 E2
# deletes with these tests; their calls' answers are the host goldens' (replayed by the Rust host).
pytestmark = pytest.mark.python_host

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
    # Lane P5l serves the layout's facts: a check asking for them is the session's, counted.
    session = toolbox.session()
    doc = session.open(FIXTURES[0].read_bytes(), name=FIXTURES[0].name)
    served = reads.served().get("check", 0)
    reply = toolbox.dispatch(session, "check", {"doc": doc, "include": ["validate", "layout"]}).to_json()
    assert "error" not in reply, reply
    assert "layout" in reply["data"], reply
    assert reads.served().get("check", 0) == served + 1
