"""Made-up documents shared by docx-agent's tests and the oracle's fact cases
(crates/ooxml-oracle/python/word_facts_cases.py): blank paragraphs used as space
(``empty_document``), list levels a style numbers (``make``, ``FULL``, ``SECTIONS_ONLY``)
and a table's edges as Accept All leaves them (``SHAPES``, ``_document``, ``_edge``,
``_p``, ``_tbl``).  A plain module: it imports no test framework, so the gates' runner
(which has none) can read it."""

from __future__ import annotations

import io
import zipfile

from lxml import etree

from docx_agent import Document

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W


def empty_document() -> tuple[Document, str, list[str]]:
    """A made-up document: an opening line, then two blank lines (one a space only) before
    the next, a table whose cell (1, 0) reads Title, a blank line, Body text, and a closing
    line; an empty paragraph also opens the document and ends the cell (1, 1)."""
    document = Document.new()
    document.insert_markdown("Opening line.\n\nNext line.\n\n| Item | Note |\n| --- | --- |\n"
                             "| x | y |\n\nClosing line.", at="end")
    opening = document._index(document.package.document_part()).paragraphs[0].id
    blanks = [document.paragraph(opening).insert_after(" ").id]
    blanks.insert(0, document.paragraph(opening).insert_after("").id)
    document.paragraph(opening).insert_before("")
    table = document.tables()[0]
    cell = table.cell(1, 0).paragraphs[0]
    cell.insert_after("Body text")
    blanks.append(cell.insert_after("").id)
    cell.set_text("Title")
    table.cell(1, 1).paragraphs[0].insert_after("")
    return document, table.id, blanks


NS = ('xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" '
      'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"')
_REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml."


def _numpr(level: int | None) -> str:
    return "<w:numPr>" + (f'<w:ilvl w:val="{level}"/>' if level else "") + '<w:numId w:val="1"/></w:numPr>'


def _style(style_id: str, name: str, ppr: str) -> str:
    return (f'<w:style w:type="paragraph" w:customStyle="1" w:styleId="{style_id}"><w:name w:val="{name}"/>'
            f'<w:basedOn w:val="Normal"/><w:qFormat/><w:pPr>{ppr}</w:pPr></w:style>')


def _lvl(level: int, text: str, indent: int, style: str | None) -> str:
    return (f'<w:lvl w:ilvl="{level}"><w:start w:val="1"/><w:numFmt w:val="decimal"/>'
            + (f'<w:pStyle w:val="{style}"/>' if style else "")
            + f'<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/>'
            f'<w:pPr><w:ind w:left="{indent}" w:hanging="{indent}"/></w:pPr></w:lvl>')


