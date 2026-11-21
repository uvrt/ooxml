"""Made-up "Shelf" tables, shared by ``test_rows_r58`` and the reads oracle's facts cases
(``crates/ooxml-oracle/python/word_facts_cases.py``), which run without pytest."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))

from e3_probe import _package  # noqa: E402

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
