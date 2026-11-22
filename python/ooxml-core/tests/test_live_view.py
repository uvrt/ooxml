"""``Session.view`` and ``Session.page_svg``: the Word live view from Python (lane LV2).

The pages, their blocks and sizes, the pages an edit changed, one page's SVG tagged with the
version's ids after the document moved on; the errors as exceptions.
"""
from __future__ import annotations

import io
import json
import zipfile
from pathlib import Path

import ooxml_core
import pytest

_NS = ('xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" '
       'xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"')
_FONTS = Path(__file__).resolve().parents[2] / "pptx2svg/packages/pptx2svg-fonts/src/pptx2svg_fonts/files"


def _paragraph(pid: str, text: str, page_break: bool = False) -> str:
    before = "<w:pPr><w:pageBreakBefore/></w:pPr>" if page_break else ""
    return f'<w:p w14:paraId="{pid}" w14:textId="77777777">{before}<w:r><w:t>{text}</w:t></w:r></w:p>'


def _docx() -> bytes:
    body = _paragraph("11111111", "one") + _paragraph("22222222", "two", page_break=True)
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("_rels/.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/'
                         'relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/'
                         'officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
                         "</Relationships>")
        archive.writestr("word/document.xml", f"<w:document {_NS}><w:body>{body}</w:body></w:document>")
    return out.getvalue()


@pytest.fixture
def opened():
    session = ooxml_core.Session()
    doc = session.open(_docx(), name="a.docx")
    layout = ooxml_core.CallLayout(ooxml_core.WordPages(ooxml_core.WordFonts([("folder", str(_FONTS))])))
    return session, doc, layout


def test_a_view_then_the_pages_an_edit_changed(opened):
    session, doc, layout = opened
    first = session.view(doc, layout=layout)
    assert [(number, blocks) for number, blocks, _, _ in first["pages"]] == [(1, ("p:11111111",)),
                                                                            (2, ("p:22222222",))]
    assert first["changed"] == [1, 2] and not first["stale"] and first["since"] is None
    text, _, _ = session.call("word_set_text", json.dumps({"doc": doc, "target": "p:22222222", "text": "deux"}))
    assert json.loads(text)["ok"], text
    second = session.view(doc, first["token"], layout=layout)
    assert second["changed"] == [2] and second["token"] != first["token"]
    svg, warnings = session.page_svg(doc, 2, first["token"], layout=layout)
    assert 'data-docx-agent-id="p:22222222"' in svg and isinstance(warnings, list)


def test_the_errors(opened):
    session, doc, layout = opened
    token = session.view(doc, layout=layout)["token"]
    with pytest.raises(ooxml_core.UnknownToken):
        session.page_svg(doc, 1, "nope", layout=layout)
    with pytest.raises(LookupError):
        session.page_svg(doc, 1, "nope", layout=layout)
    with pytest.raises(IndexError):
        session.page_svg(doc, 3, token, layout=layout)
    with pytest.raises(ValueError):
        session.page_svg(doc, 1, token, layout=layout, text="paths")
    with pytest.raises(ooxml_core.ViewRefused) as refused:
        session.page_svg(doc, 1, token, layout=layout, text="outlines")
    assert refused.value.args[0] == "refused"
    with pytest.raises(ValueError):
        session.view(doc, layout=ooxml_core.CallLayout())
    stale = session.view(doc, "nope", layout=layout)
    assert stale["stale"] and stale["changed"] == [1, 2] and stale["note"]
