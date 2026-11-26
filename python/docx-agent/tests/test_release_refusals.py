"""The release notes' refused list holds through the Toolbox (round 60, lane RN): each form
tools/release_refusals.py lists is a call docx-agent's Toolbox refuses as not supported yet in
that form (``details.not_supported``), on a made-up document.  A form the Toolbox serves is
not listed (READER says ``None`` of it), so the notes never send the team away from a call that
works.  Every document and value is made up.
"""

from __future__ import annotations

import fnmatch
import io
import re
import sys
import zipfile
from pathlib import Path

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS
import word_docs
from test_tables_from_data import CLOCK, ds_put

TOOLS_DIR = Path(__file__).resolve().parents[3] / "tools"
if not (TOOLS_DIR / "release_refusals.py").exists():
    pytest.skip("the repository's tools/ folder is not here", allow_module_level=True)
sys.path.insert(0, str(TOOLS_DIR))
import release_refusals  # noqa: E402

DELETED_MARK = '<w:pPr><w:rPr><w:del w:id="91" w:author="Writer A" w:date="2026-01-01T00:00:00Z"/></w:rPr></w:pPr>'
#: A table whose first cell's paragraph mark is deleted while its row stays, then a paragraph.
CELL_ENDS = ("<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w='2000'/><w:gridCol w:w='2000'/></w:tblGrid><w:tr>"
             f"<w:tc><w:p>{DELETED_MARK}<w:r><w:t>a</w:t></w:r></w:p></w:tc>"
             "<w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl>" + word_docs.paragraph("after"))
#: A VML shape stating no size, which the page layout cannot lay out.
SHAPE = ('<w:pict><v:rect xmlns:v="urn:schemas-microsoft-com:vml" style="position:absolute" '
         'fillcolor="#4472c4"/></w:pict>')
SVG = b'<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>'
MARKDOWN = "| check | rating |\n|---|---|\n| Alpha | 2 |\n| Beta | 4 |\n\n# Head\n\nText one.\n\nText two.\n"


@pytest.fixture(scope="module")
def box():
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


class Doc:
    """A session with one made-up document, ``d1``, and the ids its ``word_read`` names."""

    def __init__(self, box, data: bytes | None = None, markdown: str | None = MARKDOWN) -> None:
        self.box = box
        self.session = box.session(clock=CLOCK)
        self.session.open(data or word_docs.blank(), "report.docx")
        if markdown:
            self.ok("word_insert_markdown", at="end", markdown=markdown)
        self.ok("ds_put", id="scores")
        text = self.ok("word_read", view="markup").data
        self.tables = re.findall(r"t:[0-9A-F]{8}", text)
        self.paragraphs = re.findall(r"p:[0-9A-F]{8}", text)

    def call(self, tool: str, **arguments):
        if tool != "ds_put":
            arguments = {"doc": "d1", **arguments}
        return self.box.dispatch(self.session, tool, arguments)

    def ok(self, tool: str, **arguments):
        result = self.call(tool, **arguments)
        assert result.ok, result.to_json()
        return result


def _note_shape() -> bytes:
    """A document whose footnote holds a shape the layout cannot lay out."""
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        doc = Doc(toolbox, markdown="Body text.\n")
        doc.ok("word_notes", action="insert", kind="footnote", at=f"{doc.paragraphs[0]}@4", text="note")
        data = word_docs.saved(doc.session, "d1")
    return word_docs.replaced(data, {"word/footnotes.xml": lambda xml: xml.replace(
        "<w:t>note</w:t>", f"<w:t>note</w:t></w:r><w:r>{SHAPE}")})


#: A frame (``w:framePr``, not a drop cap) as a header's paragraph declares it.
FRAME = ("<w:framePr w:w='2000' w:h='400' w:hAnchor='page' w:vAnchor='page' w:x='500' w:y='500'/>")
#: A drop cap in a table cell, which the layout lays out only among the body's top-level paragraphs.
DROP_CAP_IN_A_CELL = ("<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w='4000'/></w:tblGrid><w:tr><w:tc><w:p><w:pPr>"
                      "<w:framePr w:dropCap='drop' w:lines='3' w:wrap='around' w:vAnchor='text' w:hAnchor='text'/>"
                      "</w:pPr><w:r><w:t>D</w:t></w:r></w:p><w:p><w:r><w:t>rop cap in a cell.</w:t></w:r></w:p></w:tc>"
                      "</w:tr></w:tbl>" + word_docs.paragraph("after"))


