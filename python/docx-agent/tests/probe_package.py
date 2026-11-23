"""The made-up package the Word probes of tables measured (``tools/e3_probe.py`` once): a mode 15
document of the given body, Normal's styles and settings (``tools/make_fixtures.py``).  The
tests that hold the tools to Word's observations build their documents with it.

Must not: import pytest or docx-agent's library (a plain builder of package bytes).
"""

from __future__ import annotations

import io
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))

from make_fixtures import DECL, NS, REL, WML, content_types, relationships, settings, styles  # noqa: E402


def package(body: str, *, with_picture_rel: bool = False) -> bytes:
    section = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
               'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')
    parts = {
        "[Content_Types].xml": content_types({"word/document.xml": WML + ".document.main+xml",
                                              "word/styles.xml": WML + ".styles+xml",
                                              "word/settings.xml": WML + ".settings+xml"}),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([("rId1", REL + "styles", "styles.xml", False),
                                                       ("rId2", REL + "settings", "settings.xml", False)]),
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}{section}</w:body></w:document>",
        "word/styles.xml": styles(),
        "word/settings.xml": settings(15),
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, data in parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 10, 3, 12, 0, 0)), data)
    return buffer.getvalue()


#: The namespaces a drawing needs, beside ``make_fixtures.NS``.
DRAWING_NS = ('xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" '
              'xmlns:wp14="http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing" '
              'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
              'xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"')
#: An inline picture of 72 x 36 points, ``d:1`` (as Word inserts one), of the image ``rIdImg``.
PICTURE = (
    '<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0" wp14:anchorId="3B000002" '
    'wp14:editId="3B000003"><wp:extent cx="914400" cy="457200"/><wp:effectExtent l="0" t="0" r="0" b="0"/>'
    '<wp:docPr id="1" name="Picture 1" descr="Case 1"/><wp:cNvGraphicFramePr><a:graphicFrameLocks '
    'noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData '
    'uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" '
    'name="Picture 1" descr="Case 1"/><pic:cNvPicPr><a:picLocks noChangeAspect="1" noChangeArrowheads="1"/>'
    '</pic:cNvPicPr></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImg"/><a:stretch><a:fillRect/></a:stretch>'
    '</pic:blipFill><pic:spPr bwMode="auto"><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/>'
    '</a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>'
    '</wp:inline></w:drawing>')


def picture_document() -> bytes:
    """A paragraph holding :data:`PICTURE` between text, and a paragraph after (the drawings
    probe's first case, once ``tools/e5_probe.py``)."""
    from make_fixtures import png

    filler = "Text that runs beside and around the drawing so that its wrapping shows on the page. "
    body = ('<w:p w14:paraId="3A000001" w14:textId="77777777"><w:r><w:t>Drawings follow.</w:t></w:r></w:p>'
            '<w:p w14:paraId="3A000002" w14:textId="77777777"><w:r><w:t xml:space="preserve">Case 1: </w:t></w:r>'
            f'<w:r>{PICTURE}</w:r><w:r><w:t xml:space="preserve"> {filler * 3}</w:t></w:r></w:p>'
            '<w:p w14:paraId="3A000003" w14:textId="77777777"><w:r><w:t>The end.</w:t></w:r></w:p>')
    section = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
               'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')
    types = content_types({"word/document.xml": WML + ".document.main+xml", "word/styles.xml": WML + ".styles+xml",
                           "word/settings.xml": WML + ".settings+xml"})
    types = types.replace("<Default ", '<Default Extension="png" ContentType="image/png"/><Default ', 1)
    parts = {
        "[Content_Types].xml": types,
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([("rId1", REL + "styles", "styles.xml", False),
                                                       ("rId2", REL + "settings", "settings.xml", False),
                                                       ("rIdImg", REL + "image", "media/image1.png", False)]),
        "word/document.xml": f"{DECL}<w:document {NS} {DRAWING_NS}><w:body>{body}{section}</w:body></w:document>",
        "word/styles.xml": styles(),
        "word/settings.xml": settings(15),
        "word/media/image1.png": png(16, 8, (200, 60, 40)),
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, data in parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 10, 4, 12, 0, 0)), data)
    return buffer.getvalue()
