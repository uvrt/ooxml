"""Round 58's table-regions probe document (``probes/table_regions_probe.py``; Word's
observations in ``observations/table-regions-word.json``): a table style per case, each
region with its own fill, text colour and size, and the tables that use them.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

import io
import zipfile

from probe_package import DECL, NS, REL, WML, content_types, relationships, settings, styles

REGIONS = ["wholeTable", "band1Vert", "band2Vert", "band1Horz", "band2Horz", "firstCol", "lastCol",
           "firstRow", "lastRow", "neCell", "nwCell", "seCell", "swCell"]


#: One fill and one text colour per region (and ``own``: the style's own properties).
FILLS = {"own": "F2F2F2", "wholeTable": "D9D9D9", "band1Vert": "FFE699", "band2Vert": "C6E0B4",
         "band1Horz": "BDD7EE", "band2Horz": "F8CBAD", "firstCol": "FF0000", "lastCol": "00B050",
         "firstRow": "0070C0", "lastRow": "7030A0", "neCell": "FFC000", "nwCell": "000000",
         "seCell": "808000", "swCell": "008080"}


COLOURS = {"own": "404040", "wholeTable": "111111", "band1Vert": "AA0000", "band2Vert": "00AA00",
           "band1Horz": "0000AA", "band2Horz": "AA00AA", "firstCol": "00AAAA", "lastCol": "AAAA00",
           "firstRow": "FF6600", "lastRow": "6600FF", "neCell": "0099FF", "nwCell": "FF0099",
           "seCell": "99FF00", "swCell": "663300"}


#: Half points.
SIZES = {"own": 16, "wholeTable": 17, "band1Vert": 18, "band2Vert": 19, "band1Horz": 20, "band2Horz": 21,
         "firstCol": 22, "lastCol": 23, "firstRow": 24, "lastRow": 25, "neCell": 26, "nwCell": 27,
         "seCell": 28, "swCell": 29}


ALL_ON = dict(firstRow=1, lastRow=1, firstColumn=1, lastColumn=1, noHBand=0, noVBand=0)


def _part(region: str, ppr: str = "", rpr: str = "", tcpr: str = "") -> str:
    inner = (f"<w:pPr>{ppr}</w:pPr>" if ppr else "") + (f"<w:rPr>{rpr}</w:rPr>" if rpr else "") \
        + (f"<w:tcPr>{tcpr}</w:tcPr>" if tcpr else "")
    return f'<w:tblStylePr w:type="{region}">{inner}</w:tblStylePr>'


def _shd(fill: str) -> str:
    return f'<w:shd w:val="clear" w:color="auto" w:fill="{fill}"/>'


def _style(sid: str, *, ppr: str = "", rpr: str = "", tblpr: str = "", tcpr: str = "", parts: str = "",
           bands: tuple[int, int] = (1, 1)) -> str:
    band = (f'<w:tblStyleRowBandSize w:val="{bands[0]}"/><w:tblStyleColBandSize w:val="{bands[1]}"/>'
            if bands else "")
    return (f'<w:style w:type="table" w:customStyle="1" w:styleId="{sid}"><w:name w:val="Regions {sid}"/>'
            '<w:basedOn w:val="TableNormal"/><w:uiPriority w:val="99"/>'
            f'<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/>{ppr}</w:pPr>'
            + (f"<w:rPr>{rpr}</w:rPr>" if rpr else "")
            + f'<w:tblPr>{band}{tblpr}</w:tblPr>'
            + (f"<w:tcPr>{tcpr}</w:tcPr>" if tcpr else "") + parts + "</w:style>")


def _styles() -> str:
    out = []
    out.append(_style("S", tcpr=_shd(FILLS["own"]),
                      parts="".join(_part(r, tcpr=_shd(FILLS[r])) for r in REGIONS)))
    out.append(_style("C", rpr=f'<w:color w:val="{COLOURS["own"]}"/><w:sz w:val="{SIZES["own"]}"/>',
                      parts="".join(_part(r, rpr=f'<w:color w:val="{COLOURS[r]}"/><w:sz w:val="{SIZES[r]}"/>')
                                    for r in REGIONS)))
    out.append(_style("B", parts="".join(_part(r, rpr="<w:b/><w:bCs/>")
                                         for r in ("firstRow", "firstCol", "band1Horz", "band1Vert"))))
    out.append(_style("K", parts=_part("firstRow", rpr="<w:b/>") + _part("firstCol", rpr="<w:b/>")
                      + _part("nwCell", rpr='<w:b w:val="0"/>') + _part("lastRow", rpr="<w:i/>")
                      + _part("seCell", rpr="<w:b/>")))
    out.append(_style("H", parts="".join(_part(r, tcpr=_shd(FILLS[r])) for r in ("firstRow", "band1Horz",
                                                                                   "band2Horz"))))
    out.append(_style("N", bands=(2, 2),
                      parts="".join(_part(r, tcpr=_shd(FILLS[r])) for r in ("band1Horz", "band2Horz"))
                      + "".join(_part(r, rpr=f'<w:color w:val="{COLOURS[r]}"/>') for r in ("band1Vert", "band2Vert"))
                      + _part("firstRow", rpr="<w:b/>") + _part("firstCol", rpr="<w:i/>")))
    out.append(_style("O", parts="".join(_part(r, tcpr=_shd(FILLS[r])) for r in ("band1Horz", "band2Horz",
                                                                                   "firstRow"))
                      + "".join(_part(r, rpr=f'<w:color w:val="{COLOURS[r]}"/>')
                                for r in ("band1Vert", "band2Vert", "firstCol"))))
    out.append(_style("M", parts="".join(_part(r, rpr=f'<w:color w:val="{COLOURS[r]}"/>')
                                         for r in ("band1Vert", "band2Vert", "firstCol", "lastCol"))))
    out.append(_style("P", ppr='<w:jc w:val="left"/>',
                      parts=_part("band1Horz", ppr='<w:jc w:val="right"/>')
                      + _part("firstCol", ppr='<w:jc w:val="center"/>')
                      + _part("firstRow", ppr='<w:jc w:val="right"/>')
                      + _part("nwCell", ppr='<w:jc w:val="center"/>')
                      + _part("band1Vert", ppr='<w:jc w:val="center"/>')))
    single = '<w:{side} w:val="single" w:sz="4" w:space="0" w:color="000000"/>'
    own = "<w:tblBorders>" + "".join(single.format(side=s) for s in ("top", "left", "bottom", "right",
                                                                       "insideH", "insideV")) + "</w:tblBorders>"
    out.append(_style("D", tblpr=own, parts=(
        _part("firstRow", tcpr='<w:tcBorders><w:bottom w:val="single" w:sz="24" w:space="0" w:color="C00000"/>'
                               "</w:tcBorders>")
        + _part("firstCol", tcpr='<w:tcBorders><w:right w:val="single" w:sz="18" w:space="0" w:color="0070C0"/>'
                                 "</w:tcBorders>")
        + _part("band1Horz", tcpr='<w:tcBorders><w:top w:val="single" w:sz="12" w:space="0" w:color="00B050"/>'
                                  '<w:bottom w:val="single" w:sz="12" w:space="0" w:color="00B050"/>'
                                  '<w:insideV w:val="nil"/></w:tcBorders>')
        + _part("lastRow", tcpr='<w:tcBorders><w:top w:val="double" w:sz="4" w:space="0" w:color="7030A0"/>'
                                "</w:tcBorders>")
        + _part("lastCol", tcpr='<w:tcBorders><w:left w:val="nil"/></w:tcBorders>'))))
    out.append(_style("R", parts="".join(
        _part(r, rpr=f'<w:color w:val="{COLOURS[r]}"/>', tcpr=_shd(FILLS[r]))
        for r in ("firstRow", "lastRow", "firstCol", "lastCol"))))
    out.append(_style("W", rpr=f'<w:color w:val="{COLOURS["own"]}"/>', tcpr=_shd(FILLS["own"]),
                      parts=_part("wholeTable", rpr=f'<w:color w:val="{COLOURS["wholeTable"]}"/>',
                                  tcpr=_shd(FILLS["wholeTable"]))))
    out.append(_style("V", parts=_part("wholeTable", rpr=f'<w:color w:val="{COLOURS["wholeTable"]}"/>',
                                       tcpr=_shd(FILLS["wholeTable"]))))
    out.append(_style("G", parts="".join(_part(r, tcpr=_shd(FILLS[r])) for r in ("firstRow", "band1Horz",
                                                                                   "band2Horz"))))
    def sides(**given: str) -> str:
        order = ("top", "left", "bottom", "right", "insideH", "insideV")
        return "<w:tcBorders>" + "".join(
            f'<w:{side} w:val="single" w:sz="12" w:space="0" w:color="{given[side]}"/>'
            for side in order if side in given) + "</w:tcBorders>"

    # E: a row region's sides; F: a column region's; Q: a two-row band's.
    out.append(_style("E", parts=_part("band1Horz", tcpr=sides(left="FF0000", right="0000FF", insideV="00B050"))
                      + _part("firstRow", tcpr=sides(insideV="FFC000", bottom="7030A0"))))
    out.append(_style("F", parts=_part("firstCol", tcpr=sides(top="FF0000", bottom="0000FF", insideH="00B050"))
                      + _part("band1Vert", tcpr=sides(top="7030A0", insideH="FFC000", left="008080"))))
    out.append(_style("Q", bands=(2, 1), parts=_part("band1Horz", tcpr=sides(top="0000FF", bottom="00B050",
                                                                              insideH="FF0000"))))
    out.append(_style("A"))
    return "".join(out)


#: Cell fills (direct) under automatic text colour, case ``A``: rows of four.
AUTO_FILLS = ["000000", "111111", "222222", "333333", "444444", "555555", "666666", "777777",
              "000080", "0000FF", "002060", "1F3864", "203864", "2F5496", "305496", "0070C0",
              "800000", "C00000", "FF0000", "006100", "008000", "00B050", "375623", "385723",
              "7030A0", "404040", "595959", "3A3A3A", "4D4D4D", "262626", "0D0D0D", "1A1A1A",
              "3B3B3B", "3C3C3C", "3D3D3D", "3E3E3E", "3F3F3F", "414141", "424242", "434343"]


#: (case, rows, columns, look, header rows, merged rows {row: [spans]}).
CASES = [
    ("S", 5, 5, ALL_ON, 0, {}),
    ("C", 5, 5, ALL_ON, 0, {}),
    ("B", 4, 4, dict(ALL_ON, lastRow=0, lastColumn=0), 0, {}),
    ("K", 4, 4, ALL_ON, 0, {}),
    ("H", 6, 3, dict(ALL_ON, lastRow=0, firstColumn=0, lastColumn=0, noVBand=1), 2, {}),
    ("N", 7, 7, dict(ALL_ON, lastRow=0, lastColumn=0), 0, {}),
    ("O", 4, 4, dict(firstRow=0, lastRow=0, firstColumn=0, lastColumn=0, noHBand=0, noVBand=0), 0, {}),
    ("M", 4, 5, dict(firstRow=0, lastRow=0, firstColumn=1, lastColumn=1, noHBand=1, noVBand=0), 0,
     {1: [1, 2, 1, 1], 2: [1, 1, 1, 2], 3: [2, 1, 1, 1]}),
    ("P", 4, 4, dict(ALL_ON, lastRow=0, lastColumn=0), 0, {}),
    ("D", 4, 4, ALL_ON, 0, {}),
    ("R", 3, 3, dict(ALL_ON, noHBand=1, noVBand=1), 0, {}),
    ("W", 2, 2, dict(ALL_ON, firstRow=0, firstColumn=0, noHBand=1, noVBand=1), 0, {}),
    ("V", 2, 2, dict(ALL_ON, firstRow=0, firstColumn=0, noHBand=1, noVBand=1), 0, {}),
    ("G", 5, 2, dict(ALL_ON, firstRow=0, lastRow=0, firstColumn=0, lastColumn=0, noVBand=1), 2, {}),
    ("E", 4, 4, dict(ALL_ON, lastRow=0, firstColumn=0, lastColumn=0, noVBand=1), 0, {}),
    ("F", 4, 4, dict(ALL_ON, firstRow=0, lastRow=0, lastColumn=0, noHBand=1), 0, {}),
    ("Q", 6, 3, dict(ALL_ON, firstRow=0, lastRow=0, firstColumn=0, lastColumn=0, noVBand=1), 0, {}),
    ("A", len(AUTO_FILLS) // 4, 4, dict(firstRow=0, lastRow=0, firstColumn=0, lastColumn=0, noHBand=1, noVBand=1),
     0, {}),
]


WIDTH = 1500


def _look(look: dict) -> str:
    return "<w:tblLook " + " ".join(f'w:{k}="{v}"' for k, v in look.items()) + "/>"


def _table(case: str, rows: int, columns: int, look: dict, header: int, merged: dict) -> str:
    grid = "<w:tblGrid>" + f'<w:gridCol w:w="{WIDTH}"/>' * columns + "</w:tblGrid>"
    out = []
    for r in range(rows):
        spans = merged.get(r, [1] * columns)
        cells, column = [], 0
        for span in spans:
            extra = f'<w:gridSpan w:val="{span}"/>' if span > 1 else ""
            if case == "A":
                extra += _shd(AUTO_FILLS[r * 4 + column])
            cells.append(f'<w:tc><w:tcPr><w:tcW w:w="{WIDTH * span}" w:type="dxa"/>{extra}</w:tcPr>'
                         f'<w:p><w:r><w:t>{case}{r}{column}</w:t></w:r></w:p></w:tc>')
            column += span
        trpr = "<w:trPr><w:tblHeader/></w:trPr>" if r < header else ""
        out.append(f"<w:tr>{trpr}{''.join(cells)}</w:tr>")
    props = (f'<w:tblPr><w:tblStyle w:val="{case}"/><w:tblW w:w="0" w:type="auto"/>{_look(look)}</w:tblPr>')
    return f"<w:tbl>{props}{grid}{''.join(out)}</w:tbl><w:p><w:r><w:t>After {case}.</w:t></w:r></w:p>"


def document() -> bytes:
    body = "<w:p><w:r><w:t>Tables follow.</w:t></w:r></w:p>" + "".join(_table(*case) for case in CASES)
    section = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1000" w:right="1000" '
               'w:bottom="1000" w:left="1000" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')
    sheet = styles().replace("</w:styles>", _styles() + "</w:styles>")
    parts = {
        "[Content_Types].xml": content_types({"word/document.xml": WML + ".document.main+xml",
                                              "word/styles.xml": WML + ".styles+xml",
                                              "word/settings.xml": WML + ".settings+xml"}),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([("rId1", REL + "styles", "styles.xml", False),
                                                       ("rId2", REL + "settings", "settings.xml", False)]),
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}{section}</w:body></w:document>",
        "word/styles.xml": sheet,
        "word/settings.xml": settings(15),
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, data in parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=(2026, 11, 16, 12, 0, 0)), data)
    return buffer.getvalue()