def _header_frame() -> bytes:
    """A document whose header's paragraph is a frame the layout cannot lay out."""
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        doc = Doc(toolbox, markdown="Body text.\n")
        doc.ok("word_headers_footers", section="s:1", which="header", text="Head", action="set")
        data = word_docs.saved(doc.session, "d1")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        header = next(name for name in archive.namelist() if name.startswith("word/header"))
    return word_docs.replaced(data, {header: lambda xml: re.sub(
        r"<w:p( [^>]*)?>(<w:pPr>)?", lambda m: f"<w:p{m.group(1) or ''}><w:pPr>{FRAME}" + ("" if m.group(2) else "</w:pPr>"),
        xml, count=1)})


def _boxed_shape() -> bytes:
    """A document whose text box holds a shape text wraps around, which the layout cannot lay out
    in a text box."""
    with Toolbox([*TOOLS, ds_put], formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        doc = Doc(toolbox, markdown="Body text.\n\nMore.\n")
        doc.ok("word_drawings", action="insert_text_box", at=f"{doc.paragraphs[0]}@0", text="inside")
        doc.ok("word_drawings", action="insert_shape", at=f"{doc.paragraphs[1]}@0", preset="rect", width=40,
               height=40, wrap="square")
        data = word_docs.saved(doc.session, "d1")

    def put(xml: str) -> str:
        shape = re.findall(r'<mc:Choice Requires="wps">(<w:drawing>.*?</w:drawing>)</mc:Choice>', xml)[1]
        return xml.replace("<w:r><w:t>inside</w:t></w:r>", f"<w:r>{shape}</w:r><w:r><w:t>inside</w:t></w:r>", 1)

    return word_docs.replaced(data, {"word/document.xml": put})


def _no_settings() -> bytes:
    return word_docs.replaced(word_docs.blank(), {"word/_rels/document.xml.rels": lambda xml: re.sub(
        r'<Relationship [^>]*settings\.xml"/>', "", xml)})


#: Calls refused in each listed form: (the document's body or bytes, or None for MARKDOWN's;
#: "shape" for MARKDOWN's with a shape, "svg" for MARKDOWN's with an SVG blob (``{blob}``), or a function making the bytes; the tool; its arguments,
#: ``{t}``, ``{p}`` and ``{h}`` the document's table, a paragraph and the heading's paraId).
CASES = {
    "word_edit_table *": [
        (None, "word_edit_table", {"table": "{t}", "action": "sync", "from": "d:scores/Alpha/rating",
                                   "key": "check"}),
        (CELL_ENDS, "word_edit_table", {"table": "t@body/0", "action": "insert_row", "index": 1}),
    ],
    "check: *": [(_note_shape, "check", {"include": ["layout"]})],
    "* the layout cannot lay out": [
        (_note_shape, "render", {}),
        (_note_shape, "word_inspect", {"target": "body", "layout": True}),
        (_header_frame, "render", {}),
        (_header_frame, "check", {"include": ["reflow"]}),
        (DROP_CAP_IN_A_CELL, "render", {}),
        (_boxed_shape, "word_inspect", {"target": "body", "layout": True}),
        (_header_frame, "check", {"include": ["numbers"], "pages": [1]}),
    ],
    "word_read final view of Accept All's cell ends": [(CELL_ENDS, "word_read", {"view": "final"})],
    "word_inspect of Accept All's cell ends": [(CELL_ENDS, "word_inspect", {"target": "p@body/1"})],
    "replace_text *": [
        (None, "replace_text", {"find": "", "replace": "x"}),
        (None, "replace_text", {"find": "one", "replace": "**x**"}),
    ],
    "word_delete a *": [
        (None, "word_delete", {"target": "h:{h}"}),
        ("shape", "word_delete", {"target": "d:1"}),
    ],
    "word_insert_markdown *": [
        (None, "word_insert_markdown", {"at": "end", "markdown": "x", "like": ["{p}"]}),
        (None, "word_insert_markdown", {"at": "end", "from": "d:scores"}),
    ],
    "a picture in Markdown": [(None, "word_insert_markdown", {"at": "end", "markdown": "![a](b.png)"})],
    "a footnote in Markdown": [(None, "word_insert_markdown", {"at": "end", "markdown": "a[^1]\n\n[^1]: n\n"})],
    "word_drawings *": [
        (None, "word_drawings", {"action": "insert_text_box", "at": "{p}@0", "text": "t", "ref": "box"}),
        ("svg", "word_drawings", {"action": "insert_picture", "at": "{p}@0", "image": "{blob}",
                                  "alt_text": "a square"}),
    ],
    # (refused before the target is looked for: the document needs no chart)
    "edit_chart*": [(None, "edit_chart", {"target": "d:1", "action": "set_values", "from": "d:scores"})],
    "a document without a settings part": [
        (_no_settings, "word_headers_footers", {"section": "s:1", "which": "header", "type": "even",
                                                "text": "x", "action": "set"})],
    "the section of *: a section given by what a block holds": [
        (None, "word_headers_footers", {"section": "bm:x", "which": "header", "text": "x", "action": "set"})],
}


def _document(box, given) -> Doc:
    if given is None:
        return Doc(box)
    if given == "svg":
        doc = Doc(box)
        doc.blob = doc.session.add_blob(SVG, "picture.svg")
        return doc
    if given == "shape":
        doc = Doc(box)
        doc.ok("word_drawings", action="insert_shape", at=f"{doc.paragraphs[1]}@0", preset="rect", width=40,
               height=40)
        return doc
    if callable(given):
        return Doc(box, given(), markdown=None)
    return Doc(box, word_docs.with_body(given), markdown=None)


def _filled(value, doc: Doc):
    if isinstance(value, str):
        names = {"t": (doc.tables or [""])[0], "p": (doc.paragraphs or [""])[min(1, len(doc.paragraphs) - 1)],
                 "h": (doc.paragraphs or [""])[0][2:], "blob": getattr(doc, "blob", "")}
        return re.sub(r"\{(t|p|h|blob)\}", lambda m: names[m.group(1)], value)
    if isinstance(value, list):
        return [_filled(item, doc) for item in value]
    if isinstance(value, dict):
        return {key: _filled(item, doc) for key, item in value.items()}
    return value


def test_every_listed_form_has_a_case():
    listed = {form for form, text in release_refusals.READER.items() if text is not None}
    assert listed == set(CASES)


@pytest.mark.parametrize(("form", "number"), [(form, k) for form, cases in CASES.items() for k in range(len(cases))])
def test_a_listed_form_is_refused_through_the_toolbox(box, form, number):
    given, tool, arguments = CASES[form][number]
    doc = _document(box, given)
    result = doc.call(tool, **_filled(arguments, doc))
    error = result.to_json().get("error") or {}
    refused = (error.get("details") or {}).get("not_supported")
    assert refused is not None and fnmatch.fnmatchcase(refused, form), result.to_json()
    assert error.get("try_call") or error.get("valid_options"), result.to_json()  # the way out


LAID = CASES["* the layout cannot lay out"]


@pytest.mark.parametrize("number", range(len(LAID)))
def test_a_document_the_layout_cannot_lay_out_is_refused_with_a_way_out_that_works(box, number):
    """No layout falls back (lane fu-docx2svg): what the layout cannot lay out yet is refused as
    not supported yet, naming what the document holds, and the call it offers instead works."""
    given, tool, arguments = LAID[number]
    doc = _document(box, given)
    error = doc.call(tool, **_filled(arguments, doc)).to_json()["error"]
    assert error["details"]["not_supported"].endswith(" the layout cannot lay out"), error
    way_out = error["try_call"]
    assert doc.call(way_out["tool"], **{k: v for k, v in way_out["arguments"].items() if k != "doc"}).ok
