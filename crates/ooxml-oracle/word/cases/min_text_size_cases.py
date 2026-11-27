"""The minimum-text-size probe document (``probes/min_text_size_probe.py``; Word's observations in
``observations/min-text-size-word.json``; round 60, R140): a paragraph of every kind of text
docx-agent 0.1's per-kind minimum tells apart, each named by its first word and drawn at one size
throughout (its note reference marks too), so Word's font size of the paragraph is the size the
check reads.

Which kind a paragraph is rests on Word in two ways, and the document holds both:

* its story (body, header, footer, footnote, endnote): a header's and a footer's text, a note's;
* its style: a caption is a paragraph in the Caption style. Here the style is as a Dutch Word
  writes its built-in caption style, id ``Bijschrift`` with the name ``caption``, so 0.1's rule
  (the style's id, else its name, is ``caption``) is held to Word's idea of its Caption style.

A table's text is the body's paragraphs inside a cell (structure, not Word's say).

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

import io
import sys
import zipfile
from pathlib import Path

#: docx-agent's fixture makers (``python/docx-agent/tools``), four folders up and across.
sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "python" / "docx-agent" / "tools"))

from make_fixtures import DECL, NS, REL, WML, content_types, relationships, settings, styles  # noqa: E402

#: Each paragraph by key: its text, its size (points), its story and its style id ("" for none),
#: and whether it is in a table cell.
PARAGRAPHS = {
    "Alpha": ("Alpha body text at nine points.", 9, "body", "", False),
    "Bravo": ("Bravo body text at eleven points.", 11, "body", "", False),
    "Charlie": ("Charlie caption at nine points.", 9, "body", "Bijschrift", False),
    "Delta": ("Delta cell text at eight points.", 8, "body", "", True),
    "Echo": ("Echo caption in a cell at six and a half points.", 6.5, "body", "Bijschrift", True),
    "Foxtrot": ("Foxtrot carries a footnote and an endnote.", 11, "body", "", False),
    "Golf": ("Golf header at seven points.", 7, "header", "Header", False),
    "Hotel": ("Hotel footer at six points.", 6, "footer", "Footer", False),
    "India": ("India footnote at seven and a half points.", 7.5, "footnote", "FootnoteText", False),
    "Juliet": ("Juliet endnote at six points.", 6, "endnote", "EndnoteText", False),
}

_IDS = iter(range(1, 100))


def _run(text: str, half_points: int, extra: str = "") -> str:
    return (f'<w:r><w:rPr>{extra}<w:sz w:val="{half_points}"/><w:szCs w:val="{half_points}"/></w:rPr>'
            f'<w:t xml:space="preserve">{text}</w:t></w:r>')


def _paragraph(key: str, inner: str = "") -> str:
    text, points, _, style, _ = PARAGRAPHS[key]
    half = int(points * 2)
    # (the paragraph mark at the text's size too: Word's font size of the paragraph is one size)
    styled = f'<w:pStyle w:val="{style}"/>' if style else ""
    properties = f'<w:pPr>{styled}<w:rPr><w:sz w:val="{half}"/><w:szCs w:val="{half}"/></w:rPr></w:pPr>'
    return (f'<w:p w14:paraId="{0x5A000000 + next(_IDS):08X}" w14:textId="77777777">{properties}'
            f'{inner.format(half=half)}{_run(text, half)}</w:p>')


def _styles() -> str:
    caption = ('<w:style w:type="paragraph" w:styleId="Bijschrift"><w:name w:val="caption"/>'
               '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="35"/>'
               '<w:unhideWhenUsed/><w:qFormat/><w:pPr><w:spacing w:after="200" w:line="240" '
               'w:lineRule="auto"/></w:pPr><w:rPr><w:i/><w:iCs/><w:color w:val="44546A"/>'
               '<w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr></w:style>')
    footer = ('<w:style w:type="paragraph" w:styleId="Footer"><w:name w:val="footer"/>'
              '<w:basedOn w:val="Normal"/></w:style>')
    endnote = ('<w:style w:type="paragraph" w:styleId="EndnoteText"><w:name w:val="endnote text"/>'
               '<w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/>'
               '</w:pPr><w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>'
               '<w:style w:type="character" w:styleId="EndnoteReference"><w:name w:val="endnote reference"/>'
               '<w:basedOn w:val="DefaultParagraphFont"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr>'
               '</w:style>')
    return styles().replace("</w:styles>", caption + footer + endnote + "</w:styles>")


def document() -> bytes:
    """The probe document: Alpha to Foxtrot in the body (Delta and Echo in a table's cells), Golf
    in the header, Hotel in the footer, India a footnote of Foxtrot's and Juliet its endnote."""
    global _IDS
    _IDS = iter(range(1, 100))
    cells = "".join(f'<w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr>{_paragraph(key)}</w:tc>'
                    for key in ("Delta", "Echo"))
    table = ('<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
             '<w:tblGrid><w:gridCol w:w="4000"/><w:gridCol w:w="4000"/></w:tblGrid>'
             f'<w:tr w14:paraId="{0x5A000000 + next(_IDS):08X}" w14:textId="77777777">{cells}</w:tr></w:tbl>')
    notes = ('<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteReference w:id="1"/></w:r>'
             '<w:r><w:rPr><w:rStyle w:val="EndnoteReference"/></w:rPr><w:endnoteReference w:id="1"/></w:r>')
    foxtrot = _paragraph("Foxtrot").replace("</w:p>", notes + "</w:p>")
    section = ('<w:sectPr><w:headerReference w:type="default" r:id="rIdHeader"/>'
               '<w:footerReference w:type="default" r:id="rIdFooter"/>'
               '<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
               'w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')
    body = "".join(_paragraph(key) for key in ("Alpha", "Bravo", "Charlie")) + table + foxtrot + section

    def separators(kind: str) -> str:
        return (f'<w:{kind} w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:{kind}>'
                f'<w:{kind} w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/>'
                f'</w:r></w:p></w:{kind}>')

    def note(kind: str, key: str, reference: str, mark: str) -> str:
        own = (f'<w:r><w:rPr><w:rStyle w:val="{reference}"/><w:sz w:val="{{half}}"/><w:szCs w:val="{{half}}"/>'
               f'</w:rPr><w:{mark}/></w:r>')
        return f'<w:{kind} w:id="1">{_paragraph(key, own)}</w:{kind}>'

    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml", "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml", "word/header1.xml": WML + ".header+xml",
            "word/footer1.xml": WML + ".footer+xml", "word/footnotes.xml": WML + ".footnotes+xml",
            "word/endnotes.xml": WML + ".endnotes+xml"}),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rId1", REL + "styles", "styles.xml", False), ("rId2", REL + "settings", "settings.xml", False),
            ("rIdHeader", REL + "header", "header1.xml", False), ("rIdFooter", REL + "footer", "footer1.xml", False),
            ("rIdFoot", REL + "footnotes", "footnotes.xml", False),
            ("rIdEnd", REL + "endnotes", "endnotes.xml", False)]),
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>",
        "word/styles.xml": _styles(),
        "word/settings.xml": settings(15, notes=True).replace(
            "</w:footnotePr>", '</w:footnotePr><w:endnotePr><w:endnote w:id="-1"/><w:endnote w:id="0"/></w:endnotePr>'),
        "word/header1.xml": f"{DECL}<w:hdr {NS}>{_paragraph('Golf')}</w:hdr>",
        "word/footer1.xml": f"{DECL}<w:ftr {NS}>{_paragraph('Hotel')}</w:ftr>",
        "word/footnotes.xml": (f"{DECL}<w:footnotes {NS}>{separators('footnote')}"
                               f"{note('footnote', 'India', 'FootnoteReference', 'footnoteRef')}</w:footnotes>"),
        "word/endnotes.xml": (f"{DECL}<w:endnotes {NS}>{separators('endnote')}"
                              f"{note('endnote', 'Juliet', 'EndnoteReference', 'endnoteRef')}</w:endnotes>"),
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, data in parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 11, 27, 12, 0, 0)), data)
    return buffer.getvalue()
