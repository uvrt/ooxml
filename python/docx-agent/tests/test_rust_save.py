"""``save_document`` served by the Rust session (lane LC1): the file the session writes for a
document as opened, after an edit, as a template, cleaned, and as Markdown -- its reply's size
the file's -- and the save gate, the host's on the served path too."""
from __future__ import annotations

import io
import zipfile

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
if not hasattr(ooxml_core, "Session"):          # an ooxml-core before 0.9.0
    pytest.skip("ooxml_core has no Session", allow_module_level=True)

from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
from conftest import FIXTURE_DIR  # noqa: E402

#: A few fixtures, and a template saved as a .docx (its main part retyped on a .dotx save).
FIXTURES = sorted({*sorted((FIXTURE_DIR).rglob("*.docx"))[:6],
                   *(FIXTURE_DIR).rglob("template-saved-as-docx.docx")})
TEMPLATE = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"


CALLS = [
    ("save_document", {"name": "same.docx", "format": "docx"}),
    ("save_document", {"name": "same.dotx", "format": "dotx"}),
    ("save_document", {"name": "same.md", "format": "markdown"}),
    ("word_insert_markdown", {"at": "end", "markdown": "One more paragraph of words."}),
    ("save_document", {"name": "edited.docx", "format": "docx"}),
    ("save_document", {"name": "edited.docx", "format": "docx", "clean": True}),
]


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.stem)
def test_the_served_saves_write_their_files(path):
    from docx_agent.rust import reads

    served = reads.served().get("save_document", 0)
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session()
        d = session.open(path.read_bytes(), "old.docx")
        for tool, arguments in CALLS:
            result = box.dispatch(session, tool, {"doc": d, **arguments})
            assert result.ok, (tool, arguments, result.error)
            if tool != "save_document":
                continue
            written = session.take_outputs()[-1].data
            if arguments["format"] == "markdown":
                assert written.decode("utf-8").strip(), arguments
                continue
            size = result.data["cleaned"]["bytes"]["after"] if "cleaned" in result.data else result.data["size"]
            assert size == len(written), arguments
            with zipfile.ZipFile(io.BytesIO(written)) as package:
                types = package.read("[Content_Types].xml").decode("utf-8")
            assert (TEMPLATE in types) == (arguments["format"] == "dotx"), arguments
    saves = sum(tool == "save_document" for tool, _ in CALLS)
    assert reads.served().get("save_document", 0) - served == saves


def test_a_served_save_still_refuses_a_new_validation_problem():
    """The save gate is the host's on the served path too: the session writes the file, and
    ``call.output`` refuses it when the version has a problem the document did not open
    with -- compared by the session, which holds the version saved -- and hands the
    application nothing."""
    import dataclasses

    asked: list = []

    def problem(entry):
        asked.append(entry.version)
        return {"new": ["word/document.xml: bad"], "fixed": [], "baseline": 0}

    gated = dataclasses.replace(FORMAT, validation=problem)
    with Toolbox(TOOLS, formats=[gated], groups=GROUPS, workers=0) as box:
        session = box.session()
        d = session.open(FIXTURES[0].read_bytes(), "old.docx")
        result = box.dispatch(session, "save_document", {"doc": d, "name": "a.docx", "format": "docx"})
        assert not result.ok and result.error.code == "refused", result
        assert result.error.valid_options == ["word/document.xml: bad"]
        assert session.take_outputs() == []
    assert asked, "the gate did not compare the version"