def _paragraph(text: str, style: str | None = None) -> str:
    return ("<w:p>" + (f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else "")
            + f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>')


#: The body: chapters and sections, an aside, body text.
FULL = [("One", "ChapterTitle"), ("One first", "SectionTitle"), ("One second", "SectionTitle"),
        ("Two", "ChapterTitle"), ("Two first", "SectionTitle"), ("Aside", "AsideTitle"), ("Body text", None)]
#: Sections and no chapter.
SECTIONS_ONLY = [("Lone first", "SectionTitle"), ("Lone second", "SectionTitle"), ("Body text", None)]


def make(body=FULL) -> bytes:
    styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles {NS}>'
              '<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="20"/></w:rPr></w:rPrDefault></w:docDefaults>'
              '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>'
              '<w:style w:type="character" w:customStyle="1" w:styleId="Marked"><w:name w:val="Marked"/></w:style>'
              + _style("ChapterTitle", "Chapter title", _numpr(None) + '<w:ind w:left="300" w:hanging="300"/>')
              + _style("SectionTitle", "Section title", _numpr(1) + '<w:ind w:left="900" w:hanging="300"/>')
              + _style("AsideTitle", "Aside title", '<w:ind w:left="1100" w:hanging="200"/>')
              + "</w:styles>")
    numbering = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering {NS}>'
                 '<w:abstractNum w:abstractNumId="0"><w:nsid w:val="5D1E2F30"/>'
                 '<w:multiLevelType w:val="multilevel"/>'
                 + _lvl(0, "%1", 432, "ChapterTitle") + _lvl(1, "%1.%2", 576, "SectionTitle")
                 + _lvl(2, "%1.%2.%3", 720, None)
                 + '</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>')
    document = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>'
                + "".join(_paragraph(text, style) for text, style in body)
                + '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1418" w:right="1418" '
                'w:bottom="1418" w:left="1418" w:header="709" w:footer="709" w:gutter="0"/></w:sectPr>'
                "</w:body></w:document>")
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            f'<Override PartName="/word/document.xml" ContentType="{_CT}document.main+xml"/>'
            f'<Override PartName="/word/styles.xml" ContentType="{_CT}styles+xml"/>'
            f'<Override PartName="/word/numbering.xml" ContentType="{_CT}numbering+xml"/></Types>'))
        archive.writestr("_rels/.rels", (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f'<Relationship Id="rId1" Type="{_REL}officeDocument" Target="word/document.xml"/></Relationships>'))
        archive.writestr("word/_rels/document.xml.rels", (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f'<Relationship Id="rId1" Type="{_REL}styles" Target="styles.xml"/>'
            f'<Relationship Id="rId2" Type="{_REL}numbering" Target="numbering.xml"/></Relationships>'))
        archive.writestr("word/document.xml", document)
        archive.writestr("word/styles.xml", styles)
        archive.writestr("word/numbering.xml", numbering)
    return out.getvalue()


def _p(text: str, deleted_mark: bool = False) -> str:
    mark = f'<w:pPr><w:rPr><w:del w:id="9{len(text)}" w:author="Editor B" w:date="2026-11-13T09:00:00Z"/>' \
           '</w:rPr></w:pPr>' if deleted_mark else ""
    return f"<w:p>{mark}<w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>"


def _tc(content: str) -> str:
    return f'<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>{content}</w:tc>'


def _tbl(rows: list[list[str]]) -> str:
    grid = "".join('<w:gridCol w:w="3000"/>' for _ in rows[0])
    return (f'<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>'
            + "".join("<w:tr>" + "".join(_tc(c) for c in row) + "</w:tr>" for row in rows) + "</w:tbl>")


def _edge(at: tuple[int, int], rows: int = 2, columns: int = 2) -> str:
    cells = [[_p(f"r{r + 1}c{c + 1}") for c in range(columns)] for r in range(rows)]
    cells[at[0]][at[1]] = _p("Kept.") + _p("Last.", deleted_mark=True)
    return _tbl(cells)


SHAPES = {
    "before a table": _p("Before. ", True) + _tbl([[_p("A1"), _p("B1")]]),
    "chain into a nested table": _p("One. ", True) + _p("Two. ", True)
    + _tbl([[_tbl([[_p("Inner"), _p("Side")]]) + _p("Under"), _p("B1")]]),
    "a first cell's end": _edge((0, 0)),
    "a later row's first cell": _edge((1, 0)),
    "a middle cell's end": _edge((0, 1), rows=1, columns=3),
    "a row's end": _edge((0, 1)),
    "the table's last cell": _edge((1, 1)),
}


def _document(body: str) -> bytes:
    document = Document.new()
    tree = document.package.tree(document.package.document_part())
    container = tree.find(_W + "body")
    section = container.find(_W + "sectPr")
    for child in list(container):
        if child is not section:
            container.remove(child)
    wrapped = etree.fromstring(f'<w:body xmlns:w="{W}">{body}{_p("End.")}</w:body>')
    for child in list(wrapped):
        section.addprevious(child)
    document.package.mark_dirty(document.package.document_part())
    return document.to_bytes()
