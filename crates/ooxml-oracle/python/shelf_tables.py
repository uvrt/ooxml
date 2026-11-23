"""Made-up "Shelf" tables, shared by docx-agent's ``test_rows_r58`` and the reads oracle's facts
cases (``word_facts_cases.py``), which run without pytest.

Each is a one-part document written as ``crates/ooxml-oracle/word/probes/e3_probe.py`` writes its probes
(``_package``: its styles, settings and section, byte for byte), here so that the oracle needs
nothing of docx-agent's tools (the gates' reference is frozen: reference_packages.py).
"""

from __future__ import annotations

import io
import zipfile

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
W14 = "http://schemas.microsoft.com/office/word/2010/wordml"
MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
NS = f'xmlns:w="{W}" xmlns:r="{R}" xmlns:w14="{W14}" xmlns:mc="{MC}" mc:Ignorable="w14"'
DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
WML = "application/vnd.openxmlformats-officedocument.wordprocessingml"
SECTION = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
           'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')


def _style(kind: str, sid: str, name: str, extra: str = "", based: str | None = "Normal",
           after: str | None = None) -> str:
    basis = f'<w:basedOn w:val="{based}"/>' if based else ""
    following = f'<w:next w:val="{after}"/>' if after else ""
    return (f'<w:style w:type="{kind}" w:styleId="{sid}"><w:name w:val="{name}"/>{basis}'
            f'{following}<w:qFormat/>{extra}</w:style>')


def _styles() -> str:
    return (
        f'{DECL}<w:styles xmlns:w="{W}">'
        '<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" '
        'w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/>'
        '<w:lang w:val="en-GB" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>'
        '<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr>'
        '</w:pPrDefault></w:docDefaults>'
        '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/>'
        '<w:qFormat/></w:style>'
        '<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont">'
        '<w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/>'
        '<w:unhideWhenUsed/></w:style>'
        '<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/>'
        '<w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/>'
        '<w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
        '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar>'
        '</w:tblPr></w:style>'
        + _style("paragraph", "Heading1", "heading 1",
                 '<w:pPr><w:keepNext/><w:spacing w:before="240" w:after="0"/>'
                 '<w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr>',
                 after="Normal")
        + _style("paragraph", "Header", "header")
        + _style("paragraph", "FootnoteText", "footnote text",
                 '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
                 '<w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr>')
        + _style("character", "FootnoteReference", "footnote reference",
                 '<w:rPr><w:vertAlign w:val="superscript"/></w:rPr>', based="DefaultParagraphFont")
        + _style("character", "Hyperlink", "Hyperlink",
                 '<w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr>', based="DefaultParagraphFont")
        + '<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/>'
        '<w:basedOn w:val="TableNormal"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
        '<w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr></w:style>'
        "</w:styles>"
    )


def _settings() -> str:
    return (f'{DECL}<w:settings xmlns:w="{W}"><w:defaultTabStop w:val="708"/>'
            '<w:compat><w:compatSetting w:name="compatibilityMode" '
            'w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>'
            "</w:settings>")


def _package(body: str) -> bytes:
    """A document of ``body`` (one section, A4), its parts dated as e3_probe dates them."""
    types = "".join(f'<Override PartName="/word/{part}.xml" ContentType="{WML}.{kind}+xml"/>'
                    for part, kind in (("document", "document.main"), ("styles", "styles"), ("settings", "settings")))
    parts = {
        "[Content_Types].xml": f'{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                               '<Default Extension="rels" '
                               'ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                               f'<Default Extension="xml" ContentType="application/xml"/>{types}</Types>',
        "_rels/.rels": _relationships([("rId1", "officeDocument", "word/document.xml")]),
        "word/_rels/document.xml.rels": _relationships([("rId1", "styles", "styles.xml"),
                                                        ("rId2", "settings", "settings.xml")]),
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}{SECTION}</w:body></w:document>",
        "word/styles.xml": _styles(),
        "word/settings.xml": _settings(),
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, data in parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 10, 3, 12, 0, 0)), data)
    return buffer.getvalue()


def _relationships(rels: list[tuple[str, str, str]]) -> str:
    items = "".join(f'<Relationship Id="{rid}" Type="{REL}{kind}" Target="{target}"/>' for rid, kind, target in rels)
    return (f'{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f"{items}</Relationships>")


RUN = ('<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/>{bold}<w:color w:val="{color}"/>'
       '<w:sz w:val="16"/><w:szCs w:val="16"/>')


def _cell(text: str | None, column: int, *, header: bool = False, color: str = "1F4E79",
          fill: str = "DEEAF6") -> str:
    bold = "<w:b/>" if column == 2 or header else ""
    centre = '<w:jc w:val="center"/>' if column == 2 else ""
    font = RUN.format(bold=bold, color=color)
    if header:
        font = font.replace("Arial", "Georgia")
    runs = "" if text is None else f'<w:r><w:rPr>{font}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r>'
    return (f'<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/><w:shd w:val="clear" w:color="auto" '
            f'w:fill="{fill}"/><w:vAlign w:val="center"/></w:tcPr>'
            f'<w:p><w:pPr><w:spacing w:after="0"/>{centre}</w:pPr>{runs}</w:p></w:tc>')


def _shelf(rows: list, *, header: bool = True, colors: list | None = None, extra: str = "") -> bytes:
    """A made-up shelf table: a header row (Georgia, bold) unless ``header`` is false, then
    ``rows`` (a row of ``None`` is empty cells), each run Arial 8 pt in its row's colour,
    the marks bare -- as many writers leave a table's cells."""
    out = []
    if header:
        out.append("<w:tr>" + "".join(_cell(v, j, header=True) for j, v in enumerate(("Item", "Room", "Count")))
                   + "</w:tr>")
    for i, values in enumerate(rows):
        color = colors[i] if colors else "1F4E79"
        values = values or (None, None, None)
        out.append("<w:tr>" + "".join(_cell(v, j, color=color) for j, v in enumerate(values)) + "</w:tr>")
    grid = "<w:tblGrid>" + '<w:gridCol w:w="2400"/>' * 3 + "</w:tblGrid>"
    table = f'<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>{grid}{"".join(out)}</w:tbl>'
    return _package(f'<w:p><w:r><w:t>Shelf.</w:t></w:r></w:p>{table}{extra}<w:p><w:r><w:t>End.</w:t></w:r></w:p>')


ROWS = [("Lamp", "hall", "3"), ("Kettle", "kitchen", "1"), ("Stool", "attic", "2"), ("Chair", "porch", "4")]
