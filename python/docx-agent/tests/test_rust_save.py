"""``save_document`` served by the Rust session (lane LC1): the file the session writes for a
document as opened, after an edit, as a template, cleaned, and as Markdown -- its reply's size
the file's -- and the save gate, the host's on the served path too."""
from __future__ import annotations

import io
import zipfile

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
from conftest import FIXTURE_DIR

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
def test_the_saves_write_their_files(path):
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

